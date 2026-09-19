//! Custom lifecycle actions (`TigerSetup-Design.md` §5.14) under the same
//! transactional model as every resource: a packaged program or script runs
//! at its phase, on the operations it names, under the option that gates
//! it, with its arguments, working directory, environment, output, exit
//! code, deadline and failure policy all as declared; a failing action rolls
//! the run back without pretending to undo what the program did; an
//! interrupted action is reported and run again; and the uninstall-phase
//! actions outlive the installer that brought them, through one upgrade
//! that fails and one that commits.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use serde_json::Value;
use tigersetup_engine::format::Installer;
use tigersetup_engine::format::compose::{PayloadSource, compose};
use tigersetup_engine::format::payload::Compression;

/// The base of every small action package: one file, user scope.
fn base_manifest(version: &str) -> String {
    format!(
        "[package]\nid = \"{PRODUCT_ID}\"\nname = \"{PRODUCT_NAME}\"\n\
         version = \"{version}\"\npublisher = \"IT Tiger\"\n\n\
         [install]\nscopes = [\"user\"]\n\n[[files]]\nsource = \"payload/**\"\n\n"
    )
}

/// A one-file package at `version` whose `actions/` directory holds the
/// controlled program, the two shared scripts and `extra_files`, declaring
/// `actions_toml` after the base manifest.
fn build_action_package(
    name: &str,
    version: &str,
    actions_toml: &str,
    extra_files: &[(&str, &str)],
) -> PathBuf {
    let dir = Path::new(TMP).join(format!("actions-{name}-{version}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    write_actions_dir(&dir);
    for (file_name, content) in extra_files {
        fs::write(dir.join("actions").join(file_name), content).unwrap();
    }
    build_small_package(&dir, &format!("{}{actions_toml}", base_manifest(version)))
}

fn install(machine: &mut Machine, installer: &Path, extra: &[&str]) -> common::Run {
    let mut args = vec!["install", "--quiet", "--scope", "user"];
    args.extend_from_slice(extra);
    machine.run(installer, &args)
}

/// `verify` is `ok` for a small package's installation, with no leftovers.
fn assert_verified_small(machine: &mut Machine, installer: &Path) {
    let run = machine.run(installer, &["verify", "--scope", "user"]);
    let report = run.json();
    assert_eq!(report["status"], "ok", "{}\n{}", run.stdout, run.log_text());
    assert!(staging_dirs(&machine.state_dir()).is_empty());
}

fn actions_of(outcome: &Value) -> Vec<&Value> {
    outcome["actions"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn action_named<'a>(outcome: &'a Value, name: &str) -> &'a Value {
    actions_of(outcome)
        .into_iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("no action {name} in {outcome}"))
}

fn codes(document: &Value) -> Vec<String> {
    findings_of(document)
        .into_iter()
        .map(|(code, _)| code)
        .collect()
}

/// `(status, exit_code)` of every recorded run of the latest transaction.
fn recorded_runs(machine: &Machine) -> Vec<(String, Option<i32>)> {
    let connection = rusqlite::Connection::open_with_flags(
        machine.state_dir().join("state.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("the state database is readable after the run");
    let mut statement = connection
        .prepare("SELECT status, exit_code FROM action_run ORDER BY id")
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

/// The record line the controlled program appends, split into its fields.
fn parse_record(line: &str) -> (String, Vec<String>, Vec<(String, String)>) {
    let mut cwd = String::new();
    let mut args = Vec::new();
    let mut env = Vec::new();
    for field in line.split('\t') {
        if let Some(value) = field.strip_prefix("cwd=") {
            cwd = value.to_string();
        } else if let Some(value) = field.strip_prefix("args=") {
            args = value.split('|').map(str::to_string).collect();
        } else if let Some(value) = field.strip_prefix("env=") {
            env = value
                .split('|')
                .filter_map(|pair| pair.split_once('='))
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        }
    }
    (cwd, args, env)
}

/// An action is told everything about the run it is part of: its expanded
/// arguments — spaces, Unicode and a lone `%` all intact — its working
/// directory, and the `TIGERSETUP_*` environment; the run records what the
/// program wrote and how it ended.
#[test]
fn an_exe_action_runs_with_its_arguments_environment_and_working_directory() {
    let installer = build_action_package(
        "envelope",
        VERSION_A,
        r#"[[actions]]
name = "probe"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--record", "%PROGRAMDATA%\\ActionTests\\record.txt", "--stdout", "hello from the action", "--stderr", "a warning", "--stdout", "with a space", "--stdout", "zażółć gęślą jaźń", "--stdout", "100%", "--stdout", "%INSTALLROOT%\\bin", "--stdout", "v%VERSION%"]
working_directory = "%INSTALLROOT%\\bin"
"#,
        &[],
    );
    let mut machine = Machine::new("action-envelope");
    let run = install(&mut machine, &installer, &[]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "installed");
    let probe = action_named(&outcome, "probe");
    assert_eq!(probe["status"], "completed");
    assert_eq!(probe["exit_code"], 0);
    assert_eq!(probe["phase"], "post-install");
    assert_eq!(probe["operation"], "install");
    assert_eq!(probe["kind"], "exe");
    assert_eq!(probe["on_failure"], "fail");
    assert_eq!(probe["timeout_seconds"], 300);
    assert_eq!(probe["reboot_required"], false);
    assert!(probe["code"].is_null());
    assert!(
        probe["stdout"]
            .as_str()
            .unwrap()
            .contains("hello from the action"),
        "{probe}"
    );
    assert!(probe["stderr"].as_str().unwrap().contains("a warning"));
    assert_eq!(
        probe["program"].as_str().unwrap(),
        machine
            .state_dir()
            .join(format!(
                "txn-{}",
                outcome["transaction"]["id"].as_str().unwrap()
            ))
            .join("actions")
            .join(ACTION_FILE_NAME)
            .display()
            .to_string(),
        "the packaged program ran from the transaction's staging area"
    );
    assert_eq!(
        probe["sha256"].as_str().unwrap(),
        sha256_hex(&fs::read(action_executable()).unwrap())
    );

    let records =
        fs::read_to_string(machine.programdata.join("ActionTests").join("record.txt")).unwrap();
    let (cwd, args, env) = parse_record(records.lines().next().unwrap());
    let root = machine.install_root();
    assert!(
        cwd.eq_ignore_ascii_case(&root.join("bin").display().to_string()),
        "working directory {cwd}"
    );
    assert_eq!(
        args,
        vec![
            "--record",
            &machine
                .programdata
                .join("ActionTests")
                .join("record.txt")
                .display()
                .to_string(),
            "--stdout",
            "hello from the action",
            "--stderr",
            "a warning",
            "--stdout",
            "with a space",
            "--stdout",
            "zażółć gęślą jaźń",
            "--stdout",
            "100%",
            "--stdout",
            &root.join("bin").display().to_string(),
            "--stdout",
            "v1.0.0",
        ]
    );
    let env: std::collections::BTreeMap<_, _> = env.into_iter().collect();
    assert_eq!(env["TIGERSETUP_INSTALL_ROOT"], root.display().to_string());
    assert_eq!(env["TIGERSETUP_VERSION"], VERSION_A);
    assert_eq!(env["TIGERSETUP_PRODUCT_ID"], PRODUCT_ID);
    assert_eq!(env["TIGERSETUP_SCOPE"], "user");
    assert_eq!(env["TIGERSETUP_OPERATION"], "install");
    assert_eq!(env["TIGERSETUP_PHASE"], "post-install");
    assert_eq!(env["TIGERSETUP_ACTION"], "probe");
    assert_eq!(env["TIGERSETUP_QUIET"], "1");

    // The log names the action at every step, and carries its output.
    assert!(run.log_has("[action_started] probe (post-install, install)"));
    assert!(run.log_has("[action_output] probe stdout: hello from the action"));
    assert!(run.log_has("[action_output] probe stderr: a warning"));
    assert!(run.log_has("[action_completed] probe: status=completed exit_code=0"));
    assert_eq!(
        recorded_runs(&machine),
        vec![("completed".to_string(), Some(0))]
    );
    // The staging area, program included, is gone with the transaction.
    assert!(staging_dirs(&machine.state_dir()).is_empty());
    assert_verified_small(&mut machine, &installer);
}

/// A PowerShell script and a batch script each run through their
/// interpreter, non-interactively, with their arguments — spaces included
/// — and a `%`-free working directory of their own; the script's output
/// reaches the log.
#[test]
fn powershell_and_cmd_scripts_run_non_interactively_with_their_arguments() {
    let installer = build_action_package(
        "scripts",
        VERSION_A,
        r#"[[actions]]
name = "configure"
phase = "post-install"
kind = "powershell"
source = "actions/configure.ps1"
arguments = ["-Root", "%INSTALLROOT%", "-Out", "%PROGRAMDATA%\\ActionTests\\ps.txt", "-Text", "two words", "-Unicode", "łódź"]

[[actions]]
name = "register"
phase = "post-install"
kind = "cmd"
source = "actions/register.cmd"
arguments = ["%PROGRAMDATA%\\ActionTests\\cmd.txt", "two words", "%INSTALLROOT%"]
working_directory = "%INSTALLROOT%"
"#,
        &[
            (
                "configure.ps1",
                "param([string] $Root, [string] $Out, [string] $Text, [string] $Unicode)\r\n\
                 $ErrorActionPreference = 'Stop'\r\n\
                 New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null\r\n\
                 Set-Content -LiteralPath $Out -Value \"$Root|$Text|$Unicode|$env:TIGERSETUP_ACTION|$(Get-Location)\" -Encoding UTF8\r\n\
                 Write-Output 'configured'\r\n\
                 exit 0\r\n",
            ),
            (
                "register.cmd",
                "@echo off\r\nif not exist \"%~dp1\" mkdir \"%~dp1\"\r\n\
                 echo %~2^|%~3^|%TIGERSETUP_ACTION%^|%CD%> \"%~1\"\r\n\
                 echo registered\r\nexit /b 0\r\n",
            ),
        ],
    );
    let mut machine = Machine::new("action-scripts");
    let run = install(&mut machine, &installer, &[]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(action_named(&outcome, "configure")["status"], "completed");
    assert_eq!(action_named(&outcome, "register")["status"], "completed");
    assert_eq!(action_named(&outcome, "configure")["kind"], "powershell");
    assert_eq!(action_named(&outcome, "register")["kind"], "cmd");
    assert!(run.log_has("[action_output] configure stdout: configured"));
    assert!(run.log_has("[action_output] register stdout: registered"));
    assert!(
        run.log_has("-NoProfile -NonInteractive -ExecutionPolicy Bypass -File"),
        "the interpreter is invoked explicitly and non-interactively"
    );
    assert!(run.log_has("cmd.exe /d /s /c"));

    let root = machine.install_root();
    let ps = fs::read_to_string(machine.programdata.join("ActionTests").join("ps.txt")).unwrap();
    let ps = ps.trim_start_matches('\u{feff}').trim();
    let fields: Vec<&str> = ps.split('|').collect();
    assert_eq!(fields[0], root.display().to_string());
    assert_eq!(fields[1], "two words");
    assert_eq!(fields[2], "łódź");
    assert_eq!(fields[3], "configure");
    assert!(
        fields[4].to_ascii_lowercase().ends_with("\\actions"),
        "a script's default working directory is the directory it was extracted to: {}",
        fields[4]
    );
    let cmd = fs::read_to_string(machine.programdata.join("ActionTests").join("cmd.txt")).unwrap();
    let fields: Vec<&str> = cmd.trim().split('|').collect();
    assert_eq!(fields[0], "two words");
    assert_eq!(fields[1], root.display().to_string());
    assert_eq!(fields[2], "register");
    assert!(
        fields[3].eq_ignore_ascii_case(&root.display().to_string()),
        "the declared working directory: {}",
        fields[3]
    );
    assert_verified_small(&mut machine, &installer);
}

/// A failing action with the default policy fails the transaction: every
/// resource TigerSetup applied is rolled back, the action's own record says
/// it ran and failed, and nothing claims the program's side effects — the
/// marker it wrote — were undone.
#[test]
fn a_failing_action_rolls_back_owned_resources_but_not_its_side_effects() {
    let installer = build_action_package(
        "failing",
        VERSION_A,
        r#"[[actions]]
name = "flaky"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--marker", "%PROGRAMDATA%\\ActionTests\\flaky.txt", "--stderr", "cannot go on", "--exit", "3"]
"#,
        &[],
    );
    let mut machine = Machine::new("action-failing");
    let run = install(&mut machine, &installer, &[]);
    assert_eq!(run.exit_code, Some(1), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "rolled_back");
    assert_eq!(outcome["code"], "action_failed");
    assert!(
        outcome["message"]
            .as_str()
            .unwrap()
            .contains("action flaky (post-install) exited with code 3"),
        "{outcome}"
    );
    let flaky = action_named(&outcome, "flaky");
    assert_eq!(flaky["status"], "failed");
    assert_eq!(flaky["exit_code"], 3);
    assert_eq!(flaky["code"], "action_failed");
    assert!(flaky["stderr"].as_str().unwrap().contains("cannot go on"));
    let marker = machine.programdata.join("ActionTests").join("flaky.txt");
    assert!(
        marker.exists(),
        "what the program did before failing is not TigerSetup's to undo"
    );
    assert!(
        codes(&outcome).contains(&"action_not_reverted".to_string()),
        "the rollback says so: {:?}",
        codes(&outcome)
    );
    assert!(
        !machine.install_root().exists(),
        "the owned resources were rolled back"
    );
    assert!(!machine.key_exists(&machine.registration_key()));
    assert_eq!(
        recorded_runs(&machine),
        vec![("failed".to_string(), Some(3))]
    );
    assert_eq!(outcome["transaction"]["state"], "rolled_back");
    let verify = machine.run(&installer, &["verify", "--scope", "user"]);
    assert_eq!(verify.json()["status"], "not_installed");
    assert!(run.log_has("[action_failed] flaky: status=failed exit_code=3"));
    assert!(run.log_has("[transaction_rolled_back]"));
}

/// A `continue` action that fails is a warning, not a failure: the run
/// commits, the outcome and the log name the failure, and nothing reads as
/// success.
#[test]
fn a_continue_action_that_fails_is_reported_and_the_run_still_commits() {
    let installer = build_action_package(
        "continue",
        VERSION_A,
        r#"[[actions]]
name = "best-effort"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--exit", "5"]
on_failure = "continue"

[[actions]]
name = "after"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--marker", "%PROGRAMDATA%\\ActionTests\\after.txt"]
"#,
        &[],
    );
    let mut machine = Machine::new("action-continue");
    let run = install(&mut machine, &installer, &[]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "installed");
    let best_effort = action_named(&outcome, "best-effort");
    assert_eq!(best_effort["status"], "failed_continued");
    assert_eq!(best_effort["exit_code"], 5);
    assert_eq!(best_effort["on_failure"], "continue");
    assert!(best_effort["code"].is_null(), "it did not stop the run");
    assert_eq!(
        codes(&outcome),
        vec!["action_failed_continued".to_string()],
        "{outcome}"
    );
    assert_eq!(action_named(&outcome, "after")["status"], "completed");
    assert!(
        machine
            .programdata
            .join("ActionTests")
            .join("after.txt")
            .exists(),
        "the next action still ran"
    );
    assert!(run.log_has("[action_failed] best-effort: status=failed_continued exit_code=5"));
    assert!(run.log_has("[action_failed_continued] best-effort"));
    assert_eq!(
        recorded_runs(&machine),
        vec![
            ("failed".to_string(), Some(5)),
            ("completed".to_string(), Some(0))
        ]
    );
    assert_verified_small(&mut machine, &installer);
}

/// Declared success codes are success; a declared reboot code is success
/// with a reboot pending, which the outcome and the exit code carry.
#[test]
fn custom_success_codes_and_reboot_codes_are_honoured() {
    let installer = build_action_package(
        "codes",
        VERSION_A,
        r#"[[actions]]
name = "odd-success"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--exit", "4"]
success_codes = [0, 4]

[[actions]]
name = "needs-reboot"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--exit", "3010"]
reboot_codes = [3010]
"#,
        &[],
    );
    let mut machine = Machine::new("action-codes");
    let run = install(&mut machine, &installer, &[]);
    assert_eq!(
        run.exit_code,
        Some(3010),
        "{}\n{}",
        run.stdout,
        run.log_text()
    );
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(outcome["code"], "ok");
    assert_eq!(outcome["reboot_required"], true);
    assert_eq!(action_named(&outcome, "odd-success")["status"], "completed");
    assert_eq!(action_named(&outcome, "odd-success")["exit_code"], 4);
    let reboot = action_named(&outcome, "needs-reboot");
    assert_eq!(reboot["status"], "completed");
    assert_eq!(reboot["exit_code"], 3010);
    assert_eq!(reboot["reboot_required"], true);
    assert!(run.log_has("reboot_required=true"));
    assert_verified_small(&mut machine, &installer);
}

/// An action that outlives its deadline is killed, and the timeout follows
/// the failure policy: the run rolls back with `action_timed_out`, within
/// the deadline rather than the program's own time.
#[test]
fn a_timed_out_action_is_killed_and_fails_the_run() {
    let installer = build_action_package(
        "timeout",
        VERSION_A,
        r#"[[actions]]
name = "sleeper"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--marker", "%PROGRAMDATA%\\ActionTests\\sleeper.txt", "--sleep", "60"]
timeout_seconds = 2
"#,
        &[],
    );
    let mut machine = Machine::new("action-timeout");
    let started = std::time::Instant::now();
    let run = install(&mut machine, &installer, &[]);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(40),
        "the deadline, not the program, ended the action"
    );
    assert_eq!(run.exit_code, Some(1), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "rolled_back");
    assert_eq!(outcome["code"], "action_timed_out");
    let sleeper = action_named(&outcome, "sleeper");
    assert_eq!(sleeper["status"], "timed_out");
    assert!(sleeper["exit_code"].is_null());
    assert_eq!(sleeper["timeout_seconds"], 2);
    assert!(
        machine
            .programdata
            .join("ActionTests")
            .join("sleeper.txt")
            .exists(),
        "the program ran until it was killed"
    );
    assert_eq!(
        recorded_runs(&machine),
        vec![("timed_out".to_string(), None)]
    );
    assert!(!machine.install_root().exists());
    assert!(run.log_has("[action_timed_out] sleeper: status=timed_out exit_code=-"));
}

/// A program that cannot be started at all follows the failure policy too.
#[test]
fn a_program_that_cannot_be_started_fails_the_run_by_its_policy() {
    let installer = build_action_package(
        "missing",
        VERSION_A,
        r#"[[actions]]
name = "absent"
phase = "post-install"
kind = "exe"
command = "%INSTALLROOT%\\bin\\not-there.exe"
on_failure = "continue"

[[actions]]
name = "absent-fatal"
phase = "post-install"
kind = "exe"
command = "%INSTALLROOT%\\bin\\not-there-either.exe"
"#,
        &[],
    );
    let mut machine = Machine::new("action-missing");
    let run = install(&mut machine, &installer, &[]);
    assert_eq!(run.exit_code, Some(1), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["code"], "action_launch_failed");
    assert_eq!(action_named(&outcome, "absent")["status"], "launch_failed");
    assert!(action_named(&outcome, "absent")["code"].is_null());
    assert_eq!(
        action_named(&outcome, "absent-fatal")["code"],
        "action_launch_failed"
    );
    assert_eq!(
        recorded_runs(&machine),
        vec![
            ("launch_failed".to_string(), None),
            ("launch_failed".to_string(), None)
        ]
    );
    assert!(run.log_has("[action_launch_failed] absent:"));
}

/// The lifecycle of the synthetic package: the pre-install action is the
/// first operation and runs on install and upgrade when its option is on,
/// the post-install cache is rebuilt on every operation it names — repair
/// included, because it opted in — the pre-uninstall script is the first
/// operation of the uninstall and the post-uninstall program the last, after
/// the install root is gone; and a same-version rerun with nothing to do runs
/// nothing.
#[test]
fn actions_follow_their_phase_their_operations_and_their_options() {
    let fixture = fixture();
    let (a, b) = (&fixture.a, &fixture.b);
    let mut machine = Machine::new("action-lifecycle");

    // Install with the preflight on: it is operation 1, before any mutation.
    let run = machine.install_with_options(a, &[("preflight", "on")]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(
        actions_of(&outcome)
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["preflight", "build-cache"]
    );
    assert_eq!(sequence_of(&run, "run_action", "preflight"), 1);
    let last_sequence = run
        .log_text()
        .lines()
        .filter(|line| line.contains("[operation_applied]"))
        .count() as i64;
    assert_eq!(
        sequence_of(&run, "run_action", "build-cache"),
        last_sequence,
        "the post-install action is the last operation"
    );
    assert!(
        sequence_of(&run, "store_action", "clear-cache")
            < sequence_of(&run, "run_action", "build-cache")
    );
    assert!(run.log_has("[action_not_required] fail-on-purpose: disabled by its option"));
    assert!(machine.action_file("preflight.txt").exists());
    assert_eq!(
        machine.action_cache().as_deref(),
        Some("TigerSetupTestApp cache for 1.0.0")
    );
    machine.assert_verified(a);
    assert!(
        run.log_has("[action_program_stored] clear-cache:"),
        "the uninstall actions' programs are kept"
    );

    // A same-version rerun with nothing to reconcile runs nothing.
    let rerun = machine.install(a);
    assert_eq!(rerun.json()["code"], "already_installed");
    assert!(actions_of(&rerun.json()).is_empty());
    let records_before = machine.action_records().len();

    // Upgrade: the preflight runs again (upgrade is in its default
    // operations, and the option is remembered), the cache follows.
    let run = machine.install(b);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(
        actions_of(&run.json())
            .iter()
            .map(|a| (
                a["name"].as_str().unwrap(),
                a["operation"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![("preflight", "upgrade"), ("build-cache", "upgrade")]
    );
    assert_eq!(
        machine.action_cache().as_deref(),
        Some("TigerSetupTestApp cache for 1.1.0")
    );
    assert_eq!(machine.action_records().len(), records_before + 2);
    machine.assert_verified(b);

    // Repair: only the action that opted into repair runs.
    fs::remove_file(machine.action_file("cache.txt")).unwrap();
    let run = machine.repair(b);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(
        actions_of(&run.json())
            .iter()
            .map(|a| (
                a["name"].as_str().unwrap(),
                a["operation"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![("build-cache", "repair")]
    );
    assert!(run.log_has("[action_not_required] preflight: not declared for this operation"));
    assert_eq!(
        machine.action_cache().as_deref(),
        Some("TigerSetupTestApp cache for 1.1.0")
    );

    // Reinstall with an option change: install-phase actions run on it.
    let run = machine.install_with_options(b, &[("extras", "on")]);
    assert_eq!(run.json()["transaction"]["kind"], "reinstall");
    assert_eq!(
        actions_of(&run.json())
            .iter()
            .map(|a| a["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["reinstall", "reinstall"]
    );

    // Uninstall: the pre-uninstall script clears the cache first, the
    // post-uninstall program leaves its marker after the root is gone.
    let run = machine.uninstall(b);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "uninstalled");
    assert_eq!(
        actions_of(&outcome)
            .iter()
            .map(|a| (
                a["name"].as_str().unwrap(),
                a["phase"].as_str().unwrap(),
                a["operation"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("clear-cache", "pre-uninstall", "uninstall"),
            ("farewell", "post-uninstall", "uninstall")
        ]
    );
    assert_eq!(sequence_of(&run, "run_action", "clear-cache"), 1);
    assert!(
        sequence_of(&run, "run_action", "farewell") > sequence_of(&run, "remove_directory", ""),
        "the post-uninstall action runs after the install root is removed"
    );
    assert!(machine.action_cache().is_none(), "the cache was cleared");
    assert!(machine.action_file("pre-uninstall.txt").exists());
    assert!(machine.action_file("post-uninstall.txt").exists());
    let records = machine.action_records();
    assert!(
        records
            .iter()
            .any(|line| line.starts_with("clear-cache\tuninstall\tpre-uninstall")),
        "{records:?}"
    );
    let farewell = records.last().unwrap();
    let (_, _, env) = parse_record(farewell);
    assert!(env.contains(&("TIGERSETUP_PHASE".to_string(), "post-uninstall".to_string())));
    assert!(env.contains(&("TIGERSETUP_OPERATION".to_string(), "uninstall".to_string())));
    machine.assert_uninstalled(b);
}

/// The failing post-install action of the synthetic package on a reinstall:
/// the previous installation stays exactly as it was, and the action's
/// marker stays too.
#[test]
fn a_failing_action_on_a_reinstall_leaves_the_previous_installation() {
    let a = &fixture().a;
    let mut machine = Machine::new("action-reinstall-fails");
    let run = machine.install(a);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let selected = machine.selected(a);

    let run = machine.install_with_options(a, &[("fail-action", "on"), ("extras", "on")]);
    assert_eq!(run.exit_code, Some(1), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "rolled_back");
    assert_eq!(outcome["code"], "action_failed");
    assert_eq!(outcome["transaction"]["kind"], "reinstall");
    let names: Vec<&str> = actions_of(&outcome)
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["fail-on-purpose"],
        "the actions after it never ran"
    );
    assert!(machine.action_file("failed-action.txt").exists());
    assert!(codes(&outcome).contains(&"action_not_reverted".to_string()));
    // The extras that the failed run had installed are gone again, and the
    // recorded options are the previous run's.
    assert_eq!(machine.selected(a), selected);
    machine.assert_verified(a);
}

/// The uninstall actions belong to the installation, not to the installer:
/// the original package can be deleted, and the copy Add/Remove Programs
/// runs — which carries no payload — still runs them from the programs the
/// state directory kept and verified.
#[test]
fn uninstall_actions_survive_the_deletion_of_the_original_package() {
    let a = &fixture().a;
    let mut machine = Machine::new("action-durable");
    let copy = machine.scratch_path("Original-Setup.exe");
    fs::copy(&a.installer, &copy).unwrap();
    let run = machine.run(&copy, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    fs::remove_file(&copy).unwrap();

    let stored_cmd =
        machine.stored_action_program(&a.action_sha256("clear-cache.cmd"), "clear-cache.cmd");
    let stored_exe =
        machine.stored_action_program(&a.action_sha256(ACTION_FILE_NAME), ACTION_FILE_NAME);
    assert!(stored_cmd.is_file(), "{}", stored_cmd.display());
    assert!(stored_exe.is_file(), "{}", stored_exe.display());
    assert!(
        !machine
            .state_dir()
            .join("actions")
            .join(a.action_sha256("build-cache.ps1"))
            .exists(),
        "an install-phase program is not kept"
    );
    let inspect = machine.run(&machine.uninstaller(), &["inspect"]).json();
    let owned: Vec<(String, String, String)> = inspect["owned"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["name"].as_str().unwrap().to_string(),
                a["phase"].as_str().unwrap().to_string(),
                a["sha256"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        owned,
        vec![
            (
                "clear-cache".to_string(),
                "pre-uninstall".to_string(),
                a.action_sha256("clear-cache.cmd")
            ),
            (
                "farewell".to_string(),
                "post-uninstall".to_string(),
                a.action_sha256(ACTION_FILE_NAME)
            ),
        ]
    );

    let run = machine.uninstall_through_the_copy();
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["outcome"], "uninstalled");
    assert_eq!(action_named(&outcome, "clear-cache")["status"], "completed");
    let farewell = action_named(&outcome, "farewell");
    assert_eq!(farewell["status"], "completed");
    assert_eq!(
        farewell["program"].as_str().unwrap(),
        stored_exe.display().to_string(),
        "the program ran from the state directory"
    );
    assert!(machine.action_file("pre-uninstall.txt").exists());
    assert!(machine.action_file("post-uninstall.txt").exists());
    machine.assert_uninstalled(a);
}

/// The uninstall actions switch with the installation they belong to: an
/// upgrade that fails before its commit leaves the previous version's
/// program current and sweeps its own; an upgrade that commits makes the
/// new program current and sweeps the previous one; and a stored program
/// that was tampered with is found by `verify`, restored by `repair`, and
/// never run.
#[test]
fn a_committed_upgrade_switches_the_uninstall_actions_and_a_failed_one_keeps_them() {
    let toml = |marker: &str| {
        format!(
            r#"[[actions]]
name = "goodbye"
phase = "pre-uninstall"
kind = "cmd"
source = "actions/goodbye.cmd"
arguments = ["%PROGRAMDATA%\\ActionTests\\{marker}.txt"]
"#
        )
    };
    let script = |text: &str| {
        format!(
            "@echo off\r\nif not exist \"%~dp1\" mkdir \"%~dp1\"\r\necho {text}> \"%~1\"\r\nexit /b 0\r\n"
        )
    };
    let old_script = script("old");
    let new_script = script("new");
    let old = build_action_package(
        "switch",
        VERSION_A,
        &toml("old"),
        &[("goodbye.cmd", &old_script)],
    );
    let new = build_action_package(
        "switch",
        VERSION_B,
        &toml("new"),
        &[("goodbye.cmd", &new_script)],
    );
    let old_sha = sha256_hex(old_script.as_bytes());
    let new_sha = sha256_hex(new_script.as_bytes());
    let mut machine = Machine::new("action-switch");

    let run = install(&mut machine, &old, &[]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let stored_old = machine.stored_action_program(&old_sha, "goodbye.cmd");
    let stored_new = machine.stored_action_program(&new_sha, "goodbye.cmd");
    assert!(stored_old.is_file());

    // The upgrade fails before its commit: the old program stays current.
    let run = install(&mut machine, &new, &["--fault", "before_commit:fail"]);
    assert_eq!(run.exit_code, Some(1), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(run.json()["outcome"], "rolled_back");
    assert!(stored_old.is_file(), "the previous program is untouched");
    assert!(!stored_new.exists(), "the failed upgrade's program is gone");
    let inspect = machine.run(&old, &["inspect", "--scope", "user"]).json();
    assert_eq!(inspect["installation"]["version"], VERSION_A);
    assert_eq!(inspect["owned"]["actions"][0]["sha256"], old_sha);

    // Tampering with the stored program is found and repaired, and a
    // tampered program is never run.
    fs::write(&stored_old, b"@echo off\r\nexit /b 0\r\n").unwrap();
    let verify = machine.run(&old, &["verify", "--scope", "user"]).json();
    assert_eq!(verify["status"], "failed");
    assert!(
        findings_of(&verify)
            .iter()
            .any(|(code, path)| code == "action_program_modified"
                && path == &stored_old.display().to_string()),
        "{verify}"
    );
    assert_eq!(verify["counts"]["action_programs_checked"], 1);
    assert_eq!(verify["counts"]["action_programs_ok"], 0);
    let run = machine.run(&old, &["repair", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(sha256_hex(&fs::read(&stored_old).unwrap()), old_sha);
    let verify = machine.run(&old, &["verify", "--scope", "user"]).json();
    assert_eq!(verify["status"], "ok", "{verify}");
    assert_eq!(verify["counts"]["action_programs_ok"], 1);

    // The upgrade commits: the new program is current, the old one swept.
    let run = install(&mut machine, &new, &[]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert!(stored_new.is_file());
    assert!(
        !stored_old.exists(),
        "the previous version's program was swept"
    );
    assert!(run.log_has("[action_program_swept]"));
    let inspect = machine.run(&new, &["inspect", "--scope", "user"]).json();
    assert_eq!(inspect["owned"]["actions"][0]["sha256"], new_sha);

    // A tampered program refuses to run: the uninstall rolls back.
    fs::write(&stored_new, b"@echo off\r\nexit /b 0\r\n").unwrap();
    let run = machine.run(&new, &["uninstall", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(1), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(run.json()["code"], "action_program_mismatch");
    assert!(machine.install_root().exists(), "nothing was removed");
    fs::write(&stored_new, new_script.as_bytes()).unwrap();

    // The new version's own uninstall action runs.
    let run = machine.run(&new, &["uninstall", "--quiet", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(
        fs::read_to_string(machine.programdata.join("ActionTests").join("new.txt"))
            .unwrap()
            .trim(),
        "new"
    );
    assert!(
        !machine
            .programdata
            .join("ActionTests")
            .join("old.txt")
            .exists()
    );
    assert!(!machine.state_dir().exists());
}

/// A crash while an action runs leaves its record `started`; the next run
/// of the same package recovers forward, reports the earlier run as
/// interrupted rather than pretending it never happened, runs the action
/// again, and completes the installation.
#[test]
fn an_interrupted_action_is_reported_and_run_again_by_the_recovery() {
    let installer = build_action_package(
        "interrupted",
        VERSION_A,
        r#"[[actions]]
name = "settle"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--record", "%PROGRAMDATA%\\ActionTests\\settle.txt"]
"#,
        &[],
    );
    let mut machine = Machine::new("action-interrupted");
    let crashed = install(
        &mut machine,
        &installer,
        &["--fault", "after_action_started:crash"],
    );
    assert_ne!(crashed.exit_code, Some(0));
    assert_eq!(recorded_runs(&machine), vec![("started".to_string(), None)]);
    let inspect = machine
        .run(&installer, &["inspect", "--scope", "user"])
        .json();
    assert_eq!(inspect["transaction"]["state"], "running");
    assert_eq!(inspect["transaction"]["recovery_direction"], "forward");

    let run = install(&mut machine, &installer, &[]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(
        outcome["code"], "already_installed",
        "the recovery installed it"
    );
    assert_eq!(outcome["recovery"]["direction"], "forward");
    assert!(
        codes(&outcome).contains(&"action_interrupted".to_string()),
        "{outcome}"
    );
    let statuses: Vec<&str> = actions_of(&outcome)
        .iter()
        .map(|a| a["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, vec!["interrupted", "completed"]);
    assert_eq!(
        recorded_runs(&machine),
        vec![
            ("interrupted".to_string(), None),
            ("completed".to_string(), Some(0))
        ]
    );
    assert_eq!(
        fs::read_to_string(machine.programdata.join("ActionTests").join("settle.txt"))
            .unwrap()
            .lines()
            .count(),
        1,
        "the crash landed before the program was started; the recovery ran it once"
    );
    assert_verified_small(&mut machine, &installer);
}

/// A different package recovers the interrupted transaction by rolling it
/// back: the action is not run, and the rollback says its earlier run was
/// not reverted.
#[test]
fn a_rollback_recovery_never_runs_an_action_and_says_what_it_left() {
    let toml = r#"[[actions]]
name = "settle"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--record", "%PROGRAMDATA%\\ActionTests\\settle.txt"]
"#;
    let first = build_action_package("rollback", VERSION_A, toml, &[]);
    let other = build_action_package("rollback", VERSION_B, toml, &[]);
    let mut machine = Machine::new("action-rollback-recovery");
    let crashed = install(
        &mut machine,
        &first,
        &["--fault", "after_action_started:crash"],
    );
    assert_ne!(crashed.exit_code, Some(0));

    let run = install(&mut machine, &other, &[]);
    assert_eq!(run.exit_code, Some(0), "{}\n{}", run.stdout, run.log_text());
    let outcome = run.json();
    assert_eq!(outcome["recovery"]["direction"], "rollback");
    assert!(
        codes(&outcome).contains(&"action_not_reverted".to_string()),
        "{outcome}"
    );
    assert_eq!(
        fs::read_to_string(machine.programdata.join("ActionTests").join("settle.txt"))
            .unwrap()
            .lines()
            .count(),
        1,
        "the crashed run never started the program and the rollback did not either; the new install ran it once"
    );
    assert_eq!(
        recorded_runs(&machine),
        vec![
            ("interrupted".to_string(), None),
            ("completed".to_string(), Some(0))
        ],
        "the crashed run's record says what became of it; the new install's follows"
    );
    assert_eq!(outcome["installation"]["version"], VERSION_B);
}

/// A package whose packaged program does not hash to what its metadata
/// records fails verification and never runs the program.
#[test]
fn a_corrupted_packaged_program_fails_verification_and_never_runs() {
    let installer = build_action_package(
        "corrupt",
        VERSION_A,
        r#"[[actions]]
name = "probe"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--marker", "%PROGRAMDATA%\\ActionTests\\corrupt.txt"]
"#,
        &[],
    );
    // Recompose the installer with the recorded hash changed: the bytes in
    // the payload no longer match what the metadata claims for them.
    let original = Installer::open(&installer).unwrap();
    let mut metadata = original.metadata().clone();
    metadata.actions[0].sha256 = "ab".repeat(32);
    let mut archive = original.payload_archive().unwrap();
    let names: Vec<String> = original
        .entries()
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    let mut sources = Vec::new();
    for name in names {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut archive.by_name(&name).unwrap(), &mut bytes).unwrap();
        sources.push(Ok(PayloadSource { entry: name, bytes }));
    }
    let corrupted = installer.with_file_name("Corrupted-Setup.exe");
    let _ = fs::remove_file(&corrupted);
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&corrupted)
        .unwrap();
    compose(
        file,
        &mut original.engine_block().unwrap(),
        &metadata,
        sources,
        Compression::Fast,
    )
    .unwrap();

    let inspection = tigersetup_build::inspect::inspect(&corrupted).unwrap();
    assert!(!inspection.is_ok());
    let problems: Vec<&str> = inspection
        .verification
        .problems
        .iter()
        .map(|p| p.code)
        .collect();
    assert_eq!(problems, vec!["action_entry_hash_mismatch"]);

    let mut machine = Machine::new("action-corrupt");
    let run = install(&mut machine, &corrupted, &[]);
    assert_eq!(run.exit_code, Some(1), "{}\n{}", run.stdout, run.log_text());
    assert_eq!(run.json()["code"], "action_program_mismatch");
    assert!(
        !machine
            .programdata
            .join("ActionTests")
            .join("corrupt.txt")
            .exists(),
        "bytes that do not match the package never run"
    );
    assert!(!machine.install_root().exists());
}

/// `inspect` makes a package that runs programs obvious: every declared
/// action with its phase, operations, kind, packaged identity, envelope and
/// predicate.
#[test]
fn inspect_lists_every_declared_action() {
    let a = &fixture().a;
    let mut machine = Machine::new("action-inspect");
    let report = machine.inspect(a).json();
    let declared = report["package"]["actions"].as_array().unwrap();
    let names: Vec<&str> = declared
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "preflight",
            "fail-on-purpose",
            "build-cache",
            "clear-cache",
            "farewell"
        ]
    );
    let build = &declared[2];
    assert_eq!(build["phase"], "post-install");
    assert_eq!(
        build["run_on"],
        serde_json::json!(["install", "upgrade", "reinstall", "repair"])
    );
    assert_eq!(build["kind"], "powershell");
    assert_eq!(build["program"], "build-cache.ps1");
    assert_eq!(
        build["packaged"]["sha256"],
        a.action_sha256("build-cache.ps1")
    );
    assert_eq!(
        build["packaged"]["entry"],
        ".tigersetup/actions/build-cache.ps1"
    );
    assert_eq!(build["timeout_seconds"], 120);
    assert_eq!(build["success_codes"], serde_json::json!([0]));
    assert_eq!(build["on_failure"], "fail");
    assert!(build["when"].is_null());
    let preflight = &declared[0];
    assert_eq!(
        preflight["run_on"],
        serde_json::json!(["install", "upgrade", "reinstall"])
    );
    assert_eq!(
        preflight["when"],
        serde_json::json!({ "option": "preflight", "equals": "true" })
    );
    assert_eq!(
        preflight["packaged"]["sha256"],
        a.action_sha256(ACTION_FILE_NAME)
    );
    assert!(report["owned"].is_null(), "nothing is installed yet");
}
