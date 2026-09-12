//! Committed ownership: the `installation` row, the typed resource tables
//! and the recorded installer options.

use std::collections::BTreeMap;

use rusqlite::OptionalExtension;

use crate::Result;
use crate::state::Db;

#[derive(Debug, Clone)]
pub struct InstallationRow {
    pub id: String,
    pub product_id: String,
    pub version: String,
    pub scope: String,
    pub install_root: String,
    pub engine_version: String,
    pub committed_at: String,
    /// The Add/Remove Programs key the installation registered under.
    pub registration_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedFile {
    /// Install-relative path with backslashes.
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedDirectory {
    /// Install-relative path with backslashes; empty for the install root.
    pub path: String,
    /// Whether TigerSetup created it (and may therefore remove it when empty).
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedRegistryKey {
    /// `HKCU\...` or `HKLM\...`.
    pub key: String,
    /// Whether TigerSetup created it (and may remove it once it is empty).
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedRegistryValue {
    pub key: String,
    pub name: String,
    /// As `win::registry::Data::kind_name` writes it.
    pub kind: String,
    /// As `win::registry::Data::text` writes it.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPathEntry {
    /// The environment key holding `Path`.
    pub hive_key: String,
    /// The exact text written (or found) in `Path`.
    pub raw: String,
    pub normalized: String,
    /// An equivalent entry existed before TigerSetup: never claimed.
    pub pre_existed: bool,
    /// TigerSetup added it and removes it at uninstall.
    pub added: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedShortcut {
    /// Absolute path of the `.lnk`.
    pub path: String,
    /// Absolute path the link points at.
    pub target: String,
}

/// Everything the database says the installation owns.
#[derive(Debug, Clone, Default)]
pub struct Owned {
    pub files: Vec<OwnedFile>,
    pub directories: Vec<OwnedDirectory>,
    pub registry_keys: Vec<OwnedRegistryKey>,
    pub registry_values: Vec<OwnedRegistryValue>,
    pub path_entries: Vec<OwnedPathEntry>,
    pub shortcuts: Vec<OwnedShortcut>,
    pub registration_key: Option<String>,
}

pub fn read(db: &Db) -> Result<Option<InstallationRow>> {
    Ok(db
        .conn()
        .query_row(
            "SELECT id, product_id, version, scope, install_root, engine_version, committed_at, registration_key FROM installation LIMIT 1",
            [],
            |row| {
                Ok(InstallationRow {
                    id: row.get(0)?,
                    product_id: row.get(1)?,
                    version: row.get(2)?,
                    scope: row.get(3)?,
                    install_root: row.get(4)?,
                    engine_version: row.get(5)?,
                    committed_at: row.get(6)?,
                    registration_key: row.get(7)?,
                })
            },
        )
        .optional()?)
}

pub fn owned_files(db: &Db) -> Result<Vec<OwnedFile>> {
    let mut statement = db
        .conn()
        .prepare("SELECT path, sha256, size FROM file ORDER BY path")?;
    let rows = statement.query_map([], |row| {
        let size: i64 = row.get(2)?;
        Ok(OwnedFile {
            path: row.get(0)?,
            sha256: row.get(1)?,
            size: size as u64,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn owned_directories(db: &Db) -> Result<Vec<OwnedDirectory>> {
    let mut statement = db
        .conn()
        .prepare("SELECT path, created FROM directory ORDER BY path")?;
    let rows = statement.query_map([], |row| {
        let created: i64 = row.get(1)?;
        Ok(OwnedDirectory {
            path: row.get(0)?,
            created: created != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn owned_registry_keys(db: &Db) -> Result<Vec<OwnedRegistryKey>> {
    let mut statement = db
        .conn()
        .prepare("SELECT key, created FROM registry_key ORDER BY key")?;
    let rows = statement.query_map([], |row| {
        let created: i64 = row.get(1)?;
        Ok(OwnedRegistryKey {
            key: row.get(0)?,
            created: created != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn owned_registry_values(db: &Db) -> Result<Vec<OwnedRegistryValue>> {
    let mut statement = db
        .conn()
        .prepare("SELECT key, name, kind, data FROM registry_value ORDER BY key, name")?;
    let rows = statement.query_map([], |row| {
        Ok(OwnedRegistryValue {
            key: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            data: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn owned_path_entries(db: &Db) -> Result<Vec<OwnedPathEntry>> {
    let mut statement = db.conn().prepare(
        "SELECT hive_key, raw, normalized, pre_existed, added FROM path_entry ORDER BY hive_key, normalized",
    )?;
    let rows = statement.query_map([], |row| {
        let pre_existed: i64 = row.get(3)?;
        let added: i64 = row.get(4)?;
        Ok(OwnedPathEntry {
            hive_key: row.get(0)?,
            raw: row.get(1)?,
            normalized: row.get(2)?,
            pre_existed: pre_existed != 0,
            added: added != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn owned_shortcuts(db: &Db) -> Result<Vec<OwnedShortcut>> {
    let mut statement = db
        .conn()
        .prepare("SELECT path, target FROM shortcut ORDER BY path")?;
    let rows = statement.query_map([], |row| {
        Ok(OwnedShortcut {
            path: row.get(0)?,
            target: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The recorded installer options, by lower-case name.
pub fn options(db: &Db) -> Result<BTreeMap<String, bool>> {
    let mut statement = db
        .conn()
        .prepare("SELECT name, value FROM installation_option ORDER BY name")?;
    let rows = statement.query_map([], |row| {
        let value: i64 = row.get(1)?;
        Ok((row.get::<_, String>(0)?, value != 0))
    })?;
    Ok(rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()?)
}

/// Every owned resource, read together.
pub fn owned(db: &Db, row: &InstallationRow) -> Result<Owned> {
    Ok(Owned {
        files: owned_files(db)?,
        directories: owned_directories(db)?,
        registry_keys: owned_registry_keys(db)?,
        registry_values: owned_registry_values(db)?,
        path_entries: owned_path_entries(db)?,
        shortcuts: owned_shortcuts(db)?,
        registration_key: row.registration_key.clone(),
    })
}

fn count(db: &Db, table: &str) -> Result<u64> {
    let count: i64 = db
        .conn()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
    Ok(count as u64)
}

pub fn file_count(db: &Db) -> Result<u64> {
    count(db, "file")
}

pub fn registry_value_count(db: &Db) -> Result<u64> {
    count(db, "registry_value")
}
