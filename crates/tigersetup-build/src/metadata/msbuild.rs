//! The MSBuild metadata provider: evaluated properties, never parsed XML
//! (`TigerSetup-Design.md` §9.1). A value may come from `Version.props`, a
//! `Directory.Build.props`, an imported `.targets`, an SDK default or a
//! command-line property, so the only correct reading of a project is the
//! one MSBuild itself performs.
//!
//! `dotnet msbuild <project> -getProperty:A -getProperty:B …` evaluates the
//! project without running a target and prints one JSON object; a global
//! property is passed as `-p:Name=Value`. Requesting two or more properties
//! is what makes the output an object — a single `-getProperty` prints the
//! bare value instead — so the provider always asks for the whole set.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{BuildError, Result};

/// The evaluation command; `dotnet` is resolved on `PATH`.
pub const PROGRAM: &str = "dotnet";

/// The properties the provider evaluates, in the order it asks for them.
pub const PROPERTIES: [&str; 8] = [
    "Version",
    "Product",
    "Company",
    "Description",
    "Copyright",
    "FileVersion",
    "InformationalVersion",
    "AssemblyName",
];

/// What one evaluation of a project yielded. A property MSBuild does not
/// define comes back as an empty string.
#[derive(Debug, Clone)]
pub struct Evaluation {
    /// The project as the manifest named it, for provenance.
    pub project: PathBuf,
    pub values: BTreeMap<String, String>,
}

impl Evaluation {
    /// An evaluated property, or `""` when MSBuild defined none.
    pub fn get(&self, name: &str) -> &str {
        self.values.get(name).map(String::as_str).unwrap_or("")
    }
}

/// The command line the provider runs, for diagnostics and for the
/// unavailability error.
pub fn command_line(project: &Path, properties: &[(String, String)]) -> String {
    let mut line = format!("{PROGRAM} msbuild {}", project.display());
    for name in PROPERTIES {
        line.push_str(&format!(" -getProperty:{name}"));
    }
    for (name, value) in properties {
        line.push_str(&format!(" -p:{name}={value}"));
    }
    line
}

/// Evaluates `project` and returns the properties MSBuild resolved.
pub fn evaluate(project: &Path, properties: &[(String, String)]) -> Result<Evaluation> {
    if !project.is_file() {
        return Err(BuildError::new(
            "metadata_source_unreadable",
            format!("the MSBuild project {} does not exist", project.display()),
        ));
    }
    let mut command = Command::new(PROGRAM);
    command.arg("msbuild").arg(project);
    for name in PROPERTIES {
        command.arg(format!("-getProperty:{name}"));
    }
    for (name, value) in properties {
        command.arg(format!("-p:{name}={value}"));
    }
    // MSBuild resolves imports relative to the project file, but running in
    // the project's directory keeps any tool that consults the working
    // directory seeing what a normal build would.
    if let Some(directory) = project.parent().filter(|d| d.is_dir()) {
        command.current_dir(directory);
    }
    let line = command_line(project, properties);
    let output = command.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            BuildError::new(
                "metadata_provider_unavailable",
                format!("`{line}` cannot run: {PROGRAM} is not on PATH ({err})"),
            )
        } else {
            BuildError::new(
                "metadata_provider_unavailable",
                format!("`{line}` cannot run: {err}"),
            )
        }
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail: String = [stderr.trim(), stdout.trim()]
            .iter()
            .filter(|part| !part.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" / ");
        return Err(BuildError::new(
            "metadata_provider_failed",
            format!(
                "`{line}` exited with {}: {detail}",
                output.status.code().unwrap_or(-1)
            ),
        ));
    }
    let values = parse(&stdout).map_err(|message| {
        BuildError::new(
            "metadata_provider_failed",
            format!("`{line}` printed no usable property object: {message}"),
        )
    })?;
    Ok(Evaluation {
        project: project.to_path_buf(),
        values,
    })
}

/// Reads the `{"Properties": {…}}` document `-getProperty` prints. MSBuild
/// escapes non-ASCII and `+` as `\uXXXX`, which the JSON reader undoes.
pub fn parse(stdout: &str) -> std::result::Result<BTreeMap<String, String>, String> {
    let document: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|err| err.to_string())?;
    let properties = document
        .get("Properties")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "the document has no Properties object".to_string())?;
    let mut values = BTreeMap::new();
    for (name, value) in properties {
        let text = value
            .as_str()
            .ok_or_else(|| format!("property {name} is not a string"))?;
        values.insert(name.clone(), text.to_string());
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `dotnet msbuild … -getProperty:…` prints on .NET SDK 10.0.400 for
    /// a project importing a `Version.props` that builds its informational
    /// version as `$(Version)+$(UtcBuildTimestamp)`.
    const OBSERVED_OUTPUT: &str = r#"{
  "Properties": {
    "Version": "0.8.1",
    "Product": "TigerMarkView",
    "Company": "IT Tiger",
    "Description": "A local Markdown viewer, reviewer and PDF exporter.",
    "Copyright": "Copyright (c) 2026 Ryszard Kozlowski",
    "FileVersion": "0.8.1.0",
    "InformationalVersion": "0.8.1+20260907.175518",
    "AssemblyName": "TigerMarkView"
  }
}
"#;

    #[test]
    fn the_evaluated_property_document_is_read_as_a_string_map() {
        let values = parse(OBSERVED_OUTPUT).unwrap();
        assert_eq!(values["Version"], "0.8.1");
        assert_eq!(values["Company"], "IT Tiger");
        assert_eq!(values["FileVersion"], "0.8.1.0");
        // The escaped plus sign is the reason the output is read as JSON
        // rather than scanned line by line.
        assert_eq!(values["InformationalVersion"], "0.8.1+20260907.175518");
        assert_eq!(values.len(), PROPERTIES.len());
    }

    #[test]
    fn output_that_is_not_a_property_document_is_refused() {
        assert!(parse("MSBUILD : error MSB1009: Project file does not exist.").is_err());
        assert!(parse("{}").is_err());
        assert!(parse(r#"{"Properties": {"Version": 1}}"#).is_err());
    }

    #[test]
    fn the_command_line_names_every_requested_property() {
        let line = command_line(
            Path::new("App.csproj"),
            &[("Configuration".into(), "Release".into())],
        );
        assert!(line.starts_with("dotnet msbuild App.csproj -getProperty:Version"));
        assert!(line.ends_with("-p:Configuration=Release"));
        for name in PROPERTIES {
            assert!(line.contains(&format!("-getProperty:{name}")), "{name}");
        }
    }

    #[test]
    fn a_project_that_does_not_exist_is_refused_before_the_provider_runs() {
        let err = evaluate(Path::new("no-such-project.csproj"), &[]).unwrap_err();
        assert_eq!(err.code, "metadata_source_unreadable");
    }
}
