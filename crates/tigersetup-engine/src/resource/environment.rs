//! The environment-variable resource: one named value of the scope's
//! environment key, set to a template the engine expands. It is a registry
//! value with a memory: the ownership row records what TigerSetup wrote and
//! what the variable held before, so a removal puts the previous value back
//! rather than deleting it, and a value something else changed afterwards
//! is preserved and reported. The PATH list is a different resource with its
//! own rules (`resource::path`) and is never written through here.

use std::path::Path;

use tigersetup_format::Metadata;

use crate::resource::predicate::{self, Options};
use crate::resource::registry;
use crate::scope::Locations;
use crate::state::installation::OwnedEnvironmentVariable;
use crate::win::registry::{Data, KeyPath};
use crate::{Error, Result};

/// A variable the desired state wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredVariable {
    /// The environment key of the scope.
    pub key: KeyPath,
    pub name: String,
    pub data: Data,
}

/// The variables the effective options enable, expanded for this
/// installation.
pub fn desired(
    metadata: &Metadata,
    options: &Options,
    locations: &Locations,
    install_root: &Path,
) -> Vec<DesiredVariable> {
    let version = &metadata.package().version;
    metadata
        .environment_variables
        .iter()
        .filter(|variable| predicate::enabled(variable.when.as_ref(), "", options))
        .map(|variable| {
            let text = registry::expand(&variable.value, install_root, version);
            DesiredVariable {
                key: locations.environment_key.clone(),
                name: variable.name.clone(),
                data: if variable.expandable {
                    Data::ExpandString(text)
                } else {
                    Data::String(text)
                },
            }
        })
        .collect()
}

/// The data an owned row says TigerSetup wrote.
pub fn owned_data(owned: &OwnedEnvironmentVariable) -> Result<Data> {
    Data::from_columns(&owned.kind, &owned.data)
}

/// The data an owned row says was there before TigerSetup; `None` when the
/// variable did not exist.
pub fn restore_data(owned: &OwnedEnvironmentVariable) -> Result<Option<Data>> {
    match (
        owned.pre_existed,
        &owned.previous_kind,
        &owned.previous_data,
    ) {
        (true, Some(kind), Some(data)) => Ok(Some(Data::from_columns(kind, data)?)),
        (true, _, _) => Err(Error::new(
            "journal_inconsistent",
            format!(
                "environment variable {} pre-existed but its previous value was not recorded",
                owned.name
            ),
        )),
        (false, _, _) => Ok(None),
    }
}

/// `<key>\<name>`, for a finding.
pub fn location(key: &str, name: &str) -> String {
    format!("{key}\\{name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::metadata::{EnvironmentVariable, OptionValue, Predicate};

    #[test]
    fn variables_expand_and_follow_their_predicate() {
        let mut metadata = Metadata {
            package: Some(tigersetup_format::metadata::Package {
                version: "1.2.3".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        metadata.environment_variables.push(EnvironmentVariable {
            name: "TSTA_HOME".into(),
            value: "%INSTALLROOT%".into(),
            expandable: true,
            when: None,
        });
        metadata.environment_variables.push(EnvironmentVariable {
            name: "TSTA_VERSION".into(),
            value: "%VERSION%".into(),
            expandable: false,
            when: Some(Predicate {
                option: "environment".into(),
                equals: "true".into(),
            }),
        });
        let locations = crate::scope::locations(tigersetup_format::identity::Scope::User);
        let mut options = Options::new();
        options.insert("environment".into(), OptionValue::Bool(false));
        let wanted = super::desired(&metadata, &options, &locations, Path::new("C:\\P"));
        assert_eq!(wanted.len(), 1);
        assert_eq!(wanted[0].name, "TSTA_HOME");
        assert_eq!(wanted[0].data, Data::ExpandString("C:\\P".into()));
        assert_eq!(wanted[0].key.to_string(), "HKCU\\Environment");
        options.insert("environment".into(), OptionValue::Bool(true));
        let wanted = super::desired(&metadata, &options, &locations, Path::new("C:\\P"));
        assert_eq!(wanted.len(), 2);
        assert_eq!(wanted[1].data, Data::String("1.2.3".into()));

        let owned = OwnedEnvironmentVariable {
            hive_key: "HKCU\\Environment".into(),
            name: "TSTA_HOME".into(),
            kind: "expand_string".into(),
            data: "C:\\P".into(),
            pre_existed: true,
            previous_kind: Some("string".into()),
            previous_data: Some("old".into()),
        };
        assert_eq!(
            owned_data(&owned).unwrap(),
            Data::ExpandString("C:\\P".into())
        );
        assert_eq!(
            restore_data(&owned).unwrap(),
            Some(Data::String("old".into()))
        );
        let fresh = OwnedEnvironmentVariable {
            pre_existed: false,
            previous_kind: None,
            previous_data: None,
            ..owned.clone()
        };
        assert_eq!(restore_data(&fresh).unwrap(), None);
        let broken = OwnedEnvironmentVariable {
            previous_kind: None,
            ..owned
        };
        assert!(restore_data(&broken).is_err());
    }
}
