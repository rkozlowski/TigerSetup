//! The compiled-executable metadata provider: the Windows `VERSIONINFO`
//! resource of a built binary (`TigerSetup-Design.md` §9.2). It keeps
//! TigerSetup useful for C++, Rust and third-party binaries that have no
//! MSBuild metadata, and it is the validation input that catches a stale
//! executable before it becomes a release artifact.
//!
//! `GetFileVersionInfoSizeW` / `GetFileVersionInfoW` / `VerQueryValueW` are
//! the only way to read the resource the way Explorer does. The resource's
//! own translation table drives the string-block lookup rather than a
//! hard-coded language, with the two conventional English blocks as a
//! fallback for binaries whose table is missing.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
};

use crate::{BuildError, Result};

/// The `VERSIONINFO` string names this provider reads.
pub const PRODUCT_VERSION: &str = "ProductVersion";
pub const PRODUCT_NAME: &str = "ProductName";
pub const COMPANY_NAME: &str = "CompanyName";
pub const COMMENTS: &str = "Comments";
pub const FILE_DESCRIPTION: &str = "FileDescription";
pub const LEGAL_COPYRIGHT: &str = "LegalCopyright";
pub const ORIGINAL_FILENAME: &str = "OriginalFilename";
pub const INTERNAL_NAME: &str = "InternalName";
pub const FILE_VERSION: &str = "FileVersion";

/// What a binary's version resource says. Every string is empty when the
/// resource does not carry it.
#[derive(Debug, Clone, Default)]
pub struct VersionInfo {
    pub path: PathBuf,
    /// Product version as written, with SemVer build metadata still attached.
    pub product_version: String,
    pub product_name: String,
    pub company_name: String,
    /// The .NET SDK writes `<Description>` here.
    pub comments: String,
    pub file_description: String,
    pub legal_copyright: String,
    pub original_filename: String,
    pub internal_name: String,
    /// Four-part file version from the fixed information block.
    pub file_version: String,
    /// The `FileVersion` string as written, which need not be numeric.
    pub file_version_string: String,
    /// The language/code-page block the strings were read from, `lllcccc`
    /// hex as `VERSIONINFO` names it.
    pub translation: String,
}

impl VersionInfo {
    /// The product description: `Comments` where the resource has one,
    /// otherwise `FileDescription`.
    pub fn description(&self) -> &str {
        if self.comments.is_empty() {
            &self.file_description
        } else {
            &self.comments
        }
    }
}

/// Everything before the first `+`: a SemVer informational version carries
/// build metadata that is not part of the package version
/// (`TigerSetup-Design.md` §9.5).
pub fn without_build_metadata(version: &str) -> &str {
    version.split('+').next().unwrap_or(version).trim()
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Reads the version resource of `path`.
pub fn read(path: &Path) -> Result<VersionInfo> {
    if !path.is_file() {
        return Err(BuildError::new(
            "metadata_source_unreadable",
            format!("the executable {} does not exist", path.display()),
        ));
    }
    let file = wide(&path.display().to_string());
    let mut handle = 0u32;
    let size = unsafe { GetFileVersionInfoSizeW(file.as_ptr(), &mut handle) };
    if size == 0 {
        return Err(BuildError::new(
            "version_info_missing",
            format!(
                "{} carries no VERSIONINFO resource (error {})",
                path.display(),
                std::io::Error::last_os_error()
            ),
        ));
    }
    let mut block = vec![0u8; size as usize];
    if unsafe { GetFileVersionInfoW(file.as_ptr(), 0, size, block.as_mut_ptr().cast()) } == 0 {
        return Err(BuildError::new(
            "version_info_missing",
            format!(
                "the VERSIONINFO resource of {} cannot be read (error {})",
                path.display(),
                std::io::Error::last_os_error()
            ),
        ));
    }

    let mut info = VersionInfo {
        path: path.to_path_buf(),
        ..VersionInfo::default()
    };
    if let Some(fixed) = fixed_info(&block) {
        info.file_version = format!(
            "{}.{}.{}.{}",
            fixed.dwFileVersionMS >> 16,
            fixed.dwFileVersionMS & 0xffff,
            fixed.dwFileVersionLS >> 16,
            fixed.dwFileVersionLS & 0xffff
        );
    }
    // The resource's own translation table first; the two conventional
    // English blocks only as a fallback, so a localized resource is still
    // read from the block it actually declares.
    let mut candidates = translations(&block);
    candidates.push((0x0409, 0x04b0));
    candidates.push((0x0409, 0x04e4));
    for (language, code_page) in candidates {
        let prefix = format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\");
        let product_version = string_value(&block, &prefix, PRODUCT_VERSION);
        let product_name = string_value(&block, &prefix, PRODUCT_NAME);
        let company_name = string_value(&block, &prefix, COMPANY_NAME);
        let comments = string_value(&block, &prefix, COMMENTS);
        let file_description = string_value(&block, &prefix, FILE_DESCRIPTION);
        let legal_copyright = string_value(&block, &prefix, LEGAL_COPYRIGHT);
        let original_filename = string_value(&block, &prefix, ORIGINAL_FILENAME);
        let internal_name = string_value(&block, &prefix, INTERNAL_NAME);
        let file_version_string = string_value(&block, &prefix, FILE_VERSION);
        if product_version.is_empty()
            && product_name.is_empty()
            && company_name.is_empty()
            && comments.is_empty()
            && file_description.is_empty()
            && legal_copyright.is_empty()
            && original_filename.is_empty()
            && internal_name.is_empty()
            && file_version_string.is_empty()
        {
            continue;
        }
        info.translation = format!("{language:04x}{code_page:04x}");
        info.product_version = product_version;
        info.product_name = product_name;
        info.company_name = company_name;
        info.comments = comments;
        info.file_description = file_description;
        info.legal_copyright = legal_copyright;
        info.original_filename = original_filename;
        info.internal_name = internal_name;
        info.file_version_string = file_version_string;
        break;
    }
    Ok(info)
}

fn fixed_info(block: &[u8]) -> Option<VS_FIXEDFILEINFO> {
    let root = wide("\\");
    let mut pointer = std::ptr::null_mut();
    let mut length = 0u32;
    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            root.as_ptr(),
            &mut pointer,
            &mut length,
        )
    };
    if ok == 0 || pointer.is_null() || (length as usize) < size_of::<VS_FIXEDFILEINFO>() {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(pointer.cast::<VS_FIXEDFILEINFO>()) })
}

/// The `\VarFileInfo\Translation` table as (language, code page) pairs.
fn translations(block: &[u8]) -> Vec<(u16, u16)> {
    let key = wide("\\VarFileInfo\\Translation");
    let mut pointer = std::ptr::null_mut();
    let mut length = 0u32;
    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            key.as_ptr(),
            &mut pointer,
            &mut length,
        )
    };
    if ok == 0 || pointer.is_null() {
        return Vec::new();
    }
    let count = length as usize / 4;
    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let pair = unsafe { std::ptr::read_unaligned(pointer.cast::<u16>().add(index * 2)) };
        let code_page =
            unsafe { std::ptr::read_unaligned(pointer.cast::<u16>().add(index * 2 + 1)) };
        out.push((pair, code_page));
    }
    out
}

fn string_value(block: &[u8], prefix: &str, name: &str) -> String {
    let key = wide(&format!("{prefix}{name}"));
    let mut pointer = std::ptr::null_mut();
    let mut length = 0u32;
    let ok = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            key.as_ptr(),
            &mut pointer,
            &mut length,
        )
    };
    if ok == 0 || pointer.is_null() || length == 0 {
        return String::new();
    }
    // `length` counts characters and includes the terminator when present.
    let mut characters = Vec::with_capacity(length as usize);
    for index in 0..length as usize {
        let unit = unsafe { std::ptr::read_unaligned(pointer.cast::<u16>().add(index)) };
        if unit == 0 {
            break;
        }
        characters.push(unit);
    }
    String::from_utf16_lossy(&characters).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbox_executable() -> PathBuf {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("notepad.exe")
    }

    #[test]
    fn a_windows_executable_yields_its_whole_string_block() {
        let path = inbox_executable();
        let info = read(&path).unwrap();
        assert!(
            info.product_name.contains("Windows"),
            "ProductName was {:?}",
            info.product_name
        );
        assert_eq!(info.company_name, "Microsoft Corporation");
        assert!(
            !info.file_description.is_empty(),
            "FileDescription was empty"
        );
        assert!(!info.legal_copyright.is_empty(), "LegalCopyright was empty");
        assert_eq!(
            info.product_version.matches('.').count(),
            3,
            "ProductVersion was {:?}",
            info.product_version
        );
        assert_eq!(
            info.file_version.matches('.').count(),
            3,
            "the fixed file version was {:?}",
            info.file_version
        );
        assert_eq!(info.translation.len(), 8, "{:?}", info.translation);
        assert!(
            !info.original_filename.is_empty(),
            "OriginalFilename was empty"
        );
        assert!(!info.internal_name.is_empty(), "InternalName was empty");
        // An inbox binary's FileVersion string need not agree with its fixed
        // block (Notepad says "10.0.x (WinBuild...)" over a 6.2.x block), so
        // the string is only required to be there.
        assert!(
            !info.file_version_string.is_empty(),
            "FileVersion string was empty"
        );
        // No Comments in an inbox binary, so the description falls back.
        assert_eq!(info.description(), info.file_description);
    }

    #[test]
    fn a_file_without_a_version_resource_is_reported_as_such() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.exe");
        std::fs::write(&path, b"not a portable executable").unwrap();
        assert_eq!(read(&path).unwrap_err().code, "version_info_missing");
        assert_eq!(
            read(&dir.path().join("absent.exe")).unwrap_err().code,
            "metadata_source_unreadable"
        );
    }

    #[test]
    fn build_metadata_is_not_part_of_the_package_version() {
        assert_eq!(without_build_metadata("0.8.1+20260907.175518"), "0.8.1");
        assert_eq!(without_build_metadata("1.2.3"), "1.2.3");
        assert_eq!(without_build_metadata(" 1.2.3 "), "1.2.3");
    }
}
