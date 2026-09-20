//! Composition: loader ‖ compressed engine ‖ payload ‖ metadata ‖ footer,
//! written to one file in one pass.

use std::io::{Read, Seek, Write};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::footer::{FORMAT_MAJOR, FORMAT_MINOR, Footer};
use crate::payload::{Compression, PayloadStats, PayloadWriter, compress};
use crate::{FormatError, Metadata, sha256};

/// Where one payload entry's bytes come from.
pub enum PayloadBytes {
    Memory(Vec<u8>),
    /// Read when the entry's turn comes, so a build holds one file at a
    /// time.
    File(PathBuf),
}

/// One file to place in the payload: its entry name and its bytes.
pub struct PayloadSource {
    pub entry: String,
    pub bytes: PayloadBytes,
}

impl PayloadSource {
    fn length(&self) -> Result<u64, FormatError> {
        match &self.bytes {
            PayloadBytes::Memory(bytes) => Ok(bytes.len() as u64),
            PayloadBytes::File(path) => Ok(std::fs::metadata(path)
                .map_err(|err| {
                    FormatError::new("io_error", format!("cannot read {}: {err}", path.display()))
                })?
                .len()),
        }
    }

    fn open(&self) -> Result<Box<dyn Read + '_>, FormatError> {
        match &self.bytes {
            PayloadBytes::Memory(bytes) => Ok(Box::new(&bytes[..])),
            PayloadBytes::File(path) => Ok(Box::new(std::fs::File::open(path).map_err(|err| {
                FormatError::new("io_error", format!("cannot read {}: {err}", path.display()))
            })?)),
        }
    }
}

/// The compressed engine block as composed: the bytes the file carries, how
/// long they decompress to, and the hash of what they decompress to — which
/// the loader checks before it executes anything.
pub struct EngineBlock {
    pub compressed: Vec<u8>,
    pub uncompressed_length: u64,
    pub executable_sha256: [u8; 32],
}

impl EngineBlock {
    /// Compresses an engine executable under `compression`.
    pub fn compress(engine: &[u8], compression: Compression) -> Result<EngineBlock, FormatError> {
        if engine.is_empty() {
            return Err(FormatError::new(
                "engine_invalid",
                "the engine executable is empty",
            ));
        }
        Ok(EngineBlock {
            compressed: compress(engine, compression)?,
            uncompressed_length: engine.len() as u64,
            executable_sha256: sha256(engine),
        })
    }

    /// The hash of the compressed bytes.
    pub fn sha256(&self) -> [u8; 32] {
        sha256(&self.compressed)
    }
}

/// What composing produced: the footer that maps the file, the payload
/// index the metadata records, and what the payload held.
pub struct Composed {
    pub footer: Footer,
    pub payload: PayloadStats,
}

/// Writes a complete installer to `out` and returns what it wrote.
///
/// `loader` is copied verbatim, `engine` is the already-compressed engine
/// block, the payload is built from `files` in the order given under
/// `compression`, the metadata is encoded with the payload index those
/// files produced, and the footer maps the result. The output must be a
/// fresh, empty stream. `metadata.payload` is replaced by the index this
/// composition wrote.
pub fn compose<W: Write + Seek + Read>(
    mut out: W,
    loader: &mut dyn Read,
    engine: &EngineBlock,
    metadata: &Metadata,
    files: Vec<PayloadSource>,
    compression: Compression,
) -> Result<Composed, FormatError> {
    let loader_len = std::io::copy(loader, &mut out)?;
    if loader_len == 0 {
        return Err(FormatError::new(
            "loader_invalid",
            "the loader block is empty",
        ));
    }
    if engine.compressed.is_empty() || engine.uncompressed_length == 0 {
        return Err(FormatError::new(
            "engine_invalid",
            "the engine block is empty",
        ));
    }

    let engine_offset = loader_len;
    out.write_all(&engine.compressed)?;
    let engine_sha256 = engine.sha256();

    // The payload: one stream, every entry, the index as a by-product.
    let payload_offset = engine_offset + engine.compressed.len() as u64;
    let mut uncompressed_length = 0u64;
    for file in &files {
        uncompressed_length = uncompressed_length
            .checked_add(file.length()?)
            .ok_or_else(|| FormatError::new("payload_invalid", "the payload is too large"))?;
    }
    let (payload_stats, index, payload_sha256) = if files.is_empty() {
        (PayloadStats::default(), Vec::new(), sha256(&[]))
    } else {
        let mut writer = PayloadWriter::new(
            HashingWriter::new(&mut out),
            compression,
            uncompressed_length,
        )?;
        for file in &files {
            let mut source = file.open()?;
            writer.add_entry(&file.entry, &mut *source)?;
        }
        let stats = writer.stats();
        let (sink, index) = writer.finish()?;
        (stats, index, sink.finish())
    };
    let metadata_offset = out.stream_position()?;
    let payload_length = metadata_offset - payload_offset;

    let mut metadata = metadata.clone();
    metadata.payload = index;
    metadata.validate()?;
    let metadata_bytes = metadata.encode_to_vec();
    out.write_all(&metadata_bytes)?;
    let metadata_sha256 = sha256(&metadata_bytes);
    let footer_offset = metadata_offset + metadata_bytes.len() as u64;

    let footer = Footer {
        format_major: FORMAT_MAJOR,
        format_minor: FORMAT_MINOR,
        engine_offset,
        engine_length: engine.compressed.len() as u64,
        engine_uncompressed_length: engine.uncompressed_length,
        metadata_offset,
        metadata_length: metadata_bytes.len() as u64,
        payload_offset,
        payload_length,
        payload_uncompressed_length: payload_stats.uncompressed_bytes,
        engine_sha256,
        engine_executable_sha256: engine.executable_sha256,
        metadata_sha256,
        payload_sha256,
    };
    debug_assert!(footer.check_layout(footer_offset).is_ok());
    out.write_all(&footer.encode())?;
    out.flush()?;
    Ok(Composed {
        footer,
        payload: payload_stats,
    })
}

/// A writer that hashes what passes through it, so the payload block's
/// SHA-256 is known when the stream ends without reading the block back.
struct HashingWriter<'a, W: Write> {
    inner: &'a mut W,
    hasher: Sha256,
}

impl<'a, W: Write> HashingWriter<'a, W> {
    fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn finish(self) -> [u8; 32] {
        self.hasher.finalize().into()
    }
}

impl<W: Write> Write for HashingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
