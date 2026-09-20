//! `tiger-setup`: the TigerSetup builder command line.
//!
//! ```text
//! tiger-setup build <manifest> [--output <dir|file.exe>] [--engine <path>] [--loader <path>]
//!                              [--property <Name=Value>]... [--offline] [--fast]
//! tiger-setup metadata <manifest> [--property <Name=Value>]... [--json]
//! tiger-setup inspect <Setup.exe> [--json] [--output-payload <file>] [--output-zip <file>]
//!                                 [--output-meta <file>] [--output-meta-json <file>]
//!                                 [--output-engine <file>]
//! tiger-setup verify <Setup.exe>
//! tiger-setup winget prepare <manifest> --installer <Setup.exe> --output <dir>
//! tiger-setup winget finalize <manifest dir> --url <url> --installer <Setup.exe>
//! ```
//!
//! Commands express operations, positional arguments identify their primary
//! subjects, and options modify behaviour; an option takes its value as a
//! separate argument (`--output dir`, never `--output=dir`).
//!
//! Exit codes: 0 success, 1 the installer failed verification, 2 invalid
//! input or a build error.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use std::time::Instant;
use tigersetup_build::metadata::{ResolvedPackage, resolve_package};

use tigersetup_build::inspect::ExportRequest;
use tigersetup_build::{BuildRequest, Compression, build, inspect, winget};

#[derive(Parser)]
#[command(
    name = "tiger-setup",
    version,
    about = "Builds and inspects TigerSetup installers"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build a Setup.exe from a TigerSetup.toml.
    Build {
        /// Path to TigerSetup.toml.
        manifest: PathBuf,
        /// Output directory, or the installer file itself when it ends in .exe
        /// (default: <name>-<version>-Setup.exe in the current directory).
        #[arg(long, value_name = "path", default_value = ".")]
        output: PathBuf,
        /// Engine executable (default: tigersetup-setup.exe beside tiger-setup.exe).
        #[arg(long, value_name = "path")]
        engine: Option<PathBuf>,
        /// Loader executable (default: tigersetup-loader.exe beside tiger-setup.exe).
        #[arg(long, value_name = "path")]
        loader: Option<PathBuf>,
        /// A global MSBuild property for metadata evaluation; may repeat.
        #[arg(long, value_name = "Name=Value")]
        property: Vec<String>,
        /// Resolve nothing from the network; dependency hints stay unresolved.
        #[arg(long)]
        offline: bool,
        /// Build for the iteration loop: a fast compression level instead of
        /// the release profile, so the build is short and the installer is
        /// larger. The result is a valid installer that installs exactly the
        /// same files.
        #[arg(long)]
        fast: bool,
    },
    /// Show the resolved product metadata and where each value came from.
    #[command(long_about = METADATA_HELP)]
    Metadata {
        /// Path to TigerSetup.toml.
        manifest: PathBuf,
        /// A global MSBuild property for metadata evaluation; may repeat.
        #[arg(long, value_name = "Name=Value")]
        property: Vec<String>,
        /// Print one machine-readable JSON document to stdout.
        #[arg(long)]
        json: bool,
    },
    /// Decode an installer's footer, metadata and payload listing, and check its hashes.
    #[command(long_about = INSPECT_HELP)]
    Inspect {
        /// The installer to read; it is never executed.
        installer: PathBuf,
        /// Print one machine-readable JSON document to stdout.
        #[arg(long)]
        json: bool,
        /// Write the compressed payload block, byte for byte, to this new file.
        #[arg(long, value_name = "file")]
        output_payload: Option<PathBuf>,
        /// Write the payload's files as an ordinary ZIP archive to this new file.
        #[arg(long, value_name = "file")]
        output_zip: Option<PathBuf>,
        /// Write the embedded Protocol Buffers metadata, decompressed, byte for byte, to this new file.
        #[arg(long, value_name = "file")]
        output_meta: Option<PathBuf>,
        /// Write the embedded metadata decoded as JSON to this new file.
        #[arg(long, value_name = "file")]
        output_meta_json: Option<PathBuf>,
        /// Write the engine executable the loader runs, decompressed, to this new file.
        #[arg(long, value_name = "file")]
        output_engine: Option<PathBuf>,
    },
    /// Check an installer's hashes, CRCs and declared files; exit 1 on failure.
    Verify {
        /// The installer to check; it is never executed.
        installer: PathBuf,
    },
    /// Generate or finish the WinGet community manifest set for an installer.
    Winget {
        #[command(subcommand)]
        command: WingetCommand,
    },
}

#[derive(Subcommand)]
enum WingetCommand {
    /// Write the manifest set for a built installer, with the installer URL
    /// left unresolved until the bytes are published.
    Prepare {
        /// Path to TigerSetup.toml.
        manifest: PathBuf,
        /// The installer the manifests describe; its bytes are hashed.
        #[arg(long, value_name = "path")]
        installer: PathBuf,
        /// Directory the manifest set is written to; created when absent.
        #[arg(long, value_name = "dir")]
        output: PathBuf,
    },
    /// Write the published URL and the hash of the exact published bytes
    /// into a generated manifest set. Never rebuilds anything.
    Finalize {
        /// Directory holding the generated manifest set.
        directory: PathBuf,
        /// The immutable public URL the installer is published at.
        #[arg(long, value_name = "url")]
        url: String,
        /// The installer file whose exact bytes are published.
        #[arg(long, value_name = "path")]
        installer: PathBuf,
    },
}

const METADATA_HELP: &str = "\
Show the resolved product metadata and where each value came from.

Every candidate is listed in precedence order: `=` marks the value the build
uses and `~` a value a higher-precedence one displaced. A value typed in
[package] wins over a provider's, and a built binary named by
[metadata].executable is validated against the declared source.

--json prints one document with a stable shape:

  {
    \"schema\": 1,
    \"package\": { \"id\", \"name\", \"version\", \"publisher\", \"description\",
                 \"copyright\", \"file_version\", \"informational_version\" },
    \"values\":  [ { \"field\", \"value\", \"source\", \"origin\", \"file\",
                 \"effective\" } ],
    \"validations\": [ { \"check\", \"detail\" } ]
  }

`source` is static, msbuild-project or pe-version-info; `origin` is the
manifest key, MSBuild property or VERSIONINFO field read; `file` is the
project or executable it was read from, empty for a typed value; `check` is
a stable identifier for the comparison performed. Identifiers are never
localized.";

const INSPECT_HELP: &str = "\
Decode an installer's footer, metadata and payload listing, and check its hashes.

The installer is read, never executed. The report goes to stdout: `--json`
makes it one document with stable field names. Exit 0 when the installer
verifies, 1 when it does not, 2 when the file is not an installer.

The `--output-*` options take the file apart, each to a file that must not
exist yet:

  --output-payload <file>    the compressed payload block, byte for byte:
                             its SHA-256 is the payload hash the footer
                             records
  --output-zip <file>        the payload's files as an ordinary ZIP archive
                             of stored entries, one per payload entry, in
                             stream order — a reconstruction any archive
                             tool opens, not a block of the file
  --output-meta <file>       the embedded Protocol Buffers metadata,
                             decompressed, byte for byte: its SHA-256 is the
                             metadata hash
  --output-meta-json <file>  the metadata decoded to JSON — every field of
                             the message tree under its proto name, with
                             enumerations as stable names; the product icon
                             is described by its length and SHA-256
  --output-engine <file>     the engine executable the loader extracts and
                             runs, decompressed: its SHA-256 is the engine
                             block hash the metadata and the footer record

Nothing is written for an installer that fails verification, and no
destination is overwritten: every destination is checked before the first
byte is written. `--json` and `--output-meta-json` are different things and
may be combined: the report on stdout describes the file, the exported JSON
is the metadata alone.";

fn parse_properties(raw: &[String]) -> Result<Vec<(String, String)>, String> {
    raw.iter()
        .map(|item| match item.split_once('=') {
            Some((name, value)) if !name.trim().is_empty() => {
                Ok((name.trim().to_string(), value.to_string()))
            }
            _ => Err(format!("--property {item:?} must be Name=Value")),
        })
        .collect()
}

fn print_provenance(package: &ResolvedPackage) {
    for (field, p) in &package.provenance {
        let mark = if p.effective { '=' } else { '~' };
        println!("{field:<21} {mark} {}", p.value);
        println!("{:<23} {} · {}", "", p.source, p.describe());
    }
    for validation in &package.validations {
        println!(
            "validated             {} · {}",
            validation.check, validation.detail
        );
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Build {
            manifest,
            output,
            engine,
            loader,
            property,
            offline,
            fast,
        } => {
            let properties = match parse_properties(&property) {
                Ok(p) => p,
                Err(message) => {
                    eprintln!("error: {message}");
                    return ExitCode::from(2);
                }
            };
            let request = BuildRequest {
                manifest_path: &manifest,
                output: &output,
                engine_path: engine.as_deref(),
                loader_path: loader.as_deref(),
                properties: &properties,
                offline,
                compression: if fast {
                    Compression::Fast
                } else {
                    Compression::Best
                },
            };
            let started = Instant::now();
            match build(&request) {
                Ok(result) => {
                    println!("Built     {}", result.installer_path.display());
                    println!(
                        "Engine    {} sha256 {}",
                        result.engine_path.display(),
                        result.engine_sha256
                    );
                    println!(
                        "Block     sha256 {} ({} bytes compressed)",
                        result.engine_block_sha256, result.engine_compressed_length
                    );
                    println!(
                        "Loader    {} sha256 {} ({} bytes)",
                        result.loader_path.display(),
                        result.loader_sha256,
                        result.loader_length
                    );
                    println!(
                        "Metadata  sha256 {} ({} bytes from {})",
                        result.metadata_sha256,
                        result.metadata_length,
                        result.metadata_uncompressed_length
                    );
                    let stats = result.payload_stats;
                    println!(
                        "Payload   sha256 {} ({} files, {} entries, {} bytes from {})",
                        result.payload_sha256,
                        result.file_count,
                        stats.entries,
                        result.payload_length,
                        stats.uncompressed_bytes
                    );
                    println!(
                        "Size      {} bytes ({}, {}, {:.1} s)",
                        result.installer_length,
                        request.compression.describe(),
                        if fast { "--fast" } else { "release" },
                        started.elapsed().as_secs_f64()
                    );
                    for (field, provenance) in &result.package.provenance {
                        if provenance.effective {
                            println!(
                                "{:<9} {} ({}: {})",
                                field,
                                provenance.value,
                                provenance.source,
                                provenance.describe()
                            );
                        }
                    }
                    for check in &result.package.validations {
                        println!("Validated {}", check.detail);
                    }
                    for (id, version, url) in &result.resolved_dependencies {
                        println!("Resolved  dependency {id} {version} from {url}");
                    }
                    for (id, reason) in &result.unresolved_dependencies {
                        println!(
                            "Warning   no acquisition hint for dependency {id} ({reason}); the installer will resolve it from the catalog when it needs to acquire one"
                        );
                    }
                    for (name, phase, program) in &result.actions {
                        println!("Action    {name} ({phase}) runs {program}");
                    }
                    ExitCode::from(0)
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::from(2)
                }
            }
        }
        Command::Metadata {
            manifest,
            property,
            json,
        } => {
            let properties = match parse_properties(&property) {
                Ok(p) => p,
                Err(message) => {
                    eprintln!("error: {message}");
                    return ExitCode::from(2);
                }
            };
            let loaded = match tigersetup_build::manifest::LoadedManifest::load(&manifest) {
                Ok(loaded) => loaded,
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::from(2);
                }
            };
            match resolve_package(&loaded, &properties) {
                Ok(package) => {
                    if json {
                        let values: Vec<serde_json::Value> = package
                            .provenance
                            .iter()
                            .map(|(field, p)| {
                                serde_json::json!({
                                    "field": field, "value": p.value,
                                    "source": p.source, "origin": p.origin,
                                    "file": p.file, "effective": p.effective
                                })
                            })
                            .collect();
                        let validations: Vec<serde_json::Value> = package
                            .validations
                            .iter()
                            .map(|v| serde_json::json!({ "check": v.check, "detail": v.detail }))
                            .collect();
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "schema": 1,
                                "package": {
                                    "id": loaded.manifest.package.id,
                                    "name": loaded.manifest.package.name,
                                    "version": package.version,
                                    "publisher": loaded.manifest.package.publisher,
                                    "description": package.description,
                                    "copyright": package.copyright,
                                    "file_version": package.file_version,
                                    "informational_version": package.informational_version,
                                },
                                "values": values,
                                "validations": validations,
                            }))
                            .unwrap_or_default()
                        );
                    } else {
                        print_provenance(&package);
                    }
                    ExitCode::from(0)
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::from(2)
                }
            }
        }
        Command::Inspect {
            installer,
            json,
            output_payload,
            output_zip,
            output_meta,
            output_meta_json,
            output_engine,
        } => match inspect::inspect(&installer) {
            Ok(inspection) => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&inspection.to_json()).unwrap_or_default()
                    );
                } else {
                    print!("{}", inspection.to_text());
                }
                let request = ExportRequest {
                    payload: output_payload,
                    zip: output_zip,
                    meta: output_meta,
                    meta_json: output_meta_json,
                    engine: output_engine,
                };
                if !request.is_empty() {
                    match inspection.export(&request) {
                        // With `--json`, stdout is the report and nothing
                        // else; the exit code says the exports were written.
                        Ok(written) if !json => {
                            for path in written {
                                println!("Exported  {}", path.display());
                            }
                        }
                        Ok(_) => {}
                        Err(err) => {
                            eprintln!("error: {err}");
                            if inspection.is_ok() {
                                return ExitCode::from(2);
                            }
                        }
                    }
                }
                ExitCode::from(if inspection.is_ok() { 0 } else { 1 })
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::from(2)
            }
        },
        Command::Verify { installer } => match inspect::inspect(&installer) {
            Ok(inspection) => {
                if inspection.is_ok() {
                    println!(
                        "ok: {} verifies ({} entries checked)",
                        installer.display(),
                        inspection.verification.entries_checked
                    );
                    ExitCode::from(0)
                } else {
                    for problem in &inspection.verification.problems {
                        eprintln!("{problem}");
                    }
                    if !inspection.engine_block_matches() {
                        eprintln!(
                            "engine_hash_mismatch: the engine block does not match the recorded hash"
                        );
                    }
                    ExitCode::from(1)
                }
            }
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::from(2)
            }
        },
        Command::Winget { command } => match command {
            WingetCommand::Prepare {
                manifest,
                installer,
                output,
            } => match winget::prepare(&manifest, &installer, &output) {
                Ok(result) => {
                    println!("Prepared  {} {}", result.identifier, result.version);
                    println!(
                        "Installer {} sha256 {}",
                        installer.display(),
                        result.installer_sha256
                    );
                    for path in &result.files {
                        println!("Wrote     {}", path.display());
                    }
                    println!(
                        "Next      tiger-setup winget finalize {} --url <published url> --installer {}",
                        output.display(),
                        installer.display()
                    );
                    ExitCode::from(0)
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::from(2)
                }
            },
            WingetCommand::Finalize {
                directory,
                url,
                installer,
            } => match winget::finalize(&directory, &url, &installer) {
                Ok(result) => {
                    println!(
                        "Finalized {} ({} installer entries)",
                        result.manifest_path.display(),
                        result.entries
                    );
                    println!("Version   {}", result.version);
                    println!("Url       {}", result.url);
                    println!("Sha256    {}", result.installer_sha256);
                    ExitCode::from(0)
                }
                Err(err) => {
                    eprintln!("error: {err}");
                    ExitCode::from(2)
                }
            },
        },
    }
}
