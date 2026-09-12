//! The four typed detectors (`TigerSetup-Design.md` §7.2, *Detection*):
//! a directory of version-named subdirectories, a registry value, a file's
//! Windows version, or an Add/Remove Programs entry. Every detector yields
//! the version it found, and the requirement decides whether it satisfies:
//! the minimum's major and not lower, or any version when no minimum is
//! declared. Detection reads the machine and nothing else — no network,
//! no mutation.
//!
//! Path templates expand the known folders the engine resolves
//! (`%PROGRAMFILES%`, `%LOCALAPPDATA%`, ...) and, after those, any
//! environment variable of the process (`%DOTNET_ROOT%`). A template whose
//! placeholder has no value describes a location that does not exist on
//! this machine, so the detector reports absence rather than failing.

use std::path::PathBuf;

use tigersetup_format::identity::{compare_versions, expand_template, version_satisfies};
use tigersetup_format::metadata::{Detector, DetectorKind};

use crate::dependency::pattern::Pattern;
use crate::win::{self, registry};
use crate::{Error, Result};

/// The three Add/Remove Programs roots a registration detector reads by
/// default.
pub const REGISTRATION_ROOTS: &[&str] = &[
    "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    "HKCU\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
];

/// What a detector found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// A version satisfying the requirement.
    Present { version: String },
    /// Nothing satisfying; `found` is the highest version seen that did not
    /// satisfy, for the log.
    Absent { found: Option<String> },
}

impl Detection {
    pub fn version(&self) -> Option<&str> {
        match self {
            Detection::Present { version } => Some(version),
            Detection::Absent { .. } => None,
        }
    }
}

/// Expands a path template; `None` when a placeholder has no value here.
pub fn expand(template: &str) -> Option<String> {
    expand_template(template, |name| {
        win::env::known_folder(name).or_else(|| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim_end_matches('\\').to_string())
                .filter(|v| !v.is_empty())
        })
    })
    .ok()
}

/// A version string a detector may report: starts with a digit, is dotted
/// numeric, and is not the "nothing" value `0.0.0.0`.
fn is_version(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        && text.split('.').any(|p| p.parse::<u64>().unwrap_or(0) != 0)
}

/// Picks the best of the versions found: a satisfying one (the highest), or
/// the highest unsatisfying one as `found`.
fn choose(versions: impl IntoIterator<Item = String>, minimum: &str) -> Detection {
    let mut best_ok: Option<String> = None;
    let mut best_any: Option<String> = None;
    for version in versions {
        if !is_version(&version) {
            continue;
        }
        if version_satisfies(&version, minimum)
            && best_ok
                .as_deref()
                .is_none_or(|b| compare_versions(&version, b).is_gt())
        {
            best_ok = Some(version.clone());
        }
        if best_any
            .as_deref()
            .is_none_or(|b| compare_versions(&version, b).is_gt())
        {
            best_any = Some(version);
        }
    }
    match best_ok {
        Some(version) => Detection::Present { version },
        None => Detection::Absent { found: best_any },
    }
}

fn pattern_of(detector: &Detector) -> Result<Option<Pattern>> {
    if detector.pattern.is_empty() {
        Ok(None)
    } else {
        Pattern::compile(&detector.pattern).map(Some)
    }
}

fn directory_version(detector: &Detector, minimum: &str) -> Result<Detection> {
    let pattern = pattern_of(detector)?;
    let Some(root) = expand(&detector.path) else {
        return Ok(Detection::Absent { found: None });
    };
    let Ok(entries) = std::fs::read_dir(PathBuf::from(&root)) else {
        return Ok(Detection::Absent { found: None });
    };
    let names = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| pattern.as_ref().is_none_or(|p| p.is_match(name)));
    Ok(choose(names, minimum))
}

fn registry_version(detector: &Detector, minimum: &str) -> Result<Detection> {
    let mut found = Vec::new();
    for key in &detector.keys {
        if let Some(key) = registry::open_native(key)
            && let Some(value) = key.string_value(&detector.value)
        {
            found.push(value);
        }
    }
    Ok(choose(found, minimum))
}

fn file_version(detector: &Detector, minimum: &str) -> Result<Detection> {
    let Some(path) = expand(&detector.path) else {
        return Ok(Detection::Absent { found: None });
    };
    Ok(choose(
        win::file_version::file_version(&PathBuf::from(path)),
        minimum,
    ))
}

fn registration(detector: &Detector, minimum: &str) -> Result<Detection> {
    let pattern = Pattern::compile(&detector.pattern)?;
    let roots: Vec<String> = if detector.keys.is_empty() {
        REGISTRATION_ROOTS.iter().map(|r| r.to_string()).collect()
    } else {
        detector.keys.clone()
    };
    let mut found = Vec::new();
    for root in roots {
        let Some(root) = registry::open_native(&root) else {
            continue;
        };
        for name in root.subkey_names() {
            let Some(entry) = root.open_subkey(&name) else {
                continue;
            };
            let display_name = entry.string_value("DisplayName").unwrap_or_default();
            if display_name.is_empty() || !pattern.is_match(&display_name) {
                continue;
            }
            if let Some(version) = entry.string_value("DisplayVersion") {
                found.push(version);
            }
        }
    }
    Ok(choose(found, minimum))
}

/// Runs the detector against this machine.
pub fn detect(detector: &Detector, minimum: &str) -> Result<Detection> {
    match DetectorKind::try_from(detector.kind) {
        Ok(DetectorKind::DirectoryVersion) => directory_version(detector, minimum),
        Ok(DetectorKind::RegistryVersion) => registry_version(detector, minimum),
        Ok(DetectorKind::FileVersion) => file_version(detector, minimum),
        Ok(DetectorKind::Registration) => registration(detector, minimum),
        _ => Err(Error::new(
            "metadata_invalid",
            format!("unknown detector kind {}", detector.kind),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win::registry::scratch::ScratchKey;

    fn detector(kind: DetectorKind) -> Detector {
        Detector {
            kind: kind as i32,
            ..Default::default()
        }
    }

    #[test]
    fn directory_versions_pick_the_highest_satisfying_subdirectory() {
        // The directory is made inside %TEMP% so that the template can name it
        // through a variable the process already has: setting one here would
        // race every other test thread reading the environment, which is why
        // `set_var` is unsafe in this edition.
        let temp = std::env::var("TEMP").unwrap();
        let dir = tempfile::Builder::new()
            .prefix("detect-")
            .tempdir_in(&temp)
            .unwrap();
        for name in ["9.0.5", "10.0.9", "10.0.11", "11.0.0-preview", "notes.txt"] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("10.0.99"), b"a file, not a directory").unwrap();
        let leaf = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut d = detector(DetectorKind::DirectoryVersion);
        d.path = format!("%TEMP%\\{leaf}");
        d.pattern = "10\\..*".into();
        assert_eq!(
            detect(&d, "10.0").unwrap(),
            Detection::Present {
                version: "10.0.11".into()
            }
        );
        assert_eq!(
            detect(&d, "10.0.12").unwrap(),
            Detection::Absent {
                found: Some("10.0.11".into())
            }
        );
        d.pattern = String::new();
        assert_eq!(
            detect(&d, "9.0").unwrap(),
            Detection::Present {
                version: "9.0.5".into()
            }
        );
        assert_eq!(
            detect(&d, "11.0").unwrap(),
            Detection::Absent {
                found: Some("10.0.11".into())
            }
        );
        d.path = "%TIGERSETUP_TEST_ROOT%\\missing".into();
        assert_eq!(detect(&d, "").unwrap(), Detection::Absent { found: None });
        d.path = "%TIGERSETUP_UNSET_VARIABLE%\\dotnet".into();
        assert_eq!(detect(&d, "").unwrap(), Detection::Absent { found: None });
        d.pattern = "(".into();
        assert_eq!(detect(&d, "").unwrap_err().code, "metadata_invalid");
    }

    #[test]
    fn registry_versions_try_keys_in_order_and_ignore_the_nothing_value() {
        let scratch = ScratchKey::new();
        scratch.set("Clients\\{A}", &[("pv", "0.0.0.0")]);
        scratch.set("Clients\\{B}", &[("pv", "152.0.4191.53")]);
        scratch.set("Clients\\{C}", &[("pv", "")]);
        let mut d = detector(DetectorKind::RegistryVersion);
        d.keys = vec![
            format!("{}\\Clients\\{{A}}", scratch.path),
            format!("{}\\Missing", scratch.path),
            format!("{}\\Clients\\{{B}}", scratch.path),
            format!("{}\\Clients\\{{C}}", scratch.path),
        ];
        d.value = "pv".into();
        assert_eq!(
            detect(&d, "").unwrap(),
            Detection::Present {
                version: "152.0.4191.53".into()
            }
        );
        assert_eq!(
            detect(&d, "153.0").unwrap(),
            Detection::Absent {
                found: Some("152.0.4191.53".into())
            }
        );
        d.keys = vec![format!("{}\\Clients\\{{A}}", scratch.path)];
        assert_eq!(detect(&d, "").unwrap(), Detection::Absent { found: None });
    }

    #[test]
    fn file_versions_come_from_the_version_resource() {
        let mut d = detector(DetectorKind::FileVersion);
        d.path = "%SystemRoot%\\System32\\kernel32.dll".into();
        // A system DLL's own version resource is the expectation: Windows
        // stamps kernel32 with a compatibility major of its own choosing.
        let actual = win::file_version::file_version(&PathBuf::from(expand(&d.path).unwrap()))
            .expect("a system DLL carries a version resource");
        assert_eq!(
            detect(&d, "").unwrap(),
            Detection::Present {
                version: actual.clone()
            }
        );
        let major = actual.split('.').next().unwrap();
        assert_eq!(
            detect(&d, &format!("{major}.0")).unwrap().version(),
            Some(actual.as_str())
        );
        assert_eq!(
            detect(&d, "99.0").unwrap(),
            Detection::Absent {
                found: Some(actual)
            }
        );
        d.path = "%SystemRoot%\\System32\\no-such-file.dll".into();
        assert_eq!(detect(&d, "").unwrap(), Detection::Absent { found: None });
    }

    #[test]
    fn registrations_match_display_names_under_the_given_roots() {
        let scratch = ScratchKey::new();
        scratch.set(
            "Uninstall\\{96749152}",
            &[
                (
                    "DisplayName",
                    "Microsoft Windows Desktop Runtime 10.0.11 (x64)",
                ),
                ("DisplayVersion", "10.0.11.50000"),
            ],
        );
        scratch.set(
            "Uninstall\\{6E0832BF}",
            &[
                (
                    "DisplayName",
                    "Microsoft Windows Desktop Runtime 10.0.11 (x86)",
                ),
                ("DisplayVersion", "10.0.11.50000"),
            ],
        );
        scratch.set("Uninstall\\NoVersion", &[("DisplayName", "Something Else")]);
        let mut d = detector(DetectorKind::Registration);
        d.keys = vec![format!("{}\\Uninstall", scratch.path)];
        d.pattern = "Microsoft Windows Desktop Runtime .* \\(x64\\)".into();
        assert_eq!(
            detect(&d, "10.0").unwrap(),
            Detection::Present {
                version: "10.0.11.50000".into()
            }
        );
        assert_eq!(detect(&d, "11.0").unwrap().version(), None);
        d.pattern = "Nothing Like This".into();
        assert_eq!(detect(&d, "").unwrap(), Detection::Absent { found: None });
        // The default roots are real Add/Remove Programs locations; nothing
        // there carries a display name that cannot exist.
        d.keys.clear();
        d.pattern = "TigerSetup test entry that no machine has [0-9]{12}".into();
        assert_eq!(detect(&d, "").unwrap(), Detection::Absent { found: None });
    }

    #[test]
    fn version_strings_are_recognised() {
        assert!(is_version("10.0.11"));
        assert!(is_version("152"));
        assert!(!is_version("0.0.0.0"));
        assert!(!is_version(""));
        assert!(!is_version("11.0.0-preview"));
        assert!(!is_version("v1"));
    }
}
