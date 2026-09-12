//! Known folders and the environment-change broadcast, with the seams the
//! test suite uses so that it never touches the machine's real folders or
//! hives.
//!
//! Every root the engine resolves goes through [`known_folder`], and every
//! shell known folder resolves the same way, in this order:
//!
//! 1. `TIGERSETUP_TEST_FOLDER_<NAME>` (for example
//!    `TIGERSETUP_TEST_FOLDER_PROGRAMDATA`), the documented test seam: an
//!    isolated machine sets one per folder and nothing it runs can reach a
//!    real one.
//! 2. `SHGetKnownFolderPath`, which is what production uses. The shell API
//!    comes before the environment on purpose: `%ProgramFiles%` and
//!    `%ProgramData%` are ordinary, writable environment variables, and a
//!    machine-scope run that trusts them would install wherever the calling
//!    environment pointed.
//! 3. The standard environment variable, so that a session whose shell
//!    service is unavailable still resolves the folder.
//!
//! `%TEMP%` is not a shell known folder: it is the environment's, and is
//! read from `TEMP` then `TMP`.
//!
//! Registry roots are relocated by `TIGERSETUP_TEST_REGISTRY_ROOT`
//! (`crate::win::registry::TEST_ROOT_VARIABLE`): every `HKCU\...` and
//! `HKLM\...` the engine touches lives under `HKCU\<value>\HKCU\...` and
//! `HKCU\<value>\HKLM\...` while it is set. The `WM_SETTINGCHANGE`
//! broadcast is skipped then, because the real environment did not change.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::ptr;

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_LocalAppData, FOLDERID_ProgramData,
    FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, FOLDERID_Programs, FOLDERID_PublicDesktop,
    SHGetKnownFolderPath,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SMTO_NOTIMEOUTIFNOTHUNG, SendMessageTimeoutW,
    WM_SETTINGCHANGE,
};
use windows_sys::core::GUID;

/// Prefix of the environment variable that overrides one known folder.
pub const TEST_FOLDER_PREFIX: &str = "TIGERSETUP_TEST_FOLDER_";

/// The shell known folders the engine resolves: the placeholder name, the
/// folder id, and the environment variable that stands in for it when the
/// shell cannot answer.
const KNOWN_FOLDERS: &[(&str, &GUID, &str)] = &[
    ("LOCALAPPDATA", &FOLDERID_LocalAppData, "LOCALAPPDATA"),
    ("PROGRAMDATA", &FOLDERID_ProgramData, "PROGRAMDATA"),
    ("PROGRAMFILES", &FOLDERID_ProgramFiles, "PROGRAMFILES"),
    (
        "PROGRAMFILESX86",
        &FOLDERID_ProgramFilesX86,
        "PROGRAMFILES(X86)",
    ),
    ("DESKTOP", &FOLDERID_Desktop, ""),
    ("PROGRAMS", &FOLDERID_Programs, ""),
    ("COMMONDESKTOP", &FOLDERID_PublicDesktop, ""),
    ("COMMONPROGRAMS", &FOLDERID_CommonPrograms, ""),
];

/// Resolves a `%NAME%` placeholder used by the identity and resource
/// templates.
pub fn known_folder(name: &str) -> Option<String> {
    let upper = name.to_ascii_uppercase();
    let value = match upper.as_str() {
        "TEMP" => std::env::var_os("TEMP").or_else(|| std::env::var_os("TMP")),
        _ => {
            let (_, id, variable) = KNOWN_FOLDERS.iter().find(|(n, _, _)| *n == upper)?;
            shell_folder(&upper, id).or_else(|| {
                (!variable.is_empty())
                    .then(|| std::env::var_os(variable))
                    .flatten()
            })
        }
    }?;
    let value = value.to_str()?.trim_end_matches('\\').to_string();
    (!value.is_empty()).then_some(value)
}

/// A shell known folder, or its test override.
fn shell_folder(name: &str, id: &GUID) -> Option<OsString> {
    if let Some(value) = std::env::var_os(format!("{TEST_FOLDER_PREFIX}{name}")) {
        return Some(value);
    }
    let mut path: *mut u16 = ptr::null_mut();
    let result = unsafe { SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut path) };
    if result < 0 || path.is_null() {
        return None;
    }
    let mut length = 0;
    while unsafe { *path.add(length) } != 0 {
        length += 1;
    }
    let value = OsString::from_wide(unsafe { std::slice::from_raw_parts(path, length) });
    unsafe { CoTaskMemFree(path as *const _) };
    Some(value)
}

/// Tells running applications that the environment changed, so a new
/// console picks up the PATH. Best effort with a short timeout; skipped
/// while the registry seam relocates the environment key.
pub fn broadcast_environment_change() -> bool {
    if crate::win::registry::Roots::from_env().is_relocated() {
        return false;
    }
    let section: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            section.as_ptr() as isize,
            SMTO_ABORTIFHUNG | SMTO_NOTIMEOUTIFNOTHUNG,
            1000,
            &mut result,
        )
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_folder_resolves_and_the_seam_overrides_it() {
        for (name, _, _) in KNOWN_FOLDERS {
            if std::env::var_os(format!("{TEST_FOLDER_PREFIX}{name}")).is_none() {
                let folder = known_folder(name).unwrap_or_default();
                assert!(folder.len() > 3, "{name} resolves to {folder:?}");
            }
        }
        assert!(known_folder("NOPE").is_none());
        assert!(known_folder("TEMP").is_some());
        // The placeholder is case-insensitive, and no value keeps a
        // trailing separator.
        let data = known_folder("programdata").unwrap();
        assert!(!data.ends_with('\\'), "{data:?}");
    }
}
