//! Machine-readable output: stable codes, the event log, the JSON documents
//! and the exit-code contract. Human text is never parsed by anything; the
//! codes are.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tigersetup_format::metadata::{InstallOption, OptionKind, OptionValue};

use crate::target::ExistingInstallation;
use crate::{Error, Result};

/// The schema number of every JSON document this engine emits.
pub const SCHEMA: u32 = 1;

/// Exit codes of a generated installer.
pub mod exit {
    pub const OK: i32 = 0;
    pub const ROLLED_BACK: i32 = 1;
    pub const INVALID: i32 = 2;
    pub const DEPENDENCY_MISSING: i32 = 3;
    pub const ELEVATION: i32 = 4;
    pub const CANCELLED: i32 = 5;
    pub const IN_USE: i32 = 6;
    pub const RECOVERY_INCOMPLETE: i32 = 7;
    pub const UNSUPPORTED_PLATFORM: i32 = 8;
    pub const REBOOT_REQUIRED: i32 = 3010;
}

/// The phases a client can show progress for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Reconciling an earlier transaction before the run's own work.
    Recovery,
    /// Detecting, acquiring and installing dependencies.
    Dependencies,
    /// Removing the legacy installation (uninstall-first migration).
    Legacy,
    /// Preparing the state directory and the plan.
    Preparing,
    /// Applying the transaction's operations.
    Applying,
    /// Running a custom action of the package.
    Actions,
    /// Committing and cleaning up.
    Finishing,
    /// Undoing after a failure or a cancellation.
    RollingBack,
}

/// Where a run is, for a progress display: the phase, how many of its
/// steps are done, and what it is working on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    /// Steps completed in the phase (operations applied, bytes downloaded).
    pub done: u64,
    /// Steps in the phase; 0 when unknown.
    pub total: u64,
    /// What is being worked on: an install-relative path, a dependency
    /// name, a URL. Empty when there is nothing to name.
    pub target: String,
}

/// One event: a stable code, a message for the log, and progress where the
/// event moves a run along.
#[derive(Debug, Clone)]
pub struct Event {
    pub code: &'static str,
    pub message: String,
    pub progress: Option<Progress>,
}

/// How the Restart Manager classified a holder, which is what decides whether
/// asking it to close can work at all: Windows closes a windowed application by
/// messaging its windows, a console application with a console control event,
/// and an application it found no window for it cannot ask at all.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HolderKind {
    /// The Restart Manager found no window and no service: a graceful request
    /// has nothing to reach, so such a holder will not close for one.
    Unknown,
    MainWindow,
    OtherWindow,
    Service,
    Explorer,
    Console,
    /// A critical system process, which is never asked.
    Critical,
}

impl HolderKind {
    /// The word the log and the in-use message carry.
    pub fn as_str(self) -> &'static str {
        match self {
            HolderKind::Unknown => "no window",
            HolderKind::MainWindow => "main window",
            HolderKind::OtherWindow => "other window",
            HolderKind::Service => "service",
            HolderKind::Explorer => "explorer",
            HolderKind::Console => "console",
            HolderKind::Critical => "critical",
        }
    }
}

/// One application holding a file a run has to replace or remove, as the
/// Restart Manager names it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HolderInfo {
    /// The application's friendly name, or its executable.
    pub name: String,
    pub process_id: u32,
    /// It asked Windows to start it again after the installation.
    pub restartable: bool,
    /// What the Restart Manager can do with it, which is why a holder that
    /// refuses to close and one Windows could never ask read differently.
    pub kind: HolderKind,
    /// When the process started, as Windows reports it, so that a process id
    /// reused after the holder exits is not mistaken for the holder. It is the
    /// second half of the identity the Restart Manager itself uses, and it is
    /// an implementation detail of asking whether the holder is still there
    /// rather than something a report is about.
    #[serde(skip)]
    pub started_at: u64,
}

/// Something the engine may not decide on its own. A question is asked only
/// where the answer changes what happens to the user's machine, and it is
/// always asked before anything is mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question<'a> {
    /// These applications hold files the run must replace or remove.
    /// Answering [`Answer::Close`] asks Windows to close them and start
    /// them again afterwards; [`Answer::Cancel`] leaves the machine alone.
    CloseApplications(&'a [HolderInfo]),
}

/// A client's answer to a [`Question`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Close,
    Cancel,
}

/// Where the engine's events go besides its own log: the CLI, a UI, a test.
pub trait EventSink {
    fn event(&mut self, event: &Event);

    /// Answers a question the engine cannot decide. The default is the
    /// unattended answer — an installation asked to run without a person
    /// present proceeds — so a client only implements this to put the
    /// question to someone.
    fn question(&mut self, _question: &Question<'_>) -> Answer {
        Answer::Close
    }
}

/// A sink that discards events.
pub struct NullSink;

impl EventSink for NullSink {
    fn event(&mut self, _event: &Event) {}
}

/// The engine's log: UTF-8 text, one event per line,
/// `<RFC 3339 UTC timestamp> [<code>] <message>`, flushed after every line so
/// that a crash right after an event still leaves the event on disk.
pub struct Log {
    path: PathBuf,
    writer: BufWriter<File>,
}

impl Log {
    pub fn create(path: &Path) -> Result<Log> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                Error::new(
                    "log_unwritable",
                    format!("cannot create {}: {err}", parent.display()),
                )
            })?;
        }
        let file = File::create(path).map_err(|err| {
            Error::new(
                "log_unwritable",
                format!("cannot create {}: {err}", path.display()),
            )
        })?;
        Ok(Log {
            path: path.to_path_buf(),
            writer: BufWriter::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn write(&mut self, event: &Event) {
        let _ = writeln!(
            self.writer,
            "{} [{}] {}",
            now_rfc3339(),
            event.code,
            event.message
        );
        let _ = self.writer.flush();
    }
}

/// Appends one line, in the log's own format, to a run's log after the run
/// has ended — what the wizard did once the transaction was over (the
/// launch after install). A log that cannot be written is left as it is:
/// nothing that happens after a commit may fail the run.
pub fn append_to_log(path: &Path, code: &str, message: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = writeln!(file, "{} [{code}] {message}", now_rfc3339());
    }
}

/// Fans events out to the log (once opened) and the client's sink.
pub struct Reporter<'a> {
    log: Option<Log>,
    sink: &'a mut dyn EventSink,
}

impl<'a> Reporter<'a> {
    pub fn new(sink: &'a mut dyn EventSink) -> Self {
        Reporter { log: None, sink }
    }

    pub fn open_log(&mut self, path: &Path) -> Result<()> {
        self.log = Some(Log::create(path)?);
        Ok(())
    }

    pub fn log_path(&self) -> Option<PathBuf> {
        self.log.as_ref().map(|log| log.path().to_path_buf())
    }

    pub fn event(&mut self, code: &'static str, message: impl Into<String>) {
        self.emit(Event {
            code,
            message: message.into(),
            progress: None,
        });
    }

    /// An event that also moves a progress display.
    pub fn progress(&mut self, code: &'static str, message: impl Into<String>, progress: Progress) {
        self.emit(Event {
            code,
            message: message.into(),
            progress: Some(progress),
        });
    }

    fn emit(&mut self, event: Event) {
        if let Some(log) = &mut self.log {
            log.write(&event);
        }
        self.sink.event(&event);
    }

    /// Puts a question to the client and logs what it answered, so that the
    /// log explains a run that stopped because someone said no.
    pub fn ask(&mut self, question: &Question<'_>) -> Answer {
        let answer = self.sink.question(question);
        let Question::CloseApplications(holders) = question;
        self.event(
            "question_answered",
            format!(
                "close_applications={} answer={}",
                holders.len(),
                match answer {
                    Answer::Close => "close",
                    Answer::Cancel => "cancel",
                }
            ),
        );
        answer
    }
}

/// One verification or uninstall finding.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Finding {
    pub fn at(code: &'static str, path: &Path) -> Finding {
        Finding {
            code,
            path: Some(path.display().to_string()),
            detail: None,
        }
    }

    /// A finding about a resource named by something other than a
    /// filesystem path: a registry key and value, a PATH entry.
    pub fn named(code: &'static str, name: impl Into<String>) -> Finding {
        Finding {
            code,
            path: Some(name.into()),
            detail: None,
        }
    }

    pub fn plain(code: &'static str, detail: impl Into<String>) -> Finding {
        Finding {
            code,
            path: None,
            detail: Some(detail.into()),
        }
    }
}

/// The committed installation as the documents report it.
#[derive(Debug, Clone, Serialize)]
pub struct InstallationInfo {
    pub id: String,
    pub product_id: String,
    pub version: String,
    pub scope: String,
    pub install_root: String,
    pub state_db: String,
    pub file_count: u64,
    pub committed_at: String,
    /// The licence text a person explicitly accepted for this installation,
    /// as `Metadata::license_sha256`; `null` when no run recorded one — an
    /// unattended install or upgrade never does.
    pub accepted_license_sha256: Option<String>,
}

/// An open or just-finished transaction as the documents report it.
#[derive(Debug, Clone, Serialize)]
pub struct TransactionInfo {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_direction: Option<&'static str>,
}

/// What the migration phase did with the installation the product replaces
/// (`TigerSetup-Design.md` §5.12).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LegacyInfo {
    /// The Add/Remove Programs key the legacy installer had registered.
    pub key: String,
    /// Its uninstaller ran and the key is gone.
    pub uninstalled: bool,
}

/// What a recovery did before the run's own work.
#[derive(Debug, Clone, Serialize)]
pub struct RecoveryInfo {
    pub direction: &'static str,
    pub operations_reapplied: u32,
    pub operations_rolled_back: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub scopes: Vec<&'static str>,
    /// The declared options, so a caller knows what `--option` takes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionInfo>,
    /// The declared custom actions, so that a package that runs arbitrary
    /// programs says so wherever it is described.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionDeclaration>,
    /// The program the wizard's completion page offers to start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchDeclaration>,
    pub metadata_sha256: String,
    pub engine: EngineInfo,
}

/// The launch after install as `inspect` describes it: the install-relative
/// program, the argument templates, the working directory — `null` for the
/// program's own, empty for the install root — and the check box's initial
/// state.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LaunchDeclaration {
    pub executable: String,
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub checked: bool,
}

impl LaunchDeclaration {
    pub fn of(launch: &tigersetup_format::metadata::Launch) -> LaunchDeclaration {
        LaunchDeclaration {
            executable: launch.executable.clone(),
            arguments: launch.arguments.clone(),
            working_directory: launch.working_directory.clone(),
            checked: launch.checked,
        }
    }
}

/// One declared option as the documents describe it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OptionInfo {
    pub name: String,
    /// `boolean` or `choice`.
    pub kind: &'static str,
    /// `true`/`false` for a boolean, the choice value for a choice.
    pub default: serde_json::Value,
    /// The declared choice values, in order; empty for a boolean.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

impl OptionInfo {
    pub fn of(option: &InstallOption) -> OptionInfo {
        let default = match option.default_value() {
            OptionValue::Bool(b) => serde_json::Value::Bool(b),
            OptionValue::Choice(c) => serde_json::Value::String(c),
        };
        OptionInfo {
            name: option.name.to_ascii_lowercase(),
            kind: if option.kind == OptionKind::Choice as i32 {
                "choice"
            } else {
                "boolean"
            },
            default,
            choices: option.choices.iter().map(|c| c.value.clone()).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineInfo {
    /// From the metadata: the TigerSetup that built this installer.
    pub tigersetup_version: String,
    /// From the metadata: SHA-256 of the engine executable it was built from.
    pub engine_sha256: String,
    /// Computed from this file: SHA-256 of its engine block, the executable
    /// after the builder gave it the product's identity and icon.
    pub engine_block_sha256: String,
}

/// What the dependency phase did with one declared dependency.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DependencyInfo {
    pub id: String,
    pub name: String,
    /// `present`, `installed`, `missing`, `unacquirable`, `install_failed`,
    /// `unverified`, `requires_elevation` or `cancelled`.
    pub status: &'static str,
    /// `detected`, `installed`, `reboot_required` or `none`.
    pub action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The dependency installer's exit code, where one was obtained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Why acquisition failed, beside an `unacquirable` status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// The stable failure code, where this dependency stopped the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
}

/// The current detection result of one declared dependency, for `inspect`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DependencyStatus {
    pub id: String,
    pub name: String,
    /// Empty when the dependency accepts any version.
    pub minimum_version: String,
    /// `present` or `absent`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Where a missing dependency's installer comes from: `winget`, `url`,
    /// `embedded` or `none`.
    pub source: &'static str,
    /// The installer this package carries for the dependency, when embedded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded: Option<EmbeddedInstallerInfo>,
}

/// An embedded dependency installer: which payload entry holds it and what
/// its bytes must hash to, so a reader can verify what the package carries.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EmbeddedInstallerInfo {
    pub entry: String,
    pub sha256: String,
    pub size: u64,
}

/// One declared custom action as `inspect` describes it: what the package
/// will run, at which phase, on which operations, and the identity of a
/// packaged program.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionDeclaration {
    pub name: String,
    pub phase: &'static str,
    pub run_on: Vec<&'static str>,
    pub kind: &'static str,
    /// The command template, or the packaged file name.
    pub program: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaged: Option<EmbeddedInstallerInfo>,
    pub arguments: Vec<String>,
    pub timeout_seconds: u32,
    pub success_codes: Vec<i32>,
    pub reboot_codes: Vec<i32>,
    pub on_failure: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<PredicateInfo>,
}

/// A resource's predicate as the documents spell it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PredicateInfo {
    pub option: String,
    pub equals: String,
}

impl ActionDeclaration {
    pub fn of(action: &tigersetup_format::metadata::Action) -> ActionDeclaration {
        ActionDeclaration {
            name: action.name.clone(),
            phase: action.phase().as_str(),
            run_on: action.operations().iter().map(|op| op.as_str()).collect(),
            kind: action.kind().as_str(),
            program: action.program().to_string(),
            packaged: action.is_packaged().then(|| EmbeddedInstallerInfo {
                entry: action.entry.clone(),
                sha256: action.sha256.clone(),
                size: action.size,
            }),
            arguments: action.arguments.clone(),
            timeout_seconds: action.timeout_seconds(),
            success_codes: action.success_codes(),
            reboot_codes: action.reboot_codes.clone(),
            on_failure: action.failure_policy().as_str(),
            when: action.when.as_ref().map(|w| PredicateInfo {
                option: w.option.clone(),
                equals: w.equals.clone(),
            }),
        }
    }
}

/// What a run did with one custom action, in the order the actions ran.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionInfo {
    pub name: String,
    pub phase: &'static str,
    /// The lifecycle operation the run was.
    pub operation: &'static str,
    pub kind: &'static str,
    /// The program or script as it was run, resolved.
    pub program: String,
    /// `completed`, `failed`, `failed_continued`, `timed_out`,
    /// `launch_failed` or `interrupted`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub reboot_required: bool,
    pub duration_ms: u64,
    pub timeout_seconds: u32,
    pub on_failure: &'static str,
    /// The stable failure code, where this action stopped the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    /// The tail of what the program wrote; the log carries more.
    pub stdout: String,
    pub stderr: String,
    /// The SHA-256 of a packaged program, verified before it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// An uninstall-phase action the installation carries for its own future
/// uninstall, as the documents list it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OwnedActionInfo {
    pub name: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

/// What the database says the installation owns besides its files, listed
/// so that a lab row or an agent can key on it without reading `state.db`.
#[derive(Debug, Clone, Serialize, Default)]
pub struct OwnedResources {
    /// The recorded installer options, by lower-case name: a boolean as a
    /// boolean, a choice as its value.
    pub options: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_key: Option<String>,
    /// `<key>\<name>` of every owned registry value, the registration's
    /// included.
    pub registry_values: Vec<String>,
    /// The exact text of every owned PATH entry, with the environment key
    /// it lives in.
    pub path_entries: Vec<PathEntryInfo>,
    /// The absolute path of every owned `.lnk` and `.url`.
    pub shortcuts: Vec<String>,
    /// Every environment variable TigerSetup set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub environment_variables: Vec<EnvironmentVariableInfo>,
    /// The name of every firewall rule TigerSetup created.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub firewall_rules: Vec<String>,
    /// The uninstall-phase actions the installation keeps for its own
    /// uninstall, with the programs the state directory holds for them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<OwnedActionInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathEntryInfo {
    pub hive_key: String,
    pub entry: String,
    /// An equivalent entry existed before: TigerSetup never claimed it.
    pub pre_existed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentVariableInfo {
    pub hive_key: String,
    pub name: String,
    /// The variable held a value before TigerSetup wrote it; a removal
    /// restores that value.
    pub pre_existed: bool,
}

/// One declared integration — a file association, URL protocol, `App
/// Paths` entry, context-menu verb or the product's capability registration
/// — as the machine holds it for the installed product.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IntegrationStatus {
    /// `file_association`, `url_protocol`, `app_path`, `context_menu` or
    /// `capabilities`.
    pub kind: &'static str,
    /// The ProgID, the scheme, the executable name, `<target>:<verb>` or
    /// the product name.
    pub id: String,
    /// Whether the recorded options enable it.
    pub enabled: bool,
    /// The registry values it consists of when enabled, how many hold the
    /// intended data now, and how many the installation owns.
    pub values_total: usize,
    pub values_present: usize,
    pub values_owned: usize,
}

/// `inspect --json`.
#[derive(Debug, Clone, Serialize)]
pub struct InspectReport {
    pub schema: u32,
    pub package: PackageInfo,
    pub installation: Option<InstallationInfo>,
    /// Present exactly when `installation` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned: Option<OwnedResources>,
    pub transaction: Option<TransactionInfo>,
    /// Every installation of the product this machine holds, in either
    /// scope, so a caller can see the one it did not ask about.
    pub installations: Vec<ExistingInstallation>,
    /// Every declared dependency as this machine currently answers it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyStatus>,
    /// Every declared integration as this machine holds it, for an
    /// installed product.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub integrations: Vec<IntegrationStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
}

/// How much of each owned resource kind `verify` looked at and how much of
/// it was as the database records it.
#[derive(Debug, Clone, Serialize, Default)]
pub struct VerifyCounts {
    pub files_checked: u64,
    pub files_ok: u64,
    pub directories_checked: u64,
    pub directories_ok: u64,
    pub registry_values_checked: u64,
    pub registry_values_ok: u64,
    pub path_entries_checked: u64,
    pub path_entries_ok: u64,
    pub shortcuts_checked: u64,
    pub shortcuts_ok: u64,
    pub registration_values_checked: u64,
    pub registration_values_ok: u64,
    pub environment_variables_checked: u64,
    pub environment_variables_ok: u64,
    pub firewall_rules_checked: u64,
    pub firewall_rules_ok: u64,
    /// The stored programs of the installation's uninstall actions.
    pub action_programs_checked: u64,
    pub action_programs_ok: u64,
}

/// `verify --json`.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub schema: u32,
    pub status: &'static str,
    pub installation: Option<InstallationInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<TransactionInfo>,
    pub findings: Vec<Finding>,
    pub counts: VerifyCounts,
    /// Every declared integration as the machine holds it; a value that is
    /// owned but not present is also a `registry_value_missing` finding.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub integrations: Vec<IntegrationStatus>,
}

impl VerifyReport {
    pub fn exit_code(&self) -> i32 {
        if self.status == "ok" {
            exit::OK
        } else {
            exit::ROLLED_BACK
        }
    }
}

/// The outcome document of a mutating run.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub schema: u32,
    pub outcome: &'static str,
    pub code: String,
    pub exit_code: i32,
    pub message: String,
    pub installation: Option<InstallationInfo>,
    pub transaction: Option<TransactionInfo>,
    pub recovery: Option<RecoveryInfo>,
    /// Present when the package declares an installation to migrate from
    /// and this machine held one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyInfo>,
    /// The applications the run closed and restarted for the file set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub closed_applications: Vec<HolderInfo>,
    /// Every declared dependency the run considered, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyInfo>,
    /// Every custom action the run executed, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionInfo>,
    /// A dependency installer or a custom action asked for a restart; the
    /// product is installed.
    pub reboot_required: bool,
    /// The dependency that stopped the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency: Option<String>,
    /// Why acquisition failed, beside `dependency_unacquirable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// The installations a `scope_conflict` or `scope_ambiguous` refusal
    /// was decided from, so the caller can name the scope it meant.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub existing_installations: Vec<ExistingInstallation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    pub log: Option<String>,
    pub duration_ms: u64,
    /// What the wizard's completion page did with the package's launch
    /// offer, once the run had ended installed. Never part of the run's
    /// result: `outcome`, `code` and `exit_code` are the transaction's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchInfo>,
}

/// The launch after install as the outcome document reports it
/// (`TigerSetup-Design.md` §11.7).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LaunchInfo {
    /// `started`, `declined` (the person cleared the check box), `failed`
    /// (starting it was attempted and failed) or `unavailable` (no
    /// non-elevated context existed, so nothing was started).
    pub status: &'static str,
    /// `launch_failed` or `launch_unavailable`, beside those statuses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    /// The program, its arguments and its working directory, resolved.
    pub program: String,
    pub arguments: Vec<String>,
    pub working_directory: String,
    /// `own_token` or `shell`, for a start that was attempted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<&'static str>,
    /// The started process, where it was seen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Whether the program's window was put in the foreground; `false` for a
    /// program that showed no window in time — a tray application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Outcome {
    pub fn new(
        outcome: &'static str,
        code: impl Into<String>,
        exit_code: i32,
        message: impl Into<String>,
    ) -> Outcome {
        Outcome {
            schema: SCHEMA,
            outcome,
            code: code.into(),
            exit_code,
            message: message.into(),
            installation: None,
            transaction: None,
            recovery: None,
            legacy: None,
            closed_applications: Vec::new(),
            dependencies: Vec::new(),
            actions: Vec::new(),
            reboot_required: false,
            dependency: None,
            reason: None,
            existing_installations: Vec::new(),
            findings: Vec::new(),
            log: None,
            duration_ms: 0,
            launch: None,
        }
    }
}

/// Current time as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn now_rfc3339() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    rfc3339_from_unix(now.as_secs() as i64, now.subsec_millis())
}

/// Formats a Unix time as RFC 3339 UTC with millisecond precision.
pub fn rfc3339_from_unix(seconds: i64, millis: u32) -> String {
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A process-unique identifier for transactions and installations.
pub fn unique_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{:x}-{:x}", nanos, std::process::id(), counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(rfc3339_from_unix(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            rfc3339_from_unix(951_782_400, 5),
            "2000-02-29T00:00:00.005Z"
        );
        assert_eq!(
            rfc3339_from_unix(1_788_722_645, 123),
            "2026-09-06T19:24:05.123Z"
        );
    }

    #[test]
    fn ids_are_unique_within_a_process() {
        assert_ne!(unique_id(), unique_id());
    }
}
