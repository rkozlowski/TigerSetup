//! One payload's inventory: every file, what 0.7.1 decides about it, the
//! family it belongs to, its DEFLATE-9 size (the baseline), and — for the
//! files 0.7.1 stores — what zstd and LZMA2 would make of each on its own.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::classify::{self, Decision, Family};
use crate::codec::{Codec, Encoder, Setting};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub ext: Option<String>,
    pub family: Family,
    pub decision: Decision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_ratio: Option<f64>,
    /// DEFLATE-9 size when 0.7.1 deflates the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deflate9_bytes: Option<u64>,
    /// What 0.7.1 writes for this entry: stored bytes, or the DEFLATE-9
    /// result when that is smaller.
    pub current_bytes: u64,
    /// For files 0.7.1 stores: the file compressed alone by the candidates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alone: Option<BTreeMap<String, u64>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bucket {
    pub files: u64,
    pub bytes: u64,
    pub current_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub app: String,
    pub payload_root: String,
    pub file_count: u64,
    pub total_bytes: u64,
    /// The 0.7.1 baseline: the sum of every entry as the product writes it.
    pub current_total_bytes: u64,
    /// Wall time of the per-file DEFLATE-9 pass, single-threaded.
    pub baseline_deflate_wall_s: f64,
    pub by_decision: BTreeMap<String, Bucket>,
    pub by_family: BTreeMap<String, Bucket>,
    pub files: Vec<FileEntry>,
}

pub fn build(app: &str, payload: &Path, alone_settings: &[Setting]) -> std::io::Result<Inventory> {
    let mut paths = Vec::new();
    walk(payload, payload, &mut paths)?;
    // The product's install order: byte order of the relative path.
    paths.sort();

    let mut files = Vec::with_capacity(paths.len());
    let mut deflate_wall = 0f64;
    let mut by_decision: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut by_family: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut not_smaller = Bucket::default();

    for (relative, full) in &paths {
        let bytes = std::fs::read(full)?;
        let (decision, probe_ratio) = classify::decide(relative, &bytes);
        let family = classify::family(relative, &bytes[..bytes.len().min(4096)]);
        let mut deflate9 = None;
        let current = match decision {
            Decision::Deflate => {
                let started = Instant::now();
                let size = classify::deflated_size(&bytes, 9);
                deflate_wall += started.elapsed().as_secs_f64();
                deflate9 = Some(size);
                if size < bytes.len() as u64 {
                    size
                } else {
                    not_smaller.files += 1;
                    not_smaller.bytes += bytes.len() as u64;
                    not_smaller.current_bytes += bytes.len() as u64;
                    bytes.len() as u64
                }
            }
            _ => bytes.len() as u64,
        };
        let alone = if decision != Decision::Deflate && !bytes.is_empty() {
            let mut map = BTreeMap::new();
            for setting in alone_settings {
                map.insert(setting.label(), encoded_size(setting, &bytes)?);
            }
            Some(map)
        } else {
            None
        };
        let decision_name = match decision {
            Decision::Signature => "signature",
            Decision::Probe => "probe",
            Decision::Deflate => "deflate",
        };
        for bucket in [
            by_decision.entry(decision_name.to_string()).or_default(),
            by_family.entry(family.name().to_string()).or_default(),
        ] {
            bucket.files += 1;
            bucket.bytes += bytes.len() as u64;
            bucket.current_bytes += current;
        }
        files.push(FileEntry {
            path: relative.clone(),
            bytes: bytes.len() as u64,
            ext: classify::extension(relative),
            family,
            decision,
            probe_ratio,
            deflate9_bytes: deflate9,
            current_bytes: current,
            alone,
        });
    }
    by_decision.insert("deflate_not_smaller".to_string(), not_smaller);

    Ok(Inventory {
        app: app.to_string(),
        payload_root: payload.display().to_string(),
        file_count: files.len() as u64,
        total_bytes: files.iter().map(|f| f.bytes).sum(),
        current_total_bytes: files.iter().map(|f| f.current_bytes).sum(),
        baseline_deflate_wall_s: deflate_wall,
        by_decision,
        by_family,
        files,
    })
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, out)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((relative, path));
        }
    }
    Ok(())
}

pub fn encoded_size(setting: &Setting, bytes: &[u8]) -> std::io::Result<u64> {
    if setting.codec == Codec::Store {
        return Ok(bytes.len() as u64);
    }
    let mut encoder = Encoder::new(setting, CountingSink(0), bytes.len() as u64)?;
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?.0)
}

pub struct CountingSink(pub u64);

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
