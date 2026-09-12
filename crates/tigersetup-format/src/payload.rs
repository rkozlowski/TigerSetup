//! The ZIP payload: a standard archive with Store or DEFLATE entries, entry
//! names equal to install-relative paths, and per-entry CRC-32 from the
//! container itself. Written deterministically: fixed timestamps, no extra
//! fields, entries in the order given.
//!
//! Every entry is either stored or encoded as standard DEFLATE, whatever
//! effort the builder spent choosing. That is deliberate: the container stays
//! an ordinary ZIP any tool can open, the engine needs no second decoder, and
//! how hard the builder searched changes the installer's size without
//! changing what it costs to install.
//!
//! Choosing an encoding has three steps, and the first two exist so that the
//! expensive one is not paid for nothing:
//!
//! 1. **Content already compressed?** A JPEG, an `.mp4`, a nested archive or
//!    a `.woff2` cannot be deflated usefully, and its own header says so.
//!    Those are stored without any codec running.
//! 2. **Large and apparently incompressible?** A few sampled slices, deflated
//!    at the cheapest level, answer that far more cheaply than the whole file
//!    does.
//! 3. **Otherwise encode.** `Compression::Fast` makes one cheap pass;
//!    `Compression::Best` searches the supported effort levels and keeps the
//!    smallest result, storing the entry when none of them beats storing it.

use std::io::{self, Read, Seek, Write};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::FormatError;

/// How much effort the builder spends choosing each entry's encoding.
///
/// Both modes produce a valid installer with identical contents; they differ
/// only in the size of the result and the time spent reaching it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// The release-quality build. Search the supported effort levels and keep
    /// the smallest standard-DEFLATE result: the installer is downloaded and
    /// installed far more often than it is built, so build CPU is the cheap
    /// side of that trade.
    #[default]
    Best,
    /// The developer and AI iteration build. One cheap pass, no search, so the
    /// edit-build-test loop stays short. The installer is functionally
    /// identical and merely larger.
    Fast,
}

/// The DEFLATE effort levels the release build tries, keeping the smallest
/// result. The list is the search: today the supported set has one useful
/// member, because `flate2` is the only DEFLATE encoder linked in and its
/// level 9 is never beaten by a lower one. A stronger encoder — Zopfli emits
/// an ordinary DEFLATE stream any inflater reads at the usual speed — would
/// be another entry here rather than another code path, but it is a
/// third-party dependency that would also be linked into the shipped engine,
/// which is a decision this list deliberately does not make on its own.
const BEST_LEVELS: &[i64] = &[9];
const FAST_LEVEL: i64 = 1;

/// Files at least this large earn a compressibility probe before the whole of
/// them is handed to a codec. Below it the probe would cost about as much as
/// the answer.
const PROBE_THRESHOLD: usize = 1 << 20;
/// How much of a large file the probe reads, as three slices taken from its
/// start, middle and end so that a compressible header cannot speak for an
/// incompressible body.
const PROBE_SLICE: usize = 64 * 1024;
const PROBE_SLICES: usize = 3;
/// A probe result at or above this ratio means deflating the whole file is
/// not worth the time. Deliberately close to 1: the probe may only skip work
/// that was going to be pointless, never work that would have paid.
const PROBE_INCOMPRESSIBLE_RATIO: f64 = 0.98;

/// What was decided for one entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Stored,
    Deflated(i64),
}

impl Encoding {
    fn method(self) -> CompressionMethod {
        match self {
            Encoding::Stored => CompressionMethod::Stored,
            Encoding::Deflated(_) => CompressionMethod::Deflated,
        }
    }

    fn level(self) -> Option<i64> {
        match self {
            Encoding::Stored => None,
            Encoding::Deflated(level) => Some(level),
        }
    }
}

/// How the payload's entries were encoded, for the build report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PayloadStats {
    pub entries: usize,
    /// Entries written without a codec, because compressing them would not
    /// have paid.
    pub stored: usize,
    /// Entries stored on the evidence of their own format signature, with no
    /// codec run at all.
    pub stored_by_signature: usize,
    /// Entries stored on the evidence of a sampled probe.
    pub stored_by_probe: usize,
    /// The bytes the entries hold before encoding.
    pub uncompressed_bytes: u64,
}

/// Writes payload entries under a compression policy.
pub struct PayloadWriter<W: Write + Seek> {
    zip: ZipWriter<W>,
    compression: Compression,
    stats: PayloadStats,
}

impl<W: Write + Seek> PayloadWriter<W> {
    pub fn new(inner: W, compression: Compression) -> Self {
        Self {
            zip: ZipWriter::new(inner),
            compression,
            stats: PayloadStats::default(),
        }
    }

    /// Adds one entry from in-memory bytes.
    pub fn add_entry(&mut self, name: &str, bytes: &[u8]) -> Result<(), FormatError> {
        let (encoding, reason) = choose(name, bytes, self.compression)?;
        self.stats.entries += 1;
        self.stats.uncompressed_bytes += bytes.len() as u64;
        if encoding == Encoding::Stored {
            self.stats.stored += 1;
            match reason {
                StoreReason::Signature => self.stats.stored_by_signature += 1,
                StoreReason::Probe => self.stats.stored_by_probe += 1,
                StoreReason::NotSmaller => {}
            }
        }
        let mut options = SimpleFileOptions::default()
            .compression_method(encoding.method())
            .last_modified_time(zip::DateTime::default())
            .large_file(bytes.len() as u64 >= u32::MAX as u64);
        if let Some(level) = encoding.level() {
            options = options.compression_level(Some(level));
        }
        self.zip.start_file(name, options)?;
        self.zip.write_all(bytes)?;
        Ok(())
    }

    /// How the entries written so far were encoded.
    pub fn stats(&self) -> PayloadStats {
        self.stats
    }

    /// Finishes the central directory and returns the inner writer positioned
    /// after the archive.
    pub fn finish(self) -> Result<W, FormatError> {
        Ok(self.zip.finish()?)
    }
}

/// Why an entry ended up stored, which is what the build report counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreReason {
    /// Its own format signature says the bytes are already compressed.
    Signature,
    /// A sampled probe found nothing to gain.
    Probe,
    /// It was encoded, and the result was no smaller than the bytes.
    NotSmaller,
}

fn choose(
    name: &str,
    bytes: &[u8],
    compression: Compression,
) -> io::Result<(Encoding, StoreReason)> {
    if bytes.is_empty() {
        return Ok((Encoding::Stored, StoreReason::Signature));
    }
    if is_precompressed(name, bytes) {
        return Ok((Encoding::Stored, StoreReason::Signature));
    }
    if bytes.len() >= PROBE_THRESHOLD && probe_is_incompressible(bytes)? {
        return Ok((Encoding::Stored, StoreReason::Probe));
    }

    let levels: &[i64] = match compression {
        Compression::Fast => &[FAST_LEVEL],
        Compression::Best => BEST_LEVELS,
    };
    let mut best: Option<(i64, u64)> = None;
    for &level in levels {
        let size = deflated_size(bytes, level)?;
        if best.is_none_or(|(_, smallest)| size < smallest) {
            best = Some((level, size));
        }
    }
    match best {
        Some((level, size)) if size < bytes.len() as u64 => {
            Ok((Encoding::Deflated(level), StoreReason::NotSmaller))
        }
        _ => Ok((Encoding::Stored, StoreReason::NotSmaller)),
    }
}

/// The size `bytes` deflates to at `level`, without keeping the result.
fn deflated_size(bytes: &[u8], level: i64) -> io::Result<u64> {
    let mut encoder = flate2::write::DeflateEncoder::new(
        CountingSink::default(),
        flate2::Compression::new(flate2_level(level)),
    );
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?.0)
}

/// `flate2` has ten levels. A Zopfli effort level is measured with the
/// strongest `flate2` setting, because that is the closest cheap predictor of
/// what Zopfli will achieve and the search only has to rank the candidates.
fn flate2_level(level: i64) -> u32 {
    level.clamp(0, 9) as u32
}

/// Deflates a few sampled slices and reports whether the whole file is worth
/// deflating. Sampling is what makes this affordable: the answer costs a few
/// hundred kilobytes of work whatever the file's size.
fn probe_is_incompressible(bytes: &[u8]) -> io::Result<bool> {
    let mut sampled = 0usize;
    let mut produced = 0u64;
    for index in 0..PROBE_SLICES {
        let Some(slice) = probe_slice(bytes, index) else {
            continue;
        };
        sampled += slice.len();
        produced += deflated_size(slice, FAST_LEVEL)?;
    }
    if sampled == 0 {
        return Ok(false);
    }
    Ok(produced as f64 / sampled as f64 >= PROBE_INCOMPRESSIBLE_RATIO)
}

/// The `index`-th of `PROBE_SLICES` slices, taken from the start, the middle
/// and the end of `bytes`.
fn probe_slice(bytes: &[u8], index: usize) -> Option<&[u8]> {
    let length = bytes.len();
    let span = PROBE_SLICE.min(length);
    let last = PROBE_SLICES - 1;
    let start = match index {
        0 => 0,
        i if i == last => length - span,
        i => (length - span) * i / last,
    };
    bytes.get(start..start + span)
}

/// Whether the bytes are already in a compressed container or codec, judged
/// by their own leading bytes and, where a format has no distinctive
/// signature, by the entry's extension.
///
/// The question this answers is only "would a codec be wasted here?", so a
/// false negative merely costs the time it was going to cost anyway, and the
/// probe catches most of those.
fn is_precompressed(name: &str, bytes: &[u8]) -> bool {
    const SIGNATURES: &[&[u8]] = &[
        b"PK\x03\x04",         // ZIP and everything built on it
        b"PK\x05\x06",         // an empty ZIP
        b"\x1f\x8b",           // gzip
        b"\xfd7zXZ\x00",       // xz
        b"\x28\xb5\x2f\xfd",   // zstd
        b"7z\xbc\xaf\x27\x1c", // 7z
        b"Rar!",               // RAR
        b"BZh",                // bzip2
        b"MSCF",               // cabinet
        b"\x89PNG\r\n\x1a\n",  // PNG
        b"\xff\xd8\xff",       // JPEG
        b"GIF8",               // GIF
        b"OggS",               // Ogg
        b"fLaC",               // FLAC
        b"ID3",                // MP3 with a tag
        b"\x1aE\xdf\xa3",      // Matroska and WebM
        b"wOFF",               // WOFF
        b"wOF2",               // WOFF2
    ];
    if SIGNATURES.iter().any(|prefix| bytes.starts_with(prefix)) {
        return true;
    }
    // RIFF containers name their form in the second field.
    if bytes.starts_with(b"RIFF") && matches!(bytes.get(8..12), Some(b"WEBP")) {
        return true;
    }
    // ISO base media (MP4, MOV, M4A) puts its brand after the box length.
    if matches!(bytes.get(4..8), Some(b"ftyp")) {
        return true;
    }

    const EXTENSIONS: &[&str] = &[
        "7z", "avi", "br", "bz2", "cab", "flac", "gif", "gz", "jpeg", "jpg", "lz4", "lzma", "m4a",
        "m4v", "mkv", "mov", "mp3", "mp4", "nupkg", "ogg", "opus", "png", "rar", "vsix", "webm",
        "webp", "woff", "woff2", "xz", "zip", "zst",
    ];
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    matches!(extension, Some(ext) if EXTENSIONS.contains(&ext.as_str()))
}

#[derive(Default)]
struct CountingSink(u64);

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Opens an archive over any `Read + Seek` block.
pub fn open_archive<R: Read + Seek>(block: R) -> Result<zip::ZipArchive<R>, FormatError> {
    Ok(zip::ZipArchive::new(block)?)
}

/// Streams an entry into `sink`, returning the number of bytes copied. The ZIP
/// reader verifies the entry's CRC-32 when the stream ends.
pub fn copy_entry<R: Read + Seek, W: Write>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    sink: &mut W,
) -> Result<u64, FormatError> {
    let mut entry = archive.by_name(name).map_err(|err| {
        FormatError::new(
            "payload_entry_missing",
            format!("payload entry {name:?}: {err}"),
        )
    })?;
    Ok(io::copy(&mut entry, sink)?)
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

    #[test]
    fn entries_round_trip_with_the_smaller_method() {
        let mut writer = PayloadWriter::new(Cursor::new(Vec::new()), Compression::Best);
        let compressible = vec![b'a'; 10_000];
        let random = noise(10_000);
        writer.add_entry("dir/text.txt", &compressible).unwrap();
        writer.add_entry("noise.bin", &random).unwrap();
        writer.add_entry("empty", &[]).unwrap();
        let stats = writer.stats();
        let bytes = writer.finish().unwrap().into_inner();

        assert_eq!(stats.entries, 3);
        assert_eq!(stats.uncompressed_bytes, 20_000);

        let mut archive = open_archive(Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 3);
        assert_eq!(
            archive.by_name("dir/text.txt").unwrap().compression(),
            CompressionMethod::Deflated
        );
        assert_eq!(
            archive.by_name("noise.bin").unwrap().compression(),
            CompressionMethod::Stored
        );
        let mut out = Vec::new();
        assert_eq!(
            copy_entry(&mut archive, "dir/text.txt", &mut out).unwrap(),
            10_000
        );
        assert_eq!(out, compressible);
        assert_eq!(
            copy_entry(&mut archive, "missing", &mut out)
                .unwrap_err()
                .code,
            "payload_entry_missing"
        );
    }

    #[test]
    fn writing_is_deterministic() {
        let build = |compression| {
            let mut writer = PayloadWriter::new(Cursor::new(Vec::new()), compression);
            writer.add_entry("a", b"hello hello hello hello").unwrap();
            writer.add_entry("b/c", b"x").unwrap();
            writer.finish().unwrap().into_inner()
        };
        assert_eq!(build(Compression::Best), build(Compression::Best));
        assert_eq!(build(Compression::Fast), build(Compression::Fast));
    }

    #[test]
    fn both_modes_produce_the_same_installed_bytes() {
        let payload = vec![b'x'; 200_000];
        let extract = |compression| {
            let mut writer = PayloadWriter::new(Cursor::new(Vec::new()), compression);
            writer.add_entry("app/data.bin", &payload).unwrap();
            let bytes = writer.finish().unwrap().into_inner();
            let mut archive = open_archive(Cursor::new(bytes)).unwrap();
            let mut out = Vec::new();
            copy_entry(&mut archive, "app/data.bin", &mut out).unwrap();
            out
        };
        assert_eq!(extract(Compression::Best), payload);
        assert_eq!(extract(Compression::Fast), payload);
    }

    #[test]
    fn the_release_build_is_never_larger_than_the_fast_one() {
        // English-like text: compressible, and enough of it that the effort
        // levels can differ.
        let text = "the quick brown fox jumps over the lazy dog. "
            .repeat(4_000)
            .into_bytes();
        let size = |compression| {
            let mut writer = PayloadWriter::new(Cursor::new(Vec::new()), compression);
            writer.add_entry("readme.txt", &text).unwrap();
            writer.finish().unwrap().into_inner().len()
        };
        assert!(size(Compression::Best) <= size(Compression::Fast));
    }

    #[test]
    fn a_signature_stores_an_already_compressed_entry_without_running_a_codec() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        // Deliberately compressible after the header: only the signature
        // should decide, because a PNG's bytes are already deflated.
        png.extend(std::iter::repeat_n(b'a', 4_000));
        let (encoding, reason) = choose("logo.png", &png, Compression::Best).unwrap();
        assert_eq!(encoding, Encoding::Stored);
        assert_eq!(reason, StoreReason::Signature);

        let unknown_extension = choose("logo.dat", &png, Compression::Best).unwrap();
        assert_eq!(unknown_extension.0, Encoding::Stored);
        assert_eq!(unknown_extension.1, StoreReason::Signature);
    }

    #[test]
    fn an_extension_stores_a_container_with_no_distinctive_signature() {
        let (encoding, reason) = choose("clip.avi", &[b'\0'; 64], Compression::Best).unwrap();
        assert_eq!(encoding, Encoding::Stored);
        assert_eq!(reason, StoreReason::Signature);
    }

    #[test]
    fn a_large_incompressible_file_is_stored_on_the_probe_alone() {
        let big = noise(4 << 20);
        let (encoding, reason) = choose("movie.raw", &big, Compression::Best).unwrap();
        assert_eq!(encoding, Encoding::Stored);
        assert_eq!(reason, StoreReason::Probe);
    }

    #[test]
    fn a_large_compressible_file_survives_the_probe() {
        let big = vec![b'z'; 4 << 20];
        let (encoding, _) = choose("payload.bin", &big, Compression::Best).unwrap();
        assert!(matches!(encoding, Encoding::Deflated(_)));
    }

    #[test]
    fn the_probe_reads_the_body_and_not_only_the_head() {
        // Three slices, from the start, the middle and the end, and they must
        // not overlap or the middle and the end of a file are never looked at.
        let bytes = noise(4 << 20);
        let slices: Vec<&[u8]> = (0..PROBE_SLICES)
            .map(|index| probe_slice(&bytes, index).expect("a slice per position"))
            .collect();
        let offset = |slice: &[u8]| slice.as_ptr() as usize - bytes.as_ptr() as usize;
        assert_eq!(offset(slices[0]), 0, "the first slice starts at the start");
        assert_eq!(
            offset(slices[PROBE_SLICES - 1]) + PROBE_SLICE,
            bytes.len(),
            "the last slice ends at the end"
        );
        for pair in slices.windows(2) {
            assert!(
                offset(pair[1]) >= offset(pair[0]) + PROBE_SLICE,
                "the slices must not overlap, or one part of the file speaks for the rest"
            );
        }
    }

    #[test]
    fn a_compressible_head_still_earns_its_compression() {
        // The probe is deliberately biased towards compressing: it may only
        // skip work that was going to be pointless. A quarter-megabyte of
        // compressible header on an otherwise incompressible file is a real
        // saving, so the file is deflated even though most of it will not
        // shrink — and the result is still far larger than the same amount of
        // wholly compressible data, which is what proves the body was read.
        let mut mixed = vec![b'a'; 256 * 1024];
        mixed.extend(noise(4 << 20));
        let (encoding, _) = choose("mixed.bin", &mixed, Compression::Best).unwrap();
        assert!(
            matches!(encoding, Encoding::Deflated(_)),
            "a compressible head is worth deflating for"
        );
        let deflated = deflated_size(&mixed, 9).unwrap();
        assert!(
            deflated > (mixed.len() as u64) * 9 / 10,
            "the incompressible body is still incompressible: {deflated} of {}",
            mixed.len()
        );
    }

    #[test]
    fn the_probe_counts_what_it_skipped() {
        let mut writer = PayloadWriter::new(Cursor::new(Vec::new()), Compression::Best);
        writer.add_entry("movie.raw", &noise(4 << 20)).unwrap();
        writer
            .add_entry("logo.png", b"\x89PNG\r\n\x1a\n....")
            .unwrap();
        writer.add_entry("readme.txt", &vec![b'a'; 8_000]).unwrap();
        let stats = writer.stats();
        assert_eq!(stats.entries, 3);
        assert_eq!(stats.stored, 2);
        assert_eq!(stats.stored_by_probe, 1);
        assert_eq!(stats.stored_by_signature, 1);
    }
}
