//! File-set resolution: `[[files]]` globs relative to the manifest become a
//! sorted list of install-relative paths with their sources and sizes.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use glob::{MatchOptions, Pattern};

use crate::manifest::FilesEntry;
use crate::{BuildError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFile {
    /// Install-relative path with forward slashes.
    pub relative: String,
    pub source: PathBuf,
    pub size: u64,
    /// The predicate of the `[[files]]` entry the file came from.
    pub when: Option<tigersetup_format::metadata::Predicate>,
}

fn is_glob_component(component: &str) -> bool {
    component.contains(['*', '?', '[', '{'])
}

/// Splits a source pattern into the literal base directory and the pattern
/// that follows it.
fn split_base(source: &str) -> (PathBuf, bool) {
    let mut base = PathBuf::new();
    let mut has_glob = false;
    for component in Path::new(source).components() {
        match component {
            Component::Normal(part) if is_glob_component(&part.to_string_lossy()) => {
                has_glob = true;
                break;
            }
            other => base.push(other.as_os_str()),
        }
    }
    (base, has_glob)
}

fn to_forward(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub fn resolve(manifest_dir: &Path, entries: &[FilesEntry]) -> Result<Vec<ResolvedFile>> {
    let exclude_options = MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    let glob_options = MatchOptions {
        case_sensitive: false,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    let mut files: BTreeMap<String, ResolvedFile> = BTreeMap::new();

    for entry in entries {
        let excludes = entry
            .exclude
            .iter()
            .map(|pattern| {
                Pattern::new(pattern).map_err(|err| {
                    BuildError::new(
                        "manifest_invalid",
                        format!("exclude pattern {pattern:?}: {err}"),
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let when = entry.when.as_ref().map(|w| w.to_metadata());
        let (base, has_glob) = split_base(&entry.source);
        let base_dir = manifest_dir.join(&base);
        let matched: Vec<PathBuf> = if has_glob {
            // `dir/**` means "everything under dir"; the glob crate's `**`
            // alone matches directories, so files need `dir/**/*`.
            let mut pattern = manifest_dir
                .join(&entry.source)
                .to_string_lossy()
                .replace('\\', "/");
            if pattern.ends_with("**") {
                pattern.push_str("/*");
            }
            let paths = glob::glob_with(&pattern, glob_options).map_err(|err| {
                BuildError::new(
                    "manifest_invalid",
                    format!("files.source {:?}: {err}", entry.source),
                )
            })?;
            paths
                .filter_map(|p| p.ok())
                .filter(|p| p.is_file())
                .collect()
        } else if base_dir.is_dir() {
            walk(&base_dir)?
        } else if base_dir.is_file() {
            vec![base_dir.clone()]
        } else {
            Vec::new()
        };
        let root_for_relative = if has_glob || base_dir.is_dir() {
            base_dir.clone()
        } else {
            base_dir.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        if matched.is_empty() {
            return Err(BuildError::new(
                "files_empty",
                format!("files.source {:?} matched no files", entry.source),
            ));
        }
        for path in matched {
            let relative_path = path.strip_prefix(&root_for_relative).map_err(|_| {
                BuildError::new(
                    "files_invalid",
                    format!(
                        "{} is outside {}",
                        path.display(),
                        root_for_relative.display()
                    ),
                )
            })?;
            let relative = to_forward(relative_path);
            if excludes
                .iter()
                .any(|pattern| pattern.matches_with(&relative, exclude_options))
            {
                continue;
            }
            tigersetup_format::metadata::validate_relative_path(&relative)?;
            let size = std::fs::metadata(&path)?.len();
            let key = relative.to_ascii_lowercase();
            if let Some(existing) = files.get(&key) {
                if existing.source != path {
                    return Err(BuildError::new(
                        "files_duplicate",
                        format!(
                            "{relative} comes from both {} and {}",
                            existing.source.display(),
                            path.display()
                        ),
                    ));
                }
                if existing.when != when {
                    return Err(BuildError::new(
                        "files_duplicate",
                        format!(
                            "{relative} is declared by two [[files]] entries with different predicates"
                        ),
                    ));
                }
            }
            files.insert(
                key,
                ResolvedFile {
                    relative,
                    source: path,
                    size,
                    when: when.clone(),
                },
            );
        }
    }
    // Collisions are detected case-insensitively (Windows), but the install
    // order is plain byte order of the path, so it is the same on every
    // machine and easy to predict.
    let mut resolved: Vec<ResolvedFile> = files.into_values().collect();
    resolved.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(resolved)
}

fn walk(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                out.push(path);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, exclude: &[&str]) -> FilesEntry {
        FilesEntry {
            source: source.into(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            when: None,
        }
    }

    #[test]
    fn globs_resolve_relative_to_their_base_and_honour_excludes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("publish/sub")).unwrap();
        std::fs::write(root.join("publish/app.exe"), b"exe").unwrap();
        std::fs::write(root.join("publish/app.pdb"), b"pdb").unwrap();
        std::fs::write(root.join("publish/sub/lib.dll"), b"dll!").unwrap();
        std::fs::write(root.join("publish/sub/lib.pdb"), b"pdb").unwrap();
        std::fs::write(root.join("extra.txt"), b"x").unwrap();

        let files = resolve(
            root,
            &[entry("publish/**", &["*.pdb"]), entry("extra.txt", &[])],
        )
        .unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.relative.as_str()).collect();
        assert_eq!(names, vec!["app.exe", "extra.txt", "sub/lib.dll"]);
        assert_eq!(files[2].size, 4);
    }

    #[test]
    fn install_order_is_byte_order_of_the_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("p/bin")).unwrap();
        std::fs::write(root.join("p/CHANGELOG.md"), b"c").unwrap();
        std::fs::write(root.join("p/bin/lib.dll"), b"l").unwrap();
        std::fs::write(root.join("p/bin/App.exe"), b"a").unwrap();
        let files = resolve(root, &[entry("p/**", &[])]).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.relative.as_str()).collect();
        assert_eq!(names, vec!["CHANGELOG.md", "bin/App.exe", "bin/lib.dll"]);
    }

    #[test]
    fn an_empty_match_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve(dir.path(), &[entry("nothing/**", &[])])
                .unwrap_err()
                .code,
            "files_empty"
        );
    }
}
