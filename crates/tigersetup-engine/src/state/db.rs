//! Opening `state.db`: durability pragmas, exclusive locking for mutating
//! runs, read-only access for inspection, schema version in `user_version`
//! with forward-only migrations. A fresh database runs every migration in
//! order, so there is one DDL per version and a migrated database is
//! identical to a new one.
//!
//! A read-only opener cannot migrate, and must not fail on an installation
//! that a mutating run has simply not touched since the engine was updated:
//! the readers tolerate every schema back to [`OLDEST_READABLE_SCHEMA`], and
//! read a column that schema does not have as `NULL`.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction};

use crate::{Error, Result};

/// Schema version this engine writes.
pub const SCHEMA_VERSION: i32 = 4;

/// The oldest schema the readers understand without a migration: version 2
/// has every typed table; version 3 only adds nullable columns to it, and
/// version 4 adds tables, nullable columns, and stores option values as text
/// where 2 and 3 stored integers — which a reader tells apart by the value's
/// own type, so it reads all three without a migration.
pub const OLDEST_READABLE_SCHEMA: i32 = 2;

pub struct Db {
    conn: Connection,
    /// The schema the file has: `SCHEMA_VERSION` after a mutating open, and
    /// whatever the last mutating run left after a read-only one.
    schema_version: i32,
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
        let mut db = Db {
            conn,
            schema_version: 0,
        };
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
        if version < OLDEST_READABLE_SCHEMA {
            return Err(Error::new(
                "state_schema_stale",
                format!(
                    "state database schema {version} needs a mutating run to migrate to {SCHEMA_VERSION}"
                ),
            ));
        }
        Ok(Some(Db {
            conn,
            schema_version: version,
        }))
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Whether the file has the columns a schema version added, for a
    /// reader that may be looking at an older file; a mutating run always
    /// has them.
    pub fn has_schema(&self, version: i32) -> bool {
        self.schema_version >= version
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

    fn migrate(&mut self) -> Result<()> {
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
        for (target, ddl) in [
            (1, SCHEMA_V1),
            (2, SCHEMA_V2),
            (3, SCHEMA_V3),
            (4, SCHEMA_V4),
        ] {
            if version < target {
                self.commit_unit(|txn| {
                    txn.execute_batch(ddl)?;
                    txn.pragma_update(None, "user_version", target)
                })?;
            }
        }
        self.schema_version = SCHEMA_VERSION;
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

/// Schema version 3: the licence text a person explicitly accepted, as the
/// hash of its exact bytes (`Metadata::license_sha256`), on the installation
/// it was accepted for and on the installing transaction that carries it to
/// the commit. Both nullable: an installation nobody accepted a licence for
/// — every unattended one — records none.
pub const SCHEMA_V3: &str = r#"
ALTER TABLE installation ADD COLUMN accepted_license_sha256 TEXT;
ALTER TABLE "transaction" ADD COLUMN accepted_license_sha256 TEXT;
"#;

/// Schema version 4: option values become text — the canonical value of a
/// boolean (`true`/`false`) or of a choice option — with the recorded
/// integers rewritten; the environment-variable and firewall-rule ownership
/// tables; and the operation row's `restore_kind`/`restore_data`, the state
/// a removal puts back (an environment variable's pre-installation value),
/// as distinct from `previous_*`, the undo record of the operation itself,
/// and the two link parameters shortcuts gained.
pub const SCHEMA_V4: &str = r#"
CREATE TABLE installation_option_v4 (
    name  TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT INTO installation_option_v4 (name, value)
    SELECT name, CASE WHEN value = 0 THEN 'false' ELSE 'true' END FROM installation_option;
DROP TABLE installation_option;
ALTER TABLE installation_option_v4 RENAME TO installation_option;

CREATE TABLE transaction_option_v4 (
    transaction_id TEXT NOT NULL REFERENCES "transaction"(id),
    name           TEXT NOT NULL,
    value          TEXT NOT NULL,
    PRIMARY KEY (transaction_id, name)
);
INSERT INTO transaction_option_v4 (transaction_id, name, value)
    SELECT transaction_id, name, CASE WHEN value = 0 THEN 'false' ELSE 'true' END FROM transaction_option;
DROP TABLE transaction_option;
ALTER TABLE transaction_option_v4 RENAME TO transaction_option;

ALTER TABLE operation ADD COLUMN restore_kind TEXT;
ALTER TABLE operation ADD COLUMN restore_data TEXT;
ALTER TABLE operation ADD COLUMN link_working_directory TEXT;
ALTER TABLE operation ADD COLUMN link_app_user_model_id TEXT;

CREATE TABLE environment_variable (
    hive_key      TEXT NOT NULL COLLATE NOCASE,
    name          TEXT NOT NULL COLLATE NOCASE,
    kind          TEXT NOT NULL,
    data          TEXT NOT NULL,
    pre_existed   INTEGER NOT NULL,
    previous_kind TEXT,
    previous_data TEXT,
    owned_since   TEXT NOT NULL REFERENCES "transaction"(id),
    PRIMARY KEY (hive_key, name)
);

CREATE TABLE firewall_rule (
    name        TEXT PRIMARY KEY COLLATE NOCASE,
    rule        TEXT NOT NULL,
    owned_since TEXT NOT NULL REFERENCES "transaction"(id)
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
            .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('installation','transaction','operation','file','directory','registry_key','registry_value','path_entry','shortcut','installation_option','transaction_option','dependency_event','environment_variable','firewall_rule')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 14);
        assert!(columns(&db, "operation").contains(&"previous_data".to_string()));
        assert!(columns(&db, "operation").contains(&"restore_data".to_string()));
        assert!(columns(&db, "installation").contains(&"accepted_license_sha256".to_string()));
        assert!(columns(&db, "\"transaction\"").contains(&"accepted_license_sha256".to_string()));
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
        assert_eq!(version, SCHEMA_VERSION);
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

    /// An installation made before the engine recorded licence acceptance
    /// is still an installation: a read-only opener describes it, with no
    /// acceptance, and the first mutating run migrates it in place.
    #[test]
    fn a_version_2_database_is_read_as_it_is_and_migrated_by_a_mutating_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.execute_batch(SCHEMA_V2).unwrap();
            conn.pragma_update(None, "user_version", 2).unwrap();
            conn.execute(
                "INSERT INTO \"transaction\" (id, kind, package_id, package_version, metadata_sha256, scope, install_root, state, started_at) VALUES ('t1', 'install', 'P', '1.0.0', 'h', 'user', 'C:\\P', 'committed', 'now')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO installation (id, product_id, version, scope, install_root, engine_version, committed_at) VALUES ('i1', 'P', '1.0.0', 'user', 'C:\\P', '0.5.2', 'now')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO installation_option (name, value) VALUES ('path', 1), ('desktop-shortcut', 0)",
                [],
            )
            .unwrap();
        }
        let reader = Db::open_ro(&path).unwrap().expect("the file is state");
        assert!(!reader.has_schema(3));
        let row = crate::state::installation::read(&reader)
            .unwrap()
            .expect("the installation is described");
        assert_eq!(row.version, "1.0.0");
        assert_eq!(row.accepted_license_sha256, None);
        assert!(
            crate::state::journal::open_transaction(&reader)
                .unwrap()
                .is_none()
        );
        // The integer options of a schema-2 file read as the booleans they
        // are, without a migration.
        let options = crate::state::installation::options(&reader).unwrap();
        assert_eq!(
            options.get("path"),
            Some(&tigersetup_format::metadata::OptionValue::Bool(true))
        );
        assert_eq!(
            options.get("desktop-shortcut"),
            Some(&tigersetup_format::metadata::OptionValue::Bool(false))
        );
        assert!(
            crate::state::installation::owned(&reader, &row)
                .unwrap()
                .environment_variables
                .is_empty(),
            "a table the schema does not have reads as empty"
        );
        drop(reader);
        let reader = Db::open_ro(&path).unwrap().unwrap();
        let version: i32 = reader
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2, "a read-only opener leaves the file as it is");
        drop(reader);

        let db = Db::open_rw(&path).unwrap();
        assert!(db.has_schema(3));
        assert!(db.has_schema(4));
        let row = crate::state::installation::read(&db).unwrap().unwrap();
        assert_eq!(
            row.accepted_license_sha256, None,
            "no acceptance is invented"
        );
        assert!(columns(&db, "installation").contains(&"accepted_license_sha256".to_string()));
        let options = crate::state::installation::options(&db).unwrap();
        assert_eq!(
            options.get("path"),
            Some(&tigersetup_format::metadata::OptionValue::Bool(true)),
            "the migrated option keeps its value"
        );
        let stored: String = db
            .conn()
            .query_row(
                "SELECT value FROM installation_option WHERE name = 'path'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "true", "the migration rewrites the integer as text");
    }

    /// A schema-3 file — the shape every 0.5.x installation has — is read
    /// as it is and migrated in place by the first mutating run, with its
    /// licence acceptance intact.
    #[test]
    fn a_version_3_database_is_read_as_it_is_and_migrated_by_a_mutating_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.execute_batch(SCHEMA_V2).unwrap();
            conn.execute_batch(SCHEMA_V3).unwrap();
            conn.pragma_update(None, "user_version", 3).unwrap();
            conn.execute(
                "INSERT INTO \"transaction\" (id, kind, package_id, package_version, metadata_sha256, scope, install_root, state, started_at, accepted_license_sha256) VALUES ('t1', 'install', 'P', '1.0.0', 'h', 'user', 'C:\\P', 'committed', 'now', 'abc')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO installation (id, product_id, version, scope, install_root, engine_version, committed_at, accepted_license_sha256) VALUES ('i1', 'P', '1.0.0', 'user', 'C:\\P', '0.5.3', 'now', 'abc')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO installation_option (name, value) VALUES ('path', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transaction_option (transaction_id, name, value) VALUES ('t1', 'path', 0)",
                [],
            )
            .unwrap();
        }
        let reader = Db::open_ro(&path).unwrap().unwrap();
        assert!(reader.has_schema(3) && !reader.has_schema(4));
        let row = crate::state::installation::read(&reader).unwrap().unwrap();
        assert_eq!(row.accepted_license_sha256.as_deref(), Some("abc"));
        assert_eq!(
            crate::state::installation::options(&reader)
                .unwrap()
                .get("path"),
            Some(&tigersetup_format::metadata::OptionValue::Bool(false))
        );
        drop(reader);

        let db = Db::open_rw(&path).unwrap();
        let row = crate::state::installation::read(&db).unwrap().unwrap();
        assert_eq!(
            row.accepted_license_sha256.as_deref(),
            Some("abc"),
            "the acceptance survives the migration"
        );
        assert_eq!(
            crate::state::installation::options(&db)
                .unwrap()
                .get("path"),
            Some(&tigersetup_format::metadata::OptionValue::Bool(false))
        );
        let transaction_options: i64 = db
            .conn()
            .query_row(
                "SELECT count(*) FROM transaction_option WHERE transaction_id = 't1' AND value = 'false'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(transaction_options, 1);
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
