//! The TigerSetup installer file format.
//!
//! A generated installer is one executable laid out as
//!
//! ```text
//! [loader (PE)] [Zstandard-compressed engine (PE)] [one solid Zstandard payload]
//! [Protocol Buffers metadata] [256-byte footer]
//! ```
//!
//! The footer is the only block found by position; it points directly at
//! the other blocks. The loader decompresses the engine and starts it; the
//! engine reads the metadata and the payload from the original file. The
//! payload is one solid stream holding every packaged file, indexed by the
//! metadata's `payload` list, which records where each entry's bytes lie in
//! the uncompressed stream. Integrity is the SHA-256 of each block, the
//! SHA-256 of the decompressed engine, a CRC-32 per payload entry and a
//! CRC-32 over the footer. Nothing here touches the Windows API: the builder,
//! the loader and the engine all read and write installers through this
//! crate, and the builder and the engine both compute package-identity
//! derivations from [`identity`], so the sides cannot drift apart.

pub mod compose;
pub mod footer;
pub mod identity;
pub mod installer;
pub mod metadata;
pub mod payload;
pub mod window;

use std::fmt;

pub use footer::{FOOTER_LEN, Footer};
pub use installer::{EntryInfo, Installer, Layout, Payload, VerifyOutcome};
pub use metadata::Metadata;

/// Every failure the format layer can report, with a stable machine-readable
/// code and a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError {
    pub code: &'static str,
    pub message: String,
}

impl FormatError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for FormatError {}

impl From<std::io::Error> for FormatError {
    /// An I/O error that carries a format error — a payload entry's CRC or
    /// truncation, reported through `Read` — keeps its own code.
    fn from(err: std::io::Error) -> Self {
        let message = err.to_string();
        match err
            .into_inner()
            .and_then(|inner| inner.downcast::<FormatError>().ok())
        {
            Some(format_error) => *format_error,
            None => FormatError::new("io_error", message),
        }
    }
}

impl From<prost::DecodeError> for FormatError {
    fn from(err: prost::DecodeError) -> Self {
        FormatError::new("metadata_invalid", err.to_string())
    }
}

/// Lower-case hex of a SHA-256 digest.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// SHA-256 of a byte slice.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

/// SHA-256 of everything a reader yields, computed in 64 KiB chunks.
pub fn sha256_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().into())
}
