//! The registry value resource: a typed value under a key of the scope's
//! software root, owned at value level; the keys it needs are owned only
//! when TigerSetup created them.

use std::path::Path;

use tigersetup_format::Metadata;
use tigersetup_format::metadata::RegistryKind;

use crate::scope::Locations;
use crate::state::installation::OwnedRegistryValue;
use crate::win::registry::{Data, KeyPath};
use crate::{Error, Result};

/// A value the desired state wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredValue {
    pub key: KeyPath,
    pub name: String,
    pub data: Data,
}

/// Expands the placeholders a registry value's data may carry.
pub fn expand(template: &str, install_root: &Path, version: &str) -> String {
    template
        .replace("%INSTALLROOT%", &install_root.display().to_string())
        .replace("%VERSION%", version)
}

fn data_for(kind: i32, data: String, what: &str) -> Result<Data> {
    match RegistryKind::try_from(kind) {
        Ok(RegistryKind::String) => Ok(Data::String(data)),
        Ok(RegistryKind::ExpandString) => Ok(Data::ExpandString(data)),
        Ok(RegistryKind::Dword) => data.parse().map(Data::Dword).map_err(|_| {
            Error::new(
                "metadata_invalid",
                format!("registry value {what} is a DWORD but {data:?} is not a number"),
            )
        }),
        _ => Err(Error::new(
            "metadata_invalid",
            format!("registry value {what} has no kind"),
        )),
    }
}

/// The product's declared values, expanded for this installation.
pub fn product_values(
    metadata: &Metadata,
    locations: &Locations,
    install_root: &Path,
) -> Result<Vec<DesiredValue>> {
    let version = &metadata.package().version;
    metadata
        .registry_values
        .iter()
        .map(|value| {
            Ok(DesiredValue {
                key: locations.software_key(&value.key),
                name: value.name.clone(),
                data: data_for(
                    value.kind,
                    expand(&value.data, install_root, version),
                    &format!("{}\\{}", value.key, value.name),
                )?,
            })
        })
        .collect()
}

/// The typed data an owned row records.
pub fn owned_data(owned: &OwnedRegistryValue) -> Result<Data> {
    Data::from_columns(&owned.kind, &owned.data)
}

/// Whether a key path is *the* Add/Remove Programs key of this installation —
/// an exact match, not "somewhere under the uninstall root". Planning splits
/// owned values on this, so a key merely below the registration would be
/// planned as a product value, which is what the split intends.
pub fn is_registration_key(key: &str, registration_key: Option<&str>) -> bool {
    registration_key.is_some_and(|r| r.eq_ignore_ascii_case(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_expand_in_data() {
        assert_eq!(
            expand("%INSTALLROOT%\\bin;%VERSION%", Path::new("C:\\P"), "1.2.3"),
            "C:\\P\\bin;1.2.3"
        );
        assert!(matches!(
            data_for(RegistryKind::Dword as i32, "7".into(), "k\\v").unwrap(),
            Data::Dword(7)
        ));
        assert_eq!(
            data_for(RegistryKind::Dword as i32, "x".into(), "k\\v")
                .unwrap_err()
                .code,
            "metadata_invalid"
        );
    }
}
