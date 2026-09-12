//! Shell links (`.lnk`) through `IShellLinkW` and `IPersistFile`. The
//! `windows-sys` crate declares COM functions but no interfaces, so the two
//! vtables this needs are declared here. A link is written like a file:
//! saved to `<link>.tigersetup-new`, flushed, renamed write-through.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, STGM_READ,
};
use windows_sys::Win32::UI::Shell::SLGP_RAWPATH;
use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};

use crate::win::fs;
use crate::{Error, Result};

const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
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
struct IPersistFileVtbl {
    base: IUnknownVtbl,
    get_class_id: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
    is_dirty: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    load: unsafe extern "system" fn(*mut c_void, PCWSTR, u32) -> HRESULT,
    save: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT,
    save_completed: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    get_cur_file: unsafe extern "system" fn(*mut c_void, *mut PWSTR) -> HRESULT,
}

/// What a shell link points at, as written into it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Link {
    pub target: String,
    pub arguments: String,
    pub description: String,
    pub icon: String,
    pub working_directory: String,
}

/// What a link path holds right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkInspection {
    Absent,
    Link(Link),
    /// A file that is not a shell link.
    NotALink,
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

fn text_of(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&u| u == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Reads what the link at `path` points at.
pub fn inspect(path: &Path) -> Result<LinkInspection> {
    if !path.exists() {
        return Ok(LinkInspection::Absent);
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
    Ok(LinkInspection::Link(Link {
        target: text_of(&target),
        arguments: shell.read_text(shell.vtbl().get_arguments),
        description: shell.read_text(shell.vtbl().get_description),
        icon,
        working_directory: shell.read_text(shell.vtbl().get_working_directory),
    }))
}

/// Writes the link durably: saved to the target's temporary, flushed and
/// renamed write-through over `path`, replacing whatever was there.
/// Returns the SHA-256 of the file written.
pub fn write(path: &Path, link: &Link) -> Result<String> {
    if let Some(parent) = path.parent() {
        fs::create_directory(parent)?;
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
    }
}
