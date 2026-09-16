//! Dependency declarations become runtime dependency records: the typed
//! detector, the unattended install switches and the acquisition hint
//! resolved from the WinGet catalog at build time
//! (`TigerSetup-Design.md` §7.9) — or, for an embedded installer, the
//! payload entry the builder carries the file as, with its exact size and
//! SHA-256 (§7.7).
//!
//! The requirement is what the package pins; the hint — version, URL,
//! SHA-256, installer type, scope and switches — is refreshable acquisition
//! metadata that the engine replaces when it is stale or fails. So a hint
//! the catalog cannot supply is a warning, never a build failure: the
//! installer still detects the dependency, and resolves current metadata
//! from the catalog when it needs to acquire one.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tigersetup_catalog::winget::{self, Requirement};
use tigersetup_format::identity::Scope;
use tigersetup_format::metadata::{
    Acquisition, AcquisitionSource, DEPENDENCY_ENTRY_PREFIX, Dependency, DependencyInstall,
    Detector, DetectorKind,
};
use tigersetup_format::{hex, sha256};

use crate::manifest::{DependencyEntry, LoadedManifest, predicate_of};
use crate::{BuildError, Result};

/// Default hint lifetime for a WinGet-sourced acquisition, in days.
pub const DEFAULT_MAX_AGE_DAYS: u32 = 14;

/// The architecture every dependency is resolved for today.
pub const ARCHITECTURE: &str = "x64";

pub struct ResolvedDependencies {
    pub dependencies: Vec<Dependency>,
    /// What the catalog pinned, as `(id, version, url)`, so a build says
    /// exactly which artifact it embedded a hint for.
    pub resolved: Vec<(String, String, String)>,
    /// Ids whose acquisition hint could not be resolved, each with the
    /// reason, so a build says why it shipped without a hint.
    pub unresolved: Vec<(String, String)>,
    /// The installer files the payload carries for embedded dependencies,
    /// as `(entry name, source path)`.
    pub embedded: Vec<(String, PathBuf)>,
}

fn detector_of(entry: &DependencyEntry) -> Detector {
    let d = &entry.detect;
    Detector {
        kind: match d.kind.as_str() {
            "directory-version" => DetectorKind::DirectoryVersion as i32,
            "registry-version" => DetectorKind::RegistryVersion as i32,
            "file-version" => DetectorKind::FileVersion as i32,
            _ => DetectorKind::Registration as i32,
        },
        path: d.path.clone().unwrap_or_default(),
        keys: d.keys.clone(),
        value: d.value.clone().unwrap_or_default(),
        pattern: d.pattern.clone().unwrap_or_default(),
    }
}

/// The payload entry an embedded installer file travels as.
pub fn embedded_entry(file: &str) -> String {
    let name = file
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(file)
        .to_string();
    format!("{DEPENDENCY_ENTRY_PREFIX}{name}")
}

/// The record a declaration yields before any catalog is consulted. An
/// embedded installer is read here, so its size and hash are the bytes'.
pub fn declared(entry: &DependencyEntry, loaded: &LoadedManifest) -> Result<Dependency> {
    let acquisition = match &entry.acquire {
        Some(a) if a.file.is_some() => {
            let file = a.file.as_deref().unwrap_or_default();
            let path = loaded.resolve(file);
            let bytes = std::fs::read(&path).map_err(|err| {
                BuildError::new(
                    "manifest_file_unreadable",
                    format!(
                        "dependency {} installer {}: {err}",
                        entry.id,
                        path.display()
                    ),
                )
            })?;
            if bytes.is_empty() {
                return Err(BuildError::new(
                    "manifest_file_unreadable",
                    format!(
                        "dependency {} installer {} is empty",
                        entry.id,
                        path.display()
                    ),
                ));
            }
            Some(Acquisition {
                source: AcquisitionSource::Embedded as i32,
                entry: embedded_entry(file),
                sha256: hex(&sha256(&bytes)),
                size: bytes.len() as u64,
                architecture: ARCHITECTURE.into(),
                installer_type: if file.to_ascii_lowercase().ends_with(".msi") {
                    "msi".into()
                } else {
                    "exe".into()
                },
                ..Default::default()
            })
        }
        Some(a) if a.url.is_some() => Some(Acquisition {
            source: AcquisitionSource::Url as i32,
            url: a.url.clone().unwrap_or_default(),
            sha256: a.sha256.clone().unwrap_or_default(),
            architecture: ARCHITECTURE.into(),
            ..Default::default()
        }),
        other => Some(Acquisition {
            source: AcquisitionSource::Winget as i32,
            package_identifier: other
                .as_ref()
                .and_then(|a| a.winget.clone())
                .unwrap_or_else(|| entry.id.clone()),
            architecture: ARCHITECTURE.into(),
            max_age_days: other
                .as_ref()
                .and_then(|a| a.max_age_days)
                .unwrap_or(DEFAULT_MAX_AGE_DAYS),
            ..Default::default()
        }),
    };
    Ok(Dependency {
        id: entry.id.clone(),
        display_name: entry.name.clone().unwrap_or_else(|| entry.id.clone()),
        minimum_version: entry.minimum.clone().unwrap_or_default(),
        detect: Some(detector_of(entry)),
        acquisition,
        install: entry.install.as_ref().map(|i| DependencyInstall {
            arguments: i.arguments.clone(),
            success_codes: i.success_codes.clone(),
            reboot_codes: i.reboot_codes.clone(),
            declared: true,
        }),
        elevation_required: entry.elevation.unwrap_or(false),
        when: predicate_of(entry.when.as_ref(), None),
    })
}

/// Whether a record still needs a hint from the catalog: only a WinGet
/// source without one, since a URL source is its own requirement.
fn needs_catalog(dependency: &Dependency) -> bool {
    dependency
        .acquisition
        .as_ref()
        .is_some_and(|a| a.source == AcquisitionSource::Winget as i32 && a.url.is_empty())
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Where the catalog index is written while a build resolves hints.
fn work_dir() -> PathBuf {
    std::env::temp_dir().join(format!("tigersetup-catalog-{}", std::process::id()))
}

/// Fills the hint of one WinGet-sourced dependency from the catalog. The
/// declaration always wins: switches and codes the package author wrote, and
/// an explicit `elevation`, are never overwritten by the catalog.
fn resolve_one(
    dependency: &mut Dependency,
    scope: Scope,
    entry: &DependencyEntry,
    work_dir: &std::path::Path,
) -> std::result::Result<String, String> {
    let acquisition = dependency.acquisition.clone().unwrap_or_default();
    let requirement = Requirement {
        minimum_version: &dependency.minimum_version,
        architecture: ARCHITECTURE,
        scope,
    };
    let resolved = winget::resolve(
        &acquisition.package_identifier,
        &requirement,
        work_dir,
        None,
    )
    .map_err(|err| err.to_string())?;
    let installer = resolved.installer;
    dependency.acquisition = Some(Acquisition {
        version: resolved.version.clone(),
        url: installer.url.clone(),
        sha256: installer.sha256.clone(),
        resolved_at: now_unix(),
        scope: installer.scope.clone(),
        installer_type: installer.installer_type.clone(),
        ..acquisition
    });
    if dependency.install.is_none() {
        dependency.install = Some(DependencyInstall {
            arguments: installer.arguments.clone(),
            success_codes: installer.success_codes.clone(),
            reboot_codes: installer.reboot_codes.clone(),
            declared: false,
        });
    }
    if entry.elevation.is_none() {
        dependency.elevation_required = installer.elevation_required;
    }
    Ok(resolved.version)
}

/// Resolves every declared dependency. `offline` skips the catalog, so the
/// build makes no network request at all.
pub fn resolve(loaded: &LoadedManifest, offline: bool) -> Result<ResolvedDependencies> {
    let scope = loaded
        .manifest
        .scopes()
        .first()
        .copied()
        .unwrap_or(Scope::User);
    let mut dependencies = Vec::new();
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    let mut embedded = Vec::new();
    let work_dir = work_dir();
    for entry in &loaded.manifest.dependencies {
        let mut dependency = declared(entry, loaded)?;
        if let Some(file) = entry.acquire.as_ref().and_then(|a| a.file.as_deref()) {
            let entry_name = embedded_entry(file);
            if embedded
                .iter()
                .any(|(name, _): &(String, PathBuf)| name.eq_ignore_ascii_case(&entry_name))
            {
                return Err(BuildError::new(
                    "manifest_invalid",
                    format!(
                        "dependency {}: two embedded installers share the file name {file:?}",
                        entry.id
                    ),
                ));
            }
            embedded.push((entry_name, loaded.resolve(file)));
        }
        if needs_catalog(&dependency) {
            match offline {
                true => unresolved.push((
                    dependency.id.clone(),
                    "the build was asked not to contact the catalog".to_string(),
                )),
                false => match resolve_one(&mut dependency, scope, entry, &work_dir) {
                    Ok(version) => {
                        let url = dependency
                            .acquisition
                            .as_ref()
                            .map(|a| a.url.clone())
                            .unwrap_or_default();
                        resolved.push((dependency.id.clone(), version, url));
                    }
                    Err(reason) => unresolved.push((dependency.id.clone(), reason)),
                },
            }
        }
        dependencies.push(dependency);
    }
    let _ = std::fs::remove_dir_all(&work_dir);
    Ok(ResolvedDependencies {
        dependencies,
        resolved,
        unresolved,
        embedded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    fn loaded(toml: &str) -> LoadedManifest {
        let manifest: Manifest = toml::from_str(toml).expect("the manifest parses");
        LoadedManifest {
            path: PathBuf::from("TigerSetup.toml"),
            directory: PathBuf::from("."),
            manifest,
        }
    }

    const WITH_DEPENDENCIES: &str = r#"
[package]
id = "Vendor.Product"
name = "Product"
version = "1.0.0"
publisher = "Vendor"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"

[[dependencies]]
id = "Vendor.Runtime"
name = "Vendor Runtime"
minimum = "10.0"
detect = { kind = "directory-version", path = "%PROGRAMFILES%\\Vendor\\Runtime", pattern = "10\\..*" }

[[dependencies]]
id = "Vendor.Pinned"
detect = { kind = "file-version", path = "%PROGRAMFILES%\\Vendor\\Pinned\\pinned.dll" }
acquire = { url = "https://vendor.invalid/pinned.exe", sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" }
install = { arguments = ["/S"], success_codes = [1], reboot_codes = [3010] }
"#;

    #[test]
    fn an_offline_build_leaves_winget_hints_unresolved_and_keeps_url_ones() {
        let loaded = loaded(WITH_DEPENDENCIES);
        let resolved = resolve(&loaded, true).unwrap();
        let unresolved: Vec<&str> = resolved
            .unresolved
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(unresolved, vec!["Vendor.Runtime"]);
        assert!(
            !resolved.unresolved[0].1.is_empty(),
            "an unresolved hint says why"
        );
        assert!(resolved.resolved.is_empty());
        assert_eq!(resolved.dependencies.len(), 2);

        let runtime = &resolved.dependencies[0];
        assert_eq!(runtime.display_name, "Vendor Runtime");
        assert_eq!(runtime.minimum_version, "10.0");
        assert_eq!(
            runtime.detect.as_ref().unwrap().kind,
            DetectorKind::DirectoryVersion as i32
        );
        let acquisition = runtime.acquisition.as_ref().unwrap();
        assert_eq!(acquisition.source, AcquisitionSource::Winget as i32);
        assert_eq!(acquisition.package_identifier, "Vendor.Runtime");
        assert_eq!(acquisition.max_age_days, DEFAULT_MAX_AGE_DAYS);
        assert!(acquisition.url.is_empty());
        assert_eq!(acquisition.resolved_at, 0);
        assert!(runtime.install.is_none());

        let pinned = &resolved.dependencies[1];
        assert_eq!(pinned.display_name, "Vendor.Pinned");
        let acquisition = pinned.acquisition.as_ref().unwrap();
        assert_eq!(acquisition.source, AcquisitionSource::Url as i32);
        assert_eq!(acquisition.url, "https://vendor.invalid/pinned.exe");
        let install = pinned.install.as_ref().unwrap();
        assert!(install.declared, "an author's switches survive a refresh");
        assert_eq!(install.arguments, vec!["/S"]);
        assert_eq!(install.reboot_codes, vec![3010]);
    }
}
