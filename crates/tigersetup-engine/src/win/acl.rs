//! Discretionary access control on a file or directory, expressed as SDDL.
//!
//! Three operations are enough for what the engine needs: replace an
//! object's DACL with an explicit one that inherits nothing ([`set_dacl`]),
//! read back the DACL that is actually on it ([`read_dacl`]), and compare
//! the two ([`Dacl`]).
//!
//! The comparison parses rather than matches text, because Windows writes a
//! DACL back in its own spelling: the access mask `0x1200a9` comes back as
//! `FRFX`, and the header gains the flags the object really has. Two lists
//! are the same when they grant the same rights to the same trustees with
//! the same inheritance.
//!
//! Ownership is part of the protection, not a detail beside it: an object's
//! owner keeps `WRITE_DAC` whatever its DACL says, so a directory a standard
//! user created is one that user can re-open at will. A descriptor whose
//! SDDL names an owner (`O:BA...`) therefore sets the owner too. The SACL is
//! never touched — auditing is the machine's policy, not an installer's.

use std::collections::BTreeSet;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
};

use crate::{Error, Result};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn failed(what: &str, path: &Path, code: u32) -> Error {
    Error::new(
        "state_directory_unprotected",
        format!(
            "cannot {what} the access control list of {}: {}",
            path.display(),
            std::io::Error::from_raw_os_error(code as i32)
        ),
    )
}

/// Replaces the object's DACL with the one `sddl` describes — a
/// `D:(A;...)...` string — and stops inheritance, so that the entries are
/// exactly the ones named and nothing is added by the parent directory's
/// inheritable rules.
pub fn set_dacl(path: &Path, sddl: &str) -> Result<()> {
    let text: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 || descriptor.is_null() {
        return Err(failed("build", path, unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }));
    }
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    let read =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) };
    let result = if read == 0 || present == 0 {
        Err(failed("build", path, unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }))
    } else {
        // An owner in the descriptor is set with the list. Taking ownership
        // is what stops the previous owner re-opening the object through
        // `WRITE_DAC`, so the two belong in one call.
        let mut owner: windows_sys::Win32::Security::PSID = std::ptr::null_mut();
        let mut owner_defaulted = 0;
        let has_owner = unsafe {
            GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) != 0
                && !owner.is_null()
        };
        let information = match has_owner {
            true => {
                DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION
                    | OWNER_SECURITY_INFORMATION
            }
            false => DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
        };
        let status = unsafe {
            SetNamedSecurityInfoW(
                wide(path).as_ptr(),
                SE_FILE_OBJECT,
                information,
                if has_owner {
                    owner
                } else {
                    std::ptr::null_mut()
                },
                std::ptr::null_mut(),
                dacl,
                std::ptr::null(),
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(failed("set", path, status))
        }
    };
    unsafe { LocalFree(descriptor as *mut _) };
    result
}

/// The object's current DACL as SDDL, without the owner, group or SACL,
/// so that it compares directly with what [`set_dacl`] was given.
pub fn read_dacl(path: &Path) -> Result<String> {
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(failed("read", path, status));
    }
    let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
    let mut length: u32 = 0;
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            &mut length,
        )
    };
    let result = if converted == 0 || text.is_null() {
        Err(failed("read", path, unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }))
    } else {
        let slice = unsafe { std::slice::from_raw_parts(text, length as usize) };
        Ok(String::from_utf16_lossy(slice))
    };
    if !text.is_null() {
        unsafe { LocalFree(text as *mut _) };
    }
    unsafe { LocalFree(descriptor as *mut _) };
    result
}

/// The object's owner, as an SDDL SID string (`S-1-5-32-544` and the like).
///
/// The owner matters because it is not constrained by the DACL: an owner may
/// always re-open the object for `WRITE_DAC` and give itself any right it
/// likes. A directory an installer protects but does not own is protected
/// only until its owner decides otherwise.
pub fn read_owner(path: &Path) -> Result<String> {
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(failed("read owner", path, status));
    }
    let mut text: windows_sys::core::PWSTR = std::ptr::null_mut();
    let mut length: u32 = 0;
    let converted = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            OWNER_SECURITY_INFORMATION,
            &mut text,
            &mut length,
        )
    };
    let result = if converted == 0 || text.is_null() {
        Err(failed("read owner", path, unsafe {
            windows_sys::Win32::Foundation::GetLastError()
        }))
    } else {
        let slice = unsafe { std::slice::from_raw_parts(text, length as usize) };
        // `O:<sid>`, with nothing after it because only the owner was asked for.
        Ok(String::from_utf16_lossy(slice)
            .trim_start_matches("O:")
            .trim()
            .to_string())
    };
    if !text.is_null() {
        unsafe { LocalFree(text as *mut _) };
    }
    unsafe { LocalFree(descriptor as *mut _) };
    result
}

/// One access control entry, as SDDL writes it:
/// `(<type>;<flags>;<rights>;<object>;<inherited object>;<trustee>)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ace {
    /// `A` for allow, `D` for deny.
    pub kind: String,
    /// The two-letter inheritance and audit flags, `OI` and `CI` among them.
    pub flags: BTreeSet<String>,
    pub rights: u32,
    /// The SID or its well-known abbreviation: `SY`, `BA`, `BU`.
    pub trustee: String,
}

impl Ace {
    /// Whether Windows placed this entry here by inheritance rather than
    /// the owner writing it.
    pub fn inherited(&self) -> bool {
        self.flags.contains("ID")
    }
}

/// A parsed discretionary access control list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dacl {
    /// `P`: the list does not inherit from the parent object.
    pub protected: bool,
    pub entries: Vec<Ace>,
}

/// The access masks SDDL abbreviates, so that `FRFX` and `0x1200a9` compare
/// equal.
const RIGHTS: &[(&str, u32)] = &[
    ("GA", 0x1000_0000),
    ("GR", 0x8000_0000),
    ("GW", 0x4000_0000),
    ("GX", 0x2000_0000),
    ("RC", 0x0002_0000),
    ("SD", 0x0001_0000),
    ("WD", 0x0004_0000),
    ("WO", 0x0008_0000),
    ("FA", 0x001F_01FF),
    ("FR", 0x0012_0089),
    ("FW", 0x0012_0116),
    ("FX", 0x0012_00A0),
    ("KA", 0x000F_003F),
    ("KR", 0x0002_0019),
    ("KW", 0x0002_0006),
    ("KX", 0x0002_0019),
];

fn parse_rights(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if text.chars().all(|c| c.is_ascii_digit()) {
        return text.parse().ok();
    }
    if text.is_empty() || !text.len().is_multiple_of(2) {
        return None;
    }
    let mut mask = 0;
    for pair in text.as_bytes().chunks(2) {
        let name = String::from_utf8_lossy(pair).to_ascii_uppercase();
        mask |= RIGHTS.iter().find(|(n, _)| *n == name)?.1;
    }
    Some(mask)
}

/// Splits a flags field such as `OICIID` into its two-letter tokens.
fn parse_flags(text: &str) -> BTreeSet<String> {
    text.as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).to_ascii_uppercase())
        .collect()
}

impl Dacl {
    /// Parses a `D:[flags](ace)(ace)...` string; `None` when it is not one.
    /// Reads the `D:` section of a security descriptor, which may be preceded
    /// by an owner and a group (`O:BAD:(A;...)`): the DACL section is the last
    /// `D:` before the first access control entry.
    pub fn parse(text: &str) -> Option<Dacl> {
        let text = text.trim();
        let first_entry = text.find('(').unwrap_or(text.len());
        let start = text[..first_entry].rfind("D:")?;
        let rest = &text[start + 2..];
        let header_end = rest.find('(').unwrap_or(rest.len());
        let protected = rest[..header_end].to_ascii_uppercase().contains('P');
        let mut entries = Vec::new();
        let mut tail = &rest[header_end..];
        while let Some(open) = tail.find('(') {
            let close = tail[open..].find(')')? + open;
            let fields: Vec<&str> = tail[open + 1..close].split(';').collect();
            if fields.len() < 6 {
                return None;
            }
            entries.push(Ace {
                kind: fields[0].to_ascii_uppercase(),
                flags: parse_flags(fields[1]),
                rights: parse_rights(fields[2])?,
                trustee: fields[5].to_ascii_uppercase(),
            });
            tail = &tail[close + 1..];
        }
        entries.sort();
        Some(Dacl { protected, entries })
    }

    /// Whether this list is exactly `wanted`: protected, carrying no
    /// inherited entry, and granting the same rights with the same
    /// inheritance to the same trustees.
    pub fn is(&self, wanted: &Dacl) -> bool {
        self.protected && !self.entries.iter().any(Ace::inherited) && self.entries == wanted.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DACL round-trips through Windows on a directory the test owns,
    /// which needs no privilege beyond owning it.
    #[test]
    fn a_directory_dacl_is_written_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state");
        std::fs::create_dir(&path).unwrap();
        // SYSTEM and Administrators full control, Users read and execute.
        let sddl = "D:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
        set_dacl(&path, sddl).unwrap();
        let read = read_dacl(&path).unwrap();
        let actual = Dacl::parse(&read).unwrap_or_else(|| panic!("{read} parses"));
        assert!(
            actual.is(&Dacl::parse(sddl).unwrap()),
            "{read} is the list that was written"
        );
    }

    #[test]
    fn rights_and_flags_compare_by_meaning_rather_than_spelling() {
        let written = Dacl::parse("D:(A;OICI;FA;;;SY)(A;OICI;0x1200a9;;;BU)").unwrap();
        let read_back = Dacl::parse("D:PAI(A;OICI;FRFX;;;BU)(A;OICI;FA;;;SY)").unwrap();
        assert!(read_back.is(&written), "FRFX is 0x1200a9, in any order");

        // An unprotected list still inherits from the parent directory.
        assert!(
            !Dacl::parse("D:(A;OICI;FA;;;SY)(A;OICI;FRFX;;;BU)")
                .unwrap()
                .is(&written)
        );
        // An inherited entry means the list is not the one that was set.
        assert!(
            !Dacl::parse("D:PAI(A;OICIID;FA;;;SY)(A;OICI;FRFX;;;BU)")
                .unwrap()
                .is(&written)
        );
        // A trustee that was not granted anything, or one that is missing.
        assert!(
            !Dacl::parse("D:P(A;OICI;FA;;;SY)(A;OICI;FRFX;;;BU)(A;OICI;FA;;;WD)")
                .unwrap()
                .is(&written)
        );
        assert!(!Dacl::parse("D:P(A;OICI;FA;;;SY)").unwrap().is(&written));
        // The same trustee with more rights than it was granted.
        assert!(
            !Dacl::parse("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BU)")
                .unwrap()
                .is(&written)
        );
        // Different inheritance is a different list: an entry that does not
        // reach the database file protects nothing.
        assert!(
            !Dacl::parse("D:P(A;OICI;FA;;;SY)(A;;FRFX;;;BU)")
                .unwrap()
                .is(&written)
        );
    }

    #[test]
    fn a_string_that_is_not_a_dacl_is_refused() {
        assert!(Dacl::parse("O:BAG:BA").is_none());
        assert!(Dacl::parse("D:(A;OICI;NOPE;;;SY)").is_none());
        assert!(Dacl::parse("D:(A;OICI;FA;;;SY").is_none());
        assert!(Dacl::parse("D:(A;OICI;FA;SY)").is_none());
        assert_eq!(Dacl::parse("D:NO_ACCESS_CONTROL").unwrap().entries.len(), 0);
    }

    #[test]
    fn a_missing_directory_is_an_error_rather_than_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert_eq!(
            read_dacl(&missing).unwrap_err().code,
            "state_directory_unprotected"
        );
        assert_eq!(
            set_dacl(&missing, "D:(A;OICI;FA;;;BA)").unwrap_err().code,
            "state_directory_unprotected"
        );
    }
}
