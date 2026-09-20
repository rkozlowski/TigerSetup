//! The TigerSetup installation engine.
//!
//! The engine receives runtime metadata (through the format crate) and an
//! intent — install, uninstall, repair, verify, inspect — and emits typed
//! events and reports. Both clients, the command line and later the UI, reach
//! it only through this module: open a package, run an intent with an event
//! sink, read the outcome. Nothing below this API is a client's business.
//!
//! Manifest is intent; the database is reality. A per-installation SQLite
//! database records what is owned; uninstall plans from it, never from a
//! manifest. Every mutating run first reconciles an open transaction, and a
//! transaction ends installed or fully rolled back.

pub mod action;
pub mod dependency;
pub mod elevation;
pub mod i18n;
pub mod legacy;
pub mod plan;
pub mod quiescence;
pub mod report;
pub mod resource;
pub mod restart;
pub mod scope;
pub mod state;
pub mod target;
pub mod txn;
pub mod win;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use tigersetup_format::identity::{self, Scope};
use tigersetup_format::metadata::{OptionValue, Role};
use tigersetup_format::{Installer, Metadata};

use report::{
    ActionDeclaration, EngineInfo, EnvironmentVariableInfo, EventSink, Finding, InspectReport,
    InstallationInfo, IntegrationStatus, Outcome, OwnedActionInfo, OwnedResources, PackageInfo,
    PathEntryInfo, Reporter, TransactionInfo, VerifyCounts, VerifyReport, exit,
};
use resource::predicate::Options;
use state::Db;
use state::installation::{self, InstallationRow, Owned};
use state::journal::{self, TransactionRow, TxnKind, TxnState};
use txn::fault::{FaultInjector, FaultSpec};
use txn::{Executor, recovery};
use win::firewall::Store;
use win::registry::{self as winreg, KeyPath};

pub use tigersetup_format as format;

/// The TigerSetup version compiled into this engine.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The uninstaller copy the engine keeps beside the state database.
pub const UNINSTALLER_FILE_NAME: &str = "uninstall.exe";

/// Every failure the engine reports: a stable code and a human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Error {
        Error {
            code,
            message: message.into(),
        }
    }

    /// The exit code a run ending with this error reports.
    pub fn exit_code(&self) -> i32 {
        match self.code {
            "recovery_incomplete" | "rollback_failed" => exit::RECOVERY_INCOMPLETE,
            "dependency_missing"
            | "dependency_unacquirable"
            | "dependency_install_failed"
            | "dependency_unverified" => exit::DEPENDENCY_MISSING,
            "dependency_requires_elevation" => exit::ELEVATION,
            "dependency_cancelled" | "cancelled" => exit::CANCELLED,
            "elevation_required" | "elevation_refused" | "elevation_unavailable" => exit::ELEVATION,
            "scope_unsupported"
            | "scope_conflict"
            | "scope_ambiguous"
            | "install_root_conflict"
            | "install_root_invalid"
            | "log_unwritable"
            | "fault_invalid"
            | "package_unreadable"
            | "footer_missing"
            | "footer_invalid"
            | "footer_crc_mismatch"
            | "format_unsupported"
            | "metadata_invalid"
            | "metadata_unsupported"
            | "metadata_hash_mismatch"
            | "payload_invalid"
            | "payload_unavailable"
            | "not_implemented"
            | "option_unknown"
            | "option_value_invalid"
            | "known_folder_unavailable" => exit::INVALID,
            "firewall_requires_elevation" => exit::ELEVATION,
            "package_in_use" => exit::IN_USE,
            "platform_unsupported" => exit::UNSUPPORTED_PLATFORM,
            _ => exit::ROLLED_BACK,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

impl From<tigersetup_format::FormatError> for Error {
    fn from(err: tigersetup_format::FormatError) -> Error {
        Error {
            code: err.code,
            message: err.message,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Error {
        Error::new("io_error", err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// An opened installer: the running executable, or any installer file.
pub struct Package {
    installer: Installer,
    metadata_sha256: String,
}

impl Package {
    pub fn open(path: &Path) -> Result<Package> {
        let installer = Installer::open(path)?;
        let metadata_sha256 = installer.metadata_sha256_hex();
        Ok(Package {
            installer,
            metadata_sha256,
        })
    }

    pub fn installer(&self) -> &Installer {
        &self.installer
    }

    pub fn metadata(&self) -> &Metadata {
        self.installer.metadata()
    }

    pub fn id(&self) -> &str {
        &self.metadata().package().id
    }

    pub fn name(&self) -> &str {
        &self.metadata().package().name
    }

    pub fn version(&self) -> &str {
        &self.metadata().package().version
    }

    pub fn metadata_sha256(&self) -> &str {
        &self.metadata_sha256
    }

    /// Whether this executable is the uninstaller copy, which carries no
    /// payload and can therefore only remove, verify and describe.
    pub fn is_uninstaller(&self) -> bool {
        self.metadata().is_uninstaller()
    }

    /// The scope a run uses when the client names none: the uninstaller's
    /// own, otherwise the package's first declared scope.
    pub fn default_scope(&self) -> Scope {
        self.metadata()
            .served_scope()
            .or_else(|| self.metadata().scopes().first().copied())
            .unwrap_or(Scope::User)
    }

    fn info(&self) -> Result<PackageInfo> {
        let package = self.metadata().package();
        let engine = self.metadata().engine();
        Ok(PackageInfo {
            id: package.id.clone(),
            name: package.name.clone(),
            version: package.version.clone(),
            publisher: package.publisher.clone(),
            scopes: self
                .metadata()
                .scopes()
                .iter()
                .map(|s| s.as_str())
                .collect(),
            options: self
                .metadata()
                .options
                .iter()
                .map(report::OptionInfo::of)
                .collect(),
            actions: self
                .metadata()
                .actions
                .iter()
                .map(ActionDeclaration::of)
                .collect(),
            metadata_sha256: self.metadata_sha256.clone(),
            engine: EngineInfo {
                tigersetup_version: engine.tigersetup_version.clone(),
                engine_sha256: engine.engine_sha256.clone(),
                engine_block_sha256: self.installer.engine_executable_sha256_hex(),
            },
        })
    }
}

/// What a client asks for besides the intent.
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub scope: Scope,
    /// Overrides the package's default install root (install only).
    pub install_root: Option<PathBuf>,
    /// Where the log goes; defaults to `<state directory>\logs\`.
    pub log_path: Option<PathBuf>,
    pub faults: Vec<FaultSpec>,
    /// A file a fault creates when it reaches its boundary, so that an
    /// outside harness can interrupt the run exactly there rather than a
    /// guessed number of seconds later. Testing only.
    pub fault_signal: Option<PathBuf>,
    /// BCP 47 tag for the engine's human-readable text.
    pub lang: String,
    /// Declared options the client set explicitly, by lower-case name. An
    /// option not named here keeps the installation's recorded value, or
    /// the package default on a first install.
    pub options: Options,
    /// Acquire and install a missing dependency when online; `false` fails
    /// with `dependency_missing` before any product change.
    pub install_dependencies: bool,
    /// The run is unattended: never wait for a person (the interactive
    /// client passes `false` and answers dependency and elevation questions
    /// through its sink).
    pub quiet: bool,
    /// The person accepted the package's licence text on the wizard's
    /// licence page in this run. A successful install, upgrade or
    /// reinstall then records that text's hash as the installation's
    /// accepted licence; the wizard skips the page while the recorded hash
    /// is the package's. Three facts stay apart: the package carries a
    /// licence, a person accepted this text, and the run may proceed
    /// unattended — a quiet run is the third and never the second, so the
    /// engine records no acceptance for one.
    pub license_accepted: bool,
    /// Set by a client to cancel at the next operation boundary; the engine
    /// rolls back and reports `cancelled`.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Set when this process is a temporary copy of an executable that lies
    /// in the state directory it is about to remove: the path it was copied
    /// from. The run then logs outside the state directory and removes the
    /// whole directory once the uninstall has committed.
    pub relaunched_from: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            scope: Scope::User,
            install_root: None,
            log_path: None,
            faults: Vec::new(),
            fault_signal: None,
            lang: "en-US".into(),
            options: BTreeMap::new(),
            install_dependencies: true,
            quiet: true,
            license_accepted: false,
            cancel: None,
            relaunched_from: None,
        }
    }
}

/// The resolved locations of a run.
#[derive(Debug, Clone)]
pub struct Roots {
    pub state_dir: PathBuf,
    pub state_db: PathBuf,
    pub install_root: PathBuf,
    /// `<state directory>\uninstall.exe`.
    pub uninstaller: PathBuf,
}

/// The state database an installation of the package keeps in `scope`,
/// whether or not the package declares that scope: the database is reality,
/// and a package may stop declaring a scope it was once installed in.
pub fn state_db_path(package: &Package, scope: Scope) -> Result<PathBuf> {
    let state_dir = identity::expand_template(
        &identity::state_directory_template(scope, package.id()),
        win::env::known_folder,
    )?;
    Ok(PathBuf::from(state_dir).join("state.db"))
}

/// Resolves the state directory and install root for the package and scope,
/// expanding known folders from the environment of this process.
pub fn resolve_roots(package: &Package, options: &RunOptions) -> Result<Roots> {
    if !package.metadata().scopes().contains(&options.scope) {
        return Err(Error::new(
            "scope_unsupported",
            format!(
                "{} does not support {} scope",
                package.name(),
                options.scope.as_str()
            ),
        ));
    }
    let expand = |template: &str| identity::expand_template(template, win::env::known_folder);
    let state_db = state_db_path(package, options.scope)?;
    let state_dir = state_db.parent().map(Path::to_path_buf).unwrap_or_default();
    let install_root = match &options.install_root {
        Some(root) => {
            if !root.is_absolute() {
                return Err(Error::new(
                    "install_root_invalid",
                    format!("{} is not an absolute path", root.display()),
                ));
            }
            root.clone()
        }
        None => {
            let template = package
                .metadata()
                .install_root_template(options.scope)
                .ok_or_else(|| {
                    Error::new(
                        "metadata_invalid",
                        "the package declares no install root for this scope",
                    )
                })?;
            PathBuf::from(expand(template)?)
        }
    };
    Ok(Roots {
        state_db,
        uninstaller: state_dir.join(UNINSTALLER_FILE_NAME),
        state_dir,
        install_root,
    })
}

/// Where a run's log goes when the client names no path: beside the state
/// database, or under `%TEMP%\TigerSetup` for a run that is about to remove
/// that directory.
///
/// An uninstall is the second case whether or not it was relaunched from the
/// state directory, because a committed uninstall removes the directory it
/// would otherwise be logging into — and a log the run deletes while writing
/// it is worse than no log: on Windows the delete succeeds, the remaining
/// events go to an unlinked file, and the outcome names a path that is not
/// there.
fn default_log_path(roots: &Roots, options: &RunOptions, kind: &str) -> PathBuf {
    let stamp: String = report::now_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let name = format!("{stamp}-{kind}.log");
    match options.relaunched_from.is_some() || kind == "uninstall" {
        true => temp_directory().join(name),
        false => roots.state_dir.join("logs").join(name),
    }
}

/// `%TEMP%\TigerSetup`, where the self-removing uninstaller lives while it
/// removes the state directory.
pub fn temp_directory() -> PathBuf {
    PathBuf::from(win::env::known_folder("TEMP").unwrap_or_else(|| ".".into())).join("TigerSetup")
}

/// A directory to stage an executable this process is about to run.
///
/// Unelevated, that is `%TEMP%\TigerSetup`. Elevated it must not be: under
/// same-account elevation `%TEMP%` is still the invoking user's own Temp
/// folder, which grants that user full control over everything created in it,
/// so an unprivileged user could replace a staged copy between the write and
/// the launch and have this process run it with an administrator's token.
///
/// So an elevated run stages under `%SystemRoot%\Temp` in a directory of its
/// own, created fresh under a name nobody can predict and given the machine
/// state directory's own owner and access control list before anything is
/// written into it.
pub fn staging_directory() -> Result<PathBuf> {
    if !elevation::is_elevated() {
        let directory = temp_directory();
        win::fs::create_directory(&directory)?;
        return Ok(directory);
    }
    let root = PathBuf::from(win::env::known_folder("SYSTEMROOT").unwrap_or_else(|| ".".into()))
        .join("Temp");
    win::fs::create_directory(&root)?;
    let directory = root.join(format!("TigerSetup-{}", report::unique_id()));
    // `create_dir`, not `create_dir_all`: a name that already exists is one
    // this process did not create, and it must not be adopted.
    std::fs::create_dir(&directory).map_err(|err| {
        Error::new(
            "io_error",
            format!("cannot create {}: {err}", directory.display()),
        )
    })?;
    win::acl::set_dacl(&directory, scope::MACHINE_STATE_DIRECTORY_DACL)?;
    Ok(directory)
}

/// Where a temporary copy of the uninstaller moves the executable it was
/// copied from. A running executable can be renamed but not deleted, so the
/// copy moves it out of the state directory before removing that directory,
/// and the client schedules both files for deletion once this process has
/// exited.
pub fn origin_aside_path(copy: &Path) -> PathBuf {
    let stem = copy
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    temp_directory().join(format!("{stem}-origin.exe"))
}

fn installation_info(db: &Db, row: &InstallationRow, state_db: &Path) -> Result<InstallationInfo> {
    Ok(InstallationInfo {
        id: row.id.clone(),
        product_id: row.product_id.clone(),
        version: row.version.clone(),
        scope: row.scope.clone(),
        install_root: row.install_root.clone(),
        state_db: state_db.display().to_string(),
        file_count: installation::file_count(db)?,
        committed_at: row.committed_at.clone(),
        accepted_license_sha256: row.accepted_license_sha256.clone(),
    })
}

/// The licence acceptance this run records: the package's licence text,
/// when a person accepted it on the wizard's page. Proceeding unattended is
/// authorization through the automation path, not agreement to the text,
/// so a quiet run yields none whatever its options claim.
fn accepted_license(package: &Package, options: &RunOptions) -> Option<String> {
    (options.license_accepted && !options.quiet)
        .then(|| package.metadata().license_sha256())
        .flatten()
}

fn transaction_info(txn: &TransactionRow, direction: Option<&'static str>) -> TransactionInfo {
    TransactionInfo {
        id: txn.id.clone(),
        kind: txn.kind.as_str().to_string(),
        state: txn.state.as_str().to_string(),
        from_version: txn.from_version.clone(),
        to_version: txn.to_version.clone(),
        recovery_direction: direction,
    }
}

/// Attaches what a recovery did to the outcome: its summary, and its findings
/// ahead of the run's own.
fn with_recovery(outcome: &mut Outcome, recovery: recovery::Recovery) {
    outcome.recovery = recovery.info;
    let mut findings = recovery.findings;
    findings.append(&mut outcome.findings);
    outcome.findings = findings;
    let mut actions = recovery.actions;
    actions.append(&mut outcome.actions);
    outcome.actions = actions;
    if recovery.reboot_required {
        outcome.reboot_required = true;
        if outcome.exit_code == exit::OK {
            outcome.exit_code = exit::REBOOT_REQUIRED;
        }
    }
}

fn new_transaction(
    package: &Package,
    roots: &Roots,
    options: &RunOptions,
    kind: TxnKind,
    from: Option<&str>,
) -> TransactionRow {
    TransactionRow {
        id: report::unique_id(),
        kind,
        from_version: from.map(str::to_string),
        to_version: match kind {
            TxnKind::Uninstall => None,
            _ => Some(package.version().to_string()),
        },
        package_id: package.id().to_string(),
        package_version: package.version().to_string(),
        metadata_sha256: package.metadata_sha256().to_string(),
        scope: options.scope.as_str().to_string(),
        install_root: roots.install_root.display().to_string(),
        state: TxnState::Running,
        started_at: report::now_rfc3339(),
        finished_at: None,
        registration_key: None,
        accepted_license_sha256: None,
    }
}

fn human(options: &RunOptions, key: &str, values: &[(&str, &str)]) -> String {
    i18n::fill(i18n::text(&options.lang, key), values)
}

/// Refuses a machine older than the package supports, before the run looks
/// at anything else. A Windows that will not answer for its own build is
/// not evidence of an old one, so it is allowed to proceed.
pub fn check_platform(package: &Package) -> Result<()> {
    let minimum = package.metadata().minimum_build();
    match win::version::build_number() {
        Some(build) if build < minimum => Err(Error::new(
            "platform_unsupported",
            format!(
                "{} {} needs Windows build {minimum} or later; this machine runs build {build}",
                package.name(),
                package.version()
            ),
        )),
        _ => Ok(()),
    }
}

/// Refuses a mutating run that cannot write the roots its scope uses. A
/// client elevates before calling the engine (see [`elevation`]); this is
/// the engine's own guarantee that no client can start a machine-scope
/// change it has no right to finish.
fn require_privileges(options: &RunOptions, roots: &Roots) -> Result<()> {
    if elevation::required(options.scope, roots) {
        return Err(elevation::unavailable(format!(
            "{} scope installs into {} and keeps its state in {}",
            options.scope.as_str(),
            roots.install_root.display(),
            roots.state_dir.display()
        )));
    }
    Ok(())
}

/// Creates the state directory and gives it the protection its scope calls
/// for, before anything is written into it. Called on every mutating run,
/// so a directory whose access control list drifted is repaired rather than
/// trusted.
fn prepare_state_directory(
    roots: &Roots,
    options: &RunOptions,
    reporter: &mut Reporter<'_>,
) -> Result<()> {
    win::fs::create_directory(&roots.state_dir)?;
    let protection = scope::locations(options.scope).protect_state_directory(&roots.state_dir)?;
    if let Some(code) = protection.code() {
        reporter.event(code, roots.state_dir.display().to_string());
    }
    Ok(())
}

/// Runs one mutating intent end to end and turns any error into an outcome.
fn run(
    kind: &'static str,
    package: &Package,
    options: &RunOptions,
    sink: &mut dyn EventSink,
    body: impl FnOnce(&Package, &RunOptions, &Roots, &mut Reporter<'_>) -> Result<Outcome>,
) -> Outcome {
    let started = Instant::now();
    let mut reporter = Reporter::new(sink);
    let mut outcome = check_platform(package)
        .and_then(|()| resolve_roots(package, options))
        .and_then(|roots| {
            require_privileges(options, &roots)?;
            // `run` carries only the mutating intents; `verify` and
            // `inspect` never reach it, which is how they keep their promise
            // to write nothing at all — a default log would live under the
            // state directory and so create it for a package the machine
            // does not have (`TigerSetup-Design.md` §6.2).
            let log_path = options
                .log_path
                .clone()
                .unwrap_or_else(|| default_log_path(&roots, options, kind));
            reporter.open_log(&log_path)?;
            reporter.event(
                "run_started",
                format!(
                    "{kind} package={} version={} scope={} elevated={} install_root={} state_db={} engine={ENGINE_VERSION}",
                    package.id(),
                    package.version(),
                    options.scope.as_str(),
                    elevation::is_elevated(),
                    roots.install_root.display(),
                    roots.state_db.display()
                ),
            );
            for fault in &options.faults {
                reporter.event("fault_armed", fault.describe());
            }
            body(package, options, &roots, &mut reporter)
        })
        .unwrap_or_else(|err| {
        reporter.event("run_failed", err.to_string());
        let message = human(options, "outcome.failed", &[("name", package.name()), ("version", package.version()), ("reason", &err.message)]);
        Outcome::new("failed", err.code, err.exit_code(), message)
    });
    outcome.log = reporter.log_path().map(|p| p.display().to_string());
    outcome.duration_ms = started.elapsed().as_millis() as u64;
    reporter.event(
        "run_finished",
        format!(
            "outcome={} code={} exit_code={}",
            outcome.outcome, outcome.code, outcome.exit_code
        ),
    );
    outcome
}

/// Refuses an intent that needs payload bytes when this executable is the
/// uninstaller copy, naming the installer that has them.
fn require_payload(package: &Package) -> Result<()> {
    if package.is_uninstaller() {
        return Err(Error::new(
            "payload_unavailable",
            format!(
                "{} carries no payload; run the {} {} installer to install or repair it",
                package.installer().path().display(),
                package.name(),
                package.version()
            ),
        ));
    }
    Ok(())
}

/// Writes `<state directory>\uninstall.exe`: this package's loader and
/// compressed engine blocks copied verbatim, the same metadata marked as the
/// uninstaller of this scope, an empty payload and a footer, staged and
/// renamed write-through so that the file is either the old one or the new
/// one. It is bootstrap, not an owned resource — an upgrade replaces it
/// before its transaction opens.
fn write_uninstaller(
    package: &Package,
    roots: &Roots,
    options: &RunOptions,
    reporter: &mut Reporter<'_>,
) -> Result<()> {
    use std::fs::OpenOptions;

    let mut metadata = package.metadata().clone();
    metadata.role = Role::Uninstaller as i32;
    metadata.uninstaller_scope = options.scope.tag();

    win::fs::create_directory(&roots.state_dir)?;
    let temp = win::fs::temp_path_for(&roots.uninstaller);
    let _ = std::fs::remove_file(&temp);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|err| {
            Error::new(
                "io_error",
                format!("cannot create {}: {err}", temp.display()),
            )
        })?;
    let handle = file.try_clone()?;
    let composed = package
        .installer()
        .engine_block_for_composition()
        .and_then(|engine| {
            // The uninstaller carries no payload, the engine block is
            // copied as it is, and its metadata is stored in a frame the
            // decoder reads: the engine links no compressor.
            tigersetup_format::compose::compose_without_payload(
                file,
                &mut package.installer().loader_block()?,
                &engine,
                &metadata,
            )
        });
    let result = composed.map_err(Error::from).and_then(|_| {
        win::fs::flush(&handle, &temp)?;
        Ok(())
    });
    drop(handle);
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    win::fs::rename_write_through(&temp, &roots.uninstaller)?;
    reporter.event(
        "uninstaller_written",
        roots.uninstaller.display().to_string(),
    );
    Ok(())
}

/// Installs the package, upgrades an installed different version, or
/// reconciles the installed same version when the client set an option
/// explicitly. Every path first recovers an open transaction.
pub fn install(package: &Package, options: &RunOptions, sink: &mut dyn EventSink) -> Outcome {
    run(
        "install",
        package,
        options,
        sink,
        |package, options, roots, reporter| {
            require_payload(package)?;
            // The scope this run was given is held to the package's policy
            // here, whichever client resolved it: an install into a scope
            // that holds nothing while the other scope holds the product is
            // refused before anything is touched, unless the package allows
            // two installations side by side (`target`).
            let resolution = target::resolve(
                package,
                Some(options.scope),
                Some(elevation::Intent::Install),
            )?;
            if let Some(outcome) = resolution.outcome(package, options) {
                reporter.event(
                    resolution.refusal_code().unwrap_or("scope_conflict"),
                    resolution
                        .error(package)
                        .map(|err| err.message)
                        .unwrap_or_default(),
                );
                return Ok(outcome);
            }
            let mut fault =
                FaultInjector::with_signal(options.faults.clone(), options.fault_signal.clone());
            // An open transaction is reconciled before anything else. On a
            // machine that holds nothing for this package there is nothing
            // to reconcile and nothing to open, so a run that stops in the
            // dependency phase creates no state database and claims nothing.
            // Its log is already in the state directory by then, which is
            // where a failed install's diagnostics belong.
            let (mut opened, recovery) = match roots.state_db.exists() {
                true => {
                    prepare_state_directory(roots, options, reporter)?;
                    let db = Db::open_rw(&roots.state_db)?;
                    let recovery = recovery::recover(
                        &db,
                        package,
                        &roots.state_dir,
                        options.quiet,
                        &mut fault,
                        reporter,
                    )?;
                    (Some(db), recovery)
                }
                false => (None, recovery::Recovery::none()),
            };

            // The options in effect decide which dependencies the run
            // requires, so they are resolved before the dependency phase;
            // the transaction resolves them again from the same inputs.
            let recorded = match &opened {
                Some(db) if installation::read(db)?.is_some() => installation::options(db)?,
                _ => Options::new(),
            };
            let effective =
                plan::effective_options(package.metadata(), &recorded, &options.options)?;

            // Dependencies are requirements, not owned resources: they are
            // satisfied before the product transaction opens and outside it,
            // so a failure here leaves no product change.
            let phase = dependency::run(package, options, &effective, roots, &mut fault, reporter);
            if let Some(db) = &opened {
                dependency::record_events(db, &phase)?;
            }
            if let Some(failure) = &phase.failure {
                let mut outcome = dependency::failure_outcome(package, options, &phase, failure);
                with_recovery(&mut outcome, recovery);
                return Ok(outcome);
            }
            // The installation this product replaces is removed by its own
            // uninstaller, once, before the product transaction and outside
            // it: a migration that fails leaves the machine exactly as it
            // was.
            let legacy = legacy::migrate(
                package.metadata().legacy.as_ref(),
                &scope::locations(options.scope),
                &winreg::Roots::from_env(),
                reporter,
            )?;

            let db = match opened.take() {
                Some(db) => db,
                None => {
                    prepare_state_directory(roots, options, reporter)?;
                    let db = Db::open_rw(&roots.state_db)?;
                    dependency::record_events(&db, &phase)?;
                    db
                }
            };

            // Install, reinstall and upgrade are one reconciliation of the
            // package and the options in effect against what is owned; only
            // a same-version run with nothing to change is a no-op — and a
            // licence acceptance the installation does not record yet is
            // something to change.
            let existing = installation::read(&db)?;
            let accepted = accepted_license(package, options);
            let mut outcome = match &existing {
                Some(row)
                    if row.version == package.version()
                        && options.options.is_empty()
                        && (accepted.is_none() || accepted == row.accepted_license_sha256) =>
                {
                    reporter.event(
                        "already_installed",
                        format!("version={} install_root={}", row.version, row.install_root),
                    );
                    let message = human(
                        options,
                        "outcome.already_installed",
                        &[
                            ("name", package.name()),
                            ("version", package.version()),
                            ("root", &row.install_root),
                        ],
                    );
                    Outcome::new("installed", "already_installed", exit::OK, message)
                }
                _ => {
                    let kind = match &existing {
                        None => TxnKind::Install,
                        Some(row) if row.version == package.version() => TxnKind::Reinstall,
                        Some(_) => TxnKind::Upgrade,
                    };
                    reconcile_run(
                        package,
                        options,
                        roots,
                        reporter,
                        &db,
                        &mut fault,
                        kind,
                        existing.as_ref(),
                        false,
                    )?
                }
            };
            if outcome.outcome == "installed" {
                let row = installation::read(&db)?.ok_or_else(|| {
                    Error::new("journal_inconsistent", "commit left no installation")
                })?;
                outcome.installation = Some(installation_info(&db, &row, &roots.state_db)?);
            }
            outcome.legacy = legacy;
            with_recovery(&mut outcome, recovery);
            dependency::attach(&mut outcome, &phase);
            Ok(outcome)
        },
    )
}

/// Reconciles the installation with the package: a missing or modified owned
/// file is rewritten from the payload, and every missing resource is
/// re-created. Repair is not a second installer engine; it is the same
/// reconciliation with the same journal.
pub fn repair(package: &Package, options: &RunOptions, sink: &mut dyn EventSink) -> Outcome {
    run(
        "repair",
        package,
        options,
        sink,
        |package, options, roots, reporter| {
            require_payload(package)?;
            prepare_state_directory(roots, options, reporter)?;
            let db = Db::open_rw(&roots.state_db)?;
            let mut fault =
                FaultInjector::with_signal(options.faults.clone(), options.fault_signal.clone());
            let recovery = recovery::recover(
                &db,
                package,
                &roots.state_dir,
                options.quiet,
                &mut fault,
                reporter,
            )?;

            let Some(existing) = installation::read(&db)? else {
                return Err(Error::new(
                    "not_installed",
                    format!(
                        "{} is not installed; there is nothing to repair",
                        package.name()
                    ),
                ));
            };
            let kind = if existing.version == package.version() {
                TxnKind::Repair
            } else {
                TxnKind::Upgrade
            };
            let mut outcome = reconcile_run(
                package,
                options,
                roots,
                reporter,
                &db,
                &mut fault,
                kind,
                Some(&existing),
                true,
            )?;
            if outcome.outcome == "installed" {
                let row = installation::read(&db)?.ok_or_else(|| {
                    Error::new("journal_inconsistent", "commit left no installation")
                })?;
                if kind == TxnKind::Repair {
                    outcome.message = human(
                        options,
                        "outcome.repaired",
                        &[
                            ("name", package.name()),
                            ("version", package.version()),
                            ("root", &row.install_root),
                        ],
                    );
                }
                outcome.installation = Some(installation_info(&db, &row, &roots.state_db)?);
            }
            with_recovery(&mut outcome, recovery);
            Ok(outcome)
        },
    )
}

/// One reconciliation of desired state against owned state: install,
/// upgrade, reinstall and repair differ only in what they start from and in
/// whether an owned file that has to be rewritten is reported as repaired.
#[allow(clippy::too_many_arguments)]
fn reconcile_run(
    package: &Package,
    options: &RunOptions,
    roots: &Roots,
    reporter: &mut Reporter<'_>,
    db: &Db,
    fault: &mut FaultInjector,
    kind: TxnKind,
    existing: Option<&InstallationRow>,
    repair: bool,
) -> Result<Outcome> {
    let install_root = match existing {
        Some(row) => {
            let installed = PathBuf::from(&row.install_root);
            if let Some(requested) = &options.install_root
                && requested != &installed
            {
                return Err(Error::new(
                    "install_root_conflict",
                    format!(
                        "{} is installed at {}; this run cannot move it to {}",
                        row.version,
                        row.install_root,
                        requested.display()
                    ),
                ));
            }
            installed
        }
        None => roots.install_root.clone(),
    };

    let recorded = match existing {
        Some(_) => installation::options(db)?,
        None => Options::new(),
    };
    let effective = plan::effective_options(package.metadata(), &recorded, &options.options)?;
    if !effective.is_empty() {
        let text: Vec<String> = effective
            .iter()
            .map(|(name, value)| format!("{name}={}", value.as_text()))
            .collect();
        reporter.event("options_resolved", text.join(" "));
    }

    write_uninstaller(package, roots, options, reporter)?;

    let owned = match existing {
        Some(row) => installation::owned(db, row)?,
        None => Owned::default(),
    };
    // Nothing the database records may address a location outside this
    // scope. A machine-scope run plans from these rows with an
    // administrator's rights, so they are checked before they are planned
    // from at all.
    let locations = scope::locations(options.scope);
    locations.confine_owned(&owned)?;
    let registry_roots = winreg::Roots::from_env();
    let desired = plan::desired(
        package.metadata(),
        &effective,
        options.scope,
        &install_root,
        &roots.uninstaller,
        &registry_roots,
        &owned,
    )?;
    let shortcut_folders = locations.shortcut_folders()?;
    let firewall = firewall_store(reporter);
    let actions = action::install_plan(package.metadata(), &effective, kind);
    report_skipped_actions(&actions, reporter);
    let mut payload = package.installer().payload()?;
    let planning = Instant::now();
    let file_batches = plan::FileBatchIndex::of(package.metadata());
    let planned = plan::reconcile(plan::Reconcile {
        desired: Some(&desired),
        owned: &owned,
        install_root: &install_root,
        payload: Some(&mut payload),
        roots: &registry_roots,
        scope: options.scope,
        inspect_files: existing.is_some(),
        repair,
        shortcut_folders: &shortcut_folders,
        firewall: firewall.as_ref(),
        actions: &actions,
        file_batches: &file_batches,
    })?;
    reporter.event(
        "plan_completed",
        format!(
            "operations={} ms={}",
            planned.operations.len(),
            planning.elapsed().as_millis()
        ),
    );
    for finding in &planned.findings {
        reporter.event(finding.code, finding.path.clone().unwrap_or_default());
    }

    let mut txn = new_transaction(
        package,
        roots,
        options,
        kind,
        existing.map(|row| row.version.as_str()),
    );
    txn.install_root = install_root.display().to_string();
    txn.registration_key = Some(desired.registration_key.to_string());
    // What the commit will record as the accepted licence: the text the
    // person accepted in this run, else what the installation already
    // records — an upgrade or repair nobody accepted anything in carries
    // the earlier acceptance forward, and a rollback never reaches the
    // installation row at all.
    txn.accepted_license_sha256 = accepted_license(package, options)
        .or_else(|| existing.and_then(|row| row.accepted_license_sha256.clone()));

    // The package's own quiescence first — an application the Restart
    // Manager cannot close is stopped the way the package says — then the
    // Restart Manager for whatever still holds a file the plan touches.
    // Everything from here to the outcome runs under the quiescence's
    // settlement: a failure anywhere past the stop resumes what was
    // stopped before it is reported.
    let quiescence_context = action::Context {
        install_root: install_root.clone(),
        version: package.version().to_string(),
        product_id: package.id().to_string(),
        scope: options.scope,
        operation: action::operation_of(kind),
        quiet: options.quiet,
    };
    let entries = quiescence::install_plan(package.metadata(), &effective, kind, reporter);
    let mut quiesced = quiescence::quiesce(
        &mut quiescence::Site {
            db,
            state_dir: &roots.state_dir,
            payload: Some(&mut payload),
            transaction_id: &txn.id,
            context: quiescence_context.clone(),
            reporter,
        },
        &entries,
    )?;
    let transaction_id = txn.id.clone();
    let outcome = run_installing_transaction(
        package,
        options,
        roots,
        reporter,
        db,
        fault,
        kind,
        existing,
        txn,
        &planned,
        &effective,
        &install_root,
        payload,
    );
    let mut outcome = match outcome {
        Ok(outcome) => outcome,
        Err(err) => {
            quiesced.resume(db, &transaction_id, &quiescence_context, reporter);
            return Err(err);
        }
    };
    quiesced.settle(
        &mut outcome,
        false,
        db,
        &transaction_id,
        &quiescence_context,
        reporter,
    );
    Ok(outcome)
}

/// The transaction of an installing run, from the Restart Manager to the
/// outcome: everything a quiescence settlement wraps.
#[allow(clippy::too_many_arguments)]
fn run_installing_transaction(
    package: &Package,
    options: &RunOptions,
    roots: &Roots,
    reporter: &mut Reporter<'_>,
    db: &Db,
    fault: &mut FaultInjector,
    kind: TxnKind,
    existing: Option<&InstallationRow>,
    txn: TransactionRow,
    planned: &plan::Plan,
    effective: &Options,
    install_root: &Path,
    payload: tigersetup_format::Payload,
) -> Result<Outcome> {
    // Applications holding files this plan replaces or removes are closed
    // before the transaction opens, so a run that cannot free them has
    // changed nothing at all.
    let quiescence = restart::Quiescence::acquire(
        &restart::files_at_risk(&planned.operations, install_root),
        options.quiet,
        reporter,
    )?;

    journal::begin(db, &txn, &planned.operations, effective)?;
    let counts = planned.counts;
    reporter.event(
        "transaction_started",
        format!(
            "transaction={} kind={} from={} to={} operations={} kept={} replaced={} added={} removed={} directories_created={} directories_removed={} resources={} repaired={} actions={} stored_actions={}",
            txn.id,
            kind.as_str(),
            txn.from_version.as_deref().unwrap_or("-"),
            package.version(),
            planned.operations.len(),
            counts.kept,
            counts.replaced,
            counts.added,
            counts.removed,
            counts.directories_created,
            counts.directories_removed,
            counts.resource_operations,
            counts.repaired,
            counts.actions,
            counts.stored_actions
        ),
    );

    let installation_id = existing
        .map(|row| row.id.clone())
        .unwrap_or_else(report::unique_id);
    let mut executor = Executor::new(db, txn, &roots.state_dir, Some(payload), fault, reporter)
        .cancellable(options.cancel.clone())
        .unattended(options.quiet);
    let result = executor
        .run_forward(false)
        .and_then(|_| executor.commit(&installation_id, ENGINE_VERSION));
    let mut outcome = finish(&mut executor, result, options, package)?;
    drop(executor);
    outcome.closed_applications = quiescence.closed().to_vec();
    quiescence.release(reporter);
    let mut findings = planned.findings.clone();
    findings.append(&mut outcome.findings);
    outcome.findings = findings;
    if outcome.outcome == "installed" && kind == TxnKind::Upgrade {
        outcome.message = human(
            options,
            "outcome.upgraded",
            &[
                ("name", package.name()),
                (
                    "from",
                    existing.map(|row| row.version.as_str()).unwrap_or(""),
                ),
                ("version", package.version()),
                ("root", &install_root.display().to_string()),
            ],
        );
    }
    Ok(outcome)
}

/// The firewall store this run may write, or `None` — logged once — when
/// the process may not: rules are machine-wide and need an administrator.
fn firewall_store(reporter: &mut Reporter<'_>) -> Option<Store> {
    let store = Store::from_env();
    if store.is_writable() {
        Some(store)
    } else {
        reporter.event(
            "firewall_store_read_only",
            "this process is not elevated; firewall rules are neither created nor removed",
        );
        None
    }
}

/// Uninstalls from the database: recover an open transaction, then remove
/// what is owned; an absent installation is the desired state already.
pub fn uninstall(package: &Package, options: &RunOptions, sink: &mut dyn EventSink) -> Outcome {
    run(
        "uninstall",
        package,
        options,
        sink,
        |package, options, roots, reporter| {
            prepare_state_directory(roots, options, reporter)?;
            let db = Db::open_rw(&roots.state_db)?;
            let mut fault =
                FaultInjector::with_signal(options.faults.clone(), options.fault_signal.clone());
            let recovery = recovery::recover(
                &db,
                package,
                &roots.state_dir,
                options.quiet,
                &mut fault,
                reporter,
            )?;

            let Some(existing) = installation::read(&db)? else {
                reporter.event("not_installed", format!("package={}", package.id()));
                let message = human(
                    options,
                    "outcome.not_installed",
                    &[("name", package.name())],
                );
                let mut outcome = Outcome::new("not_installed", "not_installed", exit::OK, message);
                with_recovery(&mut outcome, recovery);
                drop(db);
                remove_state_directory(package, options, roots, reporter);
                return Ok(outcome);
            };

            let install_root = PathBuf::from(&existing.install_root);
            let owned = installation::owned(&db, &existing)?;
            let locations = scope::locations(options.scope);
            locations.confine_owned(&owned)?;
            let shortcut_folders = locations.shortcut_folders()?;
            let firewall = firewall_store(reporter);
            // The uninstall actions are the installation's own, planned
            // from the database like everything else it owns, and gated by
            // the options it recorded.
            let actions = action::uninstall_plan(&owned, &installation::options(&db)?)?;
            report_skipped_actions(&actions, reporter);
            let planning = Instant::now();
            // The package's batches are the ones its files were installed
            // by, which the uninstaller carries; an owned file the package
            // does not know goes after them.
            let file_batches = plan::FileBatchIndex::of(package.metadata());
            let planned = plan::reconcile(plan::Reconcile {
                desired: None,
                owned: &owned,
                install_root: &install_root,
                payload: None,
                roots: &winreg::Roots::from_env(),
                scope: options.scope,
                inspect_files: true,
                repair: false,
                shortcut_folders: &shortcut_folders,
                firewall: firewall.as_ref(),
                actions: &actions,
                file_batches: &file_batches,
            })?;
            reporter.event(
                "plan_completed",
                format!(
                    "operations={} ms={}",
                    planned.operations.len(),
                    planning.elapsed().as_millis()
                ),
            );
            for finding in &planned.findings {
                reporter.event(finding.code, finding.path.clone().unwrap_or_default());
            }
            let mut txn = new_transaction(
                package,
                roots,
                options,
                TxnKind::Uninstall,
                Some(&existing.version),
            );
            txn.install_root = existing.install_root.clone();
            txn.registration_key = existing.registration_key.clone();

            // The installation's own quiescence, then the Restart Manager;
            // an uninstall that commits never resumes what it stopped, a
            // failed one does.
            let quiescence_context = action::Context {
                install_root: install_root.clone(),
                version: existing.version.clone(),
                product_id: package.id().to_string(),
                scope: options.scope,
                operation: tigersetup_format::metadata::ActionOperation::Uninstall,
                quiet: options.quiet,
            };
            let entries =
                quiescence::uninstall_plan(&owned, &installation::options(&db)?, reporter)?;
            let mut quiesced = quiescence::quiesce(
                &mut quiescence::Site {
                    db: &db,
                    state_dir: &roots.state_dir,
                    payload: None,
                    transaction_id: &txn.id,
                    context: quiescence_context.clone(),
                    reporter,
                },
                &entries,
            )?;
            let transaction_id = txn.id.clone();
            let outcome = run_uninstall_transaction(
                package,
                options,
                reporter,
                &db,
                &mut fault,
                roots,
                txn,
                &planned,
                &install_root,
            );
            let mut outcome = match outcome {
                Ok(outcome) => outcome,
                Err(err) => {
                    quiesced.resume(&db, &transaction_id, &quiescence_context, reporter);
                    return Err(err);
                }
            };
            quiesced.settle(
                &mut outcome,
                true,
                &db,
                &transaction_id,
                &quiescence_context,
                reporter,
            );
            with_recovery(&mut outcome, recovery);
            if outcome.outcome == "installed" {
                outcome.outcome = "uninstalled";
                outcome.message = human(
                    options,
                    "outcome.uninstalled",
                    &[
                        ("name", package.name()),
                        ("version", &existing.version),
                        ("root", &existing.install_root),
                    ],
                );
                drop(db);
                remove_state_directory(package, options, roots, reporter);
            }
            Ok(outcome)
        },
    )
}

/// The transaction of an uninstall, from the Restart Manager to the
/// outcome: everything a quiescence settlement wraps. The outcome still
/// reads `installed` for a committed uninstall; the caller names it.
#[allow(clippy::too_many_arguments)]
fn run_uninstall_transaction(
    package: &Package,
    options: &RunOptions,
    reporter: &mut Reporter<'_>,
    db: &Db,
    fault: &mut FaultInjector,
    roots: &Roots,
    txn: TransactionRow,
    planned: &plan::Plan,
    install_root: &Path,
) -> Result<Outcome> {
    let quiescence = restart::Quiescence::acquire(
        &restart::files_at_risk(&planned.operations, install_root),
        options.quiet,
        reporter,
    )?;
    journal::begin(db, &txn, &planned.operations, &BTreeMap::new())?;
    reporter.event(
                "transaction_started",
                format!(
                    "transaction={} kind=uninstall from={} operations={} removed={} directories_removed={} resources={} actions={}",
                    txn.id,
                    txn.from_version.as_deref().unwrap_or("-"),
                    planned.operations.len(),
                    planned.counts.removed,
                    planned.counts.directories_removed,
                    planned.counts.resource_operations,
                    planned.counts.actions
                ),
            );
    let mut executor = Executor::new(db, txn, &roots.state_dir, None, fault, reporter)
        .cancellable(options.cancel.clone())
        .unattended(options.quiet);
    let result = executor
        .run_forward(false)
        .and_then(|_| executor.commit("", ENGINE_VERSION));
    let mut outcome = finish(&mut executor, result, options, package)?;
    drop(executor);
    outcome.closed_applications = quiescence.closed().to_vec();
    quiescence.release(reporter);
    let mut findings = planned.findings.clone();
    findings.append(&mut outcome.findings);
    outcome.findings = findings;
    Ok(outcome)
}

/// Removes the whole state directory — database, logs, staging and the
/// uninstaller copy — once an uninstall has committed, whichever executable
/// ran it, so an uninstalled product leaves nothing of TigerSetup's behind.
/// The one complication is the uninstaller copy removing the directory it
/// lives in, which is why a running origin is moved aside first.
fn remove_state_directory(
    package: &Package,
    options: &RunOptions,
    roots: &Roots,
    reporter: &mut Reporter<'_>,
) {
    let origin = options
        .relaunched_from
        .as_deref()
        .unwrap_or_else(|| package.installer().path());
    // The executable this process was copied from is still running: Windows
    // lets it be renamed but not deleted, so it moves out of the directory
    // first and the client schedules it for deletion afterwards. An
    // installer that uninstalls runs from somewhere else entirely, and then
    // nothing in the directory is held open.
    if origin.starts_with(&roots.state_dir) && origin.exists() {
        let aside = origin_aside_path(package.installer().path());
        let moved = win::fs::create_directory(&temp_directory())
            .map_err(|err| err.message)
            .and_then(|()| std::fs::rename(origin, &aside).map_err(|err| err.to_string()));
        match moved {
            Ok(()) => reporter.event("uninstaller_moved_aside", aside.display().to_string()),
            Err(err) => {
                reporter.event(
                    "state_directory_cleanup_failed",
                    format!("{}: {err}", origin.display()),
                );
                return;
            }
        }
    }
    match std::fs::remove_dir_all(&roots.state_dir) {
        Ok(()) => reporter.event(
            "state_directory_removed",
            roots.state_dir.display().to_string(),
        ),
        Err(err) => reporter.event(
            "state_directory_cleanup_failed",
            format!("{}: {err}", roots.state_dir.display()),
        ),
    }
}

/// Turns the result of the forward walk and commit into an outcome, rolling
/// back on failure.
fn finish(
    executor: &mut Executor<'_, '_>,
    result: Result<()>,
    options: &RunOptions,
    package: &Package,
) -> Result<Outcome> {
    let name = package.name();
    let version = package.version();
    let outcome: Result<Outcome> = match result {
        Ok(()) => {
            executor.cleanup();
            let root = executor.transaction().install_root.clone();
            let message = human(
                options,
                "outcome.installed",
                &[("name", name), ("version", version), ("root", &root)],
            );
            let mut outcome = Outcome::new("installed", "ok", exit::OK, message);
            outcome.transaction = Some(transaction_info(executor.transaction(), None));
            Ok(outcome)
        }
        Err(err) => {
            executor
                .reporter
                .event("transaction_failed", err.to_string());
            match executor.rollback() {
                Ok(_) => {
                    executor.cleanup();
                    let cancelled = err.code == "cancelled";
                    let message = human(
                        options,
                        if cancelled {
                            "outcome.cancelled"
                        } else {
                            "outcome.rolled_back"
                        },
                        &[
                            ("name", name),
                            ("version", version),
                            ("reason", &err.message),
                        ],
                    );
                    let mut outcome = if cancelled {
                        Outcome::new("cancelled", err.code, exit::CANCELLED, message)
                    } else {
                        Outcome::new("rolled_back", err.code, exit::ROLLED_BACK, message)
                    };
                    outcome.transaction = Some(transaction_info(executor.transaction(), None));
                    Ok(outcome)
                }
                Err(rollback_err) => {
                    let message = human(
                        options,
                        "outcome.failed",
                        &[
                            ("name", name),
                            ("version", version),
                            (
                                "reason",
                                &format!("{}; {}", err.message, rollback_err.message),
                            ),
                        ],
                    );
                    let mut outcome = Outcome::new(
                        "failed",
                        rollback_err.code,
                        exit::RECOVERY_INCOMPLETE,
                        message,
                    );
                    outcome.transaction = Some(transaction_info(executor.transaction(), None));
                    Ok(outcome)
                }
            }
        }
    };
    let mut outcome = outcome?;
    outcome.findings = executor.take_findings();
    let (actions, reboot_required) = executor.take_actions();
    outcome.actions = actions;
    if reboot_required {
        outcome.reboot_required = true;
        if outcome.exit_code == exit::OK {
            outcome.exit_code = exit::REBOOT_REQUIRED;
        }
    }
    Ok(outcome)
}

/// Logs the declared actions a run leaves out, and why.
fn report_skipped_actions(actions: &action::ActionPlan, reporter: &mut Reporter<'_>) {
    for (name, why) in &actions.skipped {
        reporter.event(
            "action_not_required",
            format!(
                "{name}: {}",
                match *why {
                    "option" => "disabled by its option",
                    _ => "not declared for this operation",
                }
            ),
        );
    }
}

/// Whether a failure to open the state database is one a read-only command
/// reports as a finding rather than as an error of its own.
fn unreadable_state(err: &Error) -> bool {
    matches!(err.code, "database_busy" | "state_unreadable")
}

/// Compares the owned state in the database with the machine. Never mutates,
/// so it reports what it observes — `<resource>_missing` and
/// `<resource>_modified` — and leaves every decision about what to do to a
/// mutating run.
pub fn verify(package: &Package, options: &RunOptions) -> Result<VerifyReport> {
    let roots = resolve_roots(package, options)?;
    let mut report = VerifyReport {
        schema: report::SCHEMA,
        status: "not_installed",
        installation: None,
        transaction: None,
        findings: Vec::new(),
        counts: VerifyCounts::default(),
        integrations: Vec::new(),
    };
    let db = match Db::open_ro(&roots.state_db) {
        Ok(Some(db)) => db,
        Ok(None) => return Ok(report),
        // Reading never elevates and never fails the caller with an error
        // it cannot act on: a database another run holds, or one this
        // process may not read, is reported as a finding.
        Err(err) if unreadable_state(&err) => {
            report.status = "failed";
            report.findings.push(Finding::plain(err.code, err.message));
            return Ok(report);
        }
        Err(err) => return Err(err),
    };
    if let Some(txn) = journal::open_transaction(&db)? {
        report.status = "transaction_open";
        report.transaction = Some(transaction_info(
            &txn,
            Some(recovery::direction(&txn, package)),
        ));
        if let Some(row) = installation::read(&db)? {
            report.installation = Some(installation_info(&db, &row, &roots.state_db)?);
        }
        return Ok(report);
    }
    let Some(row) = installation::read(&db)? else {
        return Ok(report);
    };
    report.installation = Some(installation_info(&db, &row, &roots.state_db)?);
    let install_root = PathBuf::from(&row.install_root);
    let owned = installation::owned(&db, &row)?;

    for file in &owned.files {
        report.counts.files_checked += 1;
        let target = match plan::absolute(&install_root, &file.path) {
            Ok(target) => target,
            Err(err) => {
                report.findings.push(Finding {
                    code: "path_outside_root",
                    path: Some(file.path.clone()),
                    detail: Some(err.message),
                });
                continue;
            }
        };
        match win::fs::inspect(&target)? {
            win::fs::Inspection::Absent => {
                report.findings.push(Finding::at("file_missing", &target))
            }
            win::fs::Inspection::Present { sha256, .. } if sha256 != file.sha256 => {
                report.findings.push(Finding::at("file_modified", &target));
            }
            win::fs::Inspection::Present { .. } => report.counts.files_ok += 1,
        }
    }
    for directory in &owned.directories {
        report.counts.directories_checked += 1;
        let target = match plan::absolute(&install_root, &directory.path) {
            Ok(target) => target,
            Err(err) => {
                report.findings.push(Finding {
                    code: "path_outside_root",
                    path: Some(directory.path.clone()),
                    detail: Some(err.message),
                });
                continue;
            }
        };
        if resource::directory::exists(&target) {
            report.counts.directories_ok += 1;
        } else {
            report
                .findings
                .push(Finding::at("directory_missing", &target));
        }
    }

    let registry_roots = winreg::Roots::from_env();
    if let Some(key) = &owned.registration_key
        && let Ok(parsed) = KeyPath::parse(key)
        && !winreg::key_exists(&registry_roots, &parsed)?
    {
        report
            .findings
            .push(Finding::named("registration_missing", key.clone()));
    }
    for value in &owned.registry_values {
        let registration =
            resource::registry::is_registration_key(&value.key, owned.registration_key.as_deref());
        let (checked, ok, missing, modified) = if registration {
            (
                &mut report.counts.registration_values_checked,
                &mut report.counts.registration_values_ok,
                "registration_missing",
                "registration_modified",
            )
        } else {
            (
                &mut report.counts.registry_values_checked,
                &mut report.counts.registry_values_ok,
                "registry_value_missing",
                "registry_value_modified",
            )
        };
        *checked += 1;
        let location = format!("{}\\{}", value.key, value.name);
        let Ok(key) = KeyPath::parse(&value.key) else {
            report.findings.push(Finding::named(missing, location));
            continue;
        };
        let recorded = resource::registry::owned_data(value)?;
        match winreg::read_value(&registry_roots, &key, &value.name)? {
            None => report.findings.push(Finding::named(missing, location)),
            Some(current) if current == recorded => *ok += 1,
            Some(_) => report.findings.push(Finding::named(modified, location)),
        }
    }
    for entry in &owned.path_entries {
        if !entry.added {
            continue;
        }
        report.counts.path_entries_checked += 1;
        let location = format!(
            "{}\\{} {}",
            entry.hive_key,
            resource::path::VALUE_NAME,
            entry.raw
        );
        let Ok(key) = KeyPath::parse(&entry.hive_key) else {
            report
                .findings
                .push(Finding::named("path_entry_missing", location));
            continue;
        };
        let (text, _) = resource::path::read(&registry_roots, &key)?;
        if resource::path::find_owned(&text, &entry.raw, &entry.normalized).is_some() {
            report.counts.path_entries_ok += 1;
        } else {
            report
                .findings
                .push(Finding::named("path_entry_missing", location));
        }
    }
    for link in &owned.shortcuts {
        report.counts.shortcuts_checked += 1;
        let path = resource::shortcut::link_path(&link.path)?;
        match win::shortcut::inspect(&path)? {
            win::shortcut::LinkInspection::Absent => {
                report.findings.push(Finding::at("shortcut_missing", &path))
            }
            win::shortcut::LinkInspection::Link(current)
                if current.target.eq_ignore_ascii_case(&link.target) =>
            {
                report.counts.shortcuts_ok += 1
            }
            _ => report
                .findings
                .push(Finding::at("shortcut_modified", &path)),
        }
    }
    for variable in &owned.environment_variables {
        report.counts.environment_variables_checked += 1;
        let location = resource::environment::location(&variable.hive_key, &variable.name);
        let Ok(key) = KeyPath::parse(&variable.hive_key) else {
            report
                .findings
                .push(Finding::named("environment_variable_missing", location));
            continue;
        };
        let written = resource::environment::owned_data(variable)?;
        match winreg::read_value(&registry_roots, &key, &variable.name)? {
            None => report
                .findings
                .push(Finding::named("environment_variable_missing", location)),
            Some(current) if current == written => report.counts.environment_variables_ok += 1,
            Some(_) => report
                .findings
                .push(Finding::named("environment_variable_modified", location)),
        }
    }
    let firewall = Store::from_env();
    for rule in &owned.firewall_rules {
        report.counts.firewall_rules_checked += 1;
        let recorded = resource::firewall::rule_of(Some(&rule.rule), "owned firewall rule")?;
        match firewall.list(&rule.name)?.as_slice() {
            [] => report
                .findings
                .push(Finding::named("firewall_rule_missing", rule.name.clone())),
            [current] if resource::firewall::matches(current, &recorded) => {
                report.counts.firewall_rules_ok += 1
            }
            _ => report
                .findings
                .push(Finding::named("firewall_rule_modified", rule.name.clone())),
        }
    }
    for record in &owned.actions {
        let (Some(sha256), Some(file_name)) = (&record.artifact_sha256, &record.artifact_file)
        else {
            continue;
        };
        report.counts.action_programs_checked += 1;
        let program = action::store_dir(&roots.state_dir, sha256).join(file_name);
        match win::fs::inspect(&program)? {
            win::fs::Inspection::Absent => report
                .findings
                .push(Finding::at("action_program_missing", &program)),
            win::fs::Inspection::Present { sha256: actual, .. } if &actual != sha256 => report
                .findings
                .push(Finding::at("action_program_modified", &program)),
            win::fs::Inspection::Present { .. } => report.counts.action_programs_ok += 1,
        }
    }
    report.integrations = integration_statuses(
        package,
        &row,
        &owned,
        &installation::options(&db)?,
        &registry_roots,
    )?;

    report.status = if report.findings.is_empty() {
        "ok"
    } else {
        "failed"
    };
    Ok(report)
}

/// The declared integrations as this machine holds them for an installed
/// product: compiled against the recorded options and the recorded install
/// root, then read back. `enabled` is what the options say; the counts are
/// what the registry says.
fn integration_statuses(
    package: &Package,
    row: &InstallationRow,
    owned: &Owned,
    recorded: &Options,
    registry_roots: &winreg::Roots,
) -> Result<Vec<IntegrationStatus>> {
    let metadata = package.metadata();
    if metadata.file_associations.is_empty()
        && metadata.url_protocols.is_empty()
        && metadata.app_paths.is_empty()
        && metadata.context_menu_verbs.is_empty()
    {
        return Ok(Vec::new());
    }
    let Some(scope) = Scope::parse(&row.scope) else {
        return Ok(Vec::new());
    };
    // The recorded options completed with the package's defaults, so an
    // option a newer package added reads as its default.
    let effective = plan::effective_options(metadata, recorded, &Options::new())?;
    let locations = scope::locations(scope);
    let compiled = resource::integration::desired(
        metadata,
        &effective,
        &locations,
        Path::new(&row.install_root),
        registry_roots,
        owned,
    )?;
    compiled
        .items
        .iter()
        .map(|item| {
            let presence = resource::integration::presence(item, registry_roots, owned)?;
            Ok(IntegrationStatus {
                kind: item.kind,
                id: item.id.clone(),
                enabled: item.enabled,
                values_total: presence.values_total,
                values_present: presence.values_present,
                values_owned: presence.values_owned,
            })
        })
        .collect()
}

/// A recorded option map as a report carries it: booleans as booleans,
/// choices as strings.
fn options_json(options: &Options) -> BTreeMap<String, serde_json::Value> {
    options
        .iter()
        .map(|(name, value)| {
            let json = match value {
                OptionValue::Bool(b) => serde_json::Value::Bool(*b),
                OptionValue::Choice(c) => serde_json::Value::String(c.clone()),
            };
            (name.clone(), json)
        })
        .collect()
}

/// Describes the package and whatever the machine holds for it. Never
/// mutates.
pub fn inspect(package: &Package, options: &RunOptions) -> Result<InspectReport> {
    let mut report = InspectReport {
        schema: report::SCHEMA,
        package: package.info()?,
        installation: None,
        owned: None,
        transaction: None,
        installations: target::existing(package)?,
        dependencies: dependency::statuses(package)?,
        integrations: Vec::new(),
        findings: Vec::new(),
    };
    let roots = resolve_roots(package, options)?;
    let registry_roots = winreg::Roots::from_env();
    match Db::open_ro(&roots.state_db) {
        Ok(Some(db)) => {
            if let Some(row) = installation::read(&db)? {
                report.installation = Some(installation_info(&db, &row, &roots.state_db)?);
                let owned = installation::owned(&db, &row)?;
                let recorded = installation::options(&db)?;
                report.integrations =
                    integration_statuses(package, &row, &owned, &recorded, &registry_roots)?;
                report.owned = Some(OwnedResources {
                    options: options_json(&recorded),
                    registration_key: owned.registration_key.clone(),
                    registry_values: owned
                        .registry_values
                        .iter()
                        .map(|v| format!("{}\\{}", v.key, v.name))
                        .collect(),
                    path_entries: owned
                        .path_entries
                        .iter()
                        .map(|e| PathEntryInfo {
                            hive_key: e.hive_key.clone(),
                            entry: e.raw.clone(),
                            pre_existed: e.pre_existed,
                        })
                        .collect(),
                    shortcuts: owned.shortcuts.iter().map(|s| s.path.clone()).collect(),
                    environment_variables: owned
                        .environment_variables
                        .iter()
                        .map(|v| EnvironmentVariableInfo {
                            hive_key: v.hive_key.clone(),
                            name: v.name.clone(),
                            pre_existed: v.pre_existed,
                        })
                        .collect(),
                    firewall_rules: owned
                        .firewall_rules
                        .iter()
                        .map(|r| r.name.clone())
                        .collect(),
                    actions: owned
                        .actions
                        .iter()
                        .map(|a| OwnedActionInfo {
                            name: a.name.clone(),
                            phase: a.phase.clone(),
                            sha256: a.artifact_sha256.clone(),
                            file_name: a.artifact_file.clone(),
                        })
                        .collect(),
                });
            }
            if let Some(txn) = journal::open_transaction(&db)? {
                report.transaction = Some(transaction_info(
                    &txn,
                    Some(recovery::direction(&txn, package)),
                ));
            }
        }
        Ok(None) => {}
        Err(err) if unreadable_state(&err) => {
            report.findings.push(Finding::plain(err.code, err.message))
        }
        Err(err) => return Err(err),
    }
    Ok(report)
}

#[cfg(test)]
mod staging_tests {
    /// Unelevated, staging is the user's own `%TEMP%\TigerSetup`, which is
    /// right there: the process and the folder belong to the same person.
    /// The elevated branch cannot be exercised here — it needs an
    /// administrator — so the lab owns that half.
    #[test]
    fn an_unelevated_run_stages_in_its_own_temp_folder() {
        if super::elevation::is_elevated() {
            eprintln!("skipped: this process is elevated");
            return;
        }
        let staging = super::staging_directory().unwrap();
        assert_eq!(staging, super::temp_directory());
        assert!(staging.is_dir());
    }
}
