//! Planning: desired state from the metadata and the effective options,
//! owned state from the database and actual state on the machine become an
//! ordered list of typed operations. Install, upgrade, reinstall, repair and
//! uninstall are the same reconciliation with different inputs: install has
//! no owned side, uninstall has no desired side, the others have both.
//!
//! Forward order: directories and files, product registry keys and values,
//! PATH entries, shortcuts, and the Add/Remove Programs registration last —
//! a registration means "installed" to Windows. Removals follow in the
//! reverse resource order, so an uninstall unregisters first and removes
//! the install root last.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use tigersetup_format::identity::Scope;
use tigersetup_format::{Metadata, PayloadArchive};

use crate::report::Finding;
use crate::resource::registry::DesiredValue;
use crate::resource::shortcut::DesiredShortcut;
use crate::resource::{directory, file, path, registration, registry, shortcut};
use crate::scope::{self, Locations};
use crate::state::installation::{Owned, OwnedDirectory, OwnedFile};
use crate::state::journal::OpKind;
use crate::win::fs::{self, Inspection};
use crate::win::registry::{self as winreg, KeyPath, Roots};
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
}

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
            previous_existed: None,
            value_name: None,
            value_kind: None,
            value_data: None,
            link_arguments: None,
            link_description: None,
            link_icon: None,
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

    fn value(kind: OpKind, key: &KeyPath, name: &str, data: &winreg::Data) -> PlannedOperation {
        PlannedOperation {
            value_name: Some(name.to_string()),
            value_kind: Some(data.kind_name()),
            value_data: Some(data.text()),
            ..PlannedOperation::new(kind, key.to_string())
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
            ..PlannedOperation::new(kind, link_path.display().to_string())
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

/// The directories a package needs, the root first, then shallowest first so
/// parents precede children. The metadata's list is authoritative — the
/// format guarantees it covers every file's parents — so the rule that
/// derives directories from file paths lives in the builder alone.
fn desired_directories(metadata: &Metadata) -> Vec<String> {
    let directories: BTreeSet<String> = metadata
        .directories
        .iter()
        .map(|d| to_relative(&d.path))
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
    pub registry_values: Vec<DesiredValue>,
    /// Raw PATH entry texts.
    pub path_entries: Vec<String>,
    pub shortcuts: Vec<DesiredShortcut>,
    pub registration_key: KeyPath,
    pub registration_values: Vec<DesiredValue>,
}

/// The effective value of every declared option: an explicit value wins,
/// then the recorded one, then the declared default. An explicit option
/// the package does not declare is refused.
pub fn effective_options(
    metadata: &Metadata,
    recorded: &BTreeMap<String, bool>,
    explicit: &BTreeMap<String, bool>,
) -> Result<BTreeMap<String, bool>> {
    for name in explicit.keys() {
        if metadata.option_default(name).is_none() {
            return Err(Error::new(
                "option_unknown",
                format!("{} declares no option {name:?}", metadata.package().name),
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
                .copied()
                .unwrap_or(option.default);
            (name, value)
        })
        .collect())
}

/// Resolves the desired state for a run.
pub fn desired(
    metadata: &Metadata,
    options: &BTreeMap<String, bool>,
    scope: Scope,
    install_root: &Path,
    uninstaller: &Path,
) -> Result<Desired> {
    let locations = scope::locations(scope);
    let (registration_key, registration_values) = registration::values(
        metadata,
        &locations,
        install_root,
        uninstaller,
        &registration::install_date(),
    )?;
    let path_entries = metadata
        .path_entries
        .iter()
        .filter(|entry| shortcut::enabled(&entry.option, options))
        .map(|entry| {
            absolute(install_root, &to_relative(&entry.path))
                .map(|p| p.display().to_string().trim_end_matches('\\').to_string())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Desired {
        directories: desired_directories(metadata),
        files: metadata
            .files
            .iter()
            .map(|f| DesiredFile {
                path: to_relative(&f.path),
                entry: f.entry.clone(),
                size: f.size,
            })
            .collect(),
        registry_values: registry::product_values(metadata, &locations, install_root)?,
        path_entries,
        shortcuts: shortcut::desired(metadata, options, &locations, install_root)?,
        registration_key,
        registration_values,
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
    /// Mutating operations on registry keys and values, PATH and shortcuts.
    pub resource_operations: usize,
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
    pub payload: Option<&'a mut PayloadArchive>,
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
}

/// Reconciles desired and owned state into operations.
pub fn reconcile(input: Reconcile<'_>) -> Result<Plan> {
    let mut plan = Plan {
        operations: Vec::new(),
        findings: Vec::new(),
        counts: PlanCounts::default(),
    };
    let mut removals: Vec<PlannedOperation> = Vec::new();
    let owned = input.owned;
    let locations = scope::locations(input.scope);
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
        let owned_file_keys: BTreeSet<String> = owned.files.iter().map(|f| key(&f.path)).collect();
        let mut payload = input.payload;
        for desired_file in &desired.files {
            desired_file_keys.insert(key(&desired_file.path));
            let is_owned = owned_file_keys.contains(&key(&desired_file.path));
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
            match fs::inspect(&target)? {
                Inspection::Present { sha256, .. } => {
                    let payload = payload.as_deref_mut().ok_or_else(|| {
                        Error::new("payload_unavailable", "this run carries no payload")
                    })?;
                    let wanted = file::payload_sha256(payload, &desired_file.entry)?;
                    if sha256 == wanted {
                        plan.counts.kept += 1;
                        plan.operations.push(PlannedOperation {
                            expected_size: Some(desired_file.size),
                            applied_sha256: Some(wanted),
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
        let needed: Vec<KeyPath> = distinct_keys(desired.registry_values.iter().map(|v| &v.key));
        reconcile_keys(
            &needed,
            &locations.software_root,
            input.roots,
            owned,
            &mut keys_kept_or_created,
            &mut plan,
        )?;
    }
    reconcile_values(
        input.desired.map(|d| &d.registry_values[..]).unwrap_or(&[]),
        &product_values,
        input.roots,
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
                if shortcut::targets_install_root(&current.target, input.install_root) =>
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
    removals.append(&mut shortcut_removals);
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
    Ok(plan)
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

/// Keeps or sets every desired value; for owned values not desired, plans a
/// removal when the value still holds what TigerSetup wrote, and reports it
/// otherwise.
fn reconcile_values(
    desired: &[DesiredValue],
    owned: &[&crate::state::installation::OwnedRegistryValue],
    roots: &Roots,
    plan: &mut Plan,
    removals: &mut Vec<PlannedOperation>,
) -> Result<()> {
    let mut desired_keys = BTreeSet::new();
    for wanted in desired {
        desired_keys.insert((wanted.key.key(), wanted.name.to_ascii_lowercase()));
        let current = winreg::read_value(roots, &wanted.key, &wanted.name)?;
        let kind = if current.as_ref() == Some(&wanted.data) {
            OpKind::KeepRegistryValue
        } else {
            plan.counts.resource_operations += 1;
            OpKind::SetRegistryValue
        };
        plan.operations.push(PlannedOperation::value(
            kind,
            &wanted.key,
            &wanted.name,
            &wanted.data,
        ));
    }
    for existing in owned {
        let Ok(key_path) = KeyPath::parse(&existing.key) else {
            continue;
        };
        if desired_keys.contains(&(key_path.key(), existing.name.to_ascii_lowercase())) {
            continue;
        }
        let recorded = registry::owned_data(existing)?;
        let location = format!("{}\\{}", existing.key, existing.name);
        match winreg::read_value(roots, &key_path, &existing.name)? {
            None => plan
                .findings
                .push(Finding::named("registry_value_missing", location)),
            Some(current) if current == recorded => {
                plan.counts.resource_operations += 1;
                removals.push(PlannedOperation::value(
                    OpKind::RemoveRegistryValue,
                    &key_path,
                    &existing.name,
                    &recorded,
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
    match fs::inspect(&target)? {
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
    use crate::win::registry::Data;
    use tigersetup_format::metadata::{
        Directory, Engine, File, Install, InstallOption, Package, PathEntry, Registration,
        RegistryKind, RegistryValue,
    };

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
                },
                File {
                    path: "a.txt".into(),
                    size: 2,
                    entry: "a.txt".into(),
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
        let recorded = BTreeMap::from([("path".to_string(), false)]);
        let explicit = BTreeMap::from([("desktop-shortcut".to_string(), true)]);
        let effective = effective_options(&metadata, &recorded, &explicit).unwrap();
        assert!(!effective["path"], "recorded wins over the default");
        assert!(effective["desktop-shortcut"], "explicit wins");
        let empty = BTreeMap::new();
        assert!(effective_options(&metadata, &empty, &empty).unwrap()["path"]);
        let unknown = BTreeMap::from([("nope".to_string(), true)]);
        assert_eq!(
            effective_options(&metadata, &empty, &unknown)
                .unwrap_err()
                .code,
            "option_unknown"
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
        });
        metadata.registry_values.push(RegistryValue {
            key: "IT Tiger\\T".into(),
            name: "InstallRoot".into(),
            kind: RegistryKind::ExpandString as i32,
            data: "%INSTALLROOT%".into(),
        });
        metadata.registration = Some(Registration::default());
        let options = effective_options(&metadata, &BTreeMap::new(), &BTreeMap::new()).unwrap();
        let desired = desired(
            &metadata,
            &options,
            Scope::User,
            &root,
            &dir.path().join("uninstall.exe"),
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
            files: vec![
                OwnedFile {
                    path: "a.txt".into(),
                    sha256: hash(b"aa"),
                    size: 2,
                },
                OwnedFile {
                    path: "b\\deep\\z.txt".into(),
                    sha256: hash(b"z"),
                    size: 1,
                },
                OwnedFile {
                    path: "c\\gone.txt".into(),
                    sha256: hash(b"g"),
                    size: 1,
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
            files: vec![OwnedFile {
                path: "b\\deep\\z.txt".into(),
                sha256: hash(b"z"),
                size: 1,
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
                },
                OwnedRegistryValue {
                    key: product.to_string(),
                    name: "Edited".into(),
                    kind: "string".into(),
                    data: "original".into(),
                },
                OwnedRegistryValue {
                    key: product.to_string(),
                    name: "Gone".into(),
                    kind: "dword".into(),
                    data: "1".into(),
                },
                OwnedRegistryValue {
                    key: registration.to_string(),
                    name: "DisplayName".into(),
                    kind: "string".into(),
                    data: "T".into(),
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

    /// Builds a payload archive holding `entries` in a temporary installer
    /// file, so the planner can hash entries the way the engine does.
    /// The scope's shortcut folders as a unit test's fixtures use them: the
    /// whole temporary tree, so a link the fixture writes anywhere under it
    /// counts as inside the scope.
    fn shortcut_folders(dir: &tempfile::TempDir) -> Vec<PathBuf> {
        vec![dir.path().to_path_buf()]
    }

    fn payload_with(dir: &Path, entries: &[(&str, &[u8])]) -> (Metadata, PayloadArchive) {
        use tigersetup_format::compose::{PayloadSource, compose};
        let mut metadata = metadata();
        metadata.files = entries
            .iter()
            .map(|(path, bytes)| File {
                path: path.to_string(),
                size: bytes.len() as u64,
                entry: path.to_string(),
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
        let sources = entries.iter().map(|(path, bytes)| {
            Ok(PayloadSource {
                entry: path.to_string(),
                bytes: bytes.to_vec(),
            })
        });
        compose(
            out,
            &mut &b"engine"[..],
            &metadata,
            sources,
            tigersetup_format::payload::Compression::Fast,
        )
        .unwrap();
        let payload = tigersetup_format::Installer::open(&path)
            .unwrap()
            .payload_archive()
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
                },
                OwnedFile {
                    path: "lib\\changed.dll".into(),
                    sha256: hash(b"version a"),
                    size: 9,
                },
                OwnedFile {
                    path: "old\\gone.txt".into(),
                    sha256: hash(b"gone"),
                    size: 4,
                },
                OwnedFile {
                    path: "old\\edited.txt".into(),
                    sha256: hash(b"original"),
                    size: 8,
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
        )
        .unwrap();

        let plan = |payload: &mut PayloadArchive, repair: bool| {
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
