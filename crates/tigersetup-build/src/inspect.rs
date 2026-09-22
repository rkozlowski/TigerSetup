//! `tiger-setup inspect` and `tiger-setup verify`: decode an installer file
//! without executing it and report what it claims and whether it holds —
//! the metadata, the payload, the engine block against the recorded hashes,
//! and the Windows identity and icon the file presents to Explorer. `inspect`
//! can also write the blocks out: the embedded ZIP payload and the metadata
//! exactly as the file carries them, and the metadata decoded to JSON.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tigersetup_format::identity::Scope;
use tigersetup_format::metadata::{
    AcquisitionSource, ContextMenuTarget, DetectorKind, ExistingScopePolicy, FirewallAction,
    FirewallDirection, FirewallProtocol, Metadata, OptionKind, Predicate, RegistryKind, Role,
    ShortcutLocation,
};
use tigersetup_format::{Installer, hex, sha256};

use crate::metadata::version_info::{self, VersionInfo};
use crate::resource::{self, Image};
use crate::{BuildError, Result};

/// The stable name of an enumerated metadata value. Machine-readable output
/// carries these, never a number and never localized text.
fn role_name(role: i32) -> &'static str {
    match Role::try_from(role) {
        Ok(Role::Uninstaller) => "uninstaller",
        Ok(Role::Installer) => "installer",
        _ => "unspecified",
    }
}

fn shortcut_location_name(location: i32) -> &'static str {
    match ShortcutLocation::try_from(location) {
        Ok(ShortcutLocation::Desktop) => "desktop",
        Ok(ShortcutLocation::StartMenu) => "start-menu",
        Ok(ShortcutLocation::Startup) => "startup",
        Ok(ShortcutLocation::SendTo) => "send-to",
        _ => "unspecified",
    }
}

fn context_menu_target_name(target: i32) -> &'static str {
    match ContextMenuTarget::try_from(target) {
        Ok(ContextMenuTarget::Files) => "files",
        Ok(ContextMenuTarget::Directories) => "directories",
        Ok(ContextMenuTarget::DirectoryBackground) => "directory-background",
        _ => "unspecified",
    }
}

fn firewall_direction_name(direction: i32) -> &'static str {
    match FirewallDirection::try_from(direction) {
        Ok(FirewallDirection::In) => "in",
        Ok(FirewallDirection::Out) => "out",
        _ => "unspecified",
    }
}

fn firewall_action_name(action: i32) -> &'static str {
    match FirewallAction::try_from(action) {
        Ok(FirewallAction::Allow) => "allow",
        Ok(FirewallAction::Block) => "block",
        _ => "unspecified",
    }
}

fn firewall_protocol_name(protocol: i32) -> &'static str {
    match FirewallProtocol::try_from(protocol) {
        Ok(FirewallProtocol::Tcp) => "tcp",
        Ok(FirewallProtocol::Udp) => "udp",
        _ => "any",
    }
}

/// A resource's predicate as the documents carry it: the `when` field, or
/// the older `option` spelling, as one object; `null` for a resource that
/// is always there.
fn predicate_json(when: Option<&Predicate>, option: &str) -> Value {
    match Predicate::of(when, option) {
        Some(predicate) => json!({ "option": predicate.option, "equals": predicate.equals }),
        None => Value::Null,
    }
}

fn registry_kind_name(kind: i32) -> &'static str {
    match RegistryKind::try_from(kind) {
        Ok(RegistryKind::ExpandString) => "expand-string",
        Ok(RegistryKind::Dword) => "dword",
        Ok(RegistryKind::String) => "string",
        _ => "unspecified",
    }
}

fn detector_kind_name(kind: i32) -> &'static str {
    match DetectorKind::try_from(kind) {
        Ok(DetectorKind::DirectoryVersion) => "directory-version",
        Ok(DetectorKind::RegistryVersion) => "registry-version",
        Ok(DetectorKind::FileVersion) => "file-version",
        Ok(DetectorKind::Registration) => "registration",
        _ => "unspecified",
    }
}

fn acquisition_source_name(source: i32) -> &'static str {
    match AcquisitionSource::try_from(source) {
        Ok(AcquisitionSource::Winget) => "winget",
        Ok(AcquisitionSource::Url) => "url",
        Ok(AcquisitionSource::Embedded) => "embedded",
        _ => "unspecified",
    }
}

fn option_kind_name(kind: i32) -> &'static str {
    match OptionKind::try_from(kind) {
        Ok(OptionKind::Custom) => "custom",
        Ok(OptionKind::Path) => "path",
        Ok(OptionKind::DesktopShortcut) => "desktop-shortcut",
        Ok(OptionKind::Choice) => "choice",
        _ => "unspecified",
    }
}

/// The manifest spelling of a scope policy; an unspecified one is reported
/// as the default it means.
fn existing_scope_policy_name(policy: i32) -> &'static str {
    ExistingScopePolicy::try_from(policy)
        .unwrap_or(ExistingScopePolicy::Preserve)
        .as_str()
}

fn scope_name(tag: i32) -> Value {
    match Scope::from_tag(tag) {
        Some(scope) => Value::String(scope.as_str().into()),
        None => Value::Null,
    }
}

/// ` (option x)` for a resource an installer option controls.
fn option_suffix(option: &str) -> String {
    if option.is_empty() {
        String::new()
    } else {
        format!(" (option {option})")
    }
}

/// The declared files, in install order.
fn files_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .files
        .iter()
        .map(|f| {
            json!({
                "path": f.path, "size": f.size, "entry": f.entry,
                "when": predicate_json(f.when.as_ref(), ""),
            })
        })
        .collect()
}

/// The file batches as the engine journals them: each batch's index, the
/// range of `files` it holds and its bytes, so a reader can tell which
/// batch a file's installation is checkpointed with.
fn file_batches_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .file_batches
        .iter()
        .enumerate()
        .map(|(index, b)| {
            json!({
                "batch": index,
                "first_file": b.first_file,
                "file_count": b.file_count,
                "bytes": b.bytes,
            })
        })
        .collect()
}

fn directories_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .directories
        .iter()
        .map(|d| Value::String(d.path.clone()))
        .collect()
}

/// The declared options with their kind, the wizard labels a custom or
/// choice one carries by locale, and a choice option's values. `default` is
/// a boolean for a boolean option and the chosen value for a choice, as
/// every TigerSetup document spells option values.
fn options_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .options
        .iter()
        .map(|o| {
            let default = if o.is_choice() {
                Value::String(o.default_choice.clone())
            } else {
                Value::Bool(o.default)
            };
            let mut entry = json!({
                "name": o.name,
                "default": default,
                "kind": option_kind_name(o.kind),
                "labels": o.labels,
            });
            if o.is_choice() {
                entry["choices"] = o
                    .choices
                    .iter()
                    .map(|c| json!({ "value": c.value, "labels": c.labels }))
                    .collect();
            }
            entry
        })
        .collect()
}

fn shortcuts_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .shortcuts
        .iter()
        .map(|s| {
            let when = predicate_json(s.when.as_ref(), &s.option);
            json!({
                "location": shortcut_location_name(s.location), "name": s.name, "target": s.target,
                "arguments": s.arguments, "description": s.description, "icon": s.icon,
                // `option` keeps the older spelling for readers that key on it.
                "option": Predicate::of(s.when.as_ref(), &s.option).filter(|p| p.equals == "true").map(|p| p.option).unwrap_or_default(),
                "folder": s.folder,
                "when": when,
                "working_directory": s.working_directory,
                "app_user_model_id": s.app_user_model_id,
                "url": s.url,
                "kind": if s.url.is_empty() { "link" } else { "url" },
            })
        })
        .collect()
}

fn path_entries_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .path_entries
        .iter()
        .map(|p| {
            json!({
                "path": p.path,
                "option": Predicate::of(p.when.as_ref(), &p.option).filter(|w| w.equals == "true").map(|w| w.option).unwrap_or_default(),
                "when": predicate_json(p.when.as_ref(), &p.option),
            })
        })
        .collect()
}

fn registry_values_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .registry_values
        .iter()
        .map(|r| {
            json!({
                "root": r.root_name(),
                "key": r.key, "name": r.name, "kind": registry_kind_name(r.kind), "data": r.data,
                "when": predicate_json(r.when.as_ref(), ""),
            })
        })
        .collect()
}

fn environment_variables_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .environment_variables
        .iter()
        .map(|v| {
            json!({
                "name": v.name, "value": v.value, "expandable": v.expandable,
                "when": predicate_json(v.when.as_ref(), ""),
            })
        })
        .collect()
}

fn file_associations_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .file_associations
        .iter()
        .map(|a| {
            json!({
                "prog_id": a.prog_id, "extensions": a.extensions, "description": a.description,
                "icon": a.icon, "executable": a.executable, "arguments": a.arguments,
                "when": predicate_json(a.when.as_ref(), ""),
            })
        })
        .collect()
}

fn url_protocols_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .url_protocols
        .iter()
        .map(|u| {
            json!({
                "scheme": u.scheme, "prog_id": u.prog_id, "description": u.description,
                "icon": u.icon, "executable": u.executable, "arguments": u.arguments,
                "when": predicate_json(u.when.as_ref(), ""),
            })
        })
        .collect()
}

fn app_paths_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .app_paths
        .iter()
        .map(|a| {
            json!({
                "executable": a.executable, "add_directory": a.add_directory,
                "when": predicate_json(a.when.as_ref(), ""),
            })
        })
        .collect()
}

fn context_menu_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .context_menu_verbs
        .iter()
        .map(|v| {
            json!({
                "target": context_menu_target_name(v.target), "verb": v.verb, "label": v.label,
                "executable": v.executable, "arguments": v.arguments, "icon": v.icon,
                "extensions": v.extensions,
                "when": predicate_json(v.when.as_ref(), ""),
            })
        })
        .collect()
}

fn firewall_rules_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .firewall_rules
        .iter()
        .map(|f| {
            json!({
                "name": f.name, "description": f.description, "program": f.program,
                "direction": firewall_direction_name(f.direction),
                "action": firewall_action_name(f.action),
                "protocol": firewall_protocol_name(f.protocol),
                "local_ports": f.local_ports,
                "when": predicate_json(f.when.as_ref(), ""),
            })
        })
        .collect()
}

/// The declared custom actions, in declaration order, with the identity
/// of every packaged program so that a reader can see what an installer
/// will run and verify the bytes it carries for it.
fn action_json(a: &tigersetup_format::metadata::Action) -> Value {
    let packaged = if a.is_packaged() {
        json!({
            "entry": a.entry,
            "file_name": a.file_name,
            "size": a.size,
            "sha256": a.sha256,
        })
    } else {
        Value::Null
    };
    json!({
        "name": a.name,
        "phase": a.phase().as_str(),
        "run_on": a.operations().iter().map(|op| op.as_str()).collect::<Vec<_>>(),
        "kind": a.kind().as_str(),
        "command": a.command,
        "packaged": packaged,
        "arguments": a.arguments,
        "working_directory": a.working_directory,
        "timeout_seconds": a.timeout_seconds(),
        "success_codes": a.success_codes(),
        "reboot_codes": a.reboot_codes,
        "on_failure": a.failure_policy().as_str(),
        "when": predicate_json(a.when.as_ref(), ""),
    })
}

fn actions_json(metadata: &Metadata) -> Vec<Value> {
    metadata.actions.iter().map(action_json).collect()
}

fn quiescence_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .quiescence
        .iter()
        .map(|q| {
            json!({
                "name": q.name,
                "run_on": q.operations().iter().map(|op| op.as_str()).collect::<Vec<_>>(),
                "not_running_codes": q.not_running_codes,
                "stop": q.stop.as_ref().map(action_json).unwrap_or(Value::Null),
                "resume": q.resume.as_ref().map(action_json).unwrap_or(Value::Null),
                "when": predicate_json(q.when.as_ref(), ""),
            })
        })
        .collect()
}

/// The completion page's launch offer, or `null` for a package that makes
/// none. An absent working directory is the executable's own; an empty one
/// is the install root.
fn launch_json(metadata: &Metadata) -> Value {
    match &metadata.launch {
        Some(l) => json!({
            "executable": l.executable,
            "arguments": l.arguments,
            "working_directory": l.working_directory,
            "checked": l.checked,
        }),
        None => Value::Null,
    }
}

fn legacy_json(metadata: &Metadata) -> Value {
    match &metadata.legacy {
        Some(l) => json!({
            "installer_type": l.installer_type, "registration_key": l.registration_key
        }),
        None => Value::Null,
    }
}

/// The declared dependencies with their detection rules and acquisition
/// hints, as a generated installer carries them.
fn dependencies_json(metadata: &Metadata) -> Vec<Value> {
    metadata
        .dependencies
        .iter()
        .map(|dependency| {
            let detect = dependency.detect.clone().unwrap_or_default();
            let mut entry = json!({
                "id": dependency.id,
                "display_name": dependency.display_name,
                "minimum_version": dependency.minimum_version,
                "elevation_required": dependency.elevation_required,
                "when": predicate_json(dependency.when.as_ref(), ""),
                "detect": {
                    "kind": detector_kind_name(detect.kind),
                    "path": detect.path,
                    "keys": detect.keys,
                    "value": detect.value,
                    "pattern": detect.pattern,
                },
            });
            if let Some(acquisition) = &dependency.acquisition {
                entry["acquisition"] = json!({
                    "source": acquisition_source_name(acquisition.source),
                    "package_identifier": acquisition.package_identifier,
                    "version": acquisition.version,
                    "url": acquisition.url,
                    "sha256": acquisition.sha256,
                    "architecture": acquisition.architecture,
                    "resolved_at": acquisition.resolved_at,
                    "max_age_days": acquisition.max_age_days,
                    "scope": acquisition.scope,
                    "installer_type": acquisition.installer_type,
                    "entry": acquisition.entry,
                    "size": acquisition.size,
                });
            }
            if let Some(install) = &dependency.install {
                entry["install"] = json!({
                    "arguments": install.arguments,
                    "success_codes": install.success_codes,
                    "reboot_codes": install.reboot_codes,
                    "declared": install.declared,
                });
            }
            entry
        })
        .collect()
}

/// The embedded metadata decoded to one JSON document: the message tree of
/// `proto/tigersetup.proto` with every field under its proto name.
/// Enumerations carry their stable names, an absent message is `null`, and
/// the product icon's bytes are described by their length and SHA-256 rather
/// than dumped. Nothing here comes from the footer or from reading the
/// payload: it is what the metadata block says, and only that.
pub fn metadata_json(metadata: &Metadata) -> Value {
    let package = metadata.package();
    let install = metadata.install();
    let engine = metadata.engine();
    json!({
        "schema": metadata.schema,
        "role": role_name(metadata.role),
        "uninstaller_scope": scope_name(metadata.uninstaller_scope),
        "package": {
            "id": package.id,
            "name": package.name,
            "version": package.version,
            "publisher": package.publisher,
            "description": package.description,
            "copyright": package.copyright,
            "license": package.license,
            "license_text": package.license_text,
            "website_url": package.website_url,
            "support_url": package.support_url,
            "help_url": package.help_url,
            "icon": if package.icon.is_empty() {
                Value::Null
            } else {
                json!({ "bytes": package.icon.len(), "sha256": hex(&sha256(&package.icon)) })
            },
            "file_version": package.file_version,
        },
        "install": {
            "scopes": install.scopes.iter().map(|tag| scope_name(*tag)).collect::<Vec<_>>(),
            "user_root": install.user_root,
            "machine_root": install.machine_root,
            "minimum_build": install.minimum_build,
            "architecture": install.architecture,
            "estimated_size": install.estimated_size,
            "existing_scope": existing_scope_policy_name(install.existing_scope),
        },
        "files": files_json(metadata),
        "directories": directories_json(metadata),
        "engine": {
            "tigersetup_version": engine.tigersetup_version,
            "engine_sha256": engine.engine_sha256,
            "engine_block_sha256": engine.engine_block_sha256,
        },
        "options": options_json(metadata),
        "shortcuts": shortcuts_json(metadata),
        "path_entries": path_entries_json(metadata),
        "registry_values": registry_values_json(metadata),
        "registration": match &metadata.registration {
            Some(r) => json!({
                "key_name": r.key_name,
                "display_name": r.display_name,
                "display_version": r.display_version,
                "display_icon": r.display_icon,
            }),
            None => Value::Null,
        },
        "legacy": legacy_json(metadata),
        "dependencies": dependencies_json(metadata),
        "environment_variables": environment_variables_json(metadata),
        "file_associations": file_associations_json(metadata),
        "url_protocols": url_protocols_json(metadata),
        "app_paths": app_paths_json(metadata),
        "context_menu_verbs": context_menu_json(metadata),
        "firewall_rules": firewall_rules_json(metadata),
        "actions": actions_json(metadata),
        "quiescence": quiescence_json(metadata),
        "file_batches": file_batches_json(metadata),
        "launch": launch_json(metadata),
    })
}

/// What `inspect` writes beside its report, each to the file it names.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    /// The compressed payload block, byte for byte.
    pub payload: Option<PathBuf>,
    /// The payload reconstructed as an ordinary ZIP archive of stored
    /// entries, one per payload entry, in stream order.
    pub zip: Option<PathBuf>,
    /// The embedded metadata block, byte for byte.
    pub meta: Option<PathBuf>,
    /// The metadata decoded, as [`metadata_json`] renders it.
    pub meta_json: Option<PathBuf>,
    /// The engine executable the loader runs, decompressed.
    pub engine: Option<PathBuf>,
}

impl ExportRequest {
    pub fn is_empty(&self) -> bool {
        self.destinations().is_empty()
    }

    fn destinations(&self) -> Vec<&Path> {
        [
            &self.payload,
            &self.zip,
            &self.meta,
            &self.meta_json,
            &self.engine,
        ]
        .into_iter()
        .flatten()
        .map(PathBuf::as_path)
        .collect()
    }
}

pub struct Inspection {
    pub installer: Installer,
    /// SHA-256 of the engine executable the compressed block decompresses
    /// to, established by decompressing it; empty when the block does not
    /// decompress to what the footer declares, which `engine_problem` says.
    pub engine_block_sha256: String,
    /// Why the engine block could not be decompressed and verified, if it
    /// could not.
    pub engine_problem: Option<tigersetup_format::FormatError>,
    pub entries: Vec<tigersetup_format::EntryInfo>,
    pub verification: tigersetup_format::VerifyOutcome,
    /// The version resource the file presents to Explorer; `None` when the
    /// file has none the resource API can read.
    pub windows: Option<VersionInfo>,
    /// The images of the executable's icon (`RT_GROUP_ICON 1`); empty when
    /// the file has none.
    pub icon: Vec<Image>,
}

pub fn inspect(path: &Path) -> Result<Inspection> {
    let installer = Installer::open(path)?;
    // Decompressing the engine proves the block, its length and the hash
    // of the executable it yields — exactly what the loader checks before
    // it runs anything.
    let (engine_block_sha256, engine_problem) = match installer.extract_engine(&mut std::io::sink())
    {
        Ok(_) => (installer.engine_executable_sha256_hex(), None),
        Err(err) => (String::new(), Some(err)),
    };
    let entries = installer.entries()?;
    let verification = installer.verify()?;
    // A file without a readable version resource or icon is reported as
    // such rather than refused: the format does not require either, and an
    // installer whose engine block was replaced by hand is still worth
    // reading for everything else.
    let windows = version_info::read(path).ok();
    let icon = resource::read_group_icon(path, resource::EXECUTABLE_ICON_ID)
        .ok()
        .flatten()
        .unwrap_or_default();
    Ok(Inspection {
        installer,
        engine_block_sha256,
        engine_problem,
        entries,
        verification,
        windows,
        icon,
    })
}

impl Inspection {
    /// The engine-executable hash the metadata recorded.
    pub fn expected_engine_block_sha256(&self) -> &str {
        &self.installer.metadata().engine().engine_block_sha256
    }

    /// Whether the engine block decompresses to the executable the footer
    /// and the metadata both recorded.
    pub fn engine_block_matches(&self) -> bool {
        self.engine_problem.is_none()
            && self.engine_block_sha256 == self.expected_engine_block_sha256()
    }

    pub fn is_ok(&self) -> bool {
        self.verification.is_ok() && self.engine_block_matches()
    }

    /// Writes the requested exports, and returns the paths written in the
    /// order they were.
    ///
    /// The payload block and the metadata block are decomposition: the very
    /// byte ranges the footer addresses, read through the same validated
    /// layout `verify` hashes, so a hash of an exported file is the hash the
    /// footer records. The ZIP and the engine are reconstruction: the
    /// payload stream decoded into an ordinary archive of stored entries
    /// any tool opens, and the engine executable decompressed as the loader
    /// runs it. An installer that failed verification exports nothing: a
    /// payload that does not match its hash would come out looking like a
    /// good one.
    ///
    /// No destination is overwritten. Every destination is checked before
    /// the first byte is written, so a conflict on the last one does not
    /// leave the first ones behind; a destination that fails mid-write is
    /// removed rather than left partial.
    pub fn export(&self, request: &ExportRequest) -> Result<Vec<PathBuf>> {
        if !self.is_ok() {
            return Err(BuildError::new(
                "export_refused",
                "the installer failed verification; nothing was exported",
            ));
        }
        for destination in request.destinations() {
            if destination.exists() {
                return Err(BuildError::new(
                    "output_exists",
                    format!(
                        "{} already exists; nothing was exported",
                        destination.display()
                    ),
                ));
            }
        }
        let mut written = Vec::new();
        if let Some(path) = &request.payload {
            let mut block = self.installer.payload_block()?;
            write_export(path, |file| {
                std::io::copy(&mut block, file)?;
                Ok(())
            })?;
            written.push(path.clone());
        }
        if let Some(path) = &request.zip {
            let mut payload = self.installer.payload()?;
            let entries = self.entries.clone();
            write_export(path, |file| {
                let mut archive = zip::ZipWriter::new(file);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored)
                    .last_modified_time(zip::DateTime::default());
                for entry in &entries {
                    let options = options.large_file(entry.length >= u32::MAX as u64);
                    archive
                        .start_file(&entry.entry, options)
                        .map_err(|err| BuildError::new("output_unwritable", err.to_string()))?;
                    payload.copy_entry(&entry.entry, &mut archive)?;
                }
                archive
                    .finish()
                    .map_err(|err| BuildError::new("output_unwritable", err.to_string()))?;
                Ok(())
            })?;
            written.push(path.clone());
        }
        if let Some(path) = &request.engine {
            write_export(path, |file| {
                self.installer.extract_engine(file)?;
                Ok(())
            })?;
            written.push(path.clone());
        }
        if let Some(path) = &request.meta {
            write_export(path, |file| {
                Ok(file.write_all(self.installer.metadata_bytes())?)
            })?;
            written.push(path.clone());
        }
        if let Some(path) = &request.meta_json {
            let document = metadata_json(self.installer.metadata());
            let text = serde_json::to_string_pretty(&document).unwrap_or_default();
            write_export(path, |file| {
                file.write_all(text.as_bytes())?;
                Ok(file.write_all(b"\n")?)
            })?;
            written.push(path.clone());
        }
        Ok(written)
    }

    fn windows_json(&self) -> Value {
        match &self.windows {
            Some(info) => json!({
                "file_description": info.file_description,
                "product_name": info.product_name,
                "product_version": info.product_version,
                "file_version": info.file_version_string,
                "company_name": info.company_name,
                "copyright": info.legal_copyright,
                "original_filename": info.original_filename,
                "internal_name": info.internal_name,
            }),
            None => Value::Null,
        }
    }

    fn icon_json(&self) -> Vec<Value> {
        self.icon
            .iter()
            .map(|image| {
                json!({
                    "width": image.width(),
                    "height": image.height(),
                    "bits": image.bits(),
                    "bytes": image.bytes.len(),
                    "sha256": hex(&sha256(&image.bytes)),
                })
            })
            .collect()
    }

    pub fn to_json(&self) -> Value {
        let metadata = self.installer.metadata();
        let package = metadata.package();
        let layout = self.installer.layout();
        let footer = self.installer.footer();
        let mut problems: Vec<Value> = self
            .verification
            .problems
            .iter()
            .map(|p| json!({ "code": p.code, "message": p.message }))
            .collect();
        if let Some(err) = &self.engine_problem {
            problems.push(json!({ "code": err.code, "message": err.message }));
        } else if !self.engine_block_matches() {
            problems.push(json!({ "code": "engine_hash_mismatch", "message": "the engine executable does not match the hash recorded in the metadata" }));
        }
        let registration = metadata.registration.clone().unwrap_or_default();
        json!({
            "schema": 1,
            "file": self.installer.path().display().to_string(),
            "role": role_name(metadata.role),
            "format": { "major": footer.format_major, "minor": footer.format_minor },
            "layout": {
                "file_length": layout.file_length,
                "loader_length": layout.loader_length,
                "engine_offset": layout.engine_offset,
                "engine_length": layout.engine_length,
                "engine_uncompressed_length": layout.engine_uncompressed_length,
                "payload_offset": layout.payload_offset,
                "payload_length": layout.payload_length,
                "payload_uncompressed_length": layout.payload_uncompressed_length,
                "metadata_offset": layout.metadata_offset,
                "metadata_length": layout.metadata_length,
                "metadata_uncompressed_length": layout.metadata_uncompressed_length,
                "footer_offset": layout.footer_offset,
            },
            "package": {
                "id": package.id,
                "name": package.name,
                "version": package.version,
                "publisher": package.publisher,
                "scopes": metadata.scopes().iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                "install_roots": {
                    "user": metadata.install().user_root,
                    "machine": metadata.install().machine_root,
                },
                "minimum_build": metadata.minimum_build(),
                "architecture": metadata.install().architecture,
                "estimated_size": metadata.install().estimated_size,
                "metadata_sha256": hex(&footer.metadata_sha256),
                "metadata_block_sha256": hex(&footer.metadata_block_sha256),
                "payload_sha256": hex(&footer.payload_sha256),
                "engine": {
                    "tigersetup_version": metadata.engine().tigersetup_version,
                    "engine_sha256": metadata.engine().engine_sha256,
                    "engine_block_sha256": metadata.engine().engine_block_sha256,
                    "engine_block_sha256_actual": self.engine_block_sha256,
                    "engine_compressed_sha256": hex(&footer.engine_sha256),
                    "loader_sha256": metadata.engine().loader_sha256,
                    "loader_block_sha256": metadata.engine().loader_block_sha256,
                },
            },
            "windows": self.windows_json(),
            "icon": self.icon_json(),
            "files": files_json(metadata),
            "file_batches": file_batches_json(metadata),
            "directories": directories_json(metadata),
            "options": options_json(metadata),
            "shortcuts": shortcuts_json(metadata),
            "path_entries": path_entries_json(metadata),
            "registry_values": registry_values_json(metadata),
            "registration": {
                "key_name": metadata.registration_key_name(),
                "display_name": registration.display_name,
                "display_version": registration.display_version,
                "display_icon": registration.display_icon,
            },
            "legacy": legacy_json(metadata),
            "dependencies": dependencies_json(metadata),
            "environment_variables": environment_variables_json(metadata),
            "file_associations": file_associations_json(metadata),
            "url_protocols": url_protocols_json(metadata),
            "app_paths": app_paths_json(metadata),
            "context_menu_verbs": context_menu_json(metadata),
            "firewall_rules": firewall_rules_json(metadata),
            "actions": actions_json(metadata),
            "quiescence": quiescence_json(metadata),
            "launch": launch_json(metadata),
            "entries": self.entries.iter().map(|e| json!({
                "name": e.entry, "size": e.length, "offset": e.offset, "crc32": format!("{:08x}", e.crc32)
            })).collect::<Vec<_>>(),
            "verification": { "status": if self.is_ok() { "ok" } else { "failed" }, "entries_checked": self.verification.entries_checked, "problems": problems },
        })
    }

    pub fn to_text(&self) -> String {
        let metadata = self.installer.metadata();
        let package = metadata.package();
        let layout = self.installer.layout();
        let footer = self.installer.footer();
        let mut out = String::new();
        out.push_str(&format!("File:      {}\n", self.installer.path().display()));
        out.push_str(&format!(
            "Format:    {}.{}\n",
            footer.format_major, footer.format_minor
        ));
        out.push_str(&format!(
            "Package:   {} ({}) {} by {}\n",
            package.name, package.id, package.version, package.publisher
        ));
        out.push_str(&format!(
            "Scopes:    {}\n",
            metadata
                .scopes()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!(
            "Root:      user={}\n",
            metadata.install().user_root
        ));
        out.push_str(&format!(
            "Engine:    TigerSetup {} sha256 {}\n",
            metadata.engine().tigersetup_version,
            metadata.engine().engine_sha256
        ));
        out.push_str(&format!(
            "Block:     sha256 {} ({} B compressed to {} B)\n",
            self.expected_engine_block_sha256(),
            layout.engine_uncompressed_length,
            layout.engine_length
        ));
        out.push_str(&format!(
            "Loader:    sha256 {} ({} B)\n",
            metadata.engine().loader_sha256,
            layout.loader_length
        ));
        if let Some(info) = &self.windows {
            out.push_str(&format!(
                "Windows:   {} · {} {} · {} · {}\n",
                info.file_description,
                info.product_name,
                info.product_version,
                info.company_name,
                info.original_filename
            ));
            if !info.legal_copyright.is_empty() {
                out.push_str(&format!("           {}\n", info.legal_copyright));
            }
        }
        if !self.icon.is_empty() {
            out.push_str(&format!(
                "Icon:      {}\n",
                self.icon
                    .iter()
                    .map(|image| format!("{}×{}", image.width(), image.height()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        out.push_str(&format!(
            "Layout:    loader {} B | engine {} B @ {} | payload {} B @ {} | metadata {} B @ {} | footer @ {}\n",
            layout.loader_length,
            layout.engine_length,
            layout.engine_offset,
            layout.payload_length,
            layout.payload_offset,
            layout.metadata_length,
            layout.metadata_offset,
            layout.footer_offset
        ));
        out.push_str(&format!(
            "Metadata:  sha256 {} ({} B in one zstd block from {} B; {} files in {} batches)\n",
            hex(&footer.metadata_sha256),
            layout.metadata_length,
            layout.metadata_uncompressed_length,
            metadata.files.len(),
            metadata.file_batches.len()
        ));
        out.push_str(&format!(
            "Payload:   sha256 {} ({} files, {} entries, {} B in one zstd stream from {} B)\n",
            hex(&footer.payload_sha256),
            metadata.files.len(),
            self.entries.len(),
            layout.payload_length,
            layout.payload_uncompressed_length
        ));
        for entry in &self.entries {
            out.push_str(&format!(
                "  {:>12} {:>12} {:08x} {}\n",
                entry.offset, entry.length, entry.crc32, entry.entry
            ));
        }
        let registration = metadata.registration.clone().unwrap_or_default();
        out.push_str(&format!(
            "Registers: {} · DisplayName {} · DisplayVersion {}\n",
            metadata.registration_key_name(),
            if registration.display_name.is_empty() {
                &package.name
            } else {
                &registration.display_name
            },
            if registration.display_version.is_empty() {
                &package.version
            } else {
                &registration.display_version
            },
        ));
        if let Some(legacy) = &metadata.legacy {
            out.push_str(&format!(
                "Replaces:  {} registration {}\n",
                legacy.installer_type, legacy.registration_key
            ));
        }
        for option in &metadata.options {
            out.push_str(&format!(
                "Option:    {} (default {})\n",
                option.name,
                if option.default { "on" } else { "off" }
            ));
        }
        for shortcut in &metadata.shortcuts {
            out.push_str(&format!(
                "Shortcut:  {} {} → {}{}\n",
                shortcut_location_name(shortcut.location),
                shortcut.name,
                shortcut.target,
                option_suffix(&shortcut.option)
            ));
        }
        for entry in &metadata.path_entries {
            out.push_str(&format!(
                "Path:      {}{}\n",
                if entry.path.is_empty() {
                    "<install root>"
                } else {
                    &entry.path
                },
                option_suffix(&entry.option)
            ));
        }
        for value in &metadata.registry_values {
            let location = match value.explicit_root() {
                Some(_) => format!("{}\\{}", value.root_name(), value.key),
                None => value.key.clone(),
            };
            out.push_str(&format!(
                "Registry:  {}\\{} {} = {}\n",
                location,
                value.name,
                registry_kind_name(value.kind),
                value.data
            ));
        }
        for dependency in &metadata.dependencies {
            let detect = dependency.detect.clone().unwrap_or_default();
            let hint = match &dependency.acquisition {
                Some(acquisition) if !acquisition.url.is_empty() => acquisition.url.clone(),
                Some(acquisition) => format!(
                    "{} {}",
                    acquisition_source_name(acquisition.source),
                    acquisition.package_identifier
                ),
                None => "none".into(),
            };
            out.push_str(&format!(
                "Requires:  {} {} · detect {} · acquire {hint}\n",
                dependency.id,
                if dependency.minimum_version.is_empty() {
                    "(any version)"
                } else {
                    &dependency.minimum_version
                },
                detector_kind_name(detect.kind),
            ));
        }
        for action in &metadata.actions {
            let program = if action.is_packaged() {
                format!("packaged {} sha256 {}", action.file_name, action.sha256)
            } else {
                action.command.clone()
            };
            out.push_str(&format!(
                "Action:    {} · {} · {} · {} {} · on {} · timeout {} s · on failure {}{}\n",
                action.name,
                action.phase().as_str(),
                action.kind().as_str(),
                program,
                crate::actions::join_for_display(&action.arguments),
                action
                    .operations()
                    .iter()
                    .map(|op| op.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                action.timeout_seconds(),
                action.failure_policy().as_str(),
                match &action.when {
                    Some(when) => format!(" (when {})", when.describe()),
                    None => String::new(),
                }
            ));
        }
        for entry in &metadata.quiescence {
            let describe = |action: &tigersetup_format::metadata::Action| {
                let program = if action.is_packaged() {
                    format!("packaged {} sha256 {}", action.file_name, action.sha256)
                } else {
                    action.command.clone()
                };
                format!(
                    "{} {} {}",
                    action.kind().as_str(),
                    program,
                    crate::actions::join_for_display(&action.arguments)
                )
            };
            out.push_str(&format!(
                "Quiesce:   {} · on {} · stop: {} (not running: {:?}, timeout {} s, on failure {}) · resume: {}{}\n",
                entry.name,
                entry
                    .operations()
                    .iter()
                    .map(|op| op.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                describe(entry.stop()),
                entry.not_running_codes,
                entry.stop().timeout_seconds(),
                entry.stop().failure_policy().as_str(),
                entry
                    .resume
                    .as_ref()
                    .map(describe)
                    .unwrap_or_else(|| "none".into()),
                entry
                    .when
                    .as_ref()
                    .map(|w| format!(" · when {}", w.describe()))
                    .unwrap_or_default()
            ));
        }
        if let Some(launch) = &metadata.launch {
            out.push_str(&format!(
                "Launch:    {} {} · in {} · {}\n",
                launch.executable,
                crate::actions::join_for_display(&launch.arguments),
                match launch.working_directory.as_deref() {
                    None => "its own directory",
                    Some("") => "the install root",
                    Some(directory) => directory,
                },
                if launch.checked {
                    "offered checked"
                } else {
                    "offered unchecked"
                }
            ));
        }
        out.push_str(&format!(
            "Verify:    {}\n",
            if self.is_ok() { "ok" } else { "FAILED" }
        ));
        for problem in &self.verification.problems {
            out.push_str(&format!("  {problem}\n"));
        }
        if !self.engine_block_matches() {
            out.push_str(&format!(
                "  engine_hash_mismatch: engine block is {}\n",
                self.engine_block_sha256
            ));
        }
        out
    }
}

/// Creates `path` — it must not exist — and fills it with `fill`; a file
/// whose filling failed is removed so that nothing partial is left behind.
fn write_export(path: &Path, fill: impl FnOnce(&mut std::fs::File) -> Result<()>) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| {
            BuildError::new(
                "output_unwritable",
                format!("cannot create {}: {err}", path.display()),
            )
        })?;
    let outcome = fill(&mut file).and_then(|()| Ok(file.flush()?));
    drop(file);
    if let Err(err) = outcome {
        let _ = std::fs::remove_file(path);
        return Err(BuildError::new(
            "output_unwritable",
            format!("cannot write {}: {}", path.display(), err.message),
        ));
    }
    Ok(())
}
