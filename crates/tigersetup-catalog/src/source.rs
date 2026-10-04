//! Authenticating the pre-indexed source package (`source2.msix`) before
//! anything inside it is believed. These are the checks `winget.exe` makes
//! on the same package, in the same order:
//!
//! 1. **Signature.** The package bytes are verified by `WinVerifyTrust`
//!    (the AppX subject interface package hashes every ZIP record, the
//!    central directory, the block map and the content types), and the
//!    signer's chain must satisfy the Microsoft-root policy with the
//!    application-root flag: a Microsoft product root, not merely any
//!    trusted root.
//! 2. **Content.** The ZIP must not hold two entries with one name (a ZIP
//!    reader keeps one of them, so which one it read would depend on the
//!    reader), and every file read out of it is checked against the signed
//!    `AppxBlockMap.xml`: 64 KiB blocks, each with its SHA-256, and the
//!    file's size. What is believed is then exactly what was signed, however
//!    the ZIP reader resolved the archive.
//! 3. **Identity.** `AppxManifest.xml` must name the package
//!    `Microsoft.Winget.Source` with the Microsoft publisher, so another
//!    Microsoft-signed package cannot stand in for the index.
//! 4. **Rollback bound.** The identity version encodes the publication time
//!    (`yyyy.MMdd.HHmm.n`); an index published before the caller's bound is
//!    refused, so a replayed old index — validly signed when it was current —
//!    cannot pin a machine to old installer metadata. The encoding is a
//!    convention of the publisher rather than a documented contract, so a
//!    version that does not read as a time is judged by the first three
//!    checks alone instead of failing every resolution.
//!
//! Revocation is deliberately not checked (`WTD_REVOKE_NONE`) and no URL is
//! retrieved while the chain is built: the signing certificate lives for a
//! few days and the signature is timestamped, so revocation fetching adds no
//! protection worth a network round trip, and on a firewalled or offline
//! machine it would turn a valid index into a failure.

use std::collections::HashSet;
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom, CERT_CHAIN_POLICY_MICROSOFT_ROOT,
    CERT_CHAIN_POLICY_PARA, CERT_CHAIN_POLICY_STATUS, CertVerifyCertificateChainPolicy,
    MICROSOFT_ROOT_CERT_CHAIN_POLICY_CHECK_APPLICATION_ROOT_FLAG,
};
use windows_sys::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
    WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvSignerFromChain,
    WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_DELETE_ON_CLOSE, FILE_SHARE_READ};

use crate::{CatalogError, Reason, Result};

/// The package name the WinGet community source is published under.
pub const SOURCE_NAME: &str = "Microsoft.Winget.Source";
/// The publisher it is signed and declared by.
pub const SOURCE_PUBLISHER: &str =
    "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US";

const MANIFEST_ENTRY: &str = "AppxManifest.xml";
const BLOCK_MAP_ENTRY: &str = "AppxBlockMap.xml";
/// The only hash method a block map may declare.
const BLOCK_MAP_HASH_METHOD: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
/// The uncompressed size of every block but a file's last.
const BLOCK_LEN: usize = 64 << 10;
const MANIFEST_MAX_LEN: u64 = 1 << 20;
const BLOCK_MAP_MAX_LEN: u64 = 16 << 20;

/// What an authenticated source package says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    /// `Identity/@Version`, e.g. `2026.1004.1308.18`.
    pub version: String,
    /// The publication time the version encodes, Unix seconds; `None` for a
    /// version that does not read as one.
    pub published_at: Option<i64>,
}

fn signature_invalid(message: impl Into<String>) -> CatalogError {
    CatalogError::new(Reason::CatalogSignatureInvalid, message)
}

fn integrity(message: impl Into<String>) -> CatalogError {
    CatalogError::new(Reason::CatalogIntegrityFailure, message)
}

fn identity_mismatch(message: impl Into<String>) -> CatalogError {
    CatalogError::new(Reason::CatalogIdentityMismatch, message)
}

fn unavailable(message: impl Into<String>) -> CatalogError {
    CatalogError::new(Reason::CatalogUnavailable, message)
}

// --- Signature -------------------------------------------------------------

/// A temporary copy of bytes that only this handle can change: created
/// exclusively under an unpredictable name, shared for reading only, and
/// deleted by the system when the handle closes — on every path, a crash
/// included.
pub(crate) struct SealedFile {
    file: File,
    path: PathBuf,
}

impl SealedFile {
    pub(crate) fn create(bytes: &[u8]) -> Result<SealedFile> {
        let mut random = [0u8; 16];
        // SAFETY: the buffer is valid for its length; no algorithm handle is
        // needed with the system-preferred generator.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                random.as_mut_ptr(),
                random.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(unavailable(format!(
                "cannot name a temporary file: BCryptGenRandom 0x{status:08x}"
            )));
        }
        let path = std::env::temp_dir().join(format!(
            "tigersetup-source-{}.msix",
            tigersetup_format::hex(&random)
        ));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&path)
            .map_err(|err| unavailable(format!("cannot create {}: {err}", path.display())))?;
        file.write_all(bytes)
            .and_then(|()| file.seek(SeekFrom::Start(0)).map(drop))
            .map_err(|err| unavailable(format!("cannot write {}: {err}", path.display())))?;
        Ok(SealedFile { file, path })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// Verifies the package's Authenticode signature and that its signer chains
/// to a Microsoft application root. `msix` is written once to a
/// [`SealedFile`] and verified through that handle, so the bytes verified are
/// the bytes the caller goes on to read.
pub fn verify_signature(msix: &[u8]) -> Result<()> {
    let sealed = SealedFile::create(msix)?;
    let wide: Vec<u16> = sealed
        .path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: wide.as_ptr(),
        hFile: sealed.file.as_raw_handle(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` and everything it points at outlive both calls; the
    // state the first call opens is closed by the second whatever it found.
    let verdict = unsafe {
        let status = WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast::<c_void>(),
        );
        let verdict = if status != 0 {
            Err(signature_invalid(format!(
                "the source package signature does not verify: WinVerifyTrust 0x{:08x}",
                status as u32
            )))
        } else {
            microsoft_root(data.hWVTStateData)
        };
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(
            INVALID_HANDLE_VALUE,
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast::<c_void>(),
        );
        verdict
    };
    drop(sealed);
    verdict
}

/// The Microsoft-root policy, application roots only, over the chain
/// `WinVerifyTrust` built for the first signer.
///
/// # Safety
/// `state` is the open state of a successful `WTD_STATEACTION_VERIFY`.
unsafe fn microsoft_root(state: *mut c_void) -> Result<()> {
    // SAFETY: per the contract above; every pointer is checked before use.
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);
        if provider.is_null() {
            return Err(signature_invalid(
                "the signature verification left no state",
            ));
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, 0, 0);
        if signer.is_null() || (*signer).pChainContext.is_null() {
            return Err(signature_invalid("the source package signer has no chain"));
        }
        let para = CERT_CHAIN_POLICY_PARA {
            cbSize: std::mem::size_of::<CERT_CHAIN_POLICY_PARA>() as u32,
            dwFlags: MICROSOFT_ROOT_CERT_CHAIN_POLICY_CHECK_APPLICATION_ROOT_FLAG,
            pvExtraPolicyPara: std::ptr::null_mut(),
        };
        let mut status = CERT_CHAIN_POLICY_STATUS {
            cbSize: std::mem::size_of::<CERT_CHAIN_POLICY_STATUS>() as u32,
            ..Default::default()
        };
        let ok = CertVerifyCertificateChainPolicy(
            CERT_CHAIN_POLICY_MICROSOFT_ROOT,
            (*signer).pChainContext,
            &para,
            &mut status,
        );
        if ok == 0 || status.dwError != 0 {
            return Err(signature_invalid(format!(
                "the source package is not signed under a Microsoft application root: policy 0x{:08x}",
                status.dwError
            )));
        }
    }
    Ok(())
}

// --- Content ---------------------------------------------------------------

/// The package's files as the signature covers them: opened once, its
/// central directory free of duplicate names, its block map loaded.
pub struct SourceContent {
    archive: zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
    block_map: String,
}

impl SourceContent {
    /// Opens the package bytes. Does not verify the signature; that is
    /// [`verify_signature`] over the same bytes.
    pub fn open(msix: &[u8]) -> Result<SourceContent> {
        let names = central_directory_names(msix)?;
        let mut seen = HashSet::new();
        for name in &names {
            // AppX part names compare case-insensitively, either separator.
            if !seen.insert(name.replace('\\', "/").to_ascii_lowercase()) {
                return Err(integrity(format!(
                    "the source package holds {name:?} more than once"
                )));
            }
        }
        let archive = zip::ZipArchive::new(std::io::Cursor::new(msix.to_vec()))
            .map_err(|err| unavailable(format!("the source package is not a ZIP: {err}")))?;
        if archive.len() != names.len() {
            return Err(integrity(format!(
                "the source package lists {} entries; the ZIP reader sees {}",
                names.len(),
                archive.len()
            )));
        }
        let mut content = SourceContent {
            archive,
            block_map: String::new(),
        };
        let block_map = content.raw_entry(BLOCK_MAP_ENTRY, BLOCK_MAP_MAX_LEN)?;
        content.block_map = String::from_utf8(block_map)
            .map_err(|err| integrity(format!("{BLOCK_MAP_ENTRY} is not UTF-8: {err}")))?;
        let root = start_tags(&content.block_map, "BlockMap")
            .next()
            .ok_or_else(|| integrity(format!("{BLOCK_MAP_ENTRY} has no BlockMap element")))?;
        let method = attribute(root, "HashMethod")?;
        if method.as_deref() != Some(BLOCK_MAP_HASH_METHOD) {
            return Err(integrity(format!(
                "{BLOCK_MAP_ENTRY} declares hash method {method:?}"
            )));
        }
        Ok(content)
    }

    fn raw_entry(&mut self, name: &str, max_len: u64) -> Result<Vec<u8>> {
        let mut entry = self
            .archive
            .by_name(name)
            .map_err(|err| integrity(format!("the source package has no {name}: {err}")))?;
        if entry.size() > max_len {
            return Err(integrity(format!("{name} is {} bytes", entry.size())));
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .by_ref()
            .take(max_len + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| integrity(format!("cannot read {name}: {err}")))?;
        Ok(bytes)
    }

    /// One file's bytes, verified against the block map.
    pub fn entry(&mut self, name: &str, max_len: u64) -> Result<Vec<u8>> {
        let bytes = self.raw_entry(name, max_len)?;
        verify_block_map(&self.block_map, name, &bytes)?;
        Ok(bytes)
    }

    /// The package identity, checked against the WinGet source's.
    pub fn identity(&mut self) -> Result<SourceIdentity> {
        let manifest = self.entry(MANIFEST_ENTRY, MANIFEST_MAX_LEN)?;
        let manifest = String::from_utf8(manifest)
            .map_err(|err| identity_mismatch(format!("{MANIFEST_ENTRY} is not UTF-8: {err}")))?;
        parse_identity(&manifest)
    }
}

/// Reads one little-endian integer of `N` bytes at `at`.
fn le<const N: usize>(bytes: &[u8], at: usize) -> Option<u64> {
    let slice = bytes.get(at..at.checked_add(N)?)?;
    Some(
        slice
            .iter()
            .rev()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b)),
    )
}

/// Every entry name in the ZIP central directory, in order, read directly:
/// a ZIP reader keeps only one of two entries sharing a name, so it cannot
/// say whether there were two.
pub(crate) fn central_directory_names(zip: &[u8]) -> Result<Vec<String>> {
    let malformed = |what: &str| integrity(format!("the source package {what}"));
    const EOCD: u64 = 0x0605_4b50;
    const LOCATOR: u64 = 0x0706_4b50;
    const EOCD64: u64 = 0x0606_4b50;
    const HEADER: u64 = 0x0201_4b50;
    let floor = zip.len().saturating_sub(22 + 0xffff);
    let eocd = (floor..=zip.len().saturating_sub(22))
        .rev()
        .find(|&at| {
            le::<4>(zip, at) == Some(EOCD)
                && le::<2>(zip, at + 20).map(|c| at + 22 + c as usize) == Some(zip.len())
        })
        .ok_or_else(|| malformed("has no end of central directory"))?;
    let mut count = le::<2>(zip, eocd + 10).unwrap_or(0);
    let mut offset = le::<4>(zip, eocd + 16).unwrap_or(0);
    if count == 0xffff || offset == 0xffff_ffff {
        let locator = eocd
            .checked_sub(20)
            .filter(|&at| le::<4>(zip, at) == Some(LOCATOR))
            .ok_or_else(|| malformed("has no ZIP64 locator"))?;
        let record = le::<8>(zip, locator + 8).unwrap_or(u64::MAX) as usize;
        if le::<4>(zip, record) != Some(EOCD64) {
            return Err(malformed("has no ZIP64 end of central directory"));
        }
        count = le::<8>(zip, record + 32).unwrap_or(u64::MAX);
        offset = le::<8>(zip, record + 48).unwrap_or(u64::MAX);
    }
    let mut at = usize::try_from(offset).map_err(|_| malformed("is too large"))?;
    let mut names = Vec::new();
    for _ in 0..count {
        if le::<4>(zip, at) != Some(HEADER) {
            return Err(malformed("has a truncated central directory"));
        }
        let field = |rel: usize| le::<2>(zip, at + rel).unwrap_or(0) as usize;
        let (name_len, extra_len, comment_len) = (field(28), field(30), field(32));
        let name = zip
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(|| malformed("has a truncated entry name"))?;
        names.push(String::from_utf8_lossy(name).into_owned());
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(names)
}

// --- Block map -------------------------------------------------------------

/// Checks `bytes` against the block map's `<File>` element for `name` (a ZIP
/// entry name; the block map writes it with backslashes): the size, the
/// number of 64 KiB blocks, and each block's SHA-256.
pub fn verify_block_map(block_map: &str, name: &str, bytes: &[u8]) -> Result<()> {
    let wanted = name.replace('/', "\\");
    let block_map = strip_comments(block_map);
    let mut found = None;
    for (tag, body) in elements(&block_map, "File") {
        if attribute(tag, "Name")?.as_deref() == Some(wanted.as_str()) {
            if found.is_some() {
                return Err(integrity(format!("the block map lists {wanted} twice")));
            }
            found = Some((tag, body));
        }
    }
    let (tag, body) =
        found.ok_or_else(|| integrity(format!("the block map does not list {wanted}")))?;
    let size: u64 = attribute(tag, "Size")?
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| integrity(format!("the block map gives {wanted} no size")))?;
    if size != bytes.len() as u64 {
        return Err(integrity(format!(
            "{wanted} is {} bytes; the block map says {size}",
            bytes.len()
        )));
    }
    let hashes = start_tags(body, "Block")
        .map(|block| {
            attribute(block, "Hash")?
                .as_deref()
                .and_then(base64_decode)
                .filter(|hash| hash.len() == 32)
                .ok_or_else(|| integrity(format!("the block map has a malformed {wanted} block")))
        })
        .collect::<Result<Vec<_>>>()?;
    let chunks: Vec<&[u8]> = bytes.chunks(BLOCK_LEN).collect();
    if hashes.len() != chunks.len() {
        return Err(integrity(format!(
            "{wanted} has {} blocks; the block map lists {}",
            chunks.len(),
            hashes.len()
        )));
    }
    for (number, (chunk, hash)) in chunks.iter().zip(&hashes).enumerate() {
        if tigersetup_format::sha256(chunk).as_slice() != hash.as_slice() {
            return Err(integrity(format!(
                "{wanted} block {number} does not match the block map"
            )));
        }
    }
    Ok(())
}

/// Standard base64 with padding; `None` for anything else.
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, quad) in bytes.chunks(4).enumerate() {
        let last = index == bytes.len() / 4 - 1;
        let pad = quad.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut acc = 0u32;
        for &c in &quad[..4 - pad] {
            acc = (acc << 6) | value(c)?;
        }
        acc <<= 6 * pad as u32;
        let decoded = [(acc >> 16) as u8, (acc >> 8) as u8, acc as u8];
        out.extend_from_slice(&decoded[..3 - pad]);
    }
    Some(out)
}

// --- Identity --------------------------------------------------------------

/// The `<Identity>` element of an AppX manifest, checked against the WinGet
/// source's name and publisher, its version read as a publication time.
pub fn parse_identity(manifest: &str) -> Result<SourceIdentity> {
    let manifest = strip_comments(manifest);
    let mut tags = start_tags(&manifest, "Identity");
    let tag = tags
        .next()
        .ok_or_else(|| identity_mismatch(format!("{MANIFEST_ENTRY} has no Identity")))?;
    if tags.next().is_some() {
        return Err(identity_mismatch(format!(
            "{MANIFEST_ENTRY} has more than one Identity"
        )));
    }
    let read = |name: &str| -> Result<String> {
        attribute(tag, name)
            .map_err(|err| identity_mismatch(err.message))?
            .ok_or_else(|| identity_mismatch(format!("the package Identity has no {name}")))
    };
    let (name, publisher, version) = (read("Name")?, read("Publisher")?, read("Version")?);
    if name != SOURCE_NAME || publisher != SOURCE_PUBLISHER {
        return Err(identity_mismatch(format!(
            "the package is {name:?} by {publisher:?}, not {SOURCE_NAME} by {SOURCE_PUBLISHER}"
        )));
    }
    let published_at = published_at(&version);
    Ok(SourceIdentity {
        version,
        published_at,
    })
}

/// The publication time a source version encodes (`yyyy.MMdd.HHmm.n`), in
/// Unix seconds, read as UTC. The publisher's clock zone is not stated (the
/// served index has carried a time about an hour ahead of UTC), which is
/// immaterial against bounds measured in days.
pub fn published_at(version: &str) -> Option<i64> {
    let parts: Vec<u32> = version
        .split('.')
        .map(|part| {
            if part.is_empty() || part.len() > 5 || !part.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                part.parse().ok()
            }
        })
        .collect::<Option<_>>()?;
    let [year, month_day, hour_minute, _] = parts[..] else {
        return None;
    };
    let (month, day) = (month_day / 100, month_day % 100);
    let (hour, minute) = (hour_minute / 100, hour_minute % 100);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(2000..=9999).contains(&year) || day == 0 || day > days_in_month || hour > 23 || minute > 59
    {
        return None;
    }
    Some(
        days_from_civil(year as i64, month as i64, day as i64) * 86_400
            + i64::from(hour) * 3_600
            + i64::from(minute) * 60,
    )
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Refuses an index published before `not_before` (Unix seconds). An index
/// whose version names no publication time cannot be judged, and passes.
pub fn check_fresh(identity: &SourceIdentity, not_before: i64) -> Result<()> {
    if let Some(published_at) = identity.published_at
        && published_at < not_before
    {
        return Err(CatalogError::new(
            Reason::CatalogIndexStale,
            format!(
                "the source index {} was published at {published_at} (Unix seconds), before the accepted bound {not_before}",
                identity.version
            ),
        ));
    }
    Ok(())
}

// --- XML scanning ------------------------------------------------------------
//
// The two documents read here are small, machine-written and signed; a scan
// of start tags and their attributes is all they need.

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start + 4..].find("-->") {
            Some(end) => &rest[start + 4 + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The text of each `<tag ...>` start tag, from just after the name to the
/// closing `>` (quotes respected), whatever namespace prefixes the document
/// uses elsewhere.
fn start_tags<'a>(xml: &'a str, tag: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    elements(xml, tag).map(|(tag, _)| tag)
}

/// Each `<tag ...>` element: its start tag's attribute text and its body up
/// to `</tag>` (empty for `<tag .../>`).
fn elements<'a>(xml: &'a str, tag: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = xml;
    std::iter::from_fn(move || {
        loop {
            let start = rest.find(&open)? + open.len();
            let after = &rest[start..];
            match after.chars().next() {
                Some(c) if c.is_whitespace() || c == '/' || c == '>' => {}
                _ => {
                    rest = after;
                    continue;
                }
            }
            let mut quote = None;
            let end = after.char_indices().find_map(|(i, c)| match (quote, c) {
                (None, '"' | '\'') => {
                    quote = Some(c);
                    None
                }
                (Some(q), c) if c == q => {
                    quote = None;
                    None
                }
                (None, '>') => Some(i),
                _ => None,
            })?;
            let attributes = &after[..end];
            let tail = &after[end + 1..];
            if let Some(attributes) = attributes.strip_suffix('/') {
                rest = tail;
                return Some((attributes, ""));
            }
            let body_end = tail.find(&close).unwrap_or(tail.len());
            rest = tail;
            return Some((attributes, &tail[..body_end]));
        }
    })
}

/// One attribute's value from a start tag's attribute text, entities
/// decoded; an attribute given twice is malformed.
fn attribute(tag: &str, name: &str) -> Result<Option<String>> {
    let malformed = || integrity(format!("malformed attributes in <{}>", tag.trim()));
    let mut found = None;
    let mut rest = tag.trim_start();
    while !rest.is_empty() && rest != "/" {
        let eq = rest.find('=').ok_or_else(malformed)?;
        let key = rest[..eq].trim();
        let value_start = rest[eq + 1..].trim_start();
        let quote = value_start.chars().next().ok_or_else(malformed)?;
        if quote != '"' && quote != '\'' {
            return Err(malformed());
        }
        let value_end = value_start[1..].find(quote).ok_or_else(malformed)?;
        let value = &value_start[1..1 + value_end];
        if key == name {
            if found.is_some() {
                return Err(malformed());
            }
            found = Some(decode_entities(value));
        }
        rest = value_start[1 + value_end + 1..].trim_start();
    }
    Ok(found)
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Synthetic source packages: the real layout (block map, manifest, index),
/// unsigned.
#[cfg(test)]
pub(crate) mod test_package {
    use super::{BLOCK_LEN, BLOCK_MAP_ENTRY, BLOCK_MAP_HASH_METHOD};
    use std::io::Write;

    pub(crate) fn b64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.len();
            let acc = chunk
                .iter()
                .chain(std::iter::repeat(&0))
                .take(3)
                .fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
            for i in 0..4 {
                if i <= n {
                    out.push(ALPHABET[(acc >> (18 - 6 * i)) as usize & 63] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub(crate) fn file_element(name: &str, bytes: &[u8]) -> String {
        let blocks: String = bytes
            .chunks(BLOCK_LEN)
            .map(|chunk| {
                format!(
                    r#"<Block Hash="{}" Size="123"/>"#,
                    b64(&tigersetup_format::sha256(chunk))
                )
            })
            .collect();
        format!(
            r#"<File Name="{}" Size="{}" LfhSize="45">{blocks}</File>"#,
            name.replace('/', "\\"),
            bytes.len()
        )
    }

    pub(crate) fn block_map(files: &[(&str, &[u8])]) -> String {
        let files: String = files.iter().map(|(n, b)| file_element(n, b)).collect();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><BlockMap xmlns="http://schemas.microsoft.com/appx/2010/blockmap" HashMethod="{BLOCK_MAP_HASH_METHOD}">{files}</BlockMap>"#
        )
    }

    pub(crate) fn manifest(name: &str, publisher: &str, version: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<Package xmlns=\"x\">\n  <!-- <Identity Name=\"Decoy\"/> -->\n  <Identity Name=\"{name}\"\n            ProcessorArchitecture=\"neutral\"\n            Publisher=\"{publisher}\"\n            Version=\"{version}\" />\n</Package>"
        )
    }

    /// A ZIP with the given entries, stored, in order (duplicates allowed:
    /// written by hand, since a ZIP writer refuses them).
    pub(crate) fn raw_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let crc = {
                let mut hasher = flate2::Crc::new();
                hasher.update(data);
                hasher.sum()
            };
            let header = |sig: u32, central_record: bool, buf: &mut Vec<u8>| {
                buf.extend_from_slice(&sig.to_le_bytes());
                if central_record {
                    buf.extend_from_slice(&20u16.to_le_bytes());
                }
                buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
                buf.extend_from_slice(&0u16.to_le_bytes()); // flags
                buf.extend_from_slice(&0u16.to_le_bytes()); // stored
                buf.extend_from_slice(&0u32.to_le_bytes()); // time, date
                buf.extend_from_slice(&crc.to_le_bytes());
                buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
                buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
                buf.extend_from_slice(&0u16.to_le_bytes()); // extra
                if central_record {
                    buf.extend_from_slice(&0u16.to_le_bytes()); // comment
                    buf.extend_from_slice(&0u16.to_le_bytes()); // disk
                    buf.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
                    buf.extend_from_slice(&0u32.to_le_bytes()); // external attributes
                    buf.extend_from_slice(&offset.to_le_bytes());
                }
                buf.extend_from_slice(name.as_bytes());
            };
            header(0x0403_4b50, false, &mut out);
            out.extend_from_slice(data);
            header(0x0201_4b50, true, &mut central);
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    pub(crate) fn package(manifest: &str, index: &[u8]) -> Vec<u8> {
        let map = block_map(&[
            ("Public/index.db", index),
            ("AppxManifest.xml", manifest.as_bytes()),
        ]);
        let mut msix = Vec::new();
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut msix));
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in [
            ("Public/index.db", index),
            ("AppxManifest.xml", manifest.as_bytes()),
            (BLOCK_MAP_ENTRY, map.as_bytes()),
        ] {
            writer.start_file(name, deflated).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
        msix
    }

    pub(crate) fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_package::*;
    use super::*;
    use std::io::Write;

    #[test]
    fn base64_decodes_the_standard_alphabet_and_refuses_the_rest() {
        for len in 0..40 {
            let bytes = sample(len);
            if len > 0 {
                assert_eq!(base64_decode(&b64(&bytes)).unwrap(), bytes, "len {len}");
            }
        }
        assert_eq!(
            base64_decode("PYCziJkFU7iYZzR/+YlXrUunYdRBK2xAXy0tpMlnDHM=")
                .unwrap()
                .len(),
            32
        );
        for bad in ["", "abc", "ab=c", "a===", "ab-_", "QQ==QQ=="] {
            assert_eq!(base64_decode(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_file_matching_its_block_map_is_accepted_and_any_difference_refused() {
        // Three blocks: two full, one partial.
        let bytes = sample(2 * BLOCK_LEN + 1000);
        let map = block_map(&[("Public/index.db", &bytes), ("AppxManifest.xml", b"m")]);
        verify_block_map(&map, "Public/index.db", &bytes).unwrap();
        verify_block_map(&map, "AppxManifest.xml", b"m").unwrap();

        let mut flipped = bytes.clone();
        flipped[BLOCK_LEN + 5] ^= 1;
        let err = verify_block_map(&map, "Public/index.db", &flipped).unwrap_err();
        assert_eq!(err.reason, Reason::CatalogIntegrityFailure);
        assert!(err.message.contains("block 1"), "{err}");

        let shorter = &bytes[..bytes.len() - 1];
        let err = verify_block_map(&map, "Public/index.db", shorter).unwrap_err();
        assert_eq!(err.reason, Reason::CatalogIntegrityFailure);

        // Size right, one block missing from the map.
        let only = block_map(&[("Public/index.db", &bytes)]);
        let last = only.rfind("<Block Hash").unwrap();
        let end = last + only[last..].find("/>").unwrap() + 2;
        let truncated = format!("{}{}", &only[..last], &only[end..]);
        let err = verify_block_map(&truncated, "Public/index.db", &bytes).unwrap_err();
        assert!(err.message.contains("3 blocks"), "{err}");

        let err = verify_block_map(&map, "Public/other.db", &bytes).unwrap_err();
        assert_eq!(err.reason, Reason::CatalogIntegrityFailure);

        let twice = block_map(&[("Public/index.db", &bytes), ("Public/index.db", &bytes)]);
        let err = verify_block_map(&twice, "Public/index.db", &bytes).unwrap_err();
        assert!(err.message.contains("twice"), "{err}");

        // An empty file is listed with no blocks.
        let empty = block_map(&[("Public/empty", b"")]);
        verify_block_map(&empty, "Public/empty", b"").unwrap();
    }

    #[test]
    fn duplicate_entry_names_are_refused() {
        let names = central_directory_names(&raw_zip(&[("a.txt", b"1"), ("b.txt", b"2")])).unwrap();
        assert_eq!(names, ["a.txt", "b.txt"]);
        for second in ["Public/index.db", "public/INDEX.db", "Public\\index.db"] {
            let zip = raw_zip(&[("Public/index.db", b"good"), (second, b"evil")]);
            let err = SourceContent::open(&zip).err().unwrap();
            assert_eq!(err.reason, Reason::CatalogIntegrityFailure, "{second}");
            assert!(err.message.contains("more than once"), "{err}");
        }
        assert_eq!(
            SourceContent::open(b"not a zip").err().unwrap().reason,
            Reason::CatalogIntegrityFailure
        );
    }

    #[test]
    fn package_content_is_read_through_the_block_map() {
        let index = sample(BLOCK_LEN + 17);
        let text = manifest(SOURCE_NAME, SOURCE_PUBLISHER, "2026.1004.1308.18");
        let msix = package(&text, &index);
        let mut content = SourceContent::open(&msix).unwrap();
        assert_eq!(content.entry("Public/index.db", 1 << 20).unwrap(), index);
        let identity = content.identity().unwrap();
        assert_eq!(identity.version, "2026.1004.1308.18");

        // A package whose index differs from what its block map lists.
        let map = block_map(&[
            ("Public/index.db", &index),
            ("AppxManifest.xml", text.as_bytes()),
        ]);
        let mut other = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut other));
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("Public/index.db", options).unwrap();
            writer.write_all(&sample(100)).unwrap();
            writer.start_file(BLOCK_MAP_ENTRY, options).unwrap();
            writer.write_all(map.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        let mut content = SourceContent::open(&other).unwrap();
        assert_eq!(
            content
                .entry("Public/index.db", 1 << 20)
                .unwrap_err()
                .reason,
            Reason::CatalogIntegrityFailure
        );
    }

    #[test]
    fn identity_must_be_the_winget_source_by_microsoft() {
        let identity = parse_identity(&manifest(
            SOURCE_NAME,
            SOURCE_PUBLISHER,
            "2026.1004.1308.18",
        ))
        .unwrap();
        assert_eq!(identity.version, "2026.1004.1308.18");
        assert_eq!(identity.published_at, published_at("2026.1004.1308.18"));
        for (name, publisher, version) in [
            (
                "Microsoft.Winget.Source.Evil",
                SOURCE_PUBLISHER,
                "2026.1004.1308.18",
            ),
            (
                "microsoft.winget.source",
                SOURCE_PUBLISHER,
                "2026.1004.1308.18",
            ),
            (SOURCE_NAME, "CN=Microsoft Corporation", "2026.1004.1308.18"),
        ] {
            let err = parse_identity(&manifest(name, publisher, version)).unwrap_err();
            assert_eq!(
                err.reason,
                Reason::CatalogIdentityMismatch,
                "{name} {publisher} {version}"
            );
        }
        assert_eq!(
            parse_identity("<Package/>").unwrap_err().reason,
            Reason::CatalogIdentityMismatch
        );
        let two = format!(
            "{}<Identity Name=\"{SOURCE_NAME}\"/>",
            manifest(SOURCE_NAME, SOURCE_PUBLISHER, "2026.1004.1308.18")
        );
        assert_eq!(
            parse_identity(&two).unwrap_err().reason,
            Reason::CatalogIdentityMismatch
        );
        // Single quotes, spacing around `=` and an element merely starting
        // with the name are all read as XML reads them.
        let spaced = format!(
            "<IdentityExtra Name=\"x\"/><Identity Name = '{SOURCE_NAME}' Publisher=\"{}\" Version=\"2026.101.5.1\"/>",
            SOURCE_PUBLISHER
        );
        assert_eq!(parse_identity(&spaced).unwrap().version, "2026.101.5.1");
    }

    #[test]
    fn version_reads_as_a_publication_time_and_older_indexes_are_refused() {
        // 2026-10-04 13:08 UTC.
        assert_eq!(published_at("2026.1004.1308.18"), Some(1_791_119_280));
        assert_eq!(published_at("2000.101.0.0"), Some(946_684_800));
        assert_eq!(published_at("2024.229.2359.1"), Some(1_709_251_140));
        for bad in [
            "2026.1004.1308",
            "2026.1004.1308.18.1",
            "2026.1304.1308.18",
            "2026.1032.1308.18",
            "2025.229.1308.18",
            "2026.1004.2400.18",
            "2026.1004.1360.18",
            "2026.1004.-1.18",
            "1999.1004.1308.18",
            "2026.1004.1308.x",
        ] {
            assert_eq!(published_at(bad), None, "{bad}");
        }
        let identity = SourceIdentity {
            version: "2026.1004.1308.18".into(),
            published_at: Some(1_791_119_280),
        };
        check_fresh(&identity, 1_791_119_280).unwrap();
        check_fresh(&identity, 0).unwrap();
        let err = check_fresh(&identity, 1_791_119_281).unwrap_err();
        assert_eq!(err.reason, Reason::CatalogIndexStale);
        assert_eq!(err.reason.code(), "catalog_index_stale");

        // A version that names no time is still the WinGet source's, and is
        // judged by its signature, content and identity alone.
        let untimed = parse_identity(&manifest(SOURCE_NAME, SOURCE_PUBLISHER, "1.0.0.0")).unwrap();
        assert_eq!(untimed.published_at, None);
        check_fresh(&untimed, i64::MAX).unwrap();
    }

    #[test]
    fn unsigned_or_unparsable_packages_fail_the_signature_check() {
        let msix = package(
            &manifest(SOURCE_NAME, SOURCE_PUBLISHER, "2026.1004.1308.18"),
            &sample(5000),
        );
        for (what, bytes) in [
            ("unsigned package", msix.as_slice()),
            ("not a package", b"not a package".as_slice()),
            ("empty", b"".as_slice()),
        ] {
            let err = verify_signature(bytes).unwrap_err();
            assert_eq!(err.reason, Reason::CatalogSignatureInvalid, "{what}: {err}");
        }
    }

    #[test]
    fn the_sealed_copy_refuses_writers_and_is_gone_when_closed() {
        let sealed = SealedFile::create(b"bytes").unwrap();
        let path = sealed.path().to_path_buf();
        assert!(path.starts_with(std::env::temp_dir()));
        assert!(
            std::fs::OpenOptions::new().write(true).open(&path).is_err(),
            "a second writer must be refused"
        );
        let other = SealedFile::create(b"bytes").unwrap();
        assert_ne!(other.path(), path);
        drop(sealed);
        assert!(!path.exists());
        drop(other);
    }
}
