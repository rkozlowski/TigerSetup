//! The `dependency_event` table: what the dependency phase detected,
//! acquired and installed, each in its own commit unit. A record here is
//! history of a requirement being satisfied, not ownership: uninstall never
//! plans from it.

use crate::Result;
use crate::state::Db;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyEvent {
    pub dependency_id: String,
    pub version: String,
    /// `detected`, `acquired`, `installed` or `reboot_required`.
    pub action: String,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub recorded_at: String,
}

/// Records one event durably.
pub fn record(db: &Db, event: &DependencyEvent) -> Result<()> {
    db.commit_unit(|txn| {
        txn.execute(
            "INSERT INTO dependency_event (dependency_id, version, action, url, sha256, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                event.dependency_id,
                event.version,
                event.action,
                event.url,
                event.sha256,
                event.recorded_at
            ],
        )?;
        Ok(())
    })
}

/// Every recorded event, oldest first.
pub fn events(db: &Db) -> Result<Vec<DependencyEvent>> {
    let mut statement = db.conn().prepare(
        "SELECT dependency_id, version, action, url, sha256, recorded_at FROM dependency_event ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(DependencyEvent {
            dependency_id: row.get(0)?,
            version: row.get(1)?,
            action: row.get(2)?,
            url: row.get(3)?,
            sha256: row.get(4)?,
            recorded_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_rw(&dir.path().join("state.db")).unwrap();
        let first = DependencyEvent {
            dependency_id: "Vendor.Thing".into(),
            version: "1.2.0".into(),
            action: "acquired".into(),
            url: Some("https://x.invalid/thing.exe".into()),
            sha256: Some("ab".repeat(32)),
            recorded_at: "2026-09-07T00:00:00.000Z".into(),
        };
        let second = DependencyEvent {
            action: "installed".into(),
            url: None,
            sha256: None,
            ..first.clone()
        };
        record(&db, &first).unwrap();
        record(&db, &second).unwrap();
        assert_eq!(events(&db).unwrap(), vec![first, second]);
    }
}
