//! Committed ownership: the `installation` row, the typed resource tables
//! and the recorded installer options.

use std::collections::BTreeMap;

use rusqlite::OptionalExtension;
use rusqlite::types::Value;
use tigersetup_format::metadata::OptionValue;

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
    /// `Metadata::license_sha256` of the licence text a person explicitly
    /// accepted on the wizard's licence page, carried by the transaction
    /// that installed this version or by an earlier one; `None` when nobody
    /// did — an unattended run proceeds without agreeing to anything.
    pub accepted_license_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedFile {
    /// Install-relative path with backslashes.
    pub path: String,
    pub sha256: String,
    pub size: u64,
    /// The file's last-write time (`FILETIME`) once TigerSetup had put it
    /// in place; `None` for a row written before it was recorded.
    pub modified: Option<i64>,
}

impl OwnedFile {
    /// The record a stat is compared with: size and last-write time.
    pub fn fingerprint(&self) -> Option<crate::win::fs::Fingerprint> {
        self.modified.map(|modified| crate::win::fs::Fingerprint {
            size: self.size,
            modified,
        })
    }
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
    /// The value was there before TigerSetup wrote it; `previous_*` is what
    /// it held, and is what taking the value away puts back. A row an
    /// engine before schema 6 wrote reads as `false`: the value is deleted
    /// when it still holds what TigerSetup wrote, as it always was.
    pub pre_existed: bool,
    pub previous_kind: Option<String>,
    pub previous_data: Option<String>,
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
    /// Absolute path of the `.lnk` or `.url`.
    pub path: String,
    /// Absolute path the link points at, or the URL an Internet shortcut
    /// opens.
    pub target: String,
}

/// A variable of the scope's environment TigerSetup set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedEnvironmentVariable {
    /// The environment key holding the variable.
    pub hive_key: String,
    pub name: String,
    /// What TigerSetup wrote, as `win::registry::Data` stores it.
    pub kind: String,
    pub data: String,
    /// The variable was there before TigerSetup wrote it; `previous_*` is
    /// what it held, and is what a removal puts back.
    pub pre_existed: bool,
    pub previous_kind: Option<String>,
    pub previous_data: Option<String>,
}

/// A Windows Firewall rule TigerSetup created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedFirewallRule {
    pub name: String,
    /// The rule as written, serialized by `resource::firewall`.
    pub rule: String,
}

/// An uninstall-phase action the installation carries for its own future
/// uninstall: its definition (the metadata `Action` message, hex-encoded)
/// and, for a packaged program, the identity of the file the state
/// directory keeps under `actions\<sha256>\`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedAction {
    pub name: String,
    /// `pre-uninstall` or `post-uninstall`.
    pub phase: String,
    pub definition: String,
    pub artifact_sha256: Option<String>,
    pub artifact_file: Option<String>,
    pub artifact_size: Option<u64>,
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
    pub environment_variables: Vec<OwnedEnvironmentVariable>,
    pub firewall_rules: Vec<OwnedFirewallRule>,
    pub actions: Vec<OwnedAction>,
    pub registration_key: Option<String>,
}

pub fn read(db: &Db) -> Result<Option<InstallationRow>> {
    // A reader may be looking at a file the schema-3 column has not been
    // added to yet; there it is what the column would hold: nothing.
    let accepted_license = if db.has_schema(3) {
        "accepted_license_sha256"
    } else {
        "NULL"
    };
    Ok(db
        .conn()
        .query_row(
            &format!("SELECT id, product_id, version, scope, install_root, engine_version, committed_at, registration_key, {accepted_license} FROM installation LIMIT 1"),
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
                    accepted_license_sha256: row.get(8)?,
                })
            },
        )
        .optional()?)
}

pub fn owned_files(db: &Db) -> Result<Vec<OwnedFile>> {
    let modified = if db.has_schema(7) { "modified" } else { "NULL" };
    let mut statement = db.conn().prepare(&format!(
        "SELECT path, sha256, size, {modified} FROM file ORDER BY path"
    ))?;
    let rows = statement.query_map([], |row| {
        let size: i64 = row.get(2)?;
        Ok(OwnedFile {
            path: row.get(0)?,
            sha256: row.get(1)?,
            size: size as u64,
            modified: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The ownership row of one file, by its install-relative path.
pub fn owned_file(db: &Db, path: &str) -> Result<Option<OwnedFile>> {
    let modified = if db.has_schema(7) { "modified" } else { "NULL" };
    Ok(db
        .conn()
        .query_row(
            &format!("SELECT path, sha256, size, {modified} FROM file WHERE path = ?1"),
            [path],
            |row| {
                let size: i64 = row.get(2)?;
                Ok(OwnedFile {
                    path: row.get(0)?,
                    sha256: row.get(1)?,
                    size: size as u64,
                    modified: row.get(3)?,
                })
            },
        )
        .optional()?)
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
    // A reader may be looking at a file the schema-6 columns have not been
    // added to yet; there they are what the columns would hold: the value
    // did not pre-exist, so it is taken away by deletion.
    let previous = if db.has_schema(6) {
        "pre_existed, previous_kind, previous_data"
    } else {
        "0, NULL, NULL"
    };
    let mut statement = db.conn().prepare(&format!(
        "SELECT key, name, kind, data, {previous} FROM registry_value ORDER BY key, name"
    ))?;
    let rows = statement.query_map([], |row| {
        let pre_existed: i64 = row.get(4)?;
        Ok(OwnedRegistryValue {
            key: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            data: row.get(3)?,
            pre_existed: pre_existed != 0,
            previous_kind: row.get(5)?,
            previous_data: row.get(6)?,
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

pub fn owned_environment_variables(db: &Db) -> Result<Vec<OwnedEnvironmentVariable>> {
    if !db.has_schema(4) {
        return Ok(Vec::new());
    }
    let mut statement = db.conn().prepare(
        "SELECT hive_key, name, kind, data, pre_existed, previous_kind, previous_data FROM environment_variable ORDER BY hive_key, name",
    )?;
    let rows = statement.query_map([], |row| {
        let pre_existed: i64 = row.get(4)?;
        Ok(OwnedEnvironmentVariable {
            hive_key: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            data: row.get(3)?,
            pre_existed: pre_existed != 0,
            previous_kind: row.get(5)?,
            previous_data: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn owned_firewall_rules(db: &Db) -> Result<Vec<OwnedFirewallRule>> {
    if !db.has_schema(4) {
        return Ok(Vec::new());
    }
    let mut statement = db
        .conn()
        .prepare("SELECT name, rule FROM firewall_rule ORDER BY name")?;
    let rows = statement.query_map([], |row| {
        Ok(OwnedFirewallRule {
            name: row.get(0)?,
            rule: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn owned_actions(db: &Db) -> Result<Vec<OwnedAction>> {
    if !db.has_schema(5) {
        return Ok(Vec::new());
    }
    let mut statement = db.conn().prepare(
        "SELECT name, phase, definition, artifact_sha256, artifact_file, artifact_size FROM action ORDER BY rowid",
    )?;
    let rows = statement.query_map([], |row| {
        let size: Option<i64> = row.get(5)?;
        Ok(OwnedAction {
            name: row.get(0)?,
            phase: row.get(1)?,
            definition: row.get(2)?,
            artifact_sha256: row.get(3)?,
            artifact_file: row.get(4)?,
            artifact_size: size.map(|s| s as u64),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The hashes of every packaged program the installation keeps in its
/// action store: the uninstall actions' and the quiescence entries' stop
/// and resume programs. What a store sweep must leave alone.
pub fn owned_program_hashes(db: &Db) -> Result<std::collections::HashSet<String>> {
    let mut hashes = std::collections::HashSet::new();
    for record in owned_actions(db)? {
        if record.phase == tigersetup_format::metadata::ActionPhase::Quiesce.as_str() {
            let entry = crate::action::deserialize_quiescence(&record.definition)?;
            for program in entry.packaged() {
                hashes.insert(program.sha256.to_ascii_lowercase());
            }
        } else if let Some(sha) = record.artifact_sha256 {
            hashes.insert(sha.to_ascii_lowercase());
        }
    }
    Ok(hashes)
}

/// The option value a stored column holds: text since schema 4, an integer
/// before it. A reader looking at an older file sees the integer and reads
/// the boolean it is; a mutating run has migrated the column to text.
pub fn option_value_of(value: Value) -> OptionValue {
    match value {
        Value::Integer(n) => OptionValue::Bool(n != 0),
        Value::Text(text) => match text.as_str() {
            "true" => OptionValue::Bool(true),
            "false" => OptionValue::Bool(false),
            _ => OptionValue::Choice(text),
        },
        _ => OptionValue::Bool(false),
    }
}

/// The recorded installer options, by lower-case name.
pub fn options(db: &Db) -> Result<BTreeMap<String, OptionValue>> {
    let mut statement = db
        .conn()
        .prepare("SELECT name, value FROM installation_option ORDER BY name")?;
    let rows = statement.query_map([], |row| {
        let value: Value = row.get(1)?;
        Ok((row.get::<_, String>(0)?, option_value_of(value)))
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
        environment_variables: owned_environment_variables(db)?,
        firewall_rules: owned_firewall_rules(db)?,
        actions: owned_actions(db)?,
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
