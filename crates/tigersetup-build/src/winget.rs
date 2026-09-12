//! WinGet manifest generation (`TigerSetup-Design.md` §8.2). The same
//! identity that becomes the Add/Remove Programs registration becomes the
//! WinGet package identity, so the manifests are generated from the built
//! installer's own metadata rather than typed a second time.
//!
//! Publication is two stages because the final manifest needs the immutable
//! public URL and the hash of the bytes behind it:
//!
//! ```text
//! tiger-setup winget prepare  → manifests with an unresolved installer URL
//! tiger-setup winget finalize → the published URL and the hash of the
//!                               exact bytes that were built
//! ```
//!
//! Neither stage builds anything. `finalize` hashes the file it is given and
//! refuses a file that is not the version the manifests name: build once,
//! validate exact bytes, publish those exact bytes.

use std::fs::File;
use std::path::{Path, PathBuf};

use tigersetup_format::metadata::{AcquisitionSource, Metadata};
use tigersetup_format::{Installer, hex, sha256_reader};

use crate::manifest::LoadedManifest;
use crate::{BuildError, Result};

/// The community manifest schema the generated set declares.
pub const SCHEMA_VERSION: &str = "1.12.0";

/// The locale the generated manifest set is written for.
pub const DEFAULT_LOCALE: &str = "en-US";

/// The installer URL `prepare` writes: a syntactically valid URL that no one
/// can publish by accident, replaced by `finalize`.
pub const UNRESOLVED_URL_PREFIX: &str = "https://UNRESOLVED/";

/// The placeholders WinGet substitutes in installer switches.
const LOG_PLACEHOLDER: &str = "<LOGPATH>";
const INSTALL_LOCATION_PLACEHOLDER: &str = "<INSTALLPATH>";

/// The generated installer's exit codes as WinGet return-response types.
/// Success is never listed; `custom` carries the codes WinGet has no type
/// for (`failed and rolled back`, `elevation required`, `recovery
/// incomplete`).
pub const EXPECTED_RETURN_CODES: [(i32, &str); 9] = [
    (1, "custom"),
    (2, "invalidParameter"),
    (3, "missingDependency"),
    (4, "custom"),
    (5, "cancelledByUser"),
    (6, "packageInUseByApplication"),
    (7, "custom"),
    (8, "systemNotSupported"),
    (3010, "rebootRequiredToFinish"),
];

/// What `winget prepare` wrote.
#[derive(Debug, Clone)]
pub struct PrepareResult {
    pub identifier: String,
    pub version: String,
    /// Lower-case hex SHA-256 of the installer that was read.
    pub installer_sha256: String,
    /// The version, installer and locale manifests, in that order.
    pub files: Vec<PathBuf>,
}

/// What `winget finalize` changed.
#[derive(Debug, Clone)]
pub struct FinalizeResult {
    pub manifest_path: PathBuf,
    pub version: String,
    pub url: String,
    /// Upper-case hex SHA-256, as WinGet writes it.
    pub installer_sha256: String,
    /// Installer entries whose URL and hash the run wrote.
    pub entries: usize,
}

/// Upper-case hex SHA-256 of a file's exact bytes.
pub fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|err| {
        BuildError::new(
            "installer_unreadable",
            format!("cannot read {}: {err}", path.display()),
        )
    })?;
    Ok(hex(&sha256_reader(&mut file)?).to_ascii_uppercase())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Generates the manifest set for a manifest and the installer built from
/// it, into `output`.
pub fn prepare(
    manifest_path: &Path,
    installer_path: &Path,
    output: &Path,
) -> Result<PrepareResult> {
    let loaded = LoadedManifest::load(manifest_path)?;
    let installer = Installer::open(installer_path)?;
    let metadata = installer.metadata();
    agree(&loaded, metadata)?;

    let identifier = loaded.manifest.winget_identifier().to_string();
    let version = metadata.package().version.clone();
    let sha256 = file_sha256(installer_path)?;
    let url = format!("{UNRESOLVED_URL_PREFIX}{}", file_name(installer_path));

    std::fs::create_dir_all(output)?;
    let files = vec![
        write_document(
            output,
            &format!("{identifier}.yaml"),
            version_manifest(&identifier, &version),
        )?,
        write_document(
            output,
            &format!("{identifier}.installer.yaml"),
            installer_manifest(&loaded, metadata, &identifier, &url, &sha256),
        )?,
        write_document(
            output,
            &format!("{identifier}.locale.{DEFAULT_LOCALE}.yaml"),
            locale_manifest(&loaded, metadata, &identifier)?,
        )?,
    ];
    Ok(PrepareResult {
        identifier,
        version,
        installer_sha256: sha256.to_ascii_lowercase(),
        files,
    })
}

/// Writes the published URL and the hash of the exact installer bytes into
/// an already generated manifest set. Idempotent, and never rebuilds.
pub fn finalize(directory: &Path, url: &str, installer_path: &Path) -> Result<FinalizeResult> {
    let manifest_path = installer_manifest_in(directory)?;
    let text = std::fs::read_to_string(&manifest_path).map_err(|err| {
        BuildError::new(
            "winget_manifest_unreadable",
            format!("cannot read {}: {err}", manifest_path.display()),
        )
    })?;
    let installer = Installer::open(installer_path)?;
    let version = installer.metadata().package().version.clone();
    let declared = field_value(&text, "PackageVersion").ok_or_else(|| {
        BuildError::new(
            "winget_manifest_invalid",
            format!("{} declares no PackageVersion", manifest_path.display()),
        )
    })?;
    if declared != version {
        return Err(BuildError::new(
            "winget_manifest_mismatch",
            format!(
                "{} is version {version} but the manifest set is for {declared}",
                installer_path.display()
            ),
        ));
    }
    let published = url.rsplit('/').next().unwrap_or_default();
    let built = file_name(installer_path);
    if published != built {
        return Err(BuildError::new(
            "winget_manifest_mismatch",
            format!(
                "the URL publishes {published:?} but the installer given is {built:?}; \
                 the published bytes must be the bytes that were built"
            ),
        ));
    }

    let sha256 = file_sha256(installer_path)?;
    let mut entries = 0;
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(indent) = key_indent(line, "InstallerUrl") {
            entries += 1;
            lines.push(format!("{indent}InstallerUrl: {}", scalar(url)));
        } else if let Some(indent) = key_indent(line, "InstallerSha256") {
            lines.push(format!("{indent}InstallerSha256: {sha256}"));
        } else {
            lines.push(line.to_string());
        }
    }
    if entries == 0 {
        return Err(BuildError::new(
            "winget_manifest_invalid",
            format!("{} declares no installer entry", manifest_path.display()),
        ));
    }
    let mut out = lines.join("\n");
    out.push('\n');
    std::fs::write(&manifest_path, out)?;
    Ok(FinalizeResult {
        manifest_path,
        version,
        url: url.to_string(),
        installer_sha256: sha256,
        entries,
    })
}

/// The one `*.installer.yaml` of a generated manifest set.
fn installer_manifest_in(directory: &Path) -> Result<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|err| {
            BuildError::new(
                "winget_manifest_unreadable",
                format!("cannot read {}: {err}", directory.display()),
            )
        })?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| file_name(path).ends_with(".installer.yaml"))
        .collect();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(BuildError::new(
            "winget_manifest_missing",
            format!(
                "{} holds no <identifier>.installer.yaml",
                directory.display()
            ),
        )),
        n => Err(BuildError::new(
            "winget_manifest_invalid",
            format!("{} holds {n} installer manifests", directory.display()),
        )),
    }
}

/// The installer's own metadata against the manifest that declared it.
fn agree(loaded: &LoadedManifest, metadata: &Metadata) -> Result<()> {
    let package = &loaded.manifest.package;
    let built = metadata.package();
    let mismatch = |field: &str, declared: &str, actual: &str| {
        Err(BuildError::new(
            "winget_manifest_mismatch",
            format!(
                "the installer's {field} is {actual:?} but {} declares {declared:?}",
                loaded.path.display()
            ),
        ))
    };
    if package.id != built.id {
        return mismatch("package id", &package.id, &built.id);
    }
    if package.name != built.name {
        return mismatch("package name", &package.name, &built.name);
    }
    if package.publisher != built.publisher {
        return mismatch("publisher", &package.publisher, &built.publisher);
    }
    if let Some(version) = &package.version
        && version != &built.version
    {
        return mismatch("version", version, &built.version);
    }
    Ok(())
}

fn write_document(directory: &Path, name: &str, document: String) -> Result<PathBuf> {
    let path = directory.join(name);
    std::fs::write(&path, document)?;
    Ok(path)
}

fn header(kind: &str) -> String {
    format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.{SCHEMA_VERSION}.schema.json"
    )
}

fn version_manifest(identifier: &str, version: &str) -> String {
    let mut doc = Document::new(header("version"));
    doc.field(0, "PackageIdentifier", identifier);
    doc.field(0, "PackageVersion", version);
    doc.field(0, "DefaultLocale", DEFAULT_LOCALE);
    doc.field(0, "ManifestType", "version");
    doc.field(0, "ManifestVersion", SCHEMA_VERSION);
    doc.finish()
}

fn installer_manifest(
    loaded: &LoadedManifest,
    metadata: &Metadata,
    identifier: &str,
    url: &str,
    sha256: &str,
) -> String {
    let manifest = &loaded.manifest;
    let winget = &manifest.winget;
    let mut doc = Document::new(header("installer"));
    doc.field(0, "PackageIdentifier", identifier);
    doc.field(0, "PackageVersion", &metadata.package().version);
    doc.field(0, "InstallerLocale", DEFAULT_LOCALE);
    doc.sequence(0, "Platform", &["Windows.Desktop".to_string()]);
    doc.field(
        0,
        "MinimumOSVersion",
        &winget
            .minimum_os_version
            .clone()
            .unwrap_or_else(|| format!("10.0.{}.0", metadata.minimum_build())),
    );
    doc.field(0, "InstallerType", "exe");
    doc.sequence(
        0,
        "InstallModes",
        &[
            "interactive".to_string(),
            "silent".to_string(),
            "silentWithProgress".to_string(),
        ],
    );
    doc.field(0, "UpgradeBehavior", "install");
    doc.sequence(0, "Commands", &winget.commands);

    doc.line(0, "ExpectedReturnCodes:");
    for (code, response) in EXPECTED_RETURN_CODES {
        doc.line(0, &format!("- InstallerReturnCode: {code}"));
        doc.field(1, "ReturnResponse", response);
    }

    // Every dependency the engine acquires from the WinGet catalog is a
    // package dependency WinGet can satisfy first; a URL-acquired one has no
    // catalog identity and stays the installer's own business.
    let catalog: Vec<String> = metadata
        .dependencies
        .iter()
        .filter_map(|dependency| dependency.acquisition.as_ref())
        .filter(|acquisition| acquisition.source == AcquisitionSource::Winget as i32)
        .map(|acquisition| acquisition.package_identifier.clone())
        .filter(|identifier| !identifier.is_empty())
        .collect();
    if !catalog.is_empty() {
        doc.line(0, "Dependencies:");
        doc.line(1, "PackageDependencies:");
        for identifier in &catalog {
            doc.line(1, &format!("- PackageIdentifier: {}", scalar(identifier)));
        }
    }

    let product_code = metadata.registration_key_name();
    let registration = metadata.registration.clone().unwrap_or_default();
    let display_name = if registration.display_name.is_empty() {
        metadata.package().name.clone()
    } else {
        registration.display_name.clone()
    };
    doc.line(0, "Installers:");
    for scope in metadata.scopes() {
        doc.line(0, "- Architecture: x64");
        doc.field(1, "Scope", scope.as_str());
        doc.field(1, "InstallerUrl", url);
        doc.field(1, "InstallerSha256", sha256);
        doc.line(1, "InstallerSwitches:");
        let silent = format!("install --quiet --scope {}", scope.as_str());
        doc.field(2, "Silent", &silent);
        doc.field(2, "SilentWithProgress", &silent);
        doc.field(2, "Log", &format!("--log \"{LOG_PLACEHOLDER}\""));
        // A package that pins the scope's install root has chosen where it
        // goes; only a scope whose root is the default accepts an override.
        if manifest.install_root_is_default(scope) {
            doc.field(
                2,
                "InstallLocation",
                &format!("--install-root \"{INSTALL_LOCATION_PLACEHOLDER}\""),
            );
        }
        doc.field(1, "ProductCode", product_code);
        doc.line(1, "AppsAndFeaturesEntries:");
        doc.line(1, &format!("- DisplayName: {}", scalar(&display_name)));
        doc.field(2, "Publisher", &metadata.package().publisher);
        // WinGet compares the registration's DisplayVersion with the package
        // version, so it is only worth stating when the two differ.
        if !registration.display_version.is_empty()
            && registration.display_version != metadata.package().version
        {
            doc.field(2, "DisplayVersion", &registration.display_version);
        }
        doc.field(2, "ProductCode", product_code);
        doc.field(2, "InstallerType", "exe");
    }
    doc.field(0, "ManifestType", "installer");
    doc.field(0, "ManifestVersion", SCHEMA_VERSION);
    doc.finish()
}

fn locale_manifest(
    loaded: &LoadedManifest,
    metadata: &Metadata,
    identifier: &str,
) -> Result<String> {
    let manifest = &loaded.manifest;
    let winget = &manifest.winget;
    let package = metadata.package();
    let short_description = winget
        .short_description
        .clone()
        .unwrap_or_else(|| package.description.clone());
    for (field, value) in [
        ("Publisher", &package.publisher),
        ("PackageName", &package.name),
        ("License", &package.license),
        ("ShortDescription", &short_description),
    ] {
        if value.trim().is_empty() {
            return Err(BuildError::new(
                "winget_metadata_incomplete",
                format!(
                    "the locale manifest needs {field}; declare it in the package or in [winget]"
                ),
            ));
        }
    }

    let mut doc = Document::new(header("defaultLocale"));
    doc.field(0, "PackageIdentifier", identifier);
    doc.field(0, "PackageVersion", &package.version);
    doc.field(0, "PackageLocale", DEFAULT_LOCALE);
    doc.field(0, "Publisher", &package.publisher);
    doc.field(
        0,
        "PublisherUrl",
        winget.publisher_url.as_deref().unwrap_or(""),
    );
    doc.field(
        0,
        "PublisherSupportUrl",
        winget
            .publisher_support_url
            .as_deref()
            .unwrap_or(&package.support_url),
    );
    doc.field(0, "Author", &package.publisher);
    doc.field(0, "PackageName", &package.name);
    doc.field(
        0,
        "PackageUrl",
        winget
            .package_url
            .as_deref()
            .unwrap_or(&package.website_url),
    );
    doc.field(0, "License", &package.license);
    doc.field(0, "LicenseUrl", winget.license_url.as_deref().unwrap_or(""));
    doc.field(0, "Copyright", &package.copyright);
    doc.field(0, "ShortDescription", &short_description);
    if let Some(description) = winget.description.as_deref().filter(|d| !d.is_empty()) {
        doc.block(0, "Description", description);
    }
    doc.field(0, "Moniker", winget.moniker.as_deref().unwrap_or(""));
    doc.sequence(0, "Tags", &winget.tags);
    let documentation = winget
        .documentation_url
        .as_deref()
        .unwrap_or(&package.help_url);
    if !documentation.is_empty() {
        doc.line(0, "Documentations:");
        doc.line(0, "- DocumentLabel: Help");
        doc.field(1, "DocumentUrl", documentation);
    }
    doc.field(
        0,
        "ReleaseNotesUrl",
        winget.release_notes_url.as_deref().unwrap_or(""),
    );
    doc.field(0, "ManifestType", "defaultLocale");
    doc.field(0, "ManifestVersion", SCHEMA_VERSION);
    Ok(doc.finish())
}

/// The value of a top-level `Key: value` line.
fn field_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
        .map(|value| unquote(value.trim()))
}

/// The indentation of a `Key:` line, or `None` when the line is something
/// else. A leading `- ` counts as indentation so a rewritten line keeps the
/// sequence marker it had.
fn key_indent<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let end = line.find(key)?;
    let prefix = &line[..end];
    if !prefix.chars().all(|c| c == ' ' || c == '-') {
        return None;
    }
    let rest = &line[end + key.len()..];
    rest.starts_with(':').then_some(prefix)
}

fn unquote(value: &str) -> String {
    match value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        Some(inner) => inner.replace("''", "'"),
        None => value.to_string(),
    }
}

/// A YAML scalar: plain where that is unambiguous, single-quoted otherwise.
/// The manifests are flat mappings of known strings, so this is all the
/// quoting the shape needs.
fn scalar(value: &str) -> String {
    let mut characters = value.chars();
    // `-`, `?` and `:` open a construct only when a space follows, which is
    // why an installer switch such as `--quiet` needs no quoting.
    let ambiguous_start = match (characters.next(), characters.next()) {
        (Some('-' | '?' | ':'), following) => following.is_none() || following == Some(' '),
        (Some(first), _) => matches!(
            first,
            ',' | '['
                | ']'
                | '{'
                | '}'
                | '#'
                | '&'
                | '*'
                | '!'
                | '|'
                | '>'
                | '\''
                | '"'
                | '%'
                | '@'
                | '`'
        ),
        (None, _) => true,
    };
    let plain = !value.is_empty()
        && !value.starts_with(' ')
        && !value.ends_with(' ')
        && !ambiguous_start
        && !value.contains(": ")
        && !value.contains(" #")
        && !value.contains('\n')
        && !value.ends_with(':')
        && value.parse::<f64>().is_err();
    if plain {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

/// A YAML document built line by line. Indentation is two spaces per level,
/// as the community manifests are written.
struct Document {
    lines: Vec<String>,
}

impl Document {
    fn new(header: String) -> Document {
        Document {
            lines: vec![header],
        }
    }

    fn line(&mut self, indent: usize, text: &str) {
        self.lines.push(format!("{}{text}", "  ".repeat(indent)));
    }

    /// A `Key: value` entry; an empty value is not written at all, because a
    /// WinGet manifest states what is known and omits the rest.
    fn field(&mut self, indent: usize, key: &str, value: &str) {
        if value.trim().is_empty() {
            return;
        }
        self.line(indent, &format!("{key}: {}", scalar(value)));
    }

    fn sequence(&mut self, indent: usize, key: &str, items: &[String]) {
        if items.is_empty() {
            return;
        }
        self.line(indent, &format!("{key}:"));
        for item in items {
            self.line(indent, &format!("- {}", scalar(item)));
        }
    }

    /// A literal block scalar, which is how a multi-line description keeps
    /// its line breaks.
    fn block(&mut self, indent: usize, key: &str, value: &str) {
        self.line(indent, &format!("{key}: |-"));
        for line in value.trim_end().lines() {
            self.line(indent + 1, line);
        }
    }

    fn finish(self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_are_quoted_only_where_yaml_needs_it() {
        assert_eq!(scalar("TigerMarkView"), "TigerMarkView");
        assert_eq!(scalar("0.8.1"), "0.8.1");
        assert_eq!(
            scalar("install --quiet --scope machine"),
            "install --quiet --scope machine"
        );
        assert_eq!(scalar("--log \"<LOGPATH>\""), "--log \"<LOGPATH>\"");
        assert_eq!(
            scalar("{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1"),
            "'{E718860E-EDE4-4ACC-8235-BCF1DD40FC25}_is1'"
        );
        assert_eq!(scalar("1.5"), "'1.5'");
        assert_eq!(scalar("a: b"), "'a: b'");
        // An apostrophe inside a plain scalar is ordinary text; one that
        // opens the value is not.
        assert_eq!(scalar("it's"), "it's");
        assert_eq!(scalar("'quoted'"), "'''quoted'''");
        assert_eq!(unquote(&scalar("'quoted'")), "'quoted'");
        assert_eq!(unquote(&scalar("0.8.1")), "0.8.1");
    }

    #[test]
    fn an_installer_line_is_recognised_wherever_it_is_indented() {
        assert_eq!(key_indent("  InstallerUrl: x", "InstallerUrl"), Some("  "));
        assert_eq!(key_indent("- InstallerUrl: x", "InstallerUrl"), Some("- "));
        assert_eq!(key_indent("  InstallerUrlOther: x", "InstallerUrl"), None);
        assert_eq!(
            key_indent("  Silent: InstallerUrl: x", "InstallerUrl"),
            None
        );
    }

    #[test]
    fn a_block_scalar_keeps_the_line_breaks_of_a_long_description() {
        let mut doc = Document::new("# header".into());
        doc.block(0, "Description", "first line\nsecond line\n");
        assert_eq!(
            doc.finish(),
            "# header\nDescription: |-\n  first line\n  second line\n"
        );
    }
}
