//! `TigerSetupTestPrereq.exe`: the prerequisite installer the synthetic
//! `TigerSetupTestApp` package carries inside itself. It stands in for a
//! vendor's runtime installer, so the embedded-dependency path — extract,
//! verify, run unattended, judge the exit code, detect again — is proven
//! against a program whose behaviour is chosen by its arguments rather than
//! by a download.
//!
//! ```text
//! TigerSetupTestPrereq.exe [--install | --no-install] [--version <v>] [--exit <code>]
//! ```
//!
//! `--install` (the default) creates `%PROGRAMDATA%\TigerSetupTestPrereq\
//! <version>\prereq.txt`, which the package's `directory-version` detector
//! reads; `--no-install` creates nothing. The process then exits with
//! `--exit` (0 by default), so a test can make the "installer" report success,
//! a reboot (3010), or a failure with or without having done the job.
//! `%PROGRAMDATA%` is read from the environment, which is what lets an
//! isolated test machine redirect it.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut install = true;
    let mut version = "1.0.0".to_string();
    let mut exit = 0;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--install" => install = true,
            "--no-install" => install = false,
            "--version" => version = arguments.next().unwrap_or(version),
            "--exit" => {
                exit = arguments
                    .next()
                    .and_then(|code| code.parse().ok())
                    .unwrap_or(2);
            }
            other => {
                eprintln!("TigerSetupTestPrereq: unknown argument {other:?}");
                return ExitCode::from(2);
            }
        }
    }
    if install {
        let root = std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\ProgramData"));
        let directory = root.join("TigerSetupTestPrereq").join(&version);
        if let Err(err) = std::fs::create_dir_all(&directory).and_then(|()| {
            std::fs::write(
                directory.join("prereq.txt"),
                format!("TigerSetupTestPrereq {version}\r\n"),
            )
        }) {
            eprintln!(
                "TigerSetupTestPrereq: cannot write {}: {err}",
                directory.display()
            );
            return ExitCode::from(1);
        }
    }
    // Exit codes above 255 are ordinary on Windows (3010 asks for a reboot),
    // and `ExitCode` is a `u8`: the process exits directly instead.
    std::process::exit(exit)
}
