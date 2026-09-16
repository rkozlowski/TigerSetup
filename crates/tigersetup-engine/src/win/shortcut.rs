//! Shell links (`.lnk`) through `IShellLinkW`, `IPersistFile` and, for the
//! AppUserModelID, `IPropertyStore`; Internet shortcuts (`.url`) as the
//! plain `[InternetShortcut]` text files they are. The `windows-sys` crate
//! declares COM functions but no interfaces, so the vtables this needs are
//! declared here. A link is written like a file: saved to
//! `<link>.tigersetup-new`, flushed, renamed write-through.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, STGM_READ,
};
use windows_sys::Win32::UI::Shell::SLGP_RAWPATH;
use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};

use crate::win::fs;
use crate::{Error, Result};

const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
const IID_IPROPERTY_STORE: GUID = GUID::from_u128(0x886d8eeb_8cf2_4446_8d02_cdba1dbdcf99);
/// `PKEY_AppUserModel_ID`: `{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}`, 5.
const PKEY_APP_USER_MODEL_ID: PropertyKey = PropertyKey {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
const VT_EMPTY: u16 = 0;
const VT_LPWSTR: u16 = 31;
/// `RPC_E_CHANGED_MODE`: COM was already initialised with another model;
/// the apartment is usable all the same.
const RPC_E_CHANGED_MODE: HRESULT = 0x8001_0106u32 as i32;

#[repr(C)]
struct IUnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct IShellLinkWVtbl {
    base: IUnknownVtbl,
    get_path: unsafe extern "system" fn(*mut c_void, PWSTR, i32, *mut c_void, u32) -> HRESULT,
    get_id_list: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    set_id_list: unsafe extern "system" fn(*mut c_void, *const c_void) -> HRESULT,
    get_description: unsafe extern "system" fn(*mut c_void, PWSTR, i32) -> HRESULT,
    set_description: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_working_directory: unsafe extern "system" fn(*mut c_void, PWSTR, i32) -> HRESULT,
    set_working_directory: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_arguments: unsafe extern "system" fn(*mut c_void, PWSTR, i32) -> HRESULT,
    set_arguments: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_hotkey: unsafe extern "system" fn(*mut c_void, *mut u16) -> HRESULT,
    set_hotkey: unsafe extern "system" fn(*mut c_void, u16) -> HRESULT,
    get_show_cmd: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    set_show_cmd: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    get_icon_location: unsafe extern "system" fn(*mut c_void, PWSTR, i32, *mut i32) -> HRESULT,
    set_icon_location: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
    set_relative_path: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
    resolve: unsafe extern "system" fn(*mut c_void, HWND, u32) -> HRESULT,
    set_path: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
}

#[repr(C)]
struct PropertyKey {
    fmtid: GUID,
    pid: u32,
}

/// `PROPVARIANT`, as far as a string property needs it: the type tag, the
/// reserved words, and the pointer arm of the union.
#[repr(C)]
struct PropVariant {
    vt: u16,
    reserved: [u16; 3],
    pointer: *mut u16,
    padding: usize,
}

#[repr(C)]
struct IPropertyStoreVtbl {
    base: IUnknownVtbl,
    get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_at: unsafe extern "system" fn(*mut c_void, u32, *mut PropertyKey) -> HRESULT,
    get_value:
        unsafe extern "system" fn(*mut c_void, *const PropertyKey, *mut PropVariant) -> HRESULT,
    set_value:
        unsafe extern "system" fn(*mut c_void, *const PropertyKey, *const PropVariant) -> HRESULT,
    commit: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

#[repr(C)]
struct IPersistFileVtbl {
    base: IUnknownVtbl,
    get_class_id: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
    is_dirty: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    load: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
    save: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
    save_completed: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_cur_file: unsafe extern "system" fn(*mut c_void, *mut PWSTR) -> HRESULT,
}

/// What a shell link points at, as written into it. For an Internet
/// shortcut `target` is the URL and every other field but `icon` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Link {
    pub target: String,
    pub arguments: String,
    pub description: String,
    pub icon: String,
    pub working_directory: String,
    /// The AppUserModelID written into the link's property store; empty
    /// means none.
    pub app_user_model_id: String,
}

/// What a link path holds right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkInspection {
    Absent,
    Link(Link),
    /// A file that is not a shell link.
    NotALink,
}

/// Whether a link path names an Internet shortcut rather than a shell link.
pub fn is_url_shortcut(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("url"))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn com_error(what: &str, path: &Path, hr: HRESULT) -> Error {
    Error::new(
        "shortcut_error",
        format!("{what} {}: HRESULT 0x{:08x}", path.display(), hr as u32),
    )
}

/// A COM apartment for the current thread, released on drop.
struct Apartment {
    uninitialise: bool,
}

impl Apartment {
    fn enter(path: &Path) -> Result<Apartment> {
        let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Apartment {
                uninitialise: false,
            });
        }
        if hr < 0 {
            return Err(com_error("cannot initialise COM for", path, hr));
        }
        Ok(Apartment { uninitialise: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.uninitialise {
            unsafe { CoUninitialize() };
        }
    }
}

/// A shell-link object with its two interfaces, released on drop.
struct ShellLink {
    _apartment: Apartment,
    link: *mut c_void,
    persist: *mut c_void,
}

impl ShellLink {
    fn create(path: &Path) -> Result<ShellLink> {
        let apartment = Apartment::enter(path)?;
        let mut link: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CLSID_SHELL_LINK,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ISHELL_LINK_W,
                &mut link,
            )
        };
        if hr < 0 || link.is_null() {
            return Err(com_error("cannot create a shell link for", path, hr));
        }
        let mut persist: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            ((*(link as *mut *mut IShellLinkWVtbl).read())
                .base
                .query_interface)(link, &IID_IPERSIST_FILE, &mut persist)
        };
        if hr < 0 || persist.is_null() {
            unsafe { ((*(link as *mut *mut IShellLinkWVtbl).read()).base.release)(link) };
            return Err(com_error("cannot persist a shell link for", path, hr));
        }
        Ok(ShellLink {
            _apartment: apartment,
            link,
            persist,
        })
    }

    fn vtbl(&self) -> &IShellLinkWVtbl {
        unsafe { &*(self.link as *mut *mut IShellLinkWVtbl).read() }
    }

    fn persist_vtbl(&self) -> &IPersistFileVtbl {
        unsafe { &*(self.persist as *mut *mut IPersistFileVtbl).read() }
    }

    /// The link's property store, where the AppUserModelID lives.
    fn property_store(&self, path: &Path) -> Result<PropertyStore> {
        let mut store: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            (self.vtbl().base.query_interface)(self.link, &IID_IPROPERTY_STORE, &mut store)
        };
        if hr < 0 || store.is_null() {
            return Err(com_error("cannot open the property store of", path, hr));
        }
        Ok(PropertyStore(store))
    }

    fn read_text(
        &self,
        getter: unsafe extern "system" fn(*mut c_void, PWSTR, i32) -> HRESULT,
    ) -> String {
        let mut buffer = vec![0u16; 32_768];
        let hr = unsafe { getter(self.link, buffer.as_mut_ptr(), buffer.len() as i32) };
        if hr < 0 {
            return String::new();
        }
        text_of(&buffer)
    }
}

impl Drop for ShellLink {
    fn drop(&mut self) {
        unsafe {
            (self.persist_vtbl().base.release)(self.persist);
            (self.vtbl().base.release)(self.link);
        }
    }
}

/// A link's `IPropertyStore`, released on drop.
struct PropertyStore(*mut c_void);

impl PropertyStore {
    fn vtbl(&self) -> &IPropertyStoreVtbl {
        unsafe { &*(self.0 as *mut *mut IPropertyStoreVtbl).read() }
    }

    /// The AppUserModelID, or an empty string when the link carries none.
    fn app_user_model_id(&self) -> String {
        let mut value = PropVariant {
            vt: VT_EMPTY,
            reserved: [0; 3],
            pointer: ptr::null_mut(),
            padding: 0,
        };
        let hr = unsafe { (self.vtbl().get_value)(self.0, &PKEY_APP_USER_MODEL_ID, &mut value) };
        if hr < 0 || value.vt != VT_LPWSTR || value.pointer.is_null() {
            return String::new();
        }
        let mut length = 0;
        while unsafe { *value.pointer.add(length) } != 0 {
            length += 1;
        }
        let text =
            String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value.pointer, length) });
        // `PropVariantClear` on a `VT_LPWSTR` is exactly this.
        unsafe { CoTaskMemFree(value.pointer as *const _) };
        text
    }

    fn set_app_user_model_id(&self, id: &str, path: &Path) -> Result<()> {
        let id_w = wide(id);
        let value = PropVariant {
            vt: VT_LPWSTR,
            reserved: [0; 3],
            pointer: id_w.as_ptr() as *mut u16,
            padding: 0,
        };
        let hr = unsafe { (self.vtbl().set_value)(self.0, &PKEY_APP_USER_MODEL_ID, &value) };
        if hr < 0 {
            return Err(com_error("cannot set the AppUserModelID of", path, hr));
        }
        let hr = unsafe { (self.vtbl().commit)(self.0) };
        if hr < 0 {
            return Err(com_error("cannot commit the AppUserModelID of", path, hr));
        }
        Ok(())
    }
}

impl Drop for PropertyStore {
    fn drop(&mut self) {
        unsafe { (self.vtbl().base.release)(self.0) };
    }
}

fn text_of(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&u| u == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// The text of an Internet shortcut: the `[InternetShortcut]` section with
/// its URL and, when there is one, the icon.
fn url_file_text(link: &Link) -> String {
    let mut text = format!("[InternetShortcut]\r\nURL={}\r\n", link.target);
    if !link.icon.is_empty() {
        text.push_str(&format!("IconFile={}\r\nIconIndex=0\r\n", link.icon));
    }
    text
}

/// Reads an Internet shortcut's URL and icon; `None` when the file is not
/// one.
fn parse_url_file(text: &str) -> Option<Link> {
    let mut in_section = false;
    let mut link = Link::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[InternetShortcut]");
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            match key.trim().to_ascii_lowercase().as_str() {
                "url" => link.target = value.trim().to_string(),
                "iconfile" => link.icon = value.trim().to_string(),
                _ => {}
            }
        }
    }
    (!link.target.is_empty()).then_some(link)
}

/// Reads what the link at `path` points at.
pub fn inspect(path: &Path) -> Result<LinkInspection> {
    if !path.exists() {
        return Ok(LinkInspection::Absent);
    }
    if is_url_shortcut(path) {
        let bytes = std::fs::read(path)?;
        return Ok(match parse_url_file(&String::from_utf8_lossy(&bytes)) {
            Some(link) => LinkInspection::Link(link),
            None => LinkInspection::NotALink,
        });
    }
    let shell = ShellLink::create(path)?;
    let path_w = wide(&path.display().to_string());
    let hr = unsafe { (shell.persist_vtbl().load)(shell.persist, path_w.as_ptr(), STGM_READ) };
    if hr < 0 {
        return Ok(LinkInspection::NotALink);
    }
    let mut target = vec![0u16; 32_768];
    let hr = unsafe {
        (shell.vtbl().get_path)(
            shell.link,
            target.as_mut_ptr(),
            target.len() as i32,
            ptr::null_mut(),
            SLGP_RAWPATH as u32,
        )
    };
    if hr < 0 {
        return Ok(LinkInspection::NotALink);
    }
    let mut icon_index = 0i32;
    let mut icon = vec![0u16; 32_768];
    let hr = unsafe {
        (shell.vtbl().get_icon_location)(
            shell.link,
            icon.as_mut_ptr(),
            icon.len() as i32,
            &mut icon_index,
        )
    };
    let icon = if hr < 0 {
        String::new()
    } else {
        text_of(&icon)
    };
    let app_user_model_id = shell
        .property_store(path)
        .map(|store| store.app_user_model_id())
        .unwrap_or_default();
    Ok(LinkInspection::Link(Link {
        target: text_of(&target),
        arguments: shell.read_text(shell.vtbl().get_arguments),
        description: shell.read_text(shell.vtbl().get_description),
        icon,
        working_directory: shell.read_text(shell.vtbl().get_working_directory),
        app_user_model_id,
    }))
}

/// Writes the link durably: saved to the target's temporary, flushed and
/// renamed write-through over `path`, replacing whatever was there.
/// Returns the SHA-256 of the file written.
pub fn write(path: &Path, link: &Link) -> Result<String> {
    if let Some(parent) = path.parent() {
        fs::create_directory(parent)?;
    }
    if is_url_shortcut(path) {
        let text = url_file_text(link);
        let mut staged = fs::Staged::write(path, &mut text.as_bytes())?;
        staged.flush()?;
        let sha256 = staged.sha256.clone();
        staged.commit()?;
        return Ok(sha256);
    }
    let temp = fs::temp_path_for(path);
    {
        let shell = ShellLink::create(path)?;
        let set = |what: &str,
                   setter: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
                   value: &str|
         -> Result<()> {
            let value_w = wide(value);
            let hr = unsafe { setter(shell.link, value_w.as_ptr()) };
            if hr < 0 {
                return Err(com_error(&format!("cannot set the {what} of"), path, hr));
            }
            Ok(())
        };
        set("target", shell.vtbl().set_path, &link.target)?;
        set("arguments", shell.vtbl().set_arguments, &link.arguments)?;
        set(
            "description",
            shell.vtbl().set_description,
            &link.description,
        )?;
        set(
            "working directory",
            shell.vtbl().set_working_directory,
            &link.working_directory,
        )?;
        if !link.icon.is_empty() {
            let icon_w = wide(&link.icon);
            let hr = unsafe { (shell.vtbl().set_icon_location)(shell.link, icon_w.as_ptr(), 0) };
            if hr < 0 {
                return Err(com_error("cannot set the icon of", path, hr));
            }
        }
        if !link.app_user_model_id.is_empty() {
            shell
                .property_store(path)?
                .set_app_user_model_id(&link.app_user_model_id, path)?;
        }
        let temp_w = wide(&temp.display().to_string());
        let hr = unsafe { (shell.persist_vtbl().save)(shell.persist, temp_w.as_ptr(), 1) };
        if hr < 0 {
            let _ = std::fs::remove_file(&temp);
            return Err(com_error("cannot save", path, hr));
        }
    }
    let result = (|| {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&temp)
            .map_err(|err| {
                Error::new("io_error", format!("cannot open {}: {err}", temp.display()))
            })?;
        fs::flush(&file, &temp)?;
        drop(file);
        let sha256 = match fs::inspect(&temp)? {
            fs::Inspection::Present { sha256, .. } => sha256,
            fs::Inspection::Absent => {
                return Err(Error::new(
                    "io_error",
                    format!("{} vanished before it was renamed", temp.display()),
                ));
            }
        };
        fs::rename_write_through(&temp, path)?;
        Ok(sha256)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip_and_replace_durably() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.exe");
        std::fs::write(&target, b"not really an executable").unwrap();
        let path = dir.path().join("Sub").join("App.lnk");
        assert_eq!(inspect(&path).unwrap(), LinkInspection::Absent);

        let link = Link {
            target: target.display().to_string(),
            arguments: "--flag \"quoted value\"".into(),
            description: "A test link".into(),
            icon: target.display().to_string(),
            working_directory: dir.path().display().to_string(),
            app_user_model_id: "ITTiger.TestApp".into(),
        };
        let sha256 = write(&path, &link).unwrap();
        assert_eq!(sha256.len(), 64);
        assert!(!fs::temp_path_for(&path).exists());
        assert_eq!(inspect(&path).unwrap(), LinkInspection::Link(link.clone()));

        let other = Link {
            arguments: String::new(),
            ..link.clone()
        };
        write(&path, &other).unwrap();
        assert_eq!(inspect(&path).unwrap(), LinkInspection::Link(other));

        let plain = dir.path().join("plain.lnk");
        std::fs::write(&plain, b"this is not a shell link").unwrap();
        assert_eq!(inspect(&plain).unwrap(), LinkInspection::NotALink);

        // A link written without an AppUserModelID reads back without one.
        let bare = Link {
            app_user_model_id: String::new(),
            ..link.clone()
        };
        let bare_path = dir.path().join("Bare.lnk");
        write(&bare_path, &bare).unwrap();
        assert_eq!(inspect(&bare_path).unwrap(), LinkInspection::Link(bare));
    }

    /// An Internet shortcut is a text file: written and read as one, with
    /// the same durable write as a shell link.
    #[test]
    fn url_shortcuts_round_trip_as_internet_shortcut_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Docs.url");
        assert!(is_url_shortcut(&path));
        assert!(!is_url_shortcut(&dir.path().join("App.lnk")));
        let link = Link {
            target: "https://example.invalid/docs".into(),
            icon: "C:\\P\\app.exe".into(),
            ..Link::default()
        };
        let sha256 = write(&path, &link).unwrap();
        assert_eq!(sha256.len(), 64);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("[InternetShortcut]\r\nURL=https://example.invalid/docs\r\n"),
            "{text}"
        );
        assert_eq!(inspect(&path).unwrap(), LinkInspection::Link(link));
        std::fs::write(&path, "not a shortcut").unwrap();
        assert_eq!(inspect(&path).unwrap(), LinkInspection::NotALink);
    }
}
