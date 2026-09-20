//! `TigerSetupTestAction.exe`: the program the synthetic `TigerSetupTestApp`
//! package runs as its custom actions, and the process-level tests run as
//! theirs. It stands in for whatever a real package would run at a lifecycle
//! phase, so the action machinery — packaging, extraction and verification,
//! the launch envelope, the exit-code verdict, the timeout, the evidence —
//! is proven against a program whose every effect is chosen by its
//! arguments rather than by what some vendor's tool happens to do.
//!
//! ```text
//! TigerSetupTestAction.exe [--marker <path>] [--record <path>] [--stdout <text>]
//!                          [--stderr <text>] [--sleep <seconds>] [--exit <code>]
//!                          [--fail-if-exists <path>] [--delete <path>]
//!                          [--hold <file> --pid-file <path>] [--stop <pid-file>]
//! ```
//!
//! In this order: `--marker` creates the file (and its directories) with a
//! fixed line; `--record` appends one tab-separated line — the working
//! directory, every argument, and every `TIGERSETUP_*` variable — to the
//! file; `--stdout` and `--stderr` write the text to the respective stream;
//! `--fail-if-exists` exits 1 when the file is there; `--delete` removes
//! the file when it is there; `--sleep` waits; the process then exits with
//! `--exit` (0 by default). Every switch may repeat.
//!
//! Two switches make it stand in for an application and for the program
//! that stops it, for the quiescence tests: `--hold <file>` opens the file
//! the way a running application does — readable, not replaceable — writes
//! its own process id to `--pid-file` and then runs until it is ended;
//! `--stop <pid-file>` ends the process the file names and removes the
//! file, exiting 0, or exits 3 — "not running" — when there is no such
//! file or no such process.

use std::io::Write;
use std::path::{Path, PathBuf};

fn write_marker(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, "TigerSetupTestAction\r\n")
}

fn append_record(path: &Path, arguments: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut environment: Vec<String> = std::env::vars()
        .filter(|(name, _)| name.starts_with("TIGERSETUP_"))
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    environment.sort();
    let line = format!(
        "cwd={cwd}\targs={}\tenv={}\r\n",
        arguments.join("|"),
        environment.join("|")
    );
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    file.write_all(line.as_bytes())
}

/// The exit code `--stop` reports when nothing was running.
const NOT_RUNNING: i32 = 3;

/// Ends the process whose id `pid_file` holds.
fn stop(pid_file: &Path) -> i32 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        INFINITE, OpenProcess, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    };
    /// `SYNCHRONIZE`: the standard right to wait on the process.
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let Ok(text) = std::fs::read_to_string(pid_file) else {
        return NOT_RUNNING;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        let _ = std::fs::remove_file(pid_file);
        return NOT_RUNNING;
    };
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, 0, pid) };
    let _ = std::fs::remove_file(pid_file);
    if handle.is_null() {
        return NOT_RUNNING;
    }
    unsafe {
        TerminateProcess(handle, 0);
        WaitForSingleObject(handle, INFINITE);
        CloseHandle(handle);
    }
    0
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut exit = 0i32;
    let mut sleep = 0u64;
    let mut held: Option<std::fs::File> = None;
    let mut index = 0;
    let value = |index: &mut usize| -> Option<PathBuf> {
        *index += 1;
        arguments.get(*index).map(PathBuf::from)
    };
    while index < arguments.len() {
        let result: std::io::Result<()> = match arguments[index].as_str() {
            "--marker" => match value(&mut index) {
                Some(path) => write_marker(&path),
                None => Ok(()),
            },
            "--record" => match value(&mut index) {
                Some(path) => append_record(&path, &arguments),
                None => Ok(()),
            },
            "--stdout" => {
                if let Some(text) = value(&mut index) {
                    println!("{}", text.display());
                }
                Ok(())
            }
            "--stderr" => {
                if let Some(text) = value(&mut index) {
                    eprintln!("{}", text.display());
                }
                Ok(())
            }
            "--fail-if-exists" => {
                if value(&mut index).is_some_and(|path| path.exists()) {
                    eprintln!("TigerSetupTestAction: the file exists; failing as asked");
                    std::process::exit(1);
                }
                Ok(())
            }
            "--delete" => match value(&mut index) {
                Some(path) if path.exists() => std::fs::remove_file(&path),
                _ => Ok(()),
            },
            "--sleep" => {
                sleep = value(&mut index)
                    .and_then(|v| v.to_str().and_then(|s| s.parse().ok()))
                    .unwrap_or(0);
                Ok(())
            }
            "--exit" => {
                exit = value(&mut index)
                    .and_then(|v| v.to_str().and_then(|s| s.parse().ok()))
                    .unwrap_or(2);
                Ok(())
            }
            // Opened as most applications open their files: readable by
            // others, not replaceable or deletable while it is held. The
            // process also ignores the console control event the Restart
            // Manager closes a console application with, so it is the
            // holder the Restart Manager lists and cannot close — what a
            // package's own stop program exists for.
            "--hold" => match value(&mut index) {
                Some(path) => {
                    use std::os::windows::fs::OpenOptionsExt;
                    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
                    const FILE_SHARE_READ: u32 = 1;
                    unsafe { SetConsoleCtrlHandler(None, 1) };
                    std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ)
                        .open(&path)
                        .map(|file| held = Some(file))
                }
                None => Ok(()),
            },
            "--pid-file" => match value(&mut index) {
                Some(path) => path
                    .parent()
                    .map(std::fs::create_dir_all)
                    .unwrap_or(Ok(()))
                    .and_then(|()| std::fs::write(&path, std::process::id().to_string())),
                None => Ok(()),
            },
            "--stop" => match value(&mut index) {
                Some(path) => {
                    std::process::exit(stop(&path));
                }
                None => Ok(()),
            },
            other => {
                eprintln!("TigerSetupTestAction: unknown argument {other:?}");
                std::process::exit(2);
            }
        };
        if let Err(err) = result {
            eprintln!("TigerSetupTestAction: {}: {err}", arguments[index]);
            std::process::exit(1);
        }
        index += 1;
    }
    let _ = std::io::stdout().flush();
    if sleep > 0 {
        std::thread::sleep(std::time::Duration::from_secs(sleep));
    }
    // A held file is held until this process is ended from outside.
    if held.is_some() {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    // Exit codes above 255 are ordinary on Windows (3010 asks for a reboot):
    // the process exits directly rather than through `ExitCode`.
    std::process::exit(exit)
}
