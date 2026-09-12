//! The Windows file version of an executable or DLL (`VS_FIXEDFILEINFO`),
//! as `major.minor.build.revision`.

use std::ffi::c_void;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `None` when the file is absent or carries no version resource.
pub fn file_version(path: &Path) -> Option<String> {
    let path_w = wide(&path.display().to_string());
    let mut handle: u32 = 0;
    let size = unsafe { GetFileVersionInfoSizeW(path_w.as_ptr(), &mut handle) };
    if size == 0 {
        return None;
    }
    let mut block = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(path_w.as_ptr(), 0, size, block.as_mut_ptr() as *mut c_void) }
        == 0
    {
        return None;
    }
    let root = wide("\\");
    let mut info: *mut c_void = std::ptr::null_mut();
    let mut length: u32 = 0;
    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr() as *const c_void,
            root.as_ptr(),
            &mut info,
            &mut length,
        )
    };
    if ok == 0 || info.is_null() || (length as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    let fixed = unsafe { std::ptr::read_unaligned(info as *const VS_FIXEDFILEINFO) };
    if fixed.dwSignature != 0xFEEF_04BD {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        fixed.dwFileVersionMS >> 16,
        fixed.dwFileVersionMS & 0xFFFF,
        fixed.dwFileVersionLS >> 16,
        fixed.dwFileVersionLS & 0xFFFF
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_system_dll_has_a_four_part_version_and_a_text_file_has_none() {
        let system32 =
            std::path::PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32");
        let version = file_version(&system32.join("kernel32.dll")).unwrap();
        let parts: Vec<u32> = version.split('.').map(|p| p.parse().unwrap()).collect();
        assert_eq!(parts.len(), 4);
        assert!(parts.iter().any(|p| *p > 0), "{version}");
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("plain.txt");
        std::fs::write(&text, b"not a PE").unwrap();
        assert_eq!(file_version(&text), None);
        assert_eq!(file_version(&dir.path().join("missing.dll")), None);
    }
}
