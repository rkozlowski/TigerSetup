//! The payload: one solid Zstandard stream holding every packaged file, in
//! the order the metadata's `payload` index records, and the same codec for
//! the compressed engine block.
//!
//! Every file goes into the one stream — there is no per-file compression,
//! no compressibility probe, no signature or extension classifier and no
//! raw region for files that look compressed already. The spike that chose
//! the codec (`benchmark/compression-spike/report.md`) measured all of
//! those and found that admitting everything to the solid stream is both
//! simpler and slightly smaller. What the stream buys is the cross-file
//! matching that an installer's payload is full of — duplicated binaries,
//! satellite assemblies, resource files that share most of their bytes — and
//! what it costs is that reading one entry means decoding the stream from
//! its start up to that entry. The engine reads the stream in index order,
//! so an install decodes it once, sequentially; a repair or a single-file
//! extraction pays the skip, and that is by design: fast sequential decode
//! is why Zstandard was chosen over the codec with the smaller output.
//!
//! The index is data, not a rule: the builder records each entry's offset
//! and length in the metadata, and the engine never has to reproduce the
//! ordering the builder used. Each entry also carries a CRC-32 of its bytes,
//! checked as the entry is read, so an index that mis-slices the stream can
//! never install the wrong bytes.
//!
//! Encoding is single-threaded and deterministic: the same files in the
//! same order under the same profile produce the same bytes, in separate
//! processes and on separate days (the spike verified this for the exact
//! settings used here).

use std::collections::HashMap;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};

use sha2::Digest;

use crate::FormatError;
use crate::metadata::PayloadEntry;

/// How much effort the builder spends encoding the payload and the engine.
///
/// Both modes produce a valid installer with identical contents; they differ
/// only in the size of the result and the time spent reaching it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// The release-quality build: Zstandard level 19, a 128 MiB window
    /// (`windowLog` 27) and long-distance matching — the `zstd-19-w27`
    /// setting of the compression spike. The installer is downloaded and
    /// installed far more often than it is built, so build CPU is the cheap
    /// side of that trade.
    #[default]
    Best,
    /// The developer and AI iteration build: Zstandard level 3 with the
    /// default window, so the edit-build-test loop stays short. The
    /// installer is functionally identical and merely larger.
    Fast,
}

impl Compression {
    /// The Zstandard settings of this profile: the level, and the window
    /// log (with long-distance matching) when it is widened.
    pub fn zstd_settings(self) -> ZstdSettings {
        match self {
            Compression::Best => ZstdSettings {
                level: BEST_LEVEL,
                window_log: Some(BEST_WINDOW_LOG),
            },
            Compression::Fast => ZstdSettings {
                level: FAST_LEVEL,
                window_log: None,
            },
        }
    }

    /// The setting's name as the compression spike named it, for reports.
    pub fn describe(self) -> String {
        let settings = self.zstd_settings();
        match settings.window_log {
            Some(log) => format!("zstd-{}-w{log}", settings.level),
            None => format!("zstd-{}", settings.level),
        }
    }
}

/// The level and window of the release profile (`zstd-19-w27`).
pub const BEST_LEVEL: i32 = 19;
pub const BEST_WINDOW_LOG: u32 = 27;
/// The level of the iteration profile.
pub const FAST_LEVEL: i32 = 3;

/// The widest window any TigerSetup stream uses, and therefore the widest
/// the decoder accepts: a stream asking for more is not one of ours.
pub const MAX_WINDOW_LOG: u32 = BEST_WINDOW_LOG;

/// The Zstandard encoder parameters of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZstdSettings {
    pub level: i32,
    pub window_log: Option<u32>,
}

/// A Zstandard encoder over `sink` for `uncompressed_length` bytes under
/// `compression`: single-threaded, the content size in the frame header,
/// no frame checksum (the block is hashed whole and every entry carries a
/// CRC), so the output is a function of the input alone.
pub fn encoder<W: Write>(
    sink: W,
    compression: Compression,
    uncompressed_length: u64,
) -> Result<zstd::stream::write::Encoder<'static, W>, FormatError> {
    let settings = compression.zstd_settings();
    let mut encoder = zstd::stream::write::Encoder::new(sink, settings.level)?;
    if let Some(window_log) = settings.window_log {
        encoder.set_parameter(zstd::stream::raw::CParameter::WindowLog(window_log))?;
        encoder.long_distance_matching(true)?;
    }
    encoder.set_pledged_src_size(Some(uncompressed_length))?;
    encoder.include_checksum(false)?;
    encoder.include_contentsize(true)?;
    Ok(encoder)
}

/// A Zstandard decoder over `source`, accepting any window a TigerSetup
/// stream may use.
pub fn decoder<R: Read>(
    source: R,
) -> Result<zstd::stream::read::Decoder<'static, BufReader<R>>, FormatError> {
    let mut decoder = zstd::stream::read::Decoder::new(source)?;
    decoder.window_log_max(MAX_WINDOW_LOG)?;
    Ok(decoder)
}

/// Compresses `bytes` whole under `compression`: the engine block and the
/// metadata block.
pub fn compress(bytes: &[u8], compression: Compression) -> Result<Vec<u8>, FormatError> {
    let mut encoder = encoder(Vec::new(), compression, bytes.len() as u64)?;
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?)
}

/// The largest block a Zstandard frame may carry.
const STORED_BLOCK_MAX: usize = 128 * 1024;

/// `bytes` as one Zstandard frame that stores them: the frame header with
/// the content size and a single segment (the window is the content), no
/// checksum — as the encoder writes its frames — then raw blocks of at
/// most 128 KiB, the last one flagged. The decoder reads it like any other
/// frame, and no compressor writes it: the engine composes the uninstaller
/// copy's metadata block this way (`compose::compose_without_payload`)
/// and links the decoder alone. Deterministic in the bytes alone.
pub fn stored_frame(bytes: &[u8]) -> Vec<u8> {
    let mut frame =
        Vec::with_capacity(bytes.len() + 16 + 3 * bytes.len().div_ceil(STORED_BLOCK_MAX));
    // Magic number, then the frame header descriptor: an 8-byte frame
    // content size (flag 3), the single-segment flag, no checksum, no
    // dictionary.
    frame.extend_from_slice(&0xFD2F_B528u32.to_le_bytes());
    frame.push(0b1110_0000);
    frame.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    let mut blocks = bytes.chunks(STORED_BLOCK_MAX).peekable();
    if blocks.peek().is_none() {
        // An empty frame still ends with one (empty) last block.
        frame.extend_from_slice(&[0x01, 0x00, 0x00]);
    }
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        // Block header: bit 0 last, bits 1-2 the type (raw), bits 3-23 the size.
        let header = (block.len() as u32) << 3 | u32::from(last);
        frame.extend_from_slice(&header.to_le_bytes()[..3]);
        frame.extend_from_slice(block);
    }
    frame
}

/// Decompresses a whole block that must decode to exactly `length` bytes:
/// the metadata block, whose length the footer declares. The decoder is
/// bounded by that declaration — a frame that would produce more is
/// stopped at the bound rather than read to its end — so a damaged or
/// hostile block can neither exhaust memory nor be accepted short.
pub fn decompress_exact(compressed: &[u8], length: u64) -> Result<Vec<u8>, FormatError> {
    let bound = usize::try_from(length).map_err(|_| {
        FormatError::new("block_invalid", "the block's declared length is too large")
    })?;
    let invalid = |why: String| FormatError::new("block_invalid", why);
    let mut decoder = decoder(compressed)?;
    let mut out = Vec::with_capacity(bound);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = decoder
            .read(&mut buffer)
            .map_err(|err| invalid(format!("the block cannot be decompressed: {err}")))?;
        if n == 0 {
            break;
        }
        if out.len() + n > bound {
            return Err(invalid(
                "the block decompresses to more than the footer declares".into(),
            ));
        }
        out.extend_from_slice(&buffer[..n]);
    }
    if out.len() != bound {
        return Err(invalid(format!(
            "the block decompresses to {} bytes, the footer declares {length}",
            out.len()
        )));
    }
    Ok(out)
}

/// What the payload holds, for the build report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PayloadStats {
    pub entries: usize,
    /// The bytes the entries hold before encoding: the stream's length.
    pub uncompressed_bytes: u64,
}

/// Writes the payload stream: entries in the order given, each recorded in
/// the index with its offset, length and CRC-32.
pub struct PayloadWriter<W: Write> {
    encoder: zstd::stream::write::Encoder<'static, W>,
    position: u64,
    index: Vec<PayloadEntry>,
}

impl<W: Write> PayloadWriter<W> {
    /// Starts a stream that will hold exactly `uncompressed_length` bytes.
    pub fn new(
        sink: W,
        compression: Compression,
        uncompressed_length: u64,
    ) -> Result<Self, FormatError> {
        Ok(Self {
            encoder: encoder(sink, compression, uncompressed_length)?,
            position: 0,
            index: Vec::new(),
        })
    }

    /// Appends one entry from a reader, returning its index record.
    pub fn add_entry(
        &mut self,
        name: &str,
        source: &mut dyn Read,
    ) -> Result<PayloadEntry, FormatError> {
        let mut crc = crc32fast::Hasher::new();
        let mut sha = sha2::Sha256::new();
        let mut length = 0u64;
        let mut buffer = vec![0u8; 256 * 1024];
        loop {
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            crc.update(&buffer[..n]);
            sha.update(&buffer[..n]);
            self.encoder.write_all(&buffer[..n])?;
            length += n as u64;
        }
        let entry = PayloadEntry {
            entry: name.to_string(),
            offset: self.position,
            length,
            crc32: crc.finalize(),
            sha256: crate::hex(&sha.finalize()),
        };
        self.position += length;
        self.index.push(entry.clone());
        Ok(entry)
    }

    pub fn stats(&self) -> PayloadStats {
        PayloadStats {
            entries: self.index.len(),
            uncompressed_bytes: self.position,
        }
    }

    /// Ends the frame and returns the sink and the index.
    pub fn finish(self) -> Result<(W, Vec<PayloadEntry>), FormatError> {
        let sink = self.encoder.finish()?;
        Ok((sink, self.index))
    }
}

/// One region of the index, as the reader keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Region {
    offset: u64,
    length: u64,
    crc32: u32,
    sha256: String,
}

/// Reads entries out of the solid stream through a streaming decoder that
/// never holds more than the decoder's window in memory.
///
/// Reading is sequential in the stream: opening an entry ahead of the
/// current position decodes and discards the bytes in between; opening one
/// behind it restarts the decoder from the beginning of the stream. An
/// engine that reads entries in index order therefore decodes the stream
/// exactly once.
pub struct PayloadReader<R: Read + Seek> {
    /// The decoder over the compressed block, or the block itself between
    /// restarts. `None` only transiently, while a restart is in progress.
    state: Option<Decoding<R>>,
    /// The logical position: how many uncompressed bytes were consumed.
    position: u64,
    index: HashMap<String, Region>,
    order: Vec<PayloadEntry>,
    /// Whether the block has any bytes at all; an empty payload has no
    /// frame to decode.
    empty: bool,
}

enum Decoding<R: Read + Seek> {
    Idle(R),
    Active(zstd::stream::read::Decoder<'static, BufReader<R>>),
}

impl<R: Read + Seek> PayloadReader<R> {
    /// Opens the stream over `block` — the compressed payload block, as a
    /// self-contained reader starting at its first byte — with the index
    /// the metadata records for it. `block_length` is the compressed
    /// length; an empty block is a payload with no entries.
    pub fn new(block: R, block_length: u64, index: &[PayloadEntry]) -> Result<Self, FormatError> {
        let mut map = HashMap::with_capacity(index.len());
        for entry in index {
            map.insert(
                entry.entry.clone(),
                Region {
                    offset: entry.offset,
                    length: entry.length,
                    crc32: entry.crc32,
                    sha256: entry.sha256.clone(),
                },
            );
        }
        Ok(Self {
            state: Some(Decoding::Idle(block)),
            position: 0,
            index: map,
            order: index.to_vec(),
            empty: block_length == 0,
        })
    }

    /// The entries in stream order.
    pub fn entries(&self) -> &[PayloadEntry] {
        &self.order
    }

    /// Whether the index names `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// The index record of `name`.
    pub fn region(&self, name: &str) -> Result<PayloadEntry, FormatError> {
        let region = self.index.get(name).ok_or_else(|| missing(name))?;
        Ok(PayloadEntry {
            entry: name.to_string(),
            offset: region.offset,
            length: region.length,
            crc32: region.crc32,
            sha256: region.sha256.clone(),
        })
    }

    /// Positions the stream at the start of `name` and returns a reader
    /// over exactly its bytes, which checks the entry's CRC-32 when the last
    /// byte has been read.
    pub fn by_name(&mut self, name: &str) -> Result<Entry<'_, R>, FormatError> {
        let region = self.index.get(name).ok_or_else(|| missing(name))?.clone();
        if region.length > 0 && self.empty {
            return Err(FormatError::new(
                "payload_invalid",
                format!("payload entry {name:?}: the payload block is empty"),
            ));
        }
        if region.length > 0 {
            self.seek_to(region.offset, name)?;
        }
        Ok(Entry {
            reader: self,
            name: name.to_string(),
            remaining: region.length,
            crc: crc32fast::Hasher::new(),
            expected_crc: region.crc32,
            checked: region.length == 0,
        })
    }

    /// Copies the entry `name` into `sink` and returns its length.
    pub fn copy_entry<W: Write>(&mut self, name: &str, sink: &mut W) -> Result<u64, FormatError> {
        let mut entry = self.by_name(name)?;
        let mut buffer = vec![0u8; 256 * 1024];
        let mut copied = 0u64;
        loop {
            let n = entry.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            sink.write_all(&buffer[..n])?;
            copied += n as u64;
        }
        Ok(copied)
    }

    /// The SHA-256 of the entry `name`, computed by reading it.
    pub fn sha256_of(&mut self, name: &str) -> Result<[u8; 32], FormatError> {
        let mut entry = self.by_name(name)?;
        Ok(crate::sha256_reader(&mut entry)?)
    }

    fn seek_to(&mut self, offset: u64, name: &str) -> Result<(), FormatError> {
        if offset < self.position {
            self.restart()?;
        }
        while self.position < offset {
            let wanted = (offset - self.position).min(256 * 1024) as usize;
            let mut buffer = vec![0u8; wanted];
            let n = self.read_stream(&mut buffer)?;
            if n == 0 {
                return Err(FormatError::new(
                    "payload_truncated",
                    format!(
                        "payload entry {name:?} starts at {offset}, but the stream ends at {}",
                        self.position
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Puts the decoder back at the start of the stream.
    fn restart(&mut self) -> Result<(), FormatError> {
        let block = match self.state.take() {
            Some(Decoding::Idle(block)) => block,
            Some(Decoding::Active(decoder)) => decoder.finish().into_inner(),
            None => {
                return Err(FormatError::new(
                    "payload_invalid",
                    "the payload reader was left mid-restart",
                ));
            }
        };
        self.state = Some(Decoding::Idle(block));
        self.position = 0;
        Ok(())
    }

    /// Reads decoded bytes at the current position, starting the decoder if
    /// it is not running.
    fn read_stream(&mut self, buffer: &mut [u8]) -> Result<usize, FormatError> {
        if matches!(self.state, Some(Decoding::Idle(_))) {
            let Some(Decoding::Idle(mut block)) = self.state.take() else {
                unreachable!()
            };
            block.seek(SeekFrom::Start(0))?;
            let decoder = decoder(block)?;
            self.state = Some(Decoding::Active(decoder));
        }
        let Some(Decoding::Active(decoder)) = self.state.as_mut() else {
            return Err(FormatError::new(
                "payload_invalid",
                "the payload reader was left mid-restart",
            ));
        };
        let n = decoder.read(buffer).map_err(|err| {
            FormatError::new(
                "payload_invalid",
                format!(
                    "the payload stream cannot be decoded at {}: {err}",
                    self.position
                ),
            )
        })?;
        self.position += n as u64;
        Ok(n)
    }
}

fn missing(name: &str) -> FormatError {
    FormatError::new(
        "payload_entry_missing",
        format!("payload entry {name:?}: not in the payload index"),
    )
}

/// One entry being read: exactly its bytes, then the CRC check.
pub struct Entry<'a, R: Read + Seek> {
    reader: &'a mut PayloadReader<R>,
    name: String,
    remaining: u64,
    crc: crc32fast::Hasher,
    expected_crc: u32,
    checked: bool,
}

impl<R: Read + Seek> Entry<'_, R> {
    /// The bytes still to read.
    pub fn remaining(&self) -> u64 {
        self.remaining
    }
}

impl<R: Read + Seek> Read for Entry<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            if !self.checked {
                self.checked = true;
                let actual = std::mem::take(&mut self.crc).finalize();
                if actual != self.expected_crc {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        FormatError::new(
                            "payload_entry_crc_mismatch",
                            format!(
                                "payload entry {:?}: CRC-32 is {actual:08x}, the index records {:08x}",
                                self.name, self.expected_crc
                            ),
                        ),
                    ));
                }
            }
            return Ok(0);
        }
        let limit = usize::try_from(self.remaining)
            .unwrap_or(usize::MAX)
            .min(buf.len());
        let n = self
            .reader
            .read_stream(&mut buf[..limit])
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                FormatError::new(
                    "payload_truncated",
                    format!(
                        "payload entry {:?}: the stream ended with {} bytes still to read",
                        self.name, self.remaining
                    ),
                ),
            ));
        }
        self.crc.update(&buf[..n]);
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// The stream order the builder writes: entries in the reserved
/// `.tigersetup/` directory first — the dependency installers the engine
/// needs before the transaction and the action programs it needs at its
/// start — then the product files by extension, then by path.
///
/// Extension-then-path is the ordering the spike measured as a consistent
/// gain over plain path order with no classifier: files of one kind share
/// bytes, and putting them side by side keeps those bytes inside the
/// window. The comparison is by bytes, so it is the same on every machine.
pub fn stream_order(reserved: &mut [String], product: &mut [String]) {
    reserved.sort();
    sort_product_entries(product, |name| name.as_str());
}

/// Sorts product entries into stream order by the name `name_of` yields:
/// extension first, then path, both compared as bytes.
pub fn sort_product_entries<T>(items: &mut [T], name_of: impl Fn(&T) -> &str) {
    items.sort_by(|a, b| {
        let (a, b) = (name_of(a), name_of(b));
        extension_of(a)
            .cmp(&extension_of(b))
            .then_with(|| a.as_bytes().cmp(b.as_bytes()))
    });
}

/// The lower-cased extension of an entry name, or an empty string.
fn extension_of(name: &str) -> String {
    let file_name = name.rsplit('/').next().unwrap_or(name);
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn noise(length: usize) -> Vec<u8> {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect()
    }

    fn write(files: &[(&str, &[u8])], compression: Compression) -> (Vec<u8>, Vec<PayloadEntry>) {
        let total: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
        let mut writer = PayloadWriter::new(Vec::new(), compression, total).unwrap();
        for (name, bytes) in files {
            writer.add_entry(name, &mut &bytes[..]).unwrap();
        }
        writer.finish().unwrap()
    }

    fn open(bytes: Vec<u8>, index: &[PayloadEntry]) -> PayloadReader<Cursor<Vec<u8>>> {
        let length = bytes.len() as u64;
        PayloadReader::new(Cursor::new(bytes), length, index).unwrap()
    }

    #[test]
    fn entries_round_trip_in_any_order() {
        let compressible = vec![b'a'; 10_000];
        let random = noise(10_000);
        let files: &[(&str, &[u8])] = &[
            ("dir/text.txt", &compressible),
            ("noise.bin", &random),
            ("empty", &[]),
        ];
        let (bytes, index) = write(files, Compression::Best);
        assert_eq!(index.len(), 3);
        assert_eq!(index[1].offset, 10_000);
        assert_eq!(index[2].offset, 20_000);
        assert_eq!(index[2].length, 0);

        let mut reader = open(bytes, &index);
        let mut out = Vec::new();
        // Behind the position, ahead of it, then back again.
        reader.copy_entry("noise.bin", &mut out).unwrap();
        assert_eq!(out, random);
        out.clear();
        reader.copy_entry("dir/text.txt", &mut out).unwrap();
        assert_eq!(out, compressible);
        out.clear();
        reader.copy_entry("empty", &mut out).unwrap();
        assert!(out.is_empty());
        assert_eq!(
            reader.copy_entry("missing", &mut out).unwrap_err().code,
            "payload_entry_missing"
        );
    }

    #[test]
    fn writing_is_deterministic() {
        let build = |compression| {
            write(
                &[("a", b"hello hello hello hello"), ("b/c", b"x")],
                compression,
            )
            .0
        };
        assert_eq!(build(Compression::Best), build(Compression::Best));
        assert_eq!(build(Compression::Fast), build(Compression::Fast));
    }

    #[test]
    fn both_profiles_produce_the_same_bytes_on_extraction() {
        let payload = vec![b'x'; 200_000];
        let extract = |compression| {
            let (bytes, index) = write(&[("app/data.bin", &payload)], compression);
            let mut reader = open(bytes, &index);
            let mut out = Vec::new();
            reader.copy_entry("app/data.bin", &mut out).unwrap();
            out
        };
        assert_eq!(extract(Compression::Best), payload);
        assert_eq!(extract(Compression::Fast), payload);
    }

    #[test]
    fn the_release_profile_is_never_larger_than_the_fast_one() {
        let text = "the quick brown fox jumps over the lazy dog. "
            .repeat(4_000)
            .into_bytes();
        let size = |compression| write(&[("readme.txt", &text)], compression).0.len();
        assert!(size(Compression::Best) <= size(Compression::Fast));
    }

    #[test]
    fn a_wrong_crc_in_the_index_fails_the_read() {
        let (bytes, mut index) = write(&[("a.bin", &noise(5_000))], Compression::Fast);
        index[0].crc32 ^= 1;
        let mut reader = open(bytes, &index);
        let err = reader.copy_entry("a.bin", &mut Vec::new()).unwrap_err();
        assert_eq!(err.code, "payload_entry_crc_mismatch");
    }

    #[test]
    fn a_region_past_the_end_of_the_stream_fails_safely() {
        let (bytes, mut index) = write(&[("a.bin", &noise(5_000))], Compression::Fast);
        index[0].offset = 4_000;
        index[0].length = 5_000;
        let mut reader = open(bytes, &index);
        let err = reader.copy_entry("a.bin", &mut Vec::new()).unwrap_err();
        assert_eq!(err.code, "payload_truncated");
        index[0].offset = 9_000;
        index[0].length = 1;
        let (bytes, _) = write(&[("a.bin", &noise(5_000))], Compression::Fast);
        let mut reader = open(bytes, &index);
        let err = reader.copy_entry("a.bin", &mut Vec::new()).unwrap_err();
        assert_eq!(err.code, "payload_truncated");
    }

    #[test]
    fn a_corrupted_stream_fails_safely() {
        let (mut bytes, index) = write(&[("a.bin", &noise(50_000))], Compression::Fast);
        let middle = bytes.len() / 2;
        for b in &mut bytes[middle..middle + 16] {
            *b ^= 0xAA;
        }
        let mut reader = open(bytes, &index);
        let err = reader.copy_entry("a.bin", &mut Vec::new()).unwrap_err();
        assert!(
            err.code == "payload_invalid" || err.code == "payload_entry_crc_mismatch",
            "{err}"
        );
    }

    #[test]
    fn an_empty_payload_serves_no_entry() {
        let mut reader = PayloadReader::new(
            Cursor::new(Vec::new()),
            0,
            &[PayloadEntry {
                entry: "x".into(),
                offset: 0,
                length: 3,
                crc32: 0,
                sha256: String::new(),
            }],
        )
        .unwrap();
        assert_eq!(
            reader.copy_entry("x", &mut Vec::new()).unwrap_err().code,
            "payload_invalid"
        );
    }

    #[test]
    fn stream_order_groups_by_extension_then_path() {
        let mut reserved = vec![
            ".tigersetup/dependencies/b.exe".to_string(),
            ".tigersetup/actions/a.ps1".to_string(),
        ];
        let mut product = vec![
            "bin/z.dll".to_string(),
            "README".to_string(),
            "bin/a.exe".to_string(),
            "lib/B.DLL".to_string(),
            "bin/a.dll".to_string(),
            ".hidden".to_string(),
        ];
        stream_order(&mut reserved, &mut product);
        assert_eq!(
            reserved,
            vec![
                ".tigersetup/actions/a.ps1",
                ".tigersetup/dependencies/b.exe"
            ]
        );
        assert_eq!(
            product,
            vec![
                ".hidden",
                "README",
                "bin/a.dll",
                "bin/z.dll",
                "lib/B.DLL",
                "bin/a.exe"
            ]
        );
    }

    /// A stored frame is a Zstandard frame the ordinary decoder reads, of
    /// exactly the bytes it was given, for content of every size around
    /// the block limit.
    #[test]
    fn a_stored_frame_decodes_to_its_bytes_without_the_compressor() {
        for length in [
            0usize,
            1,
            100,
            STORED_BLOCK_MAX - 1,
            STORED_BLOCK_MAX,
            STORED_BLOCK_MAX + 1,
            3 * STORED_BLOCK_MAX + 7,
        ] {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 7 % 251) as u8).collect();
            let frame = stored_frame(&bytes);
            assert_eq!(
                frame.len(),
                bytes.len() + 13 + 3 * bytes.len().div_ceil(STORED_BLOCK_MAX).max(1)
            );
            assert_eq!(
                decompress_exact(&frame, length as u64).unwrap(),
                bytes,
                "{length}"
            );
        }
    }

    #[test]
    fn the_engine_block_round_trips() {
        let engine = noise(100_000);
        let compressed = compress(&engine, Compression::Best).unwrap();
        let mut decoder = decoder(Cursor::new(compressed)).unwrap();
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap();
        assert_eq!(out, engine);
    }
}
