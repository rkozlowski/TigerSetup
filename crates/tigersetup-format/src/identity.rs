//! Package-identity derivations the builder and the engine must agree on:
//! validation of the identity fields, per-scope roots and the installer's
//! file name. One function each, used by both sides.

use crate::FormatError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    User,
    Machine,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Machine => "machine",
        }
    }

    pub fn parse(text: &str) -> Option<Scope> {
        match text {
            "user" => Some(Scope::User),
            "machine" => Some(Scope::Machine),
            _ => None,
        }
    }
}

/// Where the per-installation state lives, as a template with the known
/// folder the engine expands on the target.
pub fn state_directory_template(scope: Scope, product_id: &str) -> String {
    match scope {
        Scope::User => format!("%LOCALAPPDATA%\\TigerSetup\\{product_id}"),
        Scope::Machine => format!("%PROGRAMDATA%\\TigerSetup\\{product_id}"),
    }
}

/// The default install root for a scope, as a template.
pub fn default_install_root_template(scope: Scope, name: &str) -> String {
    match scope {
        Scope::User => format!("%LOCALAPPDATA%\\Programs\\{name}"),
        Scope::Machine => format!("%PROGRAMFILES%\\{name}"),
    }
}

/// `<name>-<version>-Setup.exe`.
pub fn installer_file_name(name: &str, version: &str) -> String {
    format!("{name}-{version}-Setup.exe")
}

/// Expands `%NAME%` placeholders through `lookup`, which the caller binds to
/// the target machine's environment. An unknown placeholder is an error.
pub fn expand_template(
    template: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, FormatError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            return Err(FormatError::new(
                "template_invalid",
                format!("unterminated placeholder in {template:?}"),
            ));
        };
        let name = &after[..end];
        let value = lookup(name).ok_or_else(|| {
            FormatError::new(
                "known_folder_unavailable",
                format!("no value for %{name}% in {template:?}"),
            )
        })?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A product id is `Publisher.Product`-style: ASCII letters, digits, `.`, `-`
/// and `_`, starting with a letter or digit. It names a directory.
pub fn validate_product_id(id: &str) -> Result<(), FormatError> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !id.ends_with('.');
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "package_id_invalid",
            format!("package id {id:?} is not valid"),
        ))
    }
}

/// A package name names the install folder and the installer file.
pub fn validate_name(name: &str) -> Result<(), FormatError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(' ')
        && !name.ends_with(' ')
        && !name.ends_with('.')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_'));
    if ok {
        Ok(())
    } else {
        Err(FormatError::new(
            "package_name_invalid",
            format!("package name {name:?} is not valid"),
        ))
    }
}

/// Versions are `major.minor.patch`, each a decimal number without leading
/// zeros (except `0` itself).
pub fn validate_version(version: &str) -> Result<(), FormatError> {
    let parts: Vec<&str> = version.split('.').collect();
    let part_ok = |p: &str| {
        !p.is_empty()
            && p.chars().all(|c| c.is_ascii_digit())
            && (p == "0" || !p.starts_with('0'))
            && p.len() <= 9
    };
    if parts.len() == 3 && parts.iter().all(|p| part_ok(p)) {
        Ok(())
    } else {
        Err(FormatError::new(
            "package_version_invalid",
            format!("version {version:?} is not major.minor.patch"),
        ))
    }
}

/// Numeric comparison of two validated versions.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a).cmp(&parse(b))
}

/// The first numeric component of a dotted version.
pub fn major_of(version: &str) -> u64 {
    version
        .split('.')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(0)
}

/// Whether `found` satisfies a dependency's version requirement: an empty
/// minimum accepts any version; otherwise the found version has the
/// minimum's major component and is not lower (`TigerSetup-Design.md`
/// §7.9). One rule for the detector, the builder and the catalog.
pub fn version_satisfies(found: &str, minimum: &str) -> bool {
    if minimum.is_empty() {
        return true;
    }
    major_of(found) == major_of(minimum)
        && compare_versions(found, minimum) != std::cmp::Ordering::Less
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_expand_through_the_lookup() {
        let lookup = |name: &str| {
            (name == "LOCALAPPDATA").then(|| "C:\\Users\\x\\AppData\\Local".to_string())
        };
        assert_eq!(
            expand_template(
                &state_directory_template(Scope::User, "IT-Tiger.TestApp"),
                lookup
            )
            .unwrap(),
            "C:\\Users\\x\\AppData\\Local\\TigerSetup\\IT-Tiger.TestApp"
        );
        assert_eq!(
            expand_template(
                &default_install_root_template(Scope::User, "TestApp"),
                lookup
            )
            .unwrap(),
            "C:\\Users\\x\\AppData\\Local\\Programs\\TestApp"
        );
        assert_eq!(
            expand_template("%NOPE%\\x", lookup).unwrap_err().code,
            "known_folder_unavailable"
        );
        assert_eq!(
            expand_template("%NOPE", lookup).unwrap_err().code,
            "template_invalid"
        );
        assert_eq!(expand_template("plain", lookup).unwrap(), "plain");
    }

    #[test]
    fn identity_fields_are_validated() {
        assert!(validate_product_id("IT-Tiger.TigerSetupTestApp").is_ok());
        assert!(validate_product_id(".x").is_err());
        assert!(validate_product_id("a b").is_err());
        assert!(validate_name("TigerSetupTestApp").is_ok());
        assert!(validate_name("Tiger Setup").is_ok());
        assert!(validate_name("bad/name").is_err());
        assert!(validate_version("1.0.0").is_ok());
        assert!(validate_version("1.0").is_err());
        assert!(validate_version("01.0.0").is_err());
        assert_eq!(
            compare_versions("1.10.0", "1.9.0"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            installer_file_name("TigerSetupTestApp", "1.0.0"),
            "TigerSetupTestApp-1.0.0-Setup.exe"
        );
    }

    #[test]
    fn version_requirement_is_same_major_not_lower() {
        assert!(version_satisfies("10.0.11", "10.0"));
        assert!(version_satisfies("10.0.0", "10.0.0"));
        assert!(version_satisfies("152.0.4191.53", ""));
        assert!(!version_satisfies("9.0.5", "10.0"));
        assert!(!version_satisfies("11.0.0", "10.0"));
        assert!(!version_satisfies("10.0.9", "10.0.10"));
        assert_eq!(major_of("152.0.4191.53"), 152);
    }
}
