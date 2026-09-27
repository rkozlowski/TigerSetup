//! Migration from a legacy installer (`TigerSetup-Design.md` §5.12): the
//! installation the product replaces is removed by its own uninstaller,
//! once, before the product is installed, and outside the transaction, so
//! a failure leaves the machine exactly as it was.
//!
//! The uninstaller here is `TigerSetupTestAction.exe` shaped like Inno
//! Setup's: the program the registration names hands the work to a copy of
//! itself and exits at once, and the copy removes the registration first and
//! the install root last. The copy waits at a gate the test opens, so what
//! the migration does while the old uninstall is still under way is
//! observed, not raced.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::*;
use tigersetup_engine::format::identity::Scope;
use tigersetup_engine::format::metadata::Legacy;
use tigersetup_engine::report::{Event, EventSink, Reporter};
use tigersetup_engine::win::process::{self, TreeState};
use tigersetup_engine::win::registry::{Data, KeyPath};

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

/// The legacy registration key, as the engine names it in `machine`'s scope.
fn legacy_key(machine: &Machine) -> String {
    let hive = match machine.scope {
        Scope::Machine => "HKLM",
        _ => "HKCU",
    };
    format!("{hive}\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{LEGACY_KEY_NAME}")
}

/// Where `key` really is: every hive the engine touches lives under
/// `HKCU\<registry_prefix>`. Returned without the `HKCU\`.
fn physical_subkey(machine: &Machine, key: &str) -> String {
    let parsed = KeyPath::parse(key).unwrap();
    format!(
        "{}\\{}\\{}",
        machine.registry_prefix,
        parsed.hive.as_str(),
        parsed.subkey
    )
}

/// Writes an Add/Remove Programs registration of the kind a previous
/// installer technology leaves behind.
fn seed_legacy(
    machine: &Machine,
    key: &str,
    quiet_uninstall_string: &str,
    location: Option<&Path>,
) {
    let parsed = KeyPath::parse(key).unwrap();
    let roots = machine.roots();
    tigersetup_engine::win::registry::create_key(&roots, &parsed).unwrap();
    let mut values = vec![
        ("DisplayName", PRODUCT_NAME.to_string()),
        ("DisplayVersion", "0.9.0".to_string()),
        ("QuietUninstallString", quiet_uninstall_string.to_string()),
    ];
    if let Some(location) = location {
        values.push(("InstallLocation", format!("{}\\", location.display())));
    }
    for (name, value) in values {
        tigersetup_engine::win::registry::write_value(&roots, &parsed, name, &Data::String(value))
            .unwrap();
    }
    assert!(machine.key_exists(key));
}

/// An installation the legacy installer left: its root, holding its
/// uninstaller's data file.
fn seed_legacy_root(root: &Path) -> PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let data = root.join("unins000.dat");
    std::fs::write(&data, b"legacy").unwrap();
    data
}

/// The Inno-shaped uninstaller's arguments: hand off to a second phase that
/// deletes the key, waits at `gate`, then deletes the data file and removes
/// the root if that left it empty.
fn hand_off_arguments(machine: &Machine, key: &str, gate: &Path, root: &Path) -> Vec<String> {
    vec![
        "--hand-off".into(),
        "--delete-key".into(),
        physical_subkey(machine, key),
        "--wait-for".into(),
        gate.display().to_string(),
        "--delete".into(),
        root.join("unins000.dat").display().to_string(),
        "--remove-dir".into(),
        root.display().to_string(),
    ]
}

fn quote(argument: &str) -> String {
    if argument.contains(' ') {
        format!("\"{argument}\"")
    } else {
        argument.to_string()
    }
}

fn hand_off_command(machine: &Machine, key: &str, gate: &Path, root: &Path) -> String {
    let mut line = format!("\"{}\"", action_executable().display());
    for argument in hand_off_arguments(machine, key, gate, root) {
        line.push(' ');
        line.push_str(&quote(&argument));
    }
    line
}

/// Waits, bounded, until the condition holds.
fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Opens the gate when dropped, so that a failing test never leaves the
/// fake uninstaller waiting at it.
struct Gate(PathBuf);

impl Gate {
    fn new(dir: &Path) -> Gate {
        Gate(dir.join("gate"))
    }

    fn open(&self) {
        std::fs::write(&self.0, b"open").unwrap();
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, b"open");
    }
}

#[derive(Default)]
struct Events(Vec<(String, String)>);

impl EventSink for Events {
    fn event(&mut self, event: &Event) {
        self.0.push((event.code.to_string(), event.message.clone()));
    }
}

impl Events {
    fn message(&self, code: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(c, _)| c == code)
            .map(|(_, message)| message.as_str())
    }
}

fn inno(key_name: &str) -> Legacy {
    Legacy {
        installer_type: "inno".into(),
        registration_key: key_name.into(),
    }
}

/// The wait primitive: the program started exits at once, the key goes, and
/// the tree is still not over while its second phase works — it ends only
/// when the last process of the uninstaller does.
#[test]
fn a_hand_off_keeps_the_uninstall_running_after_the_started_program_exits() {
    let dir = scratch("legacy-tree");
    let machine = Machine::new("legacy-tree");
    let key = legacy_key(&machine);
    let root = dir.join("Old App");
    seed_legacy(&machine, &key, "unused", None);
    seed_legacy_root(&root);
    let gate = Gate::new(&dir);

    let mut tree = process::start_tree(
        &action_executable(),
        &hand_off_arguments(&machine, &key, &gate.0, &root),
    )
    .expect("the fake uninstaller starts");
    until("the second phase removed the key", || {
        !machine.key_exists(&key)
    });

    // The gate is shut, so the tree cannot be over: what this wait returns
    // does not depend on how long it waits.
    match tree.wait(Duration::from_secs(5)).unwrap() {
        TreeState::Running {
            root_exit_code,
            running,
        } => {
            assert_eq!(root_exit_code, Some(0), "the started program has exited");
            assert!(
                running.iter().any(|member| member
                    .image
                    .as_ref()
                    .is_some_and(|image| image.ends_with(ACTION_FILE_NAME))),
                "the second phase is named: {running:?}"
            );
        }
        ended => panic!("the tree ended with its second phase still at the gate: {ended:?}"),
    }
    assert!(root.is_dir(), "the old install root is still there");

    gate.open();
    match tree.wait(Duration::from_secs(60)).unwrap() {
        TreeState::Ended {
            exit_code,
            processes,
            root_exited,
            ended,
        } => {
            assert_eq!(exit_code, 0);
            // The started program, its second phase and the console host
            // each gets. The proof that the second phase was waited for is
            // the `Running` state above; this count is only what is recorded.
            assert!(processes >= 3, "{processes}");
            assert!(root_exited <= ended);
        }
        running => panic!("the tree did not end once the gate opened: {running:?}"),
    }
    assert!(
        !root.exists(),
        "the second phase removed the root before the tree ended"
    );
}

/// The migration itself, in each scope: a key that has already gone does
/// not release it while the uninstaller is still at work; the uninstaller
/// ending does, and by then the old install root is gone.
fn the_migration_waits_for_the_whole_uninstaller(scope: Scope) {
    let name = format!("legacy-wait-{}", scope.as_str());
    let dir = scratch(&name);
    let machine = Machine::in_scope(&name, scope);
    let key = legacy_key(&machine);
    let root = machine.install_root();
    let gate = Gate::new(&dir);
    seed_legacy(
        &machine,
        &key,
        &hand_off_command(&machine, &key, &gate.0, &root),
        Some(&root),
    );
    seed_legacy_root(&root);
    let roots = machine.roots();
    let locations = tigersetup_engine::scope::locations(scope);
    let legacy = inno(LEGACY_KEY_NAME);

    let mut events = Events::default();
    let outcome = std::thread::scope(|threads| {
        let migration = threads.spawn(|| {
            let mut reporter = Reporter::new(&mut events);
            tigersetup_engine::legacy::migrate_within(
                Some(&legacy),
                &locations,
                &roots,
                &mut reporter,
                Duration::from_secs(60),
            )
        });
        until("the second phase removed the key", || {
            !machine.key_exists(&key)
        });
        // Gate shut: the migration cannot have finished, whatever the timing.
        assert!(
            !migration.is_finished(),
            "the migration returned while the legacy uninstaller was still at work"
        );
        assert!(root.is_dir(), "the old install root is still there");
        gate.open();
        migration.join().unwrap()
    });
    let info = outcome
        .expect("the migration succeeds")
        .expect("a legacy installation was found");
    assert!(info.uninstalled);
    assert_eq!(info.key, key);
    assert!(
        !root.exists(),
        "the old install root was gone when the migration returned"
    );
    // Recorded, for the evidence; the proof is `is_finished` above.
    assert!(info.processes >= 2, "{info:?}");
    let uninstalled = events.message("legacy_uninstalled").expect("recorded");
    assert!(
        uninstalled.contains(&format!("processes={}", info.processes)),
        "{uninstalled}"
    );
    assert_eq!(info.location_remains, None, "nothing stayed behind");
    assert!(
        events.message("legacy_location_remains").is_none(),
        "{:?}",
        events.0
    );
}

#[test]
fn a_per_user_migration_waits_for_the_whole_uninstaller() {
    the_migration_waits_for_the_whole_uninstaller(Scope::User);
}

#[test]
fn a_machine_migration_waits_for_the_whole_uninstaller() {
    the_migration_waits_for_the_whole_uninstaller(Scope::Machine);
}

/// An uninstaller that does not finish in time stops the migration with a
/// diagnostic naming what still runs — even though its key has gone — and
/// is left running rather than ended halfway.
#[test]
fn an_uninstaller_that_does_not_finish_stops_the_migration_and_is_named() {
    let dir = scratch("legacy-timeout");
    let machine = Machine::new("legacy-timeout");
    let key = legacy_key(&machine);
    let root = dir.join("Old App");
    let gate = Gate::new(&dir);
    seed_legacy(
        &machine,
        &key,
        &hand_off_command(&machine, &key, &gate.0, &root),
        Some(&root),
    );
    seed_legacy_root(&root);

    let mut events = Events::default();
    let mut reporter = Reporter::new(&mut events);
    let started = Instant::now();
    let err = tigersetup_engine::legacy::migrate_within(
        Some(&inno(LEGACY_KEY_NAME)),
        &tigersetup_engine::scope::locations(Scope::User),
        &machine.roots(),
        &mut reporter,
        Duration::from_secs(2),
    )
    .expect_err("the migration gives up");
    let elapsed = started.elapsed();
    assert_eq!(err.code, "legacy_uninstall_failed");
    assert!(err.message.contains("within 2 s"), "{}", err.message);
    assert!(err.message.contains(ACTION_FILE_NAME), "{}", err.message);
    assert!(
        elapsed < Duration::from_secs(30),
        "the wait is bounded: {elapsed:?}"
    );
    assert!(!machine.key_exists(&key), "the key had gone");
    assert!(events.message("legacy_uninstalled").is_none());

    // It was left running, not killed: opening the gate lets it finish.
    gate.open();
    until("the abandoned uninstaller finished on its own", || {
        !root.exists()
    });
}

/// What the old uninstaller leaves is not TigerSetup's, and the migration
/// does not wait for it to go: it finishes, and says what stayed.
#[test]
fn a_root_the_uninstaller_kept_is_reported_and_not_waited_for() {
    let dir = scratch("legacy-kept");
    let machine = Machine::new("legacy-kept");
    let key = legacy_key(&machine);
    let root = dir.join("Old App");
    let gate = Gate::new(&dir);
    gate.open();
    seed_legacy(
        &machine,
        &key,
        &hand_off_command(&machine, &key, &gate.0, &root),
        Some(&root),
    );
    seed_legacy_root(&root);
    std::fs::write(root.join("notes.txt"), b"the user's").unwrap();

    let mut events = Events::default();
    let mut reporter = Reporter::new(&mut events);
    let info = tigersetup_engine::legacy::migrate_within(
        Some(&inno(LEGACY_KEY_NAME)),
        &tigersetup_engine::scope::locations(Scope::User),
        &machine.roots(),
        &mut reporter,
        Duration::from_secs(60),
    )
    .expect("the migration succeeds")
    .expect("a legacy installation was found");
    drop(reporter);
    assert!(root.join("notes.txt").exists());
    let remains = info.location_remains.expect("reported in the outcome");
    assert!(remains.contains("Old App"), "{remains}");
    assert_eq!(
        events.message("legacy_location_remains"),
        Some(remains.as_str())
    );
}

/// The whole ownership lifecycle through Setup.exe, in each scope: the
/// legacy installation sits in the product's own install root; after the
/// migration and the install, TigerSetup owns that root, so its uninstall
/// removes it.
fn a_migrated_install_root_is_owned_and_removed_on_uninstall(scope: Scope) {
    let name = format!("legacy-own-{}", scope.as_str());
    let dir = scratch(&name);
    let installer = build_small_package(&dir, &legacy_manifest());
    let mut machine = Machine::in_scope(&name, scope);
    let key = legacy_key(&machine);
    let root = machine.install_root();
    let gate = Gate::new(&dir);
    seed_legacy(
        &machine,
        &key,
        &hand_off_command(&machine, &key, &gate.0, &root),
        Some(&root),
    );
    seed_legacy_root(&root);
    let scope_name = machine.scope_name();

    let mut started = machine.start(&installer, &["install", "--quiet", "--scope", scope_name]);
    until("the second phase removed the key", || {
        !machine.key_exists(&key)
    });
    assert!(
        !started.ended(),
        "the install finished while the legacy uninstaller was still at work"
    );
    gate.open();
    let run = started.finish();
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    let legacy = &run.json()["legacy"];
    assert_eq!(legacy["uninstalled"], true, "{}", run.stdout);
    assert!(
        legacy["processes"].as_u64().unwrap_or(0) >= 2,
        "{}",
        run.stdout
    );
    assert!(legacy["location_remains"].is_null(), "{}", run.stdout);
    assert!(run.log_has("[legacy_uninstalled]"), "{}", run.log_text());
    assert!(
        !run.log_has("[legacy_location_remains]"),
        "{}",
        run.log_text()
    );
    assert!(root.join("bin").join("app.txt").exists());
    assert!(!root.join("unins000.dat").exists());

    let removed = machine.run(&installer, &["uninstall", "--quiet", "--scope", scope_name]);
    assert_eq!(removed.exit_code, Some(0), "{}", removed.stdout);
    assert!(
        !root.exists(),
        "the uninstall removed the install root the migration handed over: {}",
        removed.log_text()
    );
}

#[test]
fn a_per_user_migrated_install_root_is_owned_and_removed_on_uninstall() {
    a_migrated_install_root_is_owned_and_removed_on_uninstall(Scope::User);
}

#[test]
fn a_machine_migrated_install_root_is_owned_and_removed_on_uninstall() {
    a_migrated_install_root_is_owned_and_removed_on_uninstall(Scope::Machine);
}

/// The package declares the installation it replaces; the machine holds
/// one; its own quiet uninstaller runs once, before the product is
/// installed, and the run records it.
#[test]
fn a_legacy_installation_is_removed_by_its_own_uninstaller_before_the_install() {
    let dir = scratch("legacy");
    let installer = build_small_package(&dir, &legacy_manifest());
    let mut machine = Machine::new("legacy");
    let key = legacy_key(&machine);
    // The legacy uninstaller removes its own registration, as a real one does.
    let marker = dir.join("legacy-ran.txt");
    let command = format!(
        "\"{}\" /c type nul > \"{}\" & reg delete \"HKCU\\{}\" /f",
        cmd_exe().display(),
        marker.display(),
        physical_subkey(&machine, &key)
    );
    seed_legacy(&machine, &key, &command, None);

    let run = machine.run(&installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    let document = run.json();
    assert_eq!(document["legacy"]["key"], key, "{}", run.stdout);
    assert_eq!(document["legacy"]["uninstalled"], true);
    assert!(run.log_has("[legacy_found]"));
    assert!(run.log_has("[legacy_uninstalled]"));
    assert!(marker.exists(), "the legacy uninstaller ran");
    assert!(!machine.key_exists(&key), "its registration is gone");
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
    let key = legacy_key(&machine);
    let marker = dir.join("legacy-ran.txt");
    // Exits 0 and touches the marker, but never removes its own key.
    let command = format!(
        "\"{}\" /c type nul > \"{}\"",
        cmd_exe().display(),
        marker.display()
    );
    seed_legacy(&machine, &key, &command, None);

    let run = machine.run(&installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(1), "{}", run.stdout);
    let document = run.json();
    assert_eq!(
        document["code"], "legacy_uninstall_failed",
        "{}",
        run.stdout
    );
    assert!(
        document.to_string().contains("still registered"),
        "{}",
        run.stdout
    );
    assert!(marker.exists(), "it did run");
    assert!(
        !machine.install_root().exists(),
        "no product resource was written"
    );
    assert!(!run.log_has("[transaction_started]"));
}
