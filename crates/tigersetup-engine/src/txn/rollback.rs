//! The reverse walk. Every undo inspects the target before acting, so
//! running the rollback twice — or resuming one that was interrupted — does
//! the same thing as running it once. The walk's unit is the journal batch
//! the plan assigned: a batch goes `rolling_back` in one commit, its
//! operations are undone in reverse order, and it goes `rolled_back` in one
//! more; an operation with no batch goes on its own.

use std::path::Path;

use crate::resource::{directory, file, firewall, path};
use crate::state::journal::{self, OpKind, OpState, OperationRow, TxnState};
use crate::txn::executor::{
    Executor, path_entry_of, previous_data, previous_rule, restore_data, rule_of, value_name_of,
    written_data,
};
use crate::txn::fault::FaultPoint;
use crate::win::fs;
use crate::win::registry as winreg;
use crate::{Error, Result};

#[derive(Debug, Default, Clone, Copy)]
pub struct RollbackStats {
    pub rolled_back: u32,
}

impl Executor<'_, '_> {
    /// Rolls the transaction back to the state before it began.
    pub fn rollback(&mut self) -> Result<RollbackStats> {
        if self.txn.state != TxnState::RollingBack {
            journal::set_transaction_state(self.db, &self.txn.id, TxnState::RollingBack, None)?;
            self.txn.state = TxnState::RollingBack;
            self.reporter.event(
                "transaction_rolling_back",
                format!("transaction={}", self.txn.id),
            );
        }
        for swept in fs::sweep_temp_files(&self.install_root) {
            self.reporter
                .event("temp_file_swept", swept.display().to_string());
        }
        let operations = journal::operations(self.db, &self.txn.id)?;
        let mut stats = RollbackStats::default();
        // The units, last first: consecutive operations of one batch that
        // have something to undo, or one operation on its own.
        let mut units: Vec<Vec<&OperationRow>> = Vec::new();
        for op in operations.iter().rev() {
            if op.kind.is_keep() || matches!(op.state, OpState::Planned | OpState::RolledBack) {
                continue;
            }
            match (op.batch, units.last_mut()) {
                (Some(batch), Some(unit))
                    if unit.last().is_some_and(|last| last.batch == Some(batch)) =>
                {
                    unit.push(op)
                }
                _ => units.push(vec![op]),
            }
        }
        for unit in units {
            let sequences: Vec<i64> = unit
                .iter()
                .filter(|op| op.state != OpState::RollingBack)
                .map(|op| op.sequence)
                .collect();
            journal::mark_states(self.db, &self.txn.id, &sequences, OpState::RollingBack)?;
            for op in &unit {
                let undone = self.undo(op).and_then(|()| {
                    self.fault.at(
                        FaultPoint::AfterRollbackUndo,
                        Some(op.sequence),
                        &op.target,
                        self.reporter,
                    )
                });
                if let Err(err) = undone {
                    journal::mark_state(
                        self.db,
                        &self.txn.id,
                        op.sequence,
                        OpState::RollbackFailed,
                    )?;
                    journal::set_transaction_state(
                        self.db,
                        &self.txn.id,
                        TxnState::RollbackFailed,
                        None,
                    )?;
                    self.txn.state = TxnState::RollbackFailed;
                    self.reporter.event(
                        "operation_rollback_failed",
                        format!(
                            "sequence={} kind={} target={}: {err}",
                            op.sequence,
                            op.kind.as_str(),
                            op.target
                        ),
                    );
                    return Err(Error::new(
                        "rollback_failed",
                        format!("operation {} could not be undone: {err}", op.sequence),
                    ));
                }
                self.reporter.event(
                    "operation_rolled_back",
                    format!(
                        "sequence={} kind={} target={}",
                        op.sequence,
                        op.kind.as_str(),
                        op.target
                    ),
                );
                stats.rolled_back += 1;
            }
            let sequences: Vec<i64> = unit.iter().map(|op| op.sequence).collect();
            journal::mark_states(self.db, &self.txn.id, &sequences, OpState::RolledBack)?;
        }
        self.broadcast_environment();
        let now = crate::report::now_rfc3339();
        journal::set_transaction_state(self.db, &self.txn.id, TxnState::RolledBack, Some(&now))?;
        self.txn.state = TxnState::RolledBack;
        self.txn.finished_at = Some(now);
        self.reporter.event(
            "transaction_rolled_back",
            format!(
                "transaction={} kind={} operations={}",
                self.txn.id,
                self.txn.kind.as_str(),
                stats.rolled_back
            ),
        );
        Ok(stats)
    }

    /// Restores the target to its recorded previous state, whatever the
    /// operation got to. Every arm inspects before it acts, so running a
    /// rollback twice — or resuming an interrupted one — does the same thing
    /// as running it once.
    fn undo(&mut self, op: &OperationRow) -> Result<()> {
        let inconsistent = || {
            Error::new(
                "journal_inconsistent",
                format!("operation {} has no undo record", op.sequence),
            )
        };
        match op.kind {
            // A shortcut is a file: its undo is the file undo, on the .lnk.
            OpKind::InstallFile
            | OpKind::RemoveFile
            | OpKind::CreateShortcut
            | OpKind::RemoveShortcut => {
                let target = self.file_target(op)?;
                match op.previous_existed.ok_or_else(inconsistent)? {
                    true => {
                        let previous = op.previous_sha256.as_deref().ok_or_else(inconsistent)?;
                        if !file::matches(&target, previous)? {
                            let backup = op.backup_path.as_deref().ok_or_else(inconsistent)?;
                            fs::restore_from_backup(Path::new(backup), &target)?;
                        }
                        Ok(())
                    }
                    false => fs::remove_file(&target),
                }
            }
            OpKind::CreateDirectory => {
                let target = self.file_target(op)?;
                if op.previous_existed == Some(false) && !directory::remove_if_empty(&target)? {
                    self.note("directory_not_empty_preserved", &target);
                }
                Ok(())
            }
            OpKind::RemoveDirectory => {
                let target = self.file_target(op)?;
                if op.previous_existed == Some(true) && !directory::exists(&target) {
                    directory::create(&target)?;
                }
                Ok(())
            }
            OpKind::CreateRegistryKey => {
                let key = self.key_of(op)?;
                if op.previous_existed == Some(false)
                    && !winreg::delete_key_if_empty(&self.roots, &key)?
                {
                    self.note_named("registry_key_not_empty_preserved", key.to_string());
                }
                Ok(())
            }
            OpKind::RemoveRegistryKey => {
                let key = self.key_of(op)?;
                if op.previous_existed == Some(true) && !winreg::key_exists(&self.roots, &key)? {
                    winreg::create_key(&self.roots, &key)?;
                }
                Ok(())
            }
            OpKind::SetRegistryValue | OpKind::RemoveRegistryValue => self.undo_registry_value(op),
            // An environment variable is a registry value with the same undo;
            // the environment is then told once, at the end of the walk.
            OpKind::SetEnvironmentVariable | OpKind::RestoreEnvironmentVariable => {
                self.undo_registry_value(op)?;
                self.environment_changed = true;
                Ok(())
            }
            OpKind::CreateFirewallRule | OpKind::RemoveFirewallRule => self.undo_firewall_rule(op),
            OpKind::RunAction | OpKind::StoreAction => self.undo_action(op),
            OpKind::AddPathEntry => {
                let key = self.key_of(op)?;
                // The value is deleted when it empties only if TigerSetup
                // created it; a `Path` that was there stays, empty or not.
                let existed = op.previous_existed.ok_or_else(inconsistent)?;
                self.environment_changed |=
                    path::remove(&self.roots, &key, path_entry_of(op)?, !existed)?;
                Ok(())
            }
            OpKind::RemovePathEntry => {
                let key = self.key_of(op)?;
                if op.previous_existed == Some(true) {
                    self.environment_changed |= path::add(&self.roots, &key, path_entry_of(op)?)?;
                }
                Ok(())
            }
            OpKind::KeepFile
            | OpKind::KeepDirectory
            | OpKind::KeepRegistryKey
            | OpKind::KeepRegistryValue
            | OpKind::KeepPathEntry
            | OpKind::KeepShortcut
            | OpKind::KeepEnvironmentVariable
            | OpKind::KeepFirewallRule => Ok(()),
        }
    }

    /// Puts a firewall rule back as it was: the recorded previous rule, or
    /// none. A same-named rule that is now neither the previous one nor the
    /// one this transaction wrote was changed by someone else and stays.
    fn undo_firewall_rule(&mut self, op: &OperationRow) -> Result<()> {
        let previous = previous_rule(op)?;
        let written = match op.kind {
            OpKind::CreateFirewallRule => Some(rule_of(op)?),
            _ => None,
        };
        let current = self.firewall.list(&op.target)?;
        match current.as_slice() {
            [] => {
                if let Some(previous) = &previous {
                    self.firewall.put(previous)?;
                }
                Ok(())
            }
            [rule]
                if previous
                    .as_ref()
                    .is_some_and(|p| firewall::matches(rule, p)) =>
            {
                Ok(())
            }
            [rule] if written.as_ref().is_some_and(|w| firewall::matches(rule, w)) => {
                match &previous {
                    Some(previous) => self.firewall.put(previous)?,
                    None => {
                        self.firewall.remove(&op.target)?;
                    }
                }
                Ok(())
            }
            _ => {
                self.note_named("firewall_rule_modified_preserved", op.target.clone());
                Ok(())
            }
        }
    }

    /// Puts a registry value back as it was. A value whose data is now
    /// neither what was recorded nor what this transaction wrote was changed
    /// by someone else: it is preserved and reported.
    fn undo_registry_value(&mut self, op: &OperationRow) -> Result<()> {
        let key = self.key_of(op)?;
        let name = value_name_of(op)?.to_string();
        let previous = previous_data(op)?;
        let written = match op.kind {
            OpKind::SetRegistryValue | OpKind::SetEnvironmentVariable => Some(written_data(op)?),
            // A removal that restored a pre-installation value wrote that
            // value; one that deleted the value wrote nothing.
            OpKind::RemoveRegistryValue | OpKind::RestoreEnvironmentVariable => {
                match op.restore_kind {
                    Some(_) => restore_data(op)?,
                    None => None,
                }
            }
            _ => None,
        };
        let current = winreg::read_value(&self.roots, &key, &name)?;
        if current == previous {
            return Ok(());
        }
        if current.is_none() || current == written {
            match &previous {
                Some(data) => winreg::write_value(&self.roots, &key, &name, data)?,
                None => winreg::delete_value(&self.roots, &key, &name)?,
            }
            return Ok(());
        }
        self.note_named(
            "registry_value_modified_preserved",
            format!("{key}\\{name}"),
        );
        Ok(())
    }
}
