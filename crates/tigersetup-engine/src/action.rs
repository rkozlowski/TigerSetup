//! Custom lifecycle actions (`TigerSetup-Design.md` §5.14): programs a
//! package asks TigerSetup to run at a phase of a run, under a controlled
//! envelope, with their execution recorded.
//!
//! The line the design draws is kept here on purpose. For a typed resource
//! TigerSetup knows what changed, so its journal can put it back. For an
//! action TigerSetup knows what it started, when, with what result — and
//! nothing about what the program did to the machine. So an action is a
//! journaled operation like any other (`run_action`), walked forward and
//! rolled back with the transaction, but its rollback undoes nothing and
//! says so (`action_not_reverted`); its evidence is the `action_run` row,
//! the log, and the outcome document.
//!
//! What this module owns: the definition as the journal and the ownership
//! table carry it (the metadata `Action` message, hex-encoded), which
//! actions a run executes and in what order, the placeholder expansion of
//! commands and arguments, the launch each kind resolves to, the verdict on
//! an exit code, and where the state directory keeps the programs an
//! uninstall will need after the installer is gone.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tigersetup_format::Metadata;
use tigersetup_format::identity::Scope;
use tigersetup_format::metadata::{
    Action, ActionFailurePolicy, ActionKind, ActionOperation, Quiescence,
};

use crate::resource::predicate::{self, Options};
use crate::state::installation::Owned;
use crate::state::journal::TxnKind;
use crate::win::process::{self, Captured, Launch};
use crate::{Error, Result};

/// The subdirectory of the state directory that keeps the programs of the
/// committed uninstall-phase actions, one directory per SHA-256.
pub const STORE_DIR: &str = "actions";

/// The subdirectory of a transaction's staging area a packaged install-phase
/// program is extracted into.
pub const STAGING_DIR: &str = "actions";

/// The `value_kind` of an action operation row.
pub const VALUE_KIND: &str = "action";

/// The `value_kind` of a store operation row that keeps a quiescence entry
/// — its definition and its packaged programs — for the uninstall.
pub const QUIESCENCE_VALUE_KIND: &str = "quiescence";

/// How much of each captured stream the outcome document keeps.
pub const OUTPUT_TAIL_BYTES: usize = 4096;

/// How many lines of each captured stream the log records per action.
pub const LOG_LINES: usize = 200;

/// The definition as a journal or ownership row carries it: the metadata
/// message, hex-encoded, so that what the engine runs is exactly what the
/// package declared and the row needs no second schema.
pub fn serialize(action: &Action) -> String {
    tigersetup_format::hex(&action.encode_to_vec())
}

pub fn deserialize(text: &str) -> Result<Action> {
    Action::decode_bytes(&unhex(text, "action")?).map_err(|err| undecodable("action", err.message))
}

/// A quiescence entry as a journal or ownership row carries it.
pub fn serialize_quiescence(entry: &Quiescence) -> String {
    tigersetup_format::hex(&entry.encode_to_vec())
}

pub fn deserialize_quiescence(text: &str) -> Result<Quiescence> {
    Quiescence::decode_bytes(&unhex(text, "quiescence")?)
        .map_err(|err| undecodable("quiescence", err.message))
}

fn undecodable(what: &str, why: String) -> Error {
    Error::new(
        "journal_inconsistent",
        format!("{what} definition cannot be decoded: {why}"),
    )
}

fn unhex(text: &str, what: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(undecodable(what, "not hex".into()));
    }
    Ok((0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap_or(0))
        .collect())
}

/// The lifecycle operation a transaction is, for `run_on`.
pub fn operation_of(kind: TxnKind) -> ActionOperation {
    match kind {
        TxnKind::Install => ActionOperation::Install,
        TxnKind::Upgrade => ActionOperation::Upgrade,
        TxnKind::Reinstall => ActionOperation::Reinstall,
        TxnKind::Repair => ActionOperation::Repair,
        TxnKind::Uninstall => ActionOperation::Uninstall,
    }
}

/// The actions one transaction executes and stores: `before` runs ahead of
/// the resource operations, `after` behind them, and `store` is what the
/// installing commit records as the installation's own uninstall actions.
#[derive(Debug, Clone, Default)]
pub struct ActionPlan {
    pub before: Vec<Action>,
    pub after: Vec<Action>,
    pub store: Vec<Action>,
    /// The quiescence entries an installing commit records for the
    /// installation's uninstall: every declared one, whatever its
    /// operations, because the uninstall evaluates them itself.
    pub store_quiescence: Vec<Quiescence>,
    /// Declared actions this run skipped, with why, for the log.
    pub skipped: Vec<(String, &'static str)>,
}

/// The plan of an installing transaction: the install-phase actions whose
/// `run_on` names this operation and whose predicate the effective options
/// satisfy, in declaration order; and every uninstall-phase action, to be
/// stored — its predicate is evaluated when the uninstall runs, against
/// the options the installation records then.
pub fn install_plan(metadata: &Metadata, options: &Options, kind: TxnKind) -> ActionPlan {
    let operation = operation_of(kind);
    let mut plan = ActionPlan {
        store_quiescence: metadata.quiescence.clone(),
        ..ActionPlan::default()
    };
    for action in &metadata.actions {
        let phase = action.phase();
        if !phase.is_install() {
            plan.store.push(action.clone());
            continue;
        }
        if !action.runs_on(operation) {
            plan.skipped.push((action.name.clone(), "operation"));
            continue;
        }
        if !predicate::enabled(action.when.as_ref(), "", options) {
            plan.skipped.push((action.name.clone(), "option"));
            continue;
        }
        if phase.is_before() {
            plan.before.push(action.clone());
        } else {
            plan.after.push(action.clone());
        }
    }
    plan
}

/// The plan of an uninstall: the installation's own recorded uninstall
/// actions — never the metadata of whichever executable runs the
/// uninstall — whose predicate the recorded options satisfy.
pub fn uninstall_plan(owned: &Owned, options: &Options) -> Result<ActionPlan> {
    let mut plan = ActionPlan::default();
    for record in &owned.actions {
        // A quiescence entry's row is not an action; `quiescence` plans it.
        if record.phase == tigersetup_format::metadata::ActionPhase::Quiesce.as_str() {
            continue;
        }
        let action = deserialize(&record.definition)?;
        if !action.runs_on(ActionOperation::Uninstall) {
            plan.skipped.push((action.name.clone(), "operation"));
            continue;
        }
        if !predicate::enabled(action.when.as_ref(), "", options) {
            plan.skipped.push((action.name.clone(), "option"));
            continue;
        }
        if action.phase().is_before() {
            plan.before.push(action);
        } else {
            plan.after.push(action);
        }
    }
    Ok(plan)
}

/// Where the state directory keeps the program of a stored action:
/// `<state directory>\actions\<sha256>\<file name>`. Content-addressed, so
/// two versions' programs never overwrite each other and an upgrade that
/// fails leaves the previous installation's untouched.
pub fn store_dir(state_dir: &Path, sha256: &str) -> PathBuf {
    state_dir.join(STORE_DIR).join(sha256)
}

pub fn stored_program(state_dir: &Path, action: &Action) -> PathBuf {
    store_dir(state_dir, &action.sha256).join(&action.file_name)
}

/// Where a transaction extracts a packaged install-phase program.
pub fn staged_program(staging_dir: &Path, action: &Action) -> PathBuf {
    staging_dir.join(STAGING_DIR).join(&action.file_name)
}

/// What a run tells an action about itself, through the placeholders and
/// the environment.
#[derive(Debug, Clone)]
pub struct Context {
    pub install_root: PathBuf,
    pub version: String,
    pub product_id: String,
    pub scope: Scope,
    pub operation: ActionOperation,
    pub quiet: bool,
}

/// Expands `%INSTALLROOT%`, `%VERSION%` and the known folders in a command,
/// an argument or a working directory. A `%` that does not open a known
/// placeholder is kept as it is, because an argument such as `100%` is an
/// argument, not a template.
pub fn expand(template: &str, context: &Context) -> String {
    let lookup = |name: &str| -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "INSTALLROOT" => Some(context.install_root.display().to_string()),
            "VERSION" => Some(context.version.clone()),
            other => crate::win::env::known_folder(other),
        }
    };
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let placeholder = after.find('%').and_then(|end| {
            let name = &after[..end];
            let plausible = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '(' | ')'));
            plausible.then(|| lookup(name).map(|value| (value, end)))?
        });
        match placeholder {
            Some((value, end)) => {
                out.push_str(&rest[..start]);
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[..=start]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn system32(file: &str) -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
        .join("System32")
        .join(file)
}

/// The launch an action resolves to once its program is on disk at
/// `program`: the interpreter and its non-interactive switches for a
/// script, the program itself for an executable; the expanded arguments;
/// the working directory; and the environment a script reads its context
/// from.
pub fn launch_of(action: &Action, program: &Path, context: &Context) -> Launch {
    let arguments: Vec<String> = action
        .arguments
        .iter()
        .map(|a| expand(a, context))
        .collect();
    let working_directory = if action.working_directory.is_empty() {
        program.parent().map(Path::to_path_buf)
    } else {
        Some(PathBuf::from(expand(&action.working_directory, context)))
    };
    let phase = action.phase();
    let environment = vec![
        (
            "TIGERSETUP_INSTALL_ROOT".to_string(),
            context.install_root.display().to_string(),
        ),
        ("TIGERSETUP_VERSION".to_string(), context.version.clone()),
        (
            "TIGERSETUP_PRODUCT_ID".to_string(),
            context.product_id.clone(),
        ),
        (
            "TIGERSETUP_SCOPE".to_string(),
            context.scope.as_str().to_string(),
        ),
        (
            "TIGERSETUP_OPERATION".to_string(),
            context.operation.as_str().to_string(),
        ),
        ("TIGERSETUP_PHASE".to_string(), phase.as_str().to_string()),
        ("TIGERSETUP_ACTION".to_string(), action.name.clone()),
        (
            "TIGERSETUP_QUIET".to_string(),
            if context.quiet { "1" } else { "0" }.to_string(),
        ),
    ];
    let timeout = Duration::from_secs(u64::from(action.timeout_seconds()));
    match action.kind() {
        ActionKind::Powershell => Launch {
            program: system32("WindowsPowerShell\\v1.0\\powershell.exe"),
            arguments: [
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ]
            .iter()
            .map(|s| s.to_string())
            .chain(std::iter::once(program.display().to_string()))
            .chain(arguments)
            .collect(),
            raw_tail: None,
            working_directory,
            environment,
            timeout,
        },
        // `/s` makes cmd strip exactly the outer quotes, so the script's
        // own quoted path and quoted arguments survive its parsing.
        ActionKind::Cmd => Launch {
            program: system32("cmd.exe"),
            arguments: vec!["/d".into(), "/s".into(), "/c".into()],
            raw_tail: Some(format!(
                "\"{}\"",
                process::command_line(program, &arguments)
            )),
            working_directory,
            environment,
            timeout,
        },
        _ => Launch {
            program: program.to_path_buf(),
            arguments,
            raw_tail: None,
            working_directory,
            environment,
            timeout,
        },
    }
}

/// The command line as the log records it.
pub fn describe_launch(launch: &Launch) -> String {
    let mut line = process::command_line(&launch.program, &launch.arguments);
    if let Some(tail) = &launch.raw_tail {
        line.push(' ');
        line.push_str(tail);
    }
    line
}

/// How one execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Completed,
    Failed,
    TimedOut,
    LaunchFailed,
}

impl Status {
    /// The `action_run` status and the outcome document's word.
    pub fn as_str(self) -> &'static str {
        use crate::state::action::status;
        match self {
            Status::Completed => status::COMPLETED,
            Status::Failed => status::FAILED,
            Status::TimedOut => status::TIMED_OUT,
            Status::LaunchFailed => status::LAUNCH_FAILED,
        }
    }

    /// The stable code a run that this status stops carries.
    pub fn failure_code(self) -> &'static str {
        match self {
            Status::Completed => "ok",
            Status::Failed => "action_failed",
            Status::TimedOut => "action_timed_out",
            Status::LaunchFailed => "action_launch_failed",
        }
    }
}

/// The verdict on what a launch produced.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub status: Status,
    pub exit_code: Option<i32>,
    pub reboot_required: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

/// Judges an exit code by the declared codes: a success code or a reboot
/// code is success (the latter with a reboot pending), a deadline is a
/// timeout, anything else is a failure.
pub fn judge(action: &Action, captured: &Captured) -> Verdict {
    let (status, reboot_required) = match captured.exit_code {
        None => (Status::TimedOut, false),
        Some(code) if action.reboot_codes.contains(&code) => (Status::Completed, true),
        Some(code) if action.success_codes().contains(&code) => (Status::Completed, false),
        Some(_) => (Status::Failed, false),
    };
    Verdict {
        status,
        exit_code: captured.exit_code,
        reboot_required,
        stdout: captured.stdout.clone(),
        stderr: captured.stderr.clone(),
        duration_ms: captured.duration.as_millis() as u64,
    }
}

/// A verdict for a program that could not be started.
pub fn launch_failed(duration_ms: u64) -> Verdict {
    Verdict {
        status: Status::LaunchFailed,
        exit_code: None,
        reboot_required: false,
        stdout: String::new(),
        stderr: String::new(),
        duration_ms,
    }
}

/// The last `OUTPUT_TAIL_BYTES` of a stream, on a character boundary.
pub fn tail(text: &str) -> String {
    if text.len() <= OUTPUT_TAIL_BYTES {
        return text.to_string();
    }
    let mut start = text.len() - OUTPUT_TAIL_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// Whether a verdict lets the transaction go on, by the action's policy.
pub fn continues(action: &Action, verdict: &Verdict) -> bool {
    verdict.status == Status::Completed || action.failure_policy() == ActionFailurePolicy::Continue
}

/// The stable finding an action found `started` on restart is reported
/// with: TigerSetup does not know what it did.
pub const INTERRUPTED: &str = "action_interrupted";
/// The stable finding a rollback records for an action that ran: what the
/// program changed stays changed.
pub const NOT_REVERTED: &str = "action_not_reverted";
/// The stable finding a `continue` action that failed is reported with.
pub const FAILED_CONTINUED: &str = "action_failed_continued";

/// The message a failed action stops the run with.
pub fn failure_message(action: &Action, verdict: &Verdict) -> String {
    let phase = action.phase().as_str();
    match verdict.status {
        Status::Completed => format!("action {} ({phase}) completed", action.name),
        Status::Failed => format!(
            "action {} ({phase}) exited with code {}",
            action.name,
            verdict.exit_code.unwrap_or_default()
        ),
        Status::TimedOut => format!(
            "action {} ({phase}) was killed after {} s",
            action.name,
            action.timeout_seconds()
        ),
        Status::LaunchFailed => format!("action {} ({phase}) could not be started", action.name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::metadata::{ACTION_ENTRY_PREFIX, ActionPhase, Predicate};

    fn action(name: &str, phase: ActionPhase, kind: ActionKind) -> Action {
        Action {
            name: name.into(),
            phase: phase as i32,
            run_on: phase
                .default_operations()
                .iter()
                .map(|op| *op as i32)
                .collect(),
            kind: kind as i32,
            command: "C:\\x\\tool.exe".into(),
            ..Default::default()
        }
    }

    fn context() -> Context {
        Context {
            install_root: PathBuf::from("C:\\Apps\\Sample"),
            version: "1.2.3".into(),
            product_id: "Vendor.Sample".into(),
            scope: Scope::User,
            operation: ActionOperation::Install,
            quiet: true,
        }
    }

    #[test]
    fn definitions_round_trip_through_the_journal_encoding() {
        let mut declared = action("build-cache", ActionPhase::PostInstall, ActionKind::Cmd);
        declared.arguments = vec!["a b".into(), "ü".into()];
        declared.reboot_codes = vec![3010];
        declared.when = Some(Predicate {
            option: "x".into(),
            equals: "true".into(),
        });
        let text = serialize(&declared);
        assert!(text.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(deserialize(&text).unwrap(), declared);
        assert_eq!(deserialize("zz").unwrap_err().code, "journal_inconsistent");
    }

    #[test]
    fn a_plan_follows_the_phase_the_operation_and_the_option() {
        let mut metadata = Metadata::default();
        let mut gated = action("preflight", ActionPhase::PreInstall, ActionKind::Exe);
        gated.when = Some(Predicate {
            option: "cache".into(),
            equals: "true".into(),
        });
        let mut repair_only = action("rebuild", ActionPhase::PostInstall, ActionKind::Exe);
        repair_only.run_on = vec![ActionOperation::Repair as i32];
        metadata.actions = vec![
            gated,
            action("build-cache", ActionPhase::PostInstall, ActionKind::Exe),
            repair_only,
            action("clear-cache", ActionPhase::PreUninstall, ActionKind::Exe),
            action("farewell", ActionPhase::PostUninstall, ActionKind::Exe),
        ];
        let mut options = Options::new();
        options.insert(
            "cache".into(),
            tigersetup_format::metadata::OptionValue::Bool(false),
        );
        let names = |actions: &[Action]| -> Vec<String> {
            actions.iter().map(|a| a.name.clone()).collect()
        };

        let plan = install_plan(&metadata, &options, TxnKind::Install);
        assert!(plan.before.is_empty(), "gated off");
        assert_eq!(names(&plan.after), vec!["build-cache"]);
        assert_eq!(names(&plan.store), vec!["clear-cache", "farewell"]);
        assert_eq!(
            plan.skipped,
            vec![
                ("preflight".to_string(), "option"),
                ("rebuild".to_string(), "operation")
            ]
        );

        options.insert(
            "cache".into(),
            tigersetup_format::metadata::OptionValue::Bool(true),
        );
        let plan = install_plan(&metadata, &options, TxnKind::Upgrade);
        assert_eq!(names(&plan.before), vec!["preflight"]);
        assert_eq!(names(&plan.after), vec!["build-cache"]);

        let plan = install_plan(&metadata, &options, TxnKind::Repair);
        assert!(plan.before.is_empty(), "repair is opt-in");
        assert_eq!(names(&plan.after), vec!["rebuild"]);

        let owned = Owned {
            actions: metadata.actions[3..]
                .iter()
                .map(|a| crate::state::installation::OwnedAction {
                    name: a.name.clone(),
                    phase: a.phase().as_str().to_string(),
                    definition: serialize(a),
                    artifact_sha256: None,
                    artifact_file: None,
                    artifact_size: None,
                })
                .collect(),
            ..Owned::default()
        };
        let plan = uninstall_plan(&owned, &options).unwrap();
        assert_eq!(names(&plan.before), vec!["clear-cache"]);
        assert_eq!(names(&plan.after), vec!["farewell"]);
    }

    #[test]
    fn placeholders_expand_and_a_lone_percent_survives() {
        let context = context();
        assert_eq!(
            expand("%INSTALLROOT%\\bin\\tool.exe", &context),
            "C:\\Apps\\Sample\\bin\\tool.exe"
        );
        assert_eq!(expand("v%VERSION%", &context), "v1.2.3");
        assert_eq!(expand("%installroot%", &context), "C:\\Apps\\Sample");
        assert_eq!(expand("100%", &context), "100%");
        assert_eq!(expand("50% of %VERSION%", &context), "50% of 1.2.3");
        assert_eq!(expand("%NOT_A_FOLDER_XYZ%", &context), "%NOT_A_FOLDER_XYZ%");
        assert_eq!(expand("a %% b", &context), "a %% b");
        let temp = crate::win::env::known_folder("TEMP").unwrap();
        assert_eq!(expand("%TEMP%\\x", &context), format!("{temp}\\x"));
    }

    #[test]
    fn each_kind_resolves_to_its_interpreter_and_the_context_travels() {
        let context = context();
        let mut exe = action("tool", ActionPhase::PostInstall, ActionKind::Exe);
        exe.arguments = vec!["--root".into(), "%INSTALLROOT%".into(), "100%".into()];
        exe.timeout_seconds = 7;
        let launch = launch_of(&exe, Path::new("C:\\Apps\\Sample\\bin\\tool.exe"), &context);
        assert_eq!(
            launch.program,
            PathBuf::from("C:\\Apps\\Sample\\bin\\tool.exe")
        );
        assert_eq!(launch.arguments, vec!["--root", "C:\\Apps\\Sample", "100%"]);
        assert_eq!(
            launch.working_directory,
            Some(PathBuf::from("C:\\Apps\\Sample\\bin"))
        );
        assert_eq!(launch.timeout, Duration::from_secs(7));
        assert!(launch.raw_tail.is_none());
        let env: std::collections::BTreeMap<_, _> = launch.environment.into_iter().collect();
        assert_eq!(env["TIGERSETUP_INSTALL_ROOT"], "C:\\Apps\\Sample");
        assert_eq!(env["TIGERSETUP_VERSION"], "1.2.3");
        assert_eq!(env["TIGERSETUP_PRODUCT_ID"], "Vendor.Sample");
        assert_eq!(env["TIGERSETUP_SCOPE"], "user");
        assert_eq!(env["TIGERSETUP_OPERATION"], "install");
        assert_eq!(env["TIGERSETUP_PHASE"], "post-install");
        assert_eq!(env["TIGERSETUP_ACTION"], "tool");
        assert_eq!(env["TIGERSETUP_QUIET"], "1");

        let mut script = action(
            "configure",
            ActionPhase::PostInstall,
            ActionKind::Powershell,
        );
        script.arguments = vec!["-Root".into(), "%INSTALLROOT%".into()];
        script.working_directory = "%INSTALLROOT%\\data".into();
        let launch = launch_of(&script, Path::new("C:\\stage\\configure.ps1"), &context);
        assert!(
            launch
                .program
                .ends_with("WindowsPowerShell\\v1.0\\powershell.exe")
        );
        assert_eq!(
            launch.arguments,
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "C:\\stage\\configure.ps1",
                "-Root",
                "C:\\Apps\\Sample"
            ]
        );
        assert_eq!(
            launch.working_directory,
            Some(PathBuf::from("C:\\Apps\\Sample\\data"))
        );

        let mut batch = action("configure", ActionPhase::PostInstall, ActionKind::Cmd);
        batch.arguments = vec!["a b".into()];
        let launch = launch_of(&batch, Path::new("C:\\st age\\configure.cmd"), &context);
        assert!(launch.program.ends_with("cmd.exe"));
        assert_eq!(launch.arguments, vec!["/d", "/s", "/c"]);
        assert_eq!(
            launch.raw_tail.as_deref(),
            Some("\"\"C:\\st age\\configure.cmd\" \"a b\"\"")
        );
        assert!(describe_launch(&launch).ends_with("\"a b\"\""));
    }

    #[test]
    fn exit_codes_are_judged_by_the_declared_codes() {
        let mut declared = action("tool", ActionPhase::PostInstall, ActionKind::Exe);
        declared.success_codes = vec![0, 2];
        declared.reboot_codes = vec![3010];
        let captured = |code: Option<i32>| Captured {
            exit_code: code,
            timed_out: code.is_none(),
            ..Captured::default()
        };
        assert_eq!(
            judge(&declared, &captured(Some(0))).status,
            Status::Completed
        );
        assert_eq!(
            judge(&declared, &captured(Some(2))).status,
            Status::Completed
        );
        let reboot = judge(&declared, &captured(Some(3010)));
        assert_eq!(reboot.status, Status::Completed);
        assert!(reboot.reboot_required);
        let failed = judge(&declared, &captured(Some(1)));
        assert_eq!(failed.status, Status::Failed);
        assert_eq!(failed.exit_code, Some(1));
        assert_eq!(judge(&declared, &captured(None)).status, Status::TimedOut);
        assert!(!continues(&declared, &failed));
        declared.on_failure = ActionFailurePolicy::Continue as i32;
        assert!(continues(&declared, &failed));
        assert_eq!(Status::Failed.failure_code(), "action_failed");
        assert!(failure_message(&declared, &failed).contains("exited with code 1"));
    }

    #[test]
    fn stored_paths_are_content_addressed_and_tails_are_bounded() {
        let mut packaged = action("clear", ActionPhase::PreUninstall, ActionKind::Cmd);
        packaged.command = String::new();
        packaged.entry = format!("{ACTION_ENTRY_PREFIX}clear.cmd");
        packaged.file_name = "clear.cmd".into();
        packaged.sha256 = "ab".repeat(32);
        assert_eq!(
            stored_program(Path::new("C:\\State"), &packaged),
            PathBuf::from(format!(
                "C:\\State\\actions\\{}\\clear.cmd",
                "ab".repeat(32)
            ))
        );
        assert_eq!(
            staged_program(Path::new("C:\\State\\txn-1"), &packaged),
            PathBuf::from("C:\\State\\txn-1\\actions\\clear.cmd")
        );
        let long = "é".repeat(OUTPUT_TAIL_BYTES);
        let tailed = tail(&long);
        assert!(tailed.len() <= OUTPUT_TAIL_BYTES);
        assert!(tailed.chars().all(|c| c == 'é'));
        assert_eq!(tail("short"), "short");
    }
}
