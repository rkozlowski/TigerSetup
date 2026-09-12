//! The WinGet manifest set a built installer produces, checked field by
//! field against the shape a published community submission has: a version
//! manifest, an installer manifest and an `en-US` locale manifest at schema
//! 1.12.0, with the installer URL left unresolved until the bytes are
//! published and filled in from the exact published bytes afterwards.

mod common;

use std::path::Path;

use common::built_installer;
use tigersetup_build::winget;

/// A package declaration with everything a community submission needs: two
/// scopes, an explicit Add/Remove Programs key, catalog-backed dependencies
/// and the distribution values that live nowhere else.
const MANIFEST: &str = r#"
[package]
id = "ItTiger.SampleViewer"
name = "SampleViewer"
version = "0.8.1"
publisher = "IT Tiger"
description = "A local Markdown viewer, reviewer and PDF exporter."
copyright = "Copyright (c) 2026 Ryszard Kozlowski"
license = "MIT"
website = "https://github.com/example/SampleViewer"
support = "https://github.com/example/SampleViewer/issues"

[install]
scopes = ["machine", "user"]
minimum_build = 17763

[[files]]
source = "payload/**"

[[options]]
name = "path"
kind = "path"
default = true

[[path]]
entry = "."
option = "path"

[registration]
key_name = "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"
display_name = "SampleViewer"

[[dependencies]]
id = "Microsoft.DotNet.DesktopRuntime.10"
minimum = "10.0"
detect = { kind = "directory-version", path = "%PROGRAMFILES%\\dotnet\\shared\\Microsoft.WindowsDesktop.App", pattern = "10.*" }

[[dependencies]]
id = "Microsoft.EdgeWebView2Runtime"
detect = { kind = "registry-version", keys = ["HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"], value = "pv" }

[[dependencies]]
id = "Vendor.Custom"
detect = { kind = "file-version", path = "%PROGRAMFILES%\\Vendor\\vendor.dll" }
acquire = { url = "https://example.invalid/vendor.exe", sha256 = "0000000000000000000000000000000000000000000000000000000000000000" }
install = { arguments = ["/S"] }

[winget]
moniker = "sample-viewer"
commands = ["sample-view"]
tags = ["markdown", "pdf", "viewer"]
publisher_url = "https://www.ittiger.net/"
license_url = "https://github.com/example/SampleViewer/blob/v0.8.1/LICENSE"
release_notes_url = "https://github.com/example/SampleViewer/releases/tag/v0.8.1"
documentation_url = "https://github.com/example/SampleViewer/blob/v0.8.1/docs/HELP.md"
description = """
SampleViewer is a Windows desktop application for reading and reviewing local Markdown files.
The installer includes the graphical viewer and the sample-view command."""
"#;

/// A YAML document read back the way a submission reviewer reads it: line
/// by line, without a parser the builder does not ship.
struct Manifest {
    lines: Vec<String>,
}

impl Manifest {
    fn load(path: &Path) -> Manifest {
        let text = std::fs::read_to_string(path).unwrap();
        assert!(
            text.ends_with('\n'),
            "{} has no final newline",
            path.display()
        );
        Manifest {
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    /// Every value written for a key, at any indentation, unquoted.
    fn values(&self, key: &str) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|line| {
                let trimmed = line.trim_start_matches([' ', '-']);
                trimmed
                    .strip_prefix(&format!("{key}: "))
                    .map(|value| unquote(value.trim()))
            })
            .collect()
    }

    fn value(&self, key: &str) -> String {
        let values = self.values(key);
        assert_eq!(values.len(), 1, "{key} appears {} times", values.len());
        values.into_iter().next().unwrap()
    }

    /// The `- item` entries directly under a `key:` line at the same
    /// indentation.
    fn sequence(&self, key: &str) -> Vec<String> {
        let start = self
            .lines
            .iter()
            .position(|line| line.trim_start() == format!("{key}:"))
            .unwrap_or_else(|| panic!("no {key}: block"));
        let indent = self.lines[start].len() - self.lines[start].trim_start().len();
        self.lines[start + 1..]
            .iter()
            .take_while(|line| {
                let own = line.len() - line.trim_start().len();
                own == indent && line.trim_start().starts_with("- ")
            })
            .map(|line| unquote(line.trim_start().trim_start_matches("- ").trim()))
            .collect()
    }

    fn has_line(&self, text: &str) -> bool {
        self.lines.iter().any(|line| line == text)
    }
}

fn unquote(value: &str) -> String {
    match value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => value.to_string(),
    }
}

#[test]
fn a_prepared_manifest_set_has_the_shape_a_submission_needs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let installer = built_installer(root, MANIFEST);
    let output = root.join("winget");
    let result = winget::prepare(&root.join("TigerSetup.toml"), &installer, &output).unwrap();
    assert_eq!(result.identifier, "ItTiger.SampleViewer");
    assert_eq!(result.version, "0.8.1");
    assert_eq!(
        result.files,
        vec![
            output.join("ItTiger.SampleViewer.yaml"),
            output.join("ItTiger.SampleViewer.installer.yaml"),
            output.join("ItTiger.SampleViewer.locale.en-US.yaml"),
        ]
    );

    let version = Manifest::load(&result.files[0]);
    assert!(version.has_line(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.12.0.schema.json"
    ));
    assert_eq!(version.value("PackageIdentifier"), "ItTiger.SampleViewer");
    assert_eq!(version.value("PackageVersion"), "0.8.1");
    assert_eq!(version.value("DefaultLocale"), "en-US");
    assert_eq!(version.value("ManifestType"), "version");
    assert_eq!(version.value("ManifestVersion"), "1.12.0");

    let manifest = Manifest::load(&result.files[1]);
    assert!(manifest.has_line(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.12.0.schema.json"
    ));
    let identifiers = manifest.values("PackageIdentifier");
    assert_eq!(identifiers[0], "ItTiger.SampleViewer");
    assert_eq!(manifest.value("PackageVersion"), "0.8.1");
    assert_eq!(manifest.value("InstallerLocale"), "en-US");
    assert_eq!(manifest.sequence("Platform"), ["Windows.Desktop"]);
    assert_eq!(manifest.value("MinimumOSVersion"), "10.0.17763.0");
    assert_eq!(manifest.values("InstallerType"), ["exe", "exe", "exe"]);
    assert_eq!(
        manifest.sequence("InstallModes"),
        ["interactive", "silent", "silentWithProgress"]
    );
    assert_eq!(manifest.value("UpgradeBehavior"), "install");
    assert_eq!(manifest.sequence("Commands"), ["sample-view"]);

    // Only the catalog-backed dependencies are package dependencies; the one
    // acquired from a fixed URL has no WinGet identity.
    assert_eq!(
        identifiers[1..],
        [
            "Microsoft.DotNet.DesktopRuntime.10",
            "Microsoft.EdgeWebView2Runtime"
        ]
    );

    // Success is never an expected return code; everything else is mapped.
    let codes = manifest.values("InstallerReturnCode");
    assert_eq!(codes, ["1", "2", "3", "4", "5", "6", "7", "8", "3010"]);
    assert_eq!(
        manifest.values("ReturnResponse"),
        [
            "custom",
            "invalidParameter",
            "missingDependency",
            "custom",
            "cancelledByUser",
            "packageInUseByApplication",
            "custom",
            "systemNotSupported",
            "rebootRequiredToFinish"
        ]
    );

    // One installer entry per declared scope, in declaration order.
    assert_eq!(manifest.values("Architecture"), ["x64", "x64"]);
    assert_eq!(manifest.values("Scope"), ["machine", "user"]);
    assert_eq!(
        manifest.values("InstallerUrl"),
        [
            "https://UNRESOLVED/SampleViewer-0.8.1-Setup.exe",
            "https://UNRESOLVED/SampleViewer-0.8.1-Setup.exe"
        ]
    );
    let expected_hash = winget::file_sha256(&installer).unwrap();
    assert_eq!(
        manifest.values("InstallerSha256"),
        [expected_hash.clone(), expected_hash.clone()]
    );
    assert_eq!(expected_hash, expected_hash.to_ascii_uppercase());
    assert_eq!(
        manifest.values("Silent"),
        [
            "install --quiet --scope machine",
            "install --quiet --scope user"
        ]
    );
    assert_eq!(
        manifest.values("SilentWithProgress"),
        [
            "install --quiet --scope machine",
            "install --quiet --scope user"
        ]
    );
    assert_eq!(
        manifest.values("Log"),
        ["--log \"<LOGPATH>\"", "--log \"<LOGPATH>\""]
    );
    assert_eq!(
        manifest.values("InstallLocation"),
        [
            "--install-root \"<INSTALLPATH>\"",
            "--install-root \"<INSTALLPATH>\""
        ]
    );
    assert_eq!(
        manifest.values("ProductCode"),
        [
            "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1",
            "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1",
            "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1",
            "{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"
        ]
    );
    assert!(
        manifest.has_line("  ProductCode: '{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1'"),
        "a product code opening with a brace has to be quoted"
    );
    assert_eq!(
        manifest.values("DisplayName"),
        ["SampleViewer", "SampleViewer"]
    );
    assert_eq!(manifest.values("Publisher"), ["IT Tiger", "IT Tiger"]);
    // The registration carries no display version of its own, so the package
    // version speaks for it and none is written.
    assert!(manifest.values("DisplayVersion").is_empty());
    assert_eq!(manifest.value("ManifestType"), "installer");

    let locale = Manifest::load(&result.files[2]);
    assert!(locale.has_line(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.12.0.schema.json"
    ));
    assert_eq!(locale.value("PackageLocale"), "en-US");
    assert_eq!(locale.value("Publisher"), "IT Tiger");
    assert_eq!(locale.value("PublisherUrl"), "https://www.ittiger.net/");
    assert_eq!(
        locale.value("PublisherSupportUrl"),
        "https://github.com/example/SampleViewer/issues"
    );
    assert_eq!(locale.value("Author"), "IT Tiger");
    assert_eq!(locale.value("PackageName"), "SampleViewer");
    assert_eq!(
        locale.value("PackageUrl"),
        "https://github.com/example/SampleViewer"
    );
    assert_eq!(locale.value("License"), "MIT");
    assert_eq!(
        locale.value("LicenseUrl"),
        "https://github.com/example/SampleViewer/blob/v0.8.1/LICENSE"
    );
    assert_eq!(
        locale.value("Copyright"),
        "Copyright (c) 2026 Ryszard Kozlowski"
    );
    assert_eq!(
        locale.value("ShortDescription"),
        "A local Markdown viewer, reviewer and PDF exporter."
    );
    assert!(locale.has_line("Description: |-"));
    assert!(locale.has_line(
        "  SampleViewer is a Windows desktop application for reading and reviewing local Markdown files."
    ));
    assert_eq!(locale.value("Moniker"), "sample-viewer");
    assert_eq!(locale.sequence("Tags"), ["markdown", "pdf", "viewer"]);
    assert_eq!(locale.value("DocumentLabel"), "Help");
    assert_eq!(
        locale.value("DocumentUrl"),
        "https://github.com/example/SampleViewer/blob/v0.8.1/docs/HELP.md"
    );
    assert_eq!(
        locale.value("ReleaseNotesUrl"),
        "https://github.com/example/SampleViewer/releases/tag/v0.8.1"
    );
    assert_eq!(locale.value("ManifestType"), "defaultLocale");
    assert_eq!(locale.value("ManifestVersion"), "1.12.0");
}

#[test]
fn a_scope_whose_root_the_package_pins_accepts_no_install_location() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let pinned = MANIFEST.replace(
        "minimum_build = 17763",
        "minimum_build = 17763\nmachine_root = \"%PROGRAMFILES%\\\\IT Tiger\\\\SampleViewer\"",
    );
    let installer = built_installer(root, &pinned);
    let output = root.join("winget");
    let result = winget::prepare(&root.join("TigerSetup.toml"), &installer, &output).unwrap();
    let manifest = Manifest::load(&result.files[1]);
    assert_eq!(manifest.values("Scope"), ["machine", "user"]);
    assert_eq!(
        manifest.values("InstallLocation"),
        ["--install-root \"<INSTALLPATH>\""],
        "only the scope that uses the default root takes an override"
    );
}

#[test]
fn an_installer_the_manifest_does_not_describe_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let installer = built_installer(root, MANIFEST);
    let renamed = root.join("Renamed.toml");
    std::fs::write(
        &renamed,
        MANIFEST.replace("name = \"SampleViewer\"", "name = \"OtherViewer\""),
    )
    .unwrap();
    let err = winget::prepare(&renamed, &installer, &root.join("winget")).unwrap_err();
    assert_eq!(err.code, "winget_manifest_mismatch");
    assert!(err.message.contains("OtherViewer"), "{}", err.message);
}

#[test]
fn finalize_publishes_the_exact_bytes_and_can_be_run_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let installer = built_installer(root, MANIFEST);
    let output = root.join("winget");
    winget::prepare(&root.join("TigerSetup.toml"), &installer, &output).unwrap();

    let url = "https://github.com/example/SampleViewer/releases/download/v0.8.1/SampleViewer-0.8.1-Setup.exe";
    let first = winget::finalize(&output, url, &installer).unwrap();
    assert_eq!(first.entries, 2);
    assert_eq!(first.version, "0.8.1");
    assert_eq!(
        first.installer_sha256,
        winget::file_sha256(&installer).unwrap()
    );
    let after_first = std::fs::read_to_string(&first.manifest_path).unwrap();

    let manifest = Manifest::load(&first.manifest_path);
    assert_eq!(manifest.values("InstallerUrl"), [url, url]);
    assert_eq!(
        manifest.values("InstallerSha256"),
        [
            first.installer_sha256.clone(),
            first.installer_sha256.clone()
        ]
    );

    let second = winget::finalize(&output, url, &installer).unwrap();
    assert_eq!(
        std::fs::read_to_string(&second.manifest_path).unwrap(),
        after_first,
        "finalizing an already finalized set changes nothing"
    );
}

#[test]
fn finalize_refuses_a_url_or_an_installer_that_is_not_what_was_prepared() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let installer = built_installer(root, MANIFEST);
    let output = root.join("winget");
    winget::prepare(&root.join("TigerSetup.toml"), &installer, &output).unwrap();

    let wrong_name = winget::finalize(
        &output,
        "https://example.invalid/releases/SampleViewer-0.8.1-setup.exe",
        &installer,
    )
    .unwrap_err();
    assert_eq!(wrong_name.code, "winget_manifest_mismatch");
    assert!(
        wrong_name.message.contains("SampleViewer-0.8.1-setup.exe"),
        "{}",
        wrong_name.message
    );

    // An installer of another version is the wrong bytes for this set, and
    // no amount of rebuilding is allowed to fix that here.
    let other = tempfile::tempdir().unwrap();
    let newer = built_installer(
        other.path(),
        &MANIFEST.replace("version = \"0.8.1\"", "version = \"0.9.0\""),
    );
    let renamed = output.join("SampleViewer-0.8.1-Setup.exe");
    std::fs::copy(&newer, &renamed).unwrap();
    let wrong_version = winget::finalize(
        &output,
        "https://example.invalid/releases/SampleViewer-0.8.1-Setup.exe",
        &renamed,
    )
    .unwrap_err();
    assert_eq!(wrong_version.code, "winget_manifest_mismatch");
    assert!(
        wrong_version.message.contains("0.9.0"),
        "{}",
        wrong_version.message
    );
}
