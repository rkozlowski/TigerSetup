//! The shortcut resource: a `.lnk` in the scope's Start Menu Programs or
//! desktop folder pointing into the install root. Ownership is by link
//! path; removal is conservative — only a link whose target still resolves
//! into the install root is removed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tigersetup_format::Metadata;
use tigersetup_format::identity;
use tigersetup_format::metadata::ShortcutLocation;

use crate::plan;
use crate::scope::Locations;
use crate::win::env;
use crate::win::shortcut::Link;
use crate::{Error, Result};

/// A link the desired state wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredShortcut {
    /// Absolute path of the `.lnk`.
    pub path: PathBuf,
    pub link: Link,
}

/// Whether an option-gated resource is enabled by the effective options.
pub fn enabled(option: &str, options: &BTreeMap<String, bool>) -> bool {
    option.is_empty()
        || options
            .get(&option.to_ascii_lowercase())
            .copied()
            .unwrap_or(false)
}

/// The shortcuts the effective options enable, resolved to absolute paths.
pub fn desired(
    metadata: &Metadata,
    options: &BTreeMap<String, bool>,
    locations: &Locations,
    install_root: &Path,
) -> Result<Vec<DesiredShortcut>> {
    let mut out = Vec::new();
    for shortcut in &metadata.shortcuts {
        if !enabled(&shortcut.option, options) {
            continue;
        }
        let folder_template = match ShortcutLocation::try_from(shortcut.location) {
            Ok(ShortcutLocation::Desktop) => locations.desktop_folder,
            _ => locations.programs_folder,
        };
        let mut folder = PathBuf::from(identity::expand_template(
            folder_template,
            env::known_folder,
        )?);
        if !shortcut.folder.is_empty() {
            folder = folder.join(plan::to_relative(&shortcut.folder));
        }
        let target = plan::absolute(install_root, &plan::to_relative(&shortcut.target))?;
        let icon = if shortcut.icon.is_empty() {
            target.clone()
        } else {
            plan::absolute(install_root, &plan::to_relative(&shortcut.icon))?
        };
        let working_directory = target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| install_root.to_path_buf());
        out.push(DesiredShortcut {
            path: folder.join(format!("{}.lnk", shortcut.name)),
            link: Link {
                target: target.display().to_string(),
                arguments: shortcut.arguments.clone(),
                description: shortcut.description.clone(),
                icon: icon.display().to_string(),
                working_directory: working_directory.display().to_string(),
            },
        });
    }
    Ok(out)
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

/// Whether a link on disk is the one the desired state wants.
pub fn matches(current: &Link, desired: &Link) -> bool {
    current.target.eq_ignore_ascii_case(&desired.target)
        && current.arguments == desired.arguments
        && current.description == desired.description
        && current.icon.eq_ignore_ascii_case(&desired.icon)
}

/// The link path as the journal stores it, checked to be absolute.
pub fn link_path(text: &str) -> Result<PathBuf> {
    let path = PathBuf::from(text);
    if !path.is_absolute() || !text.to_ascii_lowercase().ends_with(".lnk") {
        return Err(Error::new(
            "journal_inconsistent",
            format!("{text:?} is not an absolute .lnk path"),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut options = BTreeMap::new();
        options.insert("desktop-shortcut".to_string(), false);
        assert!(enabled("", &options));
        assert!(!enabled("desktop-shortcut", &options));
        assert!(!enabled("unknown", &options));
        assert!(link_path("relative.lnk").is_err());
        assert!(link_path("C:\\x\\a.txt").is_err());
        assert!(link_path("C:\\x\\A.LNK").is_ok());
    }
}
