//! Recovery on restart. Every mutating run reconciles an open transaction
//! before its own work: forward when the running engine carries the same
//! package (identity, version and metadata hash) so every payload byte is at
//! hand, rollback otherwise. A transaction already rolling back is always
//! completed as a rollback. Recovery begins with a sweep of temporaries and
//! orphan backups, so a crash between a filesystem action and its journal row
//! leaves nothing behind.

use std::collections::HashSet;
use std::path::Path;

use crate::report::{ActionInfo, Finding, RecoveryInfo, Reporter};
use crate::state::Db;
use crate::state::journal::{self, TransactionRow, TxnState};
use crate::txn::executor::{Executor, staging_dir_for};
use crate::txn::fault::FaultInjector;
use crate::win::fs;
use crate::{Error, Package, Result};

pub const FORWARD: &str = "forward";
pub const ROLLBACK: &str = "rollback";

/// The direction a mutating run of `package` would take for `txn`.
pub fn direction(txn: &TransactionRow, package: &Package) -> &'static str {
    if txn.state != TxnState::Running {
        return ROLLBACK;
    }
    let same_package = txn.package_id == package.id()
        && txn.package_version == package.version()
        && txn.metadata_sha256 == package.metadata_sha256();
    if same_package { FORWARD } else { ROLLBACK }
}

/// What a recovery did, and what it preserved on the way.
pub struct Recovery {
    /// `None` when no transaction was open.
    pub info: Option<RecoveryInfo>,
    pub findings: Vec<Finding>,
    /// The custom actions the recovery ran or settled, and whether one
    /// asked for a restart.
    pub actions: Vec<ActionInfo>,
    pub reboot_required: bool,
}

impl Recovery {
    /// What a run reports when there was no state to reconcile at all.
    pub fn none() -> Recovery {
        Recovery {
            info: None,
            findings: Vec::new(),
            actions: Vec::new(),
            reboot_required: false,
        }
    }
}

/// Reconciles an open transaction, if there is one, and removes staging
/// areas that finished transactions left behind.
pub fn recover(
    db: &Db,
    package: &Package,
    state_dir: &Path,
    quiet: bool,
    fault: &mut FaultInjector,
    reporter: &mut Reporter<'_>,
) -> Result<Recovery> {
    let open = journal::open_transaction(db)?;
    remove_stale_staging(state_dir, open.as_ref().map(|t| t.id.as_str()), reporter);
    let Some(txn) = open else {
        return Ok(Recovery::none());
    };

    let dir = direction(&txn, package);
    reporter.event(
        "recovery_started",
        format!(
            "direction={dir} transaction={} kind={} state={} from={} to={}",
            txn.id,
            txn.kind.as_str(),
            txn.state.as_str(),
            txn.from_version.as_deref().unwrap_or("-"),
            txn.to_version.as_deref().unwrap_or("-")
        ),
    );

    sweep(db, &txn, state_dir, reporter)?;

    // Every installing kind may have files or packaged action programs
    // still to write; a rollback never reads the payload.
    let payload = match (dir, txn.kind.installs()) {
        (FORWARD, true) => Some(package.installer().payload_archive()?),
        _ => None,
    };
    let mut executor =
        Executor::new(db, txn, state_dir, payload, fault, reporter).unattended(quiet);
    let mut info = RecoveryInfo {
        direction: dir,
        operations_reapplied: 0,
        operations_rolled_back: 0,
    };

    let result = if dir == FORWARD {
        // An upgrade keeps the installation's identity; a first install gets
        // a new one.
        let installation_id = crate::state::installation::read(db)?
            .map(|row| row.id)
            .unwrap_or_else(crate::report::unique_id);
        match executor.run_forward(true).and_then(|stats| {
            executor.commit(&installation_id, crate::ENGINE_VERSION)?;
            Ok(stats)
        }) {
            Ok(stats) => {
                info.operations_reapplied = stats.reapplied;
                Ok(())
            }
            Err(err) => {
                executor
                    .reporter
                    .event("recovery_forward_failed", format!("{err}; rolling back"));
                executor
                    .rollback()
                    .map(|stats| info.operations_rolled_back = stats.rolled_back)
            }
        }
    } else {
        executor
            .rollback()
            .map(|stats| info.operations_rolled_back = stats.rolled_back)
    };

    match result {
        Ok(()) => {
            executor.cleanup();
            let state = executor.transaction().state;
            executor.reporter.event(
                "recovery_completed",
                format!(
                    "direction={dir} state={} reapplied={} rolled_back={}",
                    state.as_str(),
                    info.operations_reapplied,
                    info.operations_rolled_back
                ),
            );
            let (actions, reboot_required) = executor.take_actions();
            Ok(Recovery {
                info: Some(info),
                findings: executor.take_findings(),
                actions,
                reboot_required,
            })
        }
        Err(err) => Err(Error::new(
            "recovery_incomplete",
            format!(
                "transaction {} could not be recovered ({dir}): {err}",
                executor.transaction().id
            ),
        )),
    }
}

/// Deletes `*.tigersetup-new` under the install root and backup entries no
/// journal row references. A backup is named by its operation's sequence, so
/// "referenced" is decided by sequence: a journal row with a backup path keeps
/// the entry of that sequence alive, whatever the stored text looks like.
fn sweep(
    db: &Db,
    txn: &TransactionRow,
    state_dir: &Path,
    reporter: &mut Reporter<'_>,
) -> Result<()> {
    for swept in fs::sweep_temp_files(Path::new(&txn.install_root)) {
        reporter.event("temp_file_swept", swept.display().to_string());
    }
    let referenced: HashSet<i64> = journal::operations(db, &txn.id)?
        .into_iter()
        .filter(|op| op.backup_path.is_some())
        .map(|op| op.sequence)
        .collect();
    let backup_dir = staging_dir_for(state_dir, &txn.id).join("backup");
    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let sequence: Option<i64> = entry.file_name().to_str().and_then(|n| n.parse().ok());
            let live = sequence.is_some_and(|s| referenced.contains(&s));
            if !live && std::fs::remove_file(&path).is_ok() {
                reporter.event("backup_swept", path.display().to_string());
            }
        }
    }
    Ok(())
}

/// Removes `txn-*` directories that belong to no open transaction: the
/// cleanup a committed or rolled-back run did not get to.
fn remove_stale_staging(state_dir: &Path, open_id: Option<&str>, reporter: &mut Reporter<'_>) {
    let Ok(entries) = std::fs::read_dir(state_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_prefix("txn-") else {
            continue;
        };
        if Some(id) == open_id || !entry.path().is_dir() {
            continue;
        }
        if std::fs::remove_dir_all(entry.path()).is_ok() {
            reporter.event("staging_swept", entry.path().display().to_string());
        }
    }
}
