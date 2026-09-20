//! The fixed 256-byte trailer that maps the installer file.
//!
//! Layout, little-endian, all offsets absolute from the start of the file:
//!
//! ```text
//! 0    8   leading magic  "TIGERSTP"
//! 8    2   format major (2)
//! 10   2   format minor
//! 12   4   footer length (256)
//! 16   8   engine offset            the compressed engine block
//! 24   8   engine length            (compressed bytes)
//! 32   8   engine uncompressed length
//! 40   8   metadata offset
//! 48   8   metadata length
//! 56   8   payload offset           the compressed payload block
//! 64   8   payload length           (compressed bytes)
//! 72   8   payload uncompressed length
//! 80   32  SHA-256 of the compressed engine block
//! 112  32  SHA-256 of the engine executable the block decompresses to
//! 144  32  SHA-256 of the metadata block
//! 176  32  SHA-256 of the compressed payload block
//! 208  36  reserved (zero)
//! 244  4   CRC-32 (IEEE) of bytes [0, 244)
//! 248  8   trailing magic "PTSREGIT"
//! ```
//!
//! The blocks lie in the file in the order loader, engine, payload,
//! metadata, footer: the payload precedes the metadata because the metadata
//! carries the payload's index, which is only known once the payload has
//! been written, and this order lets the builder write the file in one
//! pass. The loader is bytes `[0, engine offset)`. Everything the loader
//! needs to bootstrap the engine is here: where the compressed engine is,
//! how large it decompresses to, and the hash the decompressed bytes must
//! have before they are executed.
//!
//! Format 1 — the engine as the executable stub and a ZIP payload — is not
//! read by this crate: an installer of that format carries its own engine
//! and stays self-contained, and nothing in the product inspects one.

use crate::FormatError;

pub const FOOTER_LEN: usize = 256;
pub const MAGIC_HEAD: [u8; 8] = *b"TIGERSTP";
pub const MAGIC_TAIL: [u8; 8] = *b"PTSREGIT";
pub const FORMAT_MAJOR: u16 = 2;
pub const FORMAT_MINOR: u16 = 0;

const CRC_OFFSET: usize = 244;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footer {
    pub format_major: u16,
    pub format_minor: u16,
    pub engine_offset: u64,
    pub engine_length: u64,
    pub engine_uncompressed_length: u64,
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub payload_offset: u64,
    pub payload_length: u64,
    pub payload_uncompressed_length: u64,
    pub engine_sha256: [u8; 32],
    pub engine_executable_sha256: [u8; 32],
    pub metadata_sha256: [u8; 32],
    pub payload_sha256: [u8; 32],
}

impl Footer {
    pub fn encode(&self) -> [u8; FOOTER_LEN] {
        let mut out = [0u8; FOOTER_LEN];
        out[0..8].copy_from_slice(&MAGIC_HEAD);
        out[8..10].copy_from_slice(&self.format_major.to_le_bytes());
        out[10..12].copy_from_slice(&self.format_minor.to_le_bytes());
        out[12..16].copy_from_slice(&(FOOTER_LEN as u32).to_le_bytes());
        out[16..24].copy_from_slice(&self.engine_offset.to_le_bytes());
        out[24..32].copy_from_slice(&self.engine_length.to_le_bytes());
        out[32..40].copy_from_slice(&self.engine_uncompressed_length.to_le_bytes());
        out[40..48].copy_from_slice(&self.metadata_offset.to_le_bytes());
        out[48..56].copy_from_slice(&self.metadata_length.to_le_bytes());
        out[56..64].copy_from_slice(&self.payload_offset.to_le_bytes());
        out[64..72].copy_from_slice(&self.payload_length.to_le_bytes());
        out[72..80].copy_from_slice(&self.payload_uncompressed_length.to_le_bytes());
        out[80..112].copy_from_slice(&self.engine_sha256);
        out[112..144].copy_from_slice(&self.engine_executable_sha256);
        out[144..176].copy_from_slice(&self.metadata_sha256);
        out[176..208].copy_from_slice(&self.payload_sha256);
        let crc = crc32fast::hash(&out[..CRC_OFFSET]);
        out[CRC_OFFSET..CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());
        out[248..256].copy_from_slice(&MAGIC_TAIL);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Footer, FormatError> {
        if bytes.len() != FOOTER_LEN {
            return Err(FormatError::new(
                "footer_invalid",
                format!("footer must be {FOOTER_LEN} bytes, got {}", bytes.len()),
            ));
        }
        if bytes[0..8] != MAGIC_HEAD || bytes[248..256] != MAGIC_TAIL {
            return Err(FormatError::new(
                "footer_missing",
                "the file does not end with a TigerSetup footer",
            ));
        }
        let expected_crc =
            u32::from_le_bytes(bytes[CRC_OFFSET..CRC_OFFSET + 4].try_into().unwrap());
        let actual_crc = crc32fast::hash(&bytes[..CRC_OFFSET]);
        if expected_crc != actual_crc {
            return Err(FormatError::new(
                "footer_crc_mismatch",
                format!("footer CRC-32 is {actual_crc:08x}, expected {expected_crc:08x}"),
            ));
        }
        let format_major = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        let format_minor = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        if format_major != FORMAT_MAJOR {
            return Err(FormatError::new(
                "format_unsupported",
                format!(
                    "installer format {format_major}.{format_minor} is not supported by format {FORMAT_MAJOR}.{FORMAT_MINOR}"
                ),
            ));
        }
        let footer_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        if footer_len as usize != FOOTER_LEN {
            return Err(FormatError::new(
                "footer_invalid",
                format!("footer length field is {footer_len}, expected {FOOTER_LEN}"),
            ));
        }
        let read_u64 = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let read_hash = |at: usize| {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&bytes[at..at + 32]);
            hash
        };
        Ok(Footer {
            format_major,
            format_minor,
            engine_offset: read_u64(16),
            engine_length: read_u64(24),
            engine_uncompressed_length: read_u64(32),
            metadata_offset: read_u64(40),
            metadata_length: read_u64(48),
            payload_offset: read_u64(56),
            payload_length: read_u64(64),
            payload_uncompressed_length: read_u64(72),
            engine_sha256: read_hash(80),
            engine_executable_sha256: read_hash(112),
            metadata_sha256: read_hash(144),
            payload_sha256: read_hash(176),
        })
    }

    /// Checks that the blocks the footer names lie inside the file, in
    /// order — loader, engine, payload, metadata — and end where the footer
    /// begins. The loader and the engine are never empty; the payload may
    /// be (the uninstaller copy carries none).
    pub fn check_layout(&self, footer_offset: u64) -> Result<(), FormatError> {
        let engine_end = self.engine_offset.checked_add(self.engine_length);
        let payload_end = self.payload_offset.checked_add(self.payload_length);
        let metadata_end = self.metadata_offset.checked_add(self.metadata_length);
        match (engine_end, payload_end, metadata_end) {
            (Some(ee), Some(pe), Some(me))
                if self.engine_offset > 0
                    && self.engine_length > 0
                    && self.engine_uncompressed_length > 0
                    && ee == self.payload_offset
                    && pe == self.metadata_offset
                    && self.metadata_length > 0
                    && me == footer_offset
                    && (self.payload_length == 0) == (self.payload_uncompressed_length == 0) =>
            {
                Ok(())
            }
            _ => Err(FormatError::new(
                "footer_invalid",
                "the footer's block map does not describe the file",
            )),
        }
    }
}

/// Where the footer starts in a file of `file_len` bytes whose first bytes are
/// `head` (at least the PE headers; 4 KiB is plenty).
///
/// The rule is positional: `file_len - 256`, unless the executable carries an
/// Authenticode signature. Signing appends the certificate table after the
/// footer and records it in the PE security directory, in which case the
/// footer ends where that table begins. A `head` that is not a PE image (the
/// tests use a stub loader) falls back to the end-of-file rule.
pub fn locate_footer(file_len: u64, head: &[u8]) -> Result<u64, FormatError> {
    if file_len < FOOTER_LEN as u64 {
        return Err(FormatError::new(
            "footer_missing",
            "the file is shorter than a footer",
        ));
    }
    let end = match security_directory(head) {
        Some((offset, size)) if size > 0 && (offset as u64) <= file_len => offset as u64,
        _ => file_len,
    };
    end.checked_sub(FOOTER_LEN as u64)
        .ok_or_else(|| FormatError::new("footer_missing", "the file is shorter than a footer"))
}

/// Reads `IMAGE_DIRECTORY_ENTRY_SECURITY` (offset, size) from a PE header, if
/// `head` holds a well-formed one.
fn security_directory(head: &[u8]) -> Option<(u32, u32)> {
    let u16_at = |at: usize| {
        head.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        head.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if u16_at(0)? != 0x5A4D {
        return None; // "MZ"
    }
    let pe_offset = u32_at(0x3C)? as usize;
    if u32_at(pe_offset)? != 0x0000_4550 {
        return None; // "PE\0\0"
    }
    let optional_header = pe_offset + 24;
    let magic = u16_at(optional_header)?;
    let directories = match magic {
        0x20B => optional_header + 112, // PE32+
        0x10B => optional_header + 96,  // PE32
        _ => return None,
    };
    let security = directories + 4 * 8;
    Some((u32_at(security)?, u32_at(security + 4)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Footer {
        Footer {
            format_major: FORMAT_MAJOR,
            format_minor: FORMAT_MINOR,
            engine_offset: 1000,
            engine_length: 500,
            engine_uncompressed_length: 2000,
            payload_offset: 1500,
            payload_length: 5000,
            payload_uncompressed_length: 9000,
            metadata_offset: 6500,
            metadata_length: 200,
            engine_sha256: [3u8; 32],
            engine_executable_sha256: [4u8; 32],
            metadata_sha256: [1u8; 32],
            payload_sha256: [2u8; 32],
        }
    }

    #[test]
    fn footer_round_trips() {
        let footer = sample();
        let bytes = footer.encode();
        assert_eq!(bytes.len(), FOOTER_LEN);
        assert_eq!(Footer::decode(&bytes).unwrap(), footer);
        assert!(footer.check_layout(6700).is_ok());
        assert!(footer.check_layout(6701).is_err());
    }

    #[test]
    fn an_empty_payload_is_a_valid_layout_and_a_half_empty_one_is_not() {
        let mut footer = sample();
        footer.payload_length = 0;
        footer.payload_uncompressed_length = 0;
        footer.metadata_offset = 1500;
        assert!(footer.check_layout(1700).is_ok());
        footer.payload_uncompressed_length = 1;
        assert!(footer.check_layout(1700).is_err());
    }

    #[test]
    fn an_engine_block_must_be_present() {
        let mut footer = sample();
        footer.engine_length = 0;
        footer.payload_offset = 1000;
        footer.metadata_offset = 6000;
        assert!(footer.check_layout(6200).is_err());
    }

    #[test]
    fn corrupted_footer_fails_crc() {
        let mut bytes = sample().encode();
        bytes[20] ^= 0xFF;
        assert_eq!(
            Footer::decode(&bytes).unwrap_err().code,
            "footer_crc_mismatch"
        );
    }

    #[test]
    fn missing_magic_is_reported() {
        let bytes = [0u8; FOOTER_LEN];
        assert_eq!(Footer::decode(&bytes).unwrap_err().code, "footer_missing");
    }

    #[test]
    fn unsupported_major_is_reported() {
        let mut footer = sample();
        footer.format_major = 3;
        assert_eq!(
            Footer::decode(&footer.encode()).unwrap_err().code,
            "format_unsupported"
        );
    }

    fn fake_pe(security_offset: u32, security_size: u32) -> Vec<u8> {
        let mut head = vec![0u8; 4096];
        head[0] = b'M';
        head[1] = b'Z';
        head[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        head[0x80..0x84].copy_from_slice(b"PE\0\0");
        let optional = 0x80 + 24;
        head[optional..optional + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        let security = optional + 112 + 32;
        head[security..security + 4].copy_from_slice(&security_offset.to_le_bytes());
        head[security + 4..security + 8].copy_from_slice(&security_size.to_le_bytes());
        head
    }

    #[test]
    fn footer_is_at_end_of_file_without_signature() {
        assert_eq!(locate_footer(10_000, &fake_pe(0, 0)).unwrap(), 10_000 - 256);
        assert_eq!(locate_footer(10_000, b"not a pe").unwrap(), 10_000 - 256);
    }

    #[test]
    fn footer_precedes_certificate_table_when_signed() {
        assert_eq!(
            locate_footer(12_000, &fake_pe(10_000, 2_000)).unwrap(),
            10_000 - 256
        );
    }

    #[test]
    fn short_file_has_no_footer() {
        assert_eq!(locate_footer(100, b"").unwrap_err().code, "footer_missing");
    }
}
