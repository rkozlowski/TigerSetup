//! Planning: desired state from the metadata and the effective options,
//! owned state from the database and actual state on the machine become an
//! ordered list of typed operations. Install, upgrade, reinstall, repair and
//! uninstall are the same reconciliation with different inputs: install has
//! no owned side, uninstall has no desired side, the others have both.
//!
//! Forward order: directories and files, product registry keys and values
//! (the typed integrations among them), PATH entries, environment variables,
//! shortcuts, firewall rules, and the Add/Remove Programs registration last —
//! a registration means "installed" to Windows. Removals follow in the
//! reverse resource order, so an uninstall unregisters first and removes
//! the install root last. A package's custom actions bracket all of that:
//! the `pre-*` actions are the first operations, the `post-*` actions the
//! last, with the records of the uninstall actions an installing
//! transaction keeps just before the `post-install` ones.
//!
//! Every optional resource is gated by the same predicate
//! (`resource::predicate`), so an option that controls files, a PATH mode,
//! an integration or a firewall rule is one mechanism, and a component is an
//! option that gates files.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use tigersetup_format::identity::Scope;
use tigersetup_format::metadata::{Action, OptionValue};
use tigersetup_format::{Metadata, Payload};

use crate::action::{self, ActionPlan};
use crate::report::Finding;
use crate::resource::environment::DesiredVariable;
use crate::resource::predicate::{self, Options};
use crate::resource::registry::DesiredValue;
use crate::resource::shortcut::DesiredShortcut;
use crate::resource::{
    directory, environment, firewall, integration, path, registration, registry, shortcut,
};
use crate::scope::{self, Locations};
use crate::state::installation::{Owned, OwnedDirectory, OwnedFile};
use crate::state::journal::OpKind;
use crate::win::firewall::{Rule, Store};
use crate::win::fs::{self, Inspection};
use crate::win::registry::{self as winreg, Data, KeyPath, Roots};
use crate::win::shortcut::{Link, LinkInspection};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedOperation {
    pub kind: OpKind,
    /// The resource identity of the kind (see `journal::OperationRow`).
    pub target: String,
    pub payload_entry: Option<String>,
    pub expected_size: Option<u64>,
    /// Keeps only: the hash the kept file is owned by.
    pub applied_sha256: Option<String>,
    /// Keeps only: the last-write time the kept file is owned with.
    pub applied_modified: Option<i64>,
    /// Keeps only: the record carried forward (`false` for a directory or
    /// key TigerSetup created, `true` for a kept file or a PATH entry that
    /// pre-existed).
    pub previous_existed: Option<bool>,
    pub value_name: Option<String>,
    pub value_kind: Option<String>,
    pub value_data: Option<String>,
    pub link_arguments: Option<String>,
    pub link_description: Option<String>,
    pub link_icon: Option<String>,
    /// Environment variables: the pre-installation state a removal puts
    /// back and a keep carries forward — a kind and data, or the kind
    /// [`RESTORE_ABSENT`] for a variable TigerSetup created. `None` on a set
    /// of a variable TigerSetup did not own yet, whose prepare step records
    /// what it finds.
    pub restore_kind: Option<String>,
    pub restore_data: Option<String>,
    /// Shortcuts: the AppUserModelID written into the link.
    pub link_app_user_model_id: Option<String>,
    /// Shortcuts: the working directory written into the link.
    pub link_working_directory: Option<String>,
}

/// The `restore_kind` of an environment variable that did not exist before
/// TigerSetup set it: the removal deletes it.
pub const RESTORE_ABSENT: &str = "absent";

/// Only so that the constructors below can fill the fields their kind does
/// not use; every one of them sets `kind` itself.
impl Default for PlannedOperation {
    fn default() -> PlannedOperation {
        PlannedOperation {
            kind: OpKind::InstallFile,
            target: String::new(),
            payload_entry: None,
            expected_size: None,
            applied_sha256: None,
            applied_modified: None,
            previous_existed: None,
            value_name: None,
            value_kind: None,
            value_data: None,
            link_arguments: None,
            link_description: None,
            link_icon: None,
            restore_kind: None,
            restore_data: None,
            link_app_user_model_id: None,
            link_working_directory: None,
        }
    }
}

impl PlannedOperation {
    fn new(kind: OpKind, target: impl Into<String>) -> PlannedOperation {
        PlannedOperation {
            kind,
            target: target.into(),
            ..Default::default()
        }
    }

    fn install_file(target: String, entry: &str, size: u64) -> PlannedOperation {
        PlannedOperation {
            payload_entry: Some(entry.to_string()),
            expected_size: Some(size),
            ..PlannedOperation::new(OpKind::InstallFile, target)
        }
    }

    fn path_entry(kind: OpKind, environment_key: &KeyPath, raw: &str) -> PlannedOperation {
        PlannedOperation {
            value_name: Some(path::VALUE_NAME.to_string()),
            value_data: Some(raw.to_string()),
            ..PlannedOperation::new(kind, environment_key.to_string())
        }
    }

    fn shortcut(kind: OpKind, link_path: &Path, link: &Link) -> PlannedOperation {
        PlannedOperation {
            value_data: Some(link.target.clone()),
            link_arguments: Some(link.arguments.clone()),
            link_description: Some(link.description.clone()),
            link_icon: Some(link.icon.clone()),
            link_app_user_model_id: Some(link.app_user_model_id.clone()),
            link_working_directory: Some(link.working_directory.clone()),
            ..PlannedOperation::new(kind, link_path.display().to_string())
        }
    }

    /// A registry-value or environment-variable operation: the key, the
    /// name, the data TigerSetup writes (or wrote), and the pre-installation
    /// state to put back where it is known — a kind and data, `Some(None)`
    /// for a value TigerSetup created, or `None` on a set of a value
    /// TigerSetup does not own yet, whose prepare step records what it
    /// finds.
    fn value(
        kind: OpKind,
        key: &KeyPath,
        name: &str,
        data: &Data,
        restore: Option<Option<&Data>>,
    ) -> PlannedOperation {
        let (restore_kind, restore_data) = match restore {
            None => (None, None),
            Some(None) => (Some(RESTORE_ABSENT.to_string()), None),
            Some(Some(previous)) => (Some(previous.kind_name()), Some(previous.text())),
        };
        PlannedOperation {
            value_name: Some(name.to_string()),
            value_kind: Some(data.kind_name()),
            value_data: Some(data.text()),
            restore_kind,
            restore_data,
            ..PlannedOperation::new(kind, key.to_string())
        }
    }

    fn firewall_rule(kind: OpKind, rule: &Rule) -> PlannedOperation {
        PlannedOperation {
            value_kind: Some("firewall_rule".to_string()),
            value_data: Some(rule.serialize()),
            ..PlannedOperation::new(kind, rule.name.clone())
        }
    }

    /// A custom action to run, or to keep for the installation's uninstall:
    /// the name is the target, the definition travels in `value_data`, and a
    /// packaged program's entry and size are where a file's would be.
    fn action(kind: OpKind, action: &Action) -> PlannedOperation {
        PlannedOperation {
            value_kind: Some(action::VALUE_KIND.to_string()),
            value_data: Some(action::serialize(action)),
            payload_entry: action.is_packaged().then(|| action.entry.clone()),
            expected_size: action.is_packaged().then_some(action.size),
            ..PlannedOperation::new(kind, action.name.clone())
        }
    }

    /// The store operation that records a quiescence entry, with its
    /// packaged programs, for the installation's uninstall.
    fn quiescence(entry: &tigersetup_format::metadata::Quiescence) -> PlannedOperation {
        PlannedOperation {
            value_kind: Some(action::QUIESCENCE_VALUE_KIND.to_string()),
            value_data: Some(action::serialize_quiescence(entry)),
            ..PlannedOperation::new(OpKind::StoreAction, entry.name.clone())
        }
    }
}

/// Metadata paths use `/`; the database and the journal use `\`.
pub fn to_relative(path: &str) -> String {
    path.replace('/', "\\")
}

/// Joins an install-relative path onto the root, after checking that it is a
/// valid install-relative path: no `..` or `.` component, not absolute, no
/// drive, no reserved device name, no forbidden character — the same rule the
/// format applies to metadata paths. Paths read back from the database go
/// through here before any write or delete, so a tampered row can never
/// address anything outside the install root.
pub fn absolute(install_root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty() {
        return Ok(install_root.to_path_buf());
    }
    let outside = |why: String| {
        Error::new(
            "path_outside_root",
            format!(
                "stored path {relative:?} cannot be used under {}: {why}",
                install_root.display()
            ),
        )
    };
    tigersetup_format::metadata::validate_relative_path(&relative.replace('\\', "/"))
        .map_err(|err| outside(err.message))?;
    let joined = install_root.join(relative);
    if !joined.starts_with(install_root) {
        return Err(outside("it does not stay under the install root".into()));
    }
    Ok(joined)
}

/// Windows paths compare case-insensitively.
fn key(path: &str) -> String {
    path.to_ascii_lowercase()
}

/// Every directory on the path of one install-relative file, shallowest
/// first. This is the ancestry of a path the database already holds, not the
/// rule that decides which directories a package needs — that one belongs to
/// the builder, and its result travels in the metadata.
fn ancestors_of(path: &str) -> Vec<String> {
    let components: Vec<&str> = path.split('\\').collect();
    (1..components.len())
        .map(|depth| components[..depth].join("\\"))
        .collect()
}

fn depth(path: &str) -> usize {
    if path.is_empty() {
        0
    } else {
        path.matches('\\').count() + 1
    }
}

/// The directories a package needs for the files it wants now: the root
/// first, then shallowest first so parents precede children. The metadata's
/// list is authoritative about which directories exist — the format
/// guarantees it covers every file's parents, so the rule that derives
/// directories from file paths lives in the builder alone — and a declared
/// directory is wanted while a desired file lies below it, so a component's
/// directories come and go with its files.
fn desired_directories(metadata: &Metadata, files: &[DesiredFile]) -> Vec<String> {
    let needed: BTreeSet<String> = files
        .iter()
        .flat_map(|f| ancestors_of(&f.path))
        .map(|d| key(&d))
        .collect();
    let directories: BTreeSet<String> = metadata
        .directories
        .iter()
        .map(|d| to_relative(&d.path))
        .filter(|d| needed.contains(&key(d)))
        .collect();
    let mut ordered: Vec<String> = directories.into_iter().collect();
    ordered.sort_by(|a, b| depth(a).cmp(&depth(b)).then_with(|| a.cmp(b)));
    ordered.insert(0, String::new());
    ordered
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredFile {
    pub path: String,
    pub entry: String,
    pub size: u64,
}

/// The state the metadata and the effective options want on this machine.
#[derive(Debug, Clone)]
pub struct Desired {
    pub locations: Locations,
    pub directories: Vec<String>,
    pub files: Vec<DesiredFile>,
    /// The product's `[[registry]]` values and the values every enabled
    /// integration compiles to, in that order.
    pub registry_values: Vec<DesiredValue>,
    /// Raw PATH entry texts, each once.
    pub path_entries: Vec<String>,
    pub environment_variables: Vec<DesiredVariable>,
    pub shortcuts: Vec<DesiredShortcut>,
    pub firewall_rules: Vec<Rule>,
    pub registration_key: KeyPath,
    pub registration_values: Vec<DesiredValue>,
    /// The declared integrations as compiled, for the reports.
    pub integrations: Vec<integration::Integration>,
    /// What resolving the desired state found worth saying: a shortcut
    /// folder this scope does not have, a URL scheme another application
    /// owns.
    pub findings: Vec<Finding>,
}

/// The effective value of every declared option: an explicit value wins,
/// then the recorded one, then the declared default. An explicit option
/// the package does not declare, or a value the option does not take, is
/// refused; a recorded value the option no longer takes — a choice a newer
/// package dropped — falls back to the default.
pub fn effective_options(
    metadata: &Metadata,
    recorded: &Options,
    explicit: &Options,
) -> Result<Options> {
    for (name, value) in explicit {
        let Some(option) = metadata.option(name) else {
            return Err(Error::new(
                "option_unknown",
                format!("{} declares no option {name:?}", metadata.package().name),
            ));
        };
        if OptionValue::from_text(option, &value.as_text()).is_none() {
            return Err(Error::new(
                "option_value_invalid",
                format!(
                    "option {name} takes {}, not {:?}",
                    option.accepted_values().join(", "),
                    value.as_text()
                ),
            ));
        }
    }
    Ok(metadata
        .options
        .iter()
        .map(|option| {
            let name = option.name.to_ascii_lowercase();
            let value = explicit
                .get(&name)
                .or_else(|| recorded.get(&name))
                .and_then(|value| OptionValue::from_text(option, &value.as_text()))
                .unwrap_or_else(|| option.default_value());
            (name, value)
        })
        .collect())
}

/// Resolves the desired state for a run. The machine and the owned state
/// are consulted only where a declaration's meaning depends on them: a URL
/// scheme is registered as a class only where nothing else owns the key.
pub fn desired(
    metadata: &Metadata,
    options: &Options,
    scope: Scope,
    install_root: &Path,
    uninstaller: &Path,
    roots: &Roots,
    owned: &Owned,
) -> Result<Desired> {
    let locations = scope::locations(scope);
    let (registration_key, registration_values) = registration::values(
        metadata,
        &locations,
        install_root,
        uninstaller,
        &registration::install_date(),
    )?;
    let mut path_entries: Vec<String> = Vec::new();
    for entry in metadata
        .path_entries
        .iter()
        .filter(|entry| predicate::enabled(entry.when.as_ref(), &entry.option, options))
    {
        let raw = absolute(install_root, &to_relative(&entry.path))
            .map(|p| p.display().to_string().trim_end_matches('\\').to_string())?;
        // One directory may be declared under two values of a choice
        // option ("bin" for both PATH modes that want it); it is one entry.
        if !path_entries
            .iter()
            .any(|existing| path::normalize(existing) == path::normalize(&raw))
        {
            path_entries.push(raw);
        }
    }
    let files: Vec<DesiredFile> = metadata
        .files
        .iter()
        .filter(|f| predicate::enabled(f.when.as_ref(), "", options))
        .map(|f| DesiredFile {
            path: to_relative(&f.path),
            entry: f.entry.clone(),
            size: f.size,
        })
        .collect();
    let mut registry_values =
        registry::product_values(metadata, options, &locations, install_root)?;
    let integrations =
        integration::desired(metadata, options, &locations, install_root, roots, owned)?;
    registry_values.extend(integrations.values());
    let (shortcuts, mut findings) = shortcut::desired(metadata, options, &locations, install_root)?;
    findings.extend(integrations.findings);
    Ok(Desired {
        directories: desired_directories(metadata, &files),
        files,
        registry_values,
        path_entries,
        environment_variables: environment::desired(metadata, options, &locations, install_root),
        shortcuts,
        firewall_rules: firewall::desired(metadata, options, install_root)?,
        registration_key,
        registration_values,
        integrations: integrations.items,
        findings,
        locations,
    })
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PlanCounts {
    pub kept: usize,
    pub replaced: usize,
    pub added: usize,
    pub removed: usize,
    pub directories_created: usize,
    pub directories_removed: usize,
    /// Owned files rewritten by a repair.
    pub repaired: usize,
    /// Mutating operations on registry keys and values, PATH, environment
    /// variables, shortcuts and firewall rules.
    pub resource_operations: usize,
    /// Custom actions the transaction runs.
    pub actions: usize,
    /// Uninstall-phase actions an installing transaction records.
    pub stored_actions: usize,
}

pub struct Plan {
    pub operations: Vec<PlannedOperation>,
    pub findings: Vec<Finding>,
    pub counts: PlanCounts,
}

/// The inputs of one reconciliation.
pub struct Reconcile<'a> {
    /// `None` for an uninstall.
    pub desired: Option<&'a Desired>,
    pub owned: &'a Owned,
    pub install_root: &'a Path,
    /// Needed to compare files on disk with the payload.
    pub payload: Option<&'a mut Payload>,
    pub roots: &'a Roots,
    pub scope: Scope,
    /// Inspect files on disk (an installation exists); a first install
    /// writes every file without looking.
    pub inspect_files: bool,
    /// Report owned files that are rewritten as `file_repaired`.
    pub repair: bool,
    /// Where the scope keeps its shortcuts, for deciding whether a stored
    /// link is still inside it. Passed in rather than resolved here, because
    /// the caller already resolved them and a unit test can then describe a
    /// machine of its own. The comparison itself is `scope::is_inside`, the
    /// one the executor uses.
    pub shortcut_folders: &'a [PathBuf],
    /// The firewall store this run may write, or `None` when it may not —
    /// an unelevated run — in which case every declared rule is reported
    /// as skipped and every owned one is carried forward untouched.
    pub firewall: Option<&'a Store>,
    /// The custom actions of this run: what runs before the resource
    /// operations, what runs after them, and what an installing run
    /// records for the uninstall.
    pub actions: &'a ActionPlan,
}

/// Reconciles desired and owned state into operations.
pub fn reconcile(mut input: Reconcile<'_>) -> Result<Plan> {
    let mut plan = Plan {
        operations: Vec::new(),
        findings: input
            .desired
            .map(|d| d.findings.clone())
            .unwrap_or_default(),
        counts: PlanCounts::default(),
    };
    let mut removals: Vec<PlannedOperation> = Vec::new();
    let owned = input.owned;
    let locations = scope::locations(input.scope);

    // The `pre-*` actions, ahead of everything the run changes.
    for action in &input.actions.before {
        plan.counts.actions += 1;
        plan.operations
            .push(PlannedOperation::action(OpKind::RunAction, action));
    }
    let registration_key = input
        .desired
        .map(|d| d.registration_key.to_string())
        .or_else(|| owned.registration_key.clone());

    // Directories and files.
    let mut still_desired_directories = BTreeSet::new();
    let owned_directory_by_key: HashMap<String, &OwnedDirectory> = owned
        .directories
        .iter()
        .map(|d| (key(&d.path), d))
        .collect();
    let mut desired_file_keys = BTreeSet::new();
    if let Some(desired) = input.desired {
        for directory in &desired.directories {
            still_desired_directories.insert(key(directory));
            match owned_directory_by_key.get(&key(directory)) {
                Some(existing)
                    if !input.inspect_files
                        || directory::exists(&absolute(input.install_root, directory)?) =>
                {
                    plan.operations.push(PlannedOperation {
                        previous_existed: Some(!existing.created),
                        ..PlannedOperation::new(OpKind::KeepDirectory, directory.clone())
                    })
                }
                _ => {
                    plan.counts.directories_created += 1;
                    plan.operations.push(PlannedOperation::new(
                        OpKind::CreateDirectory,
                        directory.clone(),
                    ));
                }
            }
        }
        let owned_file_by_key: HashMap<String, &OwnedFile> =
            owned.files.iter().map(|f| (key(&f.path), f)).collect();
        let payload = input.payload.take();
        for desired_file in &desired.files {
            desired_file_keys.insert(key(&desired_file.path));
            let owned_file = owned_file_by_key.get(&key(&desired_file.path)).copied();
            let is_owned = owned_file.is_some();
            let target = absolute(input.install_root, &desired_file.path)?;
            let install = PlannedOperation::install_file(
                desired_file.path.clone(),
                &desired_file.entry,
                desired_file.size,
            );
            if !input.inspect_files {
                plan.counts.added += 1;
                plan.operations.push(install);
                continue;
            }
            // What the machine holds: the owned record where the file is
            // still the one TigerSetup wrote, else its hash. What the
            // package brings: the index, which the metadata carries.
            let inspection = fs::inspect_unless_unchanged(
                &target,
                owned_file.and_then(OwnedFile::fingerprint),
                owned_file.map(|f| f.sha256.as_str()).unwrap_or(""),
            )?;
            match inspection {
                Inspection::Present { sha256, .. } => {
                    let payload = payload.as_deref().ok_or_else(|| {
                        Error::new("payload_unavailable", "this run carries no payload")
                    })?;
                    let wanted = payload.region(&desired_file.entry)?.sha256;
                    if sha256 == wanted {
                        plan.counts.kept += 1;
                        plan.operations.push(PlannedOperation {
                            expected_size: Some(desired_file.size),
                            applied_sha256: Some(wanted),
                            applied_modified: fs::fingerprint(&target)?.map(|f| f.modified),
                            previous_existed: Some(true),
                            ..PlannedOperation::new(OpKind::KeepFile, desired_file.path.clone())
                        });
                    } else {
                        plan.counts.replaced += 1;
                        if input.repair && is_owned {
                            plan.counts.repaired += 1;
                            plan.findings.push(Finding::at("file_repaired", &target));
                        }
                        plan.operations.push(install);
                    }
                }
                Inspection::Absent => {
                    plan.counts.added += 1;
                    if input.repair && is_owned {
                        plan.counts.repaired += 1;
                        plan.findings.push(Finding::at("file_repaired", &target));
                    }
                    plan.operations.push(install);
                }
            }
        }
    }

    // Product registry keys and values.
    let (product_values, registration_values): (Vec<_>, Vec<_>) = owned
        .registry_values
        .iter()
        .partition(|v| !registry::is_registration_key(&v.key, registration_key.as_deref()));
    let mut keys_kept_or_created: BTreeSet<String> = BTreeSet::new();
    let mut product_value_removals = Vec::new();
    if let Some(desired) = input.desired {
        // Every key is created down from the deepest root Windows itself
        // owns — `Software`, `Software\Classes`, `App Paths`, … — so that
        // TigerSetup never claims a key of Windows's own, only what lies
        // below it. The keys are grouped by that root.
        let needed: Vec<KeyPath> = distinct_keys(desired.registry_values.iter().map(|v| &v.key));
        let mut by_root: Vec<(KeyPath, Vec<KeyPath>)> = Vec::new();
        for key in needed {
            let root = locations.key_chain_root(&key);
            match by_root.iter_mut().find(|(r, _)| r.key() == root.key()) {
                Some((_, keys)) => keys.push(key),
                None => by_root.push((root, vec![key])),
            }
        }
        for (root, keys) in &by_root {
            reconcile_keys(
                keys,
                root,
                input.roots,
                owned,
                &mut keys_kept_or_created,
                &mut plan,
            )?;
        }
    }
    reconcile_values(
        input.desired.map(|d| &d.registry_values[..]).unwrap_or(&[]),
        &product_values,
        input.roots,
        input.repair,
        &mut plan,
        &mut product_value_removals,
    )?;

    // PATH entries.
    let environment_key = locations.environment_key.clone();
    let (current_path, _) = path::read(input.roots, &environment_key)?;
    let mut path_removals = Vec::new();
    let mut desired_path_keys = BTreeSet::new();
    if let Some(desired) = input.desired {
        for raw in &desired.path_entries {
            let normalized = path::normalize(raw);
            desired_path_keys.insert(normalized.clone());
            let owned_entry = owned.path_entries.iter().find(|e| {
                e.hive_key
                    .eq_ignore_ascii_case(&environment_key.to_string())
                    && e.normalized == normalized
            });
            let present = path::contains_equivalent(&current_path, &normalized);
            match owned_entry {
                Some(entry) if entry.pre_existed => plan.operations.push(PlannedOperation {
                    previous_existed: Some(true),
                    ..PlannedOperation::path_entry(
                        OpKind::KeepPathEntry,
                        &environment_key,
                        &entry.raw,
                    )
                }),
                Some(entry) if present => plan.operations.push(PlannedOperation {
                    previous_existed: Some(false),
                    ..PlannedOperation::path_entry(
                        OpKind::KeepPathEntry,
                        &environment_key,
                        &entry.raw,
                    )
                }),
                None if present => plan.operations.push(PlannedOperation {
                    previous_existed: Some(true),
                    ..PlannedOperation::path_entry(OpKind::KeepPathEntry, &environment_key, raw)
                }),
                _ => {
                    plan.counts.resource_operations += 1;
                    plan.operations.push(PlannedOperation::path_entry(
                        OpKind::AddPathEntry,
                        &environment_key,
                        raw,
                    ));
                }
            }
        }
    }
    for entry in &owned.path_entries {
        if desired_path_keys.contains(&entry.normalized) || !entry.added {
            continue;
        }
        let Ok(hive_key) = KeyPath::parse(&entry.hive_key) else {
            continue;
        };
        let (text, _) = if hive_key == environment_key {
            (current_path.clone(), true)
        } else {
            path::read(input.roots, &hive_key)?
        };
        if path::find_owned(&text, &entry.raw, &entry.normalized).is_some() {
            plan.counts.resource_operations += 1;
            path_removals.push(PlannedOperation::path_entry(
                OpKind::RemovePathEntry,
                &hive_key,
                &entry.raw,
            ));
        } else {
            plan.findings.push(Finding::named(
                "path_entry_missing",
                format!("{hive_key}\\{} {}", path::VALUE_NAME, entry.raw),
            ));
        }
    }

    // Environment variables.
    let mut environment_restores = Vec::new();
    let mut desired_variable_keys = BTreeSet::new();
    if let Some(desired) = input.desired {
        for wanted in &desired.environment_variables {
            let identity = (wanted.key.key(), wanted.name.to_ascii_lowercase());
            desired_variable_keys.insert(identity.clone());
            let owned_variable = owned.environment_variables.iter().find(|v| {
                v.hive_key.eq_ignore_ascii_case(&wanted.key.to_string())
                    && v.name.eq_ignore_ascii_case(&wanted.name)
            });
            let current = winreg::read_value(input.roots, &wanted.key, &wanted.name)?;
            match owned_variable {
                Some(record) => {
                    let restore = environment::restore_data(record)?;
                    let written = environment::owned_data(record)?;
                    if current.as_ref() == Some(&wanted.data) {
                        plan.operations.push(PlannedOperation::value(
                            OpKind::KeepEnvironmentVariable,
                            &wanted.key,
                            &wanted.name,
                            &wanted.data,
                            Some(restore.as_ref()),
                        ));
                    } else if current.is_none()
                        || current.as_ref() == Some(&written)
                        || input.repair
                    {
                        // Missing, or still what TigerSetup wrote and the
                        // desired data changed (a new version, a new root),
                        // or a repair: rewrite, keeping the pre-installation
                        // state to restore.
                        plan.counts.resource_operations += 1;
                        plan.operations.push(PlannedOperation::value(
                            OpKind::SetEnvironmentVariable,
                            &wanted.key,
                            &wanted.name,
                            &wanted.data,
                            Some(restore.as_ref()),
                        ));
                    } else {
                        // Somebody changed it since: their value stays, the
                        // record stays, and the run says so.
                        plan.findings.push(Finding::named(
                            "environment_variable_modified_preserved",
                            environment::location(&record.hive_key, &record.name),
                        ));
                        plan.operations.push(PlannedOperation::value(
                            OpKind::KeepEnvironmentVariable,
                            &wanted.key,
                            &wanted.name,
                            &written,
                            Some(restore.as_ref()),
                        ));
                    }
                }
                None if current.as_ref() == Some(&wanted.data) => {
                    // Already what the package wants, and not TigerSetup's:
                    // kept with itself as the value to restore, so a
                    // removal leaves it exactly as it was found.
                    plan.operations.push(PlannedOperation::value(
                        OpKind::KeepEnvironmentVariable,
                        &wanted.key,
                        &wanted.name,
                        &wanted.data,
                        Some(Some(&wanted.data)),
                    ));
                }
                None => {
                    plan.counts.resource_operations += 1;
                    plan.operations.push(PlannedOperation::value(
                        OpKind::SetEnvironmentVariable,
                        &wanted.key,
                        &wanted.name,
                        &wanted.data,
                        None,
                    ));
                }
            }
        }
    }
    for record in &owned.environment_variables {
        let Ok(hive_key) = KeyPath::parse(&record.hive_key) else {
            continue;
        };
        if desired_variable_keys.contains(&(hive_key.key(), record.name.to_ascii_lowercase())) {
            continue;
        }
        let written = environment::owned_data(record)?;
        let location = environment::location(&record.hive_key, &record.name);
        match winreg::read_value(input.roots, &hive_key, &record.name)? {
            None => plan
                .findings
                .push(Finding::named("environment_variable_missing", location)),
            Some(current) if current == written => {
                plan.counts.resource_operations += 1;
                let restore = environment::restore_data(record)?;
                environment_restores.push(PlannedOperation::value(
                    OpKind::RestoreEnvironmentVariable,
                    &hive_key,
                    &record.name,
                    &written,
                    Some(restore.as_ref()),
                ));
            }
            Some(_) => plan.findings.push(Finding::named(
                "environment_variable_modified_preserved",
                location,
            )),
        }
    }

    // Shortcuts.
    let mut shortcut_removals = Vec::new();
    let mut desired_shortcut_keys = BTreeSet::new();
    if let Some(desired) = input.desired {
        for wanted in &desired.shortcuts {
            desired_shortcut_keys.insert(key(&wanted.path.display().to_string()));
            match crate::win::shortcut::inspect(&wanted.path)? {
                LinkInspection::Link(current) if shortcut::matches(&current, &wanted.link) => {
                    plan.operations.push(PlannedOperation::shortcut(
                        OpKind::KeepShortcut,
                        &wanted.path,
                        &wanted.link,
                    ))
                }
                _ => {
                    plan.counts.resource_operations += 1;
                    plan.operations.push(PlannedOperation::shortcut(
                        OpKind::CreateShortcut,
                        &wanted.path,
                        &wanted.link,
                    ))
                }
            }
        }
    }
    for owned_shortcut in &owned.shortcuts {
        if desired_shortcut_keys.contains(&key(&owned_shortcut.path)) {
            continue;
        }
        let link_path = shortcut::link_path(&owned_shortcut.path)?;
        // A shortcut folder can move under an installation — OneDrive's
        // Known Folder Move relocates the desktop, and policy can redirect
        // the Start Menu. The link is then outside the folders this scope
        // resolves now, so TigerSetup leaves it alone and says so; refusing
        // the whole run instead would make the product impossible to
        // uninstall. The same check in the executor still refuses to act on
        // such a path, so nothing outside the scope is ever touched.
        if !scope::is_inside(&link_path, input.shortcut_folders) {
            plan.findings
                .push(Finding::at("shortcut_outside_scope_preserved", &link_path));
            continue;
        }
        match crate::win::shortcut::inspect(&link_path)? {
            LinkInspection::Absent => plan
                .findings
                .push(Finding::at("shortcut_missing", &link_path)),
            LinkInspection::Link(current)
                if shortcut::removable(
                    &link_path,
                    &current,
                    &owned_shortcut.target,
                    input.install_root,
                ) =>
            {
                plan.counts.resource_operations += 1;
                shortcut_removals.push(PlannedOperation {
                    value_data: Some(owned_shortcut.target.clone()),
                    ..PlannedOperation::new(OpKind::RemoveShortcut, owned_shortcut.path.clone())
                });
            }
            _ => plan
                .findings
                .push(Finding::at("shortcut_modified_preserved", &link_path)),
        }
    }

    // Firewall rules.
    let mut firewall_removals = Vec::new();
    reconcile_firewall(&input, owned, &mut plan, &mut firewall_removals)?;

    // Registration: the key, then its values, last of the forward order —
    // a registration means "installed" to Windows.
    let mut registration_value_removals = Vec::new();
    if let Some(desired) = input.desired {
        reconcile_keys(
            std::slice::from_ref(&desired.registration_key),
            &locations.uninstall_root,
            input.roots,
            owned,
            &mut keys_kept_or_created,
            &mut plan,
        )?;
    }
    reconcile_values(
        input
            .desired
            .map(|d| &d.registration_values[..])
            .unwrap_or(&[]),
        &registration_values,
        input.roots,
        true,
        &mut plan,
        &mut registration_value_removals,
    )?;

    // Removals, in reverse resource order.
    removals.append(&mut registration_value_removals);
    let mut key_removals: Vec<PlannedOperation> = owned
        .registry_keys
        .iter()
        .filter(|k| k.created && !keys_kept_or_created.contains(&k.key.to_ascii_lowercase()))
        .map(|k| PlannedOperation::new(OpKind::RemoveRegistryKey, k.key.clone()))
        .collect();
    key_removals.sort_by(|a, b| {
        depth(&b.target)
            .cmp(&depth(&a.target))
            .then_with(|| b.target.cmp(&a.target))
    });
    let (registration_key_removals, product_key_removals): (Vec<_>, Vec<_>) = key_removals
        .into_iter()
        .partition(|op| registry::is_registration_key(&op.target, registration_key.as_deref()));
    plan.counts.resource_operations += registration_key_removals.len() + product_key_removals.len();
    removals.extend(registration_key_removals);
    removals.append(&mut firewall_removals);
    removals.append(&mut shortcut_removals);
    removals.append(&mut environment_restores);
    removals.append(&mut path_removals);
    removals.append(&mut product_value_removals);
    removals.extend(product_key_removals);

    let mut preserved = BTreeSet::new();
    for owned_file in &owned.files {
        if desired_file_keys.contains(&key(&owned_file.path)) {
            continue;
        }
        let before = removals.len();
        remove_owned_file(
            input.install_root,
            owned_file,
            &mut removals,
            &mut plan.findings,
            &mut preserved,
        )?;
        plan.counts.removed += removals.len() - before;
    }
    let before = removals.len();
    remove_directories(
        &owned.directories,
        &still_desired_directories,
        &preserved,
        &mut removals,
    );
    plan.counts.directories_removed = removals.len() - before;

    plan.operations.append(&mut removals);

    // What the installation keeps for its own uninstall, then the `post-*`
    // actions, behind everything the run changed.
    for action in &input.actions.store {
        plan.counts.stored_actions += 1;
        plan.operations
            .push(PlannedOperation::action(OpKind::StoreAction, action));
    }
    for entry in &input.actions.store_quiescence {
        plan.counts.stored_actions += 1;
        plan.operations.push(PlannedOperation::quiescence(entry));
    }
    for action in &input.actions.after {
        plan.counts.actions += 1;
        plan.operations
            .push(PlannedOperation::action(OpKind::RunAction, action));
    }
    Ok(plan)
}

/// Keeps, creates, rewrites or removes firewall rules. A run that may not
/// write the firewall store reports what it would have done and carries
/// every owned rule forward untouched, so that a later elevated run can
/// still remove it.
fn reconcile_firewall(
    input: &Reconcile<'_>,
    owned: &Owned,
    plan: &mut Plan,
    removals: &mut Vec<PlannedOperation>,
) -> Result<()> {
    let desired_rules: &[Rule] = input.desired.map(|d| &d.firewall_rules[..]).unwrap_or(&[]);
    let mut desired_names = BTreeSet::new();
    for wanted in desired_rules {
        desired_names.insert(wanted.name.to_ascii_lowercase());
        let record = owned
            .firewall_rules
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(&wanted.name));
        let Some(store) = input.firewall else {
            plan.findings.push(Finding::named(
                "firewall_rule_skipped_unelevated",
                wanted.name.clone(),
            ));
            if let Some(record) = record {
                let recorded = firewall::rule_of(Some(&record.rule), "owned firewall rule")?;
                plan.operations.push(PlannedOperation::firewall_rule(
                    OpKind::KeepFirewallRule,
                    &recorded,
                ));
            }
            continue;
        };
        let existing = store.list(&wanted.name)?;
        let recorded = match record {
            Some(record) => Some(firewall::rule_of(
                Some(&record.rule),
                "owned firewall rule",
            )?),
            None => None,
        };
        match existing.as_slice() {
            [] => {
                plan.counts.resource_operations += 1;
                plan.operations.push(PlannedOperation::firewall_rule(
                    OpKind::CreateFirewallRule,
                    wanted,
                ));
            }
            // A rule this installation owns, or one of TigerSetup's for
            // this product that already reads as wanted (an installation
            // whose database was lost): kept, or rewritten where it drifted.
            // A rule of TigerSetup's that reads differently belongs to
            // another installation of the product — the other scope's — and
            // is preserved like a stranger's.
            [current]
                if recorded.is_some()
                    || (firewall::is_ours(current, &wanted.grouping)
                        && firewall::matches(current, wanted)) =>
            {
                if firewall::matches(current, wanted) {
                    plan.operations.push(PlannedOperation::firewall_rule(
                        OpKind::KeepFirewallRule,
                        wanted,
                    ));
                } else if let Some(recorded) = &recorded
                    && !firewall::matches(current, recorded)
                    && !input.repair
                {
                    // Somebody changed TigerSetup's rule: their rule stays,
                    // the record stays, and the run says so.
                    plan.findings.push(Finding::named(
                        "firewall_rule_modified_preserved",
                        wanted.name.clone(),
                    ));
                    plan.operations.push(PlannedOperation::firewall_rule(
                        OpKind::KeepFirewallRule,
                        recorded,
                    ));
                } else {
                    plan.counts.resource_operations += 1;
                    plan.operations.push(PlannedOperation::firewall_rule(
                        OpKind::CreateFirewallRule,
                        wanted,
                    ));
                }
            }
            [_] => plan.findings.push(Finding::named(
                "firewall_rule_name_in_use_preserved",
                wanted.name.clone(),
            )),
            _ => {
                plan.findings.push(Finding::named(
                    "firewall_rule_ambiguous_preserved",
                    wanted.name.clone(),
                ));
                if let Some(recorded) = &recorded {
                    plan.operations.push(PlannedOperation::firewall_rule(
                        OpKind::KeepFirewallRule,
                        recorded,
                    ));
                }
            }
        }
    }
    for record in &owned.firewall_rules {
        if desired_names.contains(&record.name.to_ascii_lowercase()) {
            continue;
        }
        let recorded = firewall::rule_of(Some(&record.rule), "owned firewall rule")?;
        let Some(store) = input.firewall else {
            plan.findings.push(Finding::named(
                "firewall_rule_skipped_unelevated",
                record.name.clone(),
            ));
            plan.operations.push(PlannedOperation::firewall_rule(
                OpKind::KeepFirewallRule,
                &recorded,
            ));
            continue;
        };
        match store.list(&record.name)?.as_slice() {
            [] => plan
                .findings
                .push(Finding::named("firewall_rule_missing", record.name.clone())),
            [current] if firewall::matches(current, &recorded) => {
                plan.counts.resource_operations += 1;
                removals.push(PlannedOperation::firewall_rule(
                    OpKind::RemoveFirewallRule,
                    &recorded,
                ));
            }
            [current] if firewall::is_ours(current, &recorded.grouping) => plan.findings.push(
                Finding::named("firewall_rule_modified_preserved", record.name.clone()),
            ),
            [_] => plan.findings.push(Finding::named(
                "firewall_rule_name_in_use_preserved",
                record.name.clone(),
            )),
            _ => plan.findings.push(Finding::named(
                "firewall_rule_ambiguous_preserved",
                record.name.clone(),
            )),
        }
    }
    Ok(())
}

fn distinct_keys<'a>(keys: impl Iterator<Item = &'a KeyPath>) -> Vec<KeyPath> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for key in keys {
        if seen.insert(key.key()) {
            out.push(key.clone());
        }
    }
    out
}

/// Keeps or creates every key `needed` implies below `stop`: an owned key
/// is kept (its `created` flag carried forward), a missing one is created,
/// an existing foreign one is neither created nor owned.
fn reconcile_keys(
    needed: &[KeyPath],
    stop: &KeyPath,
    roots: &Roots,
    owned: &Owned,
    handled: &mut BTreeSet<String>,
    plan: &mut Plan,
) -> Result<()> {
    let owned_by_key: HashMap<String, bool> = owned
        .registry_keys
        .iter()
        .map(|k| (k.key.to_ascii_lowercase(), k.created))
        .collect();
    let mut chains: Vec<KeyPath> = Vec::new();
    for needed_key in needed {
        let mut current = Some(needed_key.clone());
        while let Some(candidate) = current {
            if !candidate.is_under(stop) || candidate.key() == stop.key() {
                break;
            }
            chains.push(candidate.clone());
            current = candidate.parent();
        }
    }
    chains.sort_by(|a, b| {
        a.depth()
            .cmp(&b.depth())
            .then_with(|| a.key().cmp(&b.key()))
    });
    for candidate in chains {
        if !handled.insert(candidate.key()) {
            continue;
        }
        let exists = winreg::key_exists(roots, &candidate)?;
        match owned_by_key.get(&candidate.key()) {
            // An owned key that is still on the machine is kept, carrying its
            // ownership forward. One deleted behind TigerSetup's back is
            // created again — the values planned under it must have a key to
            // go into, and a keep is a no-op that would leave them nowhere.
            // The create is prepared against the machine as it is, so the
            // ownership recorded at commit follows what this run really did.
            Some(created) if exists => plan.operations.push(PlannedOperation {
                previous_existed: Some(!created),
                ..PlannedOperation::new(OpKind::KeepRegistryKey, candidate.to_string())
            }),
            // A key nobody owns that is already there is left alone and not
            // claimed; anything else is created.
            None if exists => {
                handled.remove(&candidate.key());
            }
            _ => {
                plan.counts.resource_operations += 1;
                plan.operations.push(PlannedOperation::new(
                    OpKind::CreateRegistryKey,
                    candidate.to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// Keeps or sets every desired value, carrying its pre-installation state
/// forward; for owned values not desired, plans a removal — which puts back
/// the recorded pre-installation state — when the value still holds what
/// TigerSetup wrote, and reports it otherwise.
///
/// A desired value somebody changed after TigerSetup wrote it is preserved
/// and reported unless `overwrite` says the desired data wins: a repair,
/// which is asked for, and the Add/Remove Programs registration, which is
/// TigerSetup's own bookkeeping and never a person's setting.
fn reconcile_values(
    desired: &[DesiredValue],
    owned: &[&crate::state::installation::OwnedRegistryValue],
    roots: &Roots,
    overwrite: bool,
    plan: &mut Plan,
    removals: &mut Vec<PlannedOperation>,
) -> Result<()> {
    let mut desired_keys = BTreeSet::new();
    for wanted in desired {
        desired_keys.insert((wanted.key.key(), wanted.name.to_ascii_lowercase()));
        let record = owned.iter().find(|v| {
            v.key.eq_ignore_ascii_case(&wanted.key.to_string())
                && v.name.eq_ignore_ascii_case(&wanted.name)
        });
        let current = winreg::read_value(roots, &wanted.key, &wanted.name)?;
        match record {
            Some(record) => {
                let restore = registry::restore_data(record)?;
                let written = registry::owned_data(record)?;
                if current.as_ref() == Some(&wanted.data) {
                    plan.operations.push(PlannedOperation::value(
                        OpKind::KeepRegistryValue,
                        &wanted.key,
                        &wanted.name,
                        &wanted.data,
                        Some(restore.as_ref()),
                    ));
                } else if current.is_none() || current.as_ref() == Some(&written) || overwrite {
                    // Missing, or still what TigerSetup wrote and the desired
                    // data changed (a new version, a new root), or the desired
                    // data wins: rewrite, keeping the pre-installation state to
                    // restore.
                    plan.counts.resource_operations += 1;
                    plan.operations.push(PlannedOperation::value(
                        OpKind::SetRegistryValue,
                        &wanted.key,
                        &wanted.name,
                        &wanted.data,
                        Some(restore.as_ref()),
                    ));
                } else {
                    // Somebody changed it since: their value stays, the record
                    // stays, and the run says so.
                    plan.findings.push(Finding::named(
                        "registry_value_modified_preserved",
                        registry::location(&record.key, &record.name),
                    ));
                    plan.operations.push(PlannedOperation::value(
                        OpKind::KeepRegistryValue,
                        &wanted.key,
                        &wanted.name,
                        &written,
                        Some(restore.as_ref()),
                    ));
                }
            }
            None if current.as_ref() == Some(&wanted.data) => {
                // Already what the package wants, and not TigerSetup's: kept
                // with itself as the value to restore, so a removal leaves it
                // exactly as it was found.
                plan.operations.push(PlannedOperation::value(
                    OpKind::KeepRegistryValue,
                    &wanted.key,
                    &wanted.name,
                    &wanted.data,
                    Some(Some(&wanted.data)),
                ));
            }
            None => {
                plan.counts.resource_operations += 1;
                plan.operations.push(PlannedOperation::value(
                    OpKind::SetRegistryValue,
                    &wanted.key,
                    &wanted.name,
                    &wanted.data,
                    None,
                ));
            }
        }
    }
    for existing in owned {
        let Ok(key_path) = KeyPath::parse(&existing.key) else {
            continue;
        };
        if desired_keys.contains(&(key_path.key(), existing.name.to_ascii_lowercase())) {
            continue;
        }
        let recorded = registry::owned_data(existing)?;
        let location = registry::location(&existing.key, &existing.name);
        match winreg::read_value(roots, &key_path, &existing.name)? {
            None => plan
                .findings
                .push(Finding::named("registry_value_missing", location)),
            Some(current) if current == recorded => {
                plan.counts.resource_operations += 1;
                let restore = registry::restore_data(existing)?;
                removals.push(PlannedOperation::value(
                    OpKind::RemoveRegistryValue,
                    &key_path,
                    &existing.name,
                    &recorded,
                    Some(restore.as_ref()),
                ));
            }
            Some(_) => plan.findings.push(Finding::named(
                "registry_value_modified_preserved",
                location,
            )),
        }
    }
    Ok(())
}

/// Plans the removal of one owned file, or a finding when it is absent or
/// was modified (preserved, with every directory on its path).
fn remove_owned_file(
    install_root: &Path,
    file: &OwnedFile,
    operations: &mut Vec<PlannedOperation>,
    findings: &mut Vec<Finding>,
    preserved: &mut BTreeSet<String>,
) -> Result<()> {
    let target = absolute(install_root, &file.path)?;
    match fs::inspect_unless_unchanged(&target, file.fingerprint(), &file.sha256)? {
        Inspection::Absent => findings.push(Finding::at("file_missing", &target)),
        Inspection::Present { sha256, .. } if sha256 != file.sha256 => {
            findings.push(Finding::at("file_modified_preserved", &target));
            preserved.insert(String::new());
            preserved.extend(ancestors_of(&file.path).iter().map(|d| key(d)));
        }
        Inspection::Present { .. } => operations.push(PlannedOperation {
            expected_size: Some(file.size),
            ..PlannedOperation::new(OpKind::RemoveFile, file.path.clone())
        }),
    }
    Ok(())
}

/// Plans the removal, deepest first, of the directories TigerSetup created
/// that are neither still desired nor on the path of a preserved file.
fn remove_directories(
    directories: &[OwnedDirectory],
    still_desired: &BTreeSet<String>,
    preserved: &BTreeSet<String>,
    operations: &mut Vec<PlannedOperation>,
) {
    let mut removable: Vec<&OwnedDirectory> = directories
        .iter()
        .filter(|d| {
            d.created
                && !still_desired.contains(&key(&d.path))
                && !preserved.contains(&key(&d.path))
        })
        .collect();
    removable.sort_by(|a, b| {
        depth(&b.path)
            .cmp(&depth(&a.path))
            .then_with(|| b.path.cmp(&a.path))
    });
    for directory in removable {
        operations.push(PlannedOperation::new(
            OpKind::RemoveDirectory,
            directory.path.clone(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::installation::{
        OwnedPathEntry, OwnedRegistryKey, OwnedRegistryValue, OwnedShortcut,
    };
    use std::collections::BTreeMap;
    use tigersetup_format::metadata::{
        Directory, Engine, File, Install, InstallOption, OptionChoice, OptionKind, Package,
        PathEntry, Registration, RegistryKind, RegistryValue,
    };

    fn on(name: &str) -> (String, OptionValue) {
        (name.to_string(), OptionValue::Bool(true))
    }

    fn off(name: &str) -> (String, OptionValue) {
        (name.to_string(), OptionValue::Bool(false))
    }

    fn metadata() -> Metadata {
        Metadata {
            schema: 1,
            package: Some(Package {
                id: "IT-Tiger.T".into(),
                name: "T".into(),
                version: "1.0.0".into(),
                publisher: "p".into(),
                ..Default::default()
            }),
            install: Some(Install {
                scopes: vec![1],
                user_root: "%LOCALAPPDATA%\\Programs\\T".into(),
                machine_root: String::new(),
                ..Default::default()
            }),
            files: vec![
                File {
                    path: "b/deep/z.txt".into(),
                    size: 1,
                    entry: "b/deep/z.txt".into(),
                    when: None,
                },
                File {
                    path: "a.txt".into(),
                    size: 2,
                    entry: "a.txt".into(),
                    when: None,
                },
            ],
            directories: vec![
                Directory { path: "b".into() },
                Directory {
                    path: "b/deep".into(),
                },
            ],
            engine: Some(Engine::default()),
            ..Default::default()
        }
    }

    fn hash(bytes: &[u8]) -> String {
        tigersetup_format::hex(&tigersetup_format::sha256(bytes))
    }

    /// A relocated registry root for one test, removed on drop.
    struct TestRoots {
        roots: Roots,
        prefix: String,
    }

    impl TestRoots {
        fn new(name: &str) -> TestRoots {
            let prefix = format!(
                "Software\\TigerSetupTests\\plan-{name}-{}-{}",
                std::process::id(),
                crate::report::unique_id()
            );
            TestRoots {
                roots: Roots::relocated(&prefix),
                prefix,
            }
        }
    }

    impl Drop for TestRoots {
        fn drop(&mut self) {
            let _ = std::process::Command::new("reg.exe")
                .args(["delete", &format!("HKCU\\{}", self.prefix), "/f"])
                .output();
        }
    }

    fn summary(plan: &Plan) -> Vec<(OpKind, String)> {
        plan.operations
            .iter()
            .map(|op| (op.kind, op.target.clone()))
            .collect()
    }

    #[test]
    fn stored_paths_must_stay_under_the_install_root() {
        let root = Path::new("C:\\Programs\\T");
        assert_eq!(absolute(root, "").unwrap(), root);
        assert_eq!(
            absolute(root, "bin\\x.dll").unwrap(),
            root.join("bin").join("x.dll")
        );
        for bad in [
            "bin\\..\\..\\Windows\\x.dll",
            "..\\elsewhere",
            "C:\\Windows\\x.dll",
            "\\\\server\\share\\x",
            "\\abs",
            "bin\\.\\x.dll",
            "bin\\nul",
            "bin\\x.dll ",
        ] {
            assert_eq!(
                absolute(root, bad).unwrap_err().code,
                "path_outside_root",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn effective_options_layer_explicit_over_recorded_over_default() {
        let mut metadata = metadata();
        metadata.options = vec![
            InstallOption {
                name: "path".into(),
                default: true,
                ..Default::default()
            },
            InstallOption {
                name: "desktop-shortcut".into(),
                default: false,
                ..Default::default()
            },
        ];
        let recorded = BTreeMap::from([off("path")]);
        let explicit = BTreeMap::from([on("desktop-shortcut")]);
        let effective = effective_options(&metadata, &recorded, &explicit).unwrap();
        assert!(!effective["path"].is_on(), "recorded wins over the default");
        assert!(effective["desktop-shortcut"].is_on(), "explicit wins");
        let empty = BTreeMap::new();
        assert!(effective_options(&metadata, &empty, &empty).unwrap()["path"].is_on());
        let unknown = BTreeMap::from([on("nope")]);
        assert_eq!(
            effective_options(&metadata, &empty, &unknown)
                .unwrap_err()
                .code,
            "option_unknown"
        );
    }

    /// A choice option layers the same way; a value the option does not
    /// take is refused when explicit and falls back to the default when
    /// recorded by an older installation.
    #[test]
    fn choice_options_layer_and_refuse_values_they_do_not_take() {
        let mut metadata = metadata();
        metadata.options = vec![InstallOption {
            name: "path-mode".into(),
            kind: OptionKind::Choice as i32,
            choices: vec![
                OptionChoice {
                    value: "none".into(),
                    ..Default::default()
                },
                OptionChoice {
                    value: "command".into(),
                    ..Default::default()
                },
            ],
            default_choice: "command".into(),
            ..Default::default()
        }];
        let empty = BTreeMap::new();
        assert_eq!(
            effective_options(&metadata, &empty, &empty).unwrap()["path-mode"],
            OptionValue::Choice("command".into())
        );
        let recorded =
            BTreeMap::from([("path-mode".to_string(), OptionValue::Choice("none".into()))]);
        assert_eq!(
            effective_options(&metadata, &recorded, &empty).unwrap()["path-mode"],
            OptionValue::Choice("none".into())
        );
        let explicit =
            BTreeMap::from([("path-mode".to_string(), OptionValue::Choice("NONE".into()))]);
        assert_eq!(
            effective_options(&metadata, &empty, &explicit).unwrap()["path-mode"],
            OptionValue::Choice("none".into()),
            "an explicit value is matched case-insensitively and canonicalised"
        );
        let stale =
            BTreeMap::from([("path-mode".to_string(), OptionValue::Choice("tools".into()))]);
        assert_eq!(
            effective_options(&metadata, &stale, &empty).unwrap()["path-mode"],
            OptionValue::Choice("command".into()),
            "a recorded value the package dropped falls back to the default"
        );
        let bad = BTreeMap::from([("path-mode".to_string(), OptionValue::Choice("tools".into()))]);
        assert_eq!(
            effective_options(&metadata, &empty, &bad).unwrap_err().code,
            "option_value_invalid"
        );
        let bool_for_choice = BTreeMap::from([on("path-mode")]);
        assert_eq!(
            effective_options(&metadata, &empty, &bool_for_choice)
                .unwrap_err()
                .code,
            "option_value_invalid"
        );
    }

    #[test]
    fn install_plan_orders_root_directories_files_then_resources() {
        let test = TestRoots::new("install");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let mut metadata = metadata();
        metadata.options.push(InstallOption {
            name: "path".into(),
            default: true,
            ..Default::default()
        });
        metadata.path_entries.push(PathEntry {
            path: "b".into(),
            option: "path".into(),
            when: None,
        });
        metadata.registry_values.push(RegistryValue {
            key: "IT Tiger\\T".into(),
            name: "InstallRoot".into(),
            kind: RegistryKind::ExpandString as i32,
            data: "%INSTALLROOT%".into(),
            when: None,
            root: 0,
        });
        metadata.registration = Some(Registration::default());
        let options = effective_options(&metadata, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        let desired = desired(
            &metadata,
            &options,
            Scope::User,
            &root,
            &dir.path().join("uninstall.exe"),
            &test.roots,
            &Owned::default(),
        )
        .unwrap();
        assert_eq!(
            desired.registry_values[0].data,
            Data::ExpandString(root.display().to_string())
        );
        assert_eq!(
            desired.path_entries,
            vec![root.join("b").display().to_string()]
        );
        let plan = reconcile(Reconcile {
            desired: Some(&desired),
            owned: &Owned::default(),
            install_root: &root,
            payload: None,
            roots: &test.roots,
            scope: Scope::User,
            inspect_files: false,
            repair: false,
            shortcut_folders: &shortcut_folders(&dir),
            firewall: None,
            actions: &ActionPlan::default(),
        })
        .unwrap();
        let kinds: Vec<OpKind> = plan.operations.iter().map(|op| op.kind).collect();
        assert_eq!(
            kinds,
            vec![
                OpKind::CreateDirectory,
                OpKind::CreateDirectory,
                OpKind::CreateDirectory,
                OpKind::InstallFile,
                OpKind::InstallFile,
                OpKind::CreateRegistryKey,
                OpKind::CreateRegistryKey,
                OpKind::SetRegistryValue,
                OpKind::AddPathEntry,
                OpKind::CreateRegistryKey,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
                OpKind::SetRegistryValue,
            ]
        );
        assert_eq!(plan.operations[3].target, "b\\deep\\z.txt");
        assert_eq!(plan.operations[3].expected_size, Some(1));
        assert_eq!(plan.operations[5].target, "HKCU\\Software\\IT Tiger");
        assert_eq!(plan.operations[6].target, "HKCU\\Software\\IT Tiger\\T");
        assert_eq!(
            plan.operations[8].value_data.as_deref(),
            Some(root.join("b").display().to_string().as_str())
        );
        assert_eq!(
            plan.operations[9].target,
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.T"
        );
        assert_eq!(
            plan.operations[10].value_name.as_deref(),
            Some("DisplayName")
        );
        assert!(plan.findings.is_empty());
    }

    #[test]
    fn uninstall_plan_preserves_modified_files_and_their_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("b\\deep")).unwrap();
        std::fs::create_dir_all(root.join("c")).unwrap();
        std::fs::write(root.join("a.txt"), b"aa").unwrap();
        std::fs::write(root.join("b\\deep\\z.txt"), b"changed").unwrap();
        let owned = Owned {
            actions: Vec::new(),
            files: vec![
                OwnedFile {
                    path: "a.txt".into(),
                    sha256: hash(b"aa"),
                    size: 2,
                    modified: None,
                },
                OwnedFile {
                    path: "b\\deep\\z.txt".into(),
                    sha256: hash(b"z"),
                    size: 1,
                    modified: None,
                },
                OwnedFile {
                    path: "c\\gone.txt".into(),
                    sha256: hash(b"g"),
                    size: 1,
                    modified: None,
                },
            ],
            directories: vec![
                OwnedDirectory {
                    path: String::new(),
                    created: true,
                },
                OwnedDirectory {
                    path: "b".into(),
                    created: true,
                },
                OwnedDirectory {
                    path: "b\\deep".into(),
                    created: true,
                },
                OwnedDirectory {
                    path: "c".into(),
                    created: false,
                },
            ],
            ..Default::default()
        };
        let plan = reconcile(Reconcile {
            desired: None,
            owned: &owned,
            install_root: root,
            payload: None,
            roots: &Roots::relocated("Software\\TigerSetupTests\\unused"),
            scope: Scope::User,
            inspect_files: true,
            repair: false,
            shortcut_folders: &shortcut_folders(&dir),
            firewall: None,
            actions: &ActionPlan::default(),
        })
        .unwrap();
        assert_eq!(
            summary(&plan),
            vec![(OpKind::RemoveFile, "a.txt".to_string())]
        );
        let codes: Vec<&str> = plan.findings.iter().map(|f| f.code).collect();
        assert_eq!(codes, vec!["file_modified_preserved", "file_missing"]);
    }

    #[test]
    fn uninstall_plan_removes_resources_then_files_then_directories() {
        let test = TestRoots::new("uninstall");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("b\\deep")).unwrap();
        std::fs::write(root.join("b\\deep\\z.txt"), b"z").unwrap();
        let product = KeyPath::parse("HKCU\\Software\\IT Tiger\\T").unwrap();
        let registration = KeyPath::parse(
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.T",
        )
        .unwrap();
        winreg::create_key(&test.roots, &product).unwrap();
        winreg::create_key(&test.roots, &registration).unwrap();
        let root_text = root.display().to_string();
        winreg::write_value(
            &test.roots,
            &product,
            "InstallRoot",
            &Data::ExpandString(root_text.clone()),
        )
        .unwrap();
        winreg::write_value(
            &test.roots,
            &product,
            "Edited",
            &Data::String("user".into()),
        )
        .unwrap();
        winreg::write_value(
            &test.roots,
            &registration,
            "DisplayName",
            &Data::String("T".into()),
        )
        .unwrap();
        let environment = KeyPath::parse("HKCU\\Environment").unwrap();
        winreg::create_key(&test.roots, &environment).unwrap();
        path::add(&test.roots, &environment, "C:\\Other").unwrap();
        path::add(&test.roots, &environment, &format!("{root_text}\\b")).unwrap();
        let link = dir.path().join("Programs").join("T.lnk");
        crate::win::shortcut::write(
            &link,
            &Link {
                target: root.join("b\\deep\\z.txt").display().to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        let foreign = dir.path().join("Programs").join("Foreign.lnk");
        crate::win::shortcut::write(
            &foreign,
            &Link {
                target: "C:\\Windows\\notepad.exe".into(),
                ..Default::default()
            },
        )
        .unwrap();

        let owned = Owned {
            environment_variables: vec![],
            firewall_rules: vec![],
            actions: vec![],
            files: vec![OwnedFile {
                path: "b\\deep\\z.txt".into(),
                sha256: hash(b"z"),
                size: 1,
                modified: None,
            }],
            directories: vec![
                OwnedDirectory {
                    path: String::new(),
                    created: true,
                },
                OwnedDirectory {
                    path: "b".into(),
                    created: true,
                },
                OwnedDirectory {
                    path: "b\\deep".into(),
                    created: true,
                },
            ],
            registry_keys: vec![
                OwnedRegistryKey {
                    key: "HKCU\\Software\\IT Tiger".into(),
                    created: false,
                },
                OwnedRegistryKey {
                    key: product.to_string(),
                    created: true,
                },
                OwnedRegistryKey {
                    key: registration.to_string(),
                    created: true,
                },
            ],
            registry_values: vec![
                OwnedRegistryValue {
                    key: product.to_string(),
                    name: "InstallRoot".into(),
                    kind: "expand_string".into(),
                    data: root_text.clone(),
                    pre_existed: false,
                    previous_kind: None,
                    previous_data: None,
                },
                OwnedRegistryValue {
                    key: product.to_string(),
                    name: "Edited".into(),
                    kind: "string".into(),
                    data: "original".into(),
                    pre_existed: false,
                    previous_kind: None,
                    previous_data: None,
                },
                OwnedRegistryValue {
                    key: product.to_string(),
                    name: "Gone".into(),
                    kind: "dword".into(),
                    data: "1".into(),
                    pre_existed: false,
                    previous_kind: None,
                    previous_data: None,
                },
                OwnedRegistryValue {
                    key: registration.to_string(),
                    name: "DisplayName".into(),
                    kind: "string".into(),
                    data: "T".into(),
                    pre_existed: false,
                    previous_kind: None,
                    previous_data: None,
                },
            ],
            path_entries: vec![
                OwnedPathEntry {
                    hive_key: "HKCU\\Environment".into(),
                    raw: format!("{root_text}\\b"),
                    normalized: path::normalize(&format!("{root_text}\\b")),
                    pre_existed: false,
                    added: true,
                },
                OwnedPathEntry {
                    hive_key: "HKCU\\Environment".into(),
                    raw: "C:\\Other".into(),
                    normalized: path::normalize("C:\\Other"),
                    pre_existed: true,
                    added: false,
                },
            ],
            shortcuts: vec![
                OwnedShortcut {
                    path: link.display().to_string(),
                    target: root.join("b\\deep\\z.txt").display().to_string(),
                },
                OwnedShortcut {
                    path: foreign.display().to_string(),
                    target: root.join("b\\deep\\z.txt").display().to_string(),
                },
                OwnedShortcut {
                    path: dir
                        .path()
                        .join("Programs")
                        .join("Missing.lnk")
                        .display()
                        .to_string(),
                    target: root.display().to_string(),
                },
            ],
            registration_key: Some(registration.to_string()),
        };
        let plan = reconcile(Reconcile {
            desired: None,
            owned: &owned,
            install_root: &root,
            payload: None,
            roots: &test.roots,
            scope: Scope::User,
            inspect_files: true,
            repair: false,
            shortcut_folders: &shortcut_folders(&dir),
            firewall: None,
            actions: &ActionPlan::default(),
        })
        .unwrap();
        assert_eq!(
            summary(&plan),
            vec![
                (OpKind::RemoveRegistryValue, registration.to_string()),
                (OpKind::RemoveRegistryKey, registration.to_string()),
                (OpKind::RemoveShortcut, link.display().to_string()),
                (OpKind::RemovePathEntry, "HKCU\\Environment".to_string()),
                (OpKind::RemoveRegistryValue, product.to_string()),
                (OpKind::RemoveRegistryKey, product.to_string()),
                (OpKind::RemoveFile, "b\\deep\\z.txt".to_string()),
                (OpKind::RemoveDirectory, "b\\deep".to_string()),
                (OpKind::RemoveDirectory, "b".to_string()),
                (OpKind::RemoveDirectory, "".to_string()),
            ]
        );
        assert_eq!(
            plan.operations[3].value_data.as_deref(),
            Some(format!("{root_text}\\b").as_str())
        );
        assert_eq!(
            plan.operations[4].value_name.as_deref(),
            Some("InstallRoot")
        );
        let mut codes: Vec<&str> = plan.findings.iter().map(|f| f.code).collect();
        codes.sort();
        assert_eq!(
            codes,
            vec![
                "registry_value_missing",
                "registry_value_modified_preserved",
                "shortcut_missing",
                "shortcut_modified_preserved",
            ]
        );
    }

    #[test]
    fn reconcile_keeps_present_resources_and_adds_missing_ones() {
        let test = TestRoots::new("reconcile");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let root_text = root.display().to_string();
        let environment = KeyPath::parse("HKCU\\Environment").unwrap();
        winreg::create_key(&test.roots, &environment).unwrap();
        let lookalike = format!("{root_text}\\b\\");
        path::add(&test.roots, &environment, &lookalike).unwrap();
        let product = KeyPath::parse("HKCU\\Software\\IT Tiger\\T").unwrap();
        winreg::create_key(&test.roots, &product).unwrap();
        winreg::write_value(
            &test.roots,
            &product,
            "InstallRoot",
            &Data::ExpandString(root_text.clone()),
        )
        .unwrap();

        let desired = Desired {
            locations: scope::locations(Scope::User),
            directories: vec![],
            files: vec![],
            registry_values: vec![
                DesiredValue {
                    key: product.clone(),
                    name: "InstallRoot".into(),
                    data: Data::ExpandString(root_text.clone()),
                },
                DesiredValue {
                    key: product.clone(),
                    name: "Version".into(),
                    data: Data::String("1.0.0".into()),
                },
            ],
            path_entries: vec![format!("{root_text}\\b")],
            shortcuts: vec![DesiredShortcut {
                path: dir.path().join("Programs").join("T.lnk"),
                link: Link {
                    target: root.join("app.exe").display().to_string(),
                    ..Default::default()
                },
            }],
            registration_key: KeyPath::parse(
                "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.T",
            )
            .unwrap(),
            registration_values: vec![],
            environment_variables: vec![],
            firewall_rules: vec![],
            integrations: vec![],
            findings: vec![],
        };
        let owned = Owned {
            registry_keys: vec![OwnedRegistryKey {
                key: product.to_string(),
                created: true,
            }],
            ..Default::default()
        };
        let plan = reconcile(Reconcile {
            desired: Some(&desired),
            owned: &owned,
            install_root: &root,
            payload: None,
            roots: &test.roots,
            scope: Scope::User,
            inspect_files: true,
            repair: false,
            shortcut_folders: &shortcut_folders(&dir),
            firewall: None,
            actions: &ActionPlan::default(),
        })
        .unwrap();
        assert_eq!(
            summary(&plan),
            vec![
                (OpKind::KeepRegistryKey, product.to_string()),
                (OpKind::KeepRegistryValue, product.to_string()),
                (OpKind::SetRegistryValue, product.to_string()),
                (OpKind::KeepPathEntry, "HKCU\\Environment".to_string()),
                (
                    OpKind::CreateShortcut,
                    dir.path()
                        .join("Programs")
                        .join("T.lnk")
                        .display()
                        .to_string()
                ),
                (
                    OpKind::CreateRegistryKey,
                    desired.registration_key.to_string()
                ),
            ]
        );
        assert_eq!(
            plan.operations[0].previous_existed,
            Some(false),
            "an owned key created by us stays created"
        );
        assert_eq!(
            plan.operations[3].previous_existed,
            Some(true),
            "the lookalike pre-existed: never claimed"
        );
        assert_eq!(plan.counts.resource_operations, 3);
    }

    /// A value at an explicit location outside `Software`: the chain of
    /// keys stops at the hive's top-level key and claims nothing that is
    /// already there; the value's pre-installation state travels with every
    /// operation — found by the first set, carried by a keep, by a set whose
    /// desired data changed, and by the removal that puts it back — and a
    /// value somebody changed is preserved unless the run is a repair.
    #[test]
    fn explicit_registry_values_carry_their_pre_installation_state() {
        let test = TestRoots::new("explicit");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let file_system =
            KeyPath::parse("HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem").unwrap();
        winreg::create_key(&test.roots, &file_system).unwrap();
        winreg::write_value(
            &test.roots,
            &file_system,
            "LongPathsEnabled",
            &Data::Dword(0),
        )
        .unwrap();
        let fresh = KeyPath::parse("HKLM\\SYSTEM\\TigerSetupTest\\Sub").unwrap();
        let mut desired = Desired {
            locations: scope::locations(Scope::Machine),
            directories: vec![],
            files: vec![],
            registry_values: vec![
                DesiredValue {
                    key: file_system.clone(),
                    name: "LongPathsEnabled".into(),
                    data: Data::Dword(1),
                },
                DesiredValue {
                    key: fresh.clone(),
                    name: "Marker".into(),
                    data: Data::String("x".into()),
                },
            ],
            path_entries: vec![],
            shortcuts: vec![],
            registration_key: KeyPath::parse(
                "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.T",
            )
            .unwrap(),
            registration_values: vec![],
            environment_variables: vec![],
            firewall_rules: vec![],
            integrations: vec![],
            findings: vec![],
        };
        let reconcile_with = |desired: Option<&Desired>, owned: &Owned, repair: bool| {
            reconcile(Reconcile {
                desired,
                owned,
                install_root: &root,
                payload: None,
                roots: &test.roots,
                scope: Scope::Machine,
                inspect_files: true,
                repair,
                shortcut_folders: &shortcut_folders(&dir),
                firewall: None,
                actions: &ActionPlan::default(),
            })
            .unwrap()
        };

        // Fresh install: the existing Windows keys are not claimed, the
        // missing chain below SYSTEM is created, and the first set of each
        // value carries no restore state — prepare records what it finds.
        let plan = reconcile_with(Some(&desired), &Owned::default(), false);
        assert_eq!(
            summary(&plan),
            vec![
                (
                    OpKind::CreateRegistryKey,
                    "HKLM\\SYSTEM\\TigerSetupTest".to_string()
                ),
                (OpKind::CreateRegistryKey, fresh.to_string()),
                (OpKind::SetRegistryValue, file_system.to_string()),
                (OpKind::SetRegistryValue, fresh.to_string()),
                (
                    OpKind::CreateRegistryKey,
                    desired.registration_key.to_string()
                ),
            ]
        );
        assert_eq!(plan.operations[2].restore_kind, None);

        // Installed: LongPathsEnabled pre-existed as 0 and was set to 1; the
        // marker did not exist. Nothing changed since.
        winreg::write_value(
            &test.roots,
            &file_system,
            "LongPathsEnabled",
            &Data::Dword(1),
        )
        .unwrap();
        winreg::create_key(&test.roots, &fresh).unwrap();
        winreg::write_value(&test.roots, &fresh, "Marker", &Data::String("x".into())).unwrap();
        let owned = Owned {
            registry_keys: vec![
                OwnedRegistryKey {
                    key: "HKLM\\SYSTEM\\TigerSetupTest".into(),
                    created: true,
                },
                OwnedRegistryKey {
                    key: fresh.to_string(),
                    created: true,
                },
            ],
            registry_values: vec![
                OwnedRegistryValue {
                    key: file_system.to_string(),
                    name: "LongPathsEnabled".into(),
                    kind: "dword".into(),
                    data: "1".into(),
                    pre_existed: true,
                    previous_kind: Some("dword".into()),
                    previous_data: Some("0".into()),
                },
                OwnedRegistryValue {
                    key: fresh.to_string(),
                    name: "Marker".into(),
                    kind: "string".into(),
                    data: "x".into(),
                    pre_existed: false,
                    previous_kind: None,
                    previous_data: None,
                },
            ],
            ..Default::default()
        };
        let plan = reconcile_with(Some(&desired), &owned, false);
        let keep = &plan.operations[2];
        assert_eq!(keep.kind, OpKind::KeepRegistryValue);
        assert_eq!(keep.restore_kind.as_deref(), Some("dword"));
        assert_eq!(keep.restore_data.as_deref(), Some("0"));
        let marker = plan
            .operations
            .iter()
            .find(|op| op.kind == OpKind::KeepRegistryValue && op.target == fresh.to_string())
            .unwrap();
        assert_eq!(marker.restore_kind.as_deref(), Some(RESTORE_ABSENT));

        // An upgrade that wants new data rewrites a value still holding what
        // TigerSetup wrote, and the pre-installation state stays the same.
        desired.registry_values[1].data = Data::String("y".into());
        let plan = reconcile_with(Some(&desired), &owned, false);
        let set = plan
            .operations
            .iter()
            .find(|op| op.target == fresh.to_string() && op.value_name.is_some())
            .unwrap();
        assert_eq!(set.kind, OpKind::SetRegistryValue);
        assert_eq!(set.restore_kind.as_deref(), Some(RESTORE_ABSENT));

        // Somebody turned the setting off again: an upgrade leaves their
        // value and says so; a repair puts the package's back.
        winreg::write_value(
            &test.roots,
            &file_system,
            "LongPathsEnabled",
            &Data::Dword(0),
        )
        .unwrap();
        let plan = reconcile_with(Some(&desired), &owned, false);
        assert_eq!(plan.operations[2].kind, OpKind::KeepRegistryValue);
        assert_eq!(plan.operations[2].value_data.as_deref(), Some("1"));
        assert_eq!(
            plan.findings.iter().map(|f| f.code).collect::<Vec<_>>(),
            vec!["registry_value_modified_preserved"]
        );
        let plan = reconcile_with(Some(&desired), &owned, true);
        assert_eq!(plan.operations[2].kind, OpKind::SetRegistryValue);
        assert!(plan.findings.is_empty());
        winreg::write_value(
            &test.roots,
            &file_system,
            "LongPathsEnabled",
            &Data::Dword(1),
        )
        .unwrap();

        // Uninstall: the removal of a pre-existing value carries what to put
        // back, the removal of a created one carries "absent", and the
        // created keys go while Windows's own stay.
        let plan = reconcile_with(None, &owned, false);
        let removals: Vec<(OpKind, String, Option<String>)> = plan
            .operations
            .iter()
            .map(|op| (op.kind, op.target.clone(), op.restore_kind.clone()))
            .collect();
        assert_eq!(
            removals,
            vec![
                (
                    OpKind::RemoveRegistryValue,
                    file_system.to_string(),
                    Some("dword".into())
                ),
                (
                    OpKind::RemoveRegistryValue,
                    fresh.to_string(),
                    Some(RESTORE_ABSENT.into())
                ),
                (OpKind::RemoveRegistryKey, fresh.to_string(), None),
                (
                    OpKind::RemoveRegistryKey,
                    "HKLM\\SYSTEM\\TigerSetupTest".to_string(),
                    None
                ),
            ]
        );
        assert_eq!(plan.operations[0].restore_data.as_deref(), Some("0"));

        // A value that already held what the package wants, and was not
        // TigerSetup's, is kept with itself as the state to put back.
        winreg::create_key(&test.roots, &fresh).unwrap();
        winreg::write_value(&test.roots, &fresh, "Marker", &Data::String("y".into())).unwrap();
        let plan = reconcile_with(Some(&desired), &Owned::default(), false);
        let keep = plan
            .operations
            .iter()
            .find(|op| op.kind == OpKind::KeepRegistryValue && op.target == fresh.to_string())
            .unwrap();
        assert_eq!(keep.restore_kind.as_deref(), Some("string"));
        assert_eq!(keep.restore_data.as_deref(), Some("y"));
    }

    /// Builds a payload archive holding `entries` in a temporary installer
    /// file, so the planner can hash entries the way the engine does.
    /// The scope's shortcut folders as a unit test's fixtures use them: the
    /// whole temporary tree, so a link the fixture writes anywhere under it
    /// counts as inside the scope.
    fn shortcut_folders(dir: &tempfile::TempDir) -> Vec<PathBuf> {
        vec![dir.path().to_path_buf()]
    }

    fn payload_with(dir: &Path, entries: &[(&str, &[u8])]) -> (Metadata, Payload) {
        use tigersetup_format::compose::{EngineBlock, PayloadBytes, PayloadSource, compose};
        let mut metadata = metadata();
        metadata.files = entries
            .iter()
            .map(|(path, bytes)| File {
                path: path.to_string(),
                size: bytes.len() as u64,
                entry: path.to_string(),
                when: None,
            })
            .collect();
        // The metadata carries every directory its files need, as a built
        // package's does, because the format refuses one that does not.
        let mut implied: BTreeSet<String> = BTreeSet::new();
        for (path, _) in entries {
            let parts: Vec<&str> = path.split('/').collect();
            for depth in 1..parts.len() {
                implied.insert(parts[..depth].join("/"));
            }
        }
        metadata.directories = implied.into_iter().map(|path| Directory { path }).collect();
        let path = dir.join("pkg.exe");
        let out = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let sources = entries
            .iter()
            .map(|(path, bytes)| PayloadSource {
                entry: path.to_string(),
                bytes: PayloadBytes::Memory(bytes.to_vec()),
            })
            .collect();
        compose(
            out,
            &mut &b"loader"[..],
            &EngineBlock::compress(b"engine", tigersetup_format::payload::Compression::Fast)
                .unwrap(),
            &metadata,
            sources,
            tigersetup_format::payload::Compression::Fast,
        )
        .unwrap();
        let payload = tigersetup_format::Installer::open(&path)
            .unwrap()
            .payload()
            .unwrap();
        (metadata, payload)
    }

    #[test]
    fn upgrade_plan_keeps_replaces_adds_and_removes() {
        let test = TestRoots::new("upgrade");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(root.join("old")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("same.txt"), b"same").unwrap();
        std::fs::write(root.join("lib\\changed.dll"), b"version a").unwrap();
        std::fs::write(root.join("old\\gone.txt"), b"gone").unwrap();
        std::fs::write(root.join("old\\edited.txt"), b"user edit").unwrap();
        let owned = Owned {
            files: vec![
                OwnedFile {
                    path: "same.txt".into(),
                    sha256: hash(b"same"),
                    size: 4,
                    modified: None,
                },
                OwnedFile {
                    path: "lib\\changed.dll".into(),
                    sha256: hash(b"version a"),
                    size: 9,
                    modified: None,
                },
                OwnedFile {
                    path: "old\\gone.txt".into(),
                    sha256: hash(b"gone"),
                    size: 4,
                    modified: None,
                },
                OwnedFile {
                    path: "old\\edited.txt".into(),
                    sha256: hash(b"original"),
                    size: 8,
                    modified: None,
                },
            ],
            directories: vec![
                OwnedDirectory {
                    path: String::new(),
                    created: false,
                },
                OwnedDirectory {
                    path: "lib".into(),
                    created: true,
                },
                OwnedDirectory {
                    path: "old".into(),
                    created: true,
                },
            ],
            ..Default::default()
        };
        let entries: [(&str, &[u8]); 4] = [
            ("same.txt", b"same"),
            ("lib/changed.dll", b"version b!"),
            ("lib/new.dll", b"new"),
            ("plugins/p.bin", b"p"),
        ];
        let (metadata, mut payload) = payload_with(dir.path(), &entries);
        let options = BTreeMap::new();
        let desired = desired(
            &metadata,
            &options,
            Scope::User,
            &root,
            &dir.path().join("u.exe"),
            &test.roots,
            &owned,
        )
        .unwrap();

        let plan = |payload: &mut Payload, repair: bool| {
            reconcile(Reconcile {
                desired: Some(&desired),
                owned: &owned,
                install_root: &root,
                payload: Some(payload),
                roots: &test.roots,
                scope: Scope::User,
                inspect_files: true,
                repair,
                shortcut_folders: &shortcut_folders(&dir),
                firewall: None,
                actions: &ActionPlan::default(),
            })
            .unwrap()
        };
        let planned = plan(&mut payload, false);
        let files: Vec<(OpKind, String)> = summary(&planned)
            .into_iter()
            .filter(|(kind, _)| {
                matches!(
                    kind,
                    OpKind::KeepDirectory
                        | OpKind::CreateDirectory
                        | OpKind::KeepFile
                        | OpKind::InstallFile
                        | OpKind::RemoveFile
                        | OpKind::RemoveDirectory
                )
            })
            .collect();
        assert_eq!(
            files,
            vec![
                (OpKind::KeepDirectory, "".into()),
                (OpKind::KeepDirectory, "lib".into()),
                (OpKind::CreateDirectory, "plugins".into()),
                (OpKind::KeepFile, "same.txt".into()),
                (OpKind::InstallFile, "lib\\changed.dll".into()),
                (OpKind::InstallFile, "lib\\new.dll".into()),
                (OpKind::InstallFile, "plugins\\p.bin".into()),
                (OpKind::RemoveFile, "old\\gone.txt".into()),
            ],
            "old\\edited.txt is preserved, so old\\ is not removed"
        );
        assert_eq!(planned.operations[0].previous_existed, Some(true));
        assert_eq!(planned.operations[1].previous_existed, Some(false));
        assert_eq!(planned.operations[3].applied_sha256, Some(hash(b"same")));
        let c = planned.counts;
        assert_eq!((c.kept, c.replaced, c.added, c.removed), (1, 1, 2, 1));
        assert_eq!((c.directories_created, c.directories_removed), (1, 0));
        assert_eq!(planned.findings.len(), 1);
        assert_eq!(planned.findings[0].code, "file_modified_preserved");
        assert!(
            planned
                .operations
                .iter()
                .any(|op| op.kind == OpKind::CreateRegistryKey),
            "the registration is created last"
        );

        // A repair reports the owned file it rewrites.
        let repaired = plan(&mut payload, true);
        assert_eq!(repaired.counts.repaired, 1);
        assert!(repaired.findings.iter().any(|f| f.code == "file_repaired"
            && f.path.as_deref()
                == Some(root.join("lib\\changed.dll").display().to_string().as_str())));

        // Without the edited file, the old directory goes too.
        std::fs::remove_file(root.join("old\\edited.txt")).unwrap();
        let planned = plan(&mut payload, false);
        let last = planned.operations.last().unwrap();
        assert_eq!(
            (last.kind, last.target.as_str()),
            (OpKind::RemoveDirectory, "old")
        );
        assert_eq!(planned.findings[0].code, "file_missing");
        assert_eq!(planned.counts.directories_removed, 1);
    }
}
