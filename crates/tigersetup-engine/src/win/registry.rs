//! Registry primitives: keys addressed as `HKCU\...` / `HKLM\...` text,
//! always opened in the 64-bit view (`KEY_WOW64_64KEY`), typed values, and
//! the root relocation the test seam needs. A key path is logical — the
//! journal and the ownership tables store it as written here — and
//! [`Roots`] decides where it physically lives.

use std::fmt;
use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE,
    KEY_WOW64_64KEY, REG_DWORD, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyExW, RegDeleteValueW, RegFlushKey, RegOpenKeyExW,
    RegQueryInfoKeyW, RegQueryValueExW, RegSetValueExW,
};

use crate::{Error, Result};

/// The environment variable that relocates every root under
/// `HKCU\<value>`: `HKCU\X` becomes `HKCU\<value>\HKCU\X` and `HKLM\X`
/// becomes `HKCU\<value>\HKLM\X`, so a test never touches the real hives.
pub const TEST_ROOT_VARIABLE: &str = "TIGERSETUP_TEST_REGISTRY_ROOT";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

impl Hive {
    pub fn as_str(self) -> &'static str {
        match self {
            Hive::CurrentUser => "HKCU",
            Hive::LocalMachine => "HKLM",
        }
    }

    pub fn parse(text: &str) -> Option<Hive> {
        match text.to_ascii_uppercase().as_str() {
            "HKCU" | "HKEY_CURRENT_USER" => Some(Hive::CurrentUser),
            "HKLM" | "HKEY_LOCAL_MACHINE" => Some(Hive::LocalMachine),
            _ => None,
        }
    }

    fn handle(self) -> HKEY {
        match self {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }
}

/// A logical key path: a hive and a subkey, written `HKCU\Software\...`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyPath {
    pub hive: Hive,
    pub subkey: String,
}

impl KeyPath {
    pub fn new(hive: Hive, subkey: impl Into<String>) -> KeyPath {
        KeyPath {
            hive,
            subkey: subkey.into(),
        }
    }

    /// Parses `HKCU\sub\key` or `HKLM\sub\key`.
    pub fn parse(text: &str) -> Result<KeyPath> {
        let invalid = || {
            Error::new(
                "registry_path_invalid",
                format!("{text:?} is not a registry key path with a hive"),
            )
        };
        let (hive, subkey) = text.split_once('\\').ok_or_else(invalid)?;
        let hive = Hive::parse(hive).ok_or_else(invalid)?;
        if subkey.is_empty() || subkey.starts_with('\\') || subkey.ends_with('\\') {
            return Err(invalid());
        }
        Ok(KeyPath::new(hive, subkey))
    }

    pub fn child(&self, name: &str) -> KeyPath {
        KeyPath::new(self.hive, format!("{}\\{name}", self.subkey))
    }

    /// The parent key, or `None` at the top of the hive.
    pub fn parent(&self) -> Option<KeyPath> {
        self.subkey
            .rsplit_once('\\')
            .map(|(parent, _)| KeyPath::new(self.hive, parent))
    }

    /// Case-insensitive identity.
    pub fn key(&self) -> String {
        self.to_string().to_ascii_lowercase()
    }

    pub fn depth(&self) -> usize {
        self.subkey.matches('\\').count() + 1
    }

    /// Whether `self` is `ancestor` or lies below it.
    pub fn is_under(&self, ancestor: &KeyPath) -> bool {
        self.hive == ancestor.hive && {
            let mine = self.subkey.to_ascii_lowercase();
            let theirs = ancestor.subkey.to_ascii_lowercase();
            mine == theirs || mine.starts_with(&format!("{theirs}\\"))
        }
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\\{}", self.hive.as_str(), self.subkey)
    }
}

/// Where the logical roots physically are: the real hives, or every root
/// relocated under a prefix beneath `HKCU`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    prefix: Option<String>,
}

impl Roots {
    /// The real hives.
    pub fn real() -> Roots {
        Roots { prefix: None }
    }

    /// Every root relocated under `HKCU\<prefix>`.
    pub fn relocated(prefix: &str) -> Roots {
        Roots {
            prefix: Some(prefix.trim_matches('\\').to_string()),
        }
    }

    /// The roots this process uses: relocated when the test seam is set.
    pub fn from_env() -> Roots {
        match std::env::var(TEST_ROOT_VARIABLE) {
            Ok(prefix) if !prefix.trim().is_empty() => Roots::relocated(&prefix),
            _ => Roots::real(),
        }
    }

    pub fn is_relocated(&self) -> bool {
        self.prefix.is_some()
    }

    /// The physical root handle and subkey of a logical key.
    fn physical(&self, key: &KeyPath) -> (HKEY, String) {
        match &self.prefix {
            None => (key.hive.handle(), key.subkey.clone()),
            Some(prefix) => (
                HKEY_CURRENT_USER,
                format!("{prefix}\\{}\\{}", key.hive.as_str(), key.subkey),
            ),
        }
    }
}

/// A typed registry value. Kinds the engine does not write are carried as
/// raw bytes so that an undo can restore exactly what was there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    String(String),
    ExpandString(String),
    Dword(u32),
    Raw { kind: u32, bytes: Vec<u8> },
}

impl Data {
    /// The kind as the database stores it.
    pub fn kind_name(&self) -> String {
        match self {
            Data::String(_) => "string".into(),
            Data::ExpandString(_) => "expand_string".into(),
            Data::Dword(_) => "dword".into(),
            Data::Raw { kind, .. } => format!("raw:{kind}"),
        }
    }

    /// The data as the database stores it: the text, the decimal number, or
    /// hex bytes.
    pub fn text(&self) -> String {
        match self {
            Data::String(text) | Data::ExpandString(text) => text.clone(),
            Data::Dword(value) => value.to_string(),
            Data::Raw { bytes, .. } => tigersetup_format::hex(bytes),
        }
    }

    /// The text of a string value, for comparisons and reports.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Data::String(text) | Data::ExpandString(text) => Some(text),
            _ => None,
        }
    }

    /// Rebuilds a value from its stored kind and text.
    pub fn from_columns(kind: &str, text: &str) -> Result<Data> {
        let invalid = || {
            Error::new(
                "journal_inconsistent",
                format!("registry data {text:?} is not a {kind}"),
            )
        };
        match kind {
            "string" => Ok(Data::String(text.into())),
            "expand_string" => Ok(Data::ExpandString(text.into())),
            "dword" => text.parse().map(Data::Dword).map_err(|_| invalid()),
            other => {
                let kind: u32 = other
                    .strip_prefix("raw:")
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(invalid)?;
                let bytes = unhex(text).ok_or_else(invalid)?;
                Ok(Data::Raw { kind, bytes })
            }
        }
    }

    fn encode(&self) -> (u32, Vec<u8>) {
        match self {
            Data::String(text) => (REG_SZ, wide_bytes(text)),
            Data::ExpandString(text) => (REG_EXPAND_SZ, wide_bytes(text)),
            Data::Dword(value) => (REG_DWORD, value.to_le_bytes().to_vec()),
            Data::Raw { kind, bytes } => (*kind, bytes.clone()),
        }
    }

    fn decode(kind: u32, bytes: &[u8]) -> Data {
        match kind {
            REG_SZ => Data::String(text_from_bytes(bytes)),
            REG_EXPAND_SZ => Data::ExpandString(text_from_bytes(bytes)),
            REG_DWORD if bytes.len() == 4 => {
                Data::Dword(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            _ => Data::Raw {
                kind,
                bytes: bytes.to_vec(),
            },
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_bytes(text: &str) -> Vec<u8> {
    wide(text).iter().flat_map(|u| u.to_le_bytes()).collect()
}

fn text_from_bytes(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn registry_error(what: &str, key: &KeyPath, code: u32) -> Error {
    Error::new(
        "registry_error",
        format!(
            "{what} {key}: {}",
            std::io::Error::from_raw_os_error(code as i32)
        ),
    )
}

/// An open key handle, closed on drop.
struct Handle(HKEY);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

/// Opens a key; `None` when it does not exist.
fn open(roots: &Roots, key: &KeyPath, access: u32) -> Result<Option<Handle>> {
    let (root, subkey) = roots.physical(key);
    let subkey_w = wide(&subkey);
    let mut handle: HKEY = ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            subkey_w.as_ptr(),
            0,
            access | KEY_WOW64_64KEY,
            &mut handle,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(Handle(handle))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        code => Err(registry_error("cannot open", key, code)),
    }
}

/// Writes the registry changes of this process to disk.
///
/// Windows flushes the registry lazily. A file the engine installs is flushed
/// before the journal records it, but a registry value written by
/// `RegSetValueEx` may still be only in memory — so a power cut can take a
/// value the journal already calls applied, and a transaction that then
/// commits leaves the database and the machine disagreeing with nothing left
/// to reconcile them. Flushing once before the commit closes that window: a
/// cut before it leaves the transaction open for recovery, and a cut after it
/// finds the values already on disk.
///
/// `RegFlushKey` flushes the hive file the key belongs to, so this is one
/// call per hive file per transaction rather than one per value. The user
/// scope writes one hive file, `HKCU` itself; the machine scope's `HKLM`
/// is a master key over separate hive files, of which the scope writes
/// `SOFTWARE` (product values, registration, `App Paths`, classes) and,
/// through an explicit registry root, `SYSTEM` — so each of those is
/// flushed by a key inside it, because flushing the master key writes
/// neither.
pub fn flush(roots: &Roots, hive: Hive) -> Result<()> {
    // A relocated root is a test seam under HKCU; flushing HKCU covers it.
    if roots.is_relocated() || hive == Hive::CurrentUser {
        return flush_handle(HKEY_CURRENT_USER, "HKCU");
    }
    for subkey in ["SOFTWARE", "SYSTEM"] {
        let subkey_w = wide(subkey);
        let mut handle: HKEY = ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                subkey_w.as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_WOW64_64KEY,
                &mut handle,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(Error::new(
                "registry_error",
                format!(
                    "cannot open HKLM\\{subkey} to flush it: {}",
                    std::io::Error::from_raw_os_error(status as i32)
                ),
            ));
        }
        let handle = Handle(handle);
        flush_handle(handle.0, &format!("HKLM\\{subkey}"))?;
    }
    Ok(())
}

fn flush_handle(handle: HKEY, name: &str) -> Result<()> {
    let status = unsafe { RegFlushKey(handle) };
    match status == ERROR_SUCCESS {
        true => Ok(()),
        false => Err(Error::new(
            "registry_error",
            format!(
                "cannot flush {name}: {}",
                std::io::Error::from_raw_os_error(status as i32)
            ),
        )),
    }
}

pub fn key_exists(roots: &Roots, key: &KeyPath) -> Result<bool> {
    Ok(open(roots, key, KEY_READ)?.is_some())
}

/// Creates the key (and any missing parent), non-volatile. Idempotent.
pub fn create_key(roots: &Roots, key: &KeyPath) -> Result<()> {
    let (root, subkey) = roots.physical(key);
    let subkey_w = wide(&subkey);
    let mut handle: HKEY = ptr::null_mut();
    let mut disposition = 0u32;
    let status = unsafe {
        RegCreateKeyExW(
            root,
            subkey_w.as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WOW64_64KEY,
            ptr::null(),
            &mut handle,
            &mut disposition,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(registry_error("cannot create", key, status));
    }
    drop(Handle(handle));
    Ok(())
}

/// Reads a value; `None` when the key or the value does not exist.
pub fn read_value(roots: &Roots, key: &KeyPath, name: &str) -> Result<Option<Data>> {
    let Some(handle) = open(roots, key, KEY_QUERY_VALUE)? else {
        return Ok(None);
    };
    let name_w = wide(name);
    let mut kind = 0u32;
    let mut size = 0u32;
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        let status = unsafe {
            RegQueryValueExW(
                handle.0,
                name_w.as_ptr(),
                ptr::null(),
                &mut kind,
                if buffer.is_empty() {
                    ptr::null_mut()
                } else {
                    buffer.as_mut_ptr()
                },
                &mut size,
            )
        };
        match status {
            ERROR_SUCCESS if buffer.is_empty() && size > 0 => {
                buffer = vec![0u8; size as usize];
            }
            ERROR_SUCCESS => {
                buffer.truncate(size as usize);
                return Ok(Some(Data::decode(kind, &buffer)));
            }
            ERROR_MORE_DATA => buffer = vec![0u8; size as usize],
            ERROR_FILE_NOT_FOUND => return Ok(None),
            code => {
                return Err(registry_error(
                    &format!("cannot read value {name:?} of"),
                    key,
                    code,
                ));
            }
        }
    }
}

/// Writes a value into an existing key.
pub fn write_value(roots: &Roots, key: &KeyPath, name: &str, data: &Data) -> Result<()> {
    let handle = open(roots, key, KEY_SET_VALUE)?
        .ok_or_else(|| registry_error("cannot open", key, ERROR_FILE_NOT_FOUND))?;
    let name_w = wide(name);
    let (kind, bytes) = data.encode();
    let status = unsafe {
        RegSetValueExW(
            handle.0,
            name_w.as_ptr(),
            0,
            kind,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(registry_error(
            &format!("cannot write value {name:?} of"),
            key,
            status,
        ));
    }
    Ok(())
}

/// Deletes a value; an absent key or value is not an error.
pub fn delete_value(roots: &Roots, key: &KeyPath, name: &str) -> Result<()> {
    let Some(handle) = open(roots, key, KEY_SET_VALUE)? else {
        return Ok(());
    };
    let name_w = wide(name);
    match unsafe { RegDeleteValueW(handle.0, name_w.as_ptr()) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        code => Err(registry_error(
            &format!("cannot delete value {name:?} of"),
            key,
            code,
        )),
    }
}

/// Whether the key holds no values and no subkeys; `None` when absent.
pub fn key_is_empty(roots: &Roots, key: &KeyPath) -> Result<Option<bool>> {
    let Some(handle) = open(roots, key, KEY_READ)? else {
        return Ok(None);
    };
    let mut subkeys = 0u32;
    let mut values = 0u32;
    let status = unsafe {
        RegQueryInfoKeyW(
            handle.0,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
            &mut subkeys,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut values,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(registry_error("cannot query", key, status));
    }
    Ok(Some(subkeys == 0 && values == 0))
}

/// Deletes the key only when it holds nothing; returns whether it is gone
/// afterwards. Never touches values or subkeys TigerSetup did not put there.
pub fn delete_key_if_empty(roots: &Roots, key: &KeyPath) -> Result<bool> {
    match key_is_empty(roots, key)? {
        None => return Ok(true),
        Some(false) => return Ok(false),
        Some(true) => {}
    }
    let Some(parent) = key.parent() else {
        return Ok(false);
    };
    let (root, parent_subkey) = roots.physical(&parent);
    let parent_w = wide(&parent_subkey);
    let mut parent_handle: HKEY = ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            parent_w.as_ptr(),
            0,
            KEY_READ | KEY_WOW64_64KEY,
            &mut parent_handle,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(registry_error("cannot open parent of", key, status));
    }
    let parent_handle = Handle(parent_handle);
    let leaf = key.subkey.rsplit('\\').next().unwrap_or(&key.subkey);
    let leaf_w = wide(leaf);
    match unsafe { RegDeleteKeyExW(parent_handle.0, leaf_w.as_ptr(), KEY_WOW64_64KEY, 0) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(true),
        code => Err(registry_error("cannot delete", key, code)),
    }
}

// ---------------------------------------------------------------------------
// Reading the machine as it is, for dependency detection.
//
// A dependency lives on the real machine, so detection never goes through
// `Roots`: it opens the hive a package's detector names and reads it. Hive
// prefixes accepted here are the ones a package may write: `HKLM\...`,
// `HKEY_LOCAL_MACHINE\...`, `HKCU\...`, `HKEY_CURRENT_USER\...`, `HKU\...`.
// The engine is a 64-bit process, so a path names the native view and
// `WOW6432Node` is spelled out where the 32-bit view is meant.
// ---------------------------------------------------------------------------
use windows_sys::Win32::Foundation::ERROR_NO_MORE_ITEMS;
use windows_sys::Win32::System::Registry::{HKEY_USERS, REG_MULTI_SZ, RegEnumKeyExW};

/// A UTF-16 buffer the registry filled, up to its first NUL.
fn from_wide(units: &[u16]) -> String {
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// An open registry key, closed when dropped.
pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

/// Splits `HIVE\sub\key` into the hive handle and the subkey path.
pub fn parse_root(path: &str) -> Option<(HKEY, &str)> {
    let (hive, rest) = match path.split_once('\\') {
        Some((hive, rest)) => (hive, rest),
        None => (path, ""),
    };
    let root = match hive.to_ascii_uppercase().as_str() {
        "HKLM" | "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKCU" | "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKU" | "HKEY_USERS" => HKEY_USERS,
        _ => return None,
    };
    Some((root, rest))
}

/// Opens a key by its full path for reading; `None` when it does not exist
/// or the path names no known hive.
pub fn open_native(path: &str) -> Option<Key> {
    let (root, subkey) = parse_root(path)?;
    open_under(root, subkey)
}

fn open_under(root: HKEY, subkey: &str) -> Option<Key> {
    let subkey_w = wide(subkey);
    let mut handle: HKEY = std::ptr::null_mut();
    let status = unsafe { RegOpenKeyExW(root, subkey_w.as_ptr(), 0, KEY_READ, &mut handle) };
    (status == ERROR_SUCCESS && !handle.is_null()).then_some(Key(handle))
}

impl Key {
    pub fn open_subkey(&self, name: &str) -> Option<Key> {
        open_under(self.0, name)
    }

    /// A `REG_SZ`, `REG_EXPAND_SZ` (unexpanded) or `REG_MULTI_SZ` (first
    /// string) value; `None` when absent or of another type.
    pub fn string_value(&self, name: &str) -> Option<String> {
        let name_w = wide(name);
        let mut kind: u32 = 0;
        let mut size: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name_w.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS || !matches!(kind, REG_SZ | REG_EXPAND_SZ | REG_MULTI_SZ) {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name_w.as_ptr(),
                std::ptr::null(),
                &mut kind,
                buffer.as_mut_ptr() as *mut u8,
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        buffer.truncate((size as usize) / 2);
        Some(from_wide(&buffer))
    }

    /// The names of the key's direct subkeys.
    pub fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut index = 0u32;
        loop {
            let mut buffer = vec![0u16; 256];
            let mut length = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => names.push(from_wide(&buffer[..length as usize])),
                ERROR_MORE_DATA => {}
                ERROR_NO_MORE_ITEMS => break,
                _ => break,
            }
            index += 1;
        }
        names
    }

    #[cfg(test)]
    pub(crate) fn raw(&self) -> HKEY {
        self.0
    }
}

/// Test-only writes: a scratch key under `HKCU\Software\TigerSetupTest`,
/// removed when the guard drops.
#[cfg(test)]
pub(crate) mod scratch {
    use super::*;
    use windows_sys::Win32::System::Registry::{
        KEY_ALL_ACCESS, REG_OPTION_NON_VOLATILE, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
    };

    pub struct ScratchKey {
        pub path: String,
        subkey: String,
    }

    impl ScratchKey {
        /// Creates `HKCU\Software\TigerSetupTest\<random>`.
        pub fn new() -> ScratchKey {
            let subkey = format!("Software\\TigerSetupTest\\{}", crate::report::unique_id());
            let key = create(&subkey);
            drop(key);
            ScratchKey {
                path: format!("HKCU\\{subkey}"),
                subkey,
            }
        }

        /// Creates a subkey and sets string values on it.
        pub fn set(&self, relative: &str, values: &[(&str, &str)]) {
            let path = if relative.is_empty() {
                self.subkey.clone()
            } else {
                format!("{}\\{relative}", self.subkey)
            };
            let key = create(&path);
            for (name, value) in values {
                let name_w = wide(name);
                let value_w = wide(value);
                let status = unsafe {
                    RegSetValueExW(
                        key.raw(),
                        name_w.as_ptr(),
                        0,
                        REG_SZ,
                        value_w.as_ptr() as *const u8,
                        (value_w.len() * 2) as u32,
                    )
                };
                assert_eq!(status, ERROR_SUCCESS);
            }
        }
    }

    impl Drop for ScratchKey {
        fn drop(&mut self) {
            let subkey_w = wide(&self.subkey);
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, subkey_w.as_ptr()) };
        }
    }

    fn create(subkey: &str) -> Key {
        let subkey_w = wide(subkey);
        let mut handle: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey_w.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(status, ERROR_SUCCESS, "cannot create HKCU\\{subkey}");
        Key(handle)
    }
}

#[cfg(test)]
mod detection_tests {
    use super::*;

    #[test]
    fn hives_are_parsed_and_values_read() {
        let (root, rest) = parse_root("HKLM\\SOFTWARE\\Microsoft").unwrap();
        assert_eq!(root, HKEY_LOCAL_MACHINE);
        assert_eq!(rest, "SOFTWARE\\Microsoft");
        assert_eq!(parse_root("hkey_current_user").unwrap().1, "");
        assert!(parse_root("HKXX\\x").is_none());

        let scratch = scratch::ScratchKey::new();
        scratch.set("Clients\\{ABC}", &[("pv", "152.0.4191.53"), ("name", "x")]);
        scratch.set("Clients\\{DEF}", &[("pv", "0.0.0.0")]);
        let key = open_native(&format!("{}\\Clients\\{{ABC}}", scratch.path)).unwrap();
        assert_eq!(key.string_value("pv").as_deref(), Some("152.0.4191.53"));
        assert_eq!(key.string_value("missing"), None);
        let clients = open_native(&format!("{}\\Clients", scratch.path)).unwrap();
        let mut names = clients.subkey_names();
        names.sort();
        assert_eq!(names, vec!["{ABC}", "{DEF}"]);
        assert!(open_native(&format!("{}\\Nope", scratch.path)).is_none());
        let path = scratch.path.clone();
        drop(scratch);
        assert!(open_native(&path).is_none(), "the scratch key is removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both hives a transaction may write can be flushed by the process
    /// that wrote them: the user's own, and the machine's two hive files
    /// — opened for query, which needs no administrator — so that the
    /// commit's flush never fails on rights.
    #[test]
    fn every_hive_a_scope_writes_can_be_flushed() {
        flush(&Roots::real(), Hive::CurrentUser).unwrap();
        flush(&Roots::real(), Hive::LocalMachine).unwrap();
        let (roots, _) = test_roots();
        flush(&roots, Hive::LocalMachine).unwrap();
    }

    fn test_roots() -> (Roots, KeyPath) {
        let prefix = format!(
            "Software\\TigerSetupTests\\registry-{}-{}",
            std::process::id(),
            crate::report::unique_id()
        );
        (
            Roots::relocated(&prefix),
            KeyPath::parse("HKLM\\Software\\Probe").unwrap(),
        )
    }

    fn cleanup(roots: &Roots) {
        // The relocated tree lives under HKCU; remove it bottom-up.
        let mut key = KeyPath::parse("HKLM\\Software\\Probe\\Child").unwrap();
        loop {
            let _ = delete_key_if_empty(roots, &key);
            match key.parent() {
                Some(parent) => key = parent,
                None => break,
            }
        }
        let prefix = roots.prefix.clone().unwrap();
        let mut key = KeyPath::new(Hive::CurrentUser, prefix);
        loop {
            let _ = delete_key_if_empty(&Roots::real(), &key);
            match key.parent() {
                Some(parent) if !parent.subkey.eq_ignore_ascii_case("software") => key = parent,
                _ => break,
            }
        }
    }

    #[test]
    fn key_paths_parse_and_relate() {
        let key = KeyPath::parse("HKCU\\Software\\IT Tiger\\TestApp").unwrap();
        assert_eq!(key.hive, Hive::CurrentUser);
        assert_eq!(key.to_string(), "HKCU\\Software\\IT Tiger\\TestApp");
        assert_eq!(key.depth(), 3);
        assert_eq!(
            key.parent().unwrap().to_string(),
            "HKCU\\Software\\IT Tiger"
        );
        assert!(key.is_under(&KeyPath::parse("hkcu\\software").unwrap()));
        assert!(!key.is_under(&KeyPath::parse("HKLM\\Software").unwrap()));
        assert!(!KeyPath::parse("HKCU\\Software\\IT").unwrap().is_under(&key));
        for bad in ["Software\\x", "HKCU", "HKCU\\", "HKXX\\a", "HKCU\\\\a"] {
            assert_eq!(
                KeyPath::parse(bad).unwrap_err().code,
                "registry_path_invalid",
                "{bad}"
            );
        }
        let relocated = Roots::relocated("Software\\T");
        assert_eq!(
            relocated.physical(&key).1,
            "Software\\T\\HKCU\\Software\\IT Tiger\\TestApp"
        );
        assert!(!Roots::real().is_relocated());
    }

    #[test]
    fn data_round_trips_through_its_columns() {
        for data in [
            Data::String("a b".into()),
            Data::ExpandString("%X%\\y".into()),
            Data::Dword(4_000_000_000),
            Data::Raw {
                kind: 3,
                bytes: vec![1, 2, 255],
            },
        ] {
            let back = Data::from_columns(&data.kind_name(), &data.text()).unwrap();
            assert_eq!(back, data);
        }
        assert!(Data::from_columns("dword", "x").is_err());
        assert!(Data::from_columns("raw:3", "abc").is_err());
    }

    #[test]
    fn values_and_keys_round_trip_under_a_relocated_root() {
        let (roots, key) = test_roots();
        assert!(!key_exists(&roots, &key).unwrap());
        assert_eq!(read_value(&roots, &key, "v").unwrap(), None);
        create_key(&roots, &key).unwrap();
        create_key(&roots, &key).unwrap();
        assert!(key_exists(&roots, &key).unwrap());
        assert_eq!(key_is_empty(&roots, &key).unwrap(), Some(true));

        let expand = Data::ExpandString("%LOCALAPPDATA%\\x".into());
        write_value(&roots, &key, "v", &expand).unwrap();
        assert_eq!(read_value(&roots, &key, "v").unwrap(), Some(expand));
        write_value(&roots, &key, "n", &Data::Dword(7)).unwrap();
        assert_eq!(read_value(&roots, &key, "n").unwrap(), Some(Data::Dword(7)));
        let raw = Data::Raw {
            kind: 3,
            bytes: vec![9, 8, 7],
        };
        write_value(&roots, &key, "b", &raw).unwrap();
        assert_eq!(read_value(&roots, &key, "b").unwrap(), Some(raw));
        assert_eq!(key_is_empty(&roots, &key).unwrap(), Some(false));
        assert!(!delete_key_if_empty(&roots, &key).unwrap());

        let child = key.child("Child");
        create_key(&roots, &child).unwrap();
        for name in ["v", "n", "b"] {
            delete_value(&roots, &key, name).unwrap();
        }
        delete_value(&roots, &key, "missing").unwrap();
        assert!(
            !delete_key_if_empty(&roots, &key).unwrap(),
            "a subkey remains"
        );
        assert!(delete_key_if_empty(&roots, &child).unwrap());
        assert!(delete_key_if_empty(&roots, &key).unwrap());
        assert!(delete_key_if_empty(&roots, &key).unwrap(), "absent is gone");
        assert!(!key_exists(&roots, &key).unwrap());
        cleanup(&roots);
    }
}
