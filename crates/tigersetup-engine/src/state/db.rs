//! Opening `state.db`: durability pragmas, exclusive locking for mutating
//! runs, read-only access for inspection, schema version in `user_version`
//! with forward-only migrations. A fresh database runs every migration in
//! order, so there is one DDL per version and a migrated database is
//! identical to a new one.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction};

use crate::{Error, Result};

/// Schema version this engine writes.
pub const SCHEMA_VERSION: i32 = 2;

pub struct Db {
    conn: Connection,
}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        match err.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DatabaseBusy) | Some(rusqlite::ErrorCode::DatabaseLocked) => {
                Error::new(
                    "database_busy",
                    "the state database is in use by another TigerSetup process",
                )
            }
            // The file is there but this process may not open it that way:
            // a standard user reaching a machine-scope database it has no
            // rights to, or a read-only medium.
            Some(rusqlite::ErrorCode::CannotOpen)
            | Some(rusqlite::ErrorCode::PermissionDenied)
            | Some(rusqlite::ErrorCode::ReadOnly) => Error::new(
                "state_unreadable",
                format!("the state database cannot be opened: {err}"),
            ),
            _ => Error::new("database_error", err.to_string()),
        }
    }
}

impl Db {
    /// Opens (creating if needed) the database for a mutating run. The
    /// connection holds an exclusive lock until it is dropped, so a
    /// concurrent read-only opener sees "busy" rather than a moving target.
    pub fn open_rw(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                Error::new(
                    "io_error",
                    format!("cannot create {}: {err}", parent.display()),
                )
            })?;
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        // Take the exclusive lock now rather than at the first write.
        conn.execute_batch("BEGIN EXCLUSIVE; COMMIT;")?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    /// Opens an existing database read-only with normal locking. Returns
    /// `None` when the file does not exist.
    pub fn open_ro(path: &Path) -> Result<Option<Db>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_millis(500))?;
        let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version == 0 {
            // A database another run created but never populated is no state.
            return Ok(None);
        }
        if version > SCHEMA_VERSION {
            return Err(Error::new(
                "state_schema_unsupported",
                format!(
                    "state database schema {version} is newer than this engine understands ({SCHEMA_VERSION})"
                ),
            ));
        }
        if version < SCHEMA_VERSION {
            return Err(Error::new(
                "state_schema_stale",
                format!(
                    "state database schema {version} needs a mutating run to migrate to {SCHEMA_VERSION}"
                ),
            ));
        }
        Ok(Some(Db { conn }))
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Runs `f` inside one short SQL transaction and commits it. With
    /// `journal_mode = DELETE` and `synchronous = FULL` the commit is durable
    /// when this returns.
    pub fn commit_unit<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<T>,
    ) -> Result<T> {
        let txn = self.conn.unchecked_transaction()?;
        let value = f(&txn)?;
        txn.commit()?;
        Ok(value)
    }

    fn migrate(&self) -> Result<()> {
        let version: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::new(
                "state_schema_unsupported",
                format!(
                    "state database schema {version} is newer than this engine understands ({SCHEMA_VERSION})"
                ),
            ));
        }
        for (target, ddl) in [(1, SCHEMA_V1), (2, SCHEMA_V2)] {
            if version < target {
                self.commit_unit(|txn| {
                    txn.execute_batch(ddl)?;
                    txn.pragma_update(None, "user_version", target)
                })?;
            }
        }
        Ok(())
    }
}

/// Schema version 1: the file engine's tables, the dependency events, and
/// the resources it did not yet own reserved as placeholders.
pub const SCHEMA_V1: &str = r#"
CREATE TABLE installation (
    id             TEXT PRIMARY KEY,
    product_id     TEXT NOT NULL,
    version        TEXT NOT NULL,
    scope          TEXT NOT NULL,
    install_root   TEXT NOT NULL,
    engine_version TEXT NOT NULL,
    committed_at   TEXT NOT NULL
);

CREATE TABLE "transaction" (
    id               TEXT PRIMARY KEY,
    kind             TEXT NOT NULL,
    from_version     TEXT,
    to_version       TEXT,
    package_id       TEXT NOT NULL,
    package_version  TEXT NOT NULL,
    metadata_sha256  TEXT NOT NULL,
    scope            TEXT NOT NULL,
    install_root     TEXT NOT NULL,
    state            TEXT NOT NULL,
    started_at       TEXT NOT NULL,
    finished_at      TEXT
);

CREATE TABLE operation (
    transaction_id   TEXT NOT NULL REFERENCES "transaction"(id),
    sequence         INTEGER NOT NULL,
    kind             TEXT NOT NULL,
    target           TEXT NOT NULL,
    state            TEXT NOT NULL,
    previous_existed INTEGER,
    previous_sha256  TEXT,
    backup_path      TEXT,
    payload_entry    TEXT,
    expected_size    INTEGER,
    applied_sha256   TEXT,
    result_code      TEXT,
    PRIMARY KEY (transaction_id, sequence)
);

CREATE TABLE file (
    path        TEXT PRIMARY KEY,
    sha256      TEXT NOT NULL,
    size        INTEGER NOT NULL,
    owned_since TEXT NOT NULL REFERENCES "transaction"(id)
);

CREATE TABLE directory (
    path        TEXT PRIMARY KEY,
    created     INTEGER NOT NULL,
    owned_since TEXT NOT NULL REFERENCES "transaction"(id)
);

CREATE TABLE registry_value (id INTEGER PRIMARY KEY);
CREATE TABLE path_entry (id INTEGER PRIMARY KEY);
CREATE TABLE shortcut (id INTEGER PRIMARY KEY);
-- What the dependency phase observed. History of a requirement being
-- satisfied, never ownership: uninstall does not plan from this table.
CREATE TABLE dependency_event (
    id            INTEGER PRIMARY KEY,
    dependency_id TEXT NOT NULL,
    version       TEXT NOT NULL,
    action        TEXT NOT NULL,
    url           TEXT,
    sha256        TEXT,
    recorded_at   TEXT NOT NULL
);
"#;

/// Schema version 2: typed tables for registry keys and values, PATH
/// entries, shortcuts and the recorded installer options; the operation
/// row carries what a non-file undo needs (previous kind and data, the
/// value written, the link's parameters); the installation and the
/// transaction remember the registration key. `dependency_event` is
/// recreated, because a database written before the dependency phase
/// existed still carries the placeholder table.
const SCHEMA_V2: &str = r#"
ALTER TABLE installation ADD COLUMN registration_key TEXT;
ALTER TABLE "transaction" ADD COLUMN registration_key TEXT;

ALTER TABLE operation ADD COLUMN value_name TEXT;
ALTER TABLE operation ADD COLUMN value_kind TEXT;
ALTER TABLE operation ADD COLUMN value_data TEXT;
ALTER TABLE operation ADD COLUMN previous_kind TEXT;
ALTER TABLE operation ADD COLUMN previous_data TEXT;
ALTER TABLE operation ADD COLUMN link_arguments TEXT;
ALTER TABLE operation ADD COLUMN link_description TEXT;
ALTER TABLE operation ADD COLUMN link_icon TEXT;

DROP TABLE registry_value;
DROP TABLE path_entry;
DROP TABLE shortcut;
DROP TABLE dependency_event;

CREATE TABLE dependency_event (
    id            INTEGER PRIMARY KEY,
    dependency_id TEXT NOT NULL,
    version       TEXT NOT NULL,
    action        TEXT NOT NULL,
    url           TEXT,
    sha256        TEXT,
    recorded_at   TEXT NOT NULL
);

CREATE TABLE registry_key (
    key         TEXT PRIMARY KEY COLLATE NOCASE,
    created     INTEGER NOT NULL,
    owned_since TEXT NOT NULL REFERENCES "transaction"(id)
);

CREATE TABLE registry_value (
    key         TEXT NOT NULL COLLATE NOCASE,
    name        TEXT NOT NULL COLLATE NOCASE,
    kind        TEXT NOT NULL,
    data        TEXT NOT NULL,
    owned_since TEXT NOT NULL REFERENCES "transaction"(id),
    PRIMARY KEY (key, name)
);

CREATE TABLE path_entry (
    hive_key    TEXT NOT NULL COLLATE NOCASE,
    raw         TEXT NOT NULL,
    normalized  TEXT NOT NULL,
    pre_existed INTEGER NOT NULL,
    added       INTEGER NOT NULL,
    owned_since TEXT NOT NULL REFERENCES "transaction"(id),
    PRIMARY KEY (hive_key, normalized)
);

CREATE TABLE shortcut (
    path        TEXT PRIMARY KEY COLLATE NOCASE,
    target      TEXT NOT NULL,
    owned_since TEXT NOT NULL REFERENCES "transaction"(id)
);

CREATE TABLE installation_option (
    name  TEXT PRIMARY KEY,
    value INTEGER NOT NULL
);

CREATE TABLE transaction_option (
    transaction_id TEXT NOT NULL REFERENCES "transaction"(id),
    name           TEXT NOT NULL,
    value          INTEGER NOT NULL,
    PRIMARY KEY (transaction_id, name)
);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn columns(db: &Db, table: &str) -> Vec<String> {
        let mut statement = db
            .conn()
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn database_is_created_with_the_pragmas_and_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("state.db");
        let db = Db::open_rw(&path).unwrap();
        let journal_mode: String = db
            .conn()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "delete");
        let synchronous: i32 = db
            .conn()
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .unwrap();
        assert_eq!(synchronous, 2);
        let version: i32 = db
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        let tables: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('installation','transaction','operation','file','directory','registry_key','registry_value','path_entry','shortcut','installation_option','transaction_option','dependency_event')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 12);
        assert!(columns(&db, "operation").contains(&"previous_data".to_string()));
        assert_eq!(
            columns(&db, "path_entry"),
            vec![
                "hive_key",
                "raw",
                "normalized",
                "pre_existed",
                "added",
                "owned_since"
            ]
        );
    }

    #[test]
    fn a_version_1_database_migrates_in_place_and_keeps_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO \"transaction\" (id, kind, package_id, package_version, metadata_sha256, scope, install_root, state, started_at) VALUES ('t1', 'install', 'P', '1.0.0', 'h', 'user', 'C:\\P', 'committed', 'now')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO installation (id, product_id, version, scope, install_root, engine_version, committed_at) VALUES ('i1', 'P', '1.0.0', 'user', 'C:\\P', '0.1.0', 'now')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO file (path, sha256, size, owned_since) VALUES ('a.txt', 'abc', 3, 't1')",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            Db::open_ro(&path).err().map(|e| e.code),
            Some("state_schema_stale"),
            "a read-only opener never migrates"
        );
        let db = Db::open_rw(&path).unwrap();
        let version: i32 = db
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        let files: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM file", [], |r| r.get(0))
            .unwrap();
        assert_eq!(files, 1);
        let registration: Option<String> = db
            .conn()
            .query_row("SELECT registration_key FROM installation", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(registration, None);
        assert!(columns(&db, "registry_value").contains(&"data".to_string()));
        drop(db);
        // A second open finds nothing left to migrate.
        let db = Db::open_rw(&path).unwrap();
        assert!(columns(&db, "shortcut").contains(&"target".to_string()));
    }

    #[test]
    fn exclusive_writer_makes_a_reader_busy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let writer = Db::open_rw(&path).unwrap();
        let reader = Db::open_ro(&path);
        assert_eq!(reader.err().map(|e| e.code), Some("database_busy"));
        drop(writer);
        assert!(Db::open_ro(&path).unwrap().is_some());
        assert!(
            Db::open_ro(&dir.path().join("missing.db"))
                .unwrap()
                .is_none()
        );
    }
}
