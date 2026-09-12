//! The merged WinGet manifest: the part of it that describes how to acquire
//! and run an installer. Root-level fields are defaults every installer
//! entry inherits; an entry's own field overrides the root's (switches are
//! merged key by key, lists replace as a whole), which is WinGet's own
//! merging rule.

use std::collections::BTreeMap;

use tigersetup_format::identity::Scope;
use yaml_rust2::{Yaml, YamlLoader};

/// Installer-level values, at the root or on one entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallerFields {
    pub architecture: Option<String>,
    pub installer_type: Option<String>,
    pub scope: Option<String>,
    pub elevation_requirement: Option<String>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub switches: BTreeMap<String, String>,
    pub success_codes: Option<Vec<i64>>,
    /// `(InstallerReturnCode, ReturnResponse)`.
    pub expected_return_codes: Option<Vec<(i64, String)>>,
}

/// A merged manifest, reduced to what acquisition needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedManifest {
    pub package_identifier: String,
    pub package_version: String,
    pub root: InstallerFields,
    pub installers: Vec<InstallerFields>,
}

/// One installer chosen for a requirement, with everything the engine
/// needs to acquire, verify and run it unattended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerEntry {
    pub url: String,
    /// Lower-case hex.
    pub sha256: String,
    /// Lower-case WinGet installer type (`exe`, `burn`, `msi`, ...).
    pub installer_type: String,
    /// `user`, `machine` or empty.
    pub scope: String,
    pub architecture: String,
    /// The entry installs machine-wide or declares that it needs elevation.
    pub elevation_required: bool,
    /// Command-line arguments for an unattended run: the manifest's `Silent`
    /// and `Custom` switches split as a command line. For MSI-type installers
    /// (`msi`, `wix`) only `Custom`: the engine supplies `msiexec`'s own
    /// silent switches.
    pub arguments: Vec<String>,
    /// Exit codes that mean success besides 0.
    pub success_codes: Vec<i32>,
    /// Exit codes that mean success with a reboot pending, from the
    /// manifest's expected return codes and the installer type's defaults.
    pub reboot_codes: Vec<i32>,
}

/// Installer types `msiexec` runs.
pub fn is_msi_type(installer_type: &str) -> bool {
    matches!(installer_type, "msi" | "wix")
}

/// The reboot-related return codes WinGet itself knows for an installer
/// type, independent of any manifest: `ERROR_SUCCESS_REBOOT_REQUIRED`
/// (3010) and `ERROR_SUCCESS_REBOOT_INITIATED` (1641) for Windows Installer
/// and Burn bundles, 8 for Inno Setup.
pub fn default_reboot_codes(installer_type: &str) -> &'static [i32] {
    match installer_type {
        "msi" | "wix" | "burn" => &[3010, 1641],
        "inno" => &[8],
        _ => &[],
    }
}

fn is_reboot_response(response: &str) -> bool {
    matches!(
        response,
        "rebootRequiredToFinish" | "rebootRequiredForInstall" | "rebootInitiated"
    )
}

/// Splits a switch string into arguments the way the C runtime does:
/// whitespace separates, double quotes group, a backslash before a quote
/// escapes it.
pub fn split_command_line(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(current);
    }
    out
}

fn string_of(yaml: &Yaml) -> Option<String> {
    match yaml {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Real(r) => Some(r.clone()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

fn integer_of(yaml: &Yaml) -> Option<i64> {
    match yaml {
        Yaml::Integer(i) => Some(*i),
        Yaml::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn fields_of(node: &Yaml) -> InstallerFields {
    let mut fields = InstallerFields {
        architecture: string_of(&node["Architecture"]),
        installer_type: string_of(&node["InstallerType"]),
        scope: string_of(&node["Scope"]),
        elevation_requirement: string_of(&node["ElevationRequirement"]),
        url: string_of(&node["InstallerUrl"]),
        sha256: string_of(&node["InstallerSha256"]),
        ..Default::default()
    };
    if let Some(switches) = node["InstallerSwitches"].as_hash() {
        for (key, value) in switches {
            if let (Some(key), Some(value)) = (string_of(key), string_of(value)) {
                fields.switches.insert(key, value);
            }
        }
    }
    if let Some(codes) = node["InstallerSuccessCodes"].as_vec() {
        fields.success_codes = Some(codes.iter().filter_map(integer_of).collect());
    }
    if let Some(codes) = node["ExpectedReturnCodes"].as_vec() {
        fields.expected_return_codes = Some(
            codes
                .iter()
                .filter_map(|item| {
                    Some((
                        integer_of(&item["InstallerReturnCode"])?,
                        string_of(&item["ReturnResponse"]).unwrap_or_default(),
                    ))
                })
                .collect(),
        );
    }
    fields
}

/// Parses a merged (or singleton) manifest document.
pub fn parse(text: &str) -> Result<MergedManifest, String> {
    let documents =
        YamlLoader::load_from_str(text).map_err(|err| format!("manifest is not YAML: {err}"))?;
    let root = documents
        .first()
        .filter(|doc| doc.as_hash().is_some())
        .ok_or("manifest has no mapping document")?;
    let installers = root["Installers"]
        .as_vec()
        .map(|entries| entries.iter().map(fields_of).collect())
        .unwrap_or_default();
    Ok(MergedManifest {
        package_identifier: string_of(&root["PackageIdentifier"]).unwrap_or_default(),
        package_version: string_of(&root["PackageVersion"]).unwrap_or_default(),
        root: fields_of(root),
        installers,
    })
}

impl MergedManifest {
    /// One entry's fields with the root's defaults applied.
    fn effective(&self, entry: &InstallerFields) -> InstallerFields {
        let root = &self.root;
        let mut switches = root.switches.clone();
        switches.extend(entry.switches.clone());
        InstallerFields {
            architecture: entry
                .architecture
                .clone()
                .or_else(|| root.architecture.clone()),
            installer_type: entry
                .installer_type
                .clone()
                .or_else(|| root.installer_type.clone()),
            scope: entry.scope.clone().or_else(|| root.scope.clone()),
            elevation_requirement: entry
                .elevation_requirement
                .clone()
                .or_else(|| root.elevation_requirement.clone()),
            url: entry.url.clone().or_else(|| root.url.clone()),
            sha256: entry.sha256.clone().or_else(|| root.sha256.clone()),
            switches,
            success_codes: entry
                .success_codes
                .clone()
                .or_else(|| root.success_codes.clone()),
            expected_return_codes: entry
                .expected_return_codes
                .clone()
                .or_else(|| root.expected_return_codes.clone()),
        }
    }

    /// Chooses the installer for an architecture and a scope preference:
    /// an entry declaring the preferred scope, else one declaring no scope,
    /// else any entry for the architecture. `None` when the manifest has no
    /// usable entry for the architecture.
    pub fn select(&self, architecture: &str, scope: Scope) -> Option<InstallerEntry> {
        let candidates: Vec<InstallerFields> = self
            .installers
            .iter()
            .map(|entry| self.effective(entry))
            .filter(|entry| {
                entry
                    .architecture
                    .as_deref()
                    .is_some_and(|a| a.eq_ignore_ascii_case(architecture))
                    && entry.url.as_deref().is_some_and(|u| !u.is_empty())
                    && entry.sha256.as_deref().is_some_and(|h| h.len() == 64)
            })
            .collect();
        let scope_text = scope.as_str();
        let chosen = candidates
            .iter()
            .find(|e| {
                e.scope
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case(scope_text))
            })
            .or_else(|| {
                candidates
                    .iter()
                    .find(|e| e.scope.as_deref().unwrap_or("").is_empty())
            })
            .or_else(|| candidates.first())?;
        let installer_type = chosen
            .installer_type
            .as_deref()
            .unwrap_or("exe")
            .to_ascii_lowercase();
        let entry_scope = chosen.scope.as_deref().unwrap_or("").to_ascii_lowercase();
        let elevation_required = entry_scope == "machine"
            || chosen
                .elevation_requirement
                .as_deref()
                .is_some_and(|e| e.eq_ignore_ascii_case("elevationRequired"));
        let mut arguments = Vec::new();
        if !is_msi_type(&installer_type)
            && let Some(silent) = chosen.switches.get("Silent")
        {
            arguments.extend(split_command_line(silent));
        }
        if let Some(custom) = chosen.switches.get("Custom") {
            arguments.extend(split_command_line(custom));
        }
        let to_i32 = |code: i64| i32::try_from(code).ok();
        let success_codes: Vec<i32> = chosen
            .success_codes
            .iter()
            .flatten()
            .filter_map(|c| to_i32(*c))
            .collect();
        let mut reboot_codes: Vec<i32> = default_reboot_codes(&installer_type).to_vec();
        for (code, response) in chosen.expected_return_codes.iter().flatten() {
            if is_reboot_response(response)
                && let Some(code) = to_i32(*code)
                && !reboot_codes.contains(&code)
            {
                reboot_codes.push(code);
            }
        }
        Some(InstallerEntry {
            url: chosen.url.clone().unwrap_or_default(),
            sha256: chosen
                .sha256
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            installer_type,
            scope: entry_scope,
            architecture: chosen
                .architecture
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            elevation_required,
            arguments,
            success_codes,
            reboot_codes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEBVIEW2: &str =
        include_str!("../tests/fixtures/Microsoft.EdgeWebView2Runtime.merged.yaml");
    const DOTNET: &str =
        include_str!("../tests/fixtures/Microsoft.DotNet.DesktopRuntime.10.merged.yaml");

    #[test]
    fn root_defaults_and_per_entry_scope_select_the_user_installer() {
        let manifest = parse(WEBVIEW2).unwrap();
        assert_eq!(manifest.package_identifier, "Microsoft.EdgeWebView2Runtime");
        assert_eq!(manifest.package_version, "152.0.4191.53");
        assert_eq!(manifest.installers.len(), 6);
        let user = manifest.select("x64", Scope::User).unwrap();
        assert_eq!(user.installer_type, "exe");
        assert_eq!(user.scope, "user");
        assert_eq!(user.architecture, "x64");
        assert!(!user.elevation_required);
        assert!(
            user.url
                .ends_with("MicrosoftEdgeWebView2RuntimeInstallerX64.exe")
        );
        assert_eq!(
            user.sha256,
            "987a9d8b3107e84f9b53b4a077d28ae4814fc3d964d5a55c559e7334bbf24d61"
        );
        assert_eq!(user.arguments, vec!["/silent", "/install"]);
        assert_eq!(user.success_codes, vec![-2147219416, -2147219187]);
        assert!(user.reboot_codes.is_empty());
        let machine = manifest.select("x64", Scope::Machine).unwrap();
        assert_eq!(machine.scope, "machine");
        assert!(machine.elevation_required);
        assert_eq!(machine.url, user.url);
        assert!(manifest.select("ia64", Scope::User).is_none());
    }

    #[test]
    fn root_scope_and_per_entry_type_make_a_machine_burn_bundle() {
        let manifest = parse(DOTNET).unwrap();
        assert_eq!(manifest.package_version, "10.0.11");
        let entry = manifest.select("x64", Scope::User).unwrap();
        assert_eq!(entry.installer_type, "burn");
        assert_eq!(
            entry.scope, "machine",
            "the root Scope applies to every entry"
        );
        assert!(entry.elevation_required);
        assert!(
            entry
                .url
                .ends_with("windowsdesktop-runtime-10.0.11-win-x64.exe")
        );
        assert_eq!(
            entry.sha256,
            "61d2e1447b185d6f99c0d5799896240b48246f5440648bc031ebdb159a3bf3d1"
        );
        assert_eq!(entry.arguments, vec!["/quiet", "/norestart"]);
        assert!(entry.success_codes.is_empty());
        assert_eq!(entry.reboot_codes, vec![3010, 1641]);
    }

    #[test]
    fn msi_entries_keep_only_custom_switches_and_expected_reboot_codes_merge() {
        let text = "PackageIdentifier: Vendor.Thing\nPackageVersion: 2.0\nInstallerType: msi\nInstallerSwitches:\n  Silent: /qn\n  Custom: REBOOT=ReallySuppress\nExpectedReturnCodes:\n- InstallerReturnCode: 3010\n  ReturnResponse: rebootRequiredToFinish\n- InstallerReturnCode: 70\n  ReturnResponse: rebootInitiated\n- InstallerReturnCode: 5\n  ReturnResponse: cancelledByUser\nInstallers:\n- Architecture: x64\n  InstallerUrl: https://example.invalid/thing.msi\n  InstallerSha256: ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789\n  ElevationRequirement: elevationRequired\n";
        let manifest = parse(text).unwrap();
        let entry = manifest.select("X64", Scope::User).unwrap();
        assert_eq!(entry.installer_type, "msi");
        assert_eq!(entry.arguments, vec!["REBOOT=ReallySuppress"]);
        assert_eq!(entry.reboot_codes, vec![3010, 1641, 70]);
        assert!(entry.elevation_required);
        assert_eq!(entry.scope, "");
        assert_eq!(
            entry.sha256,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn command_lines_split_like_the_c_runtime() {
        assert_eq!(
            split_command_line("/silent  /install"),
            vec!["/silent", "/install"]
        );
        assert_eq!(
            split_command_line("--log \"C:\\Program Files\\x.log\" /S"),
            vec!["--log", "C:\\Program Files\\x.log", "/S"]
        );
        assert_eq!(split_command_line("a=\\\"q\\\""), vec!["a=\"q\""]);
        assert!(split_command_line("   ").is_empty());
        assert_eq!(split_command_line("\"\""), vec![""]);
    }

    #[test]
    fn malformed_documents_are_refused() {
        assert!(parse("- just\n- a list\n").is_err());
        assert!(parse("key: [unclosed\n").is_err());
        let empty = parse("PackageVersion: 1.0\n").unwrap();
        assert!(empty.select("x64", Scope::User).is_none());
    }
}
