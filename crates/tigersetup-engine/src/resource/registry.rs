//! The registry value resource: a typed value under a key of the scope's
//! software root, or at an explicit location in the scope's hive, owned at
//! value level; the keys it needs are owned only when TigerSetup created
//! them. The ownership row records what was there before the value was
//! written, so that taking the value away puts that back — a value that
//! pre-existed is restored, one TigerSetup created is deleted — and a value
//! somebody changed after installation is preserved and reported, exactly
//! as an environment variable is.

use std::path::Path;

use tigersetup_format::Metadata;
use tigersetup_format::metadata::RegistryKind;

use crate::resource::predicate::{self, Options};
use crate::scope::Locations;
use crate::state::installation::OwnedRegistryValue;
use crate::win::registry::{Data, Hive, KeyPath};
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

/// The product's declared values the effective options enable, expanded
/// for this installation.
pub fn product_values(
    metadata: &Metadata,
    options: &Options,
    locations: &Locations,
    install_root: &Path,
) -> Result<Vec<DesiredValue>> {
    let version = &metadata.package().version;
    metadata
        .registry_values
        .iter()
        .filter(|value| predicate::enabled(value.when.as_ref(), "", options))
        .map(|value| {
            // An explicit root names a hive, and the format has already
            // required that hive to be the one every scope of the package
            // writes; metadata that says otherwise is refused here rather
            // than written into the wrong hive.
            let key = match value.explicit_root() {
                None => locations.software_key(&value.key),
                Some(Ok(hive_scope)) if hive_scope == locations.scope => {
                    let hive = match hive_scope {
                        tigersetup_format::identity::Scope::Machine => Hive::LocalMachine,
                        tigersetup_format::identity::Scope::User => Hive::CurrentUser,
                    };
                    KeyPath::new(hive, value.key.clone())
                }
                Some(_) => {
                    return Err(Error::new(
                        "registry_root_outside_scope",
                        format!(
                            "registry value {}\\{}\\{} is not in the hive {} scope writes",
                            value.root_name(),
                            value.key,
                            value.name,
                            locations.scope.as_str()
                        ),
                    ));
                }
            };
            Ok(DesiredValue {
                key,
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

/// The typed data an owned row records: what TigerSetup wrote.
pub fn owned_data(owned: &OwnedRegistryValue) -> Result<Data> {
    Data::from_columns(&owned.kind, &owned.data)
}

/// The data an owned row says was there before TigerSetup wrote the value;
/// `None` when the value did not exist — which is also what every row
/// written before the previous state was recorded says, so an installation
/// from an older engine is taken away exactly as it always was.
pub fn restore_data(owned: &OwnedRegistryValue) -> Result<Option<Data>> {
    match (
        owned.pre_existed,
        &owned.previous_kind,
        &owned.previous_data,
    ) {
        (true, Some(kind), Some(data)) => Ok(Some(Data::from_columns(kind, data)?)),
        (true, _, _) => Err(Error::new(
            "journal_inconsistent",
            format!(
                "registry value {}\\{} pre-existed but its previous value was not recorded",
                owned.key, owned.name
            ),
        )),
        (false, _, _) => Ok(None),
    }
}

/// `<key>\<name>`, for a finding.
pub fn location(key: &str, name: &str) -> String {
    format!("{key}\\{name}")
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
