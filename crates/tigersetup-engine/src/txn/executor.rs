//! The forward walk of a transaction — prepare → applying → apply → applied
//! for every operation — its commit, and the cleanup afterwards. The same
//! walk serves a fresh run (every operation `planned`) and a forward recovery
//! (operations in whatever state the crash left them).
//!
//! The hard invariant at every step: undo state is durable before the
//! mutation, and the mutation is durable before the journal says it happened.
//! Every resource follows it identically — a file, a directory, a registry
//! key or value, a PATH entry, an environment variable, a shortcut and a
//! firewall rule differ only in what "the previous state" is and in which
//! Windows call performs the mutation. A custom action walks the same
//! states with no undo of its own (`txn::actions`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tigersetup_format::PayloadArchive;

use crate::plan::{RESTORE_ABSENT, absolute};
use crate::report::{ActionInfo, Finding, Phase, Progress, Reporter};
use crate::resource::{directory, file, firewall, path, shortcut};
use crate::scope::{self, Locations};
use crate::state::journal::{self, OpKind, OpState, OperationRow, TransactionRow, TxnKind, Undo};
use crate::state::{Db, installation};
use crate::txn::fault::{FaultInjector, FaultPoint};
use crate::win::firewall::{Rule, Store};
use crate::win::fs::{self, Inspection};
use crate::win::registry::{self as winreg, Data, KeyPath, Roots};
use crate::win::shortcut::Link;
use crate::{Error, Result};

#[derive(Debug, Default, Clone, Copy)]
pub struct ForwardStats {
    /// Operations applied for the first time.
    pub applied: u32,
    /// Operations whose apply step was re-run or reconciled during recovery.
    pub reapplied: u32,
}

pub struct Executor<'a, 'r> {
    pub(crate) db: &'a Db,
    pub(crate) txn: TransactionRow,
    pub(crate) install_root: PathBuf,
    /// The state directory, which keeps the programs of the installation's
    /// uninstall actions beside the database.
    pub(crate) state_dir: PathBuf,
    pub(crate) staging_dir: PathBuf,
    pub(crate) payload: Option<PayloadArchive>,
    pub(crate) roots: Roots,
    pub(crate) fault: &'a mut FaultInjector,
    pub(crate) reporter: &'a mut Reporter<'r>,
    /// The locations the transaction's scope allows. A row naming a
    /// registry key or a shortcut outside them is refused before the
    /// Windows call, however it got into the journal.
    pub(crate) locations: Locations,
    /// What the walk preserved instead of mutating; reaches the outcome
    /// document, not only the log.
    pub(crate) findings: Vec<Finding>,
    /// Set when a PATH value or an environment variable changed, so the
    /// environment broadcast happens once at the end of the walk rather
    /// than per entry.
    pub(crate) environment_changed: bool,
    /// Set when a value under the scope's classes, capabilities or `App
    /// Paths` changed, so the shell is told once after the commit.
    pub(crate) associations_changed: bool,
    /// The firewall store the transaction's rules go to.
    pub(crate) firewall: Store,
    /// A client's cancellation flag, read at every operation boundary of the
    /// forward walk. A rollback and a recovery never read it: both must
    /// converge on a complete state.
    cancel: Option<Arc<AtomicBool>>,
    /// Whether the run is unattended, which the actions are told.
    pub(crate) quiet: bool,
    /// What every custom action the walk ran produced, in order; reaches
    /// the outcome document.
    pub(crate) actions: Vec<ActionInfo>,
    /// Set when an action asked for a restart.
    pub(crate) reboot_required: bool,
    /// Operations the current forward walk has finished, and how many it
    /// has to do, so that every applied operation carries progress.
    applied_operations: u64,
    total_operations: u64,
}

/// `<state directory>\txn-<id>`.
pub fn staging_dir_for(state_dir: &Path, transaction_id: &str) -> PathBuf {
    state_dir.join(format!("txn-{transaction_id}"))
}

/// Keeps are journaled `applied` when the transaction begins and never
/// walked; reaching one in a walk means the journal is inconsistent.
fn keep_reached(op: &OperationRow) -> Error {
    Error::new(
        "journal_inconsistent",
        format!(
            "operation {} ({}) is a keep but was found {}",
            op.sequence,
            op.kind.as_str(),
            op.state.as_str()
        ),
    )
}

fn inconsistent(op: &OperationRow, what: &str) -> Error {
    Error::new(
        "journal_inconsistent",
        format!("operation {} has no {what}", op.sequence),
    )
}

/// The registry key an operation addresses (the environment key for a PATH
/// entry), confined to the transaction's scope.
pub(crate) fn key_of(locations: &Locations, op: &OperationRow) -> Result<KeyPath> {
    let key = KeyPath::parse(&op.target)?;
    locations.confine_key(&key)?;
    Ok(key)
}

/// The registry value name an operation addresses.
pub(crate) fn value_name_of(op: &OperationRow) -> Result<&str> {
    op.value_name
        .as_deref()
        .ok_or_else(|| inconsistent(op, "value name"))
}

/// The data the operation writes.
pub(crate) fn written_data(op: &OperationRow) -> Result<Data> {
    let kind = op
        .value_kind
        .as_deref()
        .ok_or_else(|| inconsistent(op, "value kind"))?;
    let data = op
        .value_data
        .as_deref()
        .ok_or_else(|| inconsistent(op, "value data"))?;
    Data::from_columns(kind, data)
}

/// The data the undo record says was there before; `None` when nothing was.
pub(crate) fn previous_data(op: &OperationRow) -> Result<Option<Data>> {
    match (op.previous_existed, &op.previous_kind, &op.previous_data) {
        (Some(true), Some(kind), Some(data)) => Ok(Some(Data::from_columns(kind, data)?)),
        (Some(true), _, _) => Err(inconsistent(op, "previous value")),
        _ => Ok(None),
    }
}

/// The PATH entry text an operation adds or removes.
pub(crate) fn path_entry_of(op: &OperationRow) -> Result<&str> {
    op.value_data
        .as_deref()
        .ok_or_else(|| inconsistent(op, "path entry"))
}

/// The link an operation writes. A row written before the working
/// directory travelled in the journal has none recorded; the target's
/// directory is what those links were given.
pub(crate) fn link_of(op: &OperationRow) -> Result<Link> {
    let target = op
        .value_data
        .clone()
        .ok_or_else(|| inconsistent(op, "link target"))?;
    let working_directory = match &op.link_working_directory {
        Some(directory) => directory.clone(),
        None if crate::win::shortcut::is_url_shortcut(Path::new(&op.target)) => String::new(),
        None => Path::new(&target)
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    };
    Ok(Link {
        target,
        arguments: op.link_arguments.clone().unwrap_or_default(),
        description: op.link_description.clone().unwrap_or_default(),
        icon: op.link_icon.clone().unwrap_or_default(),
        working_directory,
        app_user_model_id: op.link_app_user_model_id.clone().unwrap_or_default(),
    })
}

/// The state an environment-variable restore puts back: `None` for a
/// variable TigerSetup created, else the recorded pre-installation value.
pub(crate) fn restore_data(op: &OperationRow) -> Result<Option<Data>> {
    match (&op.restore_kind, &op.restore_data) {
        (Some(kind), _) if kind == RESTORE_ABSENT => Ok(None),
        (Some(kind), Some(data)) => Ok(Some(Data::from_columns(kind, data)?)),
        _ => Err(inconsistent(op, "restore state")),
    }
}

/// The firewall rule an operation writes or removes.
pub(crate) fn rule_of(op: &OperationRow) -> Result<Rule> {
    firewall::rule_of(
        op.value_data.as_deref(),
        &format!("operation {}", op.sequence),
    )
}

/// The firewall rule the undo record says was there before, if any.
pub(crate) fn previous_rule(op: &OperationRow) -> Result<Option<Rule>> {
    match (op.previous_existed, &op.previous_data) {
        (Some(true), Some(text)) => Ok(Some(Rule::deserialize(text)?)),
        (Some(true), None) => Err(inconsistent(op, "previous firewall rule")),
        _ => Ok(None),
    }
}

/// Whether a registry key is one whose values the shell caches: the
/// scope's classes, its capability registrations and `App Paths`.
fn is_association_key(key: &KeyPath) -> bool {
    let lower = key.subkey.to_ascii_lowercase();
    lower.starts_with("software\\classes\\")
        || lower.starts_with("software\\registeredapplications")
        || lower.contains("\\capabilities")
        || lower.contains("\\app paths\\")
}

impl<'a, 'r> Executor<'a, 'r> {
    pub fn new(
        db: &'a Db,
        txn: TransactionRow,
        state_dir: &Path,
        payload: Option<PayloadArchive>,
        fault: &'a mut FaultInjector,
        reporter: &'a mut Reporter<'r>,
    ) -> Executor<'a, 'r> {
        Executor {
            db,
            install_root: PathBuf::from(&txn.install_root),
            state_dir: state_dir.to_path_buf(),
            staging_dir: staging_dir_for(state_dir, &txn.id),
            // A journal row whose scope is not a scope at all confines
            // to user scope, the narrower of the two: it refuses every
            // machine location rather than trusting the row.
            locations: scope::locations(
                tigersetup_format::identity::Scope::parse(&txn.scope)
                    .unwrap_or(tigersetup_format::identity::Scope::User),
            ),
            txn,
            payload,
            roots: Roots::from_env(),
            fault,
            reporter,
            findings: Vec::new(),
            environment_changed: false,
            associations_changed: false,
            firewall: Store::from_env(),
            cancel: None,
            quiet: true,
            actions: Vec::new(),
            reboot_required: false,
            applied_operations: 0,
            total_operations: 0,
        }
    }

    /// Tells the actions whether the run is unattended.
    pub fn unattended(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// What the walk's custom actions produced, and whether one asked for
    /// a restart.
    pub fn take_actions(&mut self) -> (Vec<ActionInfo>, bool) {
        (
            std::mem::take(&mut self.actions),
            std::mem::replace(&mut self.reboot_required, false),
        )
    }

    /// Lets a client stop the forward walk between operations; the caller
    /// then rolls the transaction back.
    pub fn cancellable(mut self, cancel: Option<Arc<AtomicBool>>) -> Self {
        self.cancel = cancel;
        self
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_deref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    pub fn transaction(&self) -> &TransactionRow {
        &self.txn
    }

    /// Records something the walk preserved rather than mutated, in the log
    /// and in the findings the outcome document reports.
    pub(crate) fn note(&mut self, code: &'static str, path: &Path) {
        self.reporter.event(code, path.display().to_string());
        self.findings.push(Finding::at(code, path));
    }

    /// As [`Executor::note`], for a resource named by something other than a
    /// filesystem path.
    pub(crate) fn note_named(&mut self, code: &'static str, name: String) {
        self.reporter.event(code, name.clone());
        self.findings.push(Finding::named(code, name));
    }

    pub fn take_findings(&mut self) -> Vec<Finding> {
        std::mem::take(&mut self.findings)
    }

    pub(crate) fn backup_path(&self, sequence: i64) -> PathBuf {
        self.staging_dir.join("backup").join(sequence.to_string())
    }

    /// The absolute path of a file, directory or shortcut operation.
    pub(crate) fn file_target(&self, op: &OperationRow) -> Result<PathBuf> {
        match op.kind {
            OpKind::CreateShortcut | OpKind::RemoveShortcut | OpKind::KeepShortcut => {
                let link = shortcut::link_path(&op.target)?;
                self.locations.confine_shortcut(&link)?;
                Ok(link)
            }
            _ => absolute(&self.install_root, &op.target),
        }
    }

    /// The registry key an operation addresses, confined to this
    /// transaction's scope.
    pub(crate) fn key_of(&self, op: &OperationRow) -> Result<KeyPath> {
        key_of(&self.locations, op)
    }

    /// `<key>\<name>` for a report.
    fn value_location(op: &OperationRow) -> String {
        format!(
            "{}\\{}",
            op.target,
            op.value_name.as_deref().unwrap_or_default()
        )
    }

    fn describe(op: &OperationRow) -> String {
        format!(
            "sequence={} kind={} target={}",
            op.sequence,
            op.kind.as_str(),
            op.target
        )
    }

    /// Walks every operation forward. `recovering` marks a restart, where an
    /// operation may already be `prepared` or `applying`.
    pub fn run_forward(&mut self, recovering: bool) -> Result<ForwardStats> {
        fs::create_directory(&self.staging_dir.join("backup"))?;
        let operations = journal::operations(self.db, &self.txn.id)?;
        self.applied_operations = 0;
        self.total_operations = operations
            .iter()
            .filter(|op| op.state != OpState::Applied)
            .count() as u64;
        let mut stats = ForwardStats::default();
        for op in operations {
            if op.state != OpState::Applied && self.cancelled() {
                return Err(Error::new(
                    "cancelled",
                    "the run was cancelled before this operation",
                ));
            }
            match op.state {
                OpState::Applied => {}
                OpState::Planned => {
                    let op = self.prepare(op)?;
                    self.apply(&op, false)?;
                    stats.applied += 1;
                }
                OpState::Prepared => {
                    self.apply(&op, recovering)?;
                    if recovering {
                        stats.reapplied += 1;
                    } else {
                        stats.applied += 1;
                    }
                }
                OpState::Applying => {
                    if !recovering {
                        return Err(Error::new(
                            "journal_inconsistent",
                            format!("operation {} is applying in a fresh run", op.sequence),
                        ));
                    }
                    if self.reconcile_applying(&op)? {
                        stats.reapplied += 1;
                    }
                }
                OpState::RollingBack | OpState::RolledBack | OpState::RollbackFailed => {
                    return Err(Error::new(
                        "journal_inconsistent",
                        format!(
                            "operation {} is {} during a forward walk",
                            op.sequence,
                            op.state.as_str()
                        ),
                    ));
                }
            }
        }
        self.broadcast_environment();
        Ok(stats)
    }

    /// Tells running applications that the environment changed, and the
    /// shell that associations did, once per walk.
    pub(crate) fn broadcast_environment(&mut self) {
        if self.environment_changed {
            self.environment_changed = false;
            if crate::win::env::broadcast_environment_change() {
                self.reporter.event("environment_broadcast", "Environment");
            }
        }
        if self.associations_changed {
            self.associations_changed = false;
            if crate::win::env::notify_association_change() {
                self.reporter
                    .event("association_change_notified", "SHCNE_ASSOCCHANGED");
            }
        }
    }

    /// Records the undo state durably: for a file or a shortcut a flushed
    /// backup copy and the previous hash, for a registry value the previous
    /// kind and data, for a PATH entry the whole previous `Path` text, and
    /// for a key or a directory whether it was there at all.
    fn prepare(&mut self, mut op: OperationRow) -> Result<OperationRow> {
        let undo = match op.kind {
            OpKind::InstallFile
            | OpKind::RemoveFile
            | OpKind::CreateShortcut
            | OpKind::RemoveShortcut => {
                let target = self.file_target(&op)?;
                match fs::inspect(&target)? {
                    Inspection::Absent => Undo::default(),
                    Inspection::Present { sha256, .. } => {
                        let backup = self.backup_path(op.sequence);
                        fs::copy_and_flush(&target, &backup)?;
                        Undo {
                            existed: true,
                            previous_sha256: Some(sha256),
                            backup_path: Some(backup.display().to_string()),
                            ..Undo::default()
                        }
                    }
                }
            }
            OpKind::CreateDirectory | OpKind::RemoveDirectory => Undo {
                existed: directory::exists(&self.file_target(&op)?),
                ..Undo::default()
            },
            OpKind::CreateRegistryKey | OpKind::RemoveRegistryKey => Undo {
                existed: winreg::key_exists(&self.roots, &self.key_of(&op)?)?,
                ..Undo::default()
            },
            OpKind::SetRegistryValue | OpKind::RemoveRegistryValue => {
                let key = self.key_of(&op)?;
                match winreg::read_value(&self.roots, &key, value_name_of(&op)?)? {
                    None => Undo::default(),
                    Some(data) => Undo {
                        existed: true,
                        previous_kind: Some(data.kind_name()),
                        previous_data: Some(data.text()),
                        ..Undo::default()
                    },
                }
            }
            OpKind::AddPathEntry | OpKind::RemovePathEntry => {
                let key = self.key_of(&op)?;
                let (text, exists) = path::read(&self.roots, &key)?;
                Undo {
                    existed: exists,
                    previous_kind: exists.then(|| "expand_string".to_string()),
                    previous_data: exists.then_some(text),
                    ..Undo::default()
                }
            }
            // An environment variable's undo is the value it holds now,
            // exactly as a registry value's is.
            OpKind::SetEnvironmentVariable | OpKind::RestoreEnvironmentVariable => {
                let key = self.key_of(&op)?;
                match winreg::read_value(&self.roots, &key, value_name_of(&op)?)? {
                    None => Undo::default(),
                    Some(data) => Undo {
                        existed: true,
                        previous_kind: Some(data.kind_name()),
                        previous_data: Some(data.text()),
                        ..Undo::default()
                    },
                }
            }
            // A firewall rule's undo is the same-named rule that is there
            // now, whole, so a rollback can put it back attribute for
            // attribute.
            OpKind::CreateFirewallRule | OpKind::RemoveFirewallRule => {
                match self.firewall.list(&op.target)?.into_iter().next() {
                    None => Undo::default(),
                    Some(rule) => Undo {
                        existed: true,
                        previous_kind: Some("firewall_rule".to_string()),
                        previous_data: Some(rule.serialize()),
                        ..Undo::default()
                    },
                }
            }
            OpKind::RunAction | OpKind::StoreAction => self.prepare_action(&op)?,
            OpKind::KeepFile
            | OpKind::KeepDirectory
            | OpKind::KeepRegistryKey
            | OpKind::KeepRegistryValue
            | OpKind::KeepPathEntry
            | OpKind::KeepShortcut
            | OpKind::KeepEnvironmentVariable
            | OpKind::KeepFirewallRule => return Err(keep_reached(&op)),
        };
        journal::mark_prepared(self.db, &self.txn.id, op.sequence, &undo)?;
        op.state = OpState::Prepared;
        op.previous_existed = Some(undo.existed);
        op.previous_sha256 = undo.previous_sha256;
        op.backup_path = undo.backup_path;
        op.previous_kind = undo.previous_kind;
        op.previous_data = undo.previous_data;
        self.fault.at(
            FaultPoint::AfterPrepare,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        Ok(op)
    }

    /// `applying` → mutation → `applied`.
    fn apply(&mut self, op: &OperationRow, reapply: bool) -> Result<()> {
        journal::mark_state(self.db, &self.txn.id, op.sequence, OpState::Applying)?;
        self.fault.at(
            FaultPoint::AfterApplying,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        self.mutate(op)?;
        self.applied_operations += 1;
        self.reporter.progress(
            if reapply {
                "operation_reapplied"
            } else {
                "operation_applied"
            },
            Self::describe(op),
            Progress {
                phase: Phase::Applying,
                done: self.applied_operations,
                total: self.total_operations,
                target: op.target.clone(),
            },
        );
        self.fault.at(
            FaultPoint::AfterApplied,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )
    }

    /// Performs the mutation and journals `applied`. Every arm inspects the
    /// target first where inspection changes what it does, so re-running a
    /// mutation after a crash is safe.
    fn mutate(&mut self, op: &OperationRow) -> Result<()> {
        match op.kind {
            OpKind::InstallFile => {
                let target = self.file_target(op)?;
                let sha256 = self.write_file(op, &target)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, Some(&sha256), None)
            }
            OpKind::CreateDirectory => {
                directory::create(&self.file_target(op)?)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::RemoveFile => {
                fs::remove_file(&self.file_target(op)?)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::RemoveDirectory => {
                let target = self.file_target(op)?;
                let result_code = if directory::remove_if_empty(&target)? {
                    None
                } else {
                    self.note("directory_not_empty_preserved", &target);
                    Some("directory_not_empty_preserved")
                };
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, result_code)
            }
            OpKind::CreateRegistryKey => {
                winreg::create_key(&self.roots, &self.key_of(op)?)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::RemoveRegistryKey => {
                let key = self.key_of(op)?;
                let result_code = if winreg::delete_key_if_empty(&self.roots, &key)? {
                    None
                } else {
                    self.note_named("registry_key_not_empty_preserved", key.to_string());
                    Some("registry_key_not_empty_preserved")
                };
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, result_code)
            }
            OpKind::SetRegistryValue => {
                let key = self.key_of(op)?;
                winreg::write_value(&self.roots, &key, value_name_of(op)?, &written_data(op)?)?;
                self.associations_changed |= is_association_key(&key);
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::RemoveRegistryValue => {
                let key = self.key_of(op)?;
                let name = value_name_of(op)?.to_string();
                let recorded = previous_data(op)?;
                let current = winreg::read_value(&self.roots, &key, &name)?;
                let result_code = match (&current, &recorded) {
                    (None, _) => None,
                    (Some(current), Some(recorded)) if current == recorded => {
                        winreg::delete_value(&self.roots, &key, &name)?;
                        self.associations_changed |= is_association_key(&key);
                        None
                    }
                    _ => {
                        // A value someone changed after the plan was made is
                        // never deleted, whatever the journal intended.
                        self.note_named(
                            "registry_value_modified_preserved",
                            Self::value_location(op),
                        );
                        Some("registry_value_modified_preserved")
                    }
                };
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, result_code)
            }
            OpKind::AddPathEntry => {
                let key = self.key_of(op)?;
                self.environment_changed |= path::add(&self.roots, &key, path_entry_of(op)?)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::RemovePathEntry => {
                let key = self.key_of(op)?;
                self.environment_changed |=
                    path::remove(&self.roots, &key, path_entry_of(op)?, true)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::CreateShortcut => {
                let target = self.file_target(op)?;
                let link = link_of(op)?;
                let sha256 = crate::win::shortcut::write(&target, &link)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, Some(&sha256), None)
            }
            OpKind::RemoveShortcut => {
                let target = self.file_target(op)?;
                let recorded_target = op.value_data.clone().unwrap_or_default();
                let result_code = match crate::win::shortcut::inspect(&target)? {
                    crate::win::shortcut::LinkInspection::Absent => None,
                    crate::win::shortcut::LinkInspection::Link(current)
                        if shortcut::removable(
                            &target,
                            &current,
                            &recorded_target,
                            &self.install_root,
                        ) =>
                    {
                        fs::remove_file(&target)?;
                        None
                    }
                    _ => {
                        self.note("shortcut_modified_preserved", &target);
                        Some("shortcut_modified_preserved")
                    }
                };
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, result_code)
            }
            OpKind::SetEnvironmentVariable => {
                let key = self.key_of(op)?;
                // The environment key is Windows's own and is always there
                // on a real machine; a hive without it (a fresh relocated
                // one) gets it created, never owned, as the PATH resource
                // does.
                if !winreg::key_exists(&self.roots, &key)? {
                    winreg::create_key(&self.roots, &key)?;
                }
                winreg::write_value(&self.roots, &key, value_name_of(op)?, &written_data(op)?)?;
                self.environment_changed = true;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::RestoreEnvironmentVariable => {
                let key = self.key_of(op)?;
                let name = value_name_of(op)?.to_string();
                let written = written_data(op)?;
                let restore = restore_data(op)?;
                let current = winreg::read_value(&self.roots, &key, &name)?;
                let result_code = match current {
                    // Already gone, or already the pre-installation value.
                    None => None,
                    Some(ref current) if Some(current) == restore.as_ref() => None,
                    Some(ref current) if *current == written => {
                        match &restore {
                            Some(previous) => {
                                winreg::write_value(&self.roots, &key, &name, previous)?
                            }
                            None => winreg::delete_value(&self.roots, &key, &name)?,
                        }
                        self.environment_changed = true;
                        None
                    }
                    Some(_) => {
                        // Somebody changed it since TigerSetup wrote it:
                        // their value stays.
                        self.note_named(
                            "environment_variable_modified_preserved",
                            Self::value_location(op),
                        );
                        Some("environment_variable_modified_preserved")
                    }
                };
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, result_code)
            }
            OpKind::CreateFirewallRule => {
                let rule = rule_of(op)?;
                self.require_firewall(op)?;
                self.firewall.put(&rule)?;
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)
            }
            OpKind::RemoveFirewallRule => {
                let recorded = rule_of(op)?;
                self.require_firewall(op)?;
                let result_code = match self.firewall.list(&op.target)?.as_slice() {
                    [] => None,
                    [current] if firewall::matches(current, &recorded) => {
                        self.firewall.remove(&op.target)?;
                        None
                    }
                    [_] => {
                        self.note_named("firewall_rule_modified_preserved", op.target.clone());
                        Some("firewall_rule_modified_preserved")
                    }
                    _ => {
                        self.note_named("firewall_rule_ambiguous_preserved", op.target.clone());
                        Some("firewall_rule_ambiguous_preserved")
                    }
                };
                journal::mark_applied(self.db, &self.txn.id, op.sequence, None, result_code)
            }
            OpKind::RunAction => self.run_action(op),
            OpKind::StoreAction => self.store_action(op),
            OpKind::KeepFile
            | OpKind::KeepDirectory
            | OpKind::KeepRegistryKey
            | OpKind::KeepRegistryValue
            | OpKind::KeepPathEntry
            | OpKind::KeepShortcut
            | OpKind::KeepEnvironmentVariable
            | OpKind::KeepFirewallRule => Err(keep_reached(op)),
        }
    }

    /// A firewall operation planned by a run that could write the store is
    /// walked only by one that still can; a recovery from a process that
    /// lost that right stops here rather than pretending.
    fn require_firewall(&self, op: &OperationRow) -> Result<()> {
        if self.firewall.is_writable() {
            return Ok(());
        }
        Err(Error::new(
            "firewall_requires_elevation",
            format!(
                "operation {} on firewall rule {:?} needs an administrator",
                op.sequence, op.target
            ),
        ))
    }

    /// Write the entry to `<target>.tigersetup-new`, `FlushFileBuffers`,
    /// rename write-through over the target. Returns the SHA-256 written.
    fn write_file(&mut self, op: &OperationRow, target: &Path) -> Result<String> {
        let entry = op
            .payload_entry
            .as_deref()
            .ok_or_else(|| inconsistent(op, "payload entry"))?;
        let payload = self.payload.as_mut().ok_or_else(|| {
            Error::new(
                "payload_unavailable",
                "this run carries no payload for the open transaction",
            )
        })?;
        let mut staged =
            file::stage_from_payload(target, payload, entry, op.expected_size.map(|s| s as u64))?;
        self.fault.at(
            FaultPoint::AfterWriteBeforeFlush,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        if self.fault.skip_flush(op.sequence) {
            self.reporter.event("flush_skipped", Self::describe(op));
        } else {
            staged.flush()?;
        }
        self.fault.at(
            FaultPoint::AfterFlushBeforeRename,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        let sha256 = staged.sha256.clone();
        staged.commit()?;
        self.fault.at(
            FaultPoint::AfterRename,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        Ok(sha256)
    }

    /// An operation found `applying` on restart: inspect the target and
    /// decide whether the mutation happened. Returns whether it was re-done.
    fn reconcile_applying(&mut self, op: &OperationRow) -> Result<bool> {
        self.fault.at(
            FaultPoint::AfterApplying,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        let reapplied = match op.kind {
            OpKind::InstallFile => {
                let target = self.file_target(op)?;
                let entry = op
                    .payload_entry
                    .as_deref()
                    .ok_or_else(|| inconsistent(op, "payload entry"))?;
                match fs::inspect(&target)? {
                    Inspection::Absent => {
                        self.reporter.event(
                            "file_missing",
                            format!(
                                "{} (recovering operation {})",
                                target.display(),
                                op.sequence
                            ),
                        );
                        self.mutate(op)?;
                        true
                    }
                    Inspection::Present { sha256, size } => {
                        let payload = self.payload.as_mut().ok_or_else(|| {
                            Error::new(
                                "payload_unavailable",
                                "this run carries no payload for the open transaction",
                            )
                        })?;
                        let expected = file::payload_sha256(payload, entry)?;
                        if sha256 == expected {
                            journal::mark_applied(
                                self.db,
                                &self.txn.id,
                                op.sequence,
                                Some(&sha256),
                                None,
                            )?;
                            self.reporter
                                .event("operation_completed", Self::describe(op));
                            false
                        } else {
                            if op.previous_sha256.as_deref() != Some(sha256.as_str()) {
                                self.reporter.event(
                                    "file_content_mismatch",
                                    format!(
                                        "{} has sha256 {sha256} ({size} bytes), expected {expected} (recovering operation {})",
                                        target.display(),
                                        op.sequence
                                    ),
                                );
                            }
                            self.mutate(op)?;
                            true
                        }
                    }
                }
            }
            OpKind::RemoveFile => {
                let target = self.file_target(op)?;
                match fs::inspect(&target)? {
                    Inspection::Absent => {
                        journal::mark_applied(self.db, &self.txn.id, op.sequence, None, None)?;
                        self.reporter
                            .event("operation_completed", Self::describe(op));
                        false
                    }
                    Inspection::Present { sha256, .. }
                        if op.previous_sha256.as_deref() == Some(sha256.as_str()) =>
                    {
                        self.mutate(op)?;
                        true
                    }
                    Inspection::Present { .. } => {
                        // The file changed between the journal's `applying`
                        // and this restart: an owned file someone modified is
                        // never deleted, whatever the journal intended.
                        self.note("file_modified_preserved", &target);
                        journal::mark_applied(
                            self.db,
                            &self.txn.id,
                            op.sequence,
                            None,
                            Some("file_modified_preserved"),
                        )?;
                        false
                    }
                }
            }
            // Every other mutation inspects its target before it acts, so
            // performing it again is how it is reconciled.
            OpKind::CreateDirectory
            | OpKind::RemoveDirectory
            | OpKind::CreateRegistryKey
            | OpKind::RemoveRegistryKey
            | OpKind::SetRegistryValue
            | OpKind::RemoveRegistryValue
            | OpKind::AddPathEntry
            | OpKind::RemovePathEntry
            | OpKind::CreateShortcut
            | OpKind::RemoveShortcut
            | OpKind::SetEnvironmentVariable
            | OpKind::RestoreEnvironmentVariable
            | OpKind::CreateFirewallRule
            | OpKind::RemoveFirewallRule
            | OpKind::StoreAction => {
                self.mutate(op)?;
                true
            }
            OpKind::RunAction => self.reconcile_action(op)?,
            OpKind::KeepFile
            | OpKind::KeepDirectory
            | OpKind::KeepRegistryKey
            | OpKind::KeepRegistryValue
            | OpKind::KeepPathEntry
            | OpKind::KeepShortcut
            | OpKind::KeepEnvironmentVariable
            | OpKind::KeepFirewallRule => return Err(keep_reached(op)),
        };
        if reapplied {
            self.reporter
                .event("operation_reapplied", Self::describe(op));
        }
        self.fault.at(
            FaultPoint::AfterApplied,
            Some(op.sequence),
            &op.target,
            self.reporter,
        )?;
        Ok(reapplied)
    }

    /// The single SQL transaction that turns the journal into installation
    /// state (or removes it) and marks the transaction committed.
    pub fn commit(&mut self, installation_id: &str, engine_version: &str) -> Result<()> {
        // Everything this transaction wrote must be on disk before the
        // database says it happened. Files were flushed as they were
        // written; the registry is flushed here, once, because Windows
        // writes it back lazily.
        winreg::flush(&self.roots, self.locations.hive)?;
        self.fault
            .at(FaultPoint::BeforeCommit, None, "", self.reporter)?;
        let now = crate::report::now_rfc3339();
        match self.txn.kind {
            TxnKind::Install | TxnKind::Upgrade | TxnKind::Reinstall | TxnKind::Repair => {
                let operations = journal::operations(self.db, &self.txn.id)?;
                journal::commit_ownership(
                    self.db,
                    &self.txn,
                    &operations,
                    installation_id,
                    engine_version,
                    &now,
                )?;
            }
            TxnKind::Uninstall => journal::commit_uninstall(self.db, &self.txn, &now)?,
        }
        self.txn.state = journal::TxnState::Committed;
        self.txn.finished_at = Some(now);
        let files = installation::file_count(self.db)?;
        self.reporter.event(
            "transaction_committed",
            format!(
                "transaction={} kind={} owned_files={files}",
                self.txn.id,
                self.txn.kind.as_str()
            ),
        );
        self.fault.at(
            FaultPoint::AfterCommitBeforeCleanup,
            None,
            "",
            self.reporter,
        )
    }

    /// Removes the staging area and any temporary left under the install
    /// root, and the stored action programs nothing owns any more. Safe to
    /// repeat; a crash here is repaired by the next run.
    pub fn cleanup(&mut self) {
        for swept in fs::sweep_temp_files(&self.install_root) {
            self.reporter
                .event("temp_file_swept", swept.display().to_string());
        }
        self.sweep_action_store();
        if self.staging_dir.exists() {
            match std::fs::remove_dir_all(&self.staging_dir) {
                Ok(()) => self
                    .reporter
                    .event("staging_removed", self.staging_dir.display().to_string()),
                Err(err) => self.reporter.event(
                    "staging_cleanup_failed",
                    format!("{}: {err}", self.staging_dir.display()),
                ),
            }
        }
    }
}
