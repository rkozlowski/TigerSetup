//! Action declarations become runtime action records
//! (`TigerSetup-Design.md` §5.14): the phase, the operations, the kind, the
//! program — a template the engine expands on the target, or a file the
//! builder packages under its reserved payload directory with its exact
//! size and SHA-256, the way an embedded dependency installer travels — and
//! the execution envelope.

use std::path::PathBuf;

use tigersetup_format::metadata::{
    ACTION_ENTRY_PREFIX, Action, ActionFailurePolicy, ActionKind, ActionOperation, ActionPhase,
};
use tigersetup_format::{hex, sha256};

use crate::manifest::{ActionEntry, LoadedManifest, predicate_of};
use crate::{BuildError, Result};

#[derive(Debug)]
pub struct ResolvedActions {
    pub actions: Vec<Action>,
    /// The files the payload carries for packaged actions, as `(entry name,
    /// source path)`, each once however many actions share it.
    pub packaged: Vec<(String, PathBuf)>,
}

/// The payload entry a packaged action file travels as.
pub fn action_entry(file: &str) -> String {
    format!("{ACTION_ENTRY_PREFIX}{}", file_name_of(file))
}

fn file_name_of(file: &str) -> String {
    file.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(file)
        .to_string()
}

/// The arguments as one line for a human report, each quoted where it has
/// a space, so that `a b` and `"a b"` read differently.
pub fn join_for_display(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|a| {
            if a.contains(' ') || a.is_empty() {
                format!("{a:?}")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The record a declaration yields. A packaged file is read here, so its
/// size and hash are the bytes'.
pub fn declared(entry: &ActionEntry, loaded: &LoadedManifest) -> Result<Action> {
    let phase = ActionPhase::parse(&entry.phase).unwrap_or(ActionPhase::Unspecified);
    let run_on: Vec<i32> = match &entry.run_on {
        Some(list) => list
            .iter()
            .filter_map(|text| ActionOperation::parse(text))
            .map(|op| op as i32)
            .collect(),
        None => phase
            .default_operations()
            .iter()
            .map(|op| *op as i32)
            .collect(),
    };
    let (command, packaged) = match (&entry.command, &entry.source) {
        (Some(command), _) => (command.clone(), None),
        (None, Some(source)) => {
            let path = loaded.resolve(source);
            let bytes = std::fs::read(&path).map_err(|err| {
                BuildError::new(
                    "manifest_file_unreadable",
                    format!("action {} source {}: {err}", entry.name, path.display()),
                )
            })?;
            if bytes.is_empty() {
                return Err(BuildError::new(
                    "manifest_file_unreadable",
                    format!("action {} source {} is empty", entry.name, path.display()),
                ));
            }
            (String::new(), Some((source.as_str(), bytes)))
        }
        (None, None) => (String::new(), None),
    };
    let (entry_name, file_name, size, digest) = match packaged {
        Some((source, bytes)) => (
            action_entry(source),
            file_name_of(source),
            bytes.len() as u64,
            hex(&sha256(&bytes)),
        ),
        None => (String::new(), String::new(), 0, String::new()),
    };
    Ok(Action {
        name: entry.name.to_ascii_lowercase(),
        phase: phase as i32,
        run_on,
        kind: ActionKind::parse(&entry.kind).unwrap_or(ActionKind::Unspecified) as i32,
        command,
        entry: entry_name,
        file_name,
        size,
        sha256: digest,
        arguments: entry.arguments.clone(),
        working_directory: entry.working_directory.clone().unwrap_or_default(),
        timeout_seconds: entry.timeout_seconds.unwrap_or(0),
        success_codes: entry.success_codes.clone(),
        reboot_codes: entry.reboot_codes.clone(),
        on_failure: entry
            .on_failure
            .as_deref()
            .and_then(ActionFailurePolicy::parse)
            .unwrap_or(ActionFailurePolicy::Fail) as i32,
        when: predicate_of(entry.when.as_ref(), None),
    })
}

/// Resolves every declared action. Two actions may share one packaged
/// file (the same manifest-relative source); two different files with the
/// same name cannot, because the file name is the entry.
pub fn resolve(loaded: &LoadedManifest) -> Result<ResolvedActions> {
    let mut actions = Vec::new();
    let mut packaged: Vec<(String, PathBuf)> = Vec::new();
    for entry in &loaded.manifest.actions {
        let action = declared(entry, loaded)?;
        if let Some(source) = &entry.source {
            let entry_name = action_entry(source);
            let path = loaded.resolve(source);
            match packaged
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&entry_name))
            {
                Some((_, existing)) if existing == &path => {}
                Some(_) => {
                    return Err(BuildError::new(
                        "manifest_invalid",
                        format!(
                            "action {}: two packaged files share the file name {:?}",
                            entry.name,
                            file_name_of(source)
                        ),
                    ));
                }
                None => packaged.push((entry_name, path)),
            }
        }
        actions.push(action);
    }
    Ok(ResolvedActions { actions, packaged })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    fn loaded(root: &std::path::Path, toml: &str) -> LoadedManifest {
        let manifest: Manifest = toml::from_str(toml).expect("the manifest parses");
        manifest.validate().expect("the manifest validates");
        LoadedManifest {
            path: root.join("TigerSetup.toml"),
            directory: root.to_path_buf(),
            manifest,
        }
    }

    const WITH_ACTIONS: &str = r#"
[package]
id = "Vendor.Product"
name = "Product"
version = "1.0.0"
publisher = "Vendor"

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
arguments = ["-Root", "%INSTALLROOT%", "-Version", "%VERSION%"]
timeout_seconds = 120
when = { option = "cache", equals = true }

[[actions]]
name = "clear-cache"
phase = "pre-uninstall"
kind = "powershell"
source = "actions/build-cache.ps1"
arguments = ["-Clear"]
on_failure = "continue"

[[actions]]
name = "notify"
phase = "pre-uninstall"
kind = "exe"
command = "%INSTALLROOT%\\app.exe"
arguments = ["--bye"]
success_codes = [0, 2]
reboot_codes = [3010]
"#;

    #[test]
    fn declarations_become_records_and_a_shared_file_is_packaged_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("actions")).unwrap();
        std::fs::write(
            dir.path().join("actions/build-cache.ps1"),
            b"param($Root) 'cache'",
        )
        .unwrap();
        let loaded = loaded(dir.path(), WITH_ACTIONS);
        let resolved = resolve(&loaded).unwrap();
        assert_eq!(resolved.actions.len(), 3);
        assert_eq!(resolved.packaged.len(), 1, "one file, two actions");
        assert_eq!(
            resolved.packaged[0].0,
            ".tigersetup/actions/build-cache.ps1"
        );

        let build = &resolved.actions[0];
        assert_eq!(build.name, "build-cache");
        assert_eq!(build.phase(), ActionPhase::PostInstall);
        assert_eq!(build.kind(), ActionKind::Powershell);
        assert!(build.runs_on(ActionOperation::Repair));
        assert_eq!(build.entry, ".tigersetup/actions/build-cache.ps1");
        assert_eq!(build.file_name, "build-cache.ps1");
        assert_eq!(build.size, 20);
        assert_eq!(build.sha256, hex(&sha256(b"param($Root) 'cache'")));
        assert_eq!(build.timeout_seconds, 120);
        assert_eq!(build.failure_policy(), ActionFailurePolicy::Fail);
        assert_eq!(build.when.as_ref().unwrap().option, "cache");
        assert!(build.command.is_empty());

        let clear = &resolved.actions[1];
        assert_eq!(clear.operations(), vec![ActionOperation::Uninstall]);
        assert_eq!(clear.failure_policy(), ActionFailurePolicy::Continue);
        assert_eq!(clear.timeout_seconds, 0, "the engine's default");

        let notify = &resolved.actions[2];
        assert!(!notify.is_packaged());
        assert_eq!(notify.command, "%INSTALLROOT%\\app.exe");
        assert_eq!(notify.success_codes, vec![0, 2]);
        assert_eq!(notify.reboot_codes, vec![3010]);
    }

    #[test]
    fn a_missing_source_file_stops_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = loaded(dir.path(), WITH_ACTIONS);
        assert_eq!(
            resolve(&loaded).unwrap_err().code,
            "manifest_file_unreadable"
        );
    }
}
