//! The `action_run` table: every execution of a custom action, `started`
//! before the process exists and finished with what it produced, each in
//! its own commit unit. It is evidence, not ownership: nothing plans from
//! it, and it is what lets a restart tell an action that was running when
//! the power went from one that never ran (`TigerSetup-Design.md` §5.14).

use rusqlite::{OptionalExtension, params};

use crate::Result;
use crate::state::Db;

/// The status column of one run.
pub mod status {
    /// The process is being started, or was running when the record was
    /// last written.
    pub const STARTED: &str = "started";
    /// Exited with a success code (a reboot code counts).
    pub const COMPLETED: &str = "completed";
    /// Exited with a code that is neither.
    pub const FAILED: &str = "failed";
    /// Killed at its deadline.
    pub const TIMED_OUT: &str = "timed_out";
    /// The process could not be started.
    pub const LAUNCH_FAILED: &str = "launch_failed";
    /// Found `started` by a later run: TigerSetup does not know what it
    /// did before the interruption.
    pub const INTERRUPTED: &str = "interrupted";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRun {
    pub id: i64,
    pub transaction_id: String,
    pub sequence: i64,
    pub name: String,
    pub phase: String,
    pub operation: String,
    pub kind: String,
    pub program: String,
    pub on_failure: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub reboot_required: bool,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// Records that an action is about to be started, durably, before the
/// process exists. Returns the row id the finish is recorded against.
#[allow(clippy::too_many_arguments)]
pub fn start(
    db: &Db,
    transaction_id: &str,
    sequence: i64,
    name: &str,
    phase: &str,
    operation: &str,
    kind: &str,
    program: &str,
    on_failure: &str,
) -> Result<i64> {
    db.commit_unit(|sql| {
        sql.execute(
            "INSERT INTO action_run (transaction_id, sequence, name, phase, operation, kind, program, on_failure, status, started_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                transaction_id,
                sequence,
                name,
                phase,
                operation,
                kind,
                program,
                on_failure,
                status::STARTED,
                crate::report::now_rfc3339()
            ],
        )?;
        Ok(sql.last_insert_rowid())
    })
}

/// Records how a started run ended.
pub fn finish(
    db: &Db,
    id: i64,
    status: &str,
    exit_code: Option<i32>,
    reboot_required: bool,
) -> Result<()> {
    db.commit_unit(|sql| {
        sql.execute(
            "UPDATE action_run SET status = ?2, exit_code = ?3, reboot_required = ?4, finished_at = ?5 WHERE id = ?1",
            params![
                id,
                status,
                exit_code,
                reboot_required as i64,
                crate::report::now_rfc3339()
            ],
        )?;
        Ok(())
    })
}

const COLUMNS: &str = "id, transaction_id, sequence, name, phase, operation, kind, program, on_failure, status, exit_code, reboot_required, started_at, finished_at";

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActionRun> {
    let reboot: i64 = row.get(11)?;
    Ok(ActionRun {
        id: row.get(0)?,
        transaction_id: row.get(1)?,
        sequence: row.get(2)?,
        name: row.get(3)?,
        phase: row.get(4)?,
        operation: row.get(5)?,
        kind: row.get(6)?,
        program: row.get(7)?,
        on_failure: row.get(8)?,
        status: row.get(9)?,
        exit_code: row.get(10)?,
        reboot_required: reboot != 0,
        started_at: row.get(12)?,
        finished_at: row.get(13)?,
    })
}

/// The latest run recorded for one operation of a transaction, if any.
pub fn latest(db: &Db, transaction_id: &str, sequence: i64) -> Result<Option<ActionRun>> {
    if !db.has_schema(5) {
        return Ok(None);
    }
    let mut statement = db.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM action_run WHERE transaction_id = ?1 AND sequence = ?2 ORDER BY id DESC LIMIT 1"
    ))?;
    Ok(statement
        .query_row(params![transaction_id, sequence], read)
        .optional()?)
}

/// Every recorded run of a transaction, oldest first.
pub fn runs(db: &Db, transaction_id: &str) -> Result<Vec<ActionRun>> {
    if !db.has_schema(5) {
        return Ok(Vec::new());
    }
    let mut statement = db.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM action_run WHERE transaction_id = ?1 ORDER BY id"
    ))?;
    let rows = statement.query_map([transaction_id], read)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_started_then_finished_and_the_latest_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_rw(&dir.path().join("state.db")).unwrap();
        let first = start(
            &db,
            "t1",
            3,
            "build-cache",
            "post-install",
            "install",
            "exe",
            "C:\\x.exe",
            "fail",
        )
        .unwrap();
        let started = latest(&db, "t1", 3).unwrap().unwrap();
        assert_eq!(started.status, status::STARTED);
        assert_eq!(started.exit_code, None);
        assert!(started.finished_at.is_none());
        finish(&db, first, status::FAILED, Some(3), false).unwrap();
        let second = start(
            &db,
            "t1",
            3,
            "build-cache",
            "post-install",
            "install",
            "exe",
            "C:\\x.exe",
            "fail",
        )
        .unwrap();
        finish(&db, second, status::COMPLETED, Some(3010), true).unwrap();
        let all = runs(&db, "t1").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].status, status::FAILED);
        assert_eq!(all[0].exit_code, Some(3));
        assert!(all[0].finished_at.is_some());
        let last = latest(&db, "t1", 3).unwrap().unwrap();
        assert_eq!(last.id, second);
        assert!(last.reboot_required);
        assert!(latest(&db, "t1", 4).unwrap().is_none());
    }
}
