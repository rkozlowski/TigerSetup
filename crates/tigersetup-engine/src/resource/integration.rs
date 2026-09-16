//! The typed Windows integrations — file associations, URL protocols, `App
//! Paths` and classic Explorer context-menu verbs — are registry values in
//! the shape Windows documents for each, all under the scope's `Software`
//! root. This module compiles a declaration into the values it means; the
//! planner then reconciles them exactly as it reconciles `[[registry]]`
//! values, with the same value-level ownership, the same undo records and
//! the same conservative removal. There is no second registry engine here,
//! only the knowledge of which keys and values each integration is.
//!
//! What the shapes are:
//!
//! - **File association** — `Classes\<ProgID>` with its description,
//!   `DefaultIcon` and `shell\open\command`; `Classes\<.ext>\OpenWithProgids\
//!   <ProgID>` for every extension, which lists the application in *Open
//!   with* without changing the user's default; and a capability
//!   registration (`Software\<Publisher>\<Product>\Capabilities` plus
//!   `Software\RegisteredApplications`) so Windows Settings can offer it.
//! - **URL protocol** — a handler ProgID carrying `URL Protocol`, the
//!   capability's `URLAssociations` entry, and the scheme's own class key
//!   *when nothing else owns it*: a scheme another application already
//!   registered is left exactly as it is and reported.
//! - **App Paths** — `Software\Microsoft\Windows\CurrentVersion\App Paths\
//!   <exe>` with the full path and, on request, the directory.
//! - **Context-menu verb** — `Classes\*\shell\<verb>` (or
//!   `SystemFileAssociations\<.ext>\shell\<verb>`), `Classes\Directory\shell\
//!   <verb>` or `Classes\Directory\Background\shell\<verb>`, each with its
//!   label, optional `Icon` and `command`.

use std::collections::BTreeSet;
use std::path::Path;

use tigersetup_format::Metadata;
use tigersetup_format::metadata::ContextMenuTarget;

use crate::plan::{absolute, to_relative};
use crate::report::Finding;
use crate::resource::predicate::{self, Options};
use crate::resource::registry::DesiredValue;
use crate::scope::Locations;
use crate::state::installation::Owned;
use crate::win::registry::{self as winreg, Data, KeyPath, Roots};
use crate::{Error, Result};

/// One declared integration and the values it means on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integration {
    /// `file_association`, `url_protocol`, `app_path` or `context_menu`.
    pub kind: &'static str,
    /// The ProgID, the scheme, the executable name, or `<target>:<verb>`.
    pub id: String,
    /// Whether the effective options enable it.
    pub enabled: bool,
    /// The values it consists of, when enabled.
    pub values: Vec<DesiredValue>,
}

/// Every declared integration compiled against the effective options and
/// the machine, with the findings the compilation raised.
#[derive(Debug, Default)]
pub struct Integrations {
    pub items: Vec<Integration>,
    pub findings: Vec<Finding>,
}

impl Integrations {
    /// The values of every enabled integration, in declaration order.
    pub fn values(&self) -> Vec<DesiredValue> {
        self.items
            .iter()
            .filter(|i| i.enabled)
            .flat_map(|i| i.values.iter().cloned())
            .collect()
    }
}

/// `"<exe>" <arguments>`: the command a verb or a handler runs.
pub fn command_line(executable: &Path, arguments: &str) -> String {
    format!("\"{}\" {}", executable.display(), arguments)
}

fn string(key: KeyPath, name: &str, data: impl Into<String>) -> DesiredValue {
    DesiredValue {
        key,
        name: name.to_string(),
        data: Data::String(data.into()),
    }
}

/// A key component from product text: Windows forbids only the separator,
/// and a product name is never allowed to escape its own key.
fn component(text: &str) -> String {
    text.replace('\\', "-")
}

/// The capability registration's key for this product.
fn capabilities_key(metadata: &Metadata, locations: &Locations) -> KeyPath {
    let package = metadata.package();
    locations.software_key(&format!(
        "{}\\{}\\Capabilities",
        component(&package.publisher),
        component(&package.name)
    ))
}

/// The values every capability registration carries once, whichever
/// integration asked for it.
fn capability_values(metadata: &Metadata, locations: &Locations) -> Vec<DesiredValue> {
    let package = metadata.package();
    let capabilities = capabilities_key(metadata, locations);
    let description = if package.description.is_empty() {
        package.name.clone()
    } else {
        package.description.clone()
    };
    vec![
        string(capabilities.clone(), "ApplicationName", &package.name),
        string(capabilities.clone(), "ApplicationDescription", description),
        string(
            locations.software_key("RegisteredApplications"),
            &component(&package.name),
            format!(
                "Software\\{}\\{}\\Capabilities",
                component(&package.publisher),
                component(&package.name)
            ),
        ),
    ]
}

fn resolve(install_root: &Path, relative: &str) -> Result<std::path::PathBuf> {
    absolute(install_root, &to_relative(relative))
}

/// The handler class values a ProgID carries: description, icon, command,
/// and `URL Protocol` for a scheme handler.
fn handler_class(
    locations: &Locations,
    prog_id: &str,
    description: &str,
    icon: &Path,
    command: &str,
    url_protocol: bool,
) -> Vec<DesiredValue> {
    let class = locations.software_key(&format!("Classes\\{prog_id}"));
    let mut values = vec![
        string(class.clone(), "", description),
        string(
            class.child("DefaultIcon"),
            "",
            format!("{},0", icon.display()),
        ),
        string(class.child("shell\\open\\command"), "", command),
    ];
    if url_protocol {
        values.insert(1, string(class, "URL Protocol", ""));
    }
    values
}

/// Compiles the declared integrations for this run.
pub fn desired(
    metadata: &Metadata,
    options: &Options,
    locations: &Locations,
    install_root: &Path,
    roots: &Roots,
    owned: &Owned,
) -> Result<Integrations> {
    let mut out = Integrations::default();
    let mut capabilities_wanted = false;

    for association in &metadata.file_associations {
        let enabled = predicate::enabled(association.when.as_ref(), "", options);
        let mut values = Vec::new();
        if enabled {
            capabilities_wanted = true;
            let executable = resolve(install_root, &association.executable)?;
            let icon = if association.icon.is_empty() {
                executable.clone()
            } else {
                resolve(install_root, &association.icon)?
            };
            let arguments = if association.arguments.is_empty() {
                "\"%1\""
            } else {
                association.arguments.as_str()
            };
            values.extend(handler_class(
                locations,
                &association.prog_id,
                &association.description,
                &icon,
                &command_line(&executable, arguments),
                false,
            ));
            let capabilities = capabilities_key(metadata, locations);
            for extension in &association.extensions {
                values.push(string(
                    locations.software_key(&format!("Classes\\{extension}\\OpenWithProgids")),
                    &association.prog_id,
                    "",
                ));
                values.push(string(
                    capabilities.child("FileAssociations"),
                    extension,
                    &association.prog_id,
                ));
            }
        }
        out.items.push(Integration {
            kind: "file_association",
            id: association.prog_id.clone(),
            enabled,
            values,
        });
    }

    for protocol in &metadata.url_protocols {
        let enabled = predicate::enabled(protocol.when.as_ref(), "", options);
        let mut values = Vec::new();
        if enabled {
            capabilities_wanted = true;
            let prog_id = if protocol.prog_id.is_empty() {
                format!(
                    "{}.{}",
                    component(&metadata.package().name).replace('.', ""),
                    protocol.scheme
                )
            } else {
                protocol.prog_id.clone()
            };
            let executable = resolve(install_root, &protocol.executable)?;
            let icon = if protocol.icon.is_empty() {
                executable.clone()
            } else {
                resolve(install_root, &protocol.icon)?
            };
            let arguments = if protocol.arguments.is_empty() {
                "\"%1\""
            } else {
                protocol.arguments.as_str()
            };
            let command = command_line(&executable, arguments);
            let description = format!(
                "URL:{}",
                if protocol.description.is_empty() {
                    &protocol.scheme
                } else {
                    &protocol.description
                }
            );
            values.extend(handler_class(
                locations,
                &prog_id,
                &description,
                &icon,
                &command,
                true,
            ));
            values.push(string(
                capabilities_key(metadata, locations).child("URLAssociations"),
                &protocol.scheme,
                &prog_id,
            ));
            // The scheme's own class key is what resolves `scheme:` when
            // no default was ever chosen. It is written only where nothing
            // else owns it: absent, or created by TigerSetup itself.
            let scheme_key = locations.software_key(&format!("Classes\\{}", protocol.scheme));
            let ours = owned
                .registry_keys
                .iter()
                .any(|k| k.created && k.key.eq_ignore_ascii_case(&scheme_key.to_string()));
            if ours || !winreg::key_exists(roots, &scheme_key)? {
                values.extend(handler_class(
                    locations,
                    &protocol.scheme,
                    &description,
                    &icon,
                    &command,
                    true,
                ));
            } else {
                out.findings.push(Finding::named(
                    "url_protocol_scheme_in_use_preserved",
                    scheme_key.to_string(),
                ));
            }
        }
        out.items.push(Integration {
            kind: "url_protocol",
            id: protocol.scheme.clone(),
            enabled,
            values,
        });
    }

    for app_path in &metadata.app_paths {
        let enabled = predicate::enabled(app_path.when.as_ref(), "", options);
        let executable = resolve(install_root, &app_path.executable)?;
        let file_name = executable
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| {
                Error::new(
                    "metadata_invalid",
                    format!("App Paths entry {} has no file name", app_path.executable),
                )
            })?;
        let mut values = Vec::new();
        if enabled {
            let key = locations.software_key(&format!(
                "Microsoft\\Windows\\CurrentVersion\\App Paths\\{file_name}"
            ));
            values.push(string(key.clone(), "", executable.display().to_string()));
            if app_path.add_directory {
                let directory = executable
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                values.push(string(key, "Path", directory));
            }
        }
        out.items.push(Integration {
            kind: "app_path",
            id: file_name,
            enabled,
            values,
        });
    }

    for verb in &metadata.context_menu_verbs {
        let enabled = predicate::enabled(verb.when.as_ref(), "", options);
        let target = ContextMenuTarget::try_from(verb.target).unwrap_or_default();
        let target_name = match target {
            ContextMenuTarget::Directories => "directories",
            ContextMenuTarget::DirectoryBackground => "directory-background",
            _ => "files",
        };
        let mut values = Vec::new();
        if enabled {
            let executable = resolve(install_root, &verb.executable)?;
            let default_arguments = match target {
                ContextMenuTarget::DirectoryBackground => "\"%V\"",
                _ => "\"%1\"",
            };
            let arguments = if verb.arguments.is_empty() {
                default_arguments
            } else {
                verb.arguments.as_str()
            };
            let command = command_line(&executable, arguments);
            let icon = if verb.icon.is_empty() {
                None
            } else {
                Some(resolve(install_root, &verb.icon)?)
            };
            let classes: Vec<String> = match target {
                ContextMenuTarget::Directories => vec!["Directory".into()],
                ContextMenuTarget::DirectoryBackground => vec!["Directory\\Background".into()],
                _ if verb.extensions.is_empty() => vec!["*".into()],
                _ => verb
                    .extensions
                    .iter()
                    .map(|e| format!("SystemFileAssociations\\{e}"))
                    .collect(),
            };
            for class in classes {
                let key =
                    locations.software_key(&format!("Classes\\{class}\\shell\\{}", verb.verb));
                values.push(string(key.clone(), "", &verb.label));
                if let Some(icon) = &icon {
                    values.push(string(key.clone(), "Icon", icon.display().to_string()));
                }
                values.push(string(key.child("command"), "", &command));
            }
        }
        out.items.push(Integration {
            kind: "context_menu",
            id: format!("{target_name}:{}", verb.verb),
            enabled,
            values,
        });
    }

    if capabilities_wanted {
        out.items.push(Integration {
            kind: "capabilities",
            id: metadata.package().name.clone(),
            enabled: true,
            values: capability_values(metadata, locations),
        });
    }
    Ok(out)
}

/// What the machine holds of one integration: how many of its values are
/// present with the intended data, and whether the installation owns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub values_total: usize,
    pub values_present: usize,
    pub values_owned: usize,
}

/// Reads the machine for an integration's values.
pub fn presence(integration: &Integration, roots: &Roots, owned: &Owned) -> Result<Presence> {
    let owned_keys: BTreeSet<(String, String)> = owned
        .registry_values
        .iter()
        .map(|v| (v.key.to_ascii_lowercase(), v.name.to_ascii_lowercase()))
        .collect();
    let mut present = 0;
    let mut owned_count = 0;
    for value in &integration.values {
        if winreg::read_value(roots, &value.key, &value.name)?.as_ref() == Some(&value.data) {
            present += 1;
        }
        if owned_keys.contains(&(
            value.key.to_string().to_ascii_lowercase(),
            value.name.to_ascii_lowercase(),
        )) {
            owned_count += 1;
        }
    }
    Ok(Presence {
        values_total: integration.values.len(),
        values_present: present,
        values_owned: owned_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::identity::Scope;
    use tigersetup_format::metadata::{
        AppPath, ContextMenuVerb, FileAssociation, OptionValue, Package, Predicate, UrlProtocol,
    };

    fn metadata() -> Metadata {
        Metadata {
            package: Some(Package {
                id: "IT-Tiger.T".into(),
                name: "TestApp".into(),
                version: "1.0.0".into(),
                publisher: "IT Tiger".into(),
                description: "A test".into(),
                ..Default::default()
            }),
            file_associations: vec![FileAssociation {
                prog_id: "TestApp.Document".into(),
                extensions: vec![".tigertest".into()],
                description: "Tiger test document".into(),
                icon: String::new(),
                executable: "bin/app.exe".into(),
                arguments: String::new(),
                when: Some(Predicate {
                    option: "assoc".into(),
                    equals: "true".into(),
                }),
            }],
            url_protocols: vec![UrlProtocol {
                scheme: "tigertest".into(),
                prog_id: String::new(),
                description: "Tiger test link".into(),
                icon: String::new(),
                executable: "bin/app.exe".into(),
                arguments: "--open \"%1\"".into(),
                when: None,
            }],
            app_paths: vec![AppPath {
                executable: "bin/app.exe".into(),
                add_directory: true,
                when: None,
            }],
            context_menu_verbs: vec![
                ContextMenuVerb {
                    target: ContextMenuTarget::Files as i32,
                    verb: "open-with-testapp".into(),
                    label: "Open with TestApp".into(),
                    executable: "bin/app.exe".into(),
                    arguments: String::new(),
                    icon: "bin/app.exe".into(),
                    extensions: Vec::new(),
                    when: None,
                },
                ContextMenuVerb {
                    target: ContextMenuTarget::DirectoryBackground as i32,
                    verb: "testapp-here".into(),
                    label: "TestApp here".into(),
                    executable: "bin/app.exe".into(),
                    arguments: String::new(),
                    icon: String::new(),
                    extensions: Vec::new(),
                    when: None,
                },
            ],
            ..Default::default()
        }
    }

    fn located(values: &[DesiredValue]) -> Vec<String> {
        values
            .iter()
            .map(|v| format!("{}\\{} = {}", v.key, v.name, v.data.text()))
            .collect()
    }

    #[test]
    fn integrations_compile_into_the_documented_registry_shapes() {
        let metadata = metadata();
        let locations = crate::scope::locations(Scope::User);
        let roots = Roots::relocated(&format!(
            "Software\\TigerSetupTests\\integration-{}",
            std::process::id()
        ));
        let mut options = Options::new();
        options.insert("assoc".into(), OptionValue::Bool(true));
        let compiled = desired(
            &metadata,
            &options,
            &locations,
            Path::new("C:\\P"),
            &roots,
            &Owned::default(),
        )
        .unwrap();
        assert!(compiled.findings.is_empty(), "{:?}", compiled.findings);
        let kinds: Vec<&str> = compiled.items.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                "file_association",
                "url_protocol",
                "app_path",
                "context_menu",
                "context_menu",
                "capabilities"
            ]
        );
        let all = located(&compiled.values());
        for expected in [
            "HKCU\\Software\\Classes\\TestApp.Document\\ = Tiger test document",
            "HKCU\\Software\\Classes\\TestApp.Document\\DefaultIcon\\ = C:\\P\\bin\\app.exe,0",
            "HKCU\\Software\\Classes\\TestApp.Document\\shell\\open\\command\\ = \"C:\\P\\bin\\app.exe\" \"%1\"",
            "HKCU\\Software\\Classes\\.tigertest\\OpenWithProgids\\TestApp.Document = ",
            "HKCU\\Software\\IT Tiger\\TestApp\\Capabilities\\FileAssociations\\.tigertest = TestApp.Document",
            "HKCU\\Software\\Classes\\TestApp.tigertest\\URL Protocol = ",
            "HKCU\\Software\\Classes\\TestApp.tigertest\\ = URL:Tiger test link",
            "HKCU\\Software\\Classes\\TestApp.tigertest\\shell\\open\\command\\ = \"C:\\P\\bin\\app.exe\" --open \"%1\"",
            "HKCU\\Software\\Classes\\tigertest\\URL Protocol = ",
            "HKCU\\Software\\IT Tiger\\TestApp\\Capabilities\\URLAssociations\\tigertest = TestApp.tigertest",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\app.exe\\ = C:\\P\\bin\\app.exe",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\app.exe\\Path = C:\\P\\bin",
            "HKCU\\Software\\Classes\\*\\shell\\open-with-testapp\\ = Open with TestApp",
            "HKCU\\Software\\Classes\\*\\shell\\open-with-testapp\\Icon = C:\\P\\bin\\app.exe",
            "HKCU\\Software\\Classes\\*\\shell\\open-with-testapp\\command\\ = \"C:\\P\\bin\\app.exe\" \"%1\"",
            "HKCU\\Software\\Classes\\Directory\\Background\\shell\\testapp-here\\command\\ = \"C:\\P\\bin\\app.exe\" \"%V\"",
            "HKCU\\Software\\IT Tiger\\TestApp\\Capabilities\\ApplicationName = TestApp",
            "HKCU\\Software\\RegisteredApplications\\TestApp = Software\\IT Tiger\\TestApp\\Capabilities",
        ] {
            assert!(
                all.contains(&expected.to_string()),
                "missing {expected}\n{all:#?}"
            );
        }
        // Every value lies where the scope may write.
        for value in compiled.values() {
            locations.confine_key(&value.key).unwrap();
        }

        // The association follows its predicate; the capability
        // registration stays for the protocol.
        options.insert("assoc".into(), OptionValue::Bool(false));
        let compiled = desired(
            &metadata,
            &options,
            &locations,
            Path::new("C:\\P"),
            &roots,
            &Owned::default(),
        )
        .unwrap();
        assert!(!compiled.items[0].enabled);
        assert!(compiled.items[0].values.is_empty());
        assert!(compiled.items.iter().any(|i| i.kind == "capabilities"));
        assert!(
            !located(&compiled.values())
                .iter()
                .any(|v| v.contains("TestApp.Document"))
        );
    }

    /// A scheme another application registered is never rewritten: the
    /// handler ProgID and the capability are still registered, the scheme
    /// key is left alone and reported.
    #[test]
    fn a_foreign_scheme_key_is_preserved_and_reported() {
        let metadata = metadata();
        let locations = crate::scope::locations(Scope::User);
        let prefix = format!(
            "Software\\TigerSetupTests\\integration-scheme-{}",
            std::process::id()
        );
        let roots = Roots::relocated(&prefix);
        let scheme_key = locations.software_key("Classes\\tigertest");
        winreg::create_key(&roots, &scheme_key).unwrap();
        let compiled = desired(
            &metadata,
            &Options::new(),
            &locations,
            Path::new("C:\\P"),
            &roots,
            &Owned::default(),
        );
        let _ = std::process::Command::new("reg.exe")
            .args(["delete", &format!("HKCU\\{prefix}"), "/f"])
            .output();
        let compiled = compiled.unwrap();
        assert_eq!(
            compiled.findings,
            vec![Finding::named(
                "url_protocol_scheme_in_use_preserved",
                "HKCU\\Software\\Classes\\tigertest"
            )]
        );
        let all = located(&compiled.values());
        assert!(
            !all.iter()
                .any(|v| v.starts_with("HKCU\\Software\\Classes\\tigertest\\"))
        );
        assert!(
            all.iter()
                .any(|v| v.starts_with("HKCU\\Software\\Classes\\TestApp.tigertest\\"))
        );
    }
}
