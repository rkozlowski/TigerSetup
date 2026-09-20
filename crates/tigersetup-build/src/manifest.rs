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
//! [[options]]
//! name = "path-mode"                    # a choice option: one of its values
//! kind = "choice"
//! default = "command"
//! label = { "en-US" = "PATH integration" }
//! choices = [
//!   { value = "none", label = { "en-US" = "Do not change PATH" } },
//!   { value = "command", label = { "en-US" = "Add the command to PATH" } },
//! ]
//!
//! [[shortcuts]]
//! location = "start-menu"               # | "desktop" | "startup" | "send-to"
//! target = "TigerMarkView.exe"
//! app_user_model_id = "ItTiger.TigerMarkView"
//!
//! [[path]]
//! entry = "."
//! when = { option = "path-mode", equals = "command" }   # or the older `option = "path"`
//!
//! [[environment]]
//! name = "TIGERMARKVIEW_HOME"
//! value = "%INSTALLROOT%"
//!
//! [[file_associations]]
//! prog_id = "TigerMarkView.Document"
//! extensions = [".md"]
//! description = "Markdown document"
//! executable = "TigerMarkView.exe"
//!
//! [[url_protocols]]
//! scheme = "tigermarkview"
//! executable = "TigerMarkView.exe"
//!
//! [[app_paths]]
//! executable = "TigerMarkView.exe"
//!
//! [[context_menu]]
//! target = "files"                      # | "directories" | "directory-background"
//! verb = "open-with-tigermarkview"
//! label = "Open with TigerMarkView"
//! executable = "TigerMarkView.exe"
//!
//! [[firewall]]
//! name = "TigerMarkView"
//! program = "TigerMarkView.exe"
//! direction = "in"
//! action = "allow"
//!
//! [[actions]]
//! name = "build-cache"                  # a custom lifecycle action (TigerSetup-Design.md §5.14)
//! phase = "post-install"                # | "pre-install" | "pre-uninstall" | "post-uninstall"
//! run_on = ["install", "upgrade", "reinstall", "repair"]   # default: the phase's operations without repair
//! kind = "powershell"                   # | "exe" | "cmd"
//! source = "actions/build-cache.ps1"    # packaged with the installer; or command = "%INSTALLROOT%\..." on the target
//! arguments = ["-Root", "%INSTALLROOT%"]
//! timeout_seconds = 120
//! on_failure = "fail"                   # | "continue"
//!
//! [[quiescence]]
//! name = "viewer"                       # stop an application the Restart Manager cannot close (TigerSetup-Design.md §5.10)
//! run_on = ["upgrade", "reinstall", "repair", "uninstall"]   # the default
//! not_running_codes = [3]               # stop exit codes that mean "nothing was running"
//!
//! [quiescence.stop]                     # the custom action's envelope, without a phase or run_on
//! kind = "exe"
//! command = "%INSTALLROOT%\TigerMarkView.exe"
//! arguments = ["--quit"]
//! timeout_seconds = 30
//!
//! [quiescence.resume]                   # optional: started detached after the run, only if stop stopped it
//! kind = "exe"
//! command = "%INSTALLROOT%\TigerMarkView.exe"
//! arguments = ["--background"]
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
//! [[dependencies]]
//! id = "Vendor.Prerequisite"            # carried inside the installer
//! detect = { kind = "file-version", path = "%PROGRAMFILES%\\Vendor\\vendor.dll" }
//! acquire = { file = "dependencies/vendor-setup.exe" }
//! install = { arguments = ["/S"], success_codes = [0], reboot_codes = [3010] }
//!
//! Every optional resource takes the same `when = { option, equals }`
//! predicate; there is nothing more to the predicate language on purpose.
//! [winget]
//! moniker = "tiger-markview"
//! commands = ["tiger-mark"]
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tigersetup_format::identity::{self, Scope};
use tigersetup_format::metadata::{
    ActionFailurePolicy, ActionKind, ActionOperation, ActionPhase, ExistingScopePolicy, Predicate,
    RegistryRoot, RegistryValue, is_sha256_hex, names_install_root, parse_bool,
    validate_action_name, validate_dotted_version, validate_environment_name, validate_extension,
    validate_option_name, validate_ports, validate_prog_id, validate_registration_key_name,
    validate_registry_key, validate_relative_path, validate_scheme, validate_url, validate_verb,
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
    pub environment: Vec<EnvironmentEntry>,
    #[serde(default)]
    pub file_associations: Vec<FileAssociationEntry>,
    #[serde(default)]
    pub url_protocols: Vec<UrlProtocolEntry>,
    #[serde(default)]
    pub app_paths: Vec<AppPathEntry>,
    #[serde(default)]
    pub context_menu: Vec<ContextMenuEntry>,
    #[serde(default)]
    pub firewall: Vec<FirewallEntry>,
    #[serde(default)]
    pub actions: Vec<ActionEntry>,
    #[serde(default)]
    pub quiescence: Vec<QuiescenceEntry>,
    #[serde(default)]
    pub winget: WingetSection,
}

/// The one predicate an optional resource may carry: `when = { option =
/// "x", equals = <value> }`, where the value is a boolean for a boolean
/// option and a choice value for a choice option.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PredicateDecl {
    pub option: String,
    pub equals: PredicateValue,
}

/// What a predicate compares with, as TOML spells it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PredicateValue {
    Bool(bool),
    Text(String),
}

impl PredicateValue {
    /// The canonical text of the value: `true`/`false`, or the choice.
    pub fn as_text(&self) -> String {
        match self {
            PredicateValue::Bool(true) => "true".into(),
            PredicateValue::Bool(false) => "false".into(),
            PredicateValue::Text(text) => text.to_ascii_lowercase(),
        }
    }
}

impl PredicateDecl {
    /// The runtime predicate.
    pub fn to_metadata(&self) -> Predicate {
        Predicate {
            option: self.option.to_ascii_lowercase(),
            equals: self.as_canonical_text(),
        }
    }

    /// The canonical text the engine compares. A boolean spelled as a word
    /// (`"on"`, `"yes"`, `"true"`) means the boolean; no choice value may
    /// be one of those words, so the two never meet.
    fn as_canonical_text(&self) -> String {
        match &self.equals {
            PredicateValue::Text(text) => match parse_bool(text) {
                Some(true) => "true".into(),
                Some(false) => "false".into(),
                None => text.to_ascii_lowercase(),
            },
            other => other.as_text(),
        }
    }
}

/// The predicate declared as `when`, or the older `option = "x"`.
pub fn predicate_of(when: Option<&PredicateDecl>, option: Option<&String>) -> Option<Predicate> {
    match (when, option) {
        (Some(when), _) => Some(when.to_metadata()),
        (None, Some(option)) => Some(Predicate {
            option: option.to_ascii_lowercase(),
            equals: "true".into(),
        }),
        (None, None) => None,
    }
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
    /// Installs these files only while the predicate holds: the files of
    /// an optional component.
    pub when: Option<PredicateDecl>,
}

/// The default of an option as TOML spells it: a boolean, or a choice value.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum OptionDefault {
    Bool(bool),
    Choice(String),
}

impl Default for OptionDefault {
    fn default() -> Self {
        OptionDefault::Bool(false)
    }
}

/// One value of a choice option.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceEntry {
    pub value: String,
    /// Wizard labels per BCP 47 tag; `en-US` is required.
    #[serde(default)]
    pub label: BTreeMap<String, String>,
}

/// An installer option the user can set on the command line or in the
/// wizard: a boolean, or a choice of one declared value.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionEntry {
    pub name: String,
    /// `true`/`false` for a boolean option, a choice value for a choice.
    #[serde(default)]
    pub default: OptionDefault,
    /// `path`, `desktop-shortcut` (labelled by the wizard in its own
    /// language), `custom` (the default; a boolean labelled by `label`) or
    /// `choice` (one of `choices`, labelled by `label` and each choice's).
    pub kind: Option<String>,
    /// Wizard labels per BCP 47 tag for a custom or choice option; `en-US`
    /// is required.
    #[serde(default)]
    pub label: BTreeMap<String, String>,
    /// The values of a choice option, in the order the wizard shows them.
    #[serde(default)]
    pub choices: Vec<ChoiceEntry>,
}

impl OptionEntry {
    pub fn kind_name(&self) -> &str {
        self.kind.as_deref().unwrap_or("custom")
    }

    pub fn is_choice(&self) -> bool {
        self.kind_name() == "choice"
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutEntry {
    /// `start-menu`, `desktop`, `startup` or `send-to`.
    pub location: String,
    /// Link name without `.lnk` (or `.url`); defaults to the package name.
    pub name: Option<String>,
    /// Install-relative target; absent for a URL shortcut.
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub arguments: String,
    /// Defaults to the package description.
    pub description: Option<String>,
    /// Install-relative icon source; defaults to the target.
    pub icon: Option<String>,
    /// The option that enables the shortcut; absent means always. The older
    /// spelling of `when = { option, equals = true }`.
    pub option: Option<String>,
    pub when: Option<PredicateDecl>,
    /// A subfolder under the location; absent places the link directly in it.
    pub folder: Option<String>,
    /// Install-relative working directory; defaults to the target's.
    pub working_directory: Option<String>,
    /// The AppUserModelID written into the link.
    pub app_user_model_id: Option<String>,
    /// A web page the shortcut opens instead of an installed file.
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathEntryDecl {
    /// Install-relative directory; `.` is the install root.
    pub entry: String,
    pub option: Option<String>,
    pub when: Option<PredicateDecl>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    /// `HKLM` or `HKCU`: the value lives at `key` below that hive rather
    /// than under the scope's `Software` root. The hive must be the one the
    /// package's only scope writes.
    pub root: Option<String>,
    /// Key under the scope's `Software` root, or under `root` when given.
    pub key: String,
    pub name: String,
    /// `string`, `expand-string` or `dword`.
    pub kind: String,
    /// `%INSTALLROOT%` and `%VERSION%` expand at install time.
    pub data: String,
    pub when: Option<PredicateDecl>,
}

/// A variable of the scope's environment.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentEntry {
    pub name: String,
    /// `%INSTALLROOT%` and `%VERSION%` expand at install time.
    pub value: String,
    /// Written as `REG_EXPAND_SZ`; defaults to a plain string.
    #[serde(default)]
    pub expandable: bool,
    pub when: Option<PredicateDecl>,
}

/// Registers the product as a handler for file extensions without making
/// it the default.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileAssociationEntry {
    pub prog_id: String,
    /// Extensions with their leading dot.
    pub extensions: Vec<String>,
    /// The type description Explorer shows.
    pub description: String,
    /// Install-relative icon source; defaults to the executable.
    pub icon: Option<String>,
    pub executable: String,
    /// Command arguments; defaults to `"%1"`.
    pub arguments: Option<String>,
    pub when: Option<PredicateDecl>,
}

/// Registers a URL scheme handler.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UrlProtocolEntry {
    /// The scheme without its colon.
    pub scheme: String,
    /// The handler's ProgID; defaults to `<name>.<scheme>`.
    pub prog_id: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub executable: String,
    /// Command arguments; defaults to `"%1"`.
    pub arguments: Option<String>,
    pub when: Option<PredicateDecl>,
}

/// An `App Paths` registration for an installed executable.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppPathEntry {
    pub executable: String,
    /// Also register the executable's directory as the entry's `Path`.
    #[serde(default)]
    pub add_directory: bool,
    pub when: Option<PredicateDecl>,
}

/// A classic Explorer context-menu verb.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMenuEntry {
    /// `files`, `directories` or `directory-background`.
    pub target: String,
    pub verb: String,
    pub label: String,
    pub executable: String,
    /// Command arguments; defaults to `"%1"` (`"%V"` for a background).
    pub arguments: Option<String>,
    pub icon: Option<String>,
    /// `files` only: limit the verb to these extensions.
    #[serde(default)]
    pub extensions: Vec<String>,
    pub when: Option<PredicateDecl>,
}

/// A Windows Firewall rule for an installed program.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirewallEntry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub program: String,
    /// `in` or `out`.
    pub direction: String,
    /// `allow` or `block`.
    pub action: String,
    /// `tcp`, `udp` or `any` (the default).
    pub protocol: Option<String>,
    /// `80`, `8000-8010` or `80,443`; needs a protocol.
    pub local_ports: Option<String>,
    pub when: Option<PredicateDecl>,
}

/// A custom lifecycle action: a program TigerSetup starts at one phase of
/// a run, on the operations it names, under a controlled envelope. What the
/// program changes is the package author's responsibility; TigerSetup owns
/// the execution and its record (`TigerSetup-Design.md` §5.14).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionEntry {
    /// Stable identity, unique within the package; lower-case words joined
    /// by `-`.
    pub name: String,
    /// `pre-install`, `post-install`, `pre-uninstall` or `post-uninstall`.
    pub phase: String,
    /// The operations the action runs on: `install`, `upgrade`, `reinstall`,
    /// `repair` for an install phase, `uninstall` for an uninstall phase.
    /// Absent means the phase's operations without `repair`, which is
    /// opt-in.
    pub run_on: Option<Vec<String>>,
    /// `exe`, `powershell` or `cmd`.
    pub kind: String,
    /// A program or script already on the target machine, as a template
    /// (`%INSTALLROOT%`, `%VERSION%`, the known folders). Exactly one of
    /// `command` and `source`.
    pub command: Option<String>,
    /// A manifest-relative program or script the builder packages inside
    /// the installer, hashes, and the engine extracts before running.
    pub source: Option<String>,
    /// Templates, passed as separate arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
    /// Template; defaults to the directory of the program or script.
    pub working_directory: Option<String>,
    /// Defaults to the engine's default (300 s).
    pub timeout_seconds: Option<u32>,
    /// Defaults to `[0]`.
    #[serde(default)]
    pub success_codes: Vec<i32>,
    /// Exit codes that mean success with a reboot pending.
    #[serde(default)]
    pub reboot_codes: Vec<i32>,
    /// `fail` (the default) or `continue`.
    pub on_failure: Option<String>,
    pub when: Option<PredicateDecl>,
}

/// One `[[quiescence]]` entry (TigerSetup-Design.md §5.10): a program
/// that stops a running application before the Restart Manager is asked,
/// and optionally one that starts it again after the run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuiescenceEntry {
    /// Stable identity, unique among quiescence entries and actions.
    pub name: String,
    /// The operations the entry runs on; absent means `upgrade`,
    /// `reinstall`, `repair` and `uninstall`.
    pub run_on: Option<Vec<String>>,
    /// Exit codes of `stop` that mean the application was not running.
    #[serde(default)]
    pub not_running_codes: Vec<i32>,
    pub stop: ProgramEntry,
    pub resume: Option<ProgramEntry>,
    pub when: Option<PredicateDecl>,
}

/// A program a quiescence entry runs: the custom action's envelope without
/// a name, a phase, operations, reboot codes or a predicate of its own.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramEntry {
    /// `exe`, `powershell` or `cmd`.
    pub kind: String,
    /// Exactly one of `command` and `source`, as for an action.
    pub command: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub timeout_seconds: Option<u32>,
    /// Defaults to `[0]`. For `stop`: the application was running and is
    /// now stopped.
    #[serde(default)]
    pub success_codes: Vec<i32>,
    /// `fail` (the default) or `continue`; meaningful for `stop`.
    pub on_failure: Option<String>,
}

impl ProgramEntry {
    /// The declaration as an action's, so that one validation and one
    /// resolution serve both.
    pub fn as_action_entry(&self, name: &str, phase: ActionPhase) -> ActionEntry {
        ActionEntry {
            name: name.to_string(),
            phase: phase.as_str().to_string(),
            run_on: None,
            kind: self.kind.clone(),
            command: self.command.clone(),
            source: self.source.clone(),
            arguments: self.arguments.clone(),
            working_directory: self.working_directory.clone(),
            timeout_seconds: self.timeout_seconds,
            success_codes: self.success_codes.clone(),
            reboot_codes: Vec::new(),
            on_failure: self.on_failure.clone(),
            when: None,
        }
    }
}

impl QuiescenceEntry {
    /// The stop and resume programs as action entries in their phases.
    pub fn programs(&self) -> Vec<ActionEntry> {
        let mut programs = vec![self.stop.as_action_entry(&self.name, ActionPhase::Quiesce)];
        if let Some(resume) = &self.resume {
            programs.push(resume.as_action_entry(&self.name, ActionPhase::Resume));
        }
        programs
    }
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
    /// Requires the dependency only while the predicate holds.
    pub when: Option<PredicateDecl>,
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
    /// A manifest-relative installer file carried inside the generated
    /// installer and run from it; nothing is downloaded.
    pub file: Option<String>,
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

/// Every rule a program an action or a quiescence entry runs is held to:
/// its kind, exactly one of a command and a packaged source, the
/// operations it names against its phase, and its envelope.
fn validate_action_entry(action: &ActionEntry, phase: ActionPhase) -> Result<()> {
    let invalid = |message: String| BuildError::new("manifest_invalid", message);
    let kind = ActionKind::parse(&action.kind).ok_or_else(|| {
        invalid(format!(
            "action {}: kind {:?} must be exe, powershell or cmd",
            action.name, action.kind
        ))
    })?;
    if let Some(run_on) = &action.run_on {
        if run_on.is_empty() {
            return Err(invalid(format!(
                "action {}: run_on names no operation",
                action.name
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for text in run_on {
            let operation = ActionOperation::parse(text).ok_or_else(|| {
                    invalid(format!(
                        "action {}: run_on {text:?} is not install, upgrade, reinstall, repair or uninstall",
                        action.name
                    ))
                })?;
            if !seen.insert(operation) {
                return Err(invalid(format!(
                    "action {}: run_on names {text} twice",
                    action.name
                )));
            }
            if !phase.allowed_operations().contains(&operation) {
                return Err(invalid(format!(
                    "action {}: a {} action cannot run on {text}",
                    action.name,
                    phase.as_str()
                )));
            }
        }
    }
    match (&action.command, &action.source) {
        (None, None) => {
            return Err(invalid(format!(
                "action {}: names neither a command nor a source",
                action.name
            )));
        }
        (Some(_), Some(_)) => {
            return Err(invalid(format!(
                "action {}: names both a command and a source; exactly one is allowed",
                action.name
            )));
        }
        (Some(command), None) => {
            if command.trim().is_empty() {
                return Err(invalid(format!(
                    "action {}: the command is empty",
                    action.name
                )));
            }
            if !kind.accepts_file(command) {
                return Err(invalid(format!(
                    "action {}: command {command:?} is not a file a {} action runs",
                    action.name,
                    kind.as_str()
                )));
            }
            if phase == ActionPhase::PostUninstall && names_install_root(command) {
                return Err(invalid(format!(
                    "action {}: a post-uninstall action cannot run a program under %INSTALLROOT%, which is removed before it runs",
                    action.name
                )));
            }
        }
        (None, Some(source)) => {
            relative_to_manifest(&format!("action {} source", action.name), source)?;
            if !kind.accepts_file(source) {
                return Err(invalid(format!(
                    "action {}: source {source:?} is not a file a {} action runs",
                    action.name,
                    kind.as_str()
                )));
            }
        }
    }
    if let Some(directory) = &action.working_directory
        && phase == ActionPhase::PostUninstall
        && names_install_root(directory)
    {
        return Err(invalid(format!(
            "action {}: a post-uninstall action cannot work under %INSTALLROOT%, which is removed before it runs",
            action.name
        )));
    }
    if action.timeout_seconds == Some(0) {
        return Err(invalid(format!(
            "action {}: timeout_seconds must be at least 1",
            action.name
        )));
    }
    if let Some(policy) = &action.on_failure
        && ActionFailurePolicy::parse(policy).is_none()
    {
        return Err(invalid(format!(
            "action {}: on_failure {policy:?} must be fail or continue",
            action.name
        )));
    }
    Ok(())
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
            let labelled = || {
                option
                    .label
                    .get("en-US")
                    .is_some_and(|l| !l.trim().is_empty())
            };
            match option.kind_name() {
                "path" | "desktop-shortcut" => {}
                "custom" => {
                    if !labelled() {
                        return Err(invalid(format!(
                            "option {} is custom and needs label.\"en-US\"",
                            option.name
                        )));
                    }
                }
                "choice" => {
                    if !labelled() {
                        return Err(invalid(format!(
                            "option {} is a choice and needs label.\"en-US\"",
                            option.name
                        )));
                    }
                    if option.choices.len() < 2 {
                        return Err(invalid(format!(
                            "option {} is a choice and needs at least two choices",
                            option.name
                        )));
                    }
                    if option.choices.len() > tigersetup_format::metadata::MAX_CHOICES {
                        return Err(invalid(format!(
                            "option {} declares more than {} choices",
                            option.name,
                            tigersetup_format::metadata::MAX_CHOICES
                        )));
                    }
                    let mut values = std::collections::HashSet::new();
                    for choice in &option.choices {
                        validate_option_name(&choice.value).map_err(|err| {
                            invalid(format!(
                                "option {}: choice value {:?} is not valid ({})",
                                option.name, choice.value, err.message
                            ))
                        })?;
                        if parse_bool(&choice.value).is_some() {
                            return Err(invalid(format!(
                                "option {}: choice value {:?} spells a boolean and cannot be a choice",
                                option.name, choice.value
                            )));
                        }
                        if !values.insert(choice.value.clone()) {
                            return Err(invalid(format!(
                                "option {} declares choice {} twice",
                                option.name, choice.value
                            )));
                        }
                        if choice
                            .label
                            .get("en-US")
                            .is_none_or(|l| l.trim().is_empty())
                        {
                            return Err(invalid(format!(
                                "option {}: choice {} needs label.\"en-US\"",
                                option.name, choice.value
                            )));
                        }
                    }
                    match &option.default {
                        OptionDefault::Choice(value) if values.contains(value) => {}
                        OptionDefault::Choice(value) => {
                            return Err(invalid(format!(
                                "option {}: default {value:?} is not one of its choices",
                                option.name
                            )));
                        }
                        OptionDefault::Bool(_) => {
                            return Err(invalid(format!(
                                "option {} is a choice and needs one of its choices as default",
                                option.name
                            )));
                        }
                    }
                }
                other => {
                    return Err(invalid(format!(
                        "option {} has kind {other:?}; expected path, desktop-shortcut, custom or choice",
                        option.name
                    )));
                }
            }
            if !option.is_choice() {
                if !option.choices.is_empty() {
                    return Err(invalid(format!(
                        "option {} declares choices but is not a choice option",
                        option.name
                    )));
                }
                if matches!(option.default, OptionDefault::Choice(_)) {
                    return Err(invalid(format!(
                        "option {} is a boolean option; its default is true or false",
                        option.name
                    )));
                }
            }
        }
        // Every `when` names a declared option and one of its values; the
        // older `option = "x"` names a declared boolean option.
        let predicate_known =
            |what: &str, when: Option<&PredicateDecl>, option: Option<&String>| -> Result<()> {
                let Some(predicate) = predicate_of(when, option) else {
                    return Ok(());
                };
                let declared = self
                    .options
                    .iter()
                    .find(|o| o.name.eq_ignore_ascii_case(&predicate.option))
                    .ok_or_else(|| {
                        invalid(format!(
                            "{what} depends on option {:?}, which is not declared",
                            predicate.option
                        ))
                    })?;
                let accepted = if declared.is_choice() {
                    declared
                        .choices
                        .iter()
                        .any(|c| c.value.eq_ignore_ascii_case(&predicate.equals))
                } else {
                    matches!(predicate.equals.as_str(), "true" | "false")
                };
                if !accepted {
                    return Err(invalid(format!(
                        "{what} wants option {} to equal {:?}, which is not one of its values",
                        predicate.option, predicate.equals
                    )));
                }
                Ok(())
            };
        for entry in &self.files {
            predicate_known(
                &format!("files {}", entry.source),
                entry.when.as_ref(),
                None,
            )?;
        }
        for shortcut in &self.shortcuts {
            if !matches!(
                shortcut.location.as_str(),
                "start-menu" | "desktop" | "startup" | "send-to"
            ) {
                return Err(invalid(format!(
                    "shortcut location {:?} must be start-menu, desktop, startup or send-to",
                    shortcut.location
                )));
            }
            if let Some(name) = &shortcut.name {
                identity::validate_name(name)?;
            }
            match &shortcut.url {
                Some(url) => {
                    validate_url(url)?;
                    if !shortcut.target.is_empty()
                        || !shortcut.arguments.is_empty()
                        || shortcut.working_directory.is_some()
                        || shortcut.app_user_model_id.is_some()
                    {
                        return Err(invalid(format!(
                            "shortcut {url} opens a URL and cannot also name a target, arguments, a working directory or an AppUserModelID"
                        )));
                    }
                }
                None => {
                    if shortcut.target.trim().is_empty() {
                        return Err(invalid(
                            "a shortcut names neither a target nor a URL".into(),
                        ));
                    }
                    validate_relative_path(&install_relative(&shortcut.target)?)?;
                    if let Some(directory) = &shortcut.working_directory {
                        install_relative(directory)?;
                    }
                }
            }
            if let Some(icon) = &shortcut.icon {
                validate_relative_path(&install_relative(icon)?)?;
            }
            if let Some(folder) = &shortcut.folder {
                validate_relative_path(&install_relative(folder)?)?;
            }
            predicate_known(
                "a shortcut",
                shortcut.when.as_ref(),
                shortcut.option.as_ref(),
            )?;
        }
        for entry in &self.path {
            install_relative(&entry.entry)?;
            predicate_known("a path entry", entry.when.as_ref(), entry.option.as_ref())?;
        }
        for value in &self.registry {
            validate_registry_key(&value.key)?;
            if let Some(root) = &value.root {
                let Some(root) = RegistryValue::root_of(root) else {
                    return Err(invalid(format!(
                        "registry value {}\\{} has root {root:?}; it must be HKLM or HKCU, or absent for the scope's Software root",
                        value.key, value.name
                    )));
                };
                let (hive_scope, spelling, other) = match root {
                    RegistryRoot::LocalMachine => (Scope::Machine, "HKLM", "user"),
                    _ => (Scope::User, "HKCU", "machine"),
                };
                if self.scopes().iter().any(|scope| *scope != hive_scope) {
                    return Err(invalid(format!(
                        "registry value {}\\{} has root {spelling} but install.scopes allows {other} scope: an explicit root needs a package whose only scope writes that hive",
                        value.key, value.name
                    )));
                }
            }
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
            predicate_known(
                &format!("registry value {}\\{}", value.key, value.name),
                value.when.as_ref(),
                None,
            )?;
        }
        for variable in &self.environment {
            validate_environment_name(&variable.name)?;
            predicate_known(
                &format!("environment variable {}", variable.name),
                variable.when.as_ref(),
                None,
            )?;
        }
        for association in &self.file_associations {
            validate_prog_id(&association.prog_id)?;
            if association.extensions.is_empty() {
                return Err(invalid(format!(
                    "file association {} names no extension",
                    association.prog_id
                )));
            }
            for extension in &association.extensions {
                validate_extension(extension)?;
            }
            validate_relative_path(&install_relative(&association.executable)?)?;
            if let Some(icon) = &association.icon {
                validate_relative_path(&install_relative(icon)?)?;
            }
            predicate_known(
                &format!("file association {}", association.prog_id),
                association.when.as_ref(),
                None,
            )?;
        }
        for protocol in &self.url_protocols {
            validate_scheme(&protocol.scheme)?;
            if let Some(prog_id) = &protocol.prog_id {
                validate_prog_id(prog_id)?;
            }
            validate_relative_path(&install_relative(&protocol.executable)?)?;
            if let Some(icon) = &protocol.icon {
                validate_relative_path(&install_relative(icon)?)?;
            }
            predicate_known(
                &format!("URL protocol {}", protocol.scheme),
                protocol.when.as_ref(),
                None,
            )?;
        }
        for app_path in &self.app_paths {
            let relative = install_relative(&app_path.executable)?;
            validate_relative_path(&relative)?;
            if !relative.to_ascii_lowercase().ends_with(".exe") {
                return Err(invalid(format!(
                    "App Paths entry {} is not an .exe",
                    app_path.executable
                )));
            }
            predicate_known(
                &format!("App Paths entry {}", app_path.executable),
                app_path.when.as_ref(),
                None,
            )?;
        }
        for verb in &self.context_menu {
            if !matches!(
                verb.target.as_str(),
                "files" | "directories" | "directory-background"
            ) {
                return Err(invalid(format!(
                    "context menu target {:?} must be files, directories or directory-background",
                    verb.target
                )));
            }
            validate_verb(&verb.verb)?;
            if verb.label.trim().is_empty() {
                return Err(invalid(format!(
                    "context menu verb {} has no label",
                    verb.verb
                )));
            }
            validate_relative_path(&install_relative(&verb.executable)?)?;
            if let Some(icon) = &verb.icon {
                validate_relative_path(&install_relative(icon)?)?;
            }
            if !verb.extensions.is_empty() && verb.target != "files" {
                return Err(invalid(format!(
                    "context menu verb {} limits itself to extensions but does not target files",
                    verb.verb
                )));
            }
            for extension in &verb.extensions {
                validate_extension(extension)?;
            }
            predicate_known(
                &format!("context menu verb {}", verb.verb),
                verb.when.as_ref(),
                None,
            )?;
        }
        for rule in &self.firewall {
            if rule.name.trim().is_empty() {
                return Err(invalid("a firewall rule has no name".into()));
            }
            validate_relative_path(&install_relative(&rule.program)?)?;
            if !matches!(rule.direction.as_str(), "in" | "out") {
                return Err(invalid(format!(
                    "firewall rule {}: direction {:?} must be in or out",
                    rule.name, rule.direction
                )));
            }
            if !matches!(rule.action.as_str(), "allow" | "block") {
                return Err(invalid(format!(
                    "firewall rule {}: action {:?} must be allow or block",
                    rule.name, rule.action
                )));
            }
            let protocol = rule.protocol.as_deref().unwrap_or("any");
            if !matches!(protocol, "any" | "tcp" | "udp") {
                return Err(invalid(format!(
                    "firewall rule {}: protocol {protocol:?} must be tcp, udp or any",
                    rule.name
                )));
            }
            if let Some(ports) = &rule.local_ports {
                if protocol == "any" {
                    return Err(invalid(format!(
                        "firewall rule {}: local_ports needs protocol tcp or udp",
                        rule.name
                    )));
                }
                validate_ports(ports)
                    .map_err(|why| invalid(format!("firewall rule {}: {why}", rule.name)))?;
            }
            predicate_known(
                &format!("firewall rule {}", rule.name),
                rule.when.as_ref(),
                None,
            )?;
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
                let sources = [&acquire.winget, &acquire.url, &acquire.file]
                    .iter()
                    .filter(|s| s.is_some())
                    .count();
                if sources > 1 {
                    return Err(invalid(format!(
                        "dependency {}: acquire names more than one of winget, url and file",
                        dependency.id
                    )));
                }
                if acquire.url.is_some() {
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
                if let Some(file) = &acquire.file {
                    relative_to_manifest(
                        &format!("dependency {} acquire.file", dependency.id),
                        file,
                    )?;
                    let lower = file.to_ascii_lowercase();
                    if !(lower.ends_with(".exe") || lower.ends_with(".msi")) {
                        return Err(invalid(format!(
                            "dependency {}: acquire.file {file:?} must be an .exe or .msi installer",
                            dependency.id
                        )));
                    }
                    if dependency.install.is_none() {
                        return Err(invalid(format!(
                            "dependency {}: an embedded installer needs an [dependencies.install] table",
                            dependency.id
                        )));
                    }
                    if acquire.sha256.is_some() || acquire.max_age_days.is_some() {
                        return Err(invalid(format!(
                            "dependency {}: an embedded installer takes neither sha256 nor max_age_days; the builder hashes the file it embeds",
                            dependency.id
                        )));
                    }
                }
            }
            predicate_known(
                &format!("dependency {}", dependency.id),
                dependency.when.as_ref(),
                None,
            )?;
        }
        let mut action_names = std::collections::HashSet::new();
        for action in &self.actions {
            validate_action_name(&action.name)?;
            if !action_names.insert(action.name.to_ascii_lowercase()) {
                return Err(invalid(format!("action {} is declared twice", action.name)));
            }
            let phase = ActionPhase::parse(&action.phase).ok_or_else(|| {
                invalid(format!(
                    "action {}: phase {:?} must be pre-install, post-install, pre-uninstall or post-uninstall",
                    action.name, action.phase
                ))
            })?;
            validate_action_entry(action, phase)?;
            predicate_known(
                &format!("action {}", action.name),
                action.when.as_ref(),
                None,
            )?;
        }
        for entry in &self.quiescence {
            validate_action_name(&entry.name)?;
            if !action_names.insert(entry.name.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "quiescence {} shares its name with an action or another quiescence entry",
                    entry.name
                )));
            }
            if let Some(run_on) = &entry.run_on {
                if run_on.is_empty() {
                    return Err(invalid(format!(
                        "quiescence {}: run_on names no operation",
                        entry.name
                    )));
                }
                let mut seen = std::collections::HashSet::new();
                for text in run_on {
                    let operation = ActionOperation::parse(text).ok_or_else(|| {
                        invalid(format!(
                            "quiescence {}: run_on {text:?} is not install, upgrade, reinstall, repair or uninstall",
                            entry.name
                        ))
                    })?;
                    if !seen.insert(operation) {
                        return Err(invalid(format!(
                            "quiescence {}: run_on names {text} twice",
                            entry.name
                        )));
                    }
                }
            }
            let stop_success: Vec<i32> = if entry.stop.success_codes.is_empty() {
                vec![0]
            } else {
                entry.stop.success_codes.clone()
            };
            for code in &entry.not_running_codes {
                if stop_success.contains(code) {
                    return Err(invalid(format!(
                        "quiescence {}: exit code {code} is both a success code and a not-running code of stop",
                        entry.name
                    )));
                }
            }
            for program in entry.programs() {
                let phase = ActionPhase::parse_quiescence(&program.phase)
                    .expect("a quiescence program has a quiescence phase");
                validate_action_entry(&program, phase)?;
            }
            predicate_known(
                &format!("quiescence {}", entry.name),
                entry.when.as_ref(),
                None,
            )?;
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

    /// Every 0.6 resource and predicate shape, in one manifest.
    const RICH: &str = r#"
[package]
id = "IT-Tiger.TigerSetupTestApp"
name = "TigerSetupTestApp"
version = "1.0.0"
publisher = "IT Tiger"

[install]
scopes = ["user", "machine"]

[[files]]
source = "payload/**"

[[files]]
source = "extras/**"
when = { option = "extras", equals = true }

[[options]]
name = "path-mode"
kind = "choice"
default = "command"
label = { "en-US" = "PATH integration", "pl-PL" = "Integracja z PATH" }
choices = [
  { value = "none", label = { "en-US" = "Do not change PATH" } },
  { value = "command", label = { "en-US" = "Add the command" } },
  { value = "tools", label = { "en-US" = "Add the command and the tools" } },
]

[[options]]
name = "extras"
label = { "en-US" = "Install the extras" }

[[options]]
name = "integration"
label = { "en-US" = "Integrate with Windows" }
default = true

[[path]]
entry = "bin"
when = { option = "path-mode", equals = "command" }

[[path]]
entry = "bin"
when = { option = "path-mode", equals = "tools" }

[[path]]
entry = "tools"
when = { option = "path-mode", equals = "tools" }

[[shortcuts]]
location = "start-menu"
target = "bin/TigerSetupTestApp.exe"
app_user_model_id = "ITTiger.TigerSetupTestApp"
working_directory = "data"

[[shortcuts]]
location = "startup"
name = "TigerSetupTestApp Agent"
target = "bin/TigerSetupTestApp.exe"
arguments = "--agent"
when = { option = "integration", equals = "on" }

[[shortcuts]]
location = "send-to"
target = "bin/TigerSetupTestApp.exe"

[[shortcuts]]
location = "start-menu"
name = "TigerSetupTestApp Documentation"
url = "https://example.invalid/docs"

[[environment]]
name = "TIGERSETUPTESTAPP_HOME"
value = "%INSTALLROOT%"
expandable = true
when = { option = "integration", equals = true }

[[file_associations]]
prog_id = "TigerSetupTestApp.Document"
extensions = [".tigertest"]
description = "TigerSetup test document"
executable = "bin/TigerSetupTestApp.exe"
when = { option = "integration", equals = true }

[[url_protocols]]
scheme = "tigersetuptest"
description = "TigerSetup test link"
executable = "bin/TigerSetupTestApp.exe"

[[app_paths]]
executable = "bin/TigerSetupTestApp.exe"
add_directory = true

[[context_menu]]
target = "files"
verb = "open-with-tigersetuptestapp"
label = "Open with TigerSetupTestApp"
executable = "bin/TigerSetupTestApp.exe"
icon = "bin/TigerSetupTestApp.exe"

[[context_menu]]
target = "directory-background"
verb = "tigersetuptestapp-here"
label = "TigerSetupTestApp here"
executable = "bin/TigerSetupTestApp.exe"

[[firewall]]
name = "TigerSetupTestApp listener"
description = "Lets the test application listen"
program = "bin/TigerSetupTestApp.exe"
direction = "in"
action = "allow"
protocol = "tcp"
local_ports = "47110"
when = { option = "integration", equals = true }

[[dependencies]]
id = "IT-Tiger.TigerSetupTestPrereq"
detect = { kind = "directory-version", path = "%PROGRAMDATA%\\TigerSetupTestPrereq", pattern = "1\\..*" }
acquire = { file = "dependencies/TigerSetupTestPrereq.exe" }
install = { arguments = ["--install"], success_codes = [0], reboot_codes = [3010] }
when = { option = "extras", equals = true }
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
    fn the_rich_manifest_parses_and_every_predicate_is_checked() {
        let manifest: Manifest = toml::from_str(RICH).unwrap();
        manifest.validate().unwrap();
        assert!(manifest.options[0].is_choice());
        assert_eq!(
            manifest.options[0].default,
            OptionDefault::Choice("command".into())
        );
        assert_eq!(manifest.path.len(), 3);
        assert_eq!(manifest.shortcuts.len(), 4);
        assert_eq!(manifest.environment.len(), 1);
        assert_eq!(manifest.firewall.len(), 1);
        let when = manifest.shortcuts[1].when.as_ref().unwrap();
        assert_eq!(
            when.to_metadata(),
            Predicate {
                option: "integration".into(),
                equals: "true".into()
            },
            "on is spelled true in the metadata"
        );
        assert_eq!(
            predicate_of(None, Some(&"desktop-shortcut".to_string())).unwrap(),
            Predicate {
                option: "desktop-shortcut".into(),
                equals: "true".into()
            }
        );

        let refused = |edit: &str, replacement: &str, why: &str| {
            let text = RICH.replacen(edit, replacement, 1);
            assert_ne!(text, RICH, "{why}: the edit changed nothing");
            let manifest: Manifest = toml::from_str(&text)
                .unwrap_or_else(|err| panic!("{why}: the manifest no longer parses: {err}"));
            let err = manifest.validate().expect_err(why);
            // The manifest's own checks say `manifest_invalid`; the checks it
            // shares with the format say `metadata_invalid`. Both refuse.
            assert!(
                matches!(err.code, "manifest_invalid" | "metadata_invalid"),
                "{why}: {} {}",
                err.code,
                err.message
            );
        };
        refused(
            "default = \"command\"",
            "default = \"nowhere\"",
            "a choice default must be one of the choices",
        );
        refused(
            "default = \"command\"",
            "default = true",
            "a choice option takes a choice as its default",
        );
        refused(
            "equals = \"tools\" }\n\n[[path]]\nentry = \"tools\"",
            "equals = \"sideways\" }\n\n[[path]]\nentry = \"tools\"",
            "a predicate must name one of the option's values",
        );
        refused(
            "when = { option = \"extras\", equals = true }\n\n[[options]]",
            "when = { option = \"nope\", equals = true }\n\n[[options]]",
            "a predicate must name a declared option",
        );
        refused(
            "when = { option = \"integration\", equals = \"on\" }",
            "when = { option = \"integration\", equals = \"tools\" }",
            "a boolean option compares with a boolean",
        );
        refused(
            "location = \"send-to\"",
            "location = \"quick-launch\"",
            "shortcut locations are the four known ones",
        );
        refused(
            "url = \"https://example.invalid/docs\"",
            "url = \"ftp://example.invalid/docs\"",
            "a URL shortcut is http, https or file",
        );
        refused(
            "name = \"TIGERSETUPTESTAPP_HOME\"",
            "name = \"Path\"",
            "PATH is not an environment variable resource",
        );
        refused(
            "extensions = [\".tigertest\"]",
            "extensions = [\"tigertest\"]",
            "an extension carries its dot",
        );
        refused(
            "scheme = \"tigersetuptest\"",
            "scheme = \"https\"",
            "a package never registers a scheme Windows owns",
        );
        refused(
            "target = \"directory-background\"",
            "target = \"drives\"",
            "context menu targets are the three known ones",
        );
        refused(
            "protocol = \"tcp\"\nlocal_ports",
            "protocol = \"any\"\nlocal_ports",
            "ports need a protocol",
        );
        refused(
            "local_ports = \"47110\"",
            "local_ports = \"47110-1\"",
            "a port range is low to high",
        );
        refused(
            "acquire = { file = \"dependencies/TigerSetupTestPrereq.exe\" }",
            "acquire = { file = \"dependencies/TigerSetupTestPrereq.zip\" }",
            "an embedded installer is an .exe or .msi",
        );
        refused(
            "acquire = { file = \"dependencies/TigerSetupTestPrereq.exe\" }",
            "acquire = { file = \"dependencies/TigerSetupTestPrereq.exe\", url = \"https://x.invalid/a.exe\", sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\" }",
            "one acquisition source at a time",
        );
        let unlabelled = RICH.replacen(
            "  { value = \"none\", label = { \"en-US\" = \"Do not change PATH\" } },",
            "  { value = \"none\" },",
            1,
        );
        let manifest: Manifest = toml::from_str(&unlabelled).unwrap();
        assert!(
            manifest.validate().is_err(),
            "every choice needs an English label"
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

    const EXPLICIT_REGISTRY: &str = r#"
[package]
id = "IT-Tiger.Sample"
name = "Sample"
version = "1.0.0"
publisher = "IT Tiger"

[install]
scopes = ["machine"]

[[files]]
source = "payload/**"

[[options]]
name = "long-paths"
default = true
label = { "en-US" = "Disable the Windows path length limit" }

[[registry]]
root = "HKLM"
key = "SYSTEM\\CurrentControlSet\\Control\\FileSystem"
name = "LongPathsEnabled"
kind = "dword"
data = "1"
when = { option = "long-paths", equals = true }

[[registry]]
key = "IT Tiger\\Sample"
name = "InstallRoot"
kind = "expand-string"
data = "%INSTALLROOT%"
"#;

    /// A `[[registry]]` value may name an explicit hive root; the hive must
    /// be the one the package's only scope writes, and the spelling one of
    /// the two the engine knows. A value without `root` keeps its meaning.
    #[test]
    fn an_explicit_registry_root_needs_the_scope_that_writes_its_hive() {
        let manifest: Manifest = toml::from_str(EXPLICIT_REGISTRY).unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.registry[0].root.as_deref(), Some("HKLM"));
        assert_eq!(manifest.registry[1].root, None);
        for spelling in ["hklm", "HKEY_LOCAL_MACHINE"] {
            let manifest: Manifest = toml::from_str(
                &EXPLICIT_REGISTRY.replace("root = \"HKLM\"", &format!("root = \"{spelling}\"")),
            )
            .unwrap();
            manifest.validate().unwrap();
        }
        let dual_scope =
            EXPLICIT_REGISTRY.replace("scopes = [\"machine\"]", "scopes = [\"user\", \"machine\"]");
        let manifest: Manifest = toml::from_str(&dual_scope).unwrap();
        let err = manifest.validate().unwrap_err();
        assert_eq!(err.code, "manifest_invalid");
        assert!(
            err.message.contains("HKLM") && err.message.contains("user scope"),
            "{}",
            err.message
        );
        let user_only = EXPLICIT_REGISTRY.replace("scopes = [\"machine\"]", "scopes = [\"user\"]");
        let manifest: Manifest = toml::from_str(&user_only).unwrap();
        assert!(manifest.validate().is_err());
        let hkcu_user = user_only.replace("root = \"HKLM\"", "root = \"HKCU\"");
        let manifest: Manifest = toml::from_str(&hkcu_user).unwrap();
        manifest.validate().unwrap();
        for bad in ["HKCR", "HKLM\\\\SYSTEM", ""] {
            let manifest: Manifest = toml::from_str(
                &EXPLICIT_REGISTRY.replace("root = \"HKLM\"", &format!("root = \"{bad}\"")),
            )
            .unwrap();
            let err = manifest.validate().unwrap_err();
            assert!(
                err.message.contains("must be HKLM or HKCU"),
                "{bad:?}: {}",
                err.message
            );
        }
    }

    const WITH_ACTIONS: &str = r#"
[package]
id = "IT-Tiger.Sample"
name = "Sample"
version = "1.0.0"
publisher = "IT Tiger"

[[files]]
source = "payload/**"

[[options]]
name = "cache"
default = true
label = { "en-US" = "Build the cache" }

[[actions]]
name = "build-cache"
phase = "post-install"
run_on = ["install", "upgrade", "reinstall", "repair"]
kind = "powershell"
source = "actions/build-cache.ps1"
arguments = ["-Root", "%INSTALLROOT%"]
working_directory = "%INSTALLROOT%"
timeout_seconds = 60
success_codes = [0]
reboot_codes = [3010]
on_failure = "fail"
when = { option = "cache", equals = true }

[[actions]]
name = "unregister"
phase = "pre-uninstall"
kind = "exe"
command = "%INSTALLROOT%\\Sample.exe"
arguments = ["--unregister"]
on_failure = "continue"

[[actions]]
name = "farewell"
phase = "post-uninstall"
kind = "cmd"
source = "actions/farewell.cmd"
"#;

    /// Every rule an `[[actions]]` declaration is held to, and the
    /// defaults it gets.
    #[test]
    fn actions_are_parsed_and_validated() {
        let manifest: Manifest = toml::from_str(WITH_ACTIONS).unwrap();
        manifest.validate().unwrap();
        assert_eq!(manifest.actions.len(), 3);
        assert_eq!(manifest.actions[1].run_on, None, "the default set");
        assert_eq!(manifest.actions[2].timeout_seconds, None);

        let refused = |replace: (&str, &str), needle: &str| {
            let text = WITH_ACTIONS.replace(replace.0, replace.1);
            assert_ne!(text, WITH_ACTIONS, "{:?} was not found", replace.0);
            let error = match toml::from_str::<Manifest>(&text) {
                Ok(manifest) => manifest.validate().unwrap_err().to_string(),
                Err(err) => err.to_string(),
            };
            assert!(error.contains(needle), "{error}");
        };
        refused(
            ("name = \"unregister\"", "name = \"build-cache\""),
            "declared twice",
        );
        refused(
            ("name = \"unregister\"", "name = \"Unregister Me\""),
            "not valid",
        );
        refused(
            ("phase = \"post-uninstall\"", "phase = \"after-install\""),
            "phase",
        );
        refused(("kind = \"cmd\"", "kind = \"bash\""), "kind");
        refused(
            (
                "run_on = [\"install\", \"upgrade\", \"reinstall\", \"repair\"]",
                "run_on = []",
            ),
            "names no operation",
        );
        refused(
            (
                "run_on = [\"install\", \"upgrade\", \"reinstall\", \"repair\"]",
                "run_on = [\"install\", \"install\"]",
            ),
            "twice",
        );
        refused(
            (
                "run_on = [\"install\", \"upgrade\", \"reinstall\", \"repair\"]",
                "run_on = [\"uninstall\"]",
            ),
            "cannot run on uninstall",
        );
        refused(
            (
                "run_on = [\"install\", \"upgrade\", \"reinstall\", \"repair\"]",
                "run_on = [\"remove\"]",
            ),
            "is not install",
        );
        refused(
            (
                "source = \"actions/farewell.cmd\"",
                "source = \"actions/farewell.ps1\"",
            ),
            "not a file a cmd action runs",
        );
        refused(
            (
                "source = \"actions/farewell.cmd\"",
                "command = \"C:\\\\x\\\\farewell.cmd\"\nsource = \"actions/farewell.cmd\"",
            ),
            "both a command and a source",
        );
        refused(
            ("source = \"actions/farewell.cmd\"", "arguments = []"),
            "neither a command nor a source",
        );
        refused(
            (
                "source = \"actions/farewell.cmd\"",
                "source = \"C:\\\\farewell.cmd\"",
            ),
            "must be relative",
        );
        refused(
            (
                "command = \"%INSTALLROOT%\\\\Sample.exe\"",
                "command = \"%INSTALLROOT%\\\\Sample.dll\"",
            ),
            "not a file a exe action runs",
        );
        refused(
            ("timeout_seconds = 60", "timeout_seconds = 0"),
            "at least 1",
        );
        refused(
            ("on_failure = \"continue\"", "on_failure = \"ignore\""),
            "fail or continue",
        );
        refused(
            (
                "when = { option = \"cache\", equals = true }",
                "when = { option = \"nope\", equals = true }",
            ),
            "not declared",
        );
        refused(
            (
                "name = \"unregister\"\nphase = \"pre-uninstall\"",
                "name = \"unregister\"\nphase = \"post-uninstall\"",
            ),
            "%INSTALLROOT%",
        );
        refused(
            (
                "source = \"actions/farewell.cmd\"",
                "source = \"actions/farewell.cmd\"\nworking_directory = \"%INSTALLROOT%\"",
            ),
            "%INSTALLROOT%",
        );
        refused(
            (
                "arguments = [\"--unregister\"]",
                "arguments = [\"--unregister\"]\nextra = 1",
            ),
            "unknown field",
        );
    }
}
