//! Add/Remove Programs registration: the values under the scope's
//! `...\CurrentVersion\Uninstall\<key>`, written last so that a registration
//! means "installed" to Windows, all journaled as registry operations.

use std::path::Path;

use tigersetup_format::Metadata;

use crate::Result;
use crate::plan;
use crate::resource::registry::DesiredValue;
use crate::scope::Locations;
use crate::win::registry::{Data, KeyPath};

/// The registration key and its values for this installation.
pub fn values(
    metadata: &Metadata,
    locations: &Locations,
    install_root: &Path,
    uninstaller: &Path,
    install_date: &str,
) -> Result<(KeyPath, Vec<DesiredValue>)> {
    let package = metadata.package();
    let registration = metadata.registration.clone().unwrap_or_default();
    let key = locations.registration_key(metadata.registration_key_name());
    let display_name = if registration.display_name.is_empty() {
        package.name.clone()
    } else {
        registration.display_name.clone()
    };
    let display_version = if registration.display_version.is_empty() {
        package.version.clone()
    } else {
        registration.display_version.clone()
    };
    let mut parts = package
        .version
        .split('.')
        .map(|p| p.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let install_location = format!(
        "{}\\",
        install_root.display().to_string().trim_end_matches('\\')
    );
    let uninstaller = uninstaller.display().to_string();
    let estimated_kb = metadata.install().estimated_size.div_ceil(1024);

    let mut values = vec![
        ("DisplayName", Data::String(display_name)),
        ("DisplayVersion", Data::String(display_version)),
        ("Publisher", Data::String(package.publisher.clone())),
        ("InstallLocation", Data::String(install_location)),
    ];
    if !registration.display_icon.is_empty() {
        let icon = plan::absolute(install_root, &plan::to_relative(&registration.display_icon))?;
        values.push(("DisplayIcon", Data::String(icon.display().to_string())));
    }
    values.extend([
        (
            "UninstallString",
            Data::String(format!("\"{uninstaller}\"")),
        ),
        (
            "QuietUninstallString",
            Data::String(format!("\"{uninstaller}\" uninstall --quiet")),
        ),
        ("NoModify", Data::Dword(1)),
        ("NoRepair", Data::Dword(1)),
        ("InstallDate", Data::String(install_date.to_string())),
        (
            "EstimatedSize",
            Data::Dword(u32::try_from(estimated_kb).unwrap_or(u32::MAX)),
        ),
        ("VersionMajor", Data::Dword(major)),
        ("VersionMinor", Data::Dword(minor)),
    ]);
    if !package.website_url.is_empty() {
        values.push(("URLInfoAbout", Data::String(package.website_url.clone())));
    }
    if !package.help_url.is_empty() {
        values.push(("HelpLink", Data::String(package.help_url.clone())));
    }
    Ok((
        key.clone(),
        values
            .into_iter()
            .map(|(name, data)| DesiredValue {
                key: key.clone(),
                name: name.to_string(),
                data,
            })
            .collect(),
    ))
}

/// Today as `YYYYMMDD` (UTC).
pub fn install_date() -> String {
    crate::report::now_rfc3339()
        .chars()
        .take(10)
        .filter(|c| c.is_ascii_digit())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::identity::Scope;
    use tigersetup_format::metadata::{Engine, Install, Package, Registration};

    #[test]
    fn the_registration_values_derive_from_the_package() {
        let metadata = Metadata {
            schema: tigersetup_format::metadata::SCHEMA,
            package: Some(Package {
                id: "IT-Tiger.TestApp".into(),
                name: "TestApp".into(),
                version: "1.2.3".into(),
                publisher: "IT Tiger".into(),
                website_url: "https://example.invalid".into(),
                ..Default::default()
            }),
            install: Some(Install {
                scopes: vec![1],
                user_root: "%LOCALAPPDATA%\\Programs\\TestApp".into(),
                estimated_size: 1025,
                ..Default::default()
            }),
            engine: Some(Engine::default()),
            registration: Some(Registration {
                display_icon: "bin/app.exe".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let locations = crate::scope::locations(Scope::User);
        let (key, values) = values(
            &metadata,
            &locations,
            Path::new("C:\\P\\TestApp"),
            Path::new("C:\\S\\uninstall.exe"),
            "20260907",
        )
        .unwrap();
        assert_eq!(
            key.to_string(),
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.TestApp"
        );
        let get = |name: &str| values.iter().find(|v| v.name == name).unwrap().data.clone();
        assert_eq!(get("DisplayName"), Data::String("TestApp".into()));
        assert_eq!(get("DisplayVersion"), Data::String("1.2.3".into()));
        assert_eq!(
            get("InstallLocation"),
            Data::String("C:\\P\\TestApp\\".into())
        );
        assert_eq!(
            get("DisplayIcon"),
            Data::String("C:\\P\\TestApp\\bin\\app.exe".into())
        );
        assert_eq!(
            get("UninstallString"),
            Data::String("\"C:\\S\\uninstall.exe\"".into())
        );
        assert_eq!(
            get("QuietUninstallString"),
            Data::String("\"C:\\S\\uninstall.exe\" uninstall --quiet".into())
        );
        assert_eq!(get("NoModify"), Data::Dword(1));
        assert_eq!(get("EstimatedSize"), Data::Dword(2));
        assert_eq!(get("VersionMajor"), Data::Dword(1));
        assert_eq!(get("VersionMinor"), Data::Dword(2));
        assert_eq!(get("InstallDate"), Data::String("20260907".into()));
        assert_eq!(
            get("URLInfoAbout"),
            Data::String("https://example.invalid".into())
        );
        assert!(values.iter().all(|v| v.name != "HelpLink"));
        assert_eq!(install_date().len(), 8);
    }
}
