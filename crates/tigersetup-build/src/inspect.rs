//! `tiger-setup inspect` and `tiger-setup verify`: decode an installer file
//! without executing it and report what it claims and whether it holds —
//! the metadata, the payload, the engine block against the recorded hashes,
//! and the Windows identity and icon the file presents to Explorer.

use std::path::Path;

use serde_json::{Value, json};
use tigersetup_format::metadata::{
    AcquisitionSource, DetectorKind, Metadata, RegistryKind, Role, ShortcutLocation,
};
use tigersetup_format::{Installer, hex, sha256};

use crate::Result;
use crate::metadata::version_info::{self, VersionInfo};
use crate::resource::{self, Image};

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
        _ => "unspecified",
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
        _ => "unspecified",
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
                });
            }
            if let Some(install) = &dependency.install {
                entry["install"] = json!({
                    "arguments": install.arguments,
                    "success_codes": install.success_codes,
                    "reboot_codes": install.reboot_codes,
                });
            }
            entry
        })
        .collect()
}

pub struct Inspection {
    pub installer: Installer,
    /// SHA-256 of the engine block, computed from the file.
    pub engine_block_sha256: String,
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
    let engine_block_sha256 = installer.engine_sha256_hex()?;
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
        entries,
        verification,
        windows,
        icon,
    })
}

impl Inspection {
    /// The engine-block hash the metadata recorded: `engine_block_sha256`,
    /// or `engine_sha256` for metadata written before the block and the
    /// engine executable were distinguished.
    pub fn expected_engine_block_sha256(&self) -> &str {
        let engine = self.installer.metadata().engine();
        if engine.engine_block_sha256.is_empty() {
            &engine.engine_sha256
        } else {
            &engine.engine_block_sha256
        }
    }

    /// Whether the engine block is the one the metadata recorded.
    pub fn engine_block_matches(&self) -> bool {
        self.engine_block_sha256 == self.expected_engine_block_sha256()
    }

    pub fn is_ok(&self) -> bool {
        self.verification.is_ok() && self.engine_block_matches()
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
        if !self.engine_block_matches() {
            problems.push(json!({ "code": "engine_hash_mismatch", "message": "the engine block does not match the hash recorded in the metadata" }));
        }
        let registration = metadata.registration.clone().unwrap_or_default();
        json!({
            "schema": 1,
            "file": self.installer.path().display().to_string(),
            "role": role_name(metadata.role),
            "format": { "major": footer.format_major, "minor": footer.format_minor },
            "layout": {
                "file_length": layout.file_length,
                "engine_length": layout.engine_length,
                "metadata_offset": layout.metadata_offset,
                "metadata_length": layout.metadata_length,
                "payload_offset": layout.payload_offset,
                "payload_length": layout.payload_length,
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
                "payload_sha256": hex(&footer.payload_sha256),
                "engine": {
                    "tigersetup_version": metadata.engine().tigersetup_version,
                    "engine_sha256": metadata.engine().engine_sha256,
                    "engine_block_sha256": metadata.engine().engine_block_sha256,
                    "engine_block_sha256_actual": self.engine_block_sha256,
                },
            },
            "windows": self.windows_json(),
            "icon": self.icon_json(),
            "files": metadata.files.iter().map(|f| json!({ "path": f.path, "size": f.size, "entry": f.entry })).collect::<Vec<_>>(),
            "directories": metadata.directories.iter().map(|d| d.path.clone()).collect::<Vec<_>>(),
            "options": metadata.options.iter().map(|o| json!({ "name": o.name, "default": o.default })).collect::<Vec<_>>(),
            "shortcuts": metadata.shortcuts.iter().map(|s| json!({
                "location": shortcut_location_name(s.location), "name": s.name, "target": s.target,
                "arguments": s.arguments, "description": s.description, "icon": s.icon,
                "option": s.option, "folder": s.folder
            })).collect::<Vec<_>>(),
            "path_entries": metadata.path_entries.iter().map(|p| json!({ "path": p.path, "option": p.option })).collect::<Vec<_>>(),
            "registry_values": metadata.registry_values.iter().map(|r| json!({
                "key": r.key, "name": r.name, "kind": registry_kind_name(r.kind), "data": r.data
            })).collect::<Vec<_>>(),
            "registration": {
                "key_name": metadata.registration_key_name(),
                "display_name": registration.display_name,
                "display_version": registration.display_version,
                "display_icon": registration.display_icon,
            },
            "legacy": metadata.legacy.as_ref().map(|l| json!({
                "installer_type": l.installer_type, "registration_key": l.registration_key
            })),
            "dependencies": dependencies_json(metadata),
            "entries": self.entries.iter().map(|e| json!({
                "name": e.name, "size": e.size, "compressed_size": e.compressed_size, "method": e.method, "crc32": format!("{:08x}", e.crc32)
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
            "Block:     sha256 {}\n",
            self.expected_engine_block_sha256()
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
            "Layout:    engine {} B | metadata {} B @ {} | payload {} B @ {} | footer @ {}\n",
            layout.engine_length,
            layout.metadata_length,
            layout.metadata_offset,
            layout.payload_length,
            layout.payload_offset,
            layout.footer_offset
        ));
        out.push_str(&format!(
            "Metadata:  sha256 {}\n",
            hex(&footer.metadata_sha256)
        ));
        out.push_str(&format!(
            "Payload:   sha256 {} ({} files, {} entries)\n",
            hex(&footer.payload_sha256),
            metadata.files.len(),
            self.entries.len()
        ));
        for entry in &self.entries {
            out.push_str(&format!(
                "  {:>10} {:>10} {:<8} {:08x} {}\n",
                entry.size, entry.compressed_size, entry.method, entry.crc32, entry.name
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
            out.push_str(&format!(
                "Registry:  {}\\{} {} = {}\n",
                value.key,
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
