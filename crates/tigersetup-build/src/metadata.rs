//! Product metadata resolution with provenance (`TigerSetup-Design.md` §9):
//! the `static` provider reads `[package]`, the `msbuild` provider evaluates
//! the project's properties and the `exe` provider reads a built binary's
//! `VERSIONINFO`. A value typed in `[package]` wins over a provider's value,
//! every candidate is reported with where it came from, and a disagreement
//! between the source and the built binary is a build error rather than a
//! stale release artifact.
//!
//! Reading the providers and combining what they returned are separate:
//! [`resolve_package`] performs the input and output, [`combine`] decides
//! precedence and validates, so the rules hold for any provider.

pub mod msbuild;
pub mod version_info;

use std::collections::BTreeMap;

use tigersetup_format::identity;

use crate::manifest::{LoadedManifest, MetadataSource, PackageSection};
use crate::{BuildError, Result};

use msbuild::Evaluation;
use version_info::{VersionInfo, without_build_metadata};

/// The provider a value came from. Stable identifiers: machine-readable
/// output names these, never localized text.
pub const SOURCE_STATIC: &str = "static";
pub const SOURCE_MSBUILD: &str = "msbuild-project";
pub const SOURCE_VERSION_INFO: &str = "pe-version-info";

/// One candidate value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub value: String,
    /// [`SOURCE_STATIC`], [`SOURCE_MSBUILD`] or [`SOURCE_VERSION_INFO`].
    pub source: &'static str,
    /// The manifest key, MSBuild property or `VERSIONINFO` field read.
    pub origin: String,
    /// The project or executable the value was read from; empty for a value
    /// typed in the manifest. Evaluated MSBuild properties cannot say which
    /// imported file defined them, so this is the project that was evaluated.
    pub file: String,
    /// Whether this candidate is the value the build uses. A candidate that
    /// a higher-precedence value displaced stays in the report so a
    /// developer can compare the two.
    pub effective: bool,
}

impl Provenance {
    fn new(
        value: impl Into<String>,
        source: &'static str,
        origin: impl Into<String>,
    ) -> Provenance {
        Provenance {
            value: value.into(),
            source,
            origin: origin.into(),
            file: String::new(),
            effective: false,
        }
    }

    fn read_from(mut self, file: impl Into<String>) -> Provenance {
        self.file = file.into();
        self
    }

    /// `Version of ..\src\App.csproj` — what was read and where from, for a
    /// message a developer can act on.
    pub fn describe(&self) -> String {
        if self.file.is_empty() {
            self.origin.clone()
        } else {
            format!("{} of {}", self.origin, self.file)
        }
    }
}

/// A check the build performed between the declared source and the built
/// binary, and the fact it established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    /// A stable identifier for the check.
    pub check: &'static str,
    pub detail: String,
}

/// The product metadata a build resolved.
#[derive(Debug, Clone, Default)]
pub struct ResolvedPackage {
    pub version: String,
    pub description: String,
    pub copyright: String,
    /// Four-part Windows file version, when one was read.
    pub file_version: String,
    /// The version with its build metadata, when a source carries one.
    pub informational_version: String,
    /// Every candidate value with its provenance, in field and precedence
    /// order, for `tiger-setup metadata`.
    pub provenance: Vec<(String, Provenance)>,
    /// Checks the build performed between the source and the binary.
    pub validations: Vec<Validation>,
}

impl ResolvedPackage {
    /// The provenance of the value the build uses for a field.
    pub fn effective(&self, field: &str) -> Option<&Provenance> {
        self.provenance
            .iter()
            .find(|(name, p)| name == field && p.effective)
            .map(|(_, p)| p)
    }

    /// Every candidate recorded for a field, in precedence order.
    pub fn candidates(&self, field: &str) -> Vec<&Provenance> {
        self.provenance
            .iter()
            .filter(|(name, _)| name == field)
            .map(|(_, p)| p)
            .collect()
    }

    /// Records every candidate for a field in precedence order and returns
    /// the first, which is the value the build uses.
    fn record<const N: usize>(
        &mut self,
        field: &str,
        candidates: [Option<Provenance>; N],
    ) -> String {
        let mut effective = String::new();
        for mut candidate in candidates.into_iter().flatten() {
            if effective.is_empty() {
                effective = candidate.value.clone();
                candidate.effective = true;
            }
            self.provenance.push((field.to_string(), candidate));
        }
        effective
    }
}

/// The MSBuild properties an evaluation runs with: the manifest's own, with
/// the command line's taking precedence.
pub fn merged_properties(
    declared: &BTreeMap<String, String>,
    overrides: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = declared.clone();
    for (name, value) in overrides {
        merged.insert(name.clone(), value.clone());
    }
    merged.into_iter().collect()
}

/// Resolves the product metadata for the manifest's `[metadata]` source,
/// running whichever providers it declares.
pub fn resolve_package(
    loaded: &LoadedManifest,
    properties: &[(String, String)],
) -> Result<ResolvedPackage> {
    let section = &loaded.manifest.metadata;
    let evaluation = match section.source {
        MetadataSource::Msbuild => {
            let project = section.project.as_deref().ok_or_else(|| {
                BuildError::new(
                    "manifest_invalid",
                    "metadata.project is required when metadata.source is msbuild",
                )
            })?;
            let merged = merged_properties(&section.properties, properties);
            Some(msbuild::evaluate(&loaded.resolve(project), &merged)?)
        }
        MetadataSource::Static | MetadataSource::Exe => None,
    };
    // For the `exe` source the executable is the provider; for the others it
    // is the binary the declared source is validated against.
    let binary = match &section.executable {
        Some(executable) => Some(version_info::read(&loaded.resolve(executable))?),
        None => None,
    };
    combine(
        &loaded.manifest.package,
        section.source,
        evaluation.as_ref(),
        binary.as_ref(),
    )
}

/// Applies precedence to what the providers returned and validates the
/// built binary against the declared source.
pub fn combine(
    package: &PackageSection,
    source: MetadataSource,
    evaluation: Option<&Evaluation>,
    binary: Option<&VersionInfo>,
) -> Result<ResolvedPackage> {
    let project_file = evaluation
        .map(|e| e.project.display().to_string())
        .unwrap_or_default();
    let binary_file = binary
        .map(|b| b.path.display().to_string())
        .unwrap_or_default();
    let no_binary = VersionInfo::default();
    let binary_values = binary.unwrap_or(&no_binary);

    let from_project = |property: &str| -> Option<Provenance> {
        let value = evaluation.map(|e| e.get(property)).unwrap_or("");
        (!value.is_empty())
            .then(|| Provenance::new(value, SOURCE_MSBUILD, property).read_from(&project_file))
    };
    let from_binary = |field: &str, value: &str| -> Option<Provenance> {
        (!value.is_empty())
            .then(|| Provenance::new(value, SOURCE_VERSION_INFO, field).read_from(&binary_file))
    };
    let typed = |field: &str, value: &Option<String>| -> Option<Provenance> {
        value
            .as_deref()
            .filter(|v| !v.is_empty())
            .map(|v| Provenance::new(v, SOURCE_STATIC, format!("package.{field}")))
    };

    let mut resolved = ResolvedPackage::default();
    resolved.version = resolved.record(
        "version",
        [
            typed("version", &package.version),
            from_project("Version").map(strip_build_metadata),
            from_binary(
                version_info::PRODUCT_VERSION,
                &binary_values.product_version,
            )
            .map(strip_build_metadata),
        ],
    );
    if resolved.version.is_empty() {
        return Err(BuildError::new(
            "metadata_incomplete",
            format!(
                "no product version: neither package.version nor {} provides one",
                describe_source(source)
            ),
        ));
    }
    identity::validate_version(&resolved.version)?;

    resolved.description = resolved.record(
        "description",
        [
            typed("description", &package.description),
            from_project("Description"),
            from_binary(
                if binary_values.comments.is_empty() {
                    version_info::FILE_DESCRIPTION
                } else {
                    version_info::COMMENTS
                },
                binary_values.description(),
            ),
        ],
    );
    resolved.copyright = resolved.record(
        "copyright",
        [
            typed("copyright", &package.copyright),
            from_project("Copyright"),
            from_binary(
                version_info::LEGAL_COPYRIGHT,
                &binary_values.legal_copyright,
            ),
        ],
    );
    // The built binary is the authority for the four-part file version; the
    // project's property is what is left when no binary is given.
    resolved.file_version = resolved.record(
        "file_version",
        [
            from_binary("FileVersion", &binary_values.file_version),
            from_project("FileVersion"),
        ],
    );
    resolved.informational_version = resolved.record(
        "informational_version",
        [
            from_project("InformationalVersion"),
            from_binary(
                version_info::PRODUCT_VERSION,
                &binary_values.product_version,
            ),
        ],
    );

    if let Some(binary) = binary {
        validate_against_binary(&mut resolved, binary, &package.name, &package.publisher)?;
    }
    Ok(resolved)
}

fn strip_build_metadata(mut provenance: Provenance) -> Provenance {
    provenance.value = without_build_metadata(&provenance.value).to_string();
    provenance
}

fn describe_source(source: MetadataSource) -> &'static str {
    match source {
        MetadataSource::Static => "the static metadata source",
        MetadataSource::Msbuild => "the evaluated MSBuild project",
        MetadataSource::Exe => "the executable's VERSIONINFO",
    }
}

/// Compares the built binary against the declared source
/// (`TigerSetup-Design.md` §9.4). A disagreement means the binary is stale,
/// or was built from something else, and stops the build.
fn validate_against_binary(
    resolved: &mut ResolvedPackage,
    binary: &VersionInfo,
    name: &str,
    publisher: &str,
) -> Result<()> {
    let file = binary.path.display().to_string();
    let mismatch = |field: &str, actual: &str, expected: &str, origin: String| {
        BuildError::new(
            "metadata_mismatch",
            format!(
                "{field} {actual:?} of {file} does not match {expected:?} ({origin}); \
                 the executable is stale or was built from another source"
            ),
        )
    };

    // The declared version against the binary's, unless the binary is itself
    // where the version came from and there is nothing to compare.
    let version_origin = resolved.effective("version");
    let version_is_the_binarys =
        version_origin.is_some_and(|p| p.source == SOURCE_VERSION_INFO && p.file == file);
    if !version_is_the_binarys {
        let origin = version_origin.map(Provenance::describe).unwrap_or_default();
        let actual = without_build_metadata(&binary.product_version);
        if actual != resolved.version {
            return Err(mismatch(
                version_info::PRODUCT_VERSION,
                actual,
                &resolved.version,
                origin,
            ));
        }
        resolved.validations.push(Validation {
            check: "binary_product_version_matches",
            detail: format!(
                "{} {actual} of {file} matches the package version ({origin})",
                version_info::PRODUCT_VERSION
            ),
        });
    }
    if binary.company_name != publisher {
        return Err(mismatch(
            version_info::COMPANY_NAME,
            &binary.company_name,
            publisher,
            "package.publisher".into(),
        ));
    }
    resolved.validations.push(Validation {
        check: "binary_company_name_matches_publisher",
        detail: format!(
            "{} {} of {file} matches package.publisher",
            version_info::COMPANY_NAME,
            binary.company_name
        ),
    });
    // A binary without a ProductName says nothing about the package name.
    if !binary.product_name.is_empty() {
        if binary.product_name != name {
            return Err(mismatch(
                version_info::PRODUCT_NAME,
                &binary.product_name,
                name,
                "package.name".into(),
            ));
        }
        resolved.validations.push(Validation {
            check: "binary_product_name_matches_package_name",
            detail: format!(
                "{} {} of {file} matches package.name",
                version_info::PRODUCT_NAME,
                binary.product_name
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn package(body: &str) -> PackageSection {
        let text = format!("id = \"IT-Tiger.Sample\"\nname = \"Sample\"\n{body}\n");
        toml::from_str(&text).unwrap()
    }

    fn evaluation(pairs: &[(&str, &str)]) -> Evaluation {
        Evaluation {
            project: PathBuf::from("src/Sample.csproj"),
            values: pairs
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        }
    }

    fn binary(product_version: &str, product_name: &str, company: &str) -> VersionInfo {
        VersionInfo {
            path: PathBuf::from("publish/Sample.exe"),
            product_version: product_version.into(),
            product_name: product_name.into(),
            company_name: company.into(),
            legal_copyright: "(c) 2026 IT Tiger".into(),
            file_description: "Sample application".into(),
            file_version: "1.2.3.0".into(),
            translation: "040904b0".into(),
            ..VersionInfo::default()
        }
    }

    #[test]
    fn typed_values_are_used_and_reported_as_static() {
        let package = package(
            "publisher = \"IT Tiger\"\nversion = \"1.2.3\"\n\
             description = \"A sample.\"\ncopyright = \"(c) 2026\"",
        );
        let resolved = combine(&package, MetadataSource::Static, None, None).unwrap();
        assert_eq!(resolved.version, "1.2.3");
        assert_eq!(resolved.description, "A sample.");
        assert_eq!(resolved.copyright, "(c) 2026");
        assert_eq!(resolved.file_version, "");
        assert!(resolved.validations.is_empty());
        let version = resolved.effective("version").unwrap();
        assert_eq!(version.source, SOURCE_STATIC);
        assert_eq!(version.origin, "package.version");
        assert!(version.file.is_empty());
    }

    #[test]
    fn the_evaluated_project_supplies_what_the_manifest_does_not_type() {
        let package = package("publisher = \"IT Tiger\"");
        let evaluation = evaluation(&[
            ("Version", "1.2.3"),
            ("Description", "From the project."),
            ("Copyright", "(c) 2026 IT Tiger"),
            ("FileVersion", "1.2.3.0"),
            ("InformationalVersion", "1.2.3+20260907.101500"),
        ]);
        let resolved = combine(&package, MetadataSource::Msbuild, Some(&evaluation), None).unwrap();
        assert_eq!(resolved.version, "1.2.3");
        assert_eq!(resolved.description, "From the project.");
        assert_eq!(resolved.copyright, "(c) 2026 IT Tiger");
        assert_eq!(resolved.file_version, "1.2.3.0");
        assert_eq!(resolved.informational_version, "1.2.3+20260907.101500");
        let version = resolved.effective("version").unwrap();
        assert_eq!(version.source, SOURCE_MSBUILD);
        assert_eq!(version.origin, "Version");
        assert!(version.file.ends_with("Sample.csproj"));
    }

    #[test]
    fn build_metadata_never_reaches_the_package_version() {
        let package = package("publisher = \"IT Tiger\"");
        let evaluation = evaluation(&[("Version", "1.2.3+20260907.101500")]);
        let resolved = combine(&package, MetadataSource::Msbuild, Some(&evaluation), None).unwrap();
        assert_eq!(resolved.version, "1.2.3");
    }

    #[test]
    fn a_typed_value_displaces_a_providers_but_both_are_reported() {
        let package = package("publisher = \"IT Tiger\"\ndescription = \"Typed in the manifest.\"");
        let evaluation = evaluation(&[("Version", "1.2.3"), ("Description", "From the project.")]);
        let resolved = combine(&package, MetadataSource::Msbuild, Some(&evaluation), None).unwrap();
        assert_eq!(resolved.description, "Typed in the manifest.");
        let candidates = resolved.candidates("description");
        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].effective);
        assert_eq!(candidates[0].source, SOURCE_STATIC);
        assert!(!candidates[1].effective);
        assert_eq!(candidates[1].source, SOURCE_MSBUILD);
        assert_eq!(candidates[1].value, "From the project.");
    }

    #[test]
    fn the_executable_is_the_source_when_the_manifest_names_no_project() {
        let package = package("publisher = \"IT Tiger\"");
        let binary = binary("1.2.3+abc123", "Sample", "IT Tiger");
        let resolved = combine(&package, MetadataSource::Exe, None, Some(&binary)).unwrap();
        assert_eq!(resolved.version, "1.2.3");
        assert_eq!(resolved.file_version, "1.2.3.0");
        assert_eq!(resolved.informational_version, "1.2.3+abc123");
        assert_eq!(resolved.description, "Sample application");
        assert_eq!(
            resolved.effective("version").unwrap().source,
            SOURCE_VERSION_INFO
        );
        // The binary is where the version came from, so it is not compared
        // with itself; the publisher and the product name still are.
        let checks: Vec<&str> = resolved.validations.iter().map(|v| v.check).collect();
        assert_eq!(
            checks,
            [
                "binary_company_name_matches_publisher",
                "binary_product_name_matches_package_name"
            ]
        );
    }

    #[test]
    fn the_project_and_a_matching_binary_are_recorded_as_validated() {
        let package = package("publisher = \"IT Tiger\"");
        let evaluation = evaluation(&[("Version", "1.2.3")]);
        let binary = binary("1.2.3", "Sample", "IT Tiger");
        let resolved = combine(
            &package,
            MetadataSource::Msbuild,
            Some(&evaluation),
            Some(&binary),
        )
        .unwrap();
        assert_eq!(resolved.version, "1.2.3");
        let checks: Vec<&str> = resolved.validations.iter().map(|v| v.check).collect();
        assert_eq!(
            checks,
            [
                "binary_product_version_matches",
                "binary_company_name_matches_publisher",
                "binary_product_name_matches_package_name"
            ]
        );
        assert!(
            resolved.validations[0].detail.contains("Sample.csproj"),
            "{:?}",
            resolved.validations[0]
        );
    }

    #[test]
    fn a_stale_executable_stops_the_build() {
        let package = package("publisher = \"IT Tiger\"");
        let evaluation = evaluation(&[("Version", "0.9.0")]);
        let binary = binary("0.8.0", "Sample", "IT Tiger");
        let err = combine(
            &package,
            MetadataSource::Msbuild,
            Some(&evaluation),
            Some(&binary),
        )
        .unwrap_err();
        assert_eq!(err.code, "metadata_mismatch");
        assert!(err.message.contains("0.9.0"), "{}", err.message);
        assert!(err.message.contains("0.8.0"), "{}", err.message);
        assert!(err.message.contains("Version of"), "{}", err.message);
        assert!(err.message.contains("Sample.exe"), "{}", err.message);
    }

    #[test]
    fn a_publisher_the_binary_does_not_agree_with_stops_the_build() {
        let package = package("publisher = \"Someone Else\"");
        let binary = binary("1.2.3", "Sample", "IT Tiger");
        let err = combine(&package, MetadataSource::Exe, None, Some(&binary)).unwrap_err();
        assert_eq!(err.code, "metadata_mismatch");
        assert!(err.message.contains("Someone Else"), "{}", err.message);
        assert!(err.message.contains("IT Tiger"), "{}", err.message);
    }

    #[test]
    fn a_product_name_the_binary_does_not_agree_with_stops_the_build() {
        let package = package("publisher = \"IT Tiger\"");
        let binary = binary("1.2.3", "Something Else", "IT Tiger");
        let err = combine(&package, MetadataSource::Exe, None, Some(&binary)).unwrap_err();
        assert_eq!(err.code, "metadata_mismatch");
        assert!(err.message.contains("Something Else"), "{}", err.message);
        assert!(err.message.contains("Sample"), "{}", err.message);
    }

    #[test]
    fn a_binary_without_a_product_name_says_nothing_about_the_package_name() {
        let package = package("publisher = \"IT Tiger\"");
        let binary = binary("1.2.3", "", "IT Tiger");
        let resolved = combine(&package, MetadataSource::Exe, None, Some(&binary)).unwrap();
        let checks: Vec<&str> = resolved.validations.iter().map(|v| v.check).collect();
        assert_eq!(checks, ["binary_company_name_matches_publisher"]);
    }

    #[test]
    fn a_source_that_provides_no_version_is_refused() {
        let without_version = package("publisher = \"IT Tiger\"");
        let err = combine(&without_version, MetadataSource::Msbuild, None, None).unwrap_err();
        assert_eq!(err.code, "metadata_incomplete");
        let two_part = package("publisher = \"IT Tiger\"\nversion = \"1.2\"");
        assert_eq!(
            combine(&two_part, MetadataSource::Static, None, None)
                .unwrap_err()
                .code,
            "package_version_invalid"
        );
    }

    #[test]
    fn command_line_properties_win_over_the_manifests_own() {
        let declared = BTreeMap::from([
            ("Configuration".to_string(), "Debug".to_string()),
            ("Platform".to_string(), "AnyCPU".to_string()),
        ]);
        let merged = merged_properties(
            &declared,
            &[("Configuration".to_string(), "Release".to_string())],
        );
        assert_eq!(
            merged,
            vec![
                ("Configuration".to_string(), "Release".to_string()),
                ("Platform".to_string(), "AnyCPU".to_string()),
            ]
        );
    }

    /// A manifest whose `[package]` and `[metadata]` bodies the test writes.
    fn manifest_with(directory: &Path, package: &str, metadata: &str) -> LoadedManifest {
        let text = format!(
            "[package]\nid = \"IT-Tiger.Sample\"\nname = \"Sample\"\n{package}\n\
             {metadata}\n[[files]]\nsource = \"payload/**\"\n"
        );
        let path = directory.join("TigerSetup.toml");
        std::fs::write(&path, text).unwrap();
        std::fs::create_dir_all(directory.join("payload")).unwrap();
        LoadedManifest::load(&path).unwrap()
    }

    /// The whole path from a manifest to resolved metadata, through the real
    /// MSBuild provider. `dotnet` is a hard requirement of the msbuild
    /// source, so its absence is a failure and not a reason to skip.
    #[test]
    fn a_manifest_resolves_its_version_from_the_project_it_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Version.props"),
            "<Project>\n  <PropertyGroup>\n    <Version>4.5.6</Version>\n\
             <InformationalVersion>$(Version)+20260907.101500</InformationalVersion>\n\
             <Company>IT Tiger</Company>\n    <Product>Sample</Product>\n\
             <Copyright>(c) 2026 IT Tiger</Copyright>\n\
             <Description>Resolved from the project.</Description>\n\
             </PropertyGroup>\n</Project>\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Sample.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <Import Project=\"Version.props\" />\n\
             <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n\
             <FileVersion>$(Version).0</FileVersion>\n  </PropertyGroup>\n</Project>\n",
        )
        .unwrap();
        let loaded = manifest_with(
            dir.path(),
            "publisher = \"IT Tiger\"",
            "[metadata]\nsource = \"msbuild\"\nproject = \"Sample.csproj\"\n",
        );
        let resolved = resolve_package(&loaded, &[]).unwrap();
        assert_eq!(resolved.version, "4.5.6");
        assert_eq!(resolved.description, "Resolved from the project.");
        assert_eq!(resolved.copyright, "(c) 2026 IT Tiger");
        assert_eq!(resolved.file_version, "4.5.6.0");
        assert_eq!(resolved.informational_version, "4.5.6+20260907.101500");
        assert_eq!(
            resolved.effective("version").unwrap().source,
            SOURCE_MSBUILD
        );

        // A property given on the command line overrides the project's.
        let overridden = resolve_package(&loaded, &[("Version".into(), "7.8.9".into())]).unwrap();
        assert_eq!(overridden.version, "7.8.9");
    }

    /// The whole path from a manifest to the version resource of the binary
    /// it names, with an inbox executable standing in for a stale build.
    #[test]
    fn a_manifest_validates_the_executable_it_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let inbox = PathBuf::from(root).join("System32").join("notepad.exe");
        std::fs::copy(&inbox, dir.path().join("app.exe")).unwrap();
        let installed = version_info::read(&dir.path().join("app.exe")).unwrap();
        let loaded = manifest_with(
            dir.path(),
            "publisher = \"IT Tiger\"\nversion = \"1.2.3\"",
            "[metadata]\nexecutable = \"app.exe\"\n",
        );
        let err = resolve_package(&loaded, &[]).unwrap_err();
        assert_eq!(err.code, "metadata_mismatch");
        assert!(err.message.contains("1.2.3"), "{}", err.message);
        assert!(
            err.message
                .contains(without_build_metadata(&installed.product_version)),
            "{}",
            err.message
        );
    }
}
