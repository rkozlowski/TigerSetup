//! Composition: engine ‖ metadata ‖ payload ‖ footer, written to one file.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::footer::{FORMAT_MAJOR, FORMAT_MINOR, Footer};
use crate::payload::{Compression, PayloadStats, PayloadWriter};
use crate::window::{OffsetWriter, Window};
use crate::{FormatError, Metadata, sha256, sha256_reader};

/// One file to place in the payload: its entry name and its bytes.
pub struct PayloadSource {
    pub entry: String,
    pub bytes: Vec<u8>,
}

/// What composing produced: the footer that maps the file, and how the
/// payload's entries were encoded.
pub struct Composed {
    pub footer: Footer,
    pub payload: PayloadStats,
}

/// Writes a complete installer to `out` and returns what it wrote.
///
/// `engine` is copied verbatim, `metadata` is encoded, the payload is built
/// from `files` in the order given (each produced when its turn comes, so a
/// caller can read them lazily) under `compression`, and the footer maps the
/// result. The output must be a fresh, empty stream.
pub fn compose<W: Write + Seek + Read>(
    mut out: W,
    engine: &mut dyn Read,
    metadata: &Metadata,
    files: impl IntoIterator<Item = Result<PayloadSource, FormatError>>,
    compression: Compression,
) -> Result<Composed, FormatError> {
    metadata.validate()?;
    let engine_len = std::io::copy(engine, &mut out)?;
    if engine_len == 0 {
        return Err(FormatError::new(
            "engine_invalid",
            "the engine block is empty",
        ));
    }

    let metadata_bytes = metadata.encode_to_vec();
    let metadata_offset = engine_len;
    out.write_all(&metadata_bytes)?;
    let metadata_sha256 = sha256(&metadata_bytes);

    let payload_offset = metadata_offset + metadata_bytes.len() as u64;
    let mut payload = PayloadWriter::new(OffsetWriter::new(out)?, compression);
    for file in files {
        let file = file?;
        payload.add_entry(&file.entry, &file.bytes)?;
    }
    let payload_stats = payload.stats();
    let mut out = payload.finish()?.into_inner();
    let footer_offset = out.stream_position()?;
    let payload_length = footer_offset - payload_offset;

    out.seek(SeekFrom::Start(payload_offset))?;
    let payload_sha256 = {
        let mut window = Window::new(&mut out, payload_offset, payload_length)?;
        sha256_reader(&mut window)?
    };
    out.seek(SeekFrom::Start(footer_offset))?;

    let footer = Footer {
        format_major: FORMAT_MAJOR,
        format_minor: FORMAT_MINOR,
        metadata_offset,
        metadata_length: metadata_bytes.len() as u64,
        payload_offset,
        payload_length,
        metadata_sha256,
        payload_sha256,
    };
    out.write_all(&footer.encode())?;
    out.flush()?;
    Ok(Composed {
        footer,
        payload: payload_stats,
    })
}
