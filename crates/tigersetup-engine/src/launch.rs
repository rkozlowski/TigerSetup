//! Launch after install (`TigerSetup-Design.md` §11.7): the program a
//! package asks the wizard to offer on its completion page, started when
//! the person presses Finish with the box checked.
//!
//! It is deliberately not a custom action. An action is part of a
//! transaction, runs hidden and bounded under the run's own token, and can
//! fail the run; the launch happens after the transaction has committed,
//! is shown, is not waited for, runs as the signed-in user whatever token
//! the wizard holds, and can never change what the run reports as its
//! result. So there is no journal row, no `run_on`, no timeout and no exit
//! code here — only what was started and whether it came to the front.
//!
//! What this module owns: the target a declaration resolves to for an
//! installation, the start ([`crate::win::interactive`] decides the token),
//! and the [`LaunchInfo`] the outcome document and the log carry. The
//! client owns only the question — the check box — and the foreground,
//! because the right to take the foreground is its window's.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tigersetup_format::metadata::Launch;

use crate::action;
use crate::report::{self, LaunchInfo};
use crate::win::interactive::{self, StartError, Started};

/// How long the completion page stays up for the program's window before
/// it closes without one. A program that reaches its message loop and shows
/// nothing is let go after a moment instead (a tray application), and one
/// that exits at once at once.
pub const WINDOW_WITHIN: Duration = Duration::from_secs(15);

/// A declaration resolved for one installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub program: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
}

/// The program, arguments and working directory `launch` names for the
/// installation at `install_root` of `version`: the program and an
/// explicit working directory are install-relative, the working directory
/// defaults to the program's own, and every argument expands
/// `%INSTALLROOT%`, `%VERSION%` and the known folders exactly as an
/// action's does — the same text reaches the program as one argument,
/// whatever it contains.
pub fn target(launch: &Launch, install_root: &Path, version: &str) -> Target {
    let program = install_root.join(launch.executable.replace('/', "\\"));
    let working_directory = match launch.working_directory.as_deref() {
        None => program
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| install_root.to_path_buf()),
        Some("") => install_root.to_path_buf(),
        Some(directory) => install_root.join(directory.replace('/', "\\")),
    };
    Target {
        program,
        arguments: launch
            .arguments
            .iter()
            .map(|argument| action::expand_for(argument, install_root, version))
            .collect(),
        working_directory,
    }
}

impl Target {
    fn info(&self, status: &'static str) -> LaunchInfo {
        LaunchInfo {
            status,
            code: None,
            program: self.program.display().to_string(),
            arguments: self.arguments.clone(),
            working_directory: self.working_directory.display().to_string(),
            method: None,
            pid: None,
            foreground: None,
            message: None,
        }
    }

    /// What the outcome says when the person cleared the check box.
    pub fn declined(&self) -> LaunchInfo {
        self.info("declined")
    }

    /// Starts the program as the signed-in user, never elevated. A start
    /// that was attempted and failed is `failed`; one for which no
    /// non-elevated context exists is `unavailable` and never attempted
    /// with this process's own token.
    pub fn start(&self) -> (LaunchInfo, Option<Started>) {
        match interactive::start(&self.program, &self.arguments, &self.working_directory) {
            Ok(started) => {
                let mut info = self.info("started");
                info.method = Some(started.method.as_str());
                info.pid = started.pid;
                (info, Some(started))
            }
            Err(StartError::Failed(message)) => {
                let mut info = self.info("failed");
                info.code = Some("launch_failed");
                info.message = Some(message);
                (info, None)
            }
            Err(StartError::Unavailable(message)) => {
                let mut info = self.info("unavailable");
                info.code = Some("launch_unavailable");
                info.message = Some(message);
                (info, None)
            }
        }
    }
}

/// Writes what happened to the run's log, when it has one, in the log's own
/// format: `launch_started`, `launch_declined`, `launch_failed` or
/// `launch_unavailable`.
pub fn log(log_path: Option<&str>, info: &LaunchInfo) {
    let Some(path) = log_path.filter(|path| !path.is_empty()) else {
        return;
    };
    let code = match info.status {
        "started" => "launch_started",
        "declined" => "launch_declined",
        "unavailable" => "launch_unavailable",
        _ => "launch_failed",
    };
    let mut message = format!(
        "{} {} (in {})",
        info.program,
        crate::win::process::join_arguments(&info.arguments),
        info.working_directory
    );
    if let Some(method) = info.method {
        message.push_str(&format!(" method={method}"));
    }
    if let Some(pid) = info.pid {
        message.push_str(&format!(" pid={pid}"));
    }
    if let Some(foreground) = info.foreground {
        message.push_str(&format!(" foreground={foreground}"));
    }
    if let Some(reason) = &info.message {
        message.push_str(&format!(": {reason}"));
    }
    report::append_to_log(Path::new(path), code, &message);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(working_directory: Option<&str>) -> Launch {
        Launch {
            executable: "bin/App.exe".into(),
            arguments: vec![
                "--root".into(),
                "%INSTALLROOT%".into(),
                "v%VERSION%".into(),
                "100%".into(),
                "a \"quoted\" word".into(),
                "trailing\\".into(),
                String::new(),
            ],
            working_directory: working_directory.map(str::to_string),
            checked: true,
        }
    }

    /// The program and working directory are the installation's, and each
    /// argument is one argument, expanded and otherwise untouched.
    #[test]
    fn a_declaration_resolves_against_the_installation() {
        let root = Path::new("C:\\Program Files\\App");
        let target = target(&declared(None), root, "1.2.3");
        assert_eq!(target.program, root.join("bin").join("App.exe"));
        assert_eq!(
            target.working_directory,
            root.join("bin"),
            "the program's own directory by default"
        );
        assert_eq!(
            target.arguments,
            [
                "--root",
                "C:\\Program Files\\App",
                "v1.2.3",
                "100%",
                "a \"quoted\" word",
                "trailing\\",
                "",
            ]
        );
        assert_eq!(
            super::target(&declared(Some("")), root, "1.2.3").working_directory,
            root,
            "empty is the install root"
        );
        assert_eq!(
            super::target(&declared(Some("data/cache")), root, "1.2.3").working_directory,
            root.join("data").join("cache")
        );
    }

    /// A declined offer and a failed start both say what would have been
    /// started, so the outcome can be read without the metadata.
    #[test]
    fn every_status_names_the_resolved_program() {
        let dir = tempfile::tempdir().unwrap();
        let target = target(&declared(None), dir.path(), "1.0.0");
        let declined = target.declined();
        assert_eq!(declined.status, "declined");
        assert_eq!(declined.code, None);
        assert_eq!(declined.program, target.program.display().to_string());
        assert_eq!(declined.arguments.len(), 7);

        if interactive::this_process_is_elevated() {
            eprintln!("SKIPPED: the failed-start half starts a program with this process's token");
            return;
        }
        let (failed, started) = target.start();
        assert!(started.is_none());
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.code, Some("launch_failed"));
        assert!(failed.message.is_some());
    }

    #[test]
    fn the_log_line_is_in_the_logs_own_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.log");
        std::fs::write(&path, "earlier line\n").unwrap();
        let target = target(&declared(None), dir.path(), "1.0.0");
        let mut info = target.declined();
        log(Some(path.to_str().unwrap()), &info);
        info.status = "started";
        info.method = Some("own_token");
        info.pid = Some(42);
        info.foreground = Some(true);
        log(Some(path.to_str().unwrap()), &info);
        log(None, &info);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[0], "earlier line");
        assert!(lines[1].contains(" [launch_declined] "), "{text}");
        assert!(
            lines[2].contains(" [launch_started] ")
                && lines[2].ends_with("method=own_token pid=42 foreground=true"),
            "{text}"
        );
    }
}
