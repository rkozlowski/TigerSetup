//! Runtime metadata: the Protocol Buffers message tree compiled from
//! `proto/tigersetup.proto`, plus encoding, decoding and validation.

use prost::Message;

use crate::FormatError;
use crate::identity::Scope;

#[allow(clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/tigersetup.metadata.v1.rs"));
}

pub use generated::{
    Acquisition, AcquisitionSource, Dependency, DependencyInstall, Detector, DetectorKind,
    Directory, Engine, ExistingScopePolicy, File, Install, InstallOption, Legacy, Metadata,
    OptionKind, Package, PathEntry, Registration, RegistryKind, RegistryValue, Role,
    Scope as ScopeTag, Shortcut, ShortcutLocation,
};

/// The Windows build the engine itself requires (Windows 10 1809 / Server
/// 2019); a package may raise it, never lower it.
pub const ENGINE_MINIMUM_BUILD: u32 = 17763;

/// The metadata major version this crate understands.
pub const SCHEMA: u32 = 1;

impl Metadata {
    pub fn encode_to_vec(&self) -> Vec<u8> {
        Message::encode_to_vec(self)
    }

    /// Decodes and validates a metadata block.
    pub fn decode_block(bytes: &[u8]) -> Result<Metadata, FormatError> {
        let metadata = <Metadata as Message>::decode(bytes)?;
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn package(&self) -> &Package {
        self.package
            .as_ref()
            .expect("validated metadata has a package")
    }

    pub fn install(&self) -> &Install {
        self.install
            .as_ref()
            .expect("validated metadata has an install section")
    }

    pub fn engine(&self) -> &Engine {
        self.engine
            .as_ref()
            .expect("validated metadata has an engine section")
    }

    /// Whether this executable is the uninstaller copy rather than the
    /// installer.
    pub fn is_uninstaller(&self) -> bool {
        self.role == Role::Uninstaller as i32
    }

    /// The scope an uninstaller copy serves.
    pub fn served_scope(&self) -> Option<Scope> {
        Scope::from_tag(self.uninstaller_scope)
    }

    /// The declared default of an option, or `None` for an undeclared name.
    pub fn option_default(&self, name: &str) -> Option<bool> {
        self.options
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
            .map(|o| o.default)
    }

    /// The identity of the licence text the wizard shows: lower-case hex
    /// SHA-256 of the exact UTF-8 bytes of `license_text`, or `None` when
    /// there is no text to show. Any byte that differs — a changed
    /// copyright year included — is a different agreement for the purpose
    /// of recording that a person accepted it.
    pub fn license_sha256(&self) -> Option<String> {
        let text = &self.package().license_text;
        (!text.trim().is_empty()).then(|| crate::hex(&crate::sha256(text.as_bytes())))
    }

    /// The Add/Remove Programs key name: declared, or the package id.
    pub fn registration_key_name(&self) -> &str {
        match &self.registration {
            Some(r) if !r.key_name.is_empty() => &r.key_name,
            _ => &self.package().id,
        }
    }

    /// The lowest Windows build the package supports.
    pub fn minimum_build(&self) -> u32 {
        self.install().minimum_build.max(ENGINE_MINIMUM_BUILD)
    }

    /// What a run does when the product is installed in the other scope;
    /// the policy defaults to preserving the existing installation.
    pub fn existing_scope_policy(&self) -> ExistingScopePolicy {
        match ExistingScopePolicy::try_from(self.install().existing_scope) {
            Ok(ExistingScopePolicy::Unspecified) | Err(_) => ExistingScopePolicy::Preserve,
            Ok(policy) => policy,
        }
    }

    /// The scopes the package supports, in declaration order.
    pub fn scopes(&self) -> Vec<Scope> {
        self.install()
            .scopes
            .iter()
            .filter_map(|tag| Scope::from_tag(*tag))
            .collect()
    }

    /// The install-root template for a scope, or `None` when the package does
    /// not support that scope.
    pub fn install_root_template(&self, scope: Scope) -> Option<&str> {
        if !self.scopes().contains(&scope) {
            return None;
        }
        let install = self.install();
        let template = match scope {
            Scope::User => install.user_root.as_str(),
            Scope::Machine => install.machine_root.as_str(),
        };
        (!template.is_empty()).then_some(template)
    }

    /// Structural validation shared by the builder (before writing) and the
    /// engine (after reading).
    pub fn validate(&self) -> Result<(), FormatError> {
        let invalid = |message: String| FormatError::new("metadata_invalid", message);
        if self.schema != SCHEMA {
            return Err(FormatError::new(
                "metadata_unsupported",
                format!(
                    "metadata schema {} is not supported by schema {SCHEMA}",
                    self.schema
                ),
            ));
        }
        let package = self
            .package
            .as_ref()
            .ok_or_else(|| invalid("package section is missing".into()))?;
        crate::identity::validate_product_id(&package.id)?;
        crate::identity::validate_name(&package.name)?;
        crate::identity::validate_version(&package.version)?;
        let install = self
            .install
            .as_ref()
            .ok_or_else(|| invalid("install section is missing".into()))?;
        if install.scopes.is_empty() {
            return Err(invalid("install.scopes is empty".into()));
        }
        for tag in &install.scopes {
            let scope =
                Scope::from_tag(*tag).ok_or_else(|| invalid(format!("unknown scope tag {tag}")))?;
            let template = match scope {
                Scope::User => &install.user_root,
                Scope::Machine => &install.machine_root,
            };
            if template.is_empty() {
                return Err(invalid(format!(
                    "install root for scope {} is missing",
                    scope.as_str()
                )));
            }
        }
        if self.engine.is_none() {
            return Err(invalid("engine section is missing".into()));
        }
        let mut seen = std::collections::HashSet::new();
        for file in &self.files {
            validate_relative_path(&file.path)?;
            if file.entry.is_empty() {
                return Err(invalid(format!("file {} has no payload entry", file.path)));
            }
            if !seen.insert(file.path.to_ascii_lowercase()) {
                return Err(invalid(format!("file {} is declared twice", file.path)));
            }
        }
        let mut declared_directories = std::collections::HashSet::new();
        for directory in &self.directories {
            validate_relative_path(&directory.path)?;
            declared_directories.insert(directory.path.to_ascii_lowercase());
        }
        // The directory list is the whole truth about the directories a
        // package needs: it carries every parent of every file, and may carry
        // more (a directory the package wants even when empty). An engine
        // therefore installs what this list says instead of deriving the same
        // rule a second time from the file paths.
        for file in &self.files {
            let parts: Vec<&str> = file.path.split('/').collect();
            for depth in 1..parts.len() {
                let parent = parts[..depth].join("/");
                if !declared_directories.contains(&parent.to_ascii_lowercase()) {
                    return Err(invalid(format!(
                        "file {} needs directory {parent}, which is not declared",
                        file.path
                    )));
                }
            }
        }
        if Role::try_from(self.role).is_err() {
            return Err(invalid(format!("unknown role {}", self.role)));
        }
        if self.is_uninstaller() && self.served_scope().is_none() {
            return Err(invalid("an uninstaller names no scope".into()));
        }
        let mut option_names = std::collections::HashSet::new();
        for option in &self.options {
            validate_option_name(&option.name)?;
            if !option_names.insert(option.name.to_ascii_lowercase()) {
                return Err(invalid(format!("option {} is declared twice", option.name)));
            }
            match OptionKind::try_from(option.kind) {
                Ok(OptionKind::Custom) | Ok(OptionKind::Unspecified) => {
                    if option
                        .labels
                        .get("en-US")
                        .is_none_or(|l| l.trim().is_empty())
                    {
                        return Err(invalid(format!(
                            "option {} is custom and has no en-US label",
                            option.name
                        )));
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    return Err(invalid(format!(
                        "option {} has an unknown kind",
                        option.name
                    )));
                }
            }
        }
        let option_known = |name: &str| -> Result<(), FormatError> {
            if name.is_empty() || option_names.contains(&name.to_ascii_lowercase()) {
                Ok(())
            } else {
                Err(invalid(format!("option {name:?} is not declared")))
            }
        };
        for shortcut in &self.shortcuts {
            if ShortcutLocation::try_from(shortcut.location).is_err()
                || shortcut.location == ShortcutLocation::Unspecified as i32
            {
                return Err(invalid(format!(
                    "shortcut {:?} has no location",
                    shortcut.name
                )));
            }
            crate::identity::validate_name(&shortcut.name)?;
            validate_relative_path(&shortcut.target)?;
            if !shortcut.icon.is_empty() {
                validate_relative_path(&shortcut.icon)?;
            }
            if !shortcut.folder.is_empty() {
                validate_relative_path(&shortcut.folder)?;
            }
            option_known(&shortcut.option)?;
        }
        for entry in &self.path_entries {
            if !entry.path.is_empty() {
                validate_relative_path(&entry.path)?;
            }
            option_known(&entry.option)?;
        }
        for value in &self.registry_values {
            validate_registry_key(&value.key)?;
            if value.name.contains(|c: char| c.is_control()) {
                return Err(invalid(format!(
                    "registry value name {:?} is not valid",
                    value.name
                )));
            }
            if RegistryKind::try_from(value.kind).is_err()
                || value.kind == RegistryKind::Unspecified as i32
            {
                return Err(invalid(format!(
                    "registry value {}\\{} has no kind",
                    value.key, value.name
                )));
            }
            if value.kind == RegistryKind::Dword as i32 && value.data.parse::<u32>().is_err() {
                return Err(invalid(format!(
                    "registry value {}\\{} is a DWORD but {:?} is not a number",
                    value.key, value.name, value.data
                )));
            }
        }
        if let Some(registration) = &self.registration {
            if !registration.key_name.is_empty() {
                validate_registration_key_name(&registration.key_name)?;
            }
            if !registration.display_icon.is_empty() {
                validate_relative_path(&registration.display_icon)?;
            }
        }
        if let Some(legacy) = &self.legacy {
            if legacy.installer_type != "inno" {
                return Err(invalid(format!(
                    "legacy installer type {:?} is not supported",
                    legacy.installer_type
                )));
            }
            validate_registration_key_name(&legacy.registration_key)?;
        }
        let mut dependency_ids = std::collections::HashSet::new();
        for dependency in &self.dependencies {
            validate_dependency(dependency, &mut dependency_ids)?;
        }
        Ok(())
    }
}

fn validate_dependency(
    dependency: &Dependency,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), FormatError> {
    let invalid = |message: String| {
        FormatError::new(
            "metadata_invalid",
            format!("dependency {:?}: {message}", dependency.id),
        )
    };
    if dependency.id.trim().is_empty() {
        return Err(invalid("no id".into()));
    }
    if !seen.insert(dependency.id.to_ascii_lowercase()) {
        return Err(invalid("declared twice".into()));
    }
    if !dependency.minimum_version.is_empty() {
        validate_dotted_version(&dependency.minimum_version)?;
    }
    let detector = dependency
        .detect
        .as_ref()
        .ok_or_else(|| invalid("no detector".into()))?;
    match DetectorKind::try_from(detector.kind) {
        Ok(DetectorKind::DirectoryVersion) | Ok(DetectorKind::FileVersion) => {
            if detector.path.is_empty() {
                return Err(invalid("the detector needs a path".into()));
            }
        }
        Ok(DetectorKind::RegistryVersion) => {
            if detector.keys.is_empty() || detector.value.is_empty() {
                return Err(invalid(
                    "the registry detector needs keys and a value name".into(),
                ));
            }
        }
        Ok(DetectorKind::Registration) => {
            if detector.pattern.is_empty() {
                return Err(invalid("the registration detector needs a pattern".into()));
            }
        }
        _ => return Err(invalid("unknown detector kind".into())),
    }
    if let Some(acquisition) = &dependency.acquisition {
        match AcquisitionSource::try_from(acquisition.source) {
            Ok(AcquisitionSource::Winget) => {
                if acquisition.package_identifier.is_empty() {
                    return Err(invalid(
                        "WinGet acquisition needs a package identifier".into(),
                    ));
                }
            }
            Ok(AcquisitionSource::Url) => {
                if acquisition.url.is_empty() || !is_sha256_hex(&acquisition.sha256) {
                    return Err(invalid("URL acquisition needs a URL and a SHA-256".into()));
                }
            }
            _ => return Err(invalid("unknown acquisition source".into())),
        }
        if !acquisition.sha256.is_empty() && !is_sha256_hex(&acquisition.sha256) {
            return Err(invalid("the acquisition hash is not a SHA-256".into()));
        }
    }
    Ok(())
}

/// Option names are lower-case ASCII words joined by `-`.
pub fn validate_option_name(name: &str) -> Result<(), FormatError> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("option name {name:?} is not valid (lower-case words joined by '-')"),
        ))
    }
}

/// A registry key path relative to a root: backslash-separated non-empty
/// components without control characters.
pub fn validate_registry_key(key: &str) -> Result<(), FormatError> {
    let invalid =
        |why: &str| FormatError::new("metadata_invalid", format!("registry key {key:?} {why}"));
    if key.is_empty() {
        return Err(invalid("is empty"));
    }
    if key.starts_with('\\') || key.ends_with('\\') {
        return Err(invalid("must not start or end with a separator"));
    }
    if key.len() > 512 {
        return Err(invalid("is too long"));
    }
    for component in key.split('\\') {
        if component.trim().is_empty() {
            return Err(invalid("has an empty component"));
        }
        if component.contains(|c: char| c.is_control()) || component.len() > 255 {
            return Err(invalid("has a component Windows does not allow"));
        }
    }
    Ok(())
}

/// An Add/Remove Programs key name: one registry key component.
pub fn validate_registration_key_name(name: &str) -> Result<(), FormatError> {
    if name.contains('\\') {
        return Err(FormatError::new(
            "metadata_invalid",
            format!("registration key name {name:?} must be one key component"),
        ));
    }
    validate_registry_key(name)
}

/// A dependency version: one to four decimal components.
pub fn validate_dotted_version(version: &str) -> Result<(), FormatError> {
    let parts: Vec<&str> = version.split('.').collect();
    let ok = (1..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.chars().all(|c| c.is_ascii_digit()));
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "metadata_invalid",
            format!("version {version:?} is not a dotted numeric version"),
        ))
    }
}

/// Lower-case hex of 32 bytes.
pub fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// Install-relative paths use `/`, never start with a separator, never contain
/// `.` or `..` components, drive letters or characters Windows forbids.
pub fn validate_relative_path(path: &str) -> Result<(), FormatError> {
    let invalid = |why: &str| FormatError::new("metadata_invalid", format!("path {path:?} {why}"));
    if path.is_empty() {
        return Err(invalid("is empty"));
    }
    if path.contains('\\') {
        return Err(invalid("must use forward slashes"));
    }
    if path.starts_with('/') || path.ends_with('/') {
        return Err(invalid("must not start or end with a separator"));
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid("has an empty, current or parent component"));
        }
        if component.contains(|c: char| {
            matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
        }) {
            return Err(invalid(
                "contains a character Windows does not allow in file names",
            ));
        }
        if component.ends_with(' ') || component.ends_with('.') {
            return Err(invalid("has a component ending in a space or a dot"));
        }
        let stem = component.split('.').next().unwrap_or(component);
        if is_reserved_device_name(stem) {
            return Err(invalid("uses a reserved Windows device name"));
        }
    }
    Ok(())
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, case-insensitive:
/// Windows resolves these to devices whatever the extension.
fn is_reserved_device_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
}

impl Scope {
    pub fn from_tag(tag: i32) -> Option<Scope> {
        match ScopeTag::try_from(tag).ok()? {
            ScopeTag::User => Some(Scope::User),
            ScopeTag::Machine => Some(Scope::Machine),
            ScopeTag::Unspecified => None,
        }
    }

    pub fn tag(self) -> i32 {
        match self {
            Scope::User => ScopeTag::User as i32,
            Scope::Machine => ScopeTag::Machine as i32,
        }
    }
}

impl ExistingScopePolicy {
    /// The manifest spelling of a policy, which machine-readable output
    /// carries too.
    pub fn as_str(self) -> &'static str {
        match self {
            ExistingScopePolicy::AllowParallel => "allow-parallel",
            ExistingScopePolicy::Error => "error",
            ExistingScopePolicy::Preserve | ExistingScopePolicy::Unspecified => "preserve",
        }
    }

    /// The policy a manifest names.
    pub fn parse(text: &str) -> Option<ExistingScopePolicy> {
        match text {
            "preserve" => Some(ExistingScopePolicy::Preserve),
            "allow-parallel" => Some(ExistingScopePolicy::AllowParallel),
            "error" => Some(ExistingScopePolicy::Error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> Metadata {
        Metadata {
            schema: SCHEMA,
            package: Some(Package {
                id: "IT-Tiger.Sample".into(),
                name: "Sample".into(),
                version: "1.0.0".into(),
                publisher: "IT Tiger".into(),
                ..Default::default()
            }),
            install: Some(Install {
                scopes: vec![Scope::User.tag()],
                user_root: "%LOCALAPPDATA%\\Programs\\Sample".into(),
                ..Default::default()
            }),
            files: vec![File {
                path: "bin/app.exe".into(),
                size: 3,
                entry: "bin/app.exe".into(),
            }],
            directories: vec![Directory { path: "bin".into() }],
            engine: Some(Engine {
                tigersetup_version: "0.1.0".into(),
                engine_sha256: "00".into(),
                engine_block_sha256: "00".into(),
            }),
            ..Default::default()
        }
    }

    /// The engine installs the declared directories rather than deriving them
    /// again from the file paths, so metadata that does not carry a file's
    /// parents is invalid rather than merely incomplete.
    #[test]
    fn a_file_whose_parent_directory_is_not_declared_is_refused() {
        let mut metadata = sample();
        metadata.files.push(File {
            path: "bin/x64/native.dll".into(),
            size: 3,
            entry: "bin/x64/native.dll".into(),
        });
        let error = metadata.validate().unwrap_err().to_string();
        assert!(error.contains("bin/x64"), "{error}");

        metadata.directories.push(Directory {
            path: "bin/x64".into(),
        });
        metadata.validate().unwrap();
    }

    #[test]
    fn resources_and_dependencies_are_validated() {
        let mut metadata = sample();
        metadata.options.push(InstallOption {
            name: "path".into(),
            default: true,
            kind: OptionKind::Path as i32,
            ..Default::default()
        });
        metadata.path_entries.push(PathEntry {
            path: String::new(),
            option: "path".into(),
        });
        metadata.shortcuts.push(Shortcut {
            location: ShortcutLocation::StartMenu as i32,
            name: "Sample".into(),
            target: "bin/app.exe".into(),
            ..Default::default()
        });
        metadata.registry_values.push(RegistryValue {
            key: "IT Tiger\\Sample".into(),
            name: "InstallRoot".into(),
            kind: RegistryKind::ExpandString as i32,
            data: "%INSTALLROOT%".into(),
        });
        metadata.dependencies.push(Dependency {
            id: "Vendor.Runtime".into(),
            minimum_version: "10.0".into(),
            detect: Some(Detector {
                kind: DetectorKind::DirectoryVersion as i32,
                path: "%PROGRAMFILES%\\runtime".into(),
                pattern: "10.*".into(),
                ..Default::default()
            }),
            acquisition: Some(Acquisition {
                source: AcquisitionSource::Winget as i32,
                package_identifier: "Vendor.Runtime".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
        metadata.validate().unwrap();
        assert_eq!(metadata.option_default("PATH"), Some(true));
        assert_eq!(metadata.registration_key_name(), "IT-Tiger.Sample");
        assert_eq!(metadata.minimum_build(), ENGINE_MINIMUM_BUILD);

        let mut undeclared = metadata.clone();
        undeclared.path_entries[0].option = "nope".into();
        assert!(undeclared.validate().is_err());
        let mut no_detector = metadata.clone();
        no_detector.dependencies[0].detect = None;
        assert!(no_detector.validate().is_err());
        let mut bad_hash = metadata.clone();
        bad_hash.dependencies[0].acquisition = Some(Acquisition {
            source: AcquisitionSource::Url as i32,
            url: "https://example.invalid/x.exe".into(),
            sha256: "zz".into(),
            ..Default::default()
        });
        assert!(bad_hash.validate().is_err());
        let mut uninstaller = metadata.clone();
        uninstaller.role = Role::Uninstaller as i32;
        assert!(uninstaller.validate().is_err());
        uninstaller.uninstaller_scope = Scope::User.tag();
        uninstaller.validate().unwrap();
        assert!(uninstaller.is_uninstaller());
    }

    #[test]
    fn metadata_round_trips() {
        let metadata = sample();
        let bytes = metadata.encode_to_vec();
        assert_eq!(Metadata::decode_block(&bytes).unwrap(), metadata);
    }

    #[test]
    fn unknown_schema_is_refused() {
        let mut metadata = sample();
        metadata.schema = 2;
        assert_eq!(
            Metadata::decode_block(&metadata.encode_to_vec())
                .unwrap_err()
                .code,
            "metadata_unsupported"
        );
    }

    #[test]
    fn bad_paths_are_refused() {
        for bad in [
            "",
            "/abs",
            "a\\b",
            "a/../b",
            "con:",
            "trailing.",
            "x/",
            "nul.txt",
            "COM1",
            "dir/aux",
            "Lpt9.log",
        ] {
            assert!(
                validate_relative_path(bad).is_err(),
                "{bad:?} should be invalid"
            );
        }
        for good in [
            "a",
            "a/b",
            "a b/c.d",
            "x.y/z",
            "com10",
            "console.log",
            "nulls",
        ] {
            assert!(
                validate_relative_path(good).is_ok(),
                "{good:?} should be valid"
            );
        }
    }

    /// The recorded acceptance names the text a person read, byte for byte:
    /// an annual copyright edit is a new text, and no text is no agreement.
    #[test]
    fn the_licence_identity_is_the_hash_of_the_exact_text() {
        let mut metadata = sample();
        assert_eq!(metadata.license_sha256(), None);
        metadata.package.as_mut().unwrap().license_text = " \r\n\t".into();
        assert_eq!(metadata.license_sha256(), None, "blank text is no licence");
        let text = "MIT License\r\n\r\nCopyright (c) 2026 IT Tiger\r\n";
        metadata.package.as_mut().unwrap().license_text = text.into();
        let expected = crate::hex(&crate::sha256(text.as_bytes()));
        assert_eq!(
            metadata.license_sha256().as_deref(),
            Some(expected.as_str())
        );
        metadata.package.as_mut().unwrap().license_text = text.replace("2026", "2027");
        assert_ne!(
            metadata.license_sha256().as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn scope_lookup_respects_declared_scopes() {
        let metadata = sample();
        assert_eq!(
            metadata.install_root_template(Scope::User),
            Some("%LOCALAPPDATA%\\Programs\\Sample")
        );
        assert_eq!(metadata.install_root_template(Scope::Machine), None);
    }
}
