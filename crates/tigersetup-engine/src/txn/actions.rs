//! The custom-action operations of the forward walk, the recovery and the
//! reverse walk (`TigerSetup-Design.md` §5.14).
//!
//! `run_action` walks the same states as every other operation — `planned
//! → prepared → applying → applied` — and adds its own evidence: an
//! `action_run` row written `started` before the process exists and
//! finished after it exits, so that a restart finding the operation
//! `applying` can tell a program that was running from one that never was.
//! Its undo undoes nothing and records `action_not_reverted`: TigerSetup
//! knows what it started, not what the program changed.
//!
//! `store_action` keeps the packaged program of an uninstall-phase action
//! in the state directory, under its SHA-256, and the commit records the
//! action as the installation's own. Content addressing is what makes the
//! switch atomic: a new version's program never overwrites an old one, the
//! commit changes which set the `action` table names, and whatever the
//! table does not name is swept afterwards.

use std::path::Path;

use tigersetup_format::metadata::Action;

use crate::action::{self, Context, Status, Verdict};
use crate::report::{ActionInfo, Phase, Progress};
use crate::resource::file;
use crate::state::action::{self as runs, status};
use crate::state::installation;
use crate::state::journal::{self, OpKind, OperationRow, Undo};
use crate::txn::executor::Executor;
use crate::txn::fault::FaultPoint;
use crate::win::process;
use crate::{Error, Result};

impl Executor<'_, '_> {
    fn action_of(op: &OperationRow) -> Result<Action> {
        action::deserialize(Self::definition_of(op)?)
    }

    fn definition_of(op: &OperationRow) -> Result<&str> {
        op.value_data.as_deref().ok_or_else(|| {
            Error::new(
                "journal_inconsistent",
                format!("operation {} has no action definition", op.sequence),
            )
        })
    }

    /// Whether a store operation keeps a quiescence entry rather than an
    /// action.
    fn stores_quiescence(op: &OperationRow) -> bool {
        op.value_kind.as_deref() == Some(action::QUIESCENCE_VALUE_KIND)
    }

    /// The packaged programs a store operation keeps: an action's one, or
    /// a quiescence entry's stop and resume.
    fn stored_programs(op: &OperationRow) -> Result<Vec<Action>> {
        if Self::stores_quiescence(op) {
            let entry = action::deserialize_quiescence(Self::definition_of(op)?)?;
            Ok(entry.packaged().into_iter().cloned().collect())
        } else {
            let action = Self::action_of(op)?;
            Ok(action.is_packaged().then_some(action).into_iter().collect())
        }
    }

    fn context(&self) -> Context {
        Context {
            install_root: self.install_root.clone(),
            version: self.txn.package_version.clone(),
            product_id: self.txn.package_id.clone(),
            scope: tigersetup_format::identity::Scope::parse(&self.txn.scope)
                .unwrap_or(tigersetup_format::identity::Scope::User),
            operation: action::operation_of(self.txn.kind),
            quiet: self.quiet,
        }
    }

    /// The undo record of an action operation: for a stored program,
    /// whether its directory was already there (a same-hash program of the
    /// previous installation, which a rollback must not remove); nothing
    /// for a run, which has nothing to put back.
    pub(crate) fn prepare_action(&self, op: &OperationRow) -> Result<Undo> {
        Ok(match op.kind {
            OpKind::StoreAction => {
                // Whether every program's directory was already there; a
                // rollback removes what this transaction created.
                let programs = Self::stored_programs(op)?;
                Undo {
                    existed: programs
                        .iter()
                        .all(|p| action::store_dir(&self.state_dir, &p.sha256).is_dir()),
                    ..Undo::default()
                }
            }
            _ => Undo::default(),
        })
    }

    /// Writes a packaged program from the payload to `target`, flushed and
    /// verified against the hash the builder recorded. A file already there
    /// with that hash is kept.
    fn extract_program(&mut self, action: &Action, target: &Path) -> Result<()> {
        if file::matches(target, &action.sha256)? {
            return Ok(());
        }
        let payload = self.payload.as_mut().ok_or_else(|| {
            Error::new(
                "payload_unavailable",
                format!(
                    "this run carries no payload for the program of action {}",
                    action.name
                ),
            )
        })?;
        if let Some(parent) = target.parent() {
            crate::win::fs::create_directory(parent)?;
        }
        let mut staged =
            file::stage_from_payload(target, payload, &action.entry, Some(action.size))?;
        if staged.sha256 != action.sha256 {
            return Err(Error::new(
                "action_program_mismatch",
                format!(
                    "the packaged program of action {} has SHA-256 {}, the package recorded {}",
                    action.name, staged.sha256, action.sha256
                ),
            ));
        }
        staged.flush()?;
        staged.commit()
    }

    /// The program of an action, on disk and verified: extracted from the
    /// payload for a packaged install-phase action, read from the state
    /// directory for a stored uninstall-phase one, or the expanded command.
    fn program_of(&mut self, action: &Action, context: &Context) -> Result<std::path::PathBuf> {
        if !action.is_packaged() {
            return Ok(std::path::PathBuf::from(action::expand(
                &action.command,
                context,
            )));
        }
        let path = if action.phase().is_install() {
            let path = action::staged_program(&self.staging_dir, action);
            self.extract_program(action, &path)?;
            path
        } else {
            action::stored_program(&self.state_dir, action)
        };
        // A stored program is verified every time before it runs: the
        // package is the trust boundary, and a file that no longer hashes
        // to what the package recorded is not the package's.
        if !file::matches(&path, &action.sha256)? {
            return Err(Error::new(
                if path.exists() {
                    "action_program_mismatch"
                } else {
                    "action_program_missing"
                },
                format!(
                    "the program of action {} at {} is not the one the package recorded",
                    action.name,
                    path.display()
                ),
            ));
        }
        Ok(path)
    }

    fn record_output(&mut self, action: &Action, stream: &'static str, text: &str) {
        for (index, line) in text.lines().enumerate() {
            if index == action::LOG_LINES {
                self.reporter.event(
                    "action_output_truncated",
                    format!(
                        "{} {stream}: {} more lines",
                        action.name,
                        text.lines().count() - index
                    ),
                );
                break;
            }
            self.reporter
                .event("action_output", format!("{} {stream}: {line}", action.name));
        }
    }

    /// Starts the program, waits for it within its deadline, judges the
    /// result and records it — in the `action_run` row, the log and the
    /// outcome — then either journals the operation `applied` or fails the
    /// transaction, by the action's own policy.
    pub(crate) fn run_action(&mut self, op: &OperationRow) -> Result<()> {
        let action = Self::action_of(op)?;
        let context = self.context();
        let phase = action.phase();
        let program = self.program_of(&action, &context)?;
        let launch = action::launch_of(&action, &program, &context);
        let run = runs::start(
            self.db,
            &self.txn.id,
            op.sequence,
            &action.name,
            phase.as_str(),
            context.operation.as_str(),
            action.kind().as_str(),
            &program.display().to_string(),
            action.failure_policy().as_str(),
        )?;
        self.fault.at(
            FaultPoint::AfterActionStarted,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        self.reporter.progress(
            "action_started",
            format!(
                "{} ({}, {}): {} [timeout {} s, on failure {}]",
                action.name,
                phase.as_str(),
                context.operation.as_str(),
                action::describe_launch(&launch),
                action.timeout_seconds(),
                action.failure_policy().as_str()
            ),
            Progress {
                phase: Phase::Actions,
                done: 0,
                total: 0,
                target: action.name.clone(),
            },
        );
        let started = std::time::Instant::now();
        let verdict = match process::run_captured(&launch) {
            Ok(captured) => action::judge(&action, &captured),
            Err(err) => {
                self.reporter
                    .event("action_launch_failed", format!("{}: {err}", action.name));
                action::launch_failed(started.elapsed().as_millis() as u64)
            }
        };
        runs::finish(
            self.db,
            run,
            verdict.status.as_str(),
            verdict.exit_code,
            verdict.reboot_required,
        )?;
        self.record_output(&action, "stdout", &verdict.stdout);
        self.record_output(&action, "stderr", &verdict.stderr);
        self.finish_action(op, &action, &program, &verdict)
    }

    /// Records a verdict and settles the operation by the policy.
    fn finish_action(
        &mut self,
        op: &OperationRow,
        action: &Action,
        program: &Path,
        verdict: &Verdict,
    ) -> Result<()> {
        let continues = action::continues(action, verdict);
        let status = match verdict.status {
            Status::Completed => "completed",
            Status::Failed if continues => "failed_continued",
            other => other.as_str(),
        };
        let code = (!continues).then(|| verdict.status.failure_code());
        self.reporter.event(
            match verdict.status {
                Status::Completed => "action_completed",
                Status::TimedOut => "action_timed_out",
                _ => "action_failed",
            },
            format!(
                "{}: status={status} exit_code={} duration_ms={}{}",
                action.name,
                verdict
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".into()),
                verdict.duration_ms,
                if verdict.reboot_required {
                    " reboot_required=true"
                } else {
                    ""
                }
            ),
        );
        self.actions.push(ActionInfo {
            name: action.name.clone(),
            phase: action.phase().as_str(),
            operation: action::operation_of(self.txn.kind).as_str(),
            kind: action.kind().as_str(),
            program: program.display().to_string(),
            status,
            exit_code: verdict.exit_code,
            reboot_required: verdict.reboot_required,
            duration_ms: verdict.duration_ms,
            timeout_seconds: action.timeout_seconds(),
            on_failure: action.failure_policy().as_str(),
            code,
            stdout: action::tail(&verdict.stdout),
            stderr: action::tail(&verdict.stderr),
            sha256: action.is_packaged().then(|| action.sha256.clone()),
        });
        if verdict.reboot_required {
            self.reboot_required = true;
        }
        if !continues {
            return Err(Error::new(
                verdict.status.failure_code(),
                action::failure_message(action, verdict),
            ));
        }
        let result_code = match verdict.status {
            Status::Completed if verdict.reboot_required => Some("reboot_required"),
            Status::Completed => None,
            _ => {
                self.note_named(action::FAILED_CONTINUED, action.name.clone());
                Some(action::FAILED_CONTINUED)
            }
        };
        journal::mark_applied(self.db, &self.txn.id, op.sequence, None, result_code)
    }

    /// An action found `applying` on restart. The `action_run` row says
    /// what the crashed run got to: a row still `started` is a program
    /// that may have run to any point — it is marked `interrupted`,
    /// reported, and run again, which is why an action has to be safe to
    /// retry; a row that finished is settled the way the crashed run was
    /// about to settle it; no row means the program was never started.
    /// Returns whether the program was run.
    pub(crate) fn reconcile_action(&mut self, op: &OperationRow) -> Result<bool> {
        let action = Self::action_of(op)?;
        let Some(last) = runs::latest(self.db, &self.txn.id, op.sequence)? else {
            self.run_action(op)?;
            return Ok(true);
        };
        match last.status.as_str() {
            status::STARTED => {
                runs::finish(self.db, last.id, status::INTERRUPTED, None, false)?;
                self.note_named(action::INTERRUPTED, action.name.clone());
                self.actions.push(ActionInfo {
                    name: action.name.clone(),
                    phase: action.phase().as_str(),
                    operation: action::operation_of(self.txn.kind).as_str(),
                    kind: action.kind().as_str(),
                    program: last.program.clone(),
                    status: "interrupted",
                    exit_code: None,
                    reboot_required: false,
                    duration_ms: 0,
                    timeout_seconds: action.timeout_seconds(),
                    on_failure: action.failure_policy().as_str(),
                    code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    sha256: action.is_packaged().then(|| action.sha256.clone()),
                });
                self.run_action(op)?;
                Ok(true)
            }
            status::LAUNCH_FAILED | status::INTERRUPTED => {
                self.run_action(op)?;
                Ok(true)
            }
            finished => {
                let verdict = Verdict {
                    status: match finished {
                        status::COMPLETED => Status::Completed,
                        status::TIMED_OUT => Status::TimedOut,
                        _ => Status::Failed,
                    },
                    exit_code: last.exit_code,
                    reboot_required: last.reboot_required,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration_ms: 0,
                };
                self.reporter.event(
                    "action_settled",
                    format!(
                        "{}: the earlier run recorded {finished}; not run again",
                        action.name
                    ),
                );
                self.finish_action(op, &action, Path::new(&last.program), &verdict)?;
                Ok(false)
            }
        }
    }

    /// Keeps the packaged program of an uninstall-phase action in the
    /// state directory; an action without one needs nothing kept. The
    /// commit then records the action from this row.
    pub(crate) fn store_action(&mut self, op: &OperationRow) -> Result<()> {
        let programs = Self::stored_programs(op)?;
        for program in &programs {
            let target = action::stored_program(&self.state_dir, program);
            self.extract_program(program, &target)?;
            self.reporter.event(
                "action_program_stored",
                format!("{}: {}", program.name, target.display()),
            );
        }
        journal::mark_applied(
            self.db,
            &self.txn.id,
            op.sequence,
            programs.first().map(|p| p.sha256.as_str()),
            None,
        )
    }

    /// The undo of an action operation. A stored program's directory this
    /// transaction created is removed; one that was already there belongs
    /// to the previous installation and stays. A run has nothing to undo:
    /// what the program changed is not TigerSetup's to know, and the
    /// rollback says so rather than pretending.
    pub(crate) fn undo_action(&mut self, op: &OperationRow) -> Result<()> {
        match op.kind {
            OpKind::StoreAction => {
                if op.previous_existed == Some(false) {
                    let owned = installation::owned_program_hashes(self.db)?;
                    for program in Self::stored_programs(op)? {
                        // A directory the committed installation still
                        // refers to belongs to it, whatever this
                        // transaction did with it.
                        if owned.contains(&program.sha256.to_ascii_lowercase()) {
                            continue;
                        }
                        let dir = action::store_dir(&self.state_dir, &program.sha256);
                        if dir.exists() {
                            std::fs::remove_dir_all(&dir).map_err(|err| {
                                Error::new(
                                    "io_error",
                                    format!("cannot remove {}: {err}", dir.display()),
                                )
                            })?;
                        }
                    }
                }
                Ok(())
            }
            _ => {
                let action = Self::action_of(op)?;
                let Some(run) = runs::latest(self.db, &self.txn.id, op.sequence)? else {
                    return Ok(());
                };
                // A record still `started` is a program that may have run
                // to any point before the interruption; the rollback
                // records that, and never runs it again.
                if run.status == status::STARTED {
                    runs::finish(self.db, run.id, status::INTERRUPTED, None, false)?;
                }
                if run.status != status::LAUNCH_FAILED {
                    self.note_named(action::NOT_REVERTED, action.name.clone());
                }
                Ok(())
            }
        }
    }

    /// Removes the stored program directories the `action` table no longer
    /// names: after a commit, the previous version's; after a rollback,
    /// whatever the undo left. Idempotent, and never touches a directory
    /// a committed action refers to.
    pub(crate) fn sweep_action_store(&mut self) {
        let store = self.state_dir.join(action::STORE_DIR);
        let Ok(entries) = std::fs::read_dir(&store) else {
            return;
        };
        let referenced = installation::owned_program_hashes(self.db).unwrap_or_default();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if referenced.contains(&name) || !entry.path().is_dir() {
                continue;
            }
            if std::fs::remove_dir_all(entry.path()).is_ok() {
                self.reporter
                    .event("action_program_swept", entry.path().display().to_string());
            }
        }
        let _ = std::fs::remove_dir(&store);
    }
}
