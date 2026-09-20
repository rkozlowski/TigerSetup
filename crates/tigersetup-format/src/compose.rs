//! Composition: loader ‖ compressed engine ‖ payload ‖ compressed metadata
//! ‖ footer, written to one file in one pass.

use std::io::{Read, Seek, Write};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::footer::{FORMAT_MAJOR, FORMAT_MINOR, Footer};
use crate::metadata::PayloadEntry;
use crate::payload::{
    Compression, PayloadStats, PayloadWriter, compress, decompress_exact, stored_frame,
};
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

/// The metadata block as composed: the serialized message tree, and the
/// bytes the file carries for it.
pub struct MetadataBlock {
    pub serialized: Vec<u8>,
    pub compressed: Vec<u8>,
}

impl MetadataBlock {
    /// Serializes and compresses `metadata` under `compression`. The
    /// serialized bytes are the metadata's identity, which the footer
    /// records beside the hash of the compressed block; the block is one
    /// Zstandard frame under the same profile as the payload and the
    /// engine — the window is naturally no wider than the content.
    pub fn compress(
        metadata: &Metadata,
        compression: Compression,
    ) -> Result<MetadataBlock, FormatError> {
        Self::encode(metadata, |serialized| compress(serialized, compression))
    }

    /// Serializes `metadata` and stores it in a frame the decoder reads
    /// without any compressor having written it (`payload::stored_frame`):
    /// the block of the uninstaller copy the engine composes.
    pub fn stored(metadata: &Metadata) -> Result<MetadataBlock, FormatError> {
        Self::encode(metadata, |serialized| Ok(stored_frame(serialized)))
    }

    fn encode(
        metadata: &Metadata,
        frame: impl FnOnce(&[u8]) -> Result<Vec<u8>, FormatError>,
    ) -> Result<MetadataBlock, FormatError> {
        let serialized = metadata.encode_to_vec();
        if serialized.is_empty() {
            return Err(FormatError::new(
                "metadata_invalid",
                "the metadata serializes to nothing",
            ));
        }
        let compressed = frame(&serialized)?;
        // The block must come back as it went in, before it is written:
        // a composition never records a hash the reader cannot reproduce.
        let round_trip = decompress_exact(&compressed, serialized.len() as u64)
            .map_err(|err| FormatError::new("metadata_invalid", err.message))?;
        if round_trip != serialized {
            return Err(FormatError::new(
                "metadata_invalid",
                "the compressed metadata does not decompress to what was serialized",
            ));
        }
        Ok(MetadataBlock {
            serialized,
            compressed,
        })
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
/// files produced and compressed under the same profile, and the footer
/// maps the result. The output must be a fresh, empty stream.
/// `metadata.payload` is replaced by the index this composition wrote.
pub fn compose<W: Write + Seek + Read>(
    out: W,
    loader: &mut dyn Read,
    engine: &EngineBlock,
    metadata: &Metadata,
    files: Vec<PayloadSource>,
    compression: Compression,
) -> Result<Composed, FormatError> {
    let mut uncompressed_length = 0u64;
    for file in &files {
        uncompressed_length = uncompressed_length
            .checked_add(file.length()?)
            .ok_or_else(|| FormatError::new("payload_invalid", "the payload is too large"))?;
    }
    write_installer(
        out,
        loader,
        engine,
        metadata,
        |out| {
            if files.is_empty() {
                return Ok((PayloadStats::default(), Vec::new(), sha256(&[])));
            }
            let mut writer =
                PayloadWriter::new(HashingWriter::new(out), compression, uncompressed_length)?;
            for file in &files {
                let mut source = file.open()?;
                writer.add_entry(&file.entry, &mut *source)?;
            }
            let stats = writer.stats();
            let (sink, index) = writer.finish()?;
            Ok((stats, index, sink.finish()))
        },
        |metadata| MetadataBlock::compress(metadata, compression),
    )
}

/// Writes an installer with no payload — the uninstaller copy the engine
/// keeps in the state directory — around an engine block taken from an
/// existing installer, with the metadata stored in a frame no compressor
/// wrote (`MetadataBlock::stored`). This is the one composition the engine
/// performs, and it reaches no encoder: the engine links the Zstandard
/// decoder alone.
pub fn compose_without_payload<W: Write + Seek + Read>(
    out: W,
    loader: &mut dyn Read,
    engine: &EngineBlock,
    metadata: &Metadata,
) -> Result<Composed, FormatError> {
    write_installer(
        out,
        loader,
        engine,
        metadata,
        |_| Ok((PayloadStats::default(), Vec::new(), sha256(&[]))),
        MetadataBlock::stored,
    )
}

/// The one-pass write both compositions share: loader, engine, the
/// payload `write_payload` produces (with its index and hash), the
/// metadata block `encode_metadata` makes of the metadata carrying that
/// index, and the footer.
fn write_installer<W: Write + Seek + Read>(
    mut out: W,
    loader: &mut dyn Read,
    engine: &EngineBlock,
    metadata: &Metadata,
    write_payload: impl FnOnce(
        &mut W,
    ) -> Result<(PayloadStats, Vec<PayloadEntry>, [u8; 32]), FormatError>,
    encode_metadata: impl FnOnce(&Metadata) -> Result<MetadataBlock, FormatError>,
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
    let (payload_stats, index, payload_sha256) = write_payload(&mut out)?;
    let metadata_offset = out.stream_position()?;
    let payload_length = metadata_offset - payload_offset;

    let mut metadata = metadata.clone();
    metadata.payload = index;
    metadata.validate()?;
    let block = encode_metadata(&metadata)?;
    out.write_all(&block.compressed)?;
    let footer_offset = metadata_offset + block.compressed.len() as u64;

    let footer = Footer {
        format_major: FORMAT_MAJOR,
        format_minor: FORMAT_MINOR,
        engine_offset,
        engine_length: engine.compressed.len() as u64,
        engine_uncompressed_length: engine.uncompressed_length,
        payload_offset,
        payload_length,
        payload_uncompressed_length: payload_stats.uncompressed_bytes,
        metadata_offset,
        metadata_length: block.compressed.len() as u64,
        metadata_uncompressed_length: block.serialized.len() as u64,
        engine_sha256,
        engine_executable_sha256: engine.executable_sha256,
        payload_sha256,
        metadata_block_sha256: sha256(&block.compressed),
        metadata_sha256: sha256(&block.serialized),
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
