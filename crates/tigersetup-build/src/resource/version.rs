//! A `VS_VERSIONINFO` resource built in memory: the fixed `VS_FIXEDFILEINFO`
//! block, one `StringFileInfo` table for US English / Unicode (`040904b0`)
//! and the matching `VarFileInfo` translation — the same shape the resource
//! compiler emits for a `VERSIONINFO` statement, so `GetFileVersionInfo`
//! and Explorer read it the way they read any other executable's.
//!
//! The layout is a tree of nodes that each start `wLength, wValueLength,
//! wType, szKey`, pad to a 32-bit boundary before their value and again
//! before their children, and whose `wLength` counts the node without any
//! padding that follows it (the parent pads between children).

use windows_sys::Win32::Storage::FileSystem::{
    VFT_APP, VOS_NT_WINDOWS32, VS_FFI_SIGNATURE, VS_FFI_STRUCVERSION,
};

/// The strings and versions a version resource carries. An empty string is
/// left out of the string table rather than written as an empty value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub company_name: String,
    pub product_name: String,
    /// As written: `Major.Minor.Patch`.
    pub product_version: String,
    /// The version padded with `.0` to four parts: the string and the fixed
    /// block carry the same value.
    pub file_version: String,
    pub file_description: String,
    pub legal_copyright: String,
    pub original_filename: String,
    pub internal_name: String,
}

/// US English, Unicode: the language and code page of the one string table.
pub const LANGUAGE: u16 = 0x0409;
pub const CODE_PAGE: u16 = 1200;

/// Four numeric parts of a dotted version, missing parts zero and a part
/// that does not fit a 16-bit field clamped to its maximum. A version resource
/// cannot express more than that, and clamping is a visible loss where
/// wrapping would be a silent one.
pub fn version_parts(version: &str) -> [u16; 4] {
    let mut parts = [0u16; 4];
    for (slot, part) in parts.iter_mut().zip(version.split('.')) {
        *slot = part
            .trim()
            .parse::<u64>()
            .map(|value| u16::try_from(value).unwrap_or(u16::MAX))
            .unwrap_or(0);
    }
    parts
}

/// `Major.Minor.Patch` padded with `.0` to four parts.
pub fn padded_version(version: &str) -> String {
    let parts = version_parts(version);
    format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3])
}

fn utf16z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// One node: header, key, padding, value (`is_text` decides `wType` and
/// whether `wValueLength` counts characters or bytes), padding, children.
fn node(key: &str, value: &[u8], is_text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    push_u16(&mut out, 0); // wLength, patched below
    let value_length = if is_text {
        (value.len() / 2) as u16
    } else {
        value.len() as u16
    };
    push_u16(&mut out, value_length);
    push_u16(&mut out, if is_text { 1 } else { 0 });
    for unit in utf16z(key) {
        push_u16(&mut out, unit);
    }
    if !value.is_empty() {
        pad(&mut out);
        out.extend_from_slice(value);
    }
    for child in children {
        pad(&mut out);
        out.extend_from_slice(child);
    }
    let length = out.len() as u16;
    out[0..2].copy_from_slice(&length.to_le_bytes());
    out
}

fn string_node(name: &str, value: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for unit in utf16z(value) {
        push_u16(&mut bytes, unit);
    }
    node(name, &bytes, true, &[])
}

fn fixed_file_info(identity: &Identity) -> Vec<u8> {
    let file = version_parts(&identity.file_version);
    let product = version_parts(&identity.product_version);
    let ms = |parts: [u16; 4]| ((parts[0] as u32) << 16) | parts[1] as u32;
    let ls = |parts: [u16; 4]| ((parts[2] as u32) << 16) | parts[3] as u32;
    let mut out = Vec::with_capacity(52);
    push_u32(&mut out, VS_FFI_SIGNATURE as u32);
    push_u32(&mut out, VS_FFI_STRUCVERSION as u32);
    push_u32(&mut out, ms(file));
    push_u32(&mut out, ls(file));
    push_u32(&mut out, ms(product));
    push_u32(&mut out, ls(product));
    push_u32(&mut out, 0x3f); // dwFileFlagsMask
    push_u32(&mut out, 0); // dwFileFlags
    push_u32(&mut out, VOS_NT_WINDOWS32);
    push_u32(&mut out, VFT_APP as u32);
    push_u32(&mut out, 0); // dwFileSubtype
    push_u32(&mut out, 0); // dwFileDateMS
    push_u32(&mut out, 0); // dwFileDateLS
    out
}

/// The complete `VS_VERSIONINFO` resource for an identity.
pub fn encode(identity: &Identity) -> Vec<u8> {
    let strings: Vec<Vec<u8>> = [
        ("CompanyName", &identity.company_name),
        ("FileDescription", &identity.file_description),
        ("FileVersion", &identity.file_version),
        ("InternalName", &identity.internal_name),
        ("LegalCopyright", &identity.legal_copyright),
        ("OriginalFilename", &identity.original_filename),
        ("ProductName", &identity.product_name),
        ("ProductVersion", &identity.product_version),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(name, value)| string_node(name, value))
    .collect();
    let table = node(
        &format!("{LANGUAGE:04x}{CODE_PAGE:04x}"),
        &[],
        true,
        &strings,
    );
    let string_file_info = node("StringFileInfo", &[], true, &[table]);

    let mut translation = Vec::with_capacity(4);
    push_u16(&mut translation, LANGUAGE);
    push_u16(&mut translation, CODE_PAGE);
    let var = node("Translation", &translation, false, &[]);
    let var_file_info = node("VarFileInfo", &[], true, &[var]);

    node(
        "VS_VERSION_INFO",
        &fixed_file_info(identity),
        false,
        &[string_file_info, var_file_info],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_pad_to_four_parts_and_clamp_what_does_not_fit() {
        assert_eq!(version_parts("0.5.0"), [0, 5, 0, 0]);
        assert_eq!(version_parts("1.2.3.4"), [1, 2, 3, 4]);
        assert_eq!(version_parts("70000.1.x"), [u16::MAX, 1, 0, 0]);
        assert_eq!(padded_version("0.8.2"), "0.8.2.0");
        assert_eq!(padded_version(""), "0.0.0.0");
    }

    #[test]
    fn the_blob_is_dword_aligned_and_starts_with_the_fixed_block() {
        let identity = Identity {
            company_name: "IT Tiger".into(),
            product_name: "Sample".into(),
            product_version: "1.2.3".into(),
            file_version: "1.2.3.0".into(),
            file_description: "Sample Setup".into(),
            ..Identity::default()
        };
        let blob = encode(&identity);
        assert_eq!(u16::from_le_bytes([blob[0], blob[1]]) as usize, blob.len());
        assert_eq!(u16::from_le_bytes([blob[2], blob[3]]), 52);
        assert_eq!(u16::from_le_bytes([blob[4], blob[5]]), 0);
        // "VS_VERSION_INFO\0" is 16 characters: the fixed block follows the
        // 6-byte header, the 32-byte key and 2 bytes of padding.
        let fixed = 6 + 32 + 2;
        assert_eq!(
            u32::from_le_bytes(blob[fixed..fixed + 4].try_into().unwrap()),
            0xFEEF04BD
        );
        let file_ms = u32::from_le_bytes(blob[fixed + 8..fixed + 12].try_into().unwrap());
        assert_eq!(file_ms, (1 << 16) | 2);
        // The same identity encodes to the same bytes.
        assert_eq!(blob, encode(&identity));
        // An omitted string shrinks the blob rather than writing an empty value.
        let with_copyright = Identity {
            legal_copyright: "(c) IT Tiger".into(),
            ..identity
        };
        assert!(encode(&with_copyright).len() > blob.len());
    }
}
