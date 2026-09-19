//! `cspike`: the TigerSetup compression-architecture spike tool. Disposable
//! benchmark machinery (benchmark/compression-spike/README.md); nothing in
//! the product depends on it.

mod classify;
mod codec;
mod inventory;
mod layout;
mod run;
mod worker;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use codec::{Codec, Setting};

#[derive(Parser)]
#[command(name = "cspike", about = "TigerSetup compression-architecture spike")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(clap::Args, Debug, Clone)]
struct SettingArgs {
    #[arg(long, value_enum)]
    codec: CodecArg,
    #[arg(long, default_value_t = 0)]
    level: i32,
    #[arg(long)]
    window_log: Option<u32>,
    #[arg(long, default_value_t = false)]
    extreme: bool,
    #[arg(long, default_value_t = false)]
    bcj: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum CodecArg {
    Store,
    Deflate,
    Zstd,
    Lzma2,
}

impl From<SettingArgs> for Setting {
    fn from(a: SettingArgs) -> Self {
        Setting {
            codec: match a.codec {
                CodecArg::Store => Codec::Store,
                CodecArg::Deflate => Codec::Deflate,
                CodecArg::Zstd => Codec::Zstd,
                CodecArg::Lzma2 => Codec::Lzma2,
            },
            level: a.level,
            window_log: a.window_log,
            extreme: a.extreme,
            bcj: a.bcj,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Inventory one payload: classification, families, the DEFLATE-9
    /// baseline, and what the candidates make of every stored file alone.
    Inventory {
        #[arg(long)]
        app: String,
        #[arg(long)]
        payload: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Write the ordered file list for a partition and a layout.
    Layout {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long, value_enum)]
        partition: layout::Partition,
        #[arg(long, value_enum)]
        layout: layout::Layout,
        #[arg(long)]
        out: PathBuf,
    },
    /// Run every job of a plan, appending to the results file.
    Run {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        results: PathBuf,
    },
    /// Print the codec libraries and versions this build links.
    Versions,
    /// The measured work; run by `run` in a child process.
    Worker {
        #[command(subcommand)]
        work: Work,
    },
}

#[derive(Subcommand)]
enum Work {
    Compress {
        #[arg(long)]
        list: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        block_bytes: Option<u64>,
        #[command(flatten)]
        setting: SettingArgs,
    },
    Decompress {
        #[arg(long = "in")]
        input: PathBuf,
        #[command(flatten)]
        setting: SettingArgs,
    },
    PerFile {
        #[arg(long)]
        list: PathBuf,
        #[command(flatten)]
        setting: SettingArgs,
    },
    Warm {
        #[arg(long)]
        list: PathBuf,
    },
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("cspike: {e}");
        std::process::exit(1);
    }
}

fn real_main() -> std::io::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Inventory { app, payload, out } => {
            let alone = [
                Setting {
                    codec: Codec::Zstd,
                    level: 19,
                    window_log: None,
                    extreme: false,
                    bcj: false,
                },
                Setting {
                    codec: Codec::Lzma2,
                    level: 9,
                    window_log: None,
                    extreme: false,
                    bcj: false,
                },
            ];
            let inventory = inventory::build(&app, &payload, &alone)?;
            std::fs::write(&out, serde_json::to_string(&inventory)?)?;
            eprintln!(
                "{app}: {} files, {} bytes, 0.7.1 baseline {} bytes ({:.1}% of input), deflate pass {:.1}s",
                inventory.file_count,
                inventory.total_bytes,
                inventory.current_total_bytes,
                100.0 * inventory.current_total_bytes as f64 / inventory.total_bytes.max(1) as f64,
                inventory.baseline_deflate_wall_s
            );
        }
        Cmd::Layout {
            inventory,
            partition,
            layout,
            out,
        } => {
            let inv: inventory::Inventory =
                serde_json::from_str(&std::fs::read_to_string(inventory)?)?;
            let list = layout::build(&inv, partition, layout);
            std::fs::write(&out, serde_json::to_string(&list)?)?;
        }
        Cmd::Run { plan, results } => run::run(&plan, &results)?,
        Cmd::Versions => {
            let versions = serde_json::json!({
                "cspike": env!("CARGO_PKG_VERSION"),
                "libzstd": zstd::zstd_safe::version_string(),
                "liblzma": codec::effective_parameters(&Setting { codec: Codec::Lzma2, level: 9, window_log: None, extreme: false, bcj: false }, 0)["library"],
                "flate2": "1.1 (zlib-rs backend)",
                "rustc": option_env!("CSPIKE_RUSTC").unwrap_or("see Cargo.lock / rustc --version"),
            });
            println!("{}", serde_json::to_string_pretty(&versions)?);
        }
        Cmd::Worker { work } => {
            let report = match work {
                Work::Compress {
                    list,
                    out,
                    block_bytes,
                    setting,
                } => serde_json::to_string(&worker::compress(
                    &list,
                    &setting.into(),
                    block_bytes,
                    &out,
                )?)?,
                Work::Decompress { input, setting } => {
                    serde_json::to_string(&worker::decompress(&input, &setting.into())?)?
                }
                Work::PerFile { list, setting } => {
                    serde_json::to_string(&worker::per_file(&list, &setting.into())?)?
                }
                Work::Warm { list } => {
                    let (bytes, wall) = worker::warm(&list)?;
                    serde_json::json!({ "bytes": bytes, "wall_s": wall }).to_string()
                }
            };
            println!("{report}");
        }
    }
    Ok(())
}
