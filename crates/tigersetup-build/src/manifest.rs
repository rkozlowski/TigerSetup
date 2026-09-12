//! The developer-facing `TigerSetup.toml`: typed sections, validated, never
//! shipped. Every declaration is typed; a value the builder can derive is
//! derived once (`[metadata]`) and validated, never typed twice.
//!
//! ```toml
//! [package]
//! id = "ItTiger.TigerMarkView"          # stable identity: ARP key, WinGet id
//! name = "TigerMarkView"
//! publisher = "IT Tiger"
//! license = "MIT"
//! license_file = "publish/Docs/LICENSE.txt"   # shown by the wizard
//! icon = "assets/TMV.ico"
//! # version, description and copyright come from [metadata] or are typed here
//!
//! [metadata]
//! source = "msbuild"                    # "static" (default) | "msbuild" | "exe"
//! project = "../src/TigerMarkView/TigerMarkView.csproj"
//! executable = "publish/TigerMarkView.exe"   # validated against the source
//!
//! [install]
//! scopes = ["user", "machine"]          # first entry is the default
//! existing_scope = "preserve"           # | "allow-parallel" | "error"
//!
//! [installer]
//! icon = "branding"                     # the Setup.exe's own icon: omitted | "tigersetup" | "branding" | an .ico path
//!
//! [[files]]
//! source = "publish/**"
//! exclude = ["*.pdb", "*.xml"]
//!
//! [[options]]
//! name = "path"
//! default = true
//!
//! [[shortcuts]]
//! location = "start-menu"
//! target = "TigerMarkView.exe"
//!
//! [[path]]
//! entry = "."
//! option = "path"
//!
//! [registration]
//! display_icon = "TigerMarkView.exe"
//!
//! [legacy]
//! installer_type = "inno"
//! registration_key = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"
//!
//! [[dependencies]]
//! id = "Microsoft.DotNet.DesktopRuntime.10"
//! minimum = "10.0"
//! detect = { kind = "directory-version", path = "%PROGRAMFILES%\\dotnet\\shared\\Microsoft.WindowsDesktop.App", pattern = "10.*" }
//!
//! [winget]
//! moniker = "tiger-markview"
//! commands = ["tiger-mark"]
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tigersetup_format::identity::{self, Scope};
use tigersetup_format::metadata::{
    ExistingScopePolicy, is_sha256_hex, validate_dotted_version, validate_option_name,
    validate_registration_key_name, validate_registry_key, validate_relative_path,
};

use crate::{BuildError, Result};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub package: PackageSection,
    #[serde(default)]
    pub metadata: MetadataSection,
    #[serde(default)]
    pub install: InstallSection,
    #[serde(default)]
    pub installer: InstallerSection,
    #[serde(default)]
    pub files: Vec<FilesEntry>,
    #[serde(default)]
    pub options: Vec<OptionEntry>,
    #[serde(default)]
    pub shortcuts: Vec<ShortcutEntry>,
    #[serde(default)]
    pub path: Vec<PathEntryDecl>,
    #[serde(default)]
    pub registry: Vec<RegistryEntry>,
    #[serde(default)]
    pub registration: RegistrationSection,
    pub legacy: Option<LegacySection>,
    #[serde(default)]
    pub dependencies: Vec<DependencyEntry>,
    #[serde(default)]
    pub winget: WingetSection,
}

/// Identity and product strings. `version`, `description` and `copyright`
/// may come from `[metadata]` instead of being typed here; a value typed
/// here wins over a provider's value and the provenance report shows both.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSection {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub publisher: String,
    pub description: Option<String>,
    pub copyright: Option<String>,
    /// Licence identity, an SPDX identifier where one applies.
    pub license: Option<String>,
    /// The licence text the wizard shows, relative to the manifest.
    pub license_file: Option<String>,
    /// An `.ico` the wizard shows for the product, relative to the manifest.
    pub icon: Option<String>,
    pub website: Option<String>,
    pub support: Option<String>,
    pub help: Option<String>,
}

/// Where the product metadata comes from and what it is validated against
/// (`TigerSetup-Design.md` §9).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataSection {
    /// `static` (everything typed in `[package]`), `msbuild` (evaluated
    /// MSBuild properties of `project`) or `exe` (the `VERSIONINFO` of
    /// `executable`). Defaults to `static`.
    #[serde(default)]
    pub source: MetadataSource,
    /// The MSBuild project, relative to the manifest.
    pub project: Option<String>,
    /// The built executable, relative to the manifest: the source for `exe`,
    /// the validation target for `msbuild` and `static` when given.
    pub executable: Option<String>,
    /// Global MSBuild properties passed to the evaluation (`-p:Name=Value`).
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MetadataSource {
    #[default]
    Static,
    Msbuild,
    Exe,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallSection {
    /// Supported scopes, default first. Defaults to `["user"]`.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// What a run does when the product is already installed in the other
    /// scope: `preserve` (the default) continues with that installation and
    /// refuses an explicit request for the other scope, `allow-parallel`
    /// lets an explicit request create a second installation, `error`
    /// refuses every run whose scope is not the installed one.
    pub existing_scope: Option<String>,
    /// Install root for user scope; defaults to `%LOCALAPPDATA%\Programs\<name>`.
    pub user_root: Option<String>,
    /// Install root for machine scope; defaults to `%PROGRAMFILES%\<name>`.
    pub machine_root: Option<String>,
    /// Lowest supported Windows build; defaults to the engine's baseline.
    pub minimum_build: Option<u32>,
    /// `x64` is the only value today.
    pub architecture: Option<String>,
}

/// The generated `Setup.exe` itself, as distinct from what it installs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallerSection {
    /// The executable's icon — what Explorer shows for the installer file.
    /// Omitted: the branding icon (`package.icon`) when one is declared,
    /// otherwise TigerSetup's. `"tigersetup"`: TigerSetup's icon even when a
    /// branding icon is declared. `"branding"`: the branding icon, and an
    /// error when none is declared. Anything else: a manifest-relative
    /// `.ico` path that overrides both.
    pub icon: Option<String>,
}

/// What `installer.icon` resolves to: which `.ico` becomes the executable's
/// icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallerIcon {
    /// TigerSetup's own icon.
    TigerSetup,
    /// The manifest-relative path of an `.ico`: the branding icon or an
    /// explicit override.
    File(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesEntry {
    /// A glob relative to the manifest's directory. The part before the first
    /// wildcard is the base the install-relative paths are taken from.
    pub source: String,
    /// Globs matched against the install-relative path; `*` spans separators.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// An on/off installer option the user can set on the command line or in
/// the wizard.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionEntry {
    pub name: String,
    #[serde(default)]
    pub default: bool,
    /// `path`, `desktop-shortcut` (labelled by the wizard in its own
    /// language) or `custom` (the default; labelled by `label`).
    pub kind: Option<String>,
    /// Wizard labels per BCP 47 tag for a custom option; `en-US` is required.
    #[serde(default)]
    pub label: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutEntry {
    /// `start-menu` or `desktop`.
    pub location: String,
    /// Link name without `.lnk`; defaults to the package name.
    pub name: Option<String>,
    /// Install-relative target.
    pub target: String,
    #[serde(default)]
    pub arguments: String,
    /// Defaults to the package description.
    pub description: Option<String>,
    /// Install-relative icon source; defaults to the target.
    pub icon: Option<String>,
    /// The option that enables the shortcut; absent means always.
    pub option: Option<String>,
    /// A subfolder under the location; absent places the link directly in it.
    pub folder: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathEntryDecl {
    /// Install-relative directory; `.` is the install root.
    pub entry: String,
    pub option: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    /// Key under the scope's `Software` root.
    pub key: String,
    pub name: String,
    /// `string`, `expand-string` or `dword`.
    pub kind: String,
    /// `%INSTALLROOT%` and `%VERSION%` expand at install time.
    pub data: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationSection {
    /// Uninstall key name; defaults to the package id.
    pub key_name: Option<String>,
    pub display_name: Option<String>,
    pub display_version: Option<String>,
    /// Install-relative icon source.
    pub display_icon: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacySection {
    pub installer_type: String,
    pub registration_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyEntry {
    /// The requirement identity; the WinGet package identifier unless
    /// `acquire` names a URL.
    pub id: String,
    /// Shown to the user; defaults to the id.
    pub name: Option<String>,
    /// Lowest acceptable version; same major, not lower. Absent means any.
    pub minimum: Option<String>,
    pub detect: DetectorDecl,
    /// Absent means "the WinGet catalog entry for `id`".
    pub acquire: Option<AcquireDecl>,
    /// Absent means "the switches the WinGet manifest declares".
    pub install: Option<DependencyInstallDecl>,
    /// Absent means "as the WinGet manifest's scope implies".
    pub elevation: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectorDecl {
    /// `directory-version`, `registry-version`, `file-version` or
    /// `registration`.
    pub kind: String,
    pub path: Option<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    pub value: Option<String>,
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcquireDecl {
    /// A WinGet package identifier other than `id`.
    pub winget: Option<String>,
    /// A fixed URL, with `sha256`; never refreshed.
    pub url: Option<String>,
    pub sha256: Option<String>,
    /// Hint lifetime in days for the WinGet source (default 14).
    pub max_age_days: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyInstallDecl {
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub success_codes: Vec<i32>,
    #[serde(default)]
    pub reboot_codes: Vec<i32>,
}

/// What `tiger-setup winget prepare` needs beyond the package: the values
/// the community manifest carries and the installer does not.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WingetSection {
    /// Defaults to the package id.
    pub identifier: Option<String>,
    pub moniker: Option<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub package_url: Option<String>,
    pub publisher_url: Option<String>,
    pub publisher_support_url: Option<String>,
    pub license_url: Option<String>,
    pub release_notes_url: Option<String>,
    pub documentation_url: Option<String>,
    pub short_description: Option<String>,
    pub description: Option<String>,
    /// `MinimumOSVersion`; defaults to the install baseline.
    pub minimum_os_version: Option<String>,
}

/// A manifest with its location, after validation.
#[derive(Debug, Clone)]
pub struct LoadedManifest {
    pub path: PathBuf,
    pub directory: PathBuf,
    pub manifest: Manifest,
}

impl LoadedManifest {
    pub fn load(path: &Path) -> Result<LoadedManifest> {
        let text = std::fs::read_to_string(path).map_err(|err| {
            BuildError::new(
                "manifest_unreadable",
                format!("cannot read {}: {err}", path.display()),
            )
        })?;
        let manifest: Manifest = toml::from_str(&text).map_err(|err| {
            BuildError::new("manifest_invalid", format!("{}: {err}", path.display()))
        })?;
        manifest.validate()?;
        // `absolute`, not `canonicalize`: the verbatim `\\?\` prefix the
        // latter produces on Windows is not understood by glob matching.
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(LoadedManifest {
            path,
            directory,
            manifest,
        })
    }

    /// A manifest-relative path resolved against the manifest's directory.
    /// A manifest may write either separator; the result uses the platform's
    /// throughout, because it is shown to the developer in provenance and in
    /// error messages.
    pub fn resolve(&self, relative: &str) -> PathBuf {
        self.directory.join(relative.replace('/', "\\"))
    }
}

fn relative_to_manifest(what: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(BuildError::new(
            "manifest_invalid",
            format!("{what} is empty"),
        ));
    }
    if Path::new(value).is_absolute() {
        return Err(BuildError::new(
            "manifest_invalid",
            format!("{what} {value:?} must be relative to the manifest"),
        ));
    }
    Ok(())
}

/// A `.`-or-install-relative path as the manifest writes it, normalised to
/// the metadata form (forward slashes, empty for the root).
pub fn install_relative(value: &str) -> Result<String> {
    let normalised = value.trim().replace('\\', "/");
    let normalised = normalised
        .strip_prefix("./")
        .map(str::to_string)
        .unwrap_or(normalised);
    if normalised == "." || normalised.is_empty() {
        return Ok(String::new());
    }
    validate_relative_path(&normalised)?;
    Ok(normalised)
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        let invalid = |message: String| BuildError::new("manifest_invalid", message);
        identity::validate_product_id(&self.package.id)?;
        identity::validate_name(&self.package.name)?;
        if let Some(version) = &self.package.version {
            identity::validate_version(version)?;
        }
        if self.package.publisher.trim().is_empty() {
            return Err(invalid("package.publisher is empty".into()));
        }
        for (what, value) in [
            ("package.license_file", &self.package.license_file),
            ("package.icon", &self.package.icon),
            ("metadata.project", &self.metadata.project),
            ("metadata.executable", &self.metadata.executable),
        ] {
            if let Some(value) = value {
                relative_to_manifest(what, value)?;
            }
        }
        match self.installer.icon.as_deref() {
            None | Some("tigersetup") => {}
            Some("branding") => {
                if self.package.icon.is_none() {
                    return Err(invalid(
                        "installer.icon is \"branding\" but package.icon declares no icon".into(),
                    ));
                }
            }
            Some(path) => relative_to_manifest("installer.icon", path)?,
        }
        match self.metadata.source {
            MetadataSource::Static => {
                if self.package.version.is_none() {
                    return Err(invalid(
                        "package.version is required when metadata.source is static".into(),
                    ));
                }
            }
            MetadataSource::Msbuild => {
                if self.metadata.project.is_none() {
                    return Err(invalid(
                        "metadata.project is required when metadata.source is msbuild".into(),
                    ));
                }
            }
            MetadataSource::Exe => {
                if self.metadata.executable.is_none() {
                    return Err(invalid(
                        "metadata.executable is required when metadata.source is exe".into(),
                    ));
                }
            }
        }
        for scope in &self.install.scopes {
            Scope::parse(scope).ok_or_else(|| {
                invalid(format!("install.scopes contains unknown scope {scope:?}"))
            })?;
        }
        if let Some(policy) = &self.install.existing_scope
            && ExistingScopePolicy::parse(policy).is_none()
        {
            return Err(invalid(format!(
                "install.existing_scope {policy:?} is not preserve, allow-parallel or error"
            )));
        }
        if let Some(architecture) = &self.install.architecture
            && architecture != "x64"
        {
            return Err(invalid(format!(
                "install.architecture {architecture:?} is not supported; only x64 is"
            )));
        }
        if self.files.is_empty() {
            return Err(invalid("at least one [[files]] entry is required".into()));
        }
        for entry in &self.files {
            relative_to_manifest("files.source", &entry.source)?;
        }
        let mut option_names = std::collections::HashSet::new();
        for option in &self.options {
            validate_option_name(&option.name)?;
            if !option_names.insert(option.name.to_ascii_lowercase()) {
                return Err(invalid(format!("option {} is declared twice", option.name)));
            }
            match option.kind.as_deref().unwrap_or("custom") {
                "path" | "desktop-shortcut" => {}
                "custom" => {
                    if option
                        .label
                        .get("en-US")
                        .is_none_or(|l| l.trim().is_empty())
                    {
                        return Err(invalid(format!(
                            "option {} is custom and needs label.\"en-US\"",
                            option.name
                        )));
                    }
                }
                other => {
                    return Err(invalid(format!(
                        "option {} has kind {other:?}; expected path, desktop-shortcut or custom",
                        option.name
                    )));
                }
            }
        }
        let option_known = |what: &str, option: &Option<String>| -> Result<()> {
            match option {
                Some(name) if !option_names.contains(&name.to_ascii_lowercase()) => Err(invalid(
                    format!("{what} names option {name:?}, which is not declared"),
                )),
                _ => Ok(()),
            }
        };
        for shortcut in &self.shortcuts {
            if !matches!(shortcut.location.as_str(), "start-menu" | "desktop") {
                return Err(invalid(format!(
                    "shortcut location {:?} must be start-menu or desktop",
                    shortcut.location
                )));
            }
            if let Some(name) = &shortcut.name {
                identity::validate_name(name)?;
            }
            validate_relative_path(&install_relative(&shortcut.target)?)?;
            if let Some(icon) = &shortcut.icon {
                validate_relative_path(&install_relative(icon)?)?;
            }
            if let Some(folder) = &shortcut.folder {
                validate_relative_path(&install_relative(folder)?)?;
            }
            option_known("a shortcut", &shortcut.option)?;
        }
        for entry in &self.path {
            install_relative(&entry.entry)?;
            option_known("a path entry", &entry.option)?;
        }
        for value in &self.registry {
            validate_registry_key(&value.key)?;
            if !matches!(value.kind.as_str(), "string" | "expand-string" | "dword") {
                return Err(invalid(format!(
                    "registry kind {:?} must be string, expand-string or dword",
                    value.kind
                )));
            }
            if value.kind == "dword" && value.data.parse::<u32>().is_err() {
                return Err(invalid(format!(
                    "registry value {}\\{} is a dword but {:?} is not a number",
                    value.key, value.name, value.data
                )));
            }
        }
        if let Some(key_name) = &self.registration.key_name {
            validate_registration_key_name(key_name)?;
        }
        if let Some(icon) = &self.registration.display_icon {
            validate_relative_path(&install_relative(icon)?)?;
        }
        if let Some(legacy) = &self.legacy {
            if legacy.installer_type != "inno" {
                return Err(invalid(format!(
                    "legacy.installer_type {:?} is not supported; inno is",
                    legacy.installer_type
                )));
            }
            validate_registration_key_name(&legacy.registration_key)?;
        }
        let mut dependency_ids = std::collections::HashSet::new();
        for dependency in &self.dependencies {
            if dependency.id.trim().is_empty() {
                return Err(invalid("a dependency has no id".into()));
            }
            if !dependency_ids.insert(dependency.id.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "dependency {} is declared twice",
                    dependency.id
                )));
            }
            if let Some(minimum) = &dependency.minimum {
                validate_dotted_version(minimum)?;
            }
            let detect = &dependency.detect;
            match detect.kind.as_str() {
                "directory-version" | "file-version" => {
                    if detect.path.as_deref().unwrap_or("").is_empty() {
                        return Err(invalid(format!(
                            "dependency {}: detect.path is required for {}",
                            dependency.id, detect.kind
                        )));
                    }
                }
                "registry-version" => {
                    if detect.keys.is_empty() || detect.value.as_deref().unwrap_or("").is_empty() {
                        return Err(invalid(format!(
                            "dependency {}: detect.keys and detect.value are required for registry-version",
                            dependency.id
                        )));
                    }
                }
                "registration" => {
                    if detect.pattern.as_deref().unwrap_or("").is_empty() {
                        return Err(invalid(format!(
                            "dependency {}: detect.pattern is required for registration",
                            dependency.id
                        )));
                    }
                }
                other => {
                    return Err(invalid(format!(
                        "dependency {}: detect.kind {other:?} is not directory-version, registry-version, file-version or registration",
                        dependency.id
                    )));
                }
            }
            if let Some(acquire) = &dependency.acquire {
                match (&acquire.winget, &acquire.url) {
                    (Some(_), Some(_)) => {
                        return Err(invalid(format!(
                            "dependency {}: acquire names both winget and url",
                            dependency.id
                        )));
                    }
                    (None, Some(_)) => {
                        if !acquire.sha256.as_deref().is_some_and(is_sha256_hex) {
                            return Err(invalid(format!(
                                "dependency {}: acquire.url needs acquire.sha256 (64 lower-case hex digits)",
                                dependency.id
                            )));
                        }
                        if dependency.install.is_none() {
                            return Err(invalid(format!(
                                "dependency {}: a URL acquisition needs an [dependencies.install] table",
                                dependency.id
                            )));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// The scopes in declaration order, defaulting to user scope.
    pub fn scopes(&self) -> Vec<Scope> {
        if self.install.scopes.is_empty() {
            vec![Scope::User]
        } else {
            self.install
                .scopes
                .iter()
                .filter_map(|s| Scope::parse(s))
                .collect()
        }
    }

    /// The policy for an installation in the other scope; preserved unless
    /// the manifest says otherwise.
    pub fn existing_scope_policy(&self) -> ExistingScopePolicy {
        self.install
            .existing_scope
            .as_deref()
            .and_then(ExistingScopePolicy::parse)
            .unwrap_or(ExistingScopePolicy::Preserve)
    }

    pub fn install_root_template(&self, scope: Scope) -> String {
        let declared = match scope {
            Scope::User => self.install.user_root.as_deref(),
            Scope::Machine => self.install.machine_root.as_deref(),
        };
        declared
            .map(str::to_string)
            .unwrap_or_else(|| identity::default_install_root_template(scope, &self.package.name))
    }

    /// Whether a scope installs into TigerSetup's default root rather than
    /// one the manifest pins. A pinned root is the package author's choice
    /// of location and is not open to an override.
    pub fn install_root_is_default(&self, scope: Scope) -> bool {
        match scope {
            Scope::User => self.install.user_root.is_none(),
            Scope::Machine => self.install.machine_root.is_none(),
        }
    }

    /// The icon the generated `Setup.exe` carries as its own, by the
    /// `installer.icon` policy: the branding icon by default when there is
    /// one, TigerSetup's otherwise, or what the manifest names explicitly.
    pub fn installer_icon(&self) -> InstallerIcon {
        match (self.installer.icon.as_deref(), &self.package.icon) {
            (Some("tigersetup"), _) => InstallerIcon::TigerSetup,
            (Some("branding"), Some(branding)) => InstallerIcon::File(branding.clone()),
            (Some("branding"), None) => InstallerIcon::TigerSetup,
            (Some(path), _) => InstallerIcon::File(path.to_string()),
            (None, Some(branding)) => InstallerIcon::File(branding.clone()),
            (None, None) => InstallerIcon::TigerSetup,
        }
    }

    /// The WinGet package identifier: declared, or the package id.
    pub fn winget_identifier(&self) -> &str {
        self.winget
            .identifier
            .as_deref()
            .unwrap_or(&self.package.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[package]
id = "IT-Tiger.TigerSetupTestApp"
name = "TigerSetupTestApp"
version = "1.0.0"
publisher = "IT Tiger"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"
exclude = ["*.pdb"]
"#;

    const FULL: &str = r#"
[package]
id = "ItTiger.TigerMarkView"
name = "TigerMarkView"
publisher = "IT Tiger"
license = "MIT"
license_file = "publish/Docs/LICENSE.txt"

[metadata]
source = "msbuild"
project = "../src/TigerMarkView/TigerMarkView.csproj"
executable = "publish/TigerMarkView.exe"

[install]
scopes = ["user", "machine"]

[[files]]
source = "publish/**"
exclude = ["*.pdb", "*.xml"]

[[options]]
name = "path"
kind = "path"
default = true

[[options]]
name = "desktop-shortcut"
kind = "desktop-shortcut"

[[options]]
name = "samples"
label = { "en-US" = "Install the sample documents", "pl-PL" = "Zainstaluj przykładowe dokumenty" }

[[shortcuts]]
location = "start-menu"
target = "TigerMarkView.exe"

[[shortcuts]]
location = "desktop"
target = "TigerMarkView.exe"
option = "desktop-shortcut"

[[path]]
entry = "."
option = "path"

[[registry]]
key = "IT Tiger\\TigerMarkView"
name = "InstallRoot"
kind = "expand-string"
data = "%INSTALLROOT%"

[registration]
display_icon = "TigerMarkView.exe"

[legacy]
installer_type = "inno"
registration_key = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"

[[dependencies]]
id = "Microsoft.DotNet.DesktopRuntime.10"
name = ".NET Desktop Runtime 10"
minimum = "10.0"
detect = { kind = "directory-version", path = "%PROGRAMFILES%\\dotnet\\shared\\Microsoft.WindowsDesktop.App", pattern = "10.*" }

[[dependencies]]
id = "Microsoft.EdgeWebView2Runtime"
detect = { kind = "registry-version", keys = ["HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"], value = "pv" }

[[dependencies]]
id = "Vendor.Custom"
detect = { kind = "file-version", path = "%PROGRAMFILES%\\Vendor\\vendor.dll" }
acquire = { url = "https://example.invalid/vendor.exe", sha256 = "0000000000000000000000000000000000000000000000000000000000000000" }
install = { arguments = ["/S"], success_codes = [3010], reboot_codes = [3010] }
elevation = true

[winget]
moniker = "tiger-markview"
commands = ["tiger-mark"]
"#;

    #[test]
    fn sample_manifest_parses_with_defaults() {
        let manifest: Manifest = toml::from_str(SAMPLE).unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.scopes(), vec![Scope::User]);
        assert_eq!(
            manifest.install_root_template(Scope::User),
            "%LOCALAPPDATA%\\Programs\\TigerSetupTestApp"
        );
        assert_eq!(
            manifest.install_root_template(Scope::Machine),
            "%PROGRAMFILES%\\TigerSetupTestApp"
        );
        assert_eq!(manifest.winget_identifier(), "IT-Tiger.TigerSetupTestApp");
        assert_eq!(
            manifest.existing_scope_policy(),
            ExistingScopePolicy::Preserve,
            "an installation in the other scope is preserved unless the manifest says otherwise"
        );
    }

    #[test]
    fn the_existing_scope_policy_is_one_of_three_words() {
        for (word, policy) in [
            ("preserve", ExistingScopePolicy::Preserve),
            ("allow-parallel", ExistingScopePolicy::AllowParallel),
            ("error", ExistingScopePolicy::Error),
        ] {
            let text = SAMPLE.replace(
                "scopes = [\"user\"]",
                &format!(
                    "scopes = [\"user\", \"machine\"]
existing_scope = \"{word}\""
                ),
            );
            let manifest: Manifest = toml::from_str(&text).unwrap();
            manifest.validate().unwrap();
            assert_eq!(manifest.existing_scope_policy(), policy, "{word}");
            assert_eq!(policy.as_str(), word);
        }
        let text = SAMPLE.replace("scopes = [\"user\"]", "existing_scope = \"migrate\"");
        let manifest: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(manifest.validate().unwrap_err().code, "manifest_invalid");
    }

    #[test]
    fn full_manifest_parses_and_validates() {
        let manifest: Manifest = toml::from_str(FULL).unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.metadata.source, MetadataSource::Msbuild);
        assert_eq!(manifest.options.len(), 3);
        assert_eq!(manifest.dependencies.len(), 3);
        let custom_without_label = FULL.replace(
            "label = { \"en-US\" = \"Install the sample documents\", \"pl-PL\" = \"Zainstaluj przykładowe dokumenty\" }",
            "",
        );
        let manifest: Manifest = toml::from_str(&custom_without_label).unwrap();
        assert!(
            manifest.validate().is_err(),
            "a custom option needs an English label"
        );
        assert_eq!(install_relative(".").unwrap(), "");
        assert_eq!(install_relative("./cli").unwrap(), "cli");
        assert_eq!(install_relative("bin\\x").unwrap(), "bin/x");
    }

    #[test]
    fn the_installer_icon_follows_the_declared_policy() {
        let icon_of = |text: &str| -> InstallerIcon {
            let manifest: Manifest = toml::from_str(text).unwrap();
            manifest.validate().unwrap();
            manifest.installer_icon()
        };
        let branded = SAMPLE.replace(
            "publisher = \"IT Tiger\"",
            "publisher = \"IT Tiger\"
icon = \"assets/test-app.ico\"",
        );
        // Omitted: the branding icon when declared, TigerSetup's otherwise.
        assert_eq!(icon_of(SAMPLE), InstallerIcon::TigerSetup);
        assert_eq!(
            icon_of(&branded),
            InstallerIcon::File("assets/test-app.ico".into())
        );
        // "tigersetup": TigerSetup's even beside a branding icon.
        assert_eq!(
            icon_of(&format!(
                "{branded}
[installer]
icon = \"tigersetup\"
"
            )),
            InstallerIcon::TigerSetup
        );
        assert_eq!(
            icon_of(&format!(
                "{SAMPLE}
[installer]
icon = \"tigersetup\"
"
            )),
            InstallerIcon::TigerSetup
        );
        // "branding": the branding icon, never a silent fallback.
        assert_eq!(
            icon_of(&format!(
                "{branded}
[installer]
icon = \"branding\"
"
            )),
            InstallerIcon::File("assets/test-app.ico".into())
        );
        let unbranded: Manifest = toml::from_str(&format!(
            "{SAMPLE}
[installer]
icon = \"branding\"
"
        ))
        .unwrap();
        let err = unbranded.validate().unwrap_err();
        assert_eq!(err.code, "manifest_invalid");
        assert!(err.message.contains("installer.icon"), "{}", err.message);
        // A path overrides both.
        assert_eq!(
            icon_of(&format!(
                "{branded}
[installer]
icon = \"assets/setup.ico\"
"
            )),
            InstallerIcon::File("assets/setup.ico".into())
        );
        assert_eq!(
            icon_of(&format!(
                "{SAMPLE}
[installer]
icon = \"assets/setup.ico\"
"
            )),
            InstallerIcon::File("assets/setup.ico".into())
        );
        let absolute: Manifest = toml::from_str(&format!(
            "{SAMPLE}
[installer]
icon = \"C:/setup.ico\"
"
        ))
        .unwrap();
        assert_eq!(absolute.validate().unwrap_err().code, "manifest_invalid");
        assert!(
            toml::from_str::<Manifest>(&format!(
                "{SAMPLE}
[installer]
logo = \"x\"
"
            ))
            .is_err(),
            "unknown installer keys are refused"
        );
    }

    #[test]
    fn invalid_manifests_are_refused() {
        let no_files = SAMPLE.split("[[files]]").next().unwrap();
        let manifest: Manifest = toml::from_str(no_files).unwrap();
        assert_eq!(manifest.validate().unwrap_err().code, "manifest_invalid");
        let bad_version = SAMPLE.replace("1.0.0", "1.0");
        let manifest: Manifest = toml::from_str(&bad_version).unwrap();
        assert_eq!(
            manifest.validate().unwrap_err().code,
            "package_version_invalid"
        );
        let unknown_key = SAMPLE.replace("publisher", "vendor");
        assert!(toml::from_str::<Manifest>(&unknown_key).is_err());
        let no_version = SAMPLE.replace("version = \"1.0.0\"\n", "");
        let manifest: Manifest = toml::from_str(&no_version).unwrap();
        assert!(
            manifest.validate().is_err(),
            "static metadata needs a version"
        );
        let undeclared_option = FULL.replace("option = \"path\"", "option = \"nope\"");
        let manifest: Manifest = toml::from_str(&undeclared_option).unwrap();
        assert!(manifest.validate().is_err());
        let bad_legacy = FULL.replace("installer_type = \"inno\"", "installer_type = \"nsis\"");
        let manifest: Manifest = toml::from_str(&bad_legacy).unwrap();
        assert!(manifest.validate().is_err());
        let both_sources = FULL.replace(
            "acquire = { url",
            "acquire = { winget = \"Vendor.Custom\", url",
        );
        let manifest: Manifest = toml::from_str(&both_sources).unwrap();
        assert!(manifest.validate().is_err());
    }
}
