//! The generated installer's command-line client. It reads its own tail
//! through the format crate and drives the engine through its public API; it
//! never touches the filesystem or the registry itself.
//!
//! ```text
//! Setup.exe                                  the root operation, interactive
//! Setup.exe install   [--quiet] [--scope user|machine] [--install-root <path>]
//!                     [--option <name> <on|off>]... [--no-dependency-install]
//!                     [--lang <tag>] [--log <path>] [--json] [--fault <spec>]...
//! Setup.exe uninstall [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
//! Setup.exe repair    [--quiet] [--scope user|machine] [--lang <tag>] [--log <path>] [--json]
//! Setup.exe verify    [--scope user|machine] [--json]
//! Setup.exe inspect   [--scope user|machine] [--json]
//! ```
//!
//! Commands express operations, positional arguments identify their
//! subjects, and options modify behaviour; an option takes its value as a
//! separate argument (`--log <path>`, never `--log=<path>`). Without
//! `--quiet` an install, uninstall or repair is interactive. The root
//! operation is `install` for an installer and `uninstall` for the
//! uninstaller copy the engine keeps in the state directory.
//!
//! A command that names no `--scope` is about the installation the machine
//! already holds, whichever scope it is in; a first install takes the
//! package's default scope. The engine decides that (`target`): two
//! installations are never chosen between, and a `--scope` that would put a
//! second installation beside an existing one is refused unless the package
//! allows it. The refusal is a structured outcome, `scope_conflict` or
//! `scope_ambiguous`, naming every installation found.
//!
//! Machine scope needs an administrator. When `install`, `uninstall` or
//! `repair` asks for it from a process that has none, this client runs
//! itself again through the elevation prompt, waits, and reports what the
//! elevated run reported: the same document on stdout and the same exit
//! code, so a caller cannot tell which side of the prompt produced them.
//! `verify` and `inspect` never elevate — they only read.
//!
//! Exit codes: 0 success · 1 failed and rolled back · 2 invalid arguments or
//! package · 3 dependency missing · 4 elevation required · 5 cancelled ·
//! 6 package in use · 7 an earlier transaction needs recovery and could not
//! be completed · 8 unsupported platform · 3010 success, reboot required.
//!
//! The executable is a GUI-subsystem program, so a double-click shows the
//! wizard and never a console window; a command-line run attaches to the
//! parent's console for its output (`console`).

#![windows_subsystem = "windows"]

mod console;
mod ui;

use std::collections::BTreeMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::{Args, Parser, Subcommand};
use tigersetup_engine::elevation;
use tigersetup_engine::format::identity::Scope;
use tigersetup_engine::report::{Event, EventSink, Outcome, exit};
use tigersetup_engine::target;
use tigersetup_engine::txn::FaultSpec;
use tigersetup_engine::{Package, RunOptions};

#[derive(Parser)]
#[command(
    name = "Setup",
    about = "TigerSetup installer",
    disable_version_flag = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Options accepted by the root operation.
    #[command(flatten)]
    root: MutatingArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Install, or upgrade an installed version, with deterministic defaults.
    Install(MutatingArgs),
    /// Remove what the installation owns, planning from the state database.
    Uninstall(MutatingArgs),
    /// Reconcile the installation with the package: restore what is missing or changed.
    Repair(MutatingArgs),
    /// Compare the owned state with the disk without changing anything.
    Verify(ReadArgs),
    /// Describe the package and what the machine holds for it.
    Inspect(ReadArgs),
}

#[derive(Args, Clone, Default)]
struct MutatingArgs {
    /// Run unattended with deterministic defaults; no window is shown.
    #[arg(long)]
    quiet: bool,
    /// Installation scope; defaults to the scope the product is installed in,
    /// or the package's first declared scope for a first install.
    #[arg(long, value_name = "user|machine")]
    scope: Option<String>,
    /// Override the install root (install only).
    #[arg(long, value_name = "path")]
    install_root: Option<PathBuf>,
    /// Set a declared option: --option <name> <on|off>. May repeat.
    #[arg(long, value_names = ["name", "on|off"], num_args = 2)]
    option: Vec<String>,
    /// Fail with dependency_missing instead of acquiring a missing dependency.
    #[arg(long)]
    no_dependency_install: bool,
    /// Language for human-readable text (BCP 47 tag); defaults to the Windows UI language.
    #[arg(long, value_name = "tag")]
    lang: Option<String>,
    /// Write the log to this path instead of the state directory.
    #[arg(long, value_name = "path")]
    log: Option<PathBuf>,
    /// Print one machine-readable JSON document to stdout.
    #[arg(long)]
    json: bool,
    /// Inject a fault: <point>[@<sequence>]:<action>[:<seconds>][:skip_flush]. Testing only.
    #[arg(long, value_name = "spec")]
    fault: Vec<String>,
    /// File to create when an injected fault reaches its boundary, so that a
    /// harness can interrupt the run exactly there. Testing only.
    #[arg(long, value_name = "path")]
    fault_signal: Option<PathBuf>,
    /// Set by the uninstaller when it relaunches a temporary copy of itself:
    /// the path in the state directory the copy came from. Not for people.
    #[arg(long, value_name = "path", hide = true)]
    relaunched_from: Option<PathBuf>,
    /// Set on the elevated run this client starts for machine scope: where
    /// to leave the outcome document its unelevated parent reports. Not for
    /// people.
    #[arg(long, value_name = "path", hide = true)]
    elevated_result: Option<PathBuf>,
}

#[derive(Args, Clone, Default)]
struct ReadArgs {
    /// Installation scope; defaults to the scope the product is installed in,
    /// or the package's first declared scope when it is not installed.
    #[arg(long, value_name = "user|machine")]
    scope: Option<String>,
    /// Print one machine-readable JSON document to stdout.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Install,
    Uninstall,
    Repair,
    Verify,
    Inspect,
}

/// The unattended client. It answers the engine's questions the way an
/// unattended run must: applications holding the product's files are
/// closed, because there is nobody to ask.
struct Quiet;

impl EventSink for Quiet {
    fn event(&mut self, _event: &Event) {}
}

fn invalid(json: bool, code: &'static str, message: &str) -> i32 {
    if json {
        let outcome = Outcome::new("failed", code, exit::INVALID, message);
        println!("{}", serde_json::to_string(&outcome).unwrap_or_default());
    } else {
        eprintln!("error: {message}");
    }
    exit::INVALID
}

/// Reports an engine error that stopped a read-only command, with the exit
/// code the error itself carries.
fn failed(json: bool, err: &tigersetup_engine::Error) -> i32 {
    if json {
        let outcome = Outcome::new("failed", err.code, err.exit_code(), err.message.clone());
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome).unwrap_or_default()
        );
    } else {
        eprintln!("error: {}", err.message);
    }
    err.exit_code()
}

fn print_outcome(outcome: &Outcome, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(outcome).unwrap_or_default()
        );
    } else {
        println!("{}", outcome.message);
        for finding in &outcome.findings {
            println!(
                "  {} {}",
                finding.code,
                finding
                    .path
                    .as_deref()
                    .or(finding.detail.as_deref())
                    .unwrap_or("")
            );
        }
        if let Some(log) = &outcome.log {
            println!("log: {log}");
        }
    }
}

/// Windows creation flag: run without a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Copies this executable into a staging directory, runs the same command
/// there with the copy's origin recorded, and returns the copy's exit code.
/// Standard output is inherited, so the caller sees exactly one document.
///
/// The engine chooses the directory, because where it may be depends on
/// whether this process is elevated: an elevated run must not stage a binary
/// it is about to execute in a folder the invoking user can write.
fn relaunch_from_temporary(package: &Package, exe: &Path) -> std::result::Result<i32, String> {
    let directory = tigersetup_engine::staging_directory().map_err(|err| err.message)?;
    let copy = directory.join(format!(
        "{}-uninstall-{}.exe",
        package.id(),
        std::process::id()
    ));
    std::fs::copy(exe, &copy).map_err(|err| format!("cannot copy {}: {err}", copy.display()))?;
    let status = std::process::Command::new(&copy)
        .args(std::env::args_os().skip(1))
        .arg("--relaunched-from")
        .arg(exe)
        .status()
        .map_err(|err| format!("cannot start {}: {err}", copy.display()))?;
    Ok(status.code().unwrap_or(exit::ROLLED_BACK))
}

/// Runs this same command again through the elevation prompt and reports
/// what the elevated run reported. The child leaves its outcome document in
/// a file, because its standard output does not reach this process across
/// the elevation boundary; this process prints that document as its own so
/// that the caller sees exactly one.
fn relaunch_elevated(
    exe: &Path,
    intent: elevation::Intent,
    options: &RunOptions,
    json: bool,
) -> i32 {
    let elevated = match elevation::relaunch(exe, intent, options) {
        Ok(elevated) => elevated,
        Err(err) => return failed(json, &err),
    };
    match elevated.document {
        Some(document) if json => println!("{}", document.trim_end()),
        Some(document) => println!("{}", human_message(&document)),
        // The child never got far enough to write one: its exit code is
        // still the truth about what happened.
        None => {
            if json {
                let outcome = Outcome::new(
                    "failed",
                    "elevated_result_missing",
                    elevated.exit_code,
                    format!(
                        "the elevated run exited with {} without reporting an outcome",
                        elevated.exit_code
                    ),
                );
                println!(
                    "{}",
                    serde_json::to_string_pretty(&outcome).unwrap_or_default()
                );
            } else {
                eprintln!(
                    "error: the elevated run exited with {} without reporting an outcome",
                    elevated.exit_code
                );
            }
        }
    }
    elevated.exit_code
}

/// The human-readable message of an outcome document, for a caller that did
/// not ask for JSON.
fn human_message(document: &str) -> String {
    serde_json::from_str::<serde_json::Value>(document)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| document.trim_end().to_string())
}

/// Asks a detached, windowless `cmd.exe` to wait for this process to exit
/// and then delete the temporary copy it runs from, and the executable that
/// copy moved out of the state directory. Best effort: a leftover under
/// `%TEMP%` costs nothing.
fn schedule_self_deletion(exe: &Path) {
    let aside = tigersetup_engine::origin_aside_path(exe);
    // `cmd.exe` does not understand the backslash-escaped quotes Rust would
    // write for an ordinary argument, so the command line is built verbatim:
    // `/c "<script>"`, which cmd unwraps by dropping the outer pair.
    let raw = format!(
        "/c \"ping -n 3 127.0.0.1 >nul & del /f /q \"{}\" \"{}\" >nul 2>&1\"",
        aside.display(),
        exe.display()
    );
    let _ = std::process::Command::new("cmd.exe")
        .raw_arg(raw)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// The arguments an elevated relaunch of this command should repeat: this
/// run's own, without the scope and the language, which the wizard sets.
fn relaunch_arguments() -> Vec<String> {
    let mut arguments = Vec::new();
    let mut rest = std::env::args().skip(1);
    while let Some(argument) = rest.next() {
        if argument == "--scope" || argument == "--lang" {
            rest.next();
            continue;
        }
        arguments.push(argument);
    }
    arguments
}

fn run() -> i32 {
    console::attach_to_parent();
    let cli = Cli::parse();
    let (operation, mutating, read) = match cli.command {
        None => (None, cli.root, ReadArgs::default()),
        Some(Command::Install(args)) => (Some(Operation::Install), args, ReadArgs::default()),
        Some(Command::Uninstall(args)) => (Some(Operation::Uninstall), args, ReadArgs::default()),
        Some(Command::Repair(args)) => (Some(Operation::Repair), args, ReadArgs::default()),
        Some(Command::Verify(args)) => (Some(Operation::Verify), MutatingArgs::default(), args),
        Some(Command::Inspect(args)) => (Some(Operation::Inspect), MutatingArgs::default(), args),
    };
    let json = mutating.json || read.json;

    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            return invalid(
                json,
                "package_unreadable",
                &format!("cannot locate this executable: {err}"),
            );
        }
    };
    let package = match Package::open(&exe) {
        Ok(package) => package,
        Err(err) => {
            return invalid(
                json,
                err.code,
                &format!("this executable carries no valid package: {}", err.message),
            );
        }
    };

    // The root operation depends on what this executable is.
    let operation = operation.unwrap_or(if package.metadata().is_uninstaller() {
        Operation::Uninstall
    } else {
        Operation::Install
    });

    let lang = mutating
        .lang
        .clone()
        .unwrap_or_else(tigersetup_engine::i18n::detect_ui_language);
    let mutates = matches!(
        operation,
        Operation::Install | Operation::Uninstall | Operation::Repair
    );
    let interactive = mutates && !mutating.quiet;

    // Which installation this run is about. A named scope is taken as
    // named; the uninstaller copy serves the scope it was written for; and
    // otherwise the engine finds the installation the machine holds. A
    // refusal ends an unattended run here, as a structured outcome; the
    // wizard asks the engine again and shows the refusal, or the choice,
    // itself.
    let scope_text = mutating.scope.clone().or(read.scope.clone());
    let scope_given = scope_text.is_some();
    let requested = match scope_text {
        Some(text) => match Scope::parse(&text) {
            Some(scope) => Some(scope),
            None => return invalid(json, "invalid_arguments", "--scope must be user or machine"),
        },
        None => package.metadata().served_scope(),
    };
    let intent = match operation {
        Operation::Install => Some(elevation::Intent::Install),
        Operation::Uninstall => Some(elevation::Intent::Uninstall),
        Operation::Repair => Some(elevation::Intent::Repair),
        Operation::Verify | Operation::Inspect => None,
    };
    let resolution = match target::resolve(&package, requested, intent) {
        Ok(resolution) => resolution,
        Err(err) => return failed(json, &err),
    };
    let scope = match resolution.scope() {
        Some(scope) => scope,
        None if interactive => package.default_scope(),
        None => {
            let options = RunOptions {
                lang,
                ..RunOptions::default()
            };
            let outcome = resolution
                .outcome(&package, &options)
                .expect("a resolution without a scope is a refusal");
            print_outcome(&outcome, json);
            return outcome.exit_code;
        }
    };

    let mut options = BTreeMap::new();
    for pair in mutating.option.chunks(2) {
        let (name, value) = (&pair[0], &pair[1]);
        let on = match value.to_ascii_lowercase().as_str() {
            "on" | "true" | "yes" | "1" => true,
            "off" | "false" | "no" | "0" => false,
            _ => {
                return invalid(
                    json,
                    "invalid_arguments",
                    &format!("--option {name} takes on or off, not {value:?}"),
                );
            }
        };
        if package.metadata().option_default(name).is_none() {
            return invalid(
                json,
                "option_unknown",
                &format!("{} declares no option {name:?}", package.name()),
            );
        }
        options.insert(name.to_ascii_lowercase(), on);
    }

    let mut faults = Vec::with_capacity(mutating.fault.len());
    for spec in &mutating.fault {
        match FaultSpec::parse(spec) {
            Ok(fault) => faults.push(fault),
            Err(err) => return invalid(json, err.code, &err.message),
        }
    }

    let run_options = RunOptions {
        scope,
        install_root: mutating.install_root.clone(),
        log_path: mutating.log.clone(),
        faults,
        fault_signal: mutating.fault_signal.clone(),
        lang,
        options,
        install_dependencies: !mutating.no_dependency_install,
        quiet: mutating.quiet,
        // Only the wizard's licence page can say a person accepted the
        // text; the command line has no switch for it, so an unattended run
        // never records an acceptance.
        license_accepted: false,
        cancel: None,
        relaunched_from: mutating.relaunched_from.clone(),
    };

    // The wizard puts the elevation shield on the page that decides the
    // scope: the choice of a first install, the installation an ordinary
    // rerun continues with, or the one of two installations this run is
    // about. Where that page will be shown, the prompt is the wizard's to
    // raise from it; everywhere else a machine-scope run is elevated before
    // anything else, so that everything after this point — the temporary
    // uninstaller copy included — runs on the side of the prompt that may
    // actually change the machine.
    let wizard_decides_scope = interactive
        && !scope_given
        && !package.is_uninstaller()
        && (resolution.scope().is_none()
            || (operation == Operation::Install && package.metadata().scopes().len() > 1));
    if mutates
        && !wizard_decides_scope
        && mutating.elevated_result.is_none()
        && elevation::requirement(&package, &run_options)
            .is_ok_and(elevation::Requirement::needs_elevation)
    {
        let intent = match operation {
            Operation::Uninstall => elevation::Intent::Uninstall,
            Operation::Repair => elevation::Intent::Repair,
            _ => elevation::Intent::Install,
        };
        return relaunch_elevated(&exe, intent, &run_options, json);
    }

    // An uninstaller that lives in the state directory it is about to remove
    // cannot delete itself: it runs the uninstall from a temporary copy and
    // reports whatever that copy reports.
    if operation == Operation::Uninstall
        && run_options.relaunched_from.is_none()
        && let Ok(roots) = tigersetup_engine::resolve_roots(&package, &run_options)
        && exe.starts_with(&roots.state_dir)
    {
        return match relaunch_from_temporary(&package, &exe) {
            Ok(code) => code,
            Err(message) => invalid(json, "relaunch_failed", &message),
        };
    }

    match operation {
        Operation::Inspect => match tigersetup_engine::inspect(&package, &run_options) {
            Ok(report) => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).unwrap_or_default()
                    );
                } else {
                    println!(
                        "{} {} ({})",
                        report.package.name, report.package.version, report.package.id
                    );
                    match &report.installation {
                        Some(installation) => println!(
                            "installed: {} at {}",
                            installation.version, installation.install_root
                        ),
                        None => println!("not installed"),
                    }
                    if let Some(txn) = &report.transaction {
                        println!(
                            "open transaction: {} {} ({})",
                            txn.kind,
                            txn.state,
                            txn.recovery_direction.unwrap_or("-")
                        );
                    }
                    for dependency in &report.dependencies {
                        println!(
                            "dependency {}: {} {}",
                            dependency.name,
                            dependency.status,
                            dependency.version.as_deref().unwrap_or("")
                        );
                    }
                }
                exit::OK
            }
            Err(err) => failed(json, &err),
        },
        Operation::Verify => match tigersetup_engine::verify(&package, &run_options) {
            Ok(report) => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&report).unwrap_or_default()
                    );
                } else {
                    println!("{}", report.status);
                    for finding in &report.findings {
                        println!(
                            "  {} {}",
                            finding.code,
                            finding
                                .path
                                .as_deref()
                                .or(finding.detail.as_deref())
                                .unwrap_or("")
                        );
                    }
                }
                report.exit_code()
            }
            Err(err) => failed(json, &err),
        },
        Operation::Install | Operation::Uninstall | Operation::Repair => {
            let (exit_code, outcome) = if mutating.quiet {
                let mut sink = Quiet;
                let outcome = match operation {
                    Operation::Install => {
                        tigersetup_engine::install(&package, &run_options, &mut sink)
                    }
                    Operation::Uninstall => {
                        tigersetup_engine::uninstall(&package, &run_options, &mut sink)
                    }
                    _ => tigersetup_engine::repair(&package, &run_options, &mut sink),
                };
                print_outcome(&outcome, json);
                (outcome.exit_code, Some(outcome))
            } else {
                let completed = ui::run(ui::Request {
                    exe: exe.clone(),
                    package: &package,
                    operation,
                    options: run_options.clone(),
                    scope_given,
                    relaunch_arguments: relaunch_arguments(),
                });
                if json {
                    // An elevated child reported its own run; that document
                    // is passed through exactly as it was written.
                    if let Some(document) = &completed.elevated_document {
                        println!("{document}");
                    } else if let Some(outcome) = &completed.outcome {
                        print_outcome(outcome, true);
                    }
                }
                (completed.exit_code, completed.outcome)
            };
            // An elevated run hands its document back to the unelevated
            // process that started it, which reports it as its own. This is
            // the same whether the elevated child ran quietly or showed the
            // wizard.
            if let (Some(path), Some(outcome)) = (&mutating.elevated_result, &outcome) {
                let document = serde_json::to_string_pretty(outcome).unwrap_or_default();
                let _ = elevation::write_result(path, &document);
            }
            if run_options.relaunched_from.is_some() {
                schedule_self_deletion(&exe);
            }
            exit_code
        }
    }
}

fn main() {
    std::process::exit(run());
}
