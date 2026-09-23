//! `tiger-setup shell` as the Start Menu's TigerSetup Shell runs it: it opens
//! on the brief help, the command prompt runs *this* tiger-setup — even with
//! another `tiger-setup` earlier on `PATH` — and stays usable, and the `PATH`
//! change lives in that shell alone.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const BUILDER: &str = env!("CARGO_BIN_EXE_tiger-setup");

#[test]
fn the_shell_runs_this_tiger_setup_first_and_stays_open() {
    let decoy = tempfile::tempdir().unwrap();
    std::fs::write(
        decoy.path().join("tiger-setup.cmd"),
        "@echo DECOY tiger-setup\r\n",
    )
    .unwrap();
    let work = tempfile::tempdir().unwrap();
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(decoy.path());
    path.push(";");
    path.push(&inherited);

    let mut child = Command::new(BUILDER)
        .arg("shell")
        .env("PATH", &path)
        .current_dir(work.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // What a person types once the help has been shown: the shell answers,
    // and `exit` ends it with the code given.
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"where tiger-setup\r\necho SHELL-%CD%\r\nexit 7\r\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(7), "{stdout}");
    // The brief landing help, plain because the output is not a console —
    // not the complete reference.
    assert!(
        stdout.contains("tiger-setup is on PATH in this window."),
        "{stdout}"
    );
    assert!(stdout.contains("Getting started:"), "{stdout}");
    assert!(!stdout.contains("Usage: tiger-setup"), "{stdout}");
    assert!(!stdout.contains('\x1b'), "{stdout}");
    assert!(!stdout.contains("DECOY"), "{stdout}");

    let own_directory = Path::new(BUILDER).parent().unwrap();
    let found: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.to_lowercase().ends_with("tiger-setup.exe")
                || line.to_lowercase().ends_with("tiger-setup.cmd")
        })
        .collect();
    let first = found.first().copied().unwrap_or_default();
    assert!(
        Path::new(first).parent() == Some(own_directory),
        "`where` resolved {found:?} first; expected {}",
        own_directory.display()
    );
    // Started from anywhere but its own directory, the shell stays there.
    assert!(
        stdout
            .to_lowercase()
            .contains(&format!("shell-{}", work.path().display()).to_lowercase()),
        "{stdout}"
    );
}

/// The user's persistent PATH as the registry holds it.
fn persistent_user_path() -> String {
    let output = Command::new("reg.exe")
        .args(["query", "HKCU\\Environment", "/v", "Path"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_shell_never_writes_the_persistent_path() {
    let before = persistent_user_path();
    let mut child = Command::new(BUILDER)
        .arg("shell")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"exit 0\r\n")
        .unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(persistent_user_path(), before);
}
