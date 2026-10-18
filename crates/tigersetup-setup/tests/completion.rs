//! PowerShell completion through its whole lifecycle on an isolated machine:
//! a command's completion script installed as a file and activated by a
//! marked block in the CurrentUserAllHosts profile of Windows PowerShell 5.1
//! and of PowerShell 7 — exactly while the command is on PATH — kept by an
//! upgrade, restored by a repair, taken away when PATH is turned off, rolled
//! back with a run that fails, recovered after a crash, and removed by the
//! uninstall, every time leaving everything else in the profiles byte for
//! byte as it was. A profile that cannot be written is skipped without
//! failing the run, and a script another account could change is never
//! loaded.
//!
//! These run unelevated against the fixture's own Documents folder
//! (`TIGERSETUP_TEST_FOLDER_DOCUMENTS`), so the profiles are this test's and
//! never the developer's. The lab rows of `lab\Invoke-CompletionRows.ps1`
//! prove the real profiles, and completion in real Windows PowerShell 5.1
//! and PowerShell 7 sessions, on a clean Windows 11.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use serde_json::Value;
use tigersetup_engine::format::identity::Scope;

const MARKER_START: &str = "# >>> TigerSetup completion: tool (IT-Tiger.TigerSetupTestApp) >>>";

/// The user's own Windows PowerShell profile before any install: LF, no
/// byte-order mark, an ANSI (1250) Polish letter — what an older editor
/// leaves.
const USER_PROFILE: &[u8] = b"Set-Alias ll Get-ChildItem\n# \xbf\n";

fn manifest(version: &str) -> String {
    format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{version}"
publisher = "IT Tiger"

[install]
scopes = ["user", "machine"]

[[files]]
source = "payload/**"

[[options]]
name = "path"
kind = "path"
default = true

[[path]]
entry = "bin"
option = "path"

[[completion]]
executable = "bin/tool.exe"
source = "script"
script = "tool-completion.ps1"
"#
    )
}

/// A package at `version` whose completion script says which version it is.
fn package(dir: &Path, version: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("tool-completion.ps1"),
        format!("$global:TigerSetupToolCompletion = '{version}'\r\n"),
    )
    .unwrap();
    build_package(
        dir,
        &manifest(version),
        &[
            ("bin/tool.exe", b"tool"),
            ("bin/readme.txt", version.as_bytes()),
        ],
    )
}

struct Profiles {
    windows: PathBuf,
    core: PathBuf,
}

fn profiles(machine: &Machine) -> Profiles {
    Profiles {
        windows: machine
            .documents
            .join("WindowsPowerShell")
            .join("profile.ps1"),
        core: machine.documents.join("PowerShell").join("profile.ps1"),
    }
}

fn read(path: &Path) -> Option<Vec<u8>> {
    fs::read(path).ok()
}

fn has_block(path: &Path) -> bool {
    read(path).is_some_and(|bytes| {
        String::from_utf8_lossy(&bytes)
            .matches(MARKER_START)
            .count()
            == 1
    })
}

fn script(machine: &Machine) -> PathBuf {
    machine.install_root().join("completion").join("tool.ps1")
}

fn install(machine: &mut Machine, installer: &Path, extra: &[&str]) -> Value {
    let mut args = vec!["install", "--quiet", "--scope", machine.scope_name()];
    args.extend_from_slice(extra);
    let run = machine.run(installer, &args);
    let outcome = run.json();
    assert!(run.success, "{}\n{}", run.stdout, run.log_text());
    outcome
}

fn codes(outcome: &Value) -> Vec<String> {
    outcome["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .filter_map(|f| f["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The user's profile is what it was; the profile TigerSetup created is
/// gone with its directory.
fn assert_profiles_as_before(machine: &Machine, what: &str) {
    let profiles = profiles(machine);
    assert_eq!(
        read(&profiles.windows).as_deref(),
        Some(USER_PROFILE),
        "{what}: the user's profile is byte for byte as it was"
    );
    assert!(
        !profiles.core.exists() && !profiles.core.parent().unwrap().exists(),
        "{what}: the profile and directory TigerSetup created are gone"
    );
}

fn isolated_machine(name: &str, scope: Scope) -> Machine {
    let machine = Machine::in_scope(name, scope);
    machine.protect("");
    let windows = profiles(&machine).windows;
    fs::create_dir_all(windows.parent().unwrap()).unwrap();
    fs::write(&windows, USER_PROFILE).unwrap();
    machine
}

#[test]
fn completion_follows_path_through_its_lifecycle_and_leaves_the_profiles_as_found() {
    let dir = scratch("completion-lifecycle");
    let a = package(&dir.join("a"), "1.0.0");
    let b = package(&dir.join("b"), "1.1.0");
    let mut machine = isolated_machine("completion", Scope::User);
    let profiles = profiles(&machine);

    // Install: the script, and a block in each edition's profile.
    let outcome = install(&mut machine, &a, &[]);
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(
        fs::read_to_string(script(&machine)).unwrap(),
        "$global:TigerSetupToolCompletion = '1.0.0'\r\n"
    );
    let windows = read(&profiles.windows).unwrap();
    assert!(
        windows.starts_with(USER_PROFILE),
        "the user's lines stay in front"
    );
    let added = String::from_utf8_lossy(&windows[USER_PROFILE.len()..]).into_owned();
    assert!(
        added.starts_with(&format!("\n{MARKER_START}\n")) && !added.contains('\r'),
        "an LF profile gets the block after a blank line, in LF: {added:?}"
    );
    assert!(added.contains(&script(&machine).display().to_string()));
    let core = read(&profiles.core).unwrap();
    assert!(
        core.starts_with(&[0xEF, 0xBB, 0xBF]),
        "a new profile is UTF-8 with a byte-order mark"
    );
    assert!(has_block(&profiles.core));
    assert!(
        codes(&outcome).contains(&"windows_powershell_profiles_not_run".to_string()),
        "the run says Windows PowerShell's default policy runs no profile: {:?}",
        codes(&outcome)
    );
    let verify = machine.run(&a, &["verify", "--scope", "user"]).json();
    assert_eq!(verify["status"], "ok", "{verify}");
    assert_eq!(verify["counts"]["profile_blocks_checked"], 2);
    assert_eq!(verify["counts"]["profile_blocks_ok"], 2);
    let inspect = machine.run(&a, &["inspect", "--scope", "user"]).json();
    assert_eq!(
        inspect["owned"]["profile_blocks"].as_array().unwrap().len(),
        2
    );

    // Upgrade: a new script, the same blocks, the profiles untouched.
    let before = (read(&profiles.windows), read(&profiles.core));
    let outcome = install(&mut machine, &b, &[]);
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(
        fs::read_to_string(script(&machine)).unwrap(),
        "$global:TigerSetupToolCompletion = '1.1.0'\r\n"
    );
    assert_eq!(
        (read(&profiles.windows), read(&profiles.core)),
        before,
        "an upgrade leaves the profiles byte for byte as they were"
    );

    // Repair: a profile the user deleted gets its block back, and a line
    // the user added after the block stays.
    fs::remove_file(&profiles.core).unwrap();
    let mut windows = read(&profiles.windows).unwrap();
    windows.extend_from_slice(b"$after = 2\n");
    fs::write(&profiles.windows, &windows).unwrap();
    let verify = machine.run(&b, &["verify", "--scope", "user"]).json();
    assert!(codes(&verify).contains(&"profile_block_missing".to_string()));
    let run = machine.run(&b, &["repair", "--quiet", "--scope", "user"]);
    assert!(run.success, "{}", run.log_text());
    assert!(has_block(&profiles.core));
    assert_eq!(read(&profiles.windows).unwrap(), windows);

    // PATH off: completion goes, the script stays installed, the profiles
    // are what they were — with the user's later line.
    install(&mut machine, &b, &["--option", "path", "false"]);
    assert!(script(&machine).exists(), "the script is an ordinary file");
    let mut expected = USER_PROFILE.to_vec();
    expected.extend_from_slice(b"$after = 2\n");
    assert_eq!(read(&profiles.windows).unwrap(), expected);
    assert!(!profiles.core.exists() && !profiles.core.parent().unwrap().exists());
    fs::write(&profiles.windows, USER_PROFILE).unwrap();

    // PATH on again, then the uninstall.
    install(&mut machine, &b, &["--option", "path", "true"]);
    assert!(has_block(&profiles.windows) && has_block(&profiles.core));
    let run = machine.run(&b, &["uninstall", "--quiet", "--scope", "user"]);
    assert!(run.success, "{}", run.log_text());
    assert!(!script(&machine).exists());
    assert_profiles_as_before(&machine, "after the uninstall");
    machine.assert_no_tigersetup_residue();
}

#[test]
fn a_failed_run_takes_its_blocks_back_and_a_crashed_one_converges() {
    let dir = scratch("completion-transaction");
    let a = package(&dir.join("a"), "1.0.0");

    // A run that fails before its commit rolls the profiles back with
    // everything else.
    let mut machine = isolated_machine("completion-rollback", Scope::User);
    let run = machine.run(
        &a,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--fault",
            "before_commit:fail",
        ],
    );
    assert!(!run.success);
    assert_profiles_as_before(&machine, "after the rollback");

    // A crash after the profiles were written: the next install completes
    // the transaction, with one block each; the next uninstall after another
    // crash takes it back.
    let mut machine = isolated_machine("completion-crash", Scope::User);
    let profiles = profiles(&machine);
    let crashed = machine.run(
        &a,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--fault",
            "before_commit:crash",
        ],
    );
    assert!(!crashed.success);
    assert!(has_block(&profiles.windows) && has_block(&profiles.core));
    let outcome = install(&mut machine, &a, &[]);
    assert_eq!(outcome["recovery"]["direction"], "forward", "{outcome}");
    assert!(has_block(&profiles.windows) && has_block(&profiles.core));
    let verify = machine.run(&a, &["verify", "--scope", "user"]).json();
    assert_eq!(verify["counts"]["profile_blocks_ok"], 2, "{verify}");

    let mut machine = machine_after_crash(&a);
    let run = machine.run(&a, &["uninstall", "--quiet", "--scope", "user"]);
    assert!(run.success, "{}", run.log_text());
    assert_profiles_as_before(
        &machine,
        "after recovering a crashed install by uninstalling",
    );
}

/// A machine whose first install crashed after editing the profiles.
fn machine_after_crash(installer: &Path) -> Machine {
    let mut machine = isolated_machine("completion-crash-back", Scope::User);
    let crashed = machine.run(
        installer,
        &[
            "install",
            "--quiet",
            "--scope",
            "user",
            "--fault",
            "before_commit:crash",
        ],
    );
    assert!(!crashed.success);
    let profiles = profiles(&machine);
    assert!(
        has_block(&profiles.windows) && has_block(&profiles.core),
        "the crash came after the profiles were written"
    );
    machine
}

#[test]
fn a_profile_that_cannot_be_written_is_skipped_and_the_install_succeeds() {
    let dir = scratch("completion-read-only");
    let a = package(&dir.join("a"), "1.0.0");
    let mut machine = isolated_machine("completion-read-only", Scope::User);
    let profiles = profiles(&machine);
    let mut permissions = fs::metadata(&profiles.windows).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&profiles.windows, permissions).unwrap();

    let outcome = install(&mut machine, &a, &[]);
    assert_eq!(outcome["outcome"], "installed");
    assert!(
        codes(&outcome).contains(&"profile_unavailable_skipped".to_string()),
        "{:?}",
        codes(&outcome)
    );
    assert_eq!(read(&profiles.windows).as_deref(), Some(USER_PROFILE));
    assert!(
        has_block(&profiles.core),
        "the other edition is still activated"
    );
    let inspect = machine.run(&a, &["inspect", "--scope", "user"]).json();
    assert_eq!(
        inspect["owned"]["profile_blocks"].as_array().unwrap().len(),
        1,
        "a block that was not written is not owned"
    );
    let run = machine.run(&a, &["uninstall", "--quiet", "--scope", "user"]);
    assert!(run.success, "{}", run.log_text());
    assert_eq!(read(&profiles.windows).as_deref(), Some(USER_PROFILE));
    assert!(!profiles.core.exists());
    let mut writable = fs::metadata(&profiles.windows).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    writable.set_readonly(false);
    fs::set_permissions(&profiles.windows, writable).unwrap();
}

#[test]
fn without_path_nothing_is_activated_and_no_other_account_may_change_what_is() {
    let dir = scratch("completion-gates");
    let a = package(&dir.join("a"), "1.0.0");

    let mut machine = isolated_machine("completion-no-path", Scope::User);
    install(&mut machine, &a, &["--option", "path", "false"]);
    assert!(script(&machine).exists());
    assert_profiles_as_before(&machine, "with PATH off");

    // An install root authenticated users may change: the script is
    // installed and never loaded.
    let mut machine = isolated_machine("completion-writable", Scope::User);
    machine.protect("(A;OICI;0x1301bf;;;AU)");
    let outcome = install(&mut machine, &a, &[]);
    assert!(
        codes(&outcome).contains(&"completion_script_writable_by_others".to_string()),
        "{:?}",
        codes(&outcome)
    );
    assert_profiles_as_before(&machine, "with a script others may change");
}

/// A package of the controlled program as a TigerCli application: its
/// completion comes from its own `--completion pwsh`, at build time.
fn tigercli_package(dir: &Path) -> PathBuf {
    let bin = dir.join("payload").join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::copy(action_executable(), bin.join(ACTION_FILE_NAME)).unwrap();
    let manifest = format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "1.0.0"
publisher = "IT Tiger"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"

[[options]]
name = "path"
kind = "path"
default = true

[[path]]
entry = "bin"
option = "path"

[[completion]]
executable = "bin/{ACTION_FILE_NAME}"
source = "tigercli"
"#
    );
    build_package(dir, &manifest, &[])
}

/// Windows PowerShell 5.1, and PowerShell 7 where this machine has it.
fn editions() -> Vec<PathBuf> {
    let mut found = vec![PathBuf::from(
        std::env::var("SystemRoot").unwrap()
            + "\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
    )];
    if let Ok(output) = std::process::Command::new("where.exe")
        .arg("pwsh.exe")
        .output()
        && let Some(first) = String::from_utf8_lossy(&output.stdout).lines().next()
    {
        found.push(PathBuf::from(first.trim()));
    }
    found
}

#[test]
fn a_tigercli_applications_completion_completes_in_both_editions() {
    let dir = scratch("completion-tigercli");
    let installer = tigercli_package(&dir);
    let mut machine = isolated_machine("completion-tigercli", Scope::User);
    install(&mut machine, &installer, &[]);
    let installed = machine
        .install_root()
        .join("completion")
        .join("TigerSetupTestAction.ps1");
    let script = fs::read(&installed).unwrap();
    assert!(
        script.starts_with(&[0xEF, 0xBB, 0xBF])
            && String::from_utf8_lossy(&script).contains("Register-ArgumentCompleter"),
        "the build packaged what the application printed, with a byte-order mark"
    );
    let bin = machine.install_root().join("bin");
    let path = format!("{};{}", bin.display(), std::env::var("PATH").unwrap());
    for (shell, profile) in editions().iter().flat_map(|shell| {
        let profiles = profiles(&machine);
        [
            (shell.clone(), profiles.windows),
            (shell.clone(), profiles.core),
        ]
    }) {
        // The profile as a session loads it, then a Tab press on
        // `TigerSetupTestAction al`. Bypass is this child's own process
        // scope, the test's way to load a profile under any machine policy.
        let output = std::process::Command::new(&shell)
            .env("PATH", &path)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
            .arg(format!(
                ". '{}'; (TabExpansion2 -inputScript 'TigerSetupTestAction al' -cursorColumn 23).CompletionMatches.CompletionText",
                profile.display()
            ))
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "alpha",
            "{} with {}: {}",
            shell.display(),
            profile.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let run = machine.run(&installer, &["uninstall", "--quiet", "--scope", "user"]);
    assert!(run.success, "{}", run.log_text());
    assert_profiles_as_before(&machine, "after the uninstall");
}

#[test]
fn a_machine_installation_activates_the_installing_users_profiles() {
    let dir = scratch("completion-machine");
    let a = package(&dir.join("a"), "1.0.0");
    let mut machine = isolated_machine("completion-machine", Scope::Machine);
    let profiles = profiles(&machine);
    let outcome = install(&mut machine, &a, &[]);
    assert_eq!(outcome["outcome"], "installed");
    assert!(
        has_block(&profiles.windows) && has_block(&profiles.core),
        "{}",
        outcome["findings"]
    );
    let run = machine.run(&a, &["uninstall", "--quiet", "--scope", "machine"]);
    assert!(run.success, "{}", run.log_text());
    assert_profiles_as_before(&machine, "after the machine uninstall");
}
