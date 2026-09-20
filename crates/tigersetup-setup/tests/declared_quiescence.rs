//! Package-declared quiescence (`TigerSetup-Design.md` §5.10): a running
//! application the Restart Manager cannot close is stopped by the package's
//! own stop program before the Restart Manager is asked, and started again
//! afterwards — only if it was running, and on every path that leaves the
//! product on the machine, a refusal included.
//!
//! The application is `TigerSetupTestAction --hold`, a console process with
//! no message loop holding a payload file open, which is exactly the holder
//! the Restart Manager lists and never closes; the stop program is the same
//! executable's `--stop`, which reports 0 when it ended something and 3 when
//! there was nothing to end.

mod common;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use serde_json::Value;

/// A held file's process gets a console of its own: the Restart Manager
/// closes a console holder with a console control event, which would
/// otherwise reach this test runner (`LESSONS_LEARNED.md`).
const HOLDER_ISOLATION: u32 = 0x0800_0000 | 0x0000_0200;

fn base_manifest(version: &str) -> String {
    format!(
        "[package]\nid = \"{PRODUCT_ID}\"\nname = \"{PRODUCT_NAME}\"\n\
         version = \"{version}\"\npublisher = \"IT Tiger\"\n\n\
         [install]\nscopes = [\"user\"]\n\n[[files]]\nsource = \"payload/**\"\n\n"
    )
}

/// The declaration under test: the stop program ends whatever the pid file
/// names, the resume program holds the payload file again.
fn quiescence_toml(with_resume: bool, on_failure: &str) -> String {
    let stop = format!(
        "[[quiescence]]\nname = \"app\"\nnot_running_codes = [3]\n\n\
         [quiescence.stop]\nkind = \"exe\"\nsource = \"actions/{ACTION_FILE_NAME}\"\n\
         arguments = [\"--stop\", \"%PROGRAMDATA%\\\\{ACTIONS_DIR_NAME}\\\\app.pid\"]\n\
         timeout_seconds = 30\non_failure = \"{on_failure}\"\n\n"
    );
    if !with_resume {
        return stop;
    }
    format!(
        "{stop}[quiescence.resume]\nkind = \"exe\"\nsource = \"actions/{ACTION_FILE_NAME}\"\n\
         arguments = [\"--hold\", \"%INSTALLROOT%\\\\bin\\\\{HELD_FILE}\", \"--pid-file\", \"%PROGRAMDATA%\\\\{ACTIONS_DIR_NAME}\\\\app.pid\"]\n\n"
    )
}

/// The file the application holds: its content changes with the version,
/// so an upgrade has to replace it and a holder is in the way.
const HELD_FILE: &str = "version.txt";

fn build_quiescent_package(name: &str, version: &str, toml: &str) -> PathBuf {
    let dir = Path::new(TMP).join(format!("quiesce-{name}-{version}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("payload").join("bin")).unwrap();
    std::fs::write(dir.join("payload").join("bin").join(HELD_FILE), version).unwrap();
    write_actions_dir(&dir);
    build_small_package(&dir, &format!("{}{toml}", base_manifest(version)))
}

/// The application, started the way a person would leave it running:
/// holding the installed file, its pid recorded where the stop program
/// looks.
struct App {
    child: Child,
    pid_file: PathBuf,
}

impl App {
    fn start(machine: &Machine) -> App {
        let held = machine.install_root().join("bin").join(HELD_FILE);
        let pid_file = machine.actions_dir().join("app.pid");
        std::fs::create_dir_all(machine.actions_dir()).unwrap();
        let _ = std::fs::remove_file(&pid_file);
        let child = Command::new(action_executable())
            .arg("--hold")
            .arg(&held)
            .arg("--pid-file")
            .arg(&pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(HOLDER_ISOLATION)
            .spawn()
            .expect("the application starts");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !pid_file.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(pid_file.exists(), "the application recorded its pid");
        App { child, pid_file }
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The process a resume started, found by the pid file it wrote; ended so
/// that no test leaves an application behind.
fn end_resumed(pid_file: &Path) -> bool {
    let status = Command::new(action_executable())
        .arg("--stop")
        .arg(pid_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the stop program runs");
    status.code() == Some(0)
}

fn wait_for_pid_file(pid_file: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if pid_file.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn quiescence_actions(outcome: &Value) -> Vec<(String, String, String)> {
    outcome["actions"]
        .as_array()
        .map(|actions| {
            actions
                .iter()
                .filter(|a| a["phase"] == "quiesce" || a["phase"] == "resume")
                .map(|a| {
                    (
                        a["phase"].as_str().unwrap_or_default().to_string(),
                        a["name"].as_str().unwrap_or_default().to_string(),
                        a["status"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn finding_codes(outcome: &Value) -> Vec<String> {
    outcome["findings"]
        .as_array()
        .map(|f| {
            f.iter()
                .filter_map(|x| x["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn install_quiet(machine: &mut Machine, installer: &Path) -> Run {
    machine.run(installer, &["install", "--quiet", "--scope", "user"])
}

/// A running application is stopped before the Restart Manager looks, the
/// upgrade replaces the file it held, and the application is started again
/// once the upgrade has committed.
#[test]
fn a_running_application_is_stopped_before_the_upgrade_and_resumed_after_it() {
    let a = build_quiescent_package("upgrade", VERSION_A, &quiescence_toml(true, "fail"));
    let b = build_quiescent_package("upgrade", VERSION_B, &quiescence_toml(true, "fail"));
    let mut machine = Machine::new("quiesce-upgrade");
    let first = install_quiet(&mut machine, &a);
    assert_eq!(
        first.exit_code,
        Some(0),
        "{}\n{}",
        first.stdout,
        first.log_text()
    );
    assert!(
        quiescence_actions(&first.json()).is_empty(),
        "a first install runs no quiescence: {}",
        first.stdout
    );

    let mut app = App::start(&machine);
    let pid_file = app.pid_file.clone();
    let upgrade = install_quiet(&mut machine, &b);
    let outcome = upgrade.json();
    assert_eq!(
        upgrade.exit_code,
        Some(0),
        "{}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    assert!(!app.is_running(), "the stop program ended the application");
    let log = upgrade.log_text();
    let stopped_at = log.find("[quiescence_stopped]").expect("stopped");
    let rm_at = log.find("[restart_manager_checked]").unwrap_or(usize::MAX);
    let txn_at = log.find("[transaction_started]").expect("a transaction");
    assert!(
        stopped_at < rm_at && stopped_at < txn_at,
        "the stop precedes the Restart Manager and the transaction:\n{log}"
    );
    assert!(
        !log.contains("[restart_manager_holders]"),
        "nothing was left for the Restart Manager to find:\n{log}"
    );
    assert_eq!(
        quiescence_actions(&outcome),
        vec![
            ("quiesce".into(), "app".into(), "completed".into()),
            ("resume".into(), "app".into(), "completed".into()),
        ],
        "{outcome}"
    );
    let codes = finding_codes(&outcome);
    assert!(
        codes.contains(&"quiescence_stopped".to_string()),
        "{codes:?}"
    );
    assert!(
        codes.contains(&"quiescence_resumed".to_string()),
        "{codes:?}"
    );

    // The resumed application wrote a fresh pid file and holds the new
    // file; ending it is the test's job.
    assert!(
        wait_for_pid_file(&pid_file),
        "the resumed application recorded its pid"
    );
    assert!(
        end_resumed(&pid_file),
        "the resumed application was running"
    );

    // The installation is what 1.1.0 says, verified.
    let verify = machine.run(&b, &["verify", "--scope", "user"]);
    assert_eq!(verify.json()["status"], "ok", "{}", verify.stdout);
}

/// An application that is not running stays not running: the stop program
/// says so with its not-running code, and nothing is resumed.
#[test]
fn an_application_that_was_not_running_is_not_started() {
    let a = build_quiescent_package("idle", VERSION_A, &quiescence_toml(true, "fail"));
    let b = build_quiescent_package("idle", VERSION_B, &quiescence_toml(true, "fail"));
    let mut machine = Machine::new("quiesce-idle");
    install_quiet(&mut machine, &a);
    let upgrade = install_quiet(&mut machine, &b);
    let outcome = upgrade.json();
    assert_eq!(
        upgrade.exit_code,
        Some(0),
        "{}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    assert_eq!(
        quiescence_actions(&outcome),
        vec![("quiesce".into(), "app".into(), "completed".into())],
        "{outcome}"
    );
    let codes = finding_codes(&outcome);
    assert!(
        codes.contains(&"quiescence_not_running".to_string()),
        "{codes:?}"
    );
    assert!(
        !codes.contains(&"quiescence_resumed".to_string()),
        "{codes:?}"
    );
    assert!(
        !machine.actions_dir().join("app.pid").exists(),
        "nothing was started"
    );
}

/// The quiescence succeeds, then the Restart Manager finds another holder
/// that will not close: the run stops with `package_in_use`, nothing is
/// mutated, and the application that was stopped is started again.
#[test]
fn a_stopped_application_is_resumed_when_the_restart_manager_refuses_the_run() {
    let a = build_quiescent_package("refused", VERSION_A, &quiescence_toml(true, "fail"));
    let b = build_quiescent_package("refused", VERSION_B, &quiescence_toml(true, "fail"));
    let mut machine = Machine::new("quiesce-refused");
    install_quiet(&mut machine, &a);

    // The application, and a second holder the package knows nothing
    // about, on the same file.
    let mut app = App::start(&machine);
    let pid_file = app.pid_file.clone();
    let held = machine.install_root().join("bin").join(HELD_FILE);
    let stranger_pid = machine.actions_dir().join("stranger.pid");
    let _ = std::fs::remove_file(&stranger_pid);
    let mut stranger = Command::new(action_executable())
        .arg("--hold")
        .arg(&held)
        .arg("--pid-file")
        .arg(&stranger_pid)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(HOLDER_ISOLATION)
        .spawn()
        .expect("the stranger starts");
    assert!(wait_for_pid_file(&stranger_pid));
    assert!(
        matches!(stranger.try_wait(), Ok(None)),
        "the stranger is running"
    );
    assert!(
        std::fs::rename(&held, held.with_extension("moved")).is_err(),
        "the stranger's hold blocks a rename"
    );

    let upgrade = install_quiet(&mut machine, &b);
    let outcome = upgrade.json();
    let _ = stranger.kill();
    let _ = stranger.wait();
    assert_eq!(
        upgrade.exit_code,
        Some(6),
        "the stranger keeps the file: {}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    assert_eq!(outcome["code"], "package_in_use", "{outcome}");
    assert!(!app.is_running(), "the application was stopped first");
    assert!(
        upgrade.log_text().contains("[restart_manager_holders]"),
        "the stranger was found:\n{}",
        upgrade.log_text()
    );
    assert!(
        upgrade.log_text().contains("[quiescence_resumed]"),
        "the application was started again before the refusal:\n{}",
        upgrade.log_text()
    );
    assert!(
        wait_for_pid_file(&pid_file),
        "the resumed application recorded its pid"
    );
    assert!(end_resumed(&pid_file));

    // Nothing changed: still 1.0.0, verified.
    let verify = machine.run(&a, &["verify", "--scope", "user"]);
    assert_eq!(verify.json()["status"], "ok", "{}", verify.stdout);
    assert_eq!(verify.json()["installation"]["version"], VERSION_A);
}

/// An uninstall stops the application through the entry the installation
/// recorded — the installer that brought it is not consulted — and never
/// starts it again.
#[test]
fn an_uninstall_stops_the_application_and_does_not_resume_it() {
    let a = build_quiescent_package("uninstall", VERSION_A, &quiescence_toml(true, "fail"));
    let mut machine = Machine::new("quiesce-uninstall");
    install_quiet(&mut machine, &a);
    let mut app = App::start(&machine);
    let pid_file = app.pid_file.clone();

    let uninstall = machine.uninstall_through_the_copy();
    let outcome = uninstall.json();
    assert_eq!(
        uninstall.exit_code,
        Some(0),
        "{}\n{}",
        uninstall.stdout,
        uninstall.log_text()
    );
    assert_eq!(outcome["outcome"], "uninstalled");
    assert!(!app.is_running(), "the application was stopped");
    assert_eq!(
        quiescence_actions(&outcome),
        vec![("quiesce".into(), "app".into(), "completed".into())],
        "{outcome}"
    );
    assert!(
        !finding_codes(&outcome).contains(&"quiescence_resumed".to_string()),
        "{outcome}"
    );
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !pid_file.exists(),
        "nothing was started after the uninstall"
    );
    assert!(!machine.install_root().exists());
}

/// A stop program that fails ends the run before anything is mutated, and
/// with `continue` it is recorded and the run goes on.
#[test]
fn a_failing_stop_program_stops_the_run_or_is_recorded_as_told() {
    let failing = "[[quiescence]]\nname = \"app\"\n\n[quiescence.stop]\nkind = \"exe\"\n\
                   source = \"actions/TigerSetupTestAction.exe\"\narguments = [\"--exit\", \"7\"]\n";
    let a = build_quiescent_package("failing", VERSION_A, failing);
    let b = build_quiescent_package("failing", VERSION_B, failing);
    let mut machine = Machine::new("quiesce-failing");
    install_quiet(&mut machine, &a);
    let upgrade = install_quiet(&mut machine, &b);
    let outcome = upgrade.json();
    assert_eq!(
        upgrade.exit_code,
        Some(1),
        "{}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    assert_eq!(outcome["code"], "quiescence_failed", "{outcome}");
    assert!(
        !upgrade.log_text().contains("[transaction_started]"),
        "nothing was mutated:\n{}",
        upgrade.log_text()
    );
    let verify = machine.run(&a, &["verify", "--scope", "user"]);
    assert_eq!(verify.json()["installation"]["version"], VERSION_A);

    let continuing = format!("{}on_failure = \"continue\"\n", failing);
    let b2 = build_quiescent_package("continuing", VERSION_B, &continuing);
    let upgrade = install_quiet(&mut machine, &b2);
    let outcome = upgrade.json();
    assert_eq!(
        upgrade.exit_code,
        Some(0),
        "{}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    assert!(
        finding_codes(&outcome).contains(&"quiescence_failed_continued".to_string()),
        "{outcome}"
    );
    assert_eq!(
        quiescence_actions(&outcome),
        vec![("quiesce".into(), "app".into(), "failed_continued".into())],
        "{outcome}"
    );
}

/// What the package declares is what `inspect` shows, in both clients.
#[test]
fn inspect_lists_the_quiescence_entries() {
    let a = build_quiescent_package("inspect", VERSION_A, &quiescence_toml(true, "fail"));
    let inspection = tigersetup_build::inspect::inspect(&a).unwrap();
    assert!(inspection.is_ok());
    let report = inspection.to_json();
    assert_eq!(report["quiescence"][0]["name"], "app");
    assert_eq!(report["quiescence"][0]["not_running_codes"][0], 3);
    assert_eq!(report["quiescence"][0]["stop"]["phase"], "quiesce");
    assert_eq!(report["quiescence"][0]["resume"]["phase"], "resume");
    assert_eq!(
        report["quiescence"][0]["run_on"],
        serde_json::json!(["upgrade", "reinstall", "repair", "uninstall"])
    );
    assert!(inspection.to_text().contains("Quiesce:   app"));
}
