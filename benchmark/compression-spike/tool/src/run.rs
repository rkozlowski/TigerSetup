//! Runs a plan: for every job, a child `cspike worker` process per
//! measurement, its wall time and peak memory read from the process itself,
//! and the results appended to a JSON file as they land so that an
//! interrupted run resumes where it stopped.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::codec::Setting;
use crate::worker::{CompressReport, DecompressReport, PerFileReport};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub app: String,
    pub partition: String,
    pub layout: String,
    pub list: String,
    pub setting: Setting,
    #[serde(default)]
    pub block_bytes: Option<u64>,
    #[serde(default = "one")]
    pub compress_reps: u32,
    #[serde(default = "three")]
    pub decompress_reps: u32,
    /// Compress every file independently instead of one stream.
    #[serde(default)]
    pub per_file: bool,
}

fn one() -> u32 {
    1
}
fn three() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default = "one")]
    pub parallelism: u32,
    pub work_dir: String,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub peak_working_set: u64,
    pub peak_pagefile: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measured<T> {
    pub report: T,
    pub process_wall_s: f64,
    pub memory: Memory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub id: String,
    pub app: String,
    pub partition: String,
    pub layout: String,
    pub label: String,
    pub setting: Setting,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_bytes: Option<u64>,
    pub admitted_files: u64,
    pub admitted_bytes: u64,
    pub raw_files: u64,
    pub raw_bytes: u64,
    pub group_order: Vec<(String, u64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compress: Option<Measured<CompressReport>>,
    /// SHA-256 of every compressed output produced for this job; equal
    /// entries are the determinism evidence.
    pub output_sha256: Vec<String>,
    /// Compressed bytes plus the raw (stored) bytes: what the payload block
    /// would hold.
    pub total_bytes: u64,
    pub decompress_wall_s: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decompress_median_wall_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decompress_memory: Option<Memory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decompress_crc32: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_file: Option<Measured<PerFileReport>>,
    pub finished_utc: String,
    pub host: String,
}

fn peak_memory(child: &std::process::Child) -> Memory {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    let mut counters = unsafe { std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok =
        unsafe { K32GetProcessMemoryInfo(child.as_raw_handle() as _, &mut counters, counters.cb) };
    if ok == 0 {
        return Memory {
            peak_working_set: 0,
            peak_pagefile: 0,
        };
    }
    Memory {
        peak_working_set: counters.PeakWorkingSetSize as u64,
        peak_pagefile: counters.PeakPagefileUsage as u64,
    }
}

fn run_worker<T: for<'a> Deserialize<'a>>(args: &[String]) -> io::Result<Measured<T>> {
    let exe = std::env::current_exe()?;
    let started = Instant::now();
    let mut child = Command::new(exe)
        .arg("worker")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let output = {
        let mut stdout = child.stdout.take().expect("piped");
        let mut buf = Vec::new();
        io::Read::read_to_end(&mut stdout, &mut buf)?;
        buf
    };
    let status = child.wait()?;
    let process_wall_s = started.elapsed().as_secs_f64();
    let memory = peak_memory(&child);
    if !status.success() {
        return Err(io::Error::other(format!(
            "worker {:?} failed with {status}",
            args
        )));
    }
    let report: T = serde_json::from_slice(&output).map_err(|e| {
        io::Error::other(format!(
            "worker output was not the expected JSON: {e}: {}",
            String::from_utf8_lossy(&output)
        ))
    })?;
    Ok(Measured {
        report,
        process_wall_s,
        memory,
    })
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn setting_args(setting: &Setting) -> Vec<String> {
    let mut args = vec![
        "--codec".into(),
        serde_json::to_value(setting.codec)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string(),
        "--level".into(),
        setting.level.to_string(),
    ];
    if let Some(w) = setting.window_log {
        args.push("--window-log".into());
        args.push(w.to_string());
    }
    if setting.extreme {
        args.push("--extreme".into());
    }
    if setting.bcj {
        args.push("--bcj".into());
    }
    args
}

fn now_utc() -> String {
    // Seconds since the epoch, rendered without a date library: a stable
    // "when" for the results file.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

fn run_job(job: &Job, work_dir: &Path, host: &str) -> io::Result<JobResult> {
    let list: crate::layout::FileList =
        serde_json::from_str(&std::fs::read_to_string(&job.list)?).map_err(io::Error::other)?;
    let label = job.setting.label();
    let file_stem = job.id.replace(['/', '\\', ':'], "_");
    let out = work_dir.join(format!("{file_stem}.bin"));
    let mut result = JobResult {
        id: job.id.clone(),
        app: job.app.clone(),
        partition: job.partition.clone(),
        layout: job.layout.clone(),
        label: label.clone(),
        setting: job.setting.clone(),
        block_bytes: job.block_bytes,
        admitted_files: list.admitted_files,
        admitted_bytes: list.admitted_bytes,
        raw_files: list.raw_files,
        raw_bytes: list.raw_bytes,
        group_order: list.group_order.clone(),
        compress: None,
        output_sha256: Vec::new(),
        total_bytes: 0,
        decompress_wall_s: Vec::new(),
        decompress_median_wall_s: None,
        decompress_memory: None,
        decompress_crc32: None,
        per_file: None,
        finished_utc: String::new(),
        host: host.to_string(),
    };

    if job.per_file {
        let mut args = vec!["per-file".to_string(), "--list".into(), job.list.clone()];
        args.extend(setting_args(&job.setting));
        let measured: Measured<PerFileReport> = run_worker(&args)?;
        result.total_bytes = measured.report.output_bytes + list.raw_bytes;
        result.per_file = Some(measured);
        result.finished_utc = now_utc();
        return Ok(result);
    }

    for rep in 0..job.compress_reps.max(1) {
        let mut args = vec![
            "compress".to_string(),
            "--list".into(),
            job.list.clone(),
            "--out".into(),
            out.display().to_string(),
        ];
        args.extend(setting_args(&job.setting));
        if let Some(b) = job.block_bytes {
            args.push("--block-bytes".into());
            args.push(b.to_string());
        }
        let measured: Measured<CompressReport> = run_worker(&args)?;
        result.output_sha256.push(sha256_file(&out)?);
        if rep == 0 {
            result.total_bytes = measured.report.output_bytes + list.raw_bytes;
            result.compress = Some(measured);
        }
    }

    let mut memory: Option<Memory> = None;
    for _ in 0..job.decompress_reps {
        let mut args = vec![
            "decompress".to_string(),
            "--in".into(),
            out.display().to_string(),
        ];
        args.extend(setting_args(&job.setting));
        let measured: Measured<DecompressReport> = run_worker(&args)?;
        if measured.report.bytes != list.admitted_bytes {
            return Err(io::Error::other(format!(
                "{}: decompressed {} bytes, expected {}",
                job.id, measured.report.bytes, list.admitted_bytes
            )));
        }
        result.decompress_wall_s.push(measured.report.wall_s);
        result.decompress_crc32 = Some(measured.report.crc32);
        memory = Some(match memory {
            Some(m) => Memory {
                peak_working_set: m.peak_working_set.max(measured.memory.peak_working_set),
                peak_pagefile: m.peak_pagefile.max(measured.memory.peak_pagefile),
            },
            None => measured.memory,
        });
    }
    if !result.decompress_wall_s.is_empty() {
        let mut sorted = result.decompress_wall_s.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        result.decompress_median_wall_s = Some(sorted[sorted.len() / 2]);
    }
    result.decompress_memory = memory;
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(crate::worker::sidecar(&out));
    result.finished_utc = now_utc();
    Ok(result)
}

pub fn run(plan_path: &Path, results_path: &Path) -> io::Result<()> {
    let plan: Plan =
        serde_json::from_str(&std::fs::read_to_string(plan_path)?).map_err(io::Error::other)?;
    let work_dir = PathBuf::from(&plan.work_dir);
    std::fs::create_dir_all(&work_dir)?;
    let host = format!(
        "{} / {} logical CPUs",
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default(),
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );

    let mut done: BTreeMap<String, JobResult> = if results_path.exists() {
        let existing: Vec<JobResult> =
            serde_json::from_str(&std::fs::read_to_string(results_path)?)
                .map_err(io::Error::other)?;
        existing.into_iter().map(|r| (r.id.clone(), r)).collect()
    } else {
        BTreeMap::new()
    };
    let queue: VecDeque<Job> = plan
        .jobs
        .iter()
        .filter(|j| !done.contains_key(&j.id))
        .cloned()
        .collect();
    eprintln!(
        "{} jobs, {} already done, parallelism {}",
        plan.jobs.len(),
        plan.jobs.len() - queue.len(),
        plan.parallelism
    );
    let order: Vec<String> = plan.jobs.iter().map(|j| j.id.clone()).collect();
    let queue = Arc::new(Mutex::new(queue));
    let results = Arc::new(Mutex::new(std::mem::take(&mut done)));
    let failures = Arc::new(Mutex::new(Vec::<String>::new()));

    let write = |results: &BTreeMap<String, JobResult>| -> io::Result<()> {
        let ordered: Vec<&JobResult> = order.iter().filter_map(|id| results.get(id)).collect();
        let text = serde_json::to_string_pretty(&ordered).map_err(io::Error::other)?;
        let tmp = results_path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, results_path)?;
        Ok(())
    };

    std::thread::scope(|scope| {
        for _ in 0..plan.parallelism.max(1) {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            let failures = Arc::clone(&failures);
            let work_dir = work_dir.clone();
            let host = host.clone();
            let write = &write;
            scope.spawn(move || {
                loop {
                    let job = {
                        let mut q = queue.lock().unwrap();
                        q.pop_front()
                    };
                    let Some(job) = job else { break };
                    let started = Instant::now();
                    eprintln!("start  {}", job.id);
                    match run_job(&job, &work_dir, &host) {
                        Ok(result) => {
                            eprintln!(
                                "done   {} -> {} bytes in {:.1}s",
                                job.id,
                                result.total_bytes,
                                started.elapsed().as_secs_f64()
                            );
                            let mut r = results.lock().unwrap();
                            r.insert(job.id.clone(), result);
                            if let Err(e) = write(&r) {
                                eprintln!("could not write results: {e}");
                            }
                        }
                        Err(e) => {
                            eprintln!("FAILED {}: {e}", job.id);
                            failures.lock().unwrap().push(format!("{}: {e}", job.id));
                        }
                    }
                }
            });
        }
    });

    let failures = failures.lock().unwrap();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} job(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        )))
    }
}
