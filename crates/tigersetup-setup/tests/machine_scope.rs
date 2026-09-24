//! Machine scope end to end, and everything that only machine scope makes
//! visible: the locations it uses, the protection of its state directory,
//! the confinement of a privileged delete to the roots the database
//! records, the elevated run handing its outcome document back to its
//! unelevated parent, and the platform baseline.
//!
//! These run unelevated. The fixture redirects every known folder into a
//! tree the test process owns, so the engine finds a machine-scope state
//! directory it can already write and does not ask for an administrator —
//! which is exactly the seam the design documents. What genuinely needs an
//! administrator (writing the access control list, answering a real
//! elevation prompt) is asserted where this process happens to be elevated
//! and proved in the lab otherwise; each such test says so.

mod common;

use std::path::Path;

use common::*;
use tigersetup_engine::format::identity::Scope;
use tigersetup_engine::win::registry::Data;

/// The whole cycle in machine scope: the roots, the hive, the shared Start
/// Menu folder and the machine `Path` are the scope's, and nothing of the
/// user's scope is touched.
#[test]
fn machine_scope_installs_verifies_upgrades_and_uninstalls() {
    let (a, b) = (&fixture().a, &fixture().b);
    let mut machine = Machine::in_scope("machine-cycle", Scope::Machine);

    let install = machine.install(a);
    assert_eq!(install.exit_code, Some(0), "{}", install.stdout);
    let document = install.json();
    assert_eq!(document["installation"]["scope"], "machine");
    assert_eq!(
        document["installation"]["install_root"],
        machine.install_root().display().to_string()
    );
    assert!(
        machine.install_root().starts_with(&machine.programfiles),
        "machine scope installs under %PROGRAMFILES%"
    );
    assert!(
        machine.state_dir().starts_with(&machine.programdata),
        "machine scope keeps its state under %PROGRAMDATA%"
    );
    machine.assert_verified(a);

    // Every registry location is the machine hive's.
    assert_eq!(
        machine.product_key(),
        "HKLM\\Software\\IT Tiger\\TigerSetupTestApp"
    );
    assert!(machine.key_exists(&machine.registration_key()));
    assert_eq!(
        machine.read_value(&machine.product_key(), "Version"),
        Some(Data::String(VERSION_A.into()))
    );
    assert!(
        machine
            .environment_key()
            .contains("Session Manager\\Environment"),
        "the machine PATH lives in the Session Manager key"
    );
    assert_eq!(machine.path_entry_count(), 1);
    // The Start Menu link is the shared one, not the user's.
    assert!(
        machine
            .start_menu_link()
            .starts_with(&machine.common_programs)
    );
    assert!(machine.start_menu_link().exists());
    assert!(
        !machine
            .programs
            .join(format!("{PRODUCT_NAME}.lnk"))
            .exists(),
        "no link is written to the user's own Start Menu"
    );
    // Nothing of the user's scope was created.
    assert!(!machine.key_exists(PRODUCT_KEY));
    assert!(!machine.key_exists(REGISTRATION_KEY));

    let upgrade = machine.install(b);
    assert_eq!(upgrade.exit_code, Some(0), "{}", upgrade.stdout);
    machine.assert_verified(b);
    assert_eq!(
        machine.read_value(&machine.product_key(), "Version"),
        Some(Data::String(VERSION_B.into()))
    );

    let uninstall = machine.uninstall(b);
    assert_eq!(uninstall.exit_code, Some(0), "{}", uninstall.stdout);
    machine.assert_absent(b);
    assert!(
        !machine.key_exists(&machine.vendor_key()),
        "the vendor key TigerSetup created is gone"
    );
}

/// Reading machine-scope state never needs an administrator: a standard
/// user can read the database, which is what the access control list is
/// arranged to allow.
#[test]
fn reading_machine_scope_state_needs_no_administrator() {
    let a = &fixture().a;
    let mut machine = Machine::in_scope("machine-read", Scope::Machine);
    machine.install(a);

    let inspect = machine.inspect(a);
    assert_eq!(inspect.exit_code, Some(0), "{}", inspect.stdout);
    let report = inspect.json();
    assert_eq!(report["installation"]["scope"], "machine");
    assert_eq!(
        report["owned"]["registration_key"],
        machine.registration_key()
    );
    assert_eq!(
        report["owned"]["path_entries"][0]["hive_key"],
        machine.environment_key()
    );

    let verify = machine.verify(a);
    assert_eq!(verify.exit_code, Some(0), "{}", verify.stdout);
    assert_eq!(verify.json()["status"], "ok");
}

/// A state database this process may not open is reported as a finding,
/// not as an error the caller cannot act on, and reading still never asks
/// for an administrator.
#[test]
fn a_state_database_this_user_cannot_open_is_reported_rather_than_thrown() {
    use tigersetup_engine::win::acl;

    let a = &fixture().a;
    let mut machine = Machine::in_scope("machine-unreadable", Scope::Machine);
    machine.install(a);

    // Only the local system may open it, as a machine-scope database looks
    // to a user who has been shut out of it.
    let state_db = machine.state_dir().join("state.db");
    acl::set_dacl(&state_db, "D:(A;;FA;;;SY)").unwrap();

    let verify = machine.verify(a);
    assert_eq!(verify.exit_code, Some(1), "{}", verify.stdout);
    let report = verify.json();
    assert_eq!(report["status"], "failed", "{}", verify.stdout);
    assert_eq!(
        findings_of(&report)
            .iter()
            .map(|(code, _)| code.clone())
            .collect::<Vec<_>>(),
        ["state_unreadable"],
        "{}",
        verify.stdout
    );

    let inspect = machine.inspect(a);
    assert_eq!(inspect.exit_code, Some(0), "{}", inspect.stdout);
    let report = inspect.json();
    assert!(report["installation"].is_null(), "{}", inspect.stdout);
    assert_eq!(
        findings_of(&report)
            .iter()
            .map(|(code, _)| code.clone())
            .collect::<Vec<_>>(),
        ["state_unreadable"]
    );

    // Give it back so the fixture can clean up after itself.
    acl::set_dacl(&state_db, "D:(A;;FA;;;WD)").unwrap();
    assert_eq!(machine.verify(a).json()["status"], "ok");
}

/// The state directory's access control list. Writing one needs the
/// administrator token machine scope needs anyway, so an unelevated run
/// says it skipped it and the lab proves the rest.
#[test]
fn the_machine_scope_state_directory_carries_its_own_access_control_list() {
    use tigersetup_engine::win::acl;

    let a = &fixture().a;
    let mut machine = Machine::in_scope("machine-acl", Scope::Machine);
    let run = machine.install(a);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);

    if !tigersetup_engine::elevation::is_elevated() {
        assert!(
            run.log_has("[state_directory_protection_skipped]"),
            "an unelevated run says it did not write the list: {}",
            run.log_text()
        );
        eprintln!(
            "skipped: this test process is not elevated, so the state directory's \
             access control list is asserted by the lab instead"
        );
        return;
    }

    assert!(run.log_has("[state_directory_protected]"));
    let wanted = acl::Dacl::parse(tigersetup_engine::scope::MACHINE_STATE_DIRECTORY_DACL).unwrap();
    let text = acl::read_dacl(&machine.state_dir()).unwrap();
    let actual = acl::Dacl::parse(&text).unwrap_or_else(|| panic!("{text} parses"));
    assert!(
        actual.is(&wanted),
        "the state directory carries {text}, not the list the engine writes"
    );
    // The engine writes the list on the directory, before anything is put in
    // it, and what it puts there inherits exactly that list: the same rights
    // for the same trustees, every entry inherited, none added.
    let grants = |dacl: &acl::Dacl| {
        let mut grants: Vec<(String, u32, String)> = dacl
            .entries
            .iter()
            .map(|ace| (ace.kind.clone(), ace.rights, ace.trustee.clone()))
            .collect();
        grants.sort();
        grants
    };
    for path in [machine.state_dir().join("state.db"), machine.uninstaller()] {
        let text = acl::read_dacl(&path).unwrap();
        let actual = acl::Dacl::parse(&text).unwrap_or_else(|| panic!("{text} parses"));
        assert!(
            !actual.protected
                && actual.entries.iter().all(acl::Ace::inherited)
                && grants(&actual) == grants(&wanted),
            "{} carries {text}, not the state directory's list by inheritance",
            path.display()
        );
    }

    // A list a third party loosened is repaired by the next mutating run.
    acl::set_dacl(&machine.state_dir(), "D:(A;OICI;FA;;;WD)").unwrap();
    let repair = machine.repair(a);
    assert_eq!(repair.exit_code, Some(0), "{}", repair.stdout);
    assert!(repair.log_has("[state_directory_protected]"));
    let text = acl::read_dacl(&machine.state_dir()).unwrap();
    assert!(acl::Dacl::parse(&text).unwrap().is(&wanted), "{text}");
}

/// A privileged uninstall trusts the database, so the database is confined:
/// a row naming a registry key in the other hive stops the run before it
/// deletes anything. A key of the scope's own hive outside `Software` is
/// where a product value at an explicit location lives, so such a row is
/// planned like any other — and a value that is not there is reported, not
/// invented.
#[test]
fn a_registry_row_outside_the_scope_is_refused_before_anything_is_removed() {
    let a = &fixture().a;
    let mut machine = Machine::in_scope("machine-confine-key", Scope::Machine);
    machine.install(a);

    let tampered = "HKCU\\Software\\IT Tiger\\Anything";
    execute(
        &machine.state_dir().join("state.db"),
        "UPDATE registry_value SET key = ?1 WHERE name = 'Version'",
        &[tampered],
    );

    let run = machine.uninstall(a);
    assert_eq!(run.exit_code, Some(1), "{}", run.stdout);
    let document = run.json();
    assert_eq!(document["code"], "path_outside_root", "{}", run.stdout);
    assert!(
        document["message"]
            .as_str()
            .unwrap()
            .contains("HKCU\\Software\\IT Tiger\\Anything"),
        "{}",
        run.stdout
    );
    // Nothing was removed: no transaction was ever opened.
    assert!(machine.install_root().join("bin").exists());
    assert!(machine.key_exists(&machine.registration_key()));
    assert!(machine.start_menu_link().exists());
    assert!(!run.log_has("[transaction_started]"));

    // The same row moved to the hive's own SYSTEM tree is within the scope:
    // the uninstall proceeds, finds no such value, and says so.
    execute(
        &machine.state_dir().join("state.db"),
        "UPDATE registry_value SET key = ?1 WHERE name = 'Version'",
        &["HKLM\\SYSTEM\\CurrentControlSet\\Services\\Anything"],
    );
    let run = machine.uninstall(a);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    let document = run.json();
    let codes: Vec<&str> = document["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["code"].as_str())
        .collect();
    assert!(codes.contains(&"registry_value_missing"), "{}", run.stdout);
    assert!(!machine.install_root().exists());
}

/// A shortcut row is treated differently from a registry row, on purpose. A
/// hive cannot move, so a key outside the scope means the database is wrong
/// and the run stops. A shortcut folder *can* move under a live installation
/// — OneDrive's Known Folder Move relocates the desktop — so a link outside
/// the scope is never deleted, is reported, and does not stop the rest of the
/// uninstall. Either way nothing outside the scope is touched, which is what
/// the confinement is for.
#[test]
fn a_shortcut_row_outside_the_scope_is_preserved_and_never_deleted() {
    let a = &fixture().a;
    let mut machine = Machine::in_scope("machine-confine-link", Scope::Machine);
    machine.install(a);

    let tampered = "C:\\Windows\\System32\\anything.lnk";
    let start_menu_link = machine.start_menu_link().display().to_string();
    execute(
        &machine.state_dir().join("state.db"),
        "UPDATE shortcut SET path = ?1 WHERE path = ?2",
        &[tampered, &start_menu_link],
    );

    let run = machine.uninstall(a);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    let codes: Vec<String> = findings_of(&run.json())
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    assert!(
        codes.contains(&"shortcut_outside_scope_preserved".to_string()),
        "{codes:?}"
    );
    assert!(
        !Path::new(tampered).exists(),
        "the engine must never have created or touched {tampered}"
    );
    assert!(!machine.install_root().exists(), "the rest was removed");
}

/// The elevated child hands its outcome document back through the file its
/// parent named, and prints the same thing. This exercises the hand-back
/// without a prompt; the prompt itself is the lab's.
#[test]
fn an_elevated_run_leaves_its_outcome_document_where_its_parent_reads_it() {
    let a = &fixture().a;
    let mut machine = Machine::in_scope("machine-handback", Scope::Machine);
    let result = machine.temp.join("elevated.json");

    let run = machine.run(
        &a.installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "machine",
            "--elevated-result",
            &result.display().to_string(),
        ],
    );
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    let handed_back: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&result).unwrap()).unwrap();
    assert_eq!(handed_back["outcome"], "installed");
    assert_eq!(handed_back["exit_code"], 0);
    assert_eq!(handed_back, run.json(), "the same document both ways");
    machine.assert_verified(a);

    // The hidden argument stays out of the help text.
    let help = machine.run(&a.installer, &["install", "--help"]);
    assert!(
        !help.stdout.contains("--elevated-result"),
        "the hand-back argument is not for people: {}",
        help.stdout
    );
}

/// A package that needs a newer Windows than this one stops with exit 8
/// before it looks at the machine at all.
#[test]
fn a_package_that_needs_a_newer_windows_exits_eight() {
    let dir = scratch("newer-windows");
    let installer = build_small_package(
        &dir,
        &format!(
            r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{VERSION_A}"
publisher = "IT Tiger"

[install]
scopes = ["user", "machine"]
minimum_build = 99999999

[[files]]
source = "payload/**"
"#
        ),
    );
    let mut machine = Machine::in_scope("newer-windows", Scope::Machine);
    let run = machine.run(&installer, &["install", "--quiet", "--scope", "machine"]);
    assert_eq!(run.exit_code, Some(8), "{}", run.stdout);
    let document = run.json();
    assert_eq!(document["code"], "platform_unsupported");
    assert_eq!(document["exit_code"], 8);
    assert!(
        !machine.state_dir().exists() && !machine.install_root().exists(),
        "nothing was created"
    );
}

/// Runs one statement against a state database, the way a tampering third
/// party would.
fn execute(state_db: &Path, sql: &str, parameters: &[&str]) {
    let connection = rusqlite::Connection::open(state_db).unwrap();
    let changed = connection
        .execute(sql, rusqlite::params_from_iter(parameters))
        .unwrap();
    assert!(changed > 0, "{sql} changed nothing");
}
