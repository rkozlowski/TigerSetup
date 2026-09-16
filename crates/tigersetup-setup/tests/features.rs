//! The 0.6 capability batch end to end, through the built `Setup.exe`:
//! choice options and component files, environment variables, the typed
//! Windows integrations, the new shortcut kinds, firewall rules and the
//! embedded prerequisite — each following its option across install,
//! reinstall, upgrade, a failed upgrade, repair and uninstall, each removed
//! conservatively, and each described by `inspect` and `verify`.
//!
//! Everything runs on the synthetic `TigerSetupTestApp` fixture, whose
//! manifest is the consolidated acceptance package's, on an isolated
//! machine with its own folders, hives and firewall store.

mod common;

use std::fs;

use common::*;
use serde_json::Value;
use tigersetup_engine::win::firewall::Rule;
use tigersetup_engine::win::registry::{Data, KeyPath};

fn codes(document: &Value) -> Vec<String> {
    findings_of(document)
        .into_iter()
        .map(|(code, _)| code)
        .collect()
}

fn assert_ok(run: &Run) -> Value {
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    run.json()
}

#[test]
fn components_and_the_path_mode_follow_their_options_across_reinstall_and_upgrade() {
    let fixture = fixture();
    let mut machine = Machine::new("components");

    // The defaults: the command on PATH, no extras, no tools.
    assert_ok(&machine.install(&fixture.a));
    let selected = machine.selected(&fixture.a);
    assert_eq!(selected.path_mode(), "command");
    assert!(!selected.on("extras"));
    assert!(!machine.install_root().join("extras").exists());
    assert!(!machine.install_root().join("tools").exists());
    machine.assert_verified(&fixture.a);

    // A reinstall that turns the component on and changes the PATH mode
    // installs the extras and the tools, and puts both directories on PATH.
    let changed =
        machine.install_with_options(&fixture.a, &[("extras", "on"), ("path-mode", "tools")]);
    let outcome = assert_ok(&changed);
    assert_eq!(outcome["transaction"]["kind"], "reinstall");
    let selected = machine.selected(&fixture.a);
    assert_eq!(selected.path_mode(), "tools");
    assert!(selected.on("extras"));
    assert!(
        machine
            .install_root()
            .join("extras")
            .join("notes.txt")
            .exists()
    );
    assert!(
        machine
            .install_root()
            .join("tools")
            .join("tsta-tool.exe")
            .exists()
    );
    assert_eq!(machine.path_entry_count(), 1);
    assert_eq!(machine.tools_path_entry_count(), 1);
    machine.assert_verified(&fixture.a);

    // An upgrade that names nothing remembers every choice, and carries the
    // components' new bytes.
    let upgrade = assert_ok(&machine.install(&fixture.b));
    assert_eq!(upgrade["transaction"]["kind"], "upgrade");
    let selected = machine.selected(&fixture.b);
    assert_eq!(selected.path_mode(), "tools");
    assert!(selected.on("extras"));
    machine.assert_verified(&fixture.b);

    // Deselecting the component removes its files and directories; a file
    // the user changed inside it is preserved, with its directory.
    let kept = machine.install_root().join("extras").join("notes.txt");
    fs::write(&kept, b"my notes").unwrap();
    let off = machine.install_with_options(&fixture.b, &[("extras", "off")]);
    let outcome = assert_ok(&off);
    assert!(
        codes(&outcome).contains(&"file_modified_preserved".to_string()),
        "{outcome}"
    );
    assert!(kept.exists(), "the edited file stays");
    assert!(
        !machine.install_root().join("extras").join("data").exists(),
        "the untouched part of the component is gone"
    );
    let selected = machine.selected(&fixture.b);
    assert!(!selected.on("extras"));
    // `verify` is about what is owned, and the preserved file is not.
    let verify = machine.verify(&fixture.b).json();
    assert_eq!(verify["status"], "ok", "{verify}");
    fs::remove_file(&kept).unwrap();
    fs::remove_dir(machine.install_root().join("extras")).unwrap();
    machine.assert_verified(&fixture.b);

    // Back to the command mode: the tools leave PATH and the disk.
    assert_ok(&machine.install_with_options(&fixture.b, &[("path-mode", "command")]));
    assert_eq!(machine.path_entry_count(), 1);
    assert_eq!(machine.tools_path_entry_count(), 0);
    assert!(!machine.install_root().join("tools").exists());
    machine.assert_verified(&fixture.b);

    // And to none: nothing of the product on PATH.
    assert_ok(&machine.install_with_options(&fixture.b, &[("path-mode", "none")]));
    assert_eq!(machine.path_entry_count(), 0);
    machine.assert_verified(&fixture.b);

    assert_ok(&machine.uninstall(&fixture.b));
    machine.assert_uninstalled(&fixture.b);
}

#[test]
fn option_values_are_validated_typed_and_remembered_by_unattended_runs() {
    let fixture = fixture();
    let mut machine = Machine::new("option-values");

    // `inspect` describes the declared options before anything is installed.
    let report = machine.inspect(&fixture.a).json();
    let options = report["package"]["options"].as_array().unwrap();
    let path_mode = options.iter().find(|o| o["name"] == "path-mode").unwrap();
    assert_eq!(path_mode["kind"], "choice");
    assert_eq!(path_mode["default"], "command");
    assert_eq!(
        path_mode["choices"],
        serde_json::json!(["none", "command", "tools"])
    );
    let startup = options.iter().find(|o| o["name"] == "startup").unwrap();
    assert_eq!(startup["kind"], "boolean");
    assert_eq!(startup["default"], true);

    // A value the option does not take is refused before anything happens.
    for (name, value) in [
        ("path-mode", "sideways"),
        ("startup", "maybe"),
        ("path-mode", "on"),
    ] {
        let run = machine.install_with_options(&fixture.a, &[(name, value)]);
        assert_eq!(run.exit_code, Some(2), "{name}={value}: {}", run.stdout);
        assert_eq!(run.json()["code"], "option_value_invalid");
    }
    let unknown = machine.install_with_options(&fixture.a, &[("nope", "on")]);
    assert_eq!(unknown.json()["code"], "option_unknown");
    assert!(
        !machine.state_dir().exists(),
        "a refused run creates nothing"
    );

    // Boolean spellings are all the same value; a choice is matched
    // case-insensitively and recorded canonically.
    assert_ok(&machine.install_with_options(
        &fixture.a,
        &[("path-mode", "NONE"), ("startup", "no"), ("send-to", "1")],
    ));
    let owned = &machine.inspect(&fixture.a).json()["owned"];
    assert_eq!(owned["options"]["path-mode"], "none");
    assert_eq!(owned["options"]["startup"], false);
    assert_eq!(owned["options"]["send-to"], true);
    assert_eq!(
        owned["options"]["firewall"], true,
        "defaults are recorded too"
    );
    assert!(machine.send_to_link().exists());
    assert!(!machine.startup_link().exists());

    // A run that names nothing keeps the recorded values, on the same
    // version and across an upgrade.
    assert_eq!(
        machine.install(&fixture.a).json()["code"],
        "already_installed"
    );
    assert_ok(&machine.install(&fixture.b));
    let owned = &machine.inspect(&fixture.b).json()["owned"];
    assert_eq!(owned["options"]["path-mode"], "none");
    assert_eq!(owned["options"]["startup"], false);
    assert_eq!(owned["options"]["send-to"], true);
    machine.assert_verified(&fixture.b);
}

#[test]
fn a_failed_upgrade_keeps_the_previous_options_and_resources() {
    let fixture = fixture();
    let mut machine = Machine::new("failed-upgrade");
    assert_ok(&machine.install(&fixture.a));
    let before = machine.selected(&fixture.a);

    // The upgrade changes several choices and fails before its commit.
    let failed = machine.run(
        &fixture.b.installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--option",
            "extras",
            "on",
            "--option",
            "firewall",
            "off",
            "--option",
            "path-mode",
            "none",
            "--fault",
            "before_commit:fail",
        ],
    );
    let outcome = failed.json();
    assert_eq!(failed.exit_code, Some(1), "{}", failed.stdout);
    assert_eq!(outcome["outcome"], "rolled_back", "{outcome}");

    // Nothing of the attempt shows: the options, the version and every
    // resource are 1.0.0's.
    assert_eq!(machine.selected(&fixture.a), before);
    assert_eq!(
        machine.inspect(&fixture.a).json()["installation"]["version"],
        VERSION_A
    );
    assert!(!machine.install_root().join("extras").exists());
    assert_eq!(machine.firewall_rules(FIREWALL_RULE).len(), 1);
    assert_eq!(machine.path_entry_count(), 1);
    machine.assert_verified(&fixture.a);

    // The same upgrade without the fault commits the new values.
    let upgraded = machine.install_with_options(
        &fixture.b,
        &[("extras", "on"), ("firewall", "off"), ("path-mode", "none")],
    );
    assert_ok(&upgraded);
    let selected = machine.selected(&fixture.b);
    assert!(selected.on("extras"));
    assert!(!selected.on("firewall"));
    assert_eq!(selected.path_mode(), "none");
    assert!(machine.firewall_rules(FIREWALL_RULE).is_empty());
    assert_eq!(machine.path_entry_count(), 0);
    machine.assert_verified(&fixture.b);
}

#[test]
fn an_environment_variable_is_created_restored_and_never_overwrites_a_users_change() {
    let fixture = fixture();
    let a = &fixture.a;

    // Created by TigerSetup: gone again at uninstall.
    let mut machine = Machine::new("environment-created");
    assert_ok(&machine.install(a));
    let root = machine.install_root().display().to_string();
    assert_eq!(
        machine.environment_variable(),
        Some(Data::ExpandString(root.clone()))
    );
    let owned = &machine.inspect(a).json()["owned"];
    assert_eq!(owned["environment_variables"][0]["pre_existed"], false);
    assert_ok(&machine.uninstall(a));
    assert_eq!(machine.environment_variable(), None);

    // A variable that was there before: replaced for the installation's
    // lifetime, and put back exactly when the option is turned off or the
    // product is removed.
    let mut machine = Machine::new("environment-restored");
    machine.seed_environment_variable(&Data::String("C:\\Elsewhere".into()));
    assert_ok(&machine.install(a));
    let root = machine.install_root().display().to_string();
    assert_eq!(
        machine.environment_variable(),
        Some(Data::ExpandString(root.clone()))
    );
    let owned = &machine.inspect(a).json()["owned"];
    assert_eq!(owned["environment_variables"][0]["pre_existed"], true);
    assert_ok(&machine.install_with_options(a, &[("environment", "off")]));
    assert_eq!(
        machine.environment_variable(),
        Some(Data::String("C:\\Elsewhere".into())),
        "deselecting restores the previous value"
    );
    assert_ok(&machine.install_with_options(a, &[("environment", "on")]));
    assert_eq!(
        machine.environment_variable(),
        Some(Data::ExpandString(root.clone()))
    );
    // An upgrade carries the memory of the previous value forward.
    assert_ok(&machine.install(&fixture.b));
    assert_ok(&machine.uninstall(&fixture.b));
    assert_eq!(
        machine.environment_variable(),
        Some(Data::String("C:\\Elsewhere".into())),
        "uninstall restores the pre-installation value"
    );

    // A value the user changed after the install is theirs: verify says
    // so, a run that would remove it leaves it and says so, and only a
    // repair — asked for — rewrites it.
    let mut machine = Machine::new("environment-modified");
    assert_ok(&machine.install(a));
    machine.seed_environment_variable(&Data::String("D:\\Mine".into()));
    let verify = machine.verify(a).json();
    assert_eq!(verify["status"], "failed");
    assert!(
        codes(&verify).contains(&"environment_variable_modified".to_string()),
        "{verify}"
    );
    let off = assert_ok(&machine.install_with_options(a, &[("environment", "off")]));
    assert!(
        codes(&off).contains(&"environment_variable_modified_preserved".to_string()),
        "{off}"
    );
    assert_eq!(
        machine.environment_variable(),
        Some(Data::String("D:\\Mine".into())),
        "the user's value is preserved"
    );
    assert!(
        machine.inspect(a).json()["owned"]["environment_variables"]
            .as_array()
            .is_none_or(|variables| variables.is_empty()),
        "a deselected variable is no longer owned"
    );
    // Turned on again over the user's value: theirs becomes the value to
    // restore.
    assert_ok(&machine.install_with_options(a, &[("environment", "on")]));
    assert_eq!(
        machine.environment_variable(),
        Some(Data::ExpandString(
            machine.install_root().display().to_string()
        ))
    );
    machine.seed_environment_variable(&Data::String("E:\\Again".into()));
    let repair = assert_ok(&machine.repair(a));
    assert_eq!(repair["transaction"]["kind"], "repair");
    assert_eq!(
        machine.environment_variable(),
        Some(Data::ExpandString(
            machine.install_root().display().to_string()
        )),
        "repair converges to the declared state"
    );
    assert_ok(&machine.uninstall(a));
    assert_eq!(
        machine.environment_variable(),
        Some(Data::String("D:\\Mine".into())),
        "uninstall restores what was there when TigerSetup took the variable over"
    );
}

#[test]
fn integrations_are_registered_deselected_and_removed_without_touching_a_strangers() {
    let fixture = fixture();
    let a = &fixture.a;
    let mut machine = Machine::new("integrations");

    // Another application already handles the scheme: its class key is
    // left exactly as it is, the handler ProgID and the capability are
    // still registered, and the run says what it left alone.
    let foreign_scheme = machine.classes_key(SCHEME);
    let roots = machine.roots();
    let foreign_command =
        KeyPath::parse(&format!("{foreign_scheme}\\shell\\open\\command")).unwrap();
    tigersetup_engine::win::registry::create_key(&roots, &foreign_command).unwrap();
    tigersetup_engine::win::registry::write_value(
        &roots,
        &foreign_command,
        "",
        &Data::String("\"C:\\Other\\other.exe\" \"%1\"".into()),
    )
    .unwrap();
    let install = assert_ok(&machine.install(a));
    assert!(
        codes(&install).contains(&"url_protocol_scheme_in_use_preserved".to_string()),
        "{install}"
    );
    assert_eq!(
        machine.read_value(&format!("{foreign_scheme}\\shell\\open\\command"), ""),
        Some(Data::String("\"C:\\Other\\other.exe\" \"%1\"".into())),
        "the stranger's handler is untouched"
    );
    assert_eq!(
        machine.read_value(
            &machine.classes_key(&format!("TigerSetupTestApp.{SCHEME}")),
            "URL Protocol"
        ),
        Some(Data::String(String::new()))
    );
    assert_eq!(
        machine.read_value(
            &format!("{}\\URLAssociations", machine.capabilities_key()),
            SCHEME
        ),
        Some(Data::String(format!("TigerSetupTestApp.{SCHEME}")))
    );
    let report = machine.inspect(a).json();
    let protocol = report["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "url_protocol")
        .unwrap()
        .clone();
    assert_eq!(protocol["enabled"], true);
    assert_eq!(protocol["values_present"], protocol["values_total"]);
    assert_eq!(
        protocol["values_total"], 5,
        "the handler class and the capability, not the stranger's scheme key: {protocol}"
    );

    // Deselecting the association removes its values and nothing else; the
    // capability registration stays for the protocol.
    assert_ok(&machine.install_with_options(a, &[("file-association", "off")]));
    assert!(!machine.key_exists(&machine.classes_key(PROG_ID)));
    assert_eq!(
        machine.read_value(
            &machine.classes_key(&format!("{EXTENSION}\\OpenWithProgids")),
            PROG_ID
        ),
        None
    );
    assert!(machine.key_exists(&machine.capabilities_key()));
    let report = machine.inspect(a).json();
    let association = report["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "file_association")
        .unwrap()
        .clone();
    assert_eq!(association["enabled"], false);
    assert_eq!(association["values_total"], 0);
    machine.assert_verified(a);

    // Deselecting the rest takes the capability registration with it.
    assert_ok(
        &machine.install_with_options(a, &[("url-protocol", "off"), ("context-menu", "off")]),
    );
    assert!(!machine.key_exists(&machine.capabilities_key()));
    assert_eq!(
        machine.read_value(
            &machine.hive_key("Software\\RegisteredApplications"),
            PRODUCT_NAME
        ),
        None
    );
    assert!(
        machine.key_exists(&machine.app_paths_key()),
        "App Paths is unconditional"
    );
    machine.assert_verified(a);

    // Back on; then a verb the user rewired is preserved at uninstall and
    // reported, while everything else goes.
    assert_ok(&machine.install_with_options(
        a,
        &[
            ("url-protocol", "on"),
            ("context-menu", "on"),
            ("file-association", "on"),
        ],
    ));
    machine.assert_verified(a);
    let verb_command = machine.classes_key(&format!("*\\shell\\{FILES_VERB}\\command"));
    tigersetup_engine::win::registry::write_value(
        &roots,
        &KeyPath::parse(&verb_command).unwrap(),
        "",
        &Data::String("\"C:\\Mine\\tool.exe\" \"%1\"".into()),
    )
    .unwrap();
    let verify = machine.verify(a).json();
    assert!(
        codes(&verify).contains(&"registry_value_modified".to_string()),
        "{verify}"
    );
    let uninstall = assert_ok(&machine.uninstall(a));
    assert!(
        codes(&uninstall).contains(&"registry_value_modified_preserved".to_string()),
        "{uninstall}"
    );
    assert_eq!(
        machine.read_value(&verb_command, ""),
        Some(Data::String("\"C:\\Mine\\tool.exe\" \"%1\"".into()))
    );
    assert!(
        !machine.key_exists(&machine.classes_key(PROG_ID)),
        "everything TigerSetup still recognised as its own is gone"
    );
    assert!(!machine.key_exists(&machine.app_paths_key()));
    assert!(
        machine.key_exists(&foreign_scheme),
        "the stranger's scheme survives"
    );
}

#[test]
fn shortcuts_of_every_location_follow_their_options_and_scope() {
    let fixture = fixture();
    let a = &fixture.a;
    let mut machine = Machine::new("shortcuts");
    assert_ok(&machine.install_with_options(a, &[("send-to", "on")]));
    assert!(machine.startup_link().exists());
    assert!(machine.send_to_link().exists());
    let send_to = machine.link(&machine.send_to_link()).unwrap();
    assert!(
        send_to.target.eq_ignore_ascii_case(
            &machine
                .install_root()
                .join("bin")
                .join("TigerSetupTestApp.exe")
                .display()
                .to_string()
        )
    );
    let documentation = machine.link(&machine.documentation_link()).unwrap();
    assert_eq!(documentation.target, DOCUMENTATION_URL);
    machine.assert_verified(a);

    // Off again: both go.
    assert_ok(&machine.install_with_options(a, &[("send-to", "off"), ("startup", "off")]));
    assert!(!machine.startup_link().exists());
    assert!(!machine.send_to_link().exists());
    machine.assert_verified(a);

    // A URL shortcut the user pointed elsewhere is preserved at uninstall.
    fs::write(
        machine.documentation_link(),
        "[InternetShortcut]\r\nURL=https://example.invalid/mine\r\n",
    )
    .unwrap();
    let verify = machine.verify(a).json();
    assert!(
        codes(&verify).contains(&"shortcut_modified".to_string()),
        "{verify}"
    );
    let uninstall = assert_ok(&machine.uninstall(a));
    assert!(
        codes(&uninstall).contains(&"shortcut_modified_preserved".to_string()),
        "{uninstall}"
    );
    assert!(machine.documentation_link().exists());
    assert!(!machine.start_menu_link().exists());

    // Machine scope has no shared Send To folder: the link is reported as
    // unavailable rather than invented, and the rest installs.
    let mut machine = Machine::in_scope(
        "shortcuts-machine",
        tigersetup_engine::format::identity::Scope::Machine,
    );
    let install = machine.install_with_options(a, &[("send-to", "on")]);
    let outcome = assert_ok(&install);
    assert!(
        codes(&outcome).contains(&"shortcut_location_unavailable".to_string()),
        "{outcome}"
    );
    assert!(
        machine.startup_link().exists(),
        "the shared Startup folder is real"
    );
    machine.assert_verified(a);
    assert_ok(&machine.uninstall(a));
    machine.assert_uninstalled(a);
}

#[test]
fn a_firewall_rule_is_created_kept_conservatively_repaired_and_removed() {
    let fixture = fixture();
    let a = &fixture.a;

    let mut machine = Machine::new("firewall");
    assert_ok(&machine.install(a));
    let rules = machine.firewall_rules(FIREWALL_RULE);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].direction, "in");
    assert_eq!(rules[0].action, "allow");
    let owned = &machine.inspect(a).json()["owned"];
    assert_eq!(owned["firewall_rules"][0], FIREWALL_RULE);

    // Deselected: removed. Selected again: back.
    assert_ok(&machine.install_with_options(a, &[("firewall", "off")]));
    assert!(machine.firewall_rules(FIREWALL_RULE).is_empty());
    machine.assert_verified(a);
    assert_ok(&machine.install_with_options(a, &[("firewall", "on")]));
    assert_eq!(machine.firewall_rules(FIREWALL_RULE).len(), 1);

    // The user disabled the rule: verify says modified, a reinstall keeps
    // their change and says so, a repair converges to the declared rule.
    let mut disabled = machine.firewall_rules(FIREWALL_RULE)[0].clone();
    disabled.enabled = false;
    machine.firewall().put(&disabled).unwrap();
    let verify = machine.verify(a).json();
    assert!(
        codes(&verify).contains(&"firewall_rule_modified".to_string()),
        "{verify}"
    );
    let reinstall = assert_ok(&machine.install_with_options(a, &[("startup", "off")]));
    assert!(
        codes(&reinstall).contains(&"firewall_rule_modified_preserved".to_string()),
        "{reinstall}"
    );
    assert!(
        !machine.firewall_rules(FIREWALL_RULE)[0].enabled,
        "the user's change stays"
    );
    let repair = assert_ok(&machine.repair(a));
    assert_eq!(repair["transaction"]["kind"], "repair");
    assert!(
        machine.firewall_rules(FIREWALL_RULE)[0].enabled,
        "repair rewrites the rule"
    );
    machine.assert_verified(a);

    // Disabled again and deselected: preserved and reported, not removed.
    machine.firewall().put(&disabled).unwrap();
    let off = assert_ok(&machine.install_with_options(a, &[("firewall", "off")]));
    assert!(
        codes(&off).contains(&"firewall_rule_modified_preserved".to_string()),
        "{off}"
    );
    assert_eq!(
        machine.firewall_rules(FIREWALL_RULE).len(),
        1,
        "the user's rule stays"
    );
    machine.firewall().remove(FIREWALL_RULE).unwrap();

    // A failure after the rule was created rolls it back.
    assert_ok(&machine.install_with_options(a, &[("firewall", "on")]));
    assert_ok(&machine.install_with_options(a, &[("firewall", "off")]));
    let failed = machine.run(
        &a.installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--option",
            "firewall",
            "on",
            "--fault",
            "before_commit:fail",
        ],
    );
    assert_eq!(failed.json()["outcome"], "rolled_back", "{}", failed.stdout);
    assert!(
        machine.firewall_rules(FIREWALL_RULE).is_empty(),
        "the rollback removed the rule it had created"
    );

    // A stranger's rule with the same name is neither claimed, rewritten
    // nor removed.
    let mut machine = Machine::new("firewall-stranger");
    let stranger = Rule {
        name: FIREWALL_RULE.into(),
        description: "somebody else's".into(),
        grouping: String::new(),
        program: "C:\\Other\\other.exe".into(),
        direction: "in".into(),
        action: "block".into(),
        protocol: "any".into(),
        local_ports: String::new(),
        enabled: true,
    };
    machine.firewall().put(&stranger).unwrap();
    let install = assert_ok(&machine.install(a));
    assert!(
        codes(&install).contains(&"firewall_rule_name_in_use_preserved".to_string()),
        "{install}"
    );
    assert_eq!(
        machine.firewall_rules(FIREWALL_RULE),
        vec![stranger.clone()]
    );
    assert!(
        machine.inspect(a).json()["owned"]["firewall_rules"]
            .as_array()
            .is_none_or(|rules| rules.is_empty()),
        "the stranger's rule is not owned"
    );
    assert_ok(&machine.uninstall(a));
    assert_eq!(machine.firewall_rules(FIREWALL_RULE), vec![stranger]);
}

#[test]
fn repair_restores_the_new_resources_it_owns() {
    let fixture = fixture();
    let a = &fixture.a;
    let mut machine = Machine::new("repair-new");
    assert_ok(&machine.install(a));

    fs::remove_file(machine.startup_link()).unwrap();
    fs::remove_file(machine.documentation_link()).unwrap();
    machine.firewall().remove(FIREWALL_RULE).unwrap();
    let roots = machine.roots();
    tigersetup_engine::win::registry::delete_value(
        &roots,
        &KeyPath::parse(&machine.environment_key()).unwrap(),
        ENVIRONMENT_VARIABLE,
    )
    .unwrap();
    tigersetup_engine::win::registry::delete_value(
        &roots,
        &KeyPath::parse(&machine.classes_key(&format!("{PROG_ID}\\shell\\open\\command"))).unwrap(),
        "",
    )
    .unwrap();

    let broken = machine.verify(a).json();
    assert_eq!(broken["status"], "failed");
    let found = codes(&broken);
    for expected in [
        "shortcut_missing",
        "firewall_rule_missing",
        "environment_variable_missing",
        "registry_value_missing",
    ] {
        assert!(found.contains(&expected.to_string()), "{found:?}");
    }
    let association = broken["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "file_association")
        .unwrap()
        .clone();
    assert_eq!(
        association["values_present"].as_u64().unwrap() + 1,
        association["values_total"].as_u64().unwrap(),
        "{association}"
    );

    let repair = assert_ok(&machine.repair(a));
    assert_eq!(repair["transaction"]["kind"], "repair");
    machine.assert_verified(a);
}

/// The manifest of a one-file package that embeds the prerequisite and
/// runs it with `arguments`.
fn prereq_manifest(arguments: &str, when: &str) -> String {
    format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{VERSION_A}"
publisher = "IT Tiger"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"

[[options]]
name = "extras"
label = {{ "en-US" = "Extras" }}
default = false

[[dependencies]]
id = "{PREREQ_ID}"
name = "TigerSetup test prerequisite"
detect = {{ kind = "directory-version", path = "%PROGRAMDATA%\\TigerSetupTestPrereq", pattern = "1\\..*" }}
acquire = {{ file = "dependencies/{PREREQ_FILE_NAME}" }}
install = {{ arguments = [{arguments}], success_codes = [0], reboot_codes = [3010] }}
{when}
"#
    )
}

fn build_prereq_package(name: &str, arguments: &str, when: &str) -> std::path::PathBuf {
    let dir = scratch(name);
    fs::create_dir_all(dir.join("dependencies")).unwrap();
    fs::copy(
        prereq_executable(),
        dir.join("dependencies").join(PREREQ_FILE_NAME),
    )
    .unwrap();
    build_small_package(&dir, &prereq_manifest(arguments, when))
}

#[test]
fn an_embedded_prerequisite_is_extracted_verified_run_and_then_only_detected() {
    let fixture = fixture();
    let a = &fixture.a;
    let mut machine = Machine::new("embedded");

    // The package says what it carries before anything is installed.
    let report = machine.inspect(a).json();
    let prereq = &report["dependencies"][0];
    assert_eq!(prereq["id"], PREREQ_ID);
    assert_eq!(prereq["status"], "absent");
    assert_eq!(prereq["source"], "embedded");
    assert_eq!(
        prereq["embedded"]["entry"],
        format!(".tigersetup/dependencies/{PREREQ_FILE_NAME}")
    );
    assert_eq!(prereq["embedded"]["sha256"], a.prereq_sha256());
    assert!(prereq["embedded"]["size"].as_u64().unwrap() > 0);

    // Absent: extracted, verified against the recorded hash, run, detected.
    let install = machine.install(a);
    let outcome = assert_ok(&install);
    assert!(
        install.log_has("[dependency_extracted]"),
        "{}",
        install.log_text()
    );
    assert!(
        install.log_has("[dependency_installed]"),
        "{}",
        install.log_text()
    );
    assert!(
        install.log_has("[dependency_verified]"),
        "{}",
        install.log_text()
    );
    assert!(!install.log_has("[dependency_downloading]"));
    let recorded = &outcome["dependencies"][0];
    assert_eq!(recorded["status"], "installed");
    assert_eq!(recorded["action"], "installed");
    assert_eq!(recorded["sha256"], a.prereq_sha256());
    assert_eq!(
        recorded["url"],
        format!("payload:.tigersetup/dependencies/{PREREQ_FILE_NAME}")
    );
    assert!(
        machine
            .prereq_dir()
            .join("1.0.0")
            .join("prereq.txt")
            .exists()
    );
    assert!(
        !machine.state_dir().join("deps").exists(),
        "the extracted installer is removed after the phase"
    );

    // Present: detected, never extracted again — and it outlives the
    // product, being a requirement rather than a resource.
    let again = machine.install_with_options(a, &[("startup", "off")]);
    assert_ok(&again);
    assert!(again.log_has("[dependency_detected]"));
    assert!(!again.log_has("[dependency_extracted]"));
    assert_eq!(
        machine.inspect(a).json()["dependencies"][0]["status"],
        "present"
    );
    assert_ok(&machine.uninstall(a));
    assert!(machine.prereq_dir().join("1.0.0").exists());
}

#[test]
fn an_embedded_installers_exit_codes_and_predicate_follow_the_dependency_model() {
    // A reboot code: the product installs and the run exits 3010.
    let reboot = build_prereq_package("embedded-reboot", "\"--install\", \"--exit\", \"3010\"", "");
    let mut machine = Machine::new("embedded-reboot");
    let run = machine.run(&reboot, &["install", "--quiet", "--scope", "user"]);
    let outcome = run.json();
    assert_eq!(
        run.exit_code,
        Some(3010),
        "{}\n{}",
        run.stdout,
        run.log_text()
    );
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(outcome["reboot_required"], true);
    assert_eq!(outcome["dependencies"][0]["action"], "reboot_required");
    assert_eq!(outcome["dependencies"][0]["exit_code"], 3010);

    // A failure that did not do the job stops the run before the product,
    // with the installer's exit code.
    let failing = build_prereq_package(
        "embedded-failing",
        "\"--no-install\", \"--exit\", \"7\"",
        "",
    );
    let mut machine = Machine::new("embedded-failing");
    let run = machine.run(&failing, &["install", "--quiet", "--scope", "user"]);
    let outcome = run.json();
    assert_eq!(run.exit_code, Some(3), "{}", run.stdout);
    assert_eq!(outcome["code"], "dependency_install_failed");
    assert_eq!(outcome["dependencies"][0]["exit_code"], 7);
    assert!(!machine.install_root().exists(), "no product change");
    assert!(machine.inspect(&fixture().a).json()["installation"].is_null());

    // A failing exit code from an installer that nonetheless did the job is
    // believed only after detection says otherwise.
    let lying = build_prereq_package("embedded-lying", "\"--install\", \"--exit\", \"9\"", "");
    let mut machine = Machine::new("embedded-lying");
    let run = machine.run(&lying, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert!(run.log_has("[dependency_installed_despite_exit_code]"));

    // A dependency gated by an option is not a requirement while the option
    // is off, and becomes one when it is turned on.
    let gated = build_prereq_package(
        "embedded-gated",
        "\"--install\"",
        "when = { option = \"extras\", equals = true }",
    );
    let mut machine = Machine::new("embedded-gated");
    let run = machine.run(&gated, &["install", "--quiet", "--scope", "user"]);
    let outcome = run.json();
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    assert_eq!(outcome["dependencies"][0]["status"], "not_required");
    assert!(run.log_has("[dependency_not_required]"));
    assert!(
        !machine.prereq_dir().exists(),
        "nothing was extracted or run"
    );
    let run = machine.run(
        &gated,
        &[
            "install", "--quiet", "--scope", "user", "--option", "extras", "on",
        ],
    );
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(run.json()["dependencies"][0]["status"], "installed");
    assert!(machine.prereq_dir().join("1.0.0").exists());
}

#[test]
fn a_corrupted_embedded_installer_is_refused_by_the_engine_and_by_verification() {
    let intact = build_prereq_package("embedded-corrupt", "\"--install\"", "");
    let corrupted = intact.with_file_name("corrupted-Setup.exe");
    let mut bytes = fs::read(&intact).unwrap();
    // The entry's local header names it in plain text; the compressed
    // bytes follow the header, and one of them is flipped.
    let needle = format!(".tigersetup/dependencies/{PREREQ_FILE_NAME}");
    // The name also appears in the metadata block and in the central
    // directory; the local header is the occurrence 30 bytes after a
    // local-file-header signature.
    let header = (30..bytes.len() - needle.len())
        .find(|&i| {
            bytes[i..i + needle.len()] == *needle.as_bytes()
                && bytes[i - 30..i - 26] == *b"PK\x03\x04"
        })
        .expect("the entry's local header");
    let victim = header + needle.len() + 64;
    bytes[victim] ^= 0xff;
    fs::write(&corrupted, &bytes).unwrap();

    // The engine refuses the bytes before running anything.
    let mut machine = Machine::new("embedded-corrupt");
    let run = machine.run(&corrupted, &["install", "--quiet", "--scope", "user"]);
    let outcome = run.json();
    assert_eq!(run.exit_code, Some(3), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(outcome["code"], "dependency_unacquirable", "{outcome}");
    assert!(
        matches!(
            outcome["reason"].as_str(),
            Some("hash_mismatch") | Some("download_failed")
        ),
        "{outcome}"
    );
    assert!(
        !machine.prereq_dir().exists(),
        "the corrupted installer never ran"
    );
    assert!(!machine.install_root().exists());

    // The builder's verification finds the same corruption without
    // running anything.
    let inspection = tigersetup_build::inspect::inspect(&corrupted).unwrap();
    assert!(!inspection.is_ok());
    let problems: Vec<&str> = inspection
        .verification
        .problems
        .iter()
        .map(|p| p.code)
        .collect();
    assert!(problems.contains(&"payload_hash_mismatch"), "{problems:?}");
    assert!(
        problems.iter().any(|p| matches!(
            *p,
            "dependency_entry_hash_mismatch" | "payload_entry_crc_mismatch"
        )),
        "{problems:?}"
    );
    let intact_inspection = tigersetup_build::inspect::inspect(&intact).unwrap();
    assert!(intact_inspection.is_ok());
    assert_eq!(
        intact_inspection.verification.entries_checked, 2,
        "the product file and the embedded installer"
    );
}

#[test]
fn old_manifest_spellings_and_defaults_keep_working() {
    // A package written for 0.5: boolean options, `option = "..."` gates,
    // no predicates anywhere.
    let dir = scratch("legacy-manifest");
    let installer = build_small_package(
        &dir,
        &format!(
            r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{VERSION_A}"
publisher = "IT Tiger"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"

[[options]]
name = "path"
kind = "path"
default = true

[[options]]
name = "desktop-shortcut"
kind = "desktop-shortcut"

[[shortcuts]]
location = "start-menu"
target = "bin/app.txt"

[[shortcuts]]
location = "desktop"
target = "bin/app.txt"
option = "desktop-shortcut"

[[path]]
entry = "bin"
option = "path"
"#
        ),
    );
    let mut machine = Machine::new("legacy-manifest");
    let run = machine.run(
        &installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--option",
            "desktop-shortcut",
            "on",
        ],
    );
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(machine.path_entry_count(), 1);
    assert!(machine.desktop_link().exists());
    let report = machine
        .run(&installer, &["inspect", "--scope", "user"])
        .json();
    assert_eq!(report["owned"]["options"]["path"], true);
    assert_eq!(report["owned"]["options"]["desktop-shortcut"], true);
    let run = machine.run(
        &installer,
        &[
            "install", "--quiet", "--scope", "user", "--option", "path", "off",
        ],
    );
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    assert_eq!(machine.path_entry_count(), 0);
    let run = machine.run(&installer, &["uninstall", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    assert!(!machine.desktop_link().exists());
}

#[test]
fn every_new_resource_recovers_from_a_crash_at_its_boundaries() {
    // The resources suite crashes at every boundary of every operation of
    // the fixture, which now includes the new kinds; this one crashes an
    // *upgrade that changes the options* at the boundaries of the new
    // kinds' operations specifically, so a restore, a removal and a create
    // of each are each interrupted once and converged forward.
    let fixture = fixture();
    let mut machine = Machine::new("crash-new-resources");
    assert_ok(&machine.install(&fixture.a));
    let plan = machine.run(
        &fixture.b.installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--option",
            "environment",
            "off",
            "--option",
            "firewall",
            "off",
            "--option",
            "send-to",
            "on",
            "--option",
            "file-association",
            "off",
            "--fault",
            "before_commit:fail",
        ],
    );
    assert_eq!(plan.json()["outcome"], "rolled_back", "{}", plan.stdout);
    let log = plan.log_text();
    let sequences: Vec<i64> = [
        "restore_environment_variable",
        "remove_firewall_rule",
        "create_shortcut",
        "remove_registry_value",
    ]
    .iter()
    .map(|kind| first_sequence_of_kind(&log, kind))
    .collect();
    machine.assert_verified(&fixture.a);

    for sequence in sequences {
        for point in ["after_prepare", "after_applying", "after_applied"] {
            let fault = format!("{point}@{sequence}:crash");
            let crashed = machine.run(
                &fixture.b.installer,
                &[
                    "install",
                    "--quiet",
                    "--scope",
                    "user",
                    "--option",
                    "environment",
                    "off",
                    "--option",
                    "firewall",
                    "off",
                    "--option",
                    "send-to",
                    "on",
                    "--option",
                    "file-association",
                    "off",
                    "--fault",
                    &fault,
                ],
            );
            assert_ne!(crashed.exit_code, Some(0), "{fault} must crash");
            // The same package runs again: it recovers forward, then finds
            // itself installed.
            let recovered = machine.run(
                &fixture.b.installer,
                &["install", "--quiet", "--scope", "user"],
            );
            let outcome = assert_ok(&recovered);
            assert_eq!(
                outcome["recovery"]["direction"], "forward",
                "{fault}: {outcome}"
            );
            let selected = machine.selected(&fixture.b);
            assert!(!selected.on("environment"), "{fault}");
            assert!(!selected.on("firewall"), "{fault}");
            assert!(selected.on("send-to"), "{fault}");
            assert!(!selected.on("file-association"), "{fault}");
            machine.assert_verified(&fixture.b);
            // Back to 1.0.0's defaults for the next boundary.
            assert_ok(&machine.uninstall(&fixture.b));
            assert_ok(&machine.install(&fixture.a));
        }
    }
    assert_ok(&machine.uninstall(&fixture.a));
    machine.assert_uninstalled(&fixture.a);
}

#[test]
fn a_state_database_written_by_0_5_is_read_as_it_is_and_migrated_by_the_first_run() {
    // The first mutating run of 0.6 over a 0.5 installation migrates the
    // schema in place; `inspect` and `verify` beforehand read it as it is.
    // The installation is built by this engine, then its database is
    // rewritten to the 0.5 shape — integer option values, no 0.6 tables.
    let fixture = fixture();
    let a = &fixture.a;
    let mut machine = Machine::new("schema-3");
    assert_ok(&machine.install(a));
    let state_db = machine.state_dir().join("state.db");
    {
        let connection = rusqlite::Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                r#"
                DROP TABLE environment_variable;
                DROP TABLE firewall_rule;
                ALTER TABLE operation DROP COLUMN restore_kind;
                ALTER TABLE operation DROP COLUMN restore_data;
                ALTER TABLE operation DROP COLUMN link_working_directory;
                ALTER TABLE operation DROP COLUMN link_app_user_model_id;
                CREATE TABLE installation_option_v3 (name TEXT PRIMARY KEY, value INTEGER NOT NULL);
                INSERT INTO installation_option_v3 (name, value)
                    SELECT name, CASE value WHEN 'true' THEN 1 ELSE 0 END FROM installation_option WHERE value IN ('true', 'false');
                DROP TABLE installation_option;
                ALTER TABLE installation_option_v3 RENAME TO installation_option;
                CREATE TABLE transaction_option_v3 (transaction_id TEXT NOT NULL REFERENCES "transaction"(id), name TEXT NOT NULL, value INTEGER NOT NULL, PRIMARY KEY (transaction_id, name));
                INSERT INTO transaction_option_v3 (transaction_id, name, value)
                    SELECT transaction_id, name, CASE value WHEN 'true' THEN 1 ELSE 0 END FROM transaction_option WHERE value IN ('true', 'false');
                DROP TABLE transaction_option;
                ALTER TABLE transaction_option_v3 RENAME TO transaction_option;
                PRAGMA user_version = 3;
                "#,
            )
            .unwrap();
    }
    let before = fs::read(&state_db).unwrap();
    let report = machine.inspect(a).json();
    assert_eq!(report["installation"]["version"], VERSION_A);
    assert_eq!(report["owned"]["options"]["startup"], true, "{report}");
    assert!(
        report["owned"]["options"].get("path-mode").is_none(),
        "the choice option a 0.5 database could not hold is absent, not invented"
    );
    let verify = machine.verify(a).json();
    assert_eq!(verify["counts"]["environment_variables_checked"], 0);
    assert_eq!(
        fs::read(&state_db).unwrap(),
        before,
        "inspect and verify leave a 0.5 database byte for byte as it was"
    );
    let version: i32 = rusqlite::Connection::open(&state_db)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 3);

    // The first mutating run migrates, keeps the recorded booleans and
    // fills the missing choice with its default.
    assert_ok(&machine.install_with_options(a, &[("send-to", "on")]));
    let version: i32 = rusqlite::Connection::open(&state_db)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 4);
    let owned = &machine.inspect(a).json()["owned"];
    assert_eq!(owned["options"]["startup"], true);
    assert_eq!(owned["options"]["send-to"], true);
    assert_eq!(owned["options"]["path-mode"], "command");
    machine.assert_verified(a);
}
