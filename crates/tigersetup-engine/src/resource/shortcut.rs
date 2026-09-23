//! The shortcut resource: a `.lnk` in one of the scope's shortcut folders —
//! Start Menu Programs, desktop, Startup, and for user scope the Send To
//! folder — pointing into the install root, or a `.url` Internet shortcut
//! opening a web page. Ownership is by link path; removal is conservative —
//! only a link whose target still resolves into the install root (or whose
//! URL is still the recorded one) is removed. A package's own subfolder of a
//! shortcut folder goes with the last link in it: removing a link removes the
//! folders it leaves empty, never the scope's shortcut folder itself and never
//! a folder anything else is still in.

use std::path::{Path, PathBuf};

use tigersetup_format::Metadata;
use tigersetup_format::identity;
use tigersetup_format::metadata::{Shortcut, ShortcutLocation};

use crate::plan;
use crate::report::Finding;
use crate::resource::predicate::{self, Options};
use crate::scope::Locations;
use crate::win::env;
use crate::win::fs;
use crate::win::shortcut::Link;
use crate::{Error, Result};

/// A link the desired state wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredShortcut {
    /// Absolute path of the `.lnk` or `.url`.
    pub path: PathBuf,
    pub link: Link,
}

/// The shortcuts the effective options enable, resolved to absolute paths,
/// and a finding for each one whose folder this scope does not have.
pub fn desired(
    metadata: &Metadata,
    options: &Options,
    locations: &Locations,
    install_root: &Path,
) -> Result<(Vec<DesiredShortcut>, Vec<Finding>)> {
    let mut out = Vec::new();
    let mut findings = Vec::new();
    for shortcut in &metadata.shortcuts {
        if !predicate::enabled(shortcut.when.as_ref(), &shortcut.option, options) {
            continue;
        }
        let Some(folder_template) = locations.shortcut_folder(location_of(shortcut)) else {
            findings.push(Finding::named(
                "shortcut_location_unavailable",
                format!(
                    "{}: {} scope has no such folder",
                    shortcut.name,
                    locations.scope.as_str()
                ),
            ));
            continue;
        };
        let mut folder = PathBuf::from(identity::expand_template(
            folder_template,
            env::known_folder,
        )?);
        if !shortcut.folder.is_empty() {
            folder = folder.join(plan::to_relative(&shortcut.folder));
        }
        if !shortcut.url.is_empty() {
            let icon = if shortcut.icon.is_empty() {
                String::new()
            } else {
                plan::absolute(install_root, &plan::to_relative(&shortcut.icon))?
                    .display()
                    .to_string()
            };
            out.push(DesiredShortcut {
                path: folder.join(format!("{}.url", shortcut.name)),
                link: Link {
                    target: shortcut.url.clone(),
                    icon,
                    ..Link::default()
                },
            });
            continue;
        }
        let target = plan::absolute(install_root, &plan::to_relative(&shortcut.target))?;
        let icon = if shortcut.icon.is_empty() {
            target.clone()
        } else {
            plan::absolute(install_root, &plan::to_relative(&shortcut.icon))?
        };
        let working_directory = if shortcut.working_directory.is_empty() {
            target
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| install_root.to_path_buf())
        } else {
            plan::absolute(
                install_root,
                &plan::to_relative(&shortcut.working_directory),
            )?
        };
        out.push(DesiredShortcut {
            path: folder.join(format!("{}.lnk", shortcut.name)),
            link: Link {
                target: target.display().to_string(),
                arguments: shortcut.arguments.clone(),
                description: shortcut.description.clone(),
                icon: icon.display().to_string(),
                working_directory: working_directory.display().to_string(),
                app_user_model_id: shortcut.app_user_model_id.clone(),
            },
        });
    }
    Ok((out, findings))
}

/// The location a declared shortcut names; an unknown tag is the Start Menu,
/// which is where a shortcut with no better idea belongs.
pub fn location_of(shortcut: &Shortcut) -> ShortcutLocation {
    ShortcutLocation::try_from(shortcut.location).unwrap_or(ShortcutLocation::StartMenu)
}

/// Whether a link target lies inside the install root.
pub fn targets_install_root(target: &str, install_root: &Path) -> bool {
    let root = install_root
        .display()
        .to_string()
        .trim_end_matches('\\')
        .to_lowercase();
    let target = target.trim().to_lowercase();
    target == root || target.starts_with(&format!("{root}\\"))
}

/// Whether an owned link may be removed: a shell link still pointing into
/// the install root, or an Internet shortcut still opening the recorded
/// URL. Anything else was changed by someone and stays.
pub fn removable(
    link_path: &Path,
    current: &Link,
    recorded_target: &str,
    install_root: &Path,
) -> bool {
    if crate::win::shortcut::is_url_shortcut(link_path) {
        current.target.eq_ignore_ascii_case(recorded_target)
    } else {
        targets_install_root(&current.target, install_root)
    }
}

/// Whether a link on disk is the one the desired state wants. The working
/// directory and the AppUserModelID are compared too, so a link written
/// before either was declared is rewritten rather than kept.
pub fn matches(current: &Link, desired: &Link) -> bool {
    current.target.eq_ignore_ascii_case(&desired.target)
        && current.arguments == desired.arguments
        && current.description == desired.description
        && current.icon.eq_ignore_ascii_case(&desired.icon)
        && current
            .working_directory
            .eq_ignore_ascii_case(&desired.working_directory)
        && current.app_user_model_id == desired.app_user_model_id
}

/// Removes the folders a removed link leaves empty, from the link's own
/// folder upwards. It stops at any of the scope's shortcut folders (`roots`)
/// — which it never removes, Startup included although it lies inside the
/// Start Menu's Programs — at a folder outside them, at a junction or other
/// reparse point, and at the first folder that still holds anything, so a
/// folder several products share stays until the last of their links is
/// gone. Best effort: a folder that cannot be removed (in use, access
/// refused) is left and ends the walk; the link's own removal has already
/// succeeded, and a folder is not worth failing a run or a rollback for.
/// Returns the folders removed.
pub fn remove_emptied_folders(link: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let mut folder = link.parent();
    while let Some(current) = folder {
        let is_root = roots.iter().any(|root| same_folder(root, current));
        if is_root
            || !crate::scope::is_inside(current, roots)
            || is_reparse_point(current)
            || !matches!(fs::remove_directory_if_empty(current), Ok(true))
        {
            break;
        }
        removed.push(current.to_path_buf());
        folder = current.parent();
    }
    removed
}

fn same_folder(a: &Path, b: &Path) -> bool {
    let normal = |path: &Path| {
        path.display()
            .to_string()
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    normal(a) == normal(b)
}

/// A junction or a symbolic link: removing it would remove the link, not an
/// emptied folder of the package's, so the walk never does.
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}

/// The link path as the journal stores it, checked to be absolute and to
/// name a shell link or an Internet shortcut.
pub fn link_path(text: &str) -> Result<PathBuf> {
    let path = PathBuf::from(text);
    let lower = text.to_ascii_lowercase();
    if !path.is_absolute() || !(lower.ends_with(".lnk") || lower.ends_with(".url")) {
        return Err(Error::new(
            "journal_inconsistent",
            format!("{text:?} is not an absolute .lnk or .url path"),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::identity::Scope;
    use tigersetup_format::metadata::{OptionValue, Package, Predicate};

    #[test]
    fn a_removed_link_takes_the_folders_it_emptied_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let programs = dir.path().join("Programs");
        let roots = vec![programs.clone()];
        let own = programs.join("Vendor").join("MyApp");
        std::fs::create_dir_all(&own).unwrap();
        // Another product's link keeps the shared Vendor folder.
        std::fs::write(programs.join("Vendor").join("Other.lnk"), b"x").unwrap();
        let link = own.join("MyApp.lnk");
        assert_eq!(remove_emptied_folders(&link, &roots), vec![own.clone()]);
        assert!(programs.join("Vendor").is_dir());

        // The last one out removes the shared folder, never the root.
        std::fs::remove_file(programs.join("Vendor").join("Other.lnk")).unwrap();
        assert_eq!(
            remove_emptied_folders(&programs.join("Vendor").join("Other.lnk"), &roots),
            vec![programs.join("Vendor")]
        );
        assert!(programs.is_dir());

        // A folder that still holds something stays.
        std::fs::create_dir_all(&own).unwrap();
        std::fs::write(own.join("readme.txt"), b"x").unwrap();
        assert!(remove_emptied_folders(&link, &roots).is_empty());
        assert!(own.is_dir());
        // A link directly in the root touches no folder.
        assert!(remove_emptied_folders(&programs.join("Direct.lnk"), &roots).is_empty());
    }
    #[test]
    fn a_shortcut_folder_inside_another_is_never_removed() {
        // Startup lies inside the Start Menu's Programs; a link removed from
        // it must not take the Startup folder with it, empty or not.
        let dir = tempfile::tempdir().unwrap();
        let programs = dir.path().join("Programs");
        let startup = programs.join("Startup");
        std::fs::create_dir_all(&startup).unwrap();
        let roots = vec![programs.clone(), startup.clone()];
        assert!(remove_emptied_folders(&startup.join("Agent.lnk"), &roots).is_empty());
        assert!(startup.is_dir());
        // Nor is a junction in the way followed or removed.
        let target = dir.path().join("elsewhere");
        std::fs::create_dir_all(&target).unwrap();
        let junction = programs.join("Linked");
        let made = std::process::Command::new("cmd.exe")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        assert!(remove_emptied_folders(&junction.join("App.lnk"), &roots).is_empty());
        assert!(junction.exists() && target.is_dir());
    }
    #[test]
    fn the_install_root_check_is_a_case_insensitive_prefix() {
        let root = Path::new("C:\\Programs\\TestApp");
        assert!(targets_install_root(
            "c:\\programs\\testapp\\bin\\app.exe",
            root
        ));
        assert!(targets_install_root("C:\\Programs\\TestApp", root));
        assert!(!targets_install_root(
            "C:\\Programs\\TestApp2\\app.exe",
            root
        ));
        assert!(!targets_install_root("C:\\Programs\\app.exe", root));
        assert!(link_path("relative.lnk").is_err());
        assert!(link_path("C:\\x\\a.txt").is_err());
        assert!(link_path("C:\\x\\A.LNK").is_ok());
        assert!(link_path("C:\\x\\Docs.url").is_ok());
        let url = Link {
            target: "https://example.invalid/docs".into(),
            ..Link::default()
        };
        assert!(removable(
            Path::new("C:\\x\\Docs.url"),
            &url,
            "HTTPS://example.invalid/docs",
            root
        ));
        assert!(!removable(
            Path::new("C:\\x\\Docs.url"),
            &url,
            "https://example.invalid/other",
            root
        ));
    }

    /// Every location resolves to the scope's folder — except Send To in
    /// machine scope, which has none and is reported rather than invented.
    #[test]
    fn shortcuts_resolve_their_locations_working_directory_and_url() {
        let metadata = Metadata {
            package: Some(Package {
                name: "TestApp".into(),
                ..Default::default()
            }),
            shortcuts: vec![
                Shortcut {
                    location: ShortcutLocation::StartMenu as i32,
                    name: "TestApp".into(),
                    target: "bin/app.exe".into(),
                    app_user_model_id: "ITTiger.TestApp".into(),
                    working_directory: "data".into(),
                    ..Default::default()
                },
                Shortcut {
                    location: ShortcutLocation::Startup as i32,
                    name: "TestApp Agent".into(),
                    target: "bin/app.exe".into(),
                    arguments: "--agent".into(),
                    when: Some(Predicate {
                        option: "startup".into(),
                        equals: "true".into(),
                    }),
                    ..Default::default()
                },
                Shortcut {
                    location: ShortcutLocation::SendTo as i32,
                    name: "TestApp".into(),
                    target: "bin/app.exe".into(),
                    ..Default::default()
                },
                Shortcut {
                    location: ShortcutLocation::StartMenu as i32,
                    name: "TestApp Documentation".into(),
                    url: "https://example.invalid/docs".into(),
                    icon: "bin/app.exe".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut options = Options::new();
        options.insert("startup".into(), OptionValue::Bool(true));
        let root = Path::new("C:\\P");

        let user = crate::scope::locations(Scope::User);
        let (desired, findings) = desired(&metadata, &options, &user, root).unwrap();
        assert!(findings.is_empty(), "{findings:?}");
        assert_eq!(desired.len(), 4);
        assert!(desired[0].path.ends_with("TestApp.lnk"));
        assert_eq!(desired[0].link.working_directory, "C:\\P\\data");
        assert_eq!(desired[0].link.app_user_model_id, "ITTiger.TestApp");
        assert!(desired[1].path.ends_with("TestApp Agent.lnk"));
        assert_eq!(desired[1].link.working_directory, "C:\\P\\bin");
        assert!(
            desired[2].path.ends_with("SendTo\\TestApp.lnk")
                || desired[2]
                    .path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains("sendto"),
            "{:?}",
            desired[2].path
        );
        assert!(desired[3].path.ends_with("TestApp Documentation.url"));
        assert_eq!(desired[3].link.target, "https://example.invalid/docs");
        assert_eq!(desired[3].link.icon, "C:\\P\\bin\\app.exe");
        assert!(desired[3].link.working_directory.is_empty());

        options.insert("startup".into(), OptionValue::Bool(false));
        let machine = crate::scope::locations(Scope::Machine);
        let (desired, findings) = super::desired(&metadata, &options, &machine, root).unwrap();
        assert_eq!(
            desired.len(),
            2,
            "the startup link is off, Send To has no machine folder"
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "shortcut_location_unavailable");

        let wanted = &desired[0].link;
        let mut current = wanted.clone();
        current.working_directory = wanted.working_directory.to_uppercase();
        assert!(matches(&current, wanted));
        current.app_user_model_id = String::new();
        assert!(
            !matches(&current, wanted),
            "a link without the id is rewritten"
        );
    }
}
