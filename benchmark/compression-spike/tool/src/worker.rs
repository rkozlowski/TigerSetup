//! The measured work, run in a child process of its own so that its wall time
//! and peak memory are its own: stream the listed files through one encoder
//! (or one encoder per bounded block), stream a compressed file back through
//! the decoder into a CRC, or compress every file independently.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::codec::{self, Decoder, Encoder, Setting};
use crate::inventory::CountingSink;
use crate::layout::FileList;

const BUFFER: usize = 1 << 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub files: u64,
    pub uncompressed: u64,
    pub compressed: u64,
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressReport {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub wall_s: f64,
    pub files: u64,
    pub blocks: Vec<Block>,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompressReport {
    pub bytes: u64,
    pub crc32: u32,
    pub wall_s: f64,
    pub blocks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerFileReport {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub wall_s: f64,
    pub files: u64,
    /// Files whose independent result was no smaller than the file.
    pub not_smaller_files: u64,
    pub not_smaller_bytes: u64,
}

fn read_list(path: &Path) -> io::Result<FileList> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(io::Error::other)
}

fn stream_file(path: &Path, sink: &mut impl Write, buffer: &mut [u8]) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut total = 0u64;
    loop {
        let n = file.read(buffer)?;
        if n == 0 {
            break;
        }
        sink.write_all(&buffer[..n])?;
        total += n as u64;
    }
    Ok(total)
}

/// Splits the list into blocks: a block closes after the file that brings
/// it to `block_bytes` or more, so every block holds whole files and files
/// larger than the target sit alone.
fn plan_blocks(
    root: &Path,
    files: &[String],
    block_bytes: Option<u64>,
) -> io::Result<Vec<Vec<(PathBuf, u64)>>> {
    let sizes: Vec<(PathBuf, u64)> = files
        .iter()
        .map(|f| {
            let path = root.join(f);
            std::fs::metadata(&path).map(|m| (path, m.len()))
        })
        .collect::<io::Result<_>>()?;
    let Some(target) = block_bytes else {
        return Ok(vec![sizes]);
    };
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    let mut current_bytes = 0u64;
    for (path, size) in sizes {
        current.push((path, size));
        current_bytes += size;
        if current_bytes >= target {
            blocks.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    Ok(blocks)
}

pub fn compress(
    list: &Path,
    setting: &Setting,
    block_bytes: Option<u64>,
    out: &Path,
) -> io::Result<CompressReport> {
    let list = read_list(list)?;
    let root = PathBuf::from(&list.payload_root);
    let blocks = plan_blocks(&root, &list.files, block_bytes)?;
    let mut buffer = vec![0u8; BUFFER];
    let mut table = Vec::with_capacity(blocks.len());
    let mut input = 0u64;
    let mut offset = 0u64;
    let parameters = codec::effective_parameters(setting, list.admitted_bytes);

    let started = Instant::now();
    let mut writer = BufWriter::with_capacity(BUFFER, File::create(out)?);
    for block in &blocks {
        let pledged: u64 = block.iter().map(|(_, s)| s).sum();
        let mut encoder = Encoder::new(
            setting,
            Counting {
                inner: writer,
                count: 0,
            },
            pledged,
        )?;
        for (path, _) in block {
            input += stream_file(path, &mut encoder, &mut buffer)?;
        }
        let counting = encoder.finish()?;
        writer = counting.inner;
        table.push(Block {
            files: block.len() as u64,
            uncompressed: pledged,
            compressed: counting.count,
            offset,
        });
        offset += counting.count;
    }
    writer.flush()?;
    let wall = started.elapsed().as_secs_f64();
    drop(writer);

    let report = CompressReport {
        input_bytes: input,
        output_bytes: offset,
        wall_s: wall,
        files: list.files.len() as u64,
        blocks: table,
        parameters,
    };
    std::fs::write(
        sidecar(out),
        serde_json::to_string(&report.blocks).map_err(io::Error::other)?,
    )?;
    Ok(report)
}

pub fn sidecar(out: &Path) -> PathBuf {
    let mut p = out.as_os_str().to_owned();
    p.push(".blocks.json");
    PathBuf::from(p)
}

pub fn decompress(input: &Path, setting: &Setting) -> io::Result<DecompressReport> {
    let blocks: Vec<Block> = serde_json::from_str(&std::fs::read_to_string(sidecar(input))?)
        .map_err(io::Error::other)?;
    let mut buffer = vec![0u8; BUFFER];
    let mut hasher = crc32fast::Hasher::new();
    let mut total = 0u64;

    let started = Instant::now();
    let mut file = File::open(input)?;
    for block in &blocks {
        let take = (&mut file).take(block.compressed);
        let mut decoder = Decoder::new(setting, take)?;
        let mut produced = 0u64;
        loop {
            let n = decoder.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            produced += n as u64;
        }
        if produced != block.uncompressed {
            return Err(io::Error::other(format!(
                "block produced {produced} bytes, expected {}",
                block.uncompressed
            )));
        }
        total += produced;
    }
    let wall = started.elapsed().as_secs_f64();
    Ok(DecompressReport {
        bytes: total,
        crc32: hasher.finalize(),
        wall_s: wall,
        blocks: blocks.len() as u64,
    })
}

pub fn per_file(list: &Path, setting: &Setting) -> io::Result<PerFileReport> {
    let list = read_list(list)?;
    let root = PathBuf::from(&list.payload_root);
    let mut input = 0u64;
    let mut output = 0u64;
    let mut not_smaller = (0u64, 0u64);
    let started = Instant::now();
    for relative in &list.files {
        let bytes = std::fs::read(root.join(relative))?;
        let size = crate::inventory::encoded_size(setting, &bytes)?;
        input += bytes.len() as u64;
        if size >= bytes.len() as u64 {
            not_smaller.0 += 1;
            not_smaller.1 += bytes.len() as u64;
            output += bytes.len() as u64;
        } else {
            output += size;
        }
    }
    Ok(PerFileReport {
        input_bytes: input,
        output_bytes: output,
        wall_s: started.elapsed().as_secs_f64(),
        files: list.files.len() as u64,
        not_smaller_files: not_smaller.0,
        not_smaller_bytes: not_smaller.1,
    })
}

/// Counts what passes through to the writer beneath.
pub struct Counting<W: Write> {
    inner: W,
    count: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Reads the whole payload once so that the file cache is warm before a
/// timed run, and reports how long the cold read took.
pub fn warm(list: &Path) -> io::Result<(u64, f64)> {
    let list = read_list(list)?;
    let root = PathBuf::from(&list.payload_root);
    let mut buffer = vec![0u8; BUFFER];
    let started = Instant::now();
    let mut total = 0u64;
    for relative in &list.files {
        total += stream_file(&root.join(relative), &mut CountingSink(0), &mut buffer)?;
    }
    Ok((total, started.elapsed().as_secs_f64()))
}
