//! Every resource kind under the same transactional model as files:
//! registry keys and values, the scope's PATH, shortcuts and the Add/Remove
//! Programs registration. A package that declares all of them installs,
//! reconciles with changed options, repairs, upgrades and uninstalls through
//! the uninstaller copy in its state directory; a crash at any journal
//! boundary of any resource converges under recovery; and ownership stays
//! conservative for anything the user changed afterwards.

mod common;

use std::fs;
use std::sync::OnceLock;

use common::*;
use tigersetup_engine::win::registry::Data;

/// The registration values the synthetic package writes.
const REGISTRATION_VALUE_COUNT: u64 = 13;

/// The log of one clean install, learned once: the plan is deterministic, so
/// its sequence numbers hold on every machine.
fn install_log() -> &'static str {
    static LOG: OnceLock<String> = OnceLock::new();
    LOG.get_or_init(|| {
        let mut machine = Machine::new("resource-plan");
        let run = machine.install(&fixture().a);
        assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
        run.log_text()
    })
}

/// The log of one clean uninstall of that install.
fn uninstall_log() -> &'static str {
    static LOG: OnceLock<String> = OnceLock::new();
    LOG.get_or_init(|| {
        let mut machine = Machine::new("resource-plan-uninstall");
        machine.install(&fixture().a);
        let run = machine.uninstall(&fixture().a);
        assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
        run.log_text()
    })
}

/// The log of one clean upgrade over that install.
fn upgrade_log() -> &'static str {
    static LOG: OnceLock<String> = OnceLock::new();
    LOG.get_or_init(|| {
        let mut machine = Machine::new("resource-plan-upgrade");
        machine.install(&fixture().a);
        let run = machine.install(&fixture().b);
        assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
        run.log_text()
    })
}

/// The journal boundaries every resource operation passes through.
const BOUNDARIES: [&str; 3] = ["after_prepare", "after_applying", "after_applied"];

#[test]
fn every_declared_resource_is_installed_verified_and_removed() {
    let a = &fixture().a;
    let mut machine = Machine::new("resources");
    let install = machine.install(a);
    assert_eq!(
        install.exit_code,
        Some(0),
        "{}\n{}",
        install.stdout,
        install.log_text()
    );

    // The keys TigerSetup had to create are created, and the values are the
    // metadata's, expanded for this installation.
    assert!(machine.key_exists(VENDOR_KEY));
    assert!(machine.key_exists(PRODUCT_KEY));
    assert!(machine.key_exists(REGISTRATION_KEY));
    machine.assert_resources_present(a);

    // The PATH entry the `path` option enables, appended once, as an
    // expandable string.
    assert_eq!(machine.path_entry_count(), 1);
    let (text, exists) = machine.path_value();
    assert!(exists);
    assert!(
        text.ends_with(&machine.bin_path_entry()),
        "the entry is appended last: {text}"
    );
    assert!(
        matches!(
            machine.read_value(ENVIRONMENT_KEY, "Path"),
            Some(Data::ExpandString(_))
        ),
        "Path is written as an expandable string"
    );

    // The desktop shortcut is off by default.
    assert!(!machine.desktop_link().exists());

    let verify = machine.verify(a);
    let report = verify.json();
    assert_eq!(report["status"], "ok", "{}", verify.stdout);
    assert_eq!(report["counts"]["registry_values_checked"], 2);
    assert_eq!(report["counts"]["registry_values_ok"], 2);
    assert_eq!(
        report["counts"]["registration_values_checked"],
        REGISTRATION_VALUE_COUNT
    );
    assert_eq!(
        report["counts"]["registration_values_ok"],
        REGISTRATION_VALUE_COUNT
    );
    assert_eq!(report["counts"]["path_entries_checked"], 1);
    assert_eq!(report["counts"]["path_entries_ok"], 1);
    assert_eq!(report["counts"]["shortcuts_checked"], 1);
    assert_eq!(report["counts"]["shortcuts_ok"], 1);

    // `inspect` names what the lab keys on without reading the database.
    let inspect = machine.inspect(a).json();
    let owned = &inspect["owned"];
    assert_eq!(owned["options"]["path"], true);
    assert_eq!(owned["options"]["desktop-shortcut"], false);
    assert_eq!(owned["registration_key"], REGISTRATION_KEY);
    assert_eq!(owned["path_entries"][0]["entry"], machine.bin_path_entry());
    assert_eq!(owned["path_entries"][0]["hive_key"], ENVIRONMENT_KEY);
    assert_eq!(owned["path_entries"][0]["pre_existed"], false);
    assert_eq!(
        owned["shortcuts"][0],
        machine.start_menu_link().display().to_string()
    );
    let values: Vec<String> = owned["registry_values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        values.contains(&format!("{PRODUCT_KEY}\\InstallRoot")),
        "{values:?}"
    );
    assert_eq!(values.len(), 2 + REGISTRATION_VALUE_COUNT as usize);

    let uninstall = machine.uninstall(a);
    assert_eq!(
        uninstall.json()["outcome"],
        "uninstalled",
        "{}",
        uninstall.stdout
    );
    machine.assert_uninstalled(a);
    assert!(
        !machine.key_exists(VENDOR_KEY),
        "the key TigerSetup created for the product goes too"
    );
}

#[test]
fn an_explicit_option_reconciles_the_installed_version() {
    let a = &fixture().a;
    let mut machine = Machine::new("options");
    machine.install(a);
    assert_eq!(machine.path_entry_count(), 1);

    // The same version with no explicit option changes nothing.
    let again = machine.install(a);
    assert_eq!(again.json()["code"], "already_installed");

    // Turning the option off removes the entry the installation owns.
    let off = machine.install_with_options(a, &[("path", "off")]);
    let outcome = off.json();
    assert_eq!(off.exit_code, Some(0), "{}\n{}", off.stdout, off.log_text());
    assert_eq!(outcome["code"], "ok");
    assert_eq!(outcome["transaction"]["kind"], "reinstall");
    assert_eq!(machine.path_entry_count(), 0);
    assert_eq!(machine.inspect(a).json()["owned"]["options"]["path"], false);
    machine.assert_verified(a);

    // The recorded value survives a run that names no option.
    let remembered = machine.install(a);
    assert_eq!(remembered.json()["code"], "already_installed");
    assert_eq!(machine.path_entry_count(), 0);

    // Turning it on again leaves exactly one entry, and a further reinstall
    // that keeps it on still leaves exactly one.
    machine.install_with_options(a, &[("path", "on")]);
    assert_eq!(machine.path_entry_count(), 1);
    machine.install_with_options(a, &[("path", "on")]);
    assert_eq!(
        machine.path_entry_count(),
        1,
        "a reinstall that keeps the entry leaves exactly one"
    );
    machine.assert_verified(a);

    // A shortcut follows its option the same way.
    assert!(!machine.desktop_link().exists());
    machine.install_with_options(a, &[("desktop-shortcut", "on")]);
    assert!(machine.desktop_link().exists());
    assert_eq!(
        machine.link_target(&machine.desktop_link()),
        machine.link_target(&machine.start_menu_link())
    );
    machine.assert_verified(a);
    machine.install_with_options(a, &[("desktop-shortcut", "off")]);
    assert!(!machine.desktop_link().exists());
    machine.assert_verified(a);

    machine.uninstall(a);
    machine.assert_absent(a);
}

#[test]
fn an_option_choice_survives_an_upgrade() {
    let fixture = fixture();
    let mut machine = Machine::new("options-upgrade");
    machine.install(&fixture.a);
    machine.install_with_options(&fixture.a, &[("path", "off"), ("desktop-shortcut", "on")]);
    assert_eq!(machine.path_entry_count(), 0);

    let upgrade = machine.install(&fixture.b);
    assert_eq!(
        upgrade.exit_code,
        Some(0),
        "{}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    assert_eq!(
        machine.path_entry_count(),
        0,
        "the upgrade keeps the user's choice"
    );
    assert!(machine.desktop_link().exists());
    let owned = &machine.inspect(&fixture.b).json()["owned"];
    assert_eq!(owned["options"]["path"], false);
    assert_eq!(owned["options"]["desktop-shortcut"], true);
    machine.assert_verified(&fixture.b);
}

#[test]
fn an_upgrade_replaces_the_value_that_changed_and_keeps_the_rest() {
    let fixture = fixture();
    let mut machine = Machine::new("upgrade-resources");
    machine.install(&fixture.a);
    assert_eq!(
        machine.read_value(PRODUCT_KEY, "Version"),
        Some(Data::String(VERSION_A.into()))
    );

    let upgrade = machine.install(&fixture.b);
    assert_eq!(
        upgrade.exit_code,
        Some(0),
        "{}\n{}",
        upgrade.stdout,
        upgrade.log_text()
    );
    let log = upgrade.log_text();
    assert!(
        log.lines()
            .any(|l| l.contains("[operation_applied]") && l.contains("kind=set_registry_value")),
        "the changed value is written again\n{log}"
    );
    assert!(
        !log.contains("kind=create_registry_key"),
        "the keys are kept, not created again\n{log}"
    );
    assert!(
        !log.contains("kind=add_path_entry"),
        "the PATH entry is kept, not appended again\n{log}"
    );
    assert_eq!(
        machine.read_value(PRODUCT_KEY, "Version"),
        Some(Data::String(VERSION_B.into()))
    );
    assert_eq!(
        machine.read_value(REGISTRATION_KEY, "DisplayVersion"),
        Some(Data::String(VERSION_B.into())),
        "the registration names the new version"
    );
    assert_eq!(machine.path_entry_count(), 1);
    machine.assert_verified(&fixture.b);

    machine.uninstall(&fixture.b);
    machine.assert_absent(&fixture.b);
}

#[test]
fn repair_restores_every_resource_it_owns() {
    let a = &fixture().a;
    let mut machine = Machine::new("repair");
    machine.install(a);

    let removed = machine.install_root().join("locale").join("en-US.json");
    let edited = machine
        .install_root()
        .join("doc")
        .join("manual")
        .join("ch-04.md");
    fs::remove_file(&removed).unwrap();
    fs::write(&edited, b"someone rewrote this").unwrap();
    fs::remove_file(machine.start_menu_link()).unwrap();
    tigersetup_engine::win::registry::delete_value(
        &machine.roots(),
        &tigersetup_engine::win::registry::KeyPath::parse(PRODUCT_KEY).unwrap(),
        "InstallRoot",
    )
    .unwrap();
    tigersetup_engine::resource::path::remove(
        &machine.roots(),
        &tigersetup_engine::win::registry::KeyPath::parse(ENVIRONMENT_KEY).unwrap(),
        &machine.bin_path_entry(),
        false,
    )
    .unwrap();

    let broken = machine.verify(a).json();
    assert_eq!(broken["status"], "failed");
    let codes: Vec<String> = findings_of(&broken)
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    for expected in [
        "file_missing",
        "file_modified",
        "shortcut_missing",
        "registry_value_missing",
        "path_entry_missing",
    ] {
        assert!(codes.contains(&expected.to_string()), "{codes:?}");
    }

    let repair = machine.repair(a);
    let outcome = repair.json();
    assert_eq!(
        repair.exit_code,
        Some(0),
        "{}\n{}",
        repair.stdout,
        repair.log_text()
    );
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(outcome["transaction"]["kind"], "repair");
    let repaired: Vec<String> = findings_of(&outcome)
        .into_iter()
        .filter(|(code, _)| code == "file_repaired")
        .map(|(_, path)| path)
        .collect();
    assert!(
        repaired.contains(&removed.display().to_string())
            && repaired.contains(&edited.display().to_string()),
        "{repaired:?}"
    );
    assert_eq!(
        machine.read_value(PRODUCT_KEY, "InstallRoot"),
        Some(Data::ExpandString(
            machine.install_root().display().to_string()
        ))
    );
    assert_eq!(machine.path_entry_count(), 1);
    assert!(machine.start_menu_link().exists());
    machine.assert_verified(a);
}

/// Deleting a value leaves its key behind; deleting the key takes the values
/// with it, and the run that follows has nowhere to write them. The keys the
/// installation owns are therefore re-created, not assumed to be there — and
/// what a repair re-creates it still owns, so a later uninstall removes it.
#[test]
fn repair_re_creates_owned_registry_keys_that_were_deleted() {
    let a = &fixture().a;
    let mut machine = Machine::new("repair-keys");
    machine.install(a);
    let removed = machine.install_root().join("locale").join("en-US.json");
    fs::remove_file(&removed).unwrap();
    machine.delete_key_tree(VENDOR_KEY);
    machine.delete_key_tree(REGISTRATION_KEY);

    let broken = machine.verify(a).json();
    assert_eq!(broken["status"], "failed");
    let codes: Vec<String> = findings_of(&broken)
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    for expected in ["file_missing", "registry_value_missing"] {
        assert!(codes.contains(&expected.to_string()), "{codes:?}");
    }

    let repair = machine.repair(a);
    assert_eq!(
        repair.exit_code,
        Some(0),
        "{}\n{}",
        repair.stdout,
        repair.log_text()
    );
    assert_eq!(repair.json()["outcome"], "installed");
    assert!(machine.key_exists(VENDOR_KEY) && machine.key_exists(PRODUCT_KEY));
    assert!(machine.key_exists(REGISTRATION_KEY));
    assert_eq!(
        machine.read_value(PRODUCT_KEY, "Version"),
        Some(Data::String(VERSION_A.into()))
    );
    assert_eq!(
        machine.read_value(REGISTRATION_KEY, "DisplayName"),
        Some(Data::String(PRODUCT_NAME.into()))
    );
    assert!(removed.exists());
    machine.assert_verified(a);

    machine.uninstall(a);
    machine.assert_uninstalled(a);
    assert!(
        !machine.key_exists(VENDOR_KEY),
        "the re-created parent key is still owned and goes away with the product"
    );
}

/// A shortcut folder can move under a live installation: OneDrive's Known
/// Folder Move relocates the desktop, and policy can redirect the Start Menu.
/// The link recorded before the move is then outside the folders the scope
/// resolves afterwards. TigerSetup must leave that link alone and say so, and
/// must still remove everything else — an installation that cannot be
/// uninstalled is not an acceptable outcome.
#[test]
fn a_shortcut_whose_folder_moved_is_preserved_and_the_rest_is_removed() {
    let a = &fixture().a;
    let mut machine = Machine::new("moved-folder");
    machine.install_with_options(a, &[("desktop-shortcut", "on")]);
    let stranded = machine.desktop_link();
    assert!(stranded.exists());

    // The desktop moves, exactly as Known Folder Move does it: the folder the
    // scope resolves from now on is a different one, and the link stays where
    // it was written.
    let moved = machine.desktop.parent().unwrap().join("DesktopMoved");
    fs::create_dir_all(&moved).unwrap();
    machine.desktop = moved;

    let run = machine.uninstall(a);
    assert_eq!(
        run.exit_code,
        Some(0),
        "the uninstall must not be refused\n{}\n{}",
        run.stdout,
        run.log_text()
    );
    let codes: Vec<String> = findings_of(&run.json())
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    assert!(
        codes.contains(&"shortcut_outside_scope_preserved".to_string()),
        "{codes:?}"
    );
    assert!(stranded.exists(), "the link outside the scope is untouched");
    assert!(!machine.install_root().exists());
    assert!(!machine.key_exists(REGISTRATION_KEY));
    assert_eq!(machine.path_entry_count(), 0);
    assert!(!machine.start_menu_link().exists());
}

#[test]
fn repair_without_an_installation_fails_and_changes_nothing() {
    let a = &fixture().a;
    let mut machine = Machine::new("repair-absent");
    let run = machine.repair(a);
    assert_eq!(run.exit_code, Some(1), "{}", run.stdout);
    assert_eq!(run.json()["code"], "not_installed");
    assert!(!machine.install_root().exists());
}

#[test]
fn the_uninstaller_copy_removes_the_product_and_its_state_directory() {
    let a = &fixture().a;
    let mut machine = Machine::new("uninstaller-copy");
    let install = machine.install(a);
    assert!(
        install.log_has("[uninstaller_written]"),
        "{}",
        install.log_text()
    );
    let uninstaller = machine.uninstaller();
    assert!(uninstaller.exists());

    // The copy is a complete installer file for the same package, marked as
    // the uninstaller of this scope and carrying no payload.
    let copy = tigersetup_engine::Package::open(&uninstaller).unwrap();
    assert!(copy.is_uninstaller());
    assert_eq!(copy.id(), PRODUCT_ID);
    assert_eq!(copy.version(), VERSION_A);
    assert_eq!(copy.installer().payload_archive().unwrap().len(), 0);
    assert_ne!(
        copy.metadata_sha256(),
        tigersetup_engine::Package::open(&a.installer)
            .unwrap()
            .metadata_sha256(),
        "the copy is a different package to the recovery direction rule"
    );
    drop(copy);

    // It describes and verifies the installation like any installer.
    let verify = machine.run(&uninstaller, &["verify"]);
    assert_eq!(verify.json()["status"], "ok", "{}", verify.stdout);
    assert_eq!(verify.exit_code, Some(0));
    let inspect = machine.run(&uninstaller, &["inspect"]).json();
    assert_eq!(inspect["installation"]["version"], VERSION_A);

    // Installing or repairing from it is refused, naming the installer.
    for command in ["install", "repair"] {
        let run = machine.run(&uninstaller, &[command, "--quiet"]);
        assert_eq!(run.exit_code, Some(2), "{command}: {}", run.stdout);
        assert_eq!(run.json()["code"], "payload_unavailable", "{command}");
        assert!(
            run.json()["message"]
                .as_str()
                .unwrap()
                .contains(PRODUCT_NAME),
            "{command}: the message names the package"
        );
    }
    machine.assert_verified(a);

    // And it removes the product, its own directory and itself.
    let run = machine.uninstall_through_the_copy();
    let outcome = run.json();
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(outcome["outcome"], "uninstalled");
    assert!(
        run.log_has("[state_directory_removed]"),
        "{}",
        run.log_text()
    );
    assert!(
        !machine.state_dir().exists(),
        "the state directory {} must be gone",
        machine.state_dir().display()
    );
    assert!(!machine.install_root().exists());
    assert!(!machine.key_exists(REGISTRATION_KEY));
    assert_eq!(machine.path_entry_count(), 0);
    assert!(!machine.start_menu_link().exists());

    let verify = machine.run(&a.installer, &["verify", "--scope", "user"]);
    assert_eq!(
        verify.json()["status"],
        "not_installed",
        "{}",
        verify.stdout
    );
    assert_eq!(verify.exit_code, Some(1));
}

#[test]
fn the_uninstaller_copy_rolls_an_open_transaction_back() {
    let fixture = fixture();
    let mut machine = Machine::new("uninstaller-copy-recovery");
    machine.install(&fixture.a);
    let crashed = machine.install_with_faults(&fixture.b, &["before_commit:crash"]);
    assert!(!crashed.success);

    // The copy carries 1.0.0's metadata, so the open 1.1.0 upgrade is not
    // its own transaction: it can only roll it back.
    let uninstaller = machine.uninstaller();
    let inspect = machine.run(&uninstaller, &["inspect"]).json();
    assert_eq!(inspect["transaction"]["recovery_direction"], "rollback");

    let run = machine.uninstall_through_the_copy();
    let outcome = run.json();
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(outcome["outcome"], "uninstalled");
    assert_eq!(outcome["recovery"]["direction"], "rollback");
    assert_eq!(outcome["transaction"]["from_version"], VERSION_A);
    assert!(!machine.state_dir().exists());
    assert!(!machine.install_root().exists());
    assert!(!machine.key_exists(REGISTRATION_KEY));
}

#[test]
fn a_resource_the_user_changed_is_preserved_and_reported() {
    let a = &fixture().a;
    let mut machine = Machine::new("preserved");
    machine.install(a);

    // A registry value someone edited after the install.
    tigersetup_engine::win::registry::write_value(
        &machine.roots(),
        &tigersetup_engine::win::registry::KeyPath::parse(PRODUCT_KEY).unwrap(),
        "Version",
        &Data::String("edited by hand".into()),
    )
    .unwrap();
    // A shortcut repointed at something outside the install root.
    tigersetup_engine::win::shortcut::write(
        &machine.start_menu_link(),
        &tigersetup_engine::win::shortcut::Link {
            target: "C:\\Windows\\notepad.exe".into(),
            ..Default::default()
        },
    )
    .unwrap();
    // A value someone added to the product key.
    tigersetup_engine::win::registry::write_value(
        &machine.roots(),
        &tigersetup_engine::win::registry::KeyPath::parse(PRODUCT_KEY).unwrap(),
        "UserSetting",
        &Data::Dword(7),
    )
    .unwrap();

    let broken = machine.verify(a).json();
    let codes: Vec<String> = findings_of(&broken)
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    assert!(
        codes.contains(&"registry_value_modified".to_string()),
        "{codes:?}"
    );
    assert!(
        codes.contains(&"shortcut_modified".to_string()),
        "{codes:?}"
    );

    let uninstall = machine.uninstall(a);
    let outcome = uninstall.json();
    assert_eq!(
        uninstall.exit_code,
        Some(0),
        "{}\n{}",
        uninstall.stdout,
        uninstall.log_text()
    );
    let findings = findings_of(&outcome);
    let codes: Vec<&str> = findings.iter().map(|(code, _)| code.as_str()).collect();
    assert!(
        codes.contains(&"registry_value_modified_preserved"),
        "{findings:?}"
    );
    assert!(
        codes.contains(&"shortcut_modified_preserved"),
        "{findings:?}"
    );
    assert!(
        codes.contains(&"registry_key_not_empty_preserved"),
        "{findings:?}"
    );
    assert_eq!(
        machine.read_value(PRODUCT_KEY, "Version"),
        Some(Data::String("edited by hand".into())),
        "the edited value survives"
    );
    assert_eq!(
        machine.read_value(PRODUCT_KEY, "UserSetting"),
        Some(Data::Dword(7)),
        "the foreign value survives, so its key stays"
    );
    assert!(machine.key_exists(PRODUCT_KEY));
    assert!(
        machine.start_menu_link().exists(),
        "a link that no longer points into the install root is never removed"
    );
    assert!(
        !machine.key_exists(REGISTRATION_KEY),
        "everything TigerSetup still owned is gone"
    );
    assert_eq!(machine.path_entry_count(), 0);
    assert!(!machine.install_root().exists());
}

/// The two PATH vectors a conservative PATH implementation must survive,
/// seeded before the install: a pre-existing lookalike of the entry TigerSetup
/// would add, and an entry followed by an empty segment.
#[test]
fn a_pre_existing_path_lookalike_is_neither_duplicated_claimed_nor_removed() {
    let a = &fixture().a;
    let mut machine = Machine::new("path-vectors");
    let lookalike = format!("{}\\", machine.bin_path_entry());
    let seeded = format!("C:\\Windows;{lookalike};C:\\Tools;;");
    machine.seed_path(&seeded);

    let install = machine.install(a);
    assert_eq!(
        install.exit_code,
        Some(0),
        "{}\n{}",
        install.stdout,
        install.log_text()
    );
    assert_eq!(
        machine.path_value().0,
        seeded,
        "an equivalent entry is neither duplicated nor rewritten"
    );
    assert_eq!(machine.path_entry_count(), 1);
    let owned = &machine.inspect(a).json()["owned"];
    assert_eq!(
        owned["path_entries"][0]["pre_existed"], true,
        "the pre-existing entry is not claimed"
    );
    machine.assert_verified(a);

    // A reinstall leaves it exactly as it was.
    machine.install_with_options(a, &[("path", "on")]);
    assert_eq!(machine.path_value().0, seeded);
    assert_eq!(machine.path_entry_count(), 1);

    let uninstall = machine.uninstall(a);
    assert_eq!(
        uninstall.json()["outcome"],
        "uninstalled",
        "{}",
        uninstall.stdout
    );
    assert_eq!(
        machine.path_value().0,
        seeded,
        "what TigerSetup never claimed it never removes, and the trailing empty segment survives"
    );
    assert!(!machine.install_root().exists());
    assert!(!machine.key_exists(REGISTRATION_KEY));
    assert!(!machine.start_menu_link().exists());
    let verify = machine.verify(a);
    assert_eq!(
        verify.json()["status"],
        "not_installed",
        "{}",
        verify.stdout
    );
}

#[test]
fn the_path_entry_is_removed_without_disturbing_its_neighbours() {
    let a = &fixture().a;
    let mut machine = Machine::new("path-neighbours");
    let seeded = "C:\\Windows;C:\\Tools;;";
    machine.seed_path(seeded);
    machine.install(a);
    assert_eq!(
        machine.path_value().0,
        format!("C:\\Windows;C:\\Tools;;{};", machine.bin_path_entry()),
        "the entry is appended last and the trailing empty segment stays trailing"
    );
    machine.uninstall(a);
    assert_eq!(
        machine.path_value().0,
        seeded,
        "removal restores the original text exactly"
    );
}

/// One journal boundary of one operation of each new kind, crashed during an
/// install: recovery finishes the install and `verify` passes.
#[test]
fn a_crash_at_every_boundary_of_every_resource_converges_during_an_install() {
    let a = &fixture().a;
    let kinds = [
        "create_registry_key",
        "set_registry_value",
        "add_path_entry",
        "create_shortcut",
    ];
    for kind in kinds {
        let sequence = first_sequence_of_kind(install_log(), kind);
        for point in BOUNDARIES {
            let label = format!("{point}@{sequence} ({kind})");
            let mut machine = Machine::new("resource-crash");
            let crashed = machine.install_with_faults(a, &[&format!("{point}@{sequence}:crash")]);
            assert!(!crashed.success, "{label}: the run must abort");
            let rerun = machine.install(a);
            assert_eq!(
                rerun.exit_code,
                Some(0),
                "{label}: {}\n{}",
                rerun.stdout,
                rerun.log_text()
            );
            assert_eq!(rerun.json()["recovery"]["direction"], "forward", "{label}");
            machine.assert_verified(a);
            assert_eq!(machine.path_entry_count(), 1, "{label}");
        }
    }
}

/// The same boundaries during an uninstall: recovery finishes the removal.
#[test]
fn a_crash_at_every_boundary_of_every_resource_converges_during_an_uninstall() {
    let a = &fixture().a;
    let kinds = [
        "remove_registry_key",
        "remove_registry_value",
        "remove_path_entry",
        "remove_shortcut",
    ];
    for kind in kinds {
        let sequence = first_sequence_of_kind(uninstall_log(), kind);
        for point in BOUNDARIES {
            let label = format!("{point}@{sequence} ({kind})");
            let mut machine = Machine::new("resource-crash-uninstall");
            machine.install(a);
            let crashed = machine.run(
                &a.installer,
                &[
                    "uninstall",
                    "--quiet",
                    "--scope",
                    "user",
                    "--fault",
                    &format!("{point}@{sequence}:crash"),
                ],
            );
            assert!(!crashed.success, "{label}: the run must abort");

            let resume = machine.uninstall(a);
            assert_eq!(
                resume.exit_code,
                Some(0),
                "{label}: {}\n{}",
                resume.stdout,
                resume.log_text()
            );
            assert_eq!(resume.json()["recovery"]["direction"], "forward", "{label}");
            machine.assert_absent(a);

            // Running it once more changes nothing.
            let again = machine.uninstall(a);
            assert_eq!(again.json()["outcome"], "not_installed", "{label}");
            assert!(again.json()["recovery"].is_null(), "{label}");
            machine.assert_absent(a);
        }
    }
}

/// An upgrade interrupted at the boundaries of the value it replaces ends in
/// exactly one of the two versions, verified.
#[test]
fn a_crash_at_every_boundary_of_a_replaced_value_converges_during_an_upgrade() {
    let fixture = fixture();
    let sequence = first_sequence_of_kind(upgrade_log(), "set_registry_value");
    for point in BOUNDARIES {
        let label = format!("{point}@{sequence}");
        let mut machine = Machine::new("resource-crash-upgrade");
        machine.install(&fixture.a);
        let crashed =
            machine.install_with_faults(&fixture.b, &[&format!("{point}@{sequence}:crash")]);
        assert!(!crashed.success, "{label}: the run must abort");

        let rerun = machine.install(&fixture.b);
        assert_eq!(
            rerun.exit_code,
            Some(0),
            "{label}: {}\n{}",
            rerun.stdout,
            rerun.log_text()
        );
        assert_eq!(rerun.json()["recovery"]["direction"], "forward", "{label}");
        machine.assert_verified(&fixture.b);
        assert_eq!(
            machine.read_value(PRODUCT_KEY, "Version"),
            Some(Data::String(VERSION_B.into())),
            "{label}"
        );
    }
}

/// A failure after a resource was applied rolls every resource back, twice
/// over if the rollback itself is interrupted.
#[test]
fn a_failure_after_a_resource_rolls_every_resource_back() {
    let a = &fixture().a;
    let seeded = "C:\\Windows;";
    for kind in ["set_registry_value", "add_path_entry", "create_shortcut"] {
        let sequence = first_sequence_of_kind(install_log(), kind);
        let mut machine = Machine::new("resource-rollback");
        machine.seed_path(seeded);
        let run = machine.install_with_faults(a, &[&format!("after_applied@{sequence}:fail")]);
        let outcome = run.json();
        assert_eq!(run.exit_code, Some(1), "{kind}: {}", run.stdout);
        assert_eq!(outcome["outcome"], "rolled_back", "{kind}");
        assert_eq!(outcome["transaction"]["state"], "rolled_back", "{kind}");
        machine.assert_absent(a);
        assert_eq!(
            machine.path_value().0,
            seeded,
            "{kind}: the PATH is exactly as it was"
        );
        assert!(!machine.key_exists(VENDOR_KEY), "{kind}");

        // A second run installs cleanly on top of the rolled-back state.
        let rerun = machine.install(a);
        assert_eq!(
            rerun.exit_code,
            Some(0),
            "{kind}: {}\n{}",
            rerun.stdout,
            rerun.log_text()
        );
        machine.assert_verified(a);
    }
}
