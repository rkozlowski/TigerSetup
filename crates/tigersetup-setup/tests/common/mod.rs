//! Shared support for the process-level tests: a synthetic two-version
//! package built with the freshly compiled engine, and an isolated machine
//! — its own known folders, its own registry hives and its own firewall
//! store — that spawns `Setup.exe` in either scope and reads its documents.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::Value;
use tigersetup_build::{BuildRequest, build};
use tigersetup_engine::format::identity::Scope;
use tigersetup_engine::win::firewall::{Rule, Store};
use tigersetup_engine::win::registry::{Data, KeyPath, Roots};
use tigersetup_engine::win::shortcut::LinkInspection;

pub const ENGINE: &str = env!("CARGO_BIN_EXE_tigersetup-setup");
pub const TMP: &str = env!("CARGO_TARGET_TMPDIR");
pub const PRODUCT_ID: &str = "IT-Tiger.TigerSetupTestApp";
pub const PRODUCT_NAME: &str = "TigerSetupTestApp";
pub const VERSION_A: &str = "1.0.0";
pub const VERSION_B: &str = "1.1.0";

/// The product's own key under the user scope's software root.
pub const PRODUCT_KEY: &str = "HKCU\\Software\\IT Tiger\\TigerSetupTestApp";
/// The parent key TigerSetup creates for it and removes again.
pub const VENDOR_KEY: &str = "HKCU\\Software\\IT Tiger";
pub const REGISTRATION_KEY: &str =
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.TigerSetupTestApp";
pub const ENVIRONMENT_KEY: &str = "HKCU\\Environment";

/// The test identities the package registers with Windows. None of them can
/// collide with a real application.
pub const ENVIRONMENT_VARIABLE: &str = "TIGERSETUPTESTAPP_HOME";
pub const PROG_ID: &str = "TigerSetupTestApp.Document";
pub const EXTENSION: &str = ".tigertest";
pub const SCHEME: &str = "tigersetuptest";
pub const FILES_VERB: &str = "open-with-tigersetuptestapp";
pub const BACKGROUND_VERB: &str = "tigersetuptestapp-here";
pub const FIREWALL_RULE: &str = "TigerSetupTestApp listener";
pub const APP_USER_MODEL_ID: &str = "ITTiger.TigerSetupTestApp";
pub const DOCUMENTATION_URL: &str = "https://ittiger.example/tigersetup-test-app";
pub const PREREQ_ID: &str = "IT-Tiger.TigerSetupTestPrereq";
pub const PREREQ_FILE_NAME: &str = "TigerSetupTestPrereq.exe";
/// The controlled program the package's custom actions run.
pub const ACTION_FILE_NAME: &str = "TigerSetupTestAction.exe";
/// The directory, under the machine's `%PROGRAMDATA%`, the package's
/// actions write their markers, records and cache into: outside the install
/// root, so that the file set on disk stays exactly the package's, and
/// outside TigerSetup's state, so that it outlives an uninstall.
pub const ACTIONS_DIR_NAME: &str = "TigerSetupTestActions";

/// The PowerShell script of the `build-cache` post-install action: writes
/// the cache file for the installed version, and records the run.
pub const BUILD_CACHE_PS1: &str = r#"param([string] $Root, [string] $Version, [string] $Actions)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Root -PathType Container)) { throw "install root $Root is missing" }
New-Item -ItemType Directory -Force -Path $Actions | Out-Null
Set-Content -LiteralPath (Join-Path $Actions 'cache.txt') -Value "TigerSetupTestApp cache for $Version" -Encoding ASCII
Add-Content -LiteralPath (Join-Path $Actions 'record.txt') -Value "build-cache`t$Version`t$env:TIGERSETUP_OPERATION`t$env:TIGERSETUP_PHASE`t$env:TIGERSETUP_QUIET" -Encoding UTF8
Write-Output "cache built for $Version"
"#;

/// The batch script of the `clear-cache` pre-uninstall action: removes the
/// cache, leaves the pre-uninstall marker, and records the run.
pub const CLEAR_CACHE_CMD: &str = concat!(
    "@echo off\r\n",
    "if not exist \"%~1\" mkdir \"%~1\"\r\n",
    "if exist \"%~1\\cache.txt\" del /f /q \"%~1\\cache.txt\"\r\n",
    "echo TigerSetupTestAction> \"%~1\\pre-uninstall.txt\"\r\n",
    "echo clear-cache\t%TIGERSETUP_OPERATION%\t%TIGERSETUP_PHASE%>> \"%~1\\record.txt\"\r\n",
    "exit /b 0\r\n",
);

/// The `[[actions]]` of the synthetic package, shared by both versions:
/// an option-gated pre-install marker (off by default, so the default
/// journal keeps its operation numbers), a post-install cache built by a
/// PowerShell script (on repair too), an option-gated post-install failure,
/// a pre-uninstall batch script that clears the cache, and a post-uninstall
/// marker written outside the removed install root. Every program is
/// packaged; the executable is packaged once for its three actions.
pub const ACTIONS_TOML: &str = r#"[[options]]
name = "preflight"
default = false
label = { "en-US" = "Run the preflight check", "pl-PL" = "Uruchom sprawdzenie wstępne" }

[[options]]
name = "fail-action"
default = false
label = { "en-US" = "Fail the post-install action (testing)", "pl-PL" = "Przerwij akcję poinstalacyjną (test)" }

[[actions]]
name = "preflight"
phase = "pre-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--marker", "%PROGRAMDATA%\\TigerSetupTestActions\\preflight.txt", "--record", "%PROGRAMDATA%\\TigerSetupTestActions\\record.txt", "--stdout", "preflight ok"]
when = { option = "preflight", equals = true }

[[actions]]
name = "fail-on-purpose"
phase = "post-install"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--marker", "%PROGRAMDATA%\\TigerSetupTestActions\\failed-action.txt", "--stderr", "failing as asked", "--exit", "3"]
timeout_seconds = 60
when = { option = "fail-action", equals = true }

[[actions]]
name = "build-cache"
phase = "post-install"
run_on = ["install", "upgrade", "reinstall", "repair"]
kind = "powershell"
source = "actions/build-cache.ps1"
arguments = ["-Root", "%INSTALLROOT%", "-Version", "%VERSION%", "-Actions", "%PROGRAMDATA%\\TigerSetupTestActions"]
timeout_seconds = 120

[[actions]]
name = "clear-cache"
phase = "pre-uninstall"
kind = "cmd"
source = "actions/clear-cache.cmd"
arguments = ["%PROGRAMDATA%\\TigerSetupTestActions"]

[[actions]]
name = "farewell"
phase = "post-uninstall"
kind = "exe"
source = "actions/TigerSetupTestAction.exe"
arguments = ["--marker", "%PROGRAMDATA%\\TigerSetupTestActions\\post-uninstall.txt", "--record", "%PROGRAMDATA%\\TigerSetupTestActions\\record.txt"]
"#;

/// The synthetic package declares one of every resource kind, each gated by
/// the option the consolidated acceptance package names: a PATH mode
/// (none, command, tools — the last also installs the `tools` component), a
/// Start Menu link with an AppUserModelID and a working directory, an
/// option-gated desktop link (spelled the older `option = ` way, which
/// stays supported), a Startup link, a Send To link, a documentation URL
/// shortcut, two product registry values (one carrying the version, so an
/// upgrade has a value to replace), an environment variable, a file
/// association, a URL protocol, an `App Paths` entry, two context-menu
/// verbs, a firewall rule, an optional `extras` component, an embedded
/// prerequisite and an Add/Remove Programs registration with an icon.
fn manifest(version: &str) -> String {
    format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{version}"
publisher = "IT Tiger"

[install]
scopes = ["user", "machine"]

[[files]]
source = "payload/**"

[[files]]
source = "payload-extras/**"
when = {{ option = "extras", equals = true }}

[[files]]
source = "payload-tools/**"
when = {{ option = "path-mode", equals = "tools" }}

[[options]]
name = "path-mode"
kind = "choice"
default = "command"
label = {{ "en-US" = "PATH integration", "pl-PL" = "Integracja ze zmienną PATH" }}
choices = [
  {{ value = "none", label = {{ "en-US" = "Do not change PATH", "pl-PL" = "Nie zmieniaj PATH" }} }},
  {{ value = "command", label = {{ "en-US" = "Add the command to PATH", "pl-PL" = "Dodaj polecenie do PATH" }} }},
  {{ value = "tools", label = {{ "en-US" = "Add the command and the tools to PATH", "pl-PL" = "Dodaj polecenie i narzędzia do PATH" }} }},
]

[[options]]
name = "desktop-shortcut"
kind = "desktop-shortcut"
default = false

[[options]]
name = "startup"
default = true
label = {{ "en-US" = "Start the agent at sign-in", "pl-PL" = "Uruchamiaj agenta przy logowaniu" }}

[[options]]
name = "send-to"
default = false
label = {{ "en-US" = "Add a Send To entry", "pl-PL" = "Dodaj pozycję Wyślij do" }}

[[options]]
name = "file-association"
default = true
label = {{ "en-US" = "Open .tigertest documents", "pl-PL" = "Otwieraj dokumenty .tigertest" }}

[[options]]
name = "url-protocol"
default = true
label = {{ "en-US" = "Handle tigersetuptest: links", "pl-PL" = "Obsługuj łącza tigersetuptest:" }}

[[options]]
name = "context-menu"
default = true
label = {{ "en-US" = "Add Explorer context menu commands", "pl-PL" = "Dodaj polecenia do menu kontekstowego Eksploratora" }}

[[options]]
name = "environment"
default = true
label = {{ "en-US" = "Set the TIGERSETUPTESTAPP_HOME variable", "pl-PL" = "Ustaw zmienną TIGERSETUPTESTAPP_HOME" }}

[[options]]
name = "firewall"
default = true
label = {{ "en-US" = "Allow the application through the firewall", "pl-PL" = "Zezwól aplikacji na komunikację przez zaporę" }}

[[options]]
name = "extras"
default = false
label = {{ "en-US" = "Install the extras", "pl-PL" = "Zainstaluj dodatki" }}

[[shortcuts]]
location = "start-menu"
target = "bin/TigerSetupTestApp.exe"
description = "The synthetic test application"
app_user_model_id = "{APP_USER_MODEL_ID}"
working_directory = "data"

[[shortcuts]]
location = "desktop"
target = "bin/TigerSetupTestApp.exe"
option = "desktop-shortcut"

[[shortcuts]]
location = "startup"
name = "TigerSetupTestApp Agent"
target = "bin/TigerSetupTestApp.exe"
arguments = "--agent"
when = {{ option = "startup", equals = true }}

[[shortcuts]]
location = "send-to"
target = "bin/TigerSetupTestApp.exe"
when = {{ option = "send-to", equals = true }}

[[shortcuts]]
location = "start-menu"
name = "TigerSetupTestApp Documentation"
url = "{DOCUMENTATION_URL}"

[[path]]
entry = "bin"
when = {{ option = "path-mode", equals = "command" }}

[[path]]
entry = "bin"
when = {{ option = "path-mode", equals = "tools" }}

[[path]]
entry = "tools"
when = {{ option = "path-mode", equals = "tools" }}

[[registry]]
key = "IT Tiger\\TigerSetupTestApp"
name = "InstallRoot"
kind = "expand-string"
data = "%INSTALLROOT%"

[[registry]]
key = "IT Tiger\\TigerSetupTestApp"
name = "Version"
kind = "string"
data = "%VERSION%"

[[environment]]
name = "{ENVIRONMENT_VARIABLE}"
value = "%INSTALLROOT%"
expandable = true
when = {{ option = "environment", equals = true }}

[[file_associations]]
prog_id = "{PROG_ID}"
extensions = ["{EXTENSION}"]
description = "TigerSetup test document"
executable = "bin/TigerSetupTestApp.exe"
when = {{ option = "file-association", equals = true }}

[[url_protocols]]
scheme = "{SCHEME}"
description = "TigerSetup test link"
executable = "bin/TigerSetupTestApp.exe"
when = {{ option = "url-protocol", equals = true }}

[[app_paths]]
executable = "bin/TigerSetupTestApp.exe"
add_directory = true

[[context_menu]]
target = "files"
verb = "{FILES_VERB}"
label = "Open with TigerSetupTestApp"
executable = "bin/TigerSetupTestApp.exe"
icon = "bin/TigerSetupTestApp.exe"
when = {{ option = "context-menu", equals = true }}

[[context_menu]]
target = "directory-background"
verb = "{BACKGROUND_VERB}"
label = "TigerSetupTestApp here"
executable = "bin/TigerSetupTestApp.exe"
when = {{ option = "context-menu", equals = true }}

[[firewall]]
name = "{FIREWALL_RULE}"
description = "Lets the synthetic test application listen"
program = "bin/TigerSetupTestApp.exe"
direction = "in"
action = "allow"
protocol = "tcp"
local_ports = "47110"
when = {{ option = "firewall", equals = true }}

[[dependencies]]
id = "{PREREQ_ID}"
name = "TigerSetup test prerequisite"
detect = {{ kind = "directory-version", path = "%PROGRAMDATA%\\TigerSetupTestPrereq", pattern = "1\\..*" }}
acquire = {{ file = "dependencies/{PREREQ_FILE_NAME}" }}
install = {{ arguments = ["--install"], success_codes = [0], reboot_codes = [3010] }}

[registration]
display_icon = "bin/TigerSetupTestApp.exe"

{ACTIONS_TOML}"#
    )
}

/// One file of the synthetic layout: install-relative path (forward
/// slashes), size, the seed its bytes derive from, and the component it
/// belongs to (`None` for the core file set).
pub struct FileSpec {
    pub relative: String,
    pub size: usize,
    pub seed: String,
    pub component: Option<&'static str>,
}

/// The `extras` component: an option-gated file set of its own directory.
pub const COMPONENT_EXTRAS: &str = "extras";
/// The `tools` component: the files the `tools` PATH mode installs.
pub const COMPONENT_TOOLS: &str = "tools";

/// The two versions of the synthetic package: 1.0.0 has about sixty core
/// files in nested directories, several of 2–6 MB; 1.1.0 keeps most of them,
/// changes five, removes five (emptying `doc/legacy`) and adds five (one in
/// the new `data/extra`). Both carry the `extras` and `tools` components
/// beside the core.
pub fn layout(version: &str) -> Vec<FileSpec> {
    let mut base: Vec<(String, usize)> = vec![
        ("CHANGELOG.md".into(), 3_000),
        ("LICENSE.txt".into(), 1_100),
        ("bin/TigerSetupTestApp.exe".into(), 180_000),
        ("doc/readme.md".into(), 6_000),
        ("doc/legacy/old-01.md".into(), 2_500),
        ("doc/legacy/old-02.md".into(), 2_600),
        ("locale/en-US.json".into(), 4_000),
        ("locale/pl-PL.json".into(), 4_400),
    ];
    for i in 1..=8 {
        base.push((format!("bin/lib-{i:02}.dll"), 20_000 + i * 7_000));
    }
    for i in 1..=4 {
        base.push((
            format!("bin/x64/native-{i}.bin"),
            if i == 3 { 3 * 1024 * 1024 } else { 60_000 * i },
        ));
    }
    for (i, mb) in [2usize, 3, 4, 6, 4].iter().enumerate() {
        base.push((format!("data/big-{}.bin", i + 1), mb * 1024 * 1024));
    }
    base.push(("data/tables/index.dat".into(), 8_000));
    for i in 1..=20 {
        base.push((format!("data/tables/t-{i:02}.dat"), 10_000 + i * 2_000));
    }
    for i in 1..=10 {
        base.push((format!("doc/manual/ch-{i:02}.md"), 5_000 + i * 300));
    }
    for i in 1..=8 {
        base.push((format!("lib/plugins/p-{i:02}.plug"), 30_000 + i * 1_000));
    }

    let changed = [
        "CHANGELOG.md",
        "bin/TigerSetupTestApp.exe",
        "bin/lib-03.dll",
        "data/big-4.bin",
        "doc/readme.md",
    ];
    let removed = [
        "bin/lib-08.dll",
        "doc/legacy/old-01.md",
        "doc/legacy/old-02.md",
        "doc/manual/ch-10.md",
        "lib/plugins/p-08.plug",
    ];
    let added: [(&str, usize); 5] = [
        ("bin/lib-09.dll", 83_000),
        ("data/big-6.bin", 3 * 1024 * 1024),
        ("data/extra/notes.txt", 1_500),
        ("doc/manual/ch-11.md", 5_000 + 11 * 300),
        ("locale/de-DE.json", 4_200),
    ];

    let mut files: Vec<FileSpec> = Vec::new();
    if version == VERSION_A {
        for (relative, size) in base {
            files.push(FileSpec {
                seed: format!("{relative}|{VERSION_A}"),
                relative,
                size,
                component: None,
            });
        }
    } else {
        for (relative, size) in base {
            if removed.contains(&relative.as_str()) {
                continue;
            }
            let seed_version = if changed.contains(&relative.as_str()) {
                version
            } else {
                VERSION_A
            };
            files.push(FileSpec {
                seed: format!("{relative}|{seed_version}"),
                relative,
                size,
                component: None,
            });
        }
        for (relative, size) in added {
            files.push(FileSpec {
                relative: relative.to_string(),
                size,
                seed: format!("{relative}|{version}"),
                component: None,
            });
        }
    }
    // The components are the same in both versions apart from the version
    // in their seed, so an upgrade replaces them like any changed file.
    let components: [(&str, &str, usize); 4] = [
        (COMPONENT_EXTRAS, "extras/notes.txt", 2_000),
        (COMPONENT_EXTRAS, "extras/data/samples.bin", 1024 * 1024),
        (COMPONENT_TOOLS, "tools/tsta-tool.exe", 90_000),
        (COMPONENT_TOOLS, "tools/tsta-tool.txt", 1_200),
    ];
    for (component, relative, size) in components {
        files.push(FileSpec {
            relative: relative.to_string(),
            size,
            seed: format!("{relative}|{version}"),
            component: Some(component),
        });
    }
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    files
}

pub fn deterministic_bytes(seed: &str, size: usize) -> Vec<u8> {
    let mut state = seed.bytes().fold(0x9E37_79B9_7F4A_7C15u64, |acc, b| {
        acc.rotate_left(5) ^ (b as u64).wrapping_mul(0x100_0000_01B3)
    });
    let mut out = Vec::with_capacity(size + 8);
    while out.len() < size {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(size);
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    tigersetup_format::hex(&tigersetup_format::sha256(bytes))
}

/// The effective options of an installation, as `inspect` reports them:
/// `true`/`false` for a boolean, the value for a choice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selected {
    pub values: BTreeMap<String, Value>,
}

impl Selected {
    /// The package defaults: what a run that names no option installs.
    pub fn defaults() -> Selected {
        let mut values = BTreeMap::new();
        values.insert("path-mode".into(), Value::String("command".into()));
        for (name, on) in [
            ("desktop-shortcut", false),
            ("startup", true),
            ("send-to", false),
            ("file-association", true),
            ("url-protocol", true),
            ("context-menu", true),
            ("environment", true),
            ("firewall", true),
            ("extras", false),
            ("preflight", false),
            ("fail-action", false),
        ] {
            values.insert(name.into(), Value::Bool(on));
        }
        Selected { values }
    }

    pub fn from_report(report: &Value) -> Selected {
        let mut selected = Selected::defaults();
        if let Some(options) = report["owned"]["options"].as_object() {
            for (name, value) in options {
                selected.values.insert(name.clone(), value.clone());
            }
        }
        selected
    }

    pub fn on(&self, name: &str) -> bool {
        self.values
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn path_mode(&self) -> &str {
        self.values
            .get("path-mode")
            .and_then(Value::as_str)
            .unwrap_or("command")
    }

    /// Whether a component's files are installed under these options.
    pub fn has_component(&self, component: &str) -> bool {
        match component {
            COMPONENT_EXTRAS => self.on("extras"),
            COMPONENT_TOOLS => self.path_mode() == "tools",
            _ => true,
        }
    }
}

/// One built version: its installer, its payload directories and its file
/// set, core and components.
pub struct VersionFixture {
    pub version: &'static str,
    pub installer: PathBuf,
    /// The directory holding `payload`, `payload-extras`, `payload-tools`
    /// and `dependencies`.
    pub source: PathBuf,
    pub specs: Vec<(String, Option<&'static str>)>,
    /// The core file set: what every installation carries.
    pub files: Vec<String>,
}

impl VersionFixture {
    /// The install-relative paths (with backslashes, as the engine reports
    /// them) an installation with `selected` carries.
    pub fn expected_paths(&self, selected: &Selected) -> BTreeSet<String> {
        self.expected_files(selected)
            .iter()
            .map(|f| f.replace('/', "\\"))
            .collect()
    }

    /// The forward-slash file list an installation with `selected` carries.
    pub fn expected_files(&self, selected: &Selected) -> Vec<String> {
        self.specs
            .iter()
            .filter(|(_, component)| component.is_none_or(|c| selected.has_component(c)))
            .map(|(relative, _)| relative.clone())
            .collect()
    }

    /// The install-relative core paths with backslashes.
    pub fn relative_paths(&self) -> BTreeSet<String> {
        self.files.iter().map(|f| f.replace('/', "\\")).collect()
    }

    /// The payload directory a file lives in: the core payload or a
    /// component's.
    pub fn source_of(&self, relative_forward: &str) -> PathBuf {
        let component = self
            .specs
            .iter()
            .find(|(relative, _)| relative == relative_forward)
            .and_then(|(_, component)| *component);
        let directory = match component {
            Some(component) => format!("payload-{component}"),
            None => "payload".to_string(),
        };
        self.source.join(directory).join(relative_forward)
    }

    pub fn payload_sha256(&self, relative_forward: &str) -> String {
        sha256_hex(&fs::read(self.source_of(relative_forward)).unwrap())
    }

    /// The embedded prerequisite installer's bytes, as the builder read them.
    pub fn prereq_sha256(&self) -> String {
        sha256_hex(&fs::read(self.source.join("dependencies").join(PREREQ_FILE_NAME)).unwrap())
    }

    /// The SHA-256 of a packaged action file, as the builder read it.
    pub fn action_sha256(&self, file_name: &str) -> String {
        sha256_hex(&fs::read(self.source.join("actions").join(file_name)).unwrap())
    }
}

pub struct Fixture {
    pub a: VersionFixture,
    pub b: VersionFixture,
}

/// The embedded prerequisite executable: built by `cargo build` beside the
/// engine when the whole workspace is built, and built here when a single
/// package's tests were asked for.
pub fn prereq_executable() -> PathBuf {
    let beside_engine = Path::new(ENGINE).parent().unwrap().join(PREREQ_FILE_NAME);
    if beside_engine.exists() {
        return beside_engine;
    }
    let target_dir = Path::new(ENGINE)
        .ancestors()
        .nth(3)
        .expect("target/<triple>/<profile>/engine.exe");
    let profile = Path::new(ENGINE)
        .parent()
        .and_then(Path::file_name)
        .and_then(|p| p.to_str())
        .unwrap_or("debug");
    let mut command = Command::new(env!("CARGO"));
    command
        .args(["build", "-p", "tigersetup-test-prereq", "--target-dir"])
        .arg(target_dir);
    if profile != "debug" {
        command.args(["--profile", profile]);
    }
    let status = command.status().expect("cargo runs");
    assert!(status.success(), "the prerequisite fixture builds");
    assert!(beside_engine.exists(), "{}", beside_engine.display());
    beside_engine
}

/// The controlled action program: built by `cargo build` beside the engine
/// when the whole workspace is built, and built here when a single
/// package's tests were asked for.
pub fn action_executable() -> PathBuf {
    workspace_fixture("tigersetup-test-action", ACTION_FILE_NAME)
}

/// A workspace binary beside the engine, built on demand.
fn workspace_fixture(package: &str, file_name: &str) -> PathBuf {
    let beside_engine = Path::new(ENGINE).parent().unwrap().join(file_name);
    if beside_engine.exists() {
        return beside_engine;
    }
    let target_dir = Path::new(ENGINE)
        .ancestors()
        .nth(3)
        .expect("target/<triple>/<profile>/engine.exe");
    let profile = Path::new(ENGINE)
        .parent()
        .and_then(Path::file_name)
        .and_then(|p| p.to_str())
        .unwrap_or("debug");
    let mut command = Command::new(env!("CARGO"));
    command
        .args(["build", "-p", package, "--target-dir"])
        .arg(target_dir);
    if profile != "debug" {
        command.args(["--profile", profile]);
    }
    let status = command.status().expect("cargo runs");
    assert!(status.success(), "the {package} fixture builds");
    assert!(beside_engine.exists(), "{}", beside_engine.display());
    beside_engine
}

/// Writes the `actions/` directory of a package: the controlled program
/// and the two scripts.
pub fn write_actions_dir(dir: &Path) {
    let actions = dir.join("actions");
    fs::create_dir_all(&actions).unwrap();
    fs::copy(action_executable(), actions.join(ACTION_FILE_NAME)).unwrap();
    fs::write(actions.join("build-cache.ps1"), BUILD_CACHE_PS1).unwrap();
    fs::write(actions.join("clear-cache.cmd"), CLEAR_CACHE_CMD).unwrap();
}

fn build_version(root: &Path, version: &'static str) -> VersionFixture {
    let dir = root.join(version);
    let mut specs = Vec::new();
    let mut files = Vec::new();
    for spec in layout(version) {
        let directory = match spec.component {
            Some(component) => format!("payload-{component}"),
            None => "payload".to_string(),
        };
        let path = dir.join(directory).join(&spec.relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, deterministic_bytes(&spec.seed, spec.size)).unwrap();
        if spec.component.is_none() {
            files.push(spec.relative.clone());
        }
        specs.push((spec.relative, spec.component));
    }
    fs::create_dir_all(dir.join("dependencies")).unwrap();
    fs::copy(
        prereq_executable(),
        dir.join("dependencies").join(PREREQ_FILE_NAME),
    )
    .unwrap();
    write_actions_dir(&dir);
    fs::write(dir.join("TigerSetup.toml"), manifest(version)).unwrap();
    let request = BuildRequest {
        manifest_path: &dir.join("TigerSetup.toml"),
        output: &root.join("out"),
        engine_path: Some(Path::new(ENGINE)),
        properties: &[],
        offline: true,
        // Tests build many installers; the payload's size is not what they
        // measure.
        compression: tigersetup_build::Compression::Fast,
    };
    let result = build(&request).expect("the synthetic package builds");
    assert_eq!(result.file_count, specs.len());
    VersionFixture {
        version,
        installer: result.installer_path,
        source: dir,
        specs,
        files,
    }
}

/// Builds a one-file package of the same product from `manifest_text`, for
/// a test about what surrounds a run — the platform baseline, a legacy
/// installation, the elevation hand-back — rather than about its file set.
/// Everything the fixture derives from `PRODUCT_ID` and `PRODUCT_NAME`
/// therefore still applies to it.
pub fn build_small_package(dir: &Path, manifest_text: &str) -> PathBuf {
    let payload = dir.join("payload");
    fs::create_dir_all(payload.join("bin")).unwrap();
    fs::write(payload.join("bin").join("app.txt"), b"small").unwrap();
    fs::write(dir.join("TigerSetup.toml"), manifest_text).unwrap();
    let request = BuildRequest {
        manifest_path: &dir.join("TigerSetup.toml"),
        output: &dir.join("out"),
        engine_path: Some(Path::new(ENGINE)),
        properties: &[],
        offline: true,
        // Tests build many installers; the payload's size is not what they
        // measure.
        compression: tigersetup_build::Compression::Fast,
    };
    build(&request).expect("the package builds").installer_path
}

/// The licence text of the licensed test packages, and the same text after
/// the annual copyright edit: one byte apart, and a different agreement.
pub const LICENSE_2026: &str = "MIT License\r\n\r\nCopyright (c) 2026 IT Tiger\r\n";
pub const LICENSE_2027: &str = "MIT License\r\n\r\nCopyright (c) 2027 IT Tiger\r\n";

/// A one-file package of the product, at `version`, that carries
/// `license_text` for the wizard's licence page. It declares no options, so
/// a same-version rerun has nothing to reconcile but the acceptance.
pub fn build_licensed_package(dir: &Path, version: &str, license_text: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("LICENSE.txt"), license_text.as_bytes()).unwrap();
    let manifest = format!(
        r#"[package]
id = "{PRODUCT_ID}"
name = "{PRODUCT_NAME}"
version = "{version}"
publisher = "IT Tiger"
license = "MIT"
license_file = "LICENSE.txt"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"
"#
    );
    build_small_package(dir, &manifest)
}

/// The identity the engine records for an accepted licence text: lower-case
/// hex SHA-256 of its exact UTF-8 bytes.
pub fn license_sha256(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

/// A directory of this test binary's own, named after `what`, emptied first.
pub fn scratch(what: &str) -> PathBuf {
    let dir = Path::new(TMP).join(format!("{}-{what}", env!("CARGO_CRATE_NAME")));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Built once per test binary, under `target\…\tmp\fixture-<test crate>`.
pub fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let root = Path::new(TMP).join(format!("fixture-{}", env!("CARGO_CRATE_NAME")));
        let _ = fs::remove_dir_all(&root);
        Fixture {
            a: build_version(&root, VERSION_A),
            b: build_version(&root, VERSION_B),
        }
    })
}

/// One isolated machine: every known folder the engine resolves, its own
/// `%TEMP%`, and its own relocated registry roots, so that a test never
/// touches the real ones and two tests never collide. Everything it created
/// goes away with it.
///
/// A machine serves one scope, named when it is created, and every command
/// it runs names that scope. Machine scope resolves into this fixture's own
/// `%PROGRAMDATA%` and `%PROGRAMFILES%`, which the test process owns, so an
/// unelevated test can exercise it.
pub struct Machine {
    _dir: tempfile::TempDir,
    pub scope: Scope,
    pub localappdata: PathBuf,
    pub programdata: PathBuf,
    pub programfiles: PathBuf,
    pub programfiles_x86: PathBuf,
    pub temp: PathBuf,
    pub programs: PathBuf,
    pub desktop: PathBuf,
    pub startup: PathBuf,
    pub send_to: PathBuf,
    pub common_programs: PathBuf,
    pub common_desktop: PathBuf,
    pub common_startup: PathBuf,
    /// `HKCU\\<registry_prefix>` holds every hive the engine touches.
    pub registry_prefix: String,
    /// The JSON file that stands in for the Windows Firewall policy.
    pub firewall_store: PathBuf,
    runs: u32,
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = Command::new("reg.exe")
            .args(["delete", &format!("HKCU\\{}", self.registry_prefix), "/f"])
            .output();
    }
}

/// An interactive run in progress, and the log it is writing.
pub struct Started {
    pub child: std::process::Child,
    pub log: PathBuf,
}

impl Started {
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Waits for the run to end and reads what it printed.
    pub fn finish(self) -> Run {
        let log = self.log;
        let output = self.child.wait_with_output().expect("Setup.exe exits");
        Run {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            log,
        }
    }

    /// Whether the run has already ended.
    pub fn ended(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

pub struct Run {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub log: PathBuf,
}

impl Run {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|err| {
            panic!(
                "stdout is not JSON ({err}): {}\nstderr: {}",
                self.stdout, self.stderr
            )
        })
    }

    pub fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn log_has(&self, needle: &str) -> bool {
        self.log_text().lines().any(|line| line.contains(needle))
    }
}

impl Machine {
    /// A user-scope machine.
    pub fn new(name: &str) -> Machine {
        Machine::in_scope(name, Scope::User)
    }

    pub fn in_scope(name: &str, scope: Scope) -> Machine {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = tempfile::Builder::new()
            .prefix(&format!("{name}-"))
            .tempdir_in(TMP)
            .unwrap();
        let folder = |name: &str| {
            let path = dir.path().join(name);
            fs::create_dir_all(&path).unwrap();
            path
        };

        Machine {
            scope,
            localappdata: folder("LocalAppData"),
            programdata: folder("ProgramData"),
            programfiles: folder("ProgramFiles"),
            programfiles_x86: folder("ProgramFilesX86"),
            temp: folder("Temp"),
            programs: folder("Programs"),
            desktop: folder("Desktop"),
            startup: folder("Startup"),
            send_to: folder("SendTo"),
            common_programs: folder("CommonPrograms"),
            common_desktop: folder("CommonDesktop"),
            common_startup: folder("CommonStartup"),
            registry_prefix: format!(
                "Software\\TigerSetupTests\\{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ),
            firewall_store: dir.path().join("Firewall").join("rules.json"),
            _dir: dir,
            runs: 0,
        }
    }

    /// The firewall store this machine's runs write.
    pub fn firewall(&self) -> Store {
        Store::File(self.firewall_store.clone())
    }

    /// Every firewall rule of that name the machine holds.
    pub fn firewall_rules(&self, name: &str) -> Vec<Rule> {
        self.firewall().list(name).unwrap()
    }

    /// The prerequisite's detection directory, as its installer writes it.
    pub fn prereq_dir(&self) -> PathBuf {
        self.programdata.join("TigerSetupTestPrereq")
    }

    /// Where the package's actions leave their evidence on this machine.
    pub fn actions_dir(&self) -> PathBuf {
        self.programdata.join(ACTIONS_DIR_NAME)
    }

    /// A file the package's actions write, by name.
    pub fn action_file(&self, name: &str) -> PathBuf {
        self.actions_dir().join(name)
    }

    /// The lines of the actions' record file, oldest first.
    pub fn action_records(&self) -> Vec<String> {
        fs::read_to_string(self.action_file("record.txt"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// The content of the cache the post-install action builds, if any.
    pub fn action_cache(&self) -> Option<String> {
        fs::read_to_string(self.action_file("cache.txt"))
            .ok()
            .map(|text| text.trim().to_string())
    }

    /// Where the state directory keeps a stored uninstall action's program.
    pub fn stored_action_program(&self, sha256: &str, file_name: &str) -> PathBuf {
        self.state_dir()
            .join("actions")
            .join(sha256)
            .join(file_name)
    }

    /// The environment variable the package sets, as the scope's key holds
    /// it.
    pub fn environment_variable(&self) -> Option<Data> {
        self.read_value(&self.environment_key(), ENVIRONMENT_VARIABLE)
    }

    /// Sets the variable before an install, as a user or another program
    /// might have.
    pub fn seed_environment_variable(&self, data: &Data) {
        let key = KeyPath::parse(&self.environment_key()).unwrap();
        let roots = self.roots();
        tigersetup_engine::win::registry::create_key(&roots, &key).unwrap();
        tigersetup_engine::win::registry::write_value(&roots, &key, ENVIRONMENT_VARIABLE, data)
            .unwrap();
    }

    /// A key under this scope's `Software\\Classes`.
    pub fn classes_key(&self, subkey: &str) -> String {
        self.hive_key(&format!("Software\\Classes\\{subkey}"))
    }

    pub fn app_paths_key(&self) -> String {
        self.hive_key(
            "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\TigerSetupTestApp.exe",
        )
    }

    pub fn capabilities_key(&self) -> String {
        self.hive_key("Software\\IT Tiger\\TigerSetupTestApp\\Capabilities")
    }

    /// The command a handler class runs: `"<install root>\\bin\\<exe>" "%1"`.
    pub fn expected_command(&self) -> String {
        format!(
            "\"{}\" \"%1\"",
            self.install_root()
                .join("bin")
                .join("TigerSetupTestApp.exe")
                .display()
        )
    }

    /// This machine's Startup folder for its scope.
    pub fn startup_folder(&self) -> &PathBuf {
        match self.scope {
            Scope::User => &self.startup,
            Scope::Machine => &self.common_startup,
        }
    }

    pub fn startup_link(&self) -> PathBuf {
        self.startup_folder().join("TigerSetupTestApp Agent.lnk")
    }

    /// The Send To link: user scope only, as the folder is.
    pub fn send_to_link(&self) -> PathBuf {
        self.send_to.join(format!("{PRODUCT_NAME}.lnk"))
    }

    pub fn documentation_link(&self) -> PathBuf {
        self.programs_folder()
            .join("TigerSetupTestApp Documentation.url")
    }

    /// The link as written, or `None` when there is no link there.
    pub fn link(&self, link: &Path) -> Option<tigersetup_engine::win::shortcut::Link> {
        match tigersetup_engine::win::shortcut::inspect(link).unwrap() {
            LinkInspection::Link(link) => Some(link),
            _ => None,
        }
    }

    /// The options `inspect` records for the installation.
    pub fn selected(&mut self, via: &VersionFixture) -> Selected {
        Selected::from_report(&self.inspect(via).json())
    }

    pub fn scope_name(&self) -> &'static str {
        self.scope.as_str()
    }

    pub fn install_root(&self) -> PathBuf {
        match self.scope {
            Scope::User => self.localappdata.join("Programs").join(PRODUCT_NAME),
            Scope::Machine => self.programfiles.join(PRODUCT_NAME),
        }
    }

    /// A destination a person could plausibly type, inside this machine's
    /// sandbox so that a run which reached it would still be contained.
    pub fn chosen_destination(&self) -> String {
        self.programfiles
            .join("Chosen")
            .join(PRODUCT_NAME)
            .display()
            .to_string()
    }

    pub fn state_dir(&self) -> PathBuf {
        match self.scope {
            Scope::User => self.localappdata.join("TigerSetup").join(PRODUCT_ID),
            Scope::Machine => self.programdata.join("TigerSetup").join(PRODUCT_ID),
        }
    }

    /// The product's own key in this machine's scope.
    pub fn product_key(&self) -> String {
        self.hive_key("Software\\IT Tiger\\TigerSetupTestApp")
    }

    /// The parent key TigerSetup creates for the product and removes again.
    pub fn vendor_key(&self) -> String {
        self.hive_key("Software\\IT Tiger")
    }

    pub fn registration_key(&self) -> String {
        self.hive_key(&format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{PRODUCT_ID}"
        ))
    }

    /// The key whose `Path` value this machine's scope extends.
    pub fn environment_key(&self) -> String {
        match self.scope {
            Scope::User => "HKCU\\Environment".to_string(),
            Scope::Machine => {
                "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment".to_string()
            }
        }
    }

    /// This machine's Start Menu Programs folder for its scope.
    pub fn programs_folder(&self) -> &PathBuf {
        match self.scope {
            Scope::User => &self.programs,
            Scope::Machine => &self.common_programs,
        }
    }

    /// This machine's desktop folder for its scope.
    pub fn desktop_folder(&self) -> &PathBuf {
        match self.scope {
            Scope::User => &self.desktop,
            Scope::Machine => &self.common_desktop,
        }
    }

    pub fn hive_key(&self, subkey: &str) -> String {
        let hive = match self.scope {
            Scope::User => "HKCU",
            Scope::Machine => "HKLM",
        };
        format!("{hive}\\{subkey}")
    }

    /// The uninstaller copy the engine keeps beside the state database.
    pub fn uninstaller(&self) -> PathBuf {
        self.state_dir().join("uninstall.exe")
    }

    /// The registry roots of this machine, for reading what a run wrote.
    pub fn roots(&self) -> Roots {
        Roots::relocated(&self.registry_prefix)
    }

    pub fn read_value(&self, key: &str, name: &str) -> Option<Data> {
        tigersetup_engine::win::registry::read_value(
            &self.roots(),
            &KeyPath::parse(key).unwrap(),
            name,
        )
        .unwrap()
    }

    pub fn key_exists(&self, key: &str) -> bool {
        tigersetup_engine::win::registry::key_exists(&self.roots(), &KeyPath::parse(key).unwrap())
            .unwrap()
    }

    /// Deletes a key and everything under it, the way a third party or a
    /// registry cleaner would. The engine has no such operation on purpose —
    /// it never removes a key it did not create, and only when empty — so
    /// this reaches the relocated key with `reg.exe`.
    pub fn delete_key_tree(&self, key: &str) {
        let parsed = KeyPath::parse(key).unwrap();
        let physical = format!(
            "HKCU\\{}\\{}\\{}",
            self.registry_prefix,
            parsed.hive.as_str(),
            parsed.subkey
        );
        let status = Command::new("reg.exe")
            .args(["delete", &physical, "/f"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "reg delete {physical} failed");
        assert!(!self.key_exists(key), "{key} must be gone");
    }

    /// The scope's `Path` value, and whether the value exists at all.
    pub fn path_value(&self) -> (String, bool) {
        tigersetup_engine::resource::path::read(
            &self.roots(),
            &KeyPath::parse(&self.environment_key()).unwrap(),
        )
        .unwrap()
    }

    /// Sets the scope's `Path` before an install, as a lab row seeds it.
    pub fn seed_path(&self, text: &str) {
        let key = KeyPath::parse(&self.environment_key()).unwrap();
        let roots = self.roots();
        tigersetup_engine::win::registry::create_key(&roots, &key).unwrap();
        tigersetup_engine::win::registry::write_value(
            &roots,
            &key,
            "Path",
            &Data::ExpandString(text.to_string()),
        )
        .unwrap();
    }

    /// How many segments of `Path` are the install root's `bin`.
    pub fn path_entry_count(&self) -> usize {
        let wanted = tigersetup_engine::resource::path::normalize(&self.bin_path_entry());
        tigersetup_engine::resource::path::split(&self.path_value().0)
            .iter()
            .filter(|segment| {
                !segment.is_empty()
                    && tigersetup_engine::resource::path::normalize(segment) == wanted
            })
            .count()
    }

    /// The PATH entry the package declares: `<install root>\\bin`.
    pub fn bin_path_entry(&self) -> String {
        self.install_root().join("bin").display().to_string()
    }

    pub fn start_menu_link(&self) -> PathBuf {
        self.programs_folder().join(format!("{PRODUCT_NAME}.lnk"))
    }

    pub fn desktop_link(&self) -> PathBuf {
        self.desktop_folder().join(format!("{PRODUCT_NAME}.lnk"))
    }

    /// What a `.lnk` points at, or `None` when there is no link there.
    pub fn link_target(&self, link: &Path) -> Option<String> {
        match tigersetup_engine::win::shortcut::inspect(link).unwrap() {
            LinkInspection::Link(link) => Some(link.target),
            _ => None,
        }
    }

    /// `installer` with `args` plus `--json`, and `--log <fresh path>` for a
    /// mutating command, with every root the engine resolves redirected into
    /// this machine. The command is not started yet.
    fn command(&mut self, installer: &Path, args: &[&str]) -> (Command, PathBuf) {
        self.runs += 1;
        let log = self.localappdata.join(format!("run-{:02}.log", self.runs));
        let mut command = Command::new(installer);
        command.args(args).arg("--json");
        if matches!(args.first(), Some(&"install" | &"uninstall" | &"repair")) {
            command.arg("--log").arg(&log);
        }
        command
            .env("TEMP", &self.temp)
            .env("TMP", &self.temp)
            // The engine resolves known folders through the seams below.
            // The plain variables are redirected too, because a process the
            // engine starts — a dependency's installer, a legacy
            // uninstaller — reads its own environment and must see the same
            // isolated machine.
            .env("LOCALAPPDATA", &self.localappdata)
            .env("PROGRAMDATA", &self.programdata)
            .env("PROGRAMFILES", &self.programfiles)
            .env("PROGRAMFILES(X86)", &self.programfiles_x86)
            .env("TIGERSETUP_TEST_REGISTRY_ROOT", &self.registry_prefix)
            .env("TIGERSETUP_TEST_FOLDER_LOCALAPPDATA", &self.localappdata)
            .env("TIGERSETUP_TEST_FOLDER_PROGRAMDATA", &self.programdata)
            .env("TIGERSETUP_TEST_FOLDER_PROGRAMFILES", &self.programfiles)
            .env(
                "TIGERSETUP_TEST_FOLDER_PROGRAMFILESX86",
                &self.programfiles_x86,
            )
            .env("TIGERSETUP_TEST_FOLDER_PROGRAMS", &self.programs)
            .env("TIGERSETUP_TEST_FOLDER_DESKTOP", &self.desktop)
            .env("TIGERSETUP_TEST_FOLDER_STARTUP", &self.startup)
            .env("TIGERSETUP_TEST_FOLDER_SENDTO", &self.send_to)
            .env(
                "TIGERSETUP_TEST_FOLDER_COMMONPROGRAMS",
                &self.common_programs,
            )
            .env("TIGERSETUP_TEST_FOLDER_COMMONDESKTOP", &self.common_desktop)
            .env("TIGERSETUP_TEST_FOLDER_COMMONSTARTUP", &self.common_startup)
            .env(
                tigersetup_engine::win::firewall::TEST_STORE_VARIABLE,
                &self.firewall_store,
            );
        (command, log)
    }

    /// Runs `installer` to completion and reads its documents.
    pub fn run(&mut self, installer: &Path, args: &[&str]) -> Run {
        let (mut command, log) = self.command(installer, args);
        let output = command.output().expect("Setup.exe starts");
        Run {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            log,
        }
    }

    /// Starts `installer` without waiting: an interactive run a test drives
    /// through its window before reading the documents it finally prints.
    pub fn start(&mut self, installer: &Path, args: &[&str]) -> Started {
        let (mut command, log) = self.command(installer, args);
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Setup.exe starts");
        Started { child, log }
    }

    pub fn install(&mut self, version: &VersionFixture) -> Run {
        let scope = self.scope_name();
        self.run(
            &version.installer,
            &["install", "--quiet", "--scope", scope],
        )
    }

    pub fn install_with_faults(&mut self, version: &VersionFixture, faults: &[&str]) -> Run {
        let mut args = vec!["install", "--quiet", "--scope", self.scope_name()];
        for fault in faults {
            args.push("--fault");
            args.push(fault);
        }
        self.run(&version.installer, &args)
    }

    /// Installs with faults that announce the boundary they reach into
    /// `signal`, the way a lab row interrupts a run at a boundary.
    pub fn install_with_faults_signalling(
        &mut self,
        version: &VersionFixture,
        faults: &[&str],
        signal: &Path,
    ) -> Run {
        let signal = signal.display().to_string();
        let mut args = vec!["install", "--quiet", "--scope", self.scope_name()];
        for fault in faults {
            args.push("--fault");
            args.push(fault);
        }
        args.push("--fault-signal");
        args.push(&signal);
        self.run(&version.installer, &args)
    }

    /// A path inside this machine's sandbox, for a file a test writes or
    /// watches for.
    pub fn scratch_path(&self, name: &str) -> PathBuf {
        self.temp.join(name)
    }

    pub fn uninstall(&mut self, version: &VersionFixture) -> Run {
        let scope = self.scope_name();
        self.run(
            &version.installer,
            &["uninstall", "--quiet", "--scope", scope],
        )
    }

    pub fn verify(&mut self, version: &VersionFixture) -> Run {
        let scope = self.scope_name();
        self.run(&version.installer, &["verify", "--scope", scope])
    }

    pub fn inspect(&mut self, version: &VersionFixture) -> Run {
        let scope = self.scope_name();
        self.run(&version.installer, &["inspect", "--scope", scope])
    }

    /// An install with explicit option values, as `--option <name> <value>`.
    pub fn install_with_options(
        &mut self,
        version: &VersionFixture,
        options: &[(&str, &str)],
    ) -> Run {
        let mut args = vec!["install", "--quiet", "--scope", self.scope_name()];
        for (name, value) in options {
            args.push("--option");
            args.push(name);
            args.push(value);
        }
        self.run(&version.installer, &args)
    }

    pub fn repair(&mut self, version: &VersionFixture) -> Run {
        let scope = self.scope_name();
        self.run(&version.installer, &["repair", "--quiet", "--scope", scope])
    }

    /// Uninstalls the way Add/Remove Programs does: through the copy the
    /// engine keeps in the state directory, which has to remove itself.
    pub fn uninstall_through_the_copy(&mut self) -> Run {
        let uninstaller = self.uninstaller();
        assert!(uninstaller.exists(), "{} must exist", uninstaller.display());
        self.run(&uninstaller, &["uninstall", "--quiet"])
    }

    /// `verify` is `ok` for exactly `version`'s file set, nothing of a
    /// transaction remains, and every byte on disk is `version`'s.
    pub fn assert_verified(&mut self, version: &VersionFixture) {
        let run = self.verify(version);
        let report = run.json();
        assert_eq!(
            report["status"],
            "ok",
            "verify: {}\nlog: {}",
            run.stdout,
            run.log_text()
        );
        assert_eq!(run.exit_code, Some(0));
        let selected = self.selected(version);
        let expected = version.expected_files(&selected).len() as u64;
        assert_eq!(report["counts"]["files_checked"], expected, "{report}");
        assert_eq!(report["counts"]["files_ok"], expected);
        assert_eq!(report["installation"]["version"], version.version);
        assert!(
            temp_files_under(&self.install_root()).is_empty(),
            "no temporaries remain"
        );
        assert!(
            staging_dirs(&self.state_dir()).is_empty(),
            "no staging directories remain"
        );
        self.assert_disk_is_exactly_for(version, &selected);
        self.assert_resources_present_for(version, &selected);
    }

    pub fn assert_absent(&mut self, version: &VersionFixture) {
        let run = self.verify(version);
        assert_eq!(run.json()["status"], "not_installed", "{}", run.stdout);
        assert_eq!(run.exit_code, Some(1));
        assert!(
            !self.install_root().exists(),
            "install root {} must be gone",
            self.install_root().display()
        );
        assert!(
            staging_dirs(&self.state_dir()).is_empty(),
            "no staging directories remain"
        );
        assert!(
            !self.key_exists(&self.registration_key()),
            "the Add/Remove Programs registration must be gone"
        );
        assert!(
            !self.key_exists(&self.product_key()),
            "the product key must be gone"
        );
        assert_eq!(self.path_entry_count(), 0, "no PATH entry may remain");
        assert_eq!(
            self.tools_path_entry_count(),
            0,
            "no tools PATH entry may remain"
        );
        assert!(
            !self.start_menu_link().exists(),
            "the Start Menu link must be gone"
        );
        assert!(!self.desktop_link().exists(), "no desktop link may remain");
        assert!(!self.startup_link().exists(), "no Startup link may remain");
        assert!(!self.send_to_link().exists(), "no Send To link may remain");
        assert!(
            !self.documentation_link().exists(),
            "the documentation shortcut must be gone"
        );
        assert_eq!(
            self.environment_variable(),
            None,
            "the environment variable TigerSetup created must be gone"
        );
        assert!(
            self.firewall_rules(FIREWALL_RULE).is_empty(),
            "the firewall rule must be gone"
        );
        for key in [
            self.classes_key(PROG_ID),
            self.classes_key(&format!("{EXTENSION}\\OpenWithProgids")),
            self.classes_key(SCHEME),
            self.classes_key(&format!("*\\shell\\{FILES_VERB}")),
            self.classes_key(&format!("Directory\\Background\\shell\\{BACKGROUND_VERB}")),
            self.app_paths_key(),
            self.capabilities_key(),
        ] {
            assert!(!self.key_exists(&key), "{key} must be gone");
        }
        assert_eq!(
            self.read_value(
                &self.hive_key("Software\\RegisteredApplications"),
                PRODUCT_NAME
            ),
            None,
            "the capability registration must be gone"
        );
    }

    /// Everything `assert_absent` checks, plus TigerSetup's own state
    /// directory being gone: the state after an uninstall, as distinct from a
    /// rolled-back install, which is equally "not installed" but keeps the
    /// database that recorded the attempt.
    pub fn assert_uninstalled(&mut self, version: &VersionFixture) {
        self.assert_absent(version);
        assert!(
            !self.state_dir().exists(),
            "an uninstalled product leaves no state directory: {} still holds {:?}",
            self.state_dir().display(),
            files_under(&self.state_dir())
        );
    }

    /// Every resource the package declares for `version` and the recorded
    /// options is on the machine, with the data the metadata asks for.
    pub fn assert_resources_present(&mut self, version: &VersionFixture) {
        let selected = self.selected(version);
        self.assert_resources_present_for(version, &selected);
    }

    /// Every optional resource follows its option: present with the data
    /// the metadata asks for when the option enables it, absent otherwise.
    pub fn assert_optional_resources(&self, selected: &Selected) {
        let root = self.install_root();
        let exe = root.join("bin").join("TigerSetupTestApp.exe");
        // PATH: the mode decides which directories are on it.
        let (bin, tools) = match selected.path_mode() {
            "none" => (0, 0),
            "tools" => (1, 1),
            _ => (1, 0),
        };
        assert_eq!(
            self.path_entry_count(),
            bin,
            "bin on PATH for mode {}",
            selected.path_mode()
        );
        assert_eq!(
            self.tools_path_entry_count(),
            tools,
            "tools on PATH for mode {}",
            selected.path_mode()
        );
        // Shortcuts.
        assert_eq!(
            self.desktop_link().exists(),
            selected.on("desktop-shortcut")
        );
        assert_eq!(
            self.startup_link().exists(),
            selected.on("startup"),
            "the Startup link follows its option"
        );
        if selected.on("startup") {
            let link = self
                .link(&self.startup_link())
                .expect("the Startup link reads back");
            assert_eq!(link.arguments, "--agent");
        }
        let send_to_expected = selected.on("send-to") && self.scope == Scope::User;
        assert_eq!(
            self.send_to_link().exists(),
            send_to_expected,
            "the Send To link follows its option in user scope only"
        );
        let documentation = self
            .link(&self.documentation_link())
            .expect("the documentation shortcut exists");
        assert_eq!(documentation.target, DOCUMENTATION_URL);
        // Environment variable.
        if selected.on("environment") {
            assert_eq!(
                self.environment_variable(),
                Some(Data::ExpandString(root.display().to_string()))
            );
        }
        // Integrations.
        let association = self.read_value(
            &self.classes_key(&format!("{PROG_ID}\\shell\\open\\command")),
            "",
        );
        let open_with = self.read_value(
            &self.classes_key(&format!("{EXTENSION}\\OpenWithProgids")),
            PROG_ID,
        );
        if selected.on("file-association") {
            assert_eq!(association, Some(Data::String(self.expected_command())));
            assert_eq!(
                open_with,
                Some(Data::String(String::new())),
                "the extension lists the ProgID in Open With"
            );
            assert_eq!(
                self.read_value(&self.capabilities_key(), "ApplicationName"),
                Some(Data::String(PRODUCT_NAME.into()))
            );
            assert_eq!(
                self.read_value(
                    &format!("{}\\FileAssociations", self.capabilities_key()),
                    EXTENSION
                ),
                Some(Data::String(PROG_ID.into()))
            );
        } else {
            assert_eq!(association, None, "no association when the option is off");
            assert_eq!(open_with, None);
        }
        // The handler ProgID is always TigerSetup's; the scheme's own class
        // key is TigerSetup's only where nothing else owned it, so it is
        // checked as ours only when it reads as ours.
        let handler = self.classes_key(&format!("TigerSetupTestApp.{SCHEME}"));
        let handler_command = self.read_value(&format!("{handler}\\shell\\open\\command"), "");
        let scheme_command = self.read_value(
            &self.classes_key(&format!("{SCHEME}\\shell\\open\\command")),
            "",
        );
        if selected.on("url-protocol") {
            assert_eq!(handler_command, Some(Data::String(self.expected_command())));
            assert_eq!(
                self.read_value(&handler, "URL Protocol"),
                Some(Data::String(String::new())),
                "the handler ProgID carries URL Protocol"
            );
            assert_eq!(
                self.read_value(
                    &format!("{}\\URLAssociations", self.capabilities_key()),
                    SCHEME
                ),
                Some(Data::String(format!("TigerSetupTestApp.{SCHEME}")))
            );
            if scheme_command == Some(Data::String(self.expected_command())) {
                assert_eq!(
                    self.read_value(&self.classes_key(SCHEME), "URL Protocol"),
                    Some(Data::String(String::new()))
                );
            } else {
                assert!(
                    scheme_command.is_some(),
                    "the scheme class is ours, or a stranger's that pre-existed"
                );
            }
        } else {
            assert_eq!(handler_command, None, "no handler when the option is off");
            assert_ne!(
                scheme_command,
                Some(Data::String(self.expected_command())),
                "no scheme class of ours when the option is off"
            );
        }
        assert_eq!(
            self.read_value(&self.app_paths_key(), ""),
            Some(Data::String(exe.display().to_string())),
            "App Paths names the executable"
        );
        assert_eq!(
            self.read_value(&self.app_paths_key(), "Path"),
            Some(Data::String(root.join("bin").display().to_string()))
        );
        let files_verb = self.read_value(
            &self.classes_key(&format!("*\\shell\\{FILES_VERB}\\command")),
            "",
        );
        let background_verb = self.read_value(
            &self.classes_key(&format!(
                "Directory\\Background\\shell\\{BACKGROUND_VERB}\\command"
            )),
            "",
        );
        if selected.on("context-menu") {
            assert_eq!(files_verb, Some(Data::String(self.expected_command())));
            assert_eq!(
                background_verb,
                Some(Data::String(format!("\"{}\" \"%V\"", exe.display())))
            );
            assert_eq!(
                self.read_value(
                    &self.classes_key(&format!("*\\shell\\{FILES_VERB}")),
                    "Icon"
                ),
                Some(Data::String(exe.display().to_string()))
            );
        } else {
            assert_eq!(files_verb, None);
            assert_eq!(background_verb, None);
        }
        // Firewall.
        let rules = self.firewall_rules(FIREWALL_RULE);
        if selected.on("firewall") {
            assert_eq!(rules.len(), 1, "one firewall rule");
            assert!(
                rules[0]
                    .program
                    .eq_ignore_ascii_case(&exe.display().to_string())
            );
            assert_eq!(rules[0].grouping, format!("TigerSetup: {PRODUCT_NAME}"));
            assert_eq!(rules[0].local_ports, "47110");
            assert_eq!(rules[0].protocol, "tcp");
            assert!(rules[0].enabled);
        } else {
            assert!(rules.is_empty(), "no firewall rule when the option is off");
        }
        // The prerequisite was installed by the embedded installer, or was
        // already there; either way it is detected now.
        assert!(
            self.prereq_dir().join("1.0.0").join("prereq.txt").exists(),
            "the embedded prerequisite is installed"
        );
        // What the package's actions wrote is deliberately not asserted
        // here: it is the programs' side effect, which a rolled-back run
        // leaves as it was, and the action tests assert it where it is
        // the point.
    }

    fn assert_resources_present_for(&self, version: &VersionFixture, selected: &Selected) {
        self.assert_optional_resources(selected);
        assert_eq!(
            self.read_value(&self.product_key(), "InstallRoot"),
            Some(Data::ExpandString(
                self.install_root().display().to_string()
            )),
            "the product registry value"
        );
        assert_eq!(
            self.read_value(&self.product_key(), "Version"),
            Some(Data::String(version.version.to_string()))
        );
        assert_eq!(
            self.read_value(&self.registration_key(), "DisplayName"),
            Some(Data::String(PRODUCT_NAME.into()))
        );
        assert_eq!(
            self.read_value(&self.registration_key(), "DisplayVersion"),
            Some(Data::String(version.version.into()))
        );
        assert_eq!(
            self.read_value(&self.registration_key(), "InstallLocation"),
            Some(Data::String(format!("{}\\", self.install_root().display()))),
            "InstallLocation ends in a backslash"
        );
        assert_eq!(
            self.read_value(&self.registration_key(), "UninstallString"),
            Some(Data::String(format!(
                "\"{}\"",
                self.uninstaller().display()
            )))
        );
        assert_eq!(
            self.read_value(&self.registration_key(), "QuietUninstallString"),
            Some(Data::String(format!(
                "\"{}\" uninstall --quiet",
                self.uninstaller().display()
            )))
        );
        assert_eq!(
            self.read_value(&self.registration_key(), "NoModify"),
            Some(Data::Dword(1))
        );
        assert_eq!(
            self.read_value(&self.registration_key(), "DisplayIcon"),
            Some(Data::String(
                self.install_root()
                    .join("bin")
                    .join("TigerSetupTestApp.exe")
                    .display()
                    .to_string()
            ))
        );
        // The engine compares a shortcut's target case-insensitively
        // (`verify` in the engine uses `eq_ignore_ascii_case`), because
        // Windows canonicalises the drive letter of a link's target when it
        // saves the `.lnk` — so a link written from a lower-case-drive
        // install root reads back with an upper-case drive. The assertion
        // holds the same contract rather than the exact byte case, which
        // otherwise depends only on the drive-letter case the build happened
        // to bake into the temporary directory.
        let expected_target = self
            .install_root()
            .join("bin")
            .join("TigerSetupTestApp.exe")
            .display()
            .to_string();
        let start_menu = self.link(&self.start_menu_link());
        let actual_target = start_menu.as_ref().map(|link| link.target.clone());
        assert!(
            actual_target
                .as_deref()
                .is_some_and(|target| target.eq_ignore_ascii_case(&expected_target)),
            "the Start Menu link points at the application: {actual_target:?} != {expected_target:?}"
        );
        let start_menu = start_menu.unwrap();
        assert_eq!(start_menu.app_user_model_id, APP_USER_MODEL_ID);
        assert!(
            start_menu
                .working_directory
                .eq_ignore_ascii_case(&self.install_root().join("data").display().to_string()),
            "the declared working directory is written: {:?}",
            start_menu.working_directory
        );
    }

    /// The files under the install root are exactly `version`'s core set
    /// plus the components the recorded options select, and each one's
    /// SHA-256 equals the SHA-256 of `version`'s payload file.
    pub fn assert_disk_is_exactly(&mut self, version: &VersionFixture) {
        let selected = self.selected(version);
        self.assert_disk_is_exactly_for(version, &selected);
    }

    pub fn assert_disk_is_exactly_for(&self, version: &VersionFixture, selected: &Selected) {
        let on_disk = files_under(&self.install_root());
        assert_eq!(
            on_disk,
            version.expected_paths(selected),
            "the install root must hold exactly the {} file set for {:?}",
            version.version,
            selected.values
        );
        for relative in &version.expected_files(selected) {
            let installed = sha256_hex(&fs::read(self.install_root().join(relative)).unwrap());
            assert_eq!(
                installed,
                version.payload_sha256(relative),
                "{relative} must carry the {} content",
                version.version
            );
        }
    }

    /// How many segments of `Path` are the install root's `tools`.
    pub fn tools_path_entry_count(&self) -> usize {
        let wanted = tigersetup_engine::resource::path::normalize(
            &self.install_root().join("tools").display().to_string(),
        );
        tigersetup_engine::resource::path::split(&self.path_value().0)
            .iter()
            .filter(|segment| {
                !segment.is_empty()
                    && tigersetup_engine::resource::path::normalize(segment) == wanted
            })
            .count()
    }

    /// `inspect` reports exactly one installation, of `version`.
    pub fn assert_inspect_version(&mut self, via: &VersionFixture, version: &str) -> Value {
        let report = self.inspect(via).json();
        assert_eq!(report["installation"]["version"], version, "{report}");
        assert!(report["transaction"].is_null(), "{report}");
        report
    }
}

pub fn temp_files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.to_string_lossy().ends_with(".tigersetup-new") {
                out.push(path);
            }
        }
    }
    out
}

/// Every file under `root`, install-relative with backslashes.
pub fn files_under(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                out.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('/', "\\"),
                );
            }
        }
    }
    out
}

pub fn staging_dirs(state_dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(state_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir() && p.file_name().unwrap().to_string_lossy().starts_with("txn-")
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The install-relative target the fault fired on, from the log.
pub fn faulted_target(run: &Run) -> String {
    let text = run.log_text();
    let line = text
        .lines()
        .find(|line| line.contains("[fault_injected]"))
        .unwrap_or_else(|| panic!("no fault_injected line in {text}"));
    line.split(" target=")
        .nth(1)
        .expect("target= in fault line")
        .trim()
        .to_string()
}

/// The journal sequence of the operation `kind` on `target`, from the
/// `operation_applied` lines of a completed run's log.
pub fn sequence_of(run: &Run, kind: &str, target: &str) -> i64 {
    sequence_in_log(&run.log_text(), kind, target)
}

/// As [`sequence_of`], on log text kept from an earlier run.
pub fn sequence_in_log(text: &str, kind: &str, target: &str) -> i64 {
    let needle = format!("kind={kind} target={target}");
    let line = text
        .lines()
        .find(|line| line.contains("[operation_applied]") && line.ends_with(&needle))
        .unwrap_or_else(|| panic!("no applied operation {needle} in {text}"));
    line.split(" sequence=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("sequence= in operation line")
}

/// The journal sequence of the first applied operation of `kind`, whatever
/// it targets: the plan holds one operation of each resource kind whose
/// target is a machine-specific path.
pub fn first_sequence_of_kind(text: &str, kind: &str) -> i64 {
    let needle = format!("kind={kind} target=");
    let line = text
        .lines()
        .find(|line| line.contains("[operation_applied]") && line.contains(&needle))
        .unwrap_or_else(|| panic!("no applied operation of kind {kind} in {text}"));
    line.split(" sequence=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("sequence= in operation line")
}

/// `(code, path)` pairs of a document's `findings[]`.
pub fn findings_of(document: &Value) -> Vec<(String, String)> {
    document["findings"]
        .as_array()
        .map(|findings| {
            findings
                .iter()
                .map(|f| {
                    (
                        f["code"].as_str().unwrap_or_default().to_string(),
                        f["path"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
