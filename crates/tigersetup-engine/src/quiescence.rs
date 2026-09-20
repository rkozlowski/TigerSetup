//! Application quiescence a package declares (`TigerSetup-Design.md`
//! §5.10): stopping an application the Restart Manager cannot close, before
//! the Restart Manager is asked, and starting it again once the run is
//! over.
//!
//! The Restart Manager closes an application by messaging its windows, so a
//! process with no window to message — a tray helper, a service-like
//! background process, a detached worker — is listed as a holder and never
//! closed, and every upgrade of such a product ends `package_in_use`. A
//! `[[quiescence]]` entry is the package's own answer: a **stop** program,
//! run with the custom action's envelope before the Restart Manager check,
//! whose exit code says what it found — running and now stopped, or not
//! running — and an optional **resume** program, started detached once the
//! run has ended with the product on the machine.
//!
//! The lifecycle is explicit, and it is what makes this more than a
//! pre-install action moved earlier:
//!
//! - **Only what was stopped is resumed.** An application that was not
//!   running stays not running; the stop program's `not_running_codes`
//!   are how it says so.
//! - **A failure past the stop resumes.** Whether the Restart Manager then
//!   finds another holder that will not close, a dependency cannot be
//!   acquired, or the transaction rolls back, the application is started
//!   again before the failure is reported — a refusal never leaves it
//!   stopped for nothing.
//! - **An uninstall stops and never resumes;** a rolled-back uninstall
//!   resumes, because the product is still there.
//! - **A run that leaves its transaction open does not resume:** the
//!   product's files are in no state to run from, and the recovery that
//!   settles them is the run that should decide.
//! - **Nothing is claimed about the stop program's side effects.** What it
//!   did to the machine is the package author's, exactly as a custom
//!   action's is; TigerSetup records that it ran and what it reported.
//!
//! An installing run uses the package's own entries; an uninstall uses the
//! entries the installation recorded when it was installed (the `action`
//! ownership table, phase `quiesce`), with the packaged programs kept in
//! the state directory's action store, because the installer that brought
//! the product is usually gone by then. Packaged programs of an installing
//! run are extracted into that same content-addressed store, where the
//! transaction's `store_action` operation finds them already in place.

use std::path::{Path, PathBuf};

use tigersetup_format::Payload;
use tigersetup_format::metadata::{Action, ActionOperation, ActionPhase, Quiescence};

use crate::action::{self, Context, Status};
use crate::report::{ActionInfo, Finding, Outcome, Reporter};
use crate::resource::file;
use crate::resource::predicate::{self, Options};
use crate::state::Db;
use crate::state::action as runs;
use crate::state::installation::Owned;
use crate::state::journal::TxnKind;
use crate::win::process::{self, Show};
use crate::{Error, Result};

/// The finding a stopped application is reported with.
pub const STOPPED: &str = "quiescence_stopped";
/// The finding an application that was not running is reported with.
pub const NOT_RUNNING: &str = "quiescence_not_running";
/// The finding a resumed application is reported with.
pub const RESUMED: &str = "quiescence_resumed";
/// The finding a resume program that could not be started is reported with.
pub const RESUME_FAILED: &str = "quiescence_resume_failed";
/// The finding for an application left stopped because the run left its
/// transaction open.
pub const NOT_RESUMED: &str = "quiescence_not_resumed";
/// The finding a failed stop program with `on_failure = continue` is
/// reported with.
pub const FAILED_CONTINUED: &str = "quiescence_failed_continued";
/// The stable code a failed stop program stops the run with.
pub const FAILED: &str = "quiescence_failed";

/// The entries one run executes, in declaration order.
pub fn install_plan(
    metadata: &tigersetup_format::Metadata,
    options: &Options,
    kind: TxnKind,
    reporter: &mut Reporter<'_>,
) -> Vec<Quiescence> {
    select(
        metadata.quiescence.iter().cloned(),
        action::operation_of(kind),
        options,
        reporter,
    )
}

/// The entries an uninstall executes: the installation's own, gated by the
/// options it recorded.
pub fn uninstall_plan(
    owned: &Owned,
    options: &Options,
    reporter: &mut Reporter<'_>,
) -> Result<Vec<Quiescence>> {
    let mut entries = Vec::new();
    for record in &owned.actions {
        if record.phase == ActionPhase::Quiesce.as_str() {
            entries.push(action::deserialize_quiescence(&record.definition)?);
        }
    }
    Ok(select(
        entries.into_iter(),
        ActionOperation::Uninstall,
        options,
        reporter,
    ))
}

fn select(
    entries: impl Iterator<Item = Quiescence>,
    operation: ActionOperation,
    options: &Options,
    reporter: &mut Reporter<'_>,
) -> Vec<Quiescence> {
    let mut selected = Vec::new();
    for entry in entries {
        if !entry.runs_on(operation) {
            reporter.event(
                "quiescence_skipped",
                format!("{}: not on {}", entry.name, operation.as_str()),
            );
            continue;
        }
        if !predicate::enabled(entry.when.as_ref(), "", options) {
            reporter.event(
                "quiescence_skipped",
                format!(
                    "{}: disabled by option {}",
                    entry.name,
                    entry
                        .when
                        .as_ref()
                        .map(|w| w.describe())
                        .unwrap_or_default()
                ),
            );
            continue;
        }
        selected.push(entry);
    }
    selected
}

/// Where a run finds the programs and records what it did.
pub struct Site<'a, 'r> {
    pub db: &'a Db,
    pub state_dir: &'a Path,
    /// The payload of an installing run, which packaged programs are
    /// extracted from; an uninstall has none and reads the store.
    pub payload: Option<&'a mut Payload>,
    pub transaction_id: &'a str,
    pub context: Context,
    pub reporter: &'a mut Reporter<'r>,
}

/// One entry whose stop program reported the application running and
/// stopped: what the resume must start.
struct Stopped {
    entry: Quiescence,
    resume_program: Option<PathBuf>,
}

/// What a run stopped, and therefore owes a restart to.
pub struct Quiesced {
    stopped: Vec<Stopped>,
    /// Everything the stop programs produced, for the outcome document.
    pub actions: Vec<ActionInfo>,
    pub findings: Vec<Finding>,
    settled: bool,
}

impl Quiesced {
    /// Nothing was stopped, nothing is owed.
    pub fn none() -> Quiesced {
        Quiesced {
            stopped: Vec::new(),
            actions: Vec::new(),
            findings: Vec::new(),
            settled: true,
        }
    }

    /// Whether anything was stopped.
    pub fn stopped_anything(&self) -> bool {
        !self.stopped.is_empty()
    }
}

/// Runs every entry's stop program, in order. The stop of an entry that
/// fails — by exit code, timeout or launch — with `on_failure = fail` ends
/// the run with `quiescence_failed`, after the entries already stopped
/// have been resumed; with `continue`, the failure is recorded and the
/// run goes on to the Restart Manager.
pub fn quiesce(site: &mut Site<'_, '_>, entries: &[Quiescence]) -> Result<Quiesced> {
    let mut quiesced = Quiesced {
        stopped: Vec::new(),
        actions: Vec::new(),
        findings: Vec::new(),
        settled: entries.is_empty(),
    };
    for entry in entries {
        if let Err(err) = stop_one(site, entry, &mut quiesced) {
            site.reporter.event(
                "quiescence_failed",
                format!("{}: {}", entry.name, err.message),
            );
            quiesced.resume(site.db, site.transaction_id, &site.context, site.reporter);
            return Err(err);
        }
    }
    Ok(quiesced)
}

fn stop_one(site: &mut Site<'_, '_>, entry: &Quiescence, quiesced: &mut Quiesced) -> Result<()> {
    let stop = entry.stop();
    let program = program_of(site, stop)?;
    let launch = action::launch_of(stop, &program, &site.context);
    let run = runs::start(
        site.db,
        site.transaction_id,
        0,
        &entry.name,
        ActionPhase::Quiesce.as_str(),
        site.context.operation.as_str(),
        stop.kind().as_str(),
        &program.display().to_string(),
        stop.failure_policy().as_str(),
    )?;
    site.reporter.event(
        "quiescence_started",
        format!(
            "{} ({}): {} [timeout {} s, on failure {}, not running {:?}]",
            entry.name,
            site.context.operation.as_str(),
            action::describe_launch(&launch),
            stop.timeout_seconds(),
            stop.failure_policy().as_str(),
            entry.not_running_codes
        ),
    );
    let started = std::time::Instant::now();
    let mut verdict = match process::run_captured(&launch) {
        Ok(captured) => action::judge(stop, &captured),
        Err(err) => {
            site.reporter
                .event("quiescence_launch_failed", format!("{}: {err}", entry.name));
            action::launch_failed(started.elapsed().as_millis() as u64)
        }
    };
    // A "not running" code is a completed stop that stopped nothing.
    let not_running = verdict
        .exit_code
        .is_some_and(|code| entry.means_not_running(code));
    if not_running {
        verdict.status = Status::Completed;
    }
    runs::finish(
        site.db,
        run,
        verdict.status.as_str(),
        verdict.exit_code,
        false,
    )?;
    for (stream, text) in [("stdout", &verdict.stdout), ("stderr", &verdict.stderr)] {
        for line in text.lines().take(action::LOG_LINES) {
            site.reporter.event(
                "quiescence_output",
                format!("{} {stream}: {line}", entry.name),
            );
        }
    }
    let continues = action::continues(stop, &verdict);
    let (status, code) = if verdict.status == Status::Completed {
        ("completed", None)
    } else if continues {
        ("failed_continued", Some(FAILED))
    } else {
        (verdict.status.as_str(), Some(FAILED))
    };
    quiesced.actions.push(ActionInfo {
        name: entry.name.clone(),
        phase: ActionPhase::Quiesce.as_str(),
        operation: site.context.operation.as_str(),
        kind: stop.kind().as_str(),
        program: program.display().to_string(),
        status,
        exit_code: verdict.exit_code,
        reboot_required: false,
        duration_ms: verdict.duration_ms,
        timeout_seconds: stop.timeout_seconds(),
        on_failure: stop.failure_policy().as_str(),
        code,
        stdout: action::tail(&verdict.stdout),
        stderr: action::tail(&verdict.stderr),
        sha256: stop.is_packaged().then(|| stop.sha256.clone()),
    });
    match verdict.status {
        Status::Completed if not_running => {
            site.reporter.event(
                NOT_RUNNING,
                format!(
                    "{}: exit code {} means nothing was running",
                    entry.name,
                    verdict.exit_code.unwrap_or_default()
                ),
            );
            quiesced
                .findings
                .push(Finding::named(NOT_RUNNING, entry.name.clone()));
        }
        Status::Completed => {
            site.reporter.event(
                STOPPED,
                format!(
                    "{}: stopped (exit code {}, {} ms)",
                    entry.name,
                    verdict.exit_code.unwrap_or_default(),
                    verdict.duration_ms
                ),
            );
            quiesced
                .findings
                .push(Finding::named(STOPPED, entry.name.clone()));
            // The resume program is resolved now, while the payload is at
            // hand, so that a resume after a rolled-back transaction does
            // not depend on what the rollback left.
            let resume_program = match &entry.resume {
                Some(resume) => Some(program_of(site, resume)?),
                None => None,
            };
            quiesced.stopped.push(Stopped {
                entry: entry.clone(),
                resume_program,
            });
        }
        _ if continues => {
            site.reporter.event(
                FAILED_CONTINUED,
                format!(
                    "{}: {}",
                    entry.name,
                    action::failure_message(stop, &verdict)
                ),
            );
            quiesced
                .findings
                .push(Finding::named(FAILED_CONTINUED, entry.name.clone()));
        }
        _ => {
            return Err(Error::new(
                FAILED,
                format!(
                    "quiescence {}: {}",
                    entry.name,
                    action::failure_message(stop, &verdict)
                ),
            ));
        }
    }
    Ok(())
}

/// The program of a stop or resume action, on disk and verified: a
/// packaged one extracted from the payload into the action store (an
/// installing run) or found there (an uninstall); a command expanded.
fn program_of(site: &mut Site<'_, '_>, program: &Action) -> Result<PathBuf> {
    if !program.is_packaged() {
        return Ok(PathBuf::from(action::expand(
            &program.command,
            &site.context,
        )));
    }
    let path = action::stored_program(site.state_dir, program);
    if !file::matches(&path, &program.sha256)? {
        let Some(payload) = site.payload.as_deref_mut() else {
            return Err(Error::new(
                if path.exists() {
                    "action_program_mismatch"
                } else {
                    "action_program_missing"
                },
                format!(
                    "the program of quiescence {} is not in the state directory as recorded: {}",
                    program.name,
                    path.display()
                ),
            ));
        };
        if let Some(parent) = path.parent() {
            crate::win::fs::create_directory(parent)?;
        }
        let mut staged =
            file::stage_from_payload(&path, payload, &program.entry, Some(program.size))?;
        if staged.sha256 != program.sha256 {
            return Err(Error::new(
                "action_program_mismatch",
                format!(
                    "the packaged program of quiescence {} has SHA-256 {}, the package recorded {}",
                    program.name, staged.sha256, program.sha256
                ),
            ));
        }
        staged.flush()?;
        staged.commit()?;
    }
    Ok(path)
}

impl Quiesced {
    /// Settles what the run owes once its outcome is known: resumes what
    /// was stopped when the product is on the machine to run — the run
    /// installed, or rolled back to what was there — and never after an
    /// uninstall that completed or a run that left its transaction open.
    /// Appends its evidence to the outcome.
    pub fn settle(
        mut self,
        outcome: &mut Outcome,
        uninstall: bool,
        db: &Db,
        transaction_id: &str,
        context: &Context,
        reporter: &mut Reporter<'_>,
    ) {
        let transaction_open = outcome
            .transaction
            .as_ref()
            .is_some_and(|txn| txn.state != "committed" && txn.state != "rolled_back");
        // An uninstall that committed reports `installed` until its caller
        // renames the outcome, so the kind of run says what a commit meant.
        let product_present =
            !(uninstall && outcome.outcome == "installed") && outcome.outcome != "uninstalled";
        if self.stopped_anything() {
            if transaction_open {
                for stopped in &self.stopped {
                    reporter.event(
                        NOT_RESUMED,
                        format!(
                            "{}: the transaction is still open; not restarted",
                            stopped.entry.name
                        ),
                    );
                    self.findings
                        .push(Finding::named(NOT_RESUMED, stopped.entry.name.clone()));
                }
                self.stopped.clear();
                self.settled = true;
            } else if product_present {
                self.resume(db, transaction_id, context, reporter);
            } else {
                // Uninstalled: the application has nothing to come back
                // to, by design.
                self.stopped.clear();
                self.settled = true;
            }
        }
        outcome.actions.append(&mut self.actions);
        outcome.findings.append(&mut self.findings);
    }

    /// Starts the resume program of everything stopped, detached, and
    /// records each start. Idempotent: what was resumed is forgotten.
    pub fn resume(
        &mut self,
        db: &Db,
        transaction_id: &str,
        context: &Context,
        reporter: &mut Reporter<'_>,
    ) {
        for stopped in std::mem::take(&mut self.stopped) {
            let Some(resume) = &stopped.entry.resume else {
                reporter.event(
                    "quiescence_no_resume",
                    format!("{}: no resume program declared", stopped.entry.name),
                );
                continue;
            };
            let Some(program) = stopped.resume_program else {
                continue;
            };
            let launch = action::launch_of(resume, &program, context);
            let run = runs::start(
                db,
                transaction_id,
                0,
                &stopped.entry.name,
                ActionPhase::Resume.as_str(),
                context.operation.as_str(),
                resume.kind().as_str(),
                &program.display().to_string(),
                resume.failure_policy().as_str(),
            );
            let started = std::time::Instant::now();
            let result = process::start_detached(&launch, Show::Shown);
            let (status, code, finding, event) = match &result {
                Ok(pid) => (
                    "completed",
                    None,
                    RESUMED,
                    format!(
                        "{}: started {} (pid {pid})",
                        stopped.entry.name,
                        action::describe_launch(&launch)
                    ),
                ),
                Err(err) => (
                    "launch_failed",
                    Some(RESUME_FAILED),
                    RESUME_FAILED,
                    format!("{}: {err}", stopped.entry.name),
                ),
            };
            if let Ok(run) = run {
                let _ = runs::finish(
                    db,
                    run,
                    if result.is_ok() {
                        runs::status::COMPLETED
                    } else {
                        runs::status::LAUNCH_FAILED
                    },
                    None,
                    false,
                );
            }
            reporter.event(finding, event);
            self.findings
                .push(Finding::named(finding, stopped.entry.name.clone()));
            self.actions.push(ActionInfo {
                name: stopped.entry.name.clone(),
                phase: ActionPhase::Resume.as_str(),
                operation: context.operation.as_str(),
                kind: resume.kind().as_str(),
                program: program.display().to_string(),
                status,
                exit_code: None,
                reboot_required: false,
                duration_ms: started.elapsed().as_millis() as u64,
                timeout_seconds: 0,
                on_failure: "continue",
                code,
                stdout: String::new(),
                stderr: String::new(),
                sha256: resume.is_packaged().then(|| resume.sha256.clone()),
            });
        }
        self.settled = true;
    }
}

impl Drop for Quiesced {
    fn drop(&mut self) {
        // A run that ended without settling — an early return between the
        // stop and the outcome — is a programming error the log should
        // show rather than an application silently left stopped.
        debug_assert!(
            self.settled || self.stopped.is_empty(),
            "quiescence was not settled"
        );
    }
}
