//! The transaction journal: `transaction` and `operation` rows, their state
//! machines, and the durable transitions. Every transition is its own short
//! SQL transaction, so each is on disk before the next Windows mutation.
//!
//! Operation: `planned → prepared → applying → applied`; on failure
//! `rolling_back → rolled_back | rollback_failed`.
//! Transaction: `running → committed`, or `rolling_back → rolled_back`, with
//! `rollback_failed` when an undo could not be completed. Any non-terminal
//! state found on restart needs recovery.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction, params};
use tigersetup_format::metadata::OptionValue;

use crate::plan::PlannedOperation;
use crate::resource::path;
use crate::state::Db;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnKind {
    Install,
    Upgrade,
    /// The same version reconciled with explicit options.
    Reinstall,
    Repair,
    Uninstall,
}

impl TxnKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TxnKind::Install => "install",
            TxnKind::Upgrade => "upgrade",
            TxnKind::Reinstall => "reinstall",
            TxnKind::Repair => "repair",
            TxnKind::Uninstall => "uninstall",
        }
    }

    fn parse(text: &str) -> Result<TxnKind> {
        match text {
            "install" => Ok(TxnKind::Install),
            "upgrade" => Ok(TxnKind::Upgrade),
            "reinstall" => Ok(TxnKind::Reinstall),
            "repair" => Ok(TxnKind::Repair),
            "uninstall" => Ok(TxnKind::Uninstall),
            other => Err(Error::new(
                "journal_inconsistent",
                format!("unknown transaction kind {other:?}"),
            )),
        }
    }

    /// Whether the commit records ownership (as opposed to removing it).
    pub fn installs(self) -> bool {
        !matches!(self, TxnKind::Uninstall)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnState {
    Running,
    Committed,
    RollingBack,
    RolledBack,
    RollbackFailed,
}

impl TxnState {
    pub fn as_str(self) -> &'static str {
        match self {
            TxnState::Running => "running",
            TxnState::Committed => "committed",
            TxnState::RollingBack => "rolling_back",
            TxnState::RolledBack => "rolled_back",
            TxnState::RollbackFailed => "rollback_failed",
        }
    }

    fn parse(text: &str) -> Result<TxnState> {
        match text {
            "running" => Ok(TxnState::Running),
            "committed" => Ok(TxnState::Committed),
            "rolling_back" => Ok(TxnState::RollingBack),
            "rolled_back" => Ok(TxnState::RolledBack),
            "rollback_failed" => Ok(TxnState::RollbackFailed),
            other => Err(Error::new(
                "journal_inconsistent",
                format!("unknown transaction state {other:?}"),
            )),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, TxnState::Committed | TxnState::RolledBack)
    }
}

/// Every typed operation. A `keep_*` is an owned resource the new state
/// still wants unchanged: no mutation, only ownership re-tagged at commit,
/// journaled `applied` from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    InstallFile,
    CreateDirectory,
    RemoveFile,
    RemoveDirectory,
    KeepFile,
    KeepDirectory,
    CreateRegistryKey,
    RemoveRegistryKey,
    KeepRegistryKey,
    SetRegistryValue,
    RemoveRegistryValue,
    KeepRegistryValue,
    AddPathEntry,
    RemovePathEntry,
    KeepPathEntry,
    CreateShortcut,
    RemoveShortcut,
    KeepShortcut,
    /// Sets a variable of the scope's environment; the undo record is what
    /// it held before.
    SetEnvironmentVariable,
    /// Puts a variable back to its pre-installation state — the
    /// `restore_*` columns, or absent — when it still holds what TigerSetup
    /// wrote.
    RestoreEnvironmentVariable,
    KeepEnvironmentVariable,
    /// Creates a Windows Firewall rule, or rewrites an owned one; the undo
    /// record is the rule that was there before, if any.
    CreateFirewallRule,
    RemoveFirewallRule,
    KeepFirewallRule,
    /// Runs a custom action (`target` is its name, `value_data` its
    /// definition). Its undo undoes nothing and says so.
    RunAction,
    /// Keeps the program of an uninstall-phase action in the state
    /// directory and records the action as the installation's own; the
    /// undo removes a program directory this transaction created.
    StoreAction,
}

impl OpKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OpKind::InstallFile => "install_file",
            OpKind::CreateDirectory => "create_directory",
            OpKind::RemoveFile => "remove_file",
            OpKind::RemoveDirectory => "remove_directory",
            OpKind::KeepFile => "keep_file",
            OpKind::KeepDirectory => "keep_directory",
            OpKind::CreateRegistryKey => "create_registry_key",
            OpKind::RemoveRegistryKey => "remove_registry_key",
            OpKind::KeepRegistryKey => "keep_registry_key",
            OpKind::SetRegistryValue => "set_registry_value",
            OpKind::RemoveRegistryValue => "remove_registry_value",
            OpKind::KeepRegistryValue => "keep_registry_value",
            OpKind::AddPathEntry => "add_path_entry",
            OpKind::RemovePathEntry => "remove_path_entry",
            OpKind::KeepPathEntry => "keep_path_entry",
            OpKind::CreateShortcut => "create_shortcut",
            OpKind::RemoveShortcut => "remove_shortcut",
            OpKind::KeepShortcut => "keep_shortcut",
            OpKind::SetEnvironmentVariable => "set_environment_variable",
            OpKind::RestoreEnvironmentVariable => "restore_environment_variable",
            OpKind::KeepEnvironmentVariable => "keep_environment_variable",
            OpKind::CreateFirewallRule => "create_firewall_rule",
            OpKind::RemoveFirewallRule => "remove_firewall_rule",
            OpKind::KeepFirewallRule => "keep_firewall_rule",
            OpKind::RunAction => "run_action",
            OpKind::StoreAction => "store_action",
        }
    }

    fn parse(text: &str) -> Result<OpKind> {
        match text {
            "install_file" => Ok(OpKind::InstallFile),
            "create_directory" => Ok(OpKind::CreateDirectory),
            "remove_file" => Ok(OpKind::RemoveFile),
            "remove_directory" => Ok(OpKind::RemoveDirectory),
            "keep_file" => Ok(OpKind::KeepFile),
            "keep_directory" => Ok(OpKind::KeepDirectory),
            "create_registry_key" => Ok(OpKind::CreateRegistryKey),
            "remove_registry_key" => Ok(OpKind::RemoveRegistryKey),
            "keep_registry_key" => Ok(OpKind::KeepRegistryKey),
            "set_registry_value" => Ok(OpKind::SetRegistryValue),
            "remove_registry_value" => Ok(OpKind::RemoveRegistryValue),
            "keep_registry_value" => Ok(OpKind::KeepRegistryValue),
            "add_path_entry" => Ok(OpKind::AddPathEntry),
            "remove_path_entry" => Ok(OpKind::RemovePathEntry),
            "keep_path_entry" => Ok(OpKind::KeepPathEntry),
            "create_shortcut" => Ok(OpKind::CreateShortcut),
            "remove_shortcut" => Ok(OpKind::RemoveShortcut),
            "keep_shortcut" => Ok(OpKind::KeepShortcut),
            "set_environment_variable" => Ok(OpKind::SetEnvironmentVariable),
            "restore_environment_variable" => Ok(OpKind::RestoreEnvironmentVariable),
            "keep_environment_variable" => Ok(OpKind::KeepEnvironmentVariable),
            "create_firewall_rule" => Ok(OpKind::CreateFirewallRule),
            "remove_firewall_rule" => Ok(OpKind::RemoveFirewallRule),
            "keep_firewall_rule" => Ok(OpKind::KeepFirewallRule),
            "run_action" => Ok(OpKind::RunAction),
            "store_action" => Ok(OpKind::StoreAction),
            other => Err(Error::new(
                "journal_inconsistent",
                format!("unknown operation kind {other:?}"),
            )),
        }
    }

    /// Whether the operation is a custom action or its record: journaled
    /// like every other operation, but nothing TigerSetup can put back.
    pub fn is_action(self) -> bool {
        matches!(self, OpKind::RunAction | OpKind::StoreAction)
    }

    /// Keeps mutate nothing and are never walked forward or back.
    pub fn is_keep(self) -> bool {
        matches!(
            self,
            OpKind::KeepFile
                | OpKind::KeepDirectory
                | OpKind::KeepRegistryKey
                | OpKind::KeepRegistryValue
                | OpKind::KeepPathEntry
                | OpKind::KeepShortcut
                | OpKind::KeepEnvironmentVariable
                | OpKind::KeepFirewallRule
        )
    }

    /// Whether the operation changes the scope's environment block, so the
    /// walk broadcasts the change once at its end.
    pub fn touches_environment(self) -> bool {
        matches!(
            self,
            OpKind::AddPathEntry
                | OpKind::RemovePathEntry
                | OpKind::SetEnvironmentVariable
                | OpKind::RestoreEnvironmentVariable
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpState {
    Planned,
    Prepared,
    Applying,
    Applied,
    RollingBack,
    RolledBack,
    RollbackFailed,
}

impl OpState {
    pub fn as_str(self) -> &'static str {
        match self {
            OpState::Planned => "planned",
            OpState::Prepared => "prepared",
            OpState::Applying => "applying",
            OpState::Applied => "applied",
            OpState::RollingBack => "rolling_back",
            OpState::RolledBack => "rolled_back",
            OpState::RollbackFailed => "rollback_failed",
        }
    }

    fn parse(text: &str) -> Result<OpState> {
        match text {
            "planned" => Ok(OpState::Planned),
            "prepared" => Ok(OpState::Prepared),
            "applying" => Ok(OpState::Applying),
            "applied" => Ok(OpState::Applied),
            "rolling_back" => Ok(OpState::RollingBack),
            "rolled_back" => Ok(OpState::RolledBack),
            "rollback_failed" => Ok(OpState::RollbackFailed),
            other => Err(Error::new(
                "journal_inconsistent",
                format!("unknown operation state {other:?}"),
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransactionRow {
    pub id: String,
    pub kind: TxnKind,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub package_id: String,
    pub package_version: String,
    pub metadata_sha256: String,
    pub scope: String,
    pub install_root: String,
    pub state: TxnState,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// The Add/Remove Programs key an installing transaction registers.
    pub registration_key: Option<String>,
    /// The licence acceptance an installing transaction commits to the
    /// installation row: the text the person accepted in this run, or the
    /// acceptance the installation already recorded, carried forward.
    pub accepted_license_sha256: Option<String>,
}

/// One journaled operation. `target` is the resource identity of the kind:
/// an install-relative path for files and directories, a `HKCU\...` /
/// `HKLM\...` key for registry operations (the environment key for PATH
/// entries, whose entry text is `value_data`, and for environment
/// variables, whose name is `value_name`), the absolute `.lnk` or `.url`
/// path for shortcuts (whose link target or URL is `value_data`), and the
/// rule name for firewall rules (whose serialized rule is `value_data`).
#[derive(Debug, Clone)]
pub struct OperationRow {
    pub transaction_id: String,
    pub sequence: i64,
    pub kind: OpKind,
    pub target: String,
    pub state: OpState,
    pub previous_existed: Option<bool>,
    pub previous_sha256: Option<String>,
    pub backup_path: Option<String>,
    pub payload_entry: Option<String>,
    pub expected_size: Option<i64>,
    pub applied_sha256: Option<String>,
    pub result_code: Option<String>,
    /// Registry value name.
    pub value_name: Option<String>,
    /// The kind and data written (registry), the entry text (PATH) or the
    /// link target (shortcut).
    pub value_kind: Option<String>,
    pub value_data: Option<String>,
    /// The undo record of a registry value: what was there before.
    pub previous_kind: Option<String>,
    pub previous_data: Option<String>,
    pub link_arguments: Option<String>,
    pub link_description: Option<String>,
    pub link_icon: Option<String>,
    /// The state a removal puts back, as distinct from the undo record of
    /// the operation itself: an environment variable's pre-installation
    /// value (`None` kind: it did not exist), carried by a keep so the
    /// commit records it again, and by a restore so the walk can write it.
    pub restore_kind: Option<String>,
    pub restore_data: Option<String>,
    pub link_working_directory: Option<String>,
    pub link_app_user_model_id: Option<String>,
}

/// What `prepare` records durably before a mutation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Undo {
    pub existed: bool,
    pub previous_sha256: Option<String>,
    pub backup_path: Option<String>,
    pub previous_kind: Option<String>,
    pub previous_data: Option<String>,
}

const TXN_COLUMNS: &str = "id, kind, from_version, to_version, package_id, package_version, metadata_sha256, scope, install_root, state, started_at, finished_at, registration_key, accepted_license_sha256";

/// The columns a reader selects: every one the file has, and `NULL` for
/// the schema-3 column a file a mutating run has not migrated yet lacks.
fn txn_select_columns(db: &Db) -> String {
    if db.has_schema(3) {
        TXN_COLUMNS.to_string()
    } else {
        TXN_COLUMNS.replace(", accepted_license_sha256", ", NULL")
    }
}

fn read_txn(row: &rusqlite::Row<'_>) -> rusqlite::Result<(TransactionRow, Option<Error>)> {
    let kind: String = row.get(1)?;
    let state: String = row.get(9)?;
    let mut problem = None;
    let kind = TxnKind::parse(&kind).unwrap_or_else(|err| {
        problem = Some(err);
        TxnKind::Install
    });
    let state = TxnState::parse(&state).unwrap_or_else(|err| {
        problem = Some(err);
        TxnState::Running
    });
    Ok((
        TransactionRow {
            id: row.get(0)?,
            kind,
            from_version: row.get(2)?,
            to_version: row.get(3)?,
            package_id: row.get(4)?,
            package_version: row.get(5)?,
            metadata_sha256: row.get(6)?,
            scope: row.get(7)?,
            install_root: row.get(8)?,
            state,
            started_at: row.get(10)?,
            finished_at: row.get(11)?,
            registration_key: row.get(12)?,
            accepted_license_sha256: row.get(13)?,
        },
        problem,
    ))
}

fn unwrap_parsed<T>(pair: (T, Option<Error>)) -> Result<T> {
    match pair {
        (value, None) => Ok(value),
        (_, Some(err)) => Err(err),
    }
}

/// The one transaction in a non-terminal state, if any.
pub fn open_transaction(db: &Db) -> Result<Option<TransactionRow>> {
    let mut statement = db.conn().prepare(&format!(
        "SELECT {} FROM \"transaction\" WHERE state NOT IN ('committed', 'rolled_back') ORDER BY started_at DESC LIMIT 1",
        txn_select_columns(db)
    ))?;
    let row = statement.query_row([], read_txn).optional()?;
    row.map(unwrap_parsed).transpose()
}

/// Inserts the transaction, its operations and the effective options in
/// one commit: mutating operations `planned`, keeps already `applied` with
/// the ownership they carry forward.
pub fn begin(
    db: &Db,
    txn: &TransactionRow,
    operations: &[PlannedOperation],
    options: &BTreeMap<String, OptionValue>,
) -> Result<()> {
    db.commit_unit(|sql| {
        sql.execute(
            &format!("INSERT INTO \"transaction\" ({TXN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?13)"),
            params![
                txn.id,
                txn.kind.as_str(),
                txn.from_version,
                txn.to_version,
                txn.package_id,
                txn.package_version,
                txn.metadata_sha256,
                txn.scope,
                txn.install_root,
                txn.state.as_str(),
                txn.started_at,
                txn.registration_key,
                txn.accepted_license_sha256,
            ],
        )?;
        let mut insert = sql.prepare(
            "INSERT INTO operation (transaction_id, sequence, kind, target, state, payload_entry, expected_size, previous_existed, applied_sha256, value_name, value_kind, value_data, link_arguments, link_description, link_icon, restore_kind, restore_data, link_working_directory, link_app_user_model_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        )?;
        for (index, op) in operations.iter().enumerate() {
            let state = if op.kind.is_keep() {
                OpState::Applied
            } else {
                OpState::Planned
            };
            insert.execute(params![
                txn.id,
                index as i64 + 1,
                op.kind.as_str(),
                op.target,
                state.as_str(),
                op.payload_entry,
                op.expected_size.map(|s| s as i64),
                op.previous_existed.map(|b| b as i64),
                op.applied_sha256,
                op.value_name,
                op.value_kind,
                op.value_data,
                op.link_arguments,
                op.link_description,
                op.link_icon,
                op.restore_kind,
                op.restore_data,
                op.link_working_directory,
                op.link_app_user_model_id,
            ])?;
        }
        let mut option = sql.prepare(
            "INSERT INTO transaction_option (transaction_id, name, value) VALUES (?1, ?2, ?3)",
        )?;
        for (name, value) in options {
            option.execute(params![txn.id, name, value.as_text()])?;
        }
        Ok(())
    })
}

/// All operations of a transaction in sequence order.
pub fn operations(db: &Db, transaction_id: &str) -> Result<Vec<OperationRow>> {
    let mut statement = db.conn().prepare(
        "SELECT transaction_id, sequence, kind, target, state, previous_existed, previous_sha256, backup_path, payload_entry, expected_size, applied_sha256, result_code, value_name, value_kind, value_data, previous_kind, previous_data, link_arguments, link_description, link_icon, restore_kind, restore_data, link_working_directory, link_app_user_model_id FROM operation WHERE transaction_id = ?1 ORDER BY sequence",
    )?;
    let rows = statement.query_map([transaction_id], |row| {
        let kind: String = row.get(2)?;
        let state: String = row.get(4)?;
        let previous_existed: Option<i64> = row.get(5)?;
        Ok((
            kind,
            state,
            OperationRow {
                transaction_id: row.get(0)?,
                sequence: row.get(1)?,
                kind: OpKind::InstallFile,
                target: row.get(3)?,
                state: OpState::Planned,
                previous_existed: previous_existed.map(|v| v != 0),
                previous_sha256: row.get(6)?,
                backup_path: row.get(7)?,
                payload_entry: row.get(8)?,
                expected_size: row.get(9)?,
                applied_sha256: row.get(10)?,
                result_code: row.get(11)?,
                value_name: row.get(12)?,
                value_kind: row.get(13)?,
                value_data: row.get(14)?,
                previous_kind: row.get(15)?,
                previous_data: row.get(16)?,
                link_arguments: row.get(17)?,
                link_description: row.get(18)?,
                link_icon: row.get(19)?,
                restore_kind: row.get(20)?,
                restore_data: row.get(21)?,
                link_working_directory: row.get(22)?,
                link_app_user_model_id: row.get(23)?,
            },
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (kind, state, mut op) = row?;
        op.kind = OpKind::parse(&kind)?;
        op.state = OpState::parse(&state)?;
        out.push(op);
    }
    Ok(out)
}

/// `planned → prepared` with the undo record.
pub fn mark_prepared(db: &Db, transaction_id: &str, sequence: i64, undo: &Undo) -> Result<()> {
    db.commit_unit(|sql| {
        sql.execute(
            "UPDATE operation SET state = 'prepared', previous_existed = ?3, previous_sha256 = ?4, backup_path = ?5, previous_kind = ?6, previous_data = ?7 WHERE transaction_id = ?1 AND sequence = ?2",
            params![
                transaction_id,
                sequence,
                undo.existed as i64,
                undo.previous_sha256,
                undo.backup_path,
                undo.previous_kind,
                undo.previous_data
            ],
        )?;
        Ok(())
    })
}

/// Any state transition without extra data.
pub fn mark_state(db: &Db, transaction_id: &str, sequence: i64, state: OpState) -> Result<()> {
    db.commit_unit(|sql| {
        sql.execute(
            "UPDATE operation SET state = ?3 WHERE transaction_id = ?1 AND sequence = ?2",
            params![transaction_id, sequence, state.as_str()],
        )?;
        Ok(())
    })
}

/// `→ applied` with what was written.
pub fn mark_applied(
    db: &Db,
    transaction_id: &str,
    sequence: i64,
    applied_sha256: Option<&str>,
    result_code: Option<&str>,
) -> Result<()> {
    db.commit_unit(|sql| {
        sql.execute(
            "UPDATE operation SET state = 'applied', applied_sha256 = ?3, result_code = ?4 WHERE transaction_id = ?1 AND sequence = ?2",
            params![transaction_id, sequence, applied_sha256, result_code],
        )?;
        Ok(())
    })
}

pub fn set_transaction_state(
    db: &Db,
    transaction_id: &str,
    state: TxnState,
    finished_at: Option<&str>,
) -> Result<()> {
    db.commit_unit(|sql| {
        sql.execute(
            "UPDATE \"transaction\" SET state = ?2, finished_at = COALESCE(?3, finished_at) WHERE id = ?1",
            params![transaction_id, state.as_str(), finished_at],
        )?;
        Ok(())
    })
}

const OWNERSHIP_TABLES: [&str; 10] = [
    "file",
    "directory",
    "registry_key",
    "registry_value",
    "path_entry",
    "shortcut",
    "environment_variable",
    "firewall_rule",
    "action",
    "installation_option",
];

/// The commit of an installing transaction: in one SQL transaction the
/// installation row is rewritten — with the licence acceptance the
/// transaction carries — the ownership tables are rewritten from the
/// journal (installed and kept resources become the owned state; removed
/// ones simply do not reappear), the effective options are recorded, and
/// the transaction becomes `committed`. Before this commit the database
/// says "not installed (or the previous version), with an open
/// transaction"; after it the database says the new version. Nothing in
/// between is ever visible, and a transaction that never gets here leaves
/// the previous installation row, acceptance included, exactly as it was.
pub fn commit_ownership(
    db: &Db,
    txn: &TransactionRow,
    operations: &[OperationRow],
    installation_id: &str,
    engine_version: &str,
    now: &str,
) -> Result<()> {
    db.commit_unit(|sql| {
        sql.execute("DELETE FROM installation", [])?;
        sql.execute(
            "INSERT INTO installation (id, product_id, version, scope, install_root, engine_version, committed_at, registration_key, accepted_license_sha256) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![installation_id, txn.package_id, txn.package_version, txn.scope, txn.install_root, engine_version, now, txn.registration_key, txn.accepted_license_sha256],
        )?;
        for table in OWNERSHIP_TABLES {
            sql.execute(&format!("DELETE FROM {table}"), [])?;
        }
        record_ownership(sql, txn, operations)?;
        sql.execute(
            "INSERT INTO installation_option (name, value) SELECT name, value FROM transaction_option WHERE transaction_id = ?1",
            params![txn.id],
        )?;
        sql.execute(
            "UPDATE \"transaction\" SET state = 'committed', finished_at = ?2 WHERE id = ?1",
            params![txn.id, now],
        )?;
        Ok(())
    })
}

fn record_ownership(
    sql: &Transaction<'_>,
    txn: &TransactionRow,
    operations: &[OperationRow],
) -> rusqlite::Result<()> {
    let mut file = sql.prepare(
        "INSERT OR REPLACE INTO file (path, sha256, size, owned_since) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut directory = sql.prepare(
        "INSERT OR REPLACE INTO directory (path, created, owned_since) VALUES (?1, ?2, ?3)",
    )?;
    let mut registry_key = sql.prepare(
        "INSERT OR REPLACE INTO registry_key (key, created, owned_since) VALUES (?1, ?2, ?3)",
    )?;
    let mut registry_value = sql.prepare(
        "INSERT OR REPLACE INTO registry_value (key, name, kind, data, pre_existed, previous_kind, previous_data, owned_since) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    let mut path_entry = sql.prepare(
        "INSERT OR REPLACE INTO path_entry (hive_key, raw, normalized, pre_existed, added, owned_since) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut shortcut = sql.prepare(
        "INSERT OR REPLACE INTO shortcut (path, target, owned_since) VALUES (?1, ?2, ?3)",
    )?;
    let mut environment_variable = sql.prepare(
        "INSERT OR REPLACE INTO environment_variable (hive_key, name, kind, data, pre_existed, previous_kind, previous_data, owned_since) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    let mut firewall_rule = sql.prepare(
        "INSERT OR REPLACE INTO firewall_rule (name, rule, owned_since) VALUES (?1, ?2, ?3)",
    )?;
    let mut action = sql.prepare(
        "INSERT OR REPLACE INTO action (name, phase, definition, artifact_sha256, artifact_file, artifact_size, owned_since) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let text = |value: &Option<String>| value.clone().unwrap_or_default();
    for op in operations {
        match op.kind {
            OpKind::InstallFile | OpKind::KeepFile => {
                file.execute(params![
                    op.target,
                    text(&op.applied_sha256),
                    op.expected_size.unwrap_or(0),
                    txn.id
                ])?;
            }
            OpKind::CreateDirectory | OpKind::KeepDirectory => {
                let created = !op.previous_existed.unwrap_or(true);
                directory.execute(params![op.target, created as i64, txn.id])?;
            }
            OpKind::CreateRegistryKey | OpKind::KeepRegistryKey => {
                let created = !op.previous_existed.unwrap_or(true);
                registry_key.execute(params![op.target, created as i64, txn.id])?;
            }
            // A registry value's row records its pre-installation state
            // the way an environment variable's does (below): carried in
            // `restore_*` by a keep and by a set of a value already owned,
            // found by `prepare` for a set of a value not owned yet.
            OpKind::SetRegistryValue | OpKind::KeepRegistryValue => {
                let (pre_existed, previous_kind, previous_data) = pre_installation_state(op);
                registry_value.execute(params![
                    op.target,
                    text(&op.value_name),
                    text(&op.value_kind),
                    text(&op.value_data),
                    pre_existed as i64,
                    previous_kind,
                    previous_data,
                    txn.id
                ])?;
            }
            OpKind::AddPathEntry | OpKind::KeepPathEntry => {
                let raw = text(&op.value_data);
                let pre_existed =
                    op.kind == OpKind::KeepPathEntry && op.previous_existed.unwrap_or(false);
                path_entry.execute(params![
                    op.target,
                    raw,
                    path::normalize(&raw),
                    pre_existed as i64,
                    (!pre_existed) as i64,
                    txn.id
                ])?;
            }
            OpKind::CreateShortcut | OpKind::KeepShortcut => {
                shortcut.execute(params![op.target, text(&op.value_data), txn.id])?;
            }
            // The ownership row's `previous_*` is the pre-installation value,
            // never a value TigerSetup itself wrote in an earlier
            // transaction: a set of a variable already owned, and every
            // keep, carry that state in `restore_*`; a set of a variable
            // TigerSetup did not own yet records what `prepare` found.
            OpKind::SetEnvironmentVariable | OpKind::KeepEnvironmentVariable => {
                let (pre_existed, previous_kind, previous_data) = pre_installation_state(op);
                environment_variable.execute(params![
                    op.target,
                    text(&op.value_name),
                    text(&op.value_kind),
                    text(&op.value_data),
                    pre_existed as i64,
                    previous_kind,
                    previous_data,
                    txn.id
                ])?;
            }
            OpKind::CreateFirewallRule | OpKind::KeepFirewallRule => {
                firewall_rule.execute(params![op.target, text(&op.value_data), txn.id])?;
            }
            // The row carries the definition, which also names the
            // packaged program the state directory keeps for it.
            OpKind::StoreAction => {
                let definition = text(&op.value_data);
                let decoded = crate::action::deserialize(&definition)
                    .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
                let packaged = decoded.is_packaged();
                action.execute(params![
                    op.target,
                    decoded.phase().as_str(),
                    definition,
                    packaged.then(|| decoded.sha256.clone()),
                    packaged.then(|| decoded.file_name.clone()),
                    packaged.then_some(decoded.size as i64),
                    txn.id
                ])?;
            }
            OpKind::RunAction => {}
            OpKind::RemoveFile
            | OpKind::RemoveDirectory
            | OpKind::RemoveRegistryKey
            | OpKind::RemoveRegistryValue
            | OpKind::RemovePathEntry
            | OpKind::RemoveShortcut
            | OpKind::RestoreEnvironmentVariable
            | OpKind::RemoveFirewallRule => {}
        }
    }
    Ok(())
}

/// The pre-installation state an ownership row records for a registry
/// value or an environment variable: whether it existed before TigerSetup
/// wrote it, and what it held. It is never a value TigerSetup itself wrote
/// in an earlier transaction: a set of a value already owned, and every
/// keep, carry the recorded state in `restore_*`; a set of a value
/// TigerSetup did not own yet records what `prepare` found.
fn pre_installation_state(op: &OperationRow) -> (bool, Option<String>, Option<String>) {
    match &op.restore_kind {
        Some(kind) if kind == crate::plan::RESTORE_ABSENT => (false, None, None),
        Some(kind) => (true, Some(kind.clone()), op.restore_data.clone()),
        None => {
            let existed = op.previous_existed.unwrap_or(false);
            (
                existed,
                existed.then(|| op.previous_kind.clone()).flatten(),
                existed.then(|| op.previous_data.clone()).flatten(),
            )
        }
    }
}

/// The commit of an uninstall: every ownership row and the installation row
/// go away together with the transaction becoming `committed`. Resources
/// the plan skipped (missing, or modified and preserved) are no longer
/// owned either: the product is absent.
pub fn commit_uninstall(db: &Db, txn: &TransactionRow, now: &str) -> Result<()> {
    db.commit_unit(|sql| {
        for table in OWNERSHIP_TABLES {
            sql.execute(&format!("DELETE FROM {table}"), [])?;
        }
        sql.execute("DELETE FROM installation", [])?;
        sql.execute(
            "UPDATE \"transaction\" SET state = 'committed', finished_at = ?2 WHERE id = ?1",
            params![txn.id, now],
        )?;
        Ok(())
    })
}
