//! Two things that happen around a transaction rather than inside it: the
//! applications holding the product's files are closed through the Restart
//! Manager before the transaction opens, and the installation the product
//! migrates from is removed by its own uninstaller before that.
//!
//! Both are outside the transaction on purpose, so a failure in either
//! leaves the machine exactly as it was.

mod common;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use tigersetup_engine::win::registry::{Data, KeyPath};

/// The holder gets a console of its own, invisible, and is the root of its
/// own process group.
///
/// The Restart Manager closes a *console* holder by delivering a console
/// control event to it, and such an event reaches every process attached to
/// that console rather than only the one being closed. A holder that
/// inherited this suite's console would therefore take the test runner down
/// with it — and, when the suite runs from a terminal or an agent session,
/// that terminal and everything else sharing the console, killed with
/// `STATUS_CONTROL_C_EXIT` and no diagnostic of any kind. Giving the holder
/// its own console contains the event; it stays a console application with
/// no message loop, which is the thing the quiescence path is being tested
/// against.
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const HOLDER_ISOLATION: u32 = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;

/// Holds `path` open the way a running application does — readable by
/// others, but not replaceable or deletable — until it is dropped.
struct Holder {
    child: Child,
}

impl Holder {
    fn open(path: &Path) -> Holder {
        let script = format!(
            "$f = [System.IO.File]::Open('{}', 'Open', 'Read', 'Read'); \
             Write-Output 'holding'; Start-Sleep -Seconds 120",
            path.display()
        );
        let child = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(HOLDER_ISOLATION)
            .spawn()
            .expect("powershell.exe starts");
        let holder = Holder { child };
        holder.wait_until_held(path);
        holder
    }

    /// Waits until the file really cannot be replaced, so that the test
    /// never races the process it started.
    fn wait_until_held(&self, path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if !can_replace(path) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("{} was never held open", path.display());
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Whether this process could replace the file right now: the same access
/// an installation needs, asked for without changing anything.
fn can_replace(path: &Path) -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .is_ok()
}

/// An upgrade that has to replace a file another process is holding: the
/// Restart Manager closes the holder and the upgrade completes, or the run
/// stops with `package_in_use` having changed nothing. Never a mixture.
#[test]
fn an_application_holding_a_file_an_upgrade_replaces_is_closed_or_the_run_stops() {
    let (a, b) = (&fixture().a, &fixture().b);
    let mut machine = Machine::new("in-use-upgrade");
    machine.install(a);
    machine.assert_verified(a);

    // 1.1.0 rewrites this file, so the upgrade cannot leave the holder be.
    let held = machine
        .install_root()
        .join("bin")
        .join("TigerSetupTestApp.exe");
    let mut holder = Holder::open(&held);
    let holder_id = holder.id();

    let run = machine.install(b);
    let document = run.json();
    match run.exit_code {
        Some(0) => {
            assert!(
                run.log_has("[restart_manager_holders]"),
                "the holder was found: {}",
                run.log_text()
            );
            assert!(run.log_has("[restart_manager_shutdown]"));
            assert!(
                run.log_text().contains(&holder_id.to_string()),
                "the log names the process that held the file: {}",
                run.log_text()
            );
            assert!(
                !holder.is_running(),
                "the process that held the file is gone"
            );
            assert!(
                !document["closed_applications"].is_null(),
                "the outcome names what it closed: {}",
                run.stdout
            );
            machine.assert_verified(b);
            eprintln!("the Restart Manager closed the console holder and the upgrade ran");
        }
        Some(6) => {
            assert_eq!(document["code"], "package_in_use", "{}", run.stdout);
            assert!(
                !run.log_has("[transaction_started]"),
                "nothing was mutated: {}",
                run.log_text()
            );
            machine.assert_verified(a);
            eprintln!("the console holder stayed, and the upgrade stopped with package_in_use");
        }
        other => panic!("unexpected exit {other:?}: {}", run.stdout),
    }
}

/// An uninstall that has to delete a held file behaves the same way.
#[test]
fn an_application_holding_a_file_an_uninstall_removes_is_closed_or_the_run_stops() {
    let a = &fixture().a;
    let mut machine = Machine::new("in-use-uninstall");
    machine.install(a);

    let held = machine
        .install_root()
        .join("bin")
        .join("TigerSetupTestApp.exe");
    let mut holder = Holder::open(&held);

    let run = machine.uninstall(a);
    match run.exit_code {
        Some(0) => {
            assert!(
                run.log_has("[restart_manager_shutdown]"),
                "{}",
                run.log_text()
            );
            assert!(!holder.is_running());
            machine.assert_absent(a);
        }
        Some(6) => {
            assert_eq!(run.json()["code"], "package_in_use", "{}", run.stdout);
            assert!(!run.log_has("[transaction_started]"));
            machine.assert_verified(a);
        }
        other => panic!("unexpected exit {other:?}: {}", run.stdout),
    }
}

/// A first installation writes files nobody can be holding, so it never
/// opens a session and never mentions the Restart Manager.
#[test]
fn a_first_install_asks_nothing_of_the_restart_manager() {
    let a = &fixture().a;
    let mut machine = Machine::new("in-use-none");
    let run = machine.install(a);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    assert!(
        !run.log_has("[restart_manager_holders]"),
        "{}",
        run.log_text()
    );
    machine.assert_verified(a);
}

/// The package declares the installation it replaces; the machine holds
/// one; its own quiet uninstaller runs once, before the product is
/// installed, and the run records it.
#[test]
fn a_legacy_installation_is_removed_by_its_own_uninstaller_before_the_install() {
    let dir = scratch("legacy");
    let installer = build_small_package(&dir, &legacy_manifest());
    let mut machine = Machine::new("legacy");
    let legacy_key =
        format!("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{LEGACY_KEY_NAME}");
    // The legacy uninstaller removes its own registration, which is the
    // evidence TigerSetup waits for.
    let marker = dir.join("legacy-ran.txt");
    seed_legacy(
        &machine,
        &legacy_key,
        &quiet_uninstall_command(&machine, &legacy_key, &marker),
    );

    let run = machine.run(&installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    let document = run.json();
    assert_eq!(document["legacy"]["key"], legacy_key, "{}", run.stdout);
    assert_eq!(document["legacy"]["uninstalled"], true);
    assert!(run.log_has("[legacy_found]"));
    assert!(run.log_has("[legacy_uninstalled]"));
    assert!(marker.exists(), "the legacy uninstaller ran");
    assert!(!machine.key_exists(&legacy_key), "its registration is gone");
    assert!(machine.install_root().join("bin").join("app.txt").exists());

    // A second run finds no legacy registration and says nothing about one.
    std::fs::remove_file(&marker).unwrap();
    let again = machine.run(&installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(again.exit_code, Some(0), "{}", again.stdout);
    assert!(again.json()["legacy"].is_null(), "{}", again.stdout);
    assert!(
        !marker.exists(),
        "the legacy uninstaller does not run twice"
    );
    assert!(!again.log_has("[legacy_found]"));
}

/// A legacy uninstaller that reports success but leaves its registration
/// behind stops the run, before any product resource is written.
#[test]
fn a_legacy_uninstaller_that_leaves_its_registration_stops_the_run() {
    let dir = scratch("legacy-stuck");
    let installer = build_small_package(&dir, &legacy_manifest());
    let mut machine = Machine::new("legacy-stuck");
    let legacy_key =
        format!("HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{LEGACY_KEY_NAME}");
    let marker = dir.join("legacy-ran.txt");
    // Exits 0 and touches the marker, but never removes its own key.
    let command = format!(
        "\"{}\" /c type nul > \"{}\"",
        cmd_exe().display(),
        marker.display()
    );
    seed_legacy(&machine, &legacy_key, &command);

    let started = Instant::now();
    let run = machine.run(&installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(1), "{}", run.stdout);
    assert_eq!(
        run.json()["code"],
        "legacy_uninstall_failed",
        "{}",
        run.stdout
    );
    assert!(marker.exists(), "it did run");
    assert!(
        started.elapsed() >= Duration::from_secs(10),
        "the run waits for the registration to disappear before giving up"
    );
    assert!(
        !machine.install_root().exists(),
        "no product resource was written"
    );
    assert!(!run.log_has("[transaction_started]"));
}

const LEGACY_KEY_NAME: &str = "TigerSetupTestApp_is1";

fn legacy_manifest() -> String {
    format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{VERSION_A}"
publisher = "IT Tiger"

[install]
scopes = ["user", "machine"]

[[files]]
source = "payload/**"

[legacy]
installer_type = "inno"
registration_key = "{LEGACY_KEY_NAME}"
"#
    )
}

fn cmd_exe() -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").expect("SystemRoot"))
        .join("System32")
        .join("cmd.exe")
}

/// The command a legacy uninstaller publishes here: it leaves a marker and
/// deletes its own registration, as a real one does.
fn quiet_uninstall_command(machine: &Machine, legacy_key: &str, marker: &Path) -> String {
    let parsed = KeyPath::parse(legacy_key).unwrap();
    let physical = format!(
        "HKCU\\{}\\{}\\{}",
        machine.registry_prefix,
        parsed.hive.as_str(),
        parsed.subkey
    );
    format!(
        "\"{}\" /c type nul > \"{}\" & reg delete \"{physical}\" /f",
        cmd_exe().display(),
        marker.display()
    )
}

/// Writes an Add/Remove Programs registration of the kind a previous
/// installer technology leaves behind.
fn seed_legacy(machine: &Machine, legacy_key: &str, quiet_uninstall_string: &str) {
    let key = KeyPath::parse(legacy_key).unwrap();
    let roots = machine.roots();
    tigersetup_engine::win::registry::create_key(&roots, &key).unwrap();
    for (name, value) in [
        ("DisplayName", PRODUCT_NAME.to_string()),
        ("DisplayVersion", "0.9.0".to_string()),
        ("QuietUninstallString", quiet_uninstall_string.to_string()),
    ] {
        tigersetup_engine::win::registry::write_value(&roots, &key, name, &Data::String(value))
            .unwrap();
    }
    assert!(machine.key_exists(legacy_key));
}
