//! Every choice that depends on the installation scope, in one place: the
//! hive and its software root, the environment key that holds `Path`, the
//! Add/Remove Programs root, the shortcut folders, the roots the identity
//! module derives, the access control list of a machine-scope state
//! directory, and the confinement that decides whether a location stored in
//! the database may be mutated at all.
//!
//! Confinement matters because a machine-scope uninstall runs elevated and
//! plans from the database. `plan::absolute` already keeps every stored file
//! path under the install root; the same reasoning applies to the other
//! resource kinds, so a tampered row can name only a PATH or environment
//! key that is the scope's environment key, a registration key under the
//! scope's Add/Remove Programs root, a registry key or value in the scope's
//! own hive — a product value may live at an explicit location outside
//! `Software`, and a hive cannot move — and only a shortcut inside the
//! scope's own Start Menu or desktop folder. The machine-scope database is
//! writable by administrators alone (`MACHINE_STATE_DIRECTORY_DACL`), so a
//! row there names nothing its writer could not already reach.

use std::path::{Path, PathBuf};

use tigersetup_format::identity::{self, Scope};
use tigersetup_format::metadata::ShortcutLocation;

use crate::state::installation::Owned;
use crate::win::registry::{Hive, KeyPath};
use crate::win::{acl, env};
use crate::{Error, Result};

/// The machine-scope state directory's security: owned by the local
/// Administrators group, full control for SYSTEM and Administrators, read and
/// execute for everyone else, inheriting nothing and inherited by everything
/// inside.
///
/// The owner is part of it. `%ProgramData%` lets any user create a
/// subdirectory, and an object's owner keeps `WRITE_DAC` however the list
/// reads — so a state directory a standard user created first would stay that
/// user's to reopen, and the uninstaller in it is a binary Add/Remove
/// Programs later runs elevated (`TigerSetup-Design.md` §5.11).
pub const MACHINE_STATE_DIRECTORY_DACL: &str =
    "O:BAD:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";

/// The owners a machine-scope state directory may already have: SYSTEM and
/// the local Administrators group. Anything else means an unprivileged user
/// got there first.
///
/// Both spellings, because Windows writes a well-known trustee back as its
/// SDDL alias (`BA`, `SY`) rather than the SID that was given to it — reading
/// only the SIDs would make every run believe it had to claim the directory
/// again.
const TRUSTED_OWNERS: [&str; 4] = ["S-1-5-18", "S-1-5-32-544", "SY", "BA"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    pub scope: Scope,
    pub hive: Hive,
    /// `HKCU\Software` or `HKLM\Software`: product registry values live
    /// below it.
    pub software_root: KeyPath,
    /// The key whose `Path` value is the scope's PATH.
    pub environment_key: KeyPath,
    /// `...\Microsoft\Windows\CurrentVersion\Uninstall` of the hive.
    pub uninstall_root: KeyPath,
    /// Template of the Start Menu Programs folder.
    pub programs_folder: &'static str,
    /// Template of the desktop folder.
    pub desktop_folder: &'static str,
    /// Template of the Startup folder.
    pub startup_folder: &'static str,
    /// Template of the Send To folder; `None` for machine scope, which has
    /// no shared one.
    pub send_to_folder: Option<&'static str>,
}

/// The locations of a scope.
pub fn locations(scope: Scope) -> Locations {
    let hive = match scope {
        Scope::User => Hive::CurrentUser,
        Scope::Machine => Hive::LocalMachine,
    };
    Locations {
        scope,
        hive,
        software_root: KeyPath::new(hive, "Software"),
        environment_key: match scope {
            Scope::User => KeyPath::new(hive, "Environment"),
            Scope::Machine => KeyPath::new(
                hive,
                "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
            ),
        },
        uninstall_root: KeyPath::new(
            hive,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
        programs_folder: match scope {
            Scope::User => "%PROGRAMS%",
            Scope::Machine => "%COMMONPROGRAMS%",
        },
        desktop_folder: match scope {
            Scope::User => "%DESKTOP%",
            Scope::Machine => "%COMMONDESKTOP%",
        },
        startup_folder: match scope {
            Scope::User => "%STARTUP%",
            Scope::Machine => "%COMMONSTARTUP%",
        },
        send_to_folder: match scope {
            Scope::User => Some("%SENDTO%"),
            Scope::Machine => None,
        },
    }
}

impl Locations {
    /// The state directory template for a product.
    pub fn state_directory_template(&self, product_id: &str) -> String {
        identity::state_directory_template(self.scope, product_id)
    }

    /// The Add/Remove Programs key of a registration key name.
    pub fn registration_key(&self, key_name: &str) -> KeyPath {
        self.uninstall_root.child(key_name)
    }

    /// A product key path relative to the software root.
    pub fn software_key(&self, relative: &str) -> KeyPath {
        self.software_root.child(relative)
    }

    /// The deepest key Windows itself owns above `key`: the root a chain
    /// of created keys stops at, so that TigerSetup owns nothing of
    /// Windows's own. `Software\Classes` and `App Paths` are Windows's for
    /// the hive, as `Software` and the Add/Remove Programs root are; a key
    /// below a root that is missing is still created whole, so a root that
    /// happens to be absent costs nothing. A key that itself holds a value
    /// (`RegisteredApplications`) is not a root: it is created and owned
    /// where it is missing, and left alone where Windows already has it.
    ///
    /// A key at an explicit location outside `Software` stops at the hive's
    /// top-level key (`SYSTEM`, …), which is Windows's; below it, every key
    /// that is already there is left alone and unowned, as always.
    pub fn key_chain_root(&self, key: &KeyPath) -> KeyPath {
        let roots = [
            self.uninstall_root.clone(),
            self.software_key("Microsoft\\Windows\\CurrentVersion\\App Paths"),
            self.software_key("Classes"),
            self.software_root.clone(),
        ];
        roots
            .into_iter()
            .find(|root| key.is_under(root))
            .unwrap_or_else(|| {
                let top = key.subkey.split('\\').next().unwrap_or_default();
                KeyPath::new(key.hive, top)
            })
    }

    /// The template of the folder a shortcut location names in this scope,
    /// or `None` where the scope has no such folder.
    pub fn shortcut_folder(&self, location: ShortcutLocation) -> Option<&'static str> {
        match location {
            ShortcutLocation::Desktop => Some(self.desktop_folder),
            ShortcutLocation::Startup => Some(self.startup_folder),
            ShortcutLocation::SendTo => self.send_to_folder,
            _ => Some(self.programs_folder),
        }
    }

    /// Every shortcut folder of this scope — Start Menu Programs, desktop,
    /// Startup and, for user scope, Send To — resolved on this machine.
    pub fn shortcut_folders(&self) -> Result<Vec<PathBuf>> {
        [
            self.programs_folder,
            self.desktop_folder,
            self.startup_folder,
        ]
        .iter()
        .chain(self.send_to_folder.iter())
        .map(|template| {
            Ok(PathBuf::from(identity::expand_template(
                template,
                env::known_folder,
            )?))
        })
        .collect()
    }

    /// Whether a stored key of a PATH entry, an environment variable or the
    /// registration is one this scope may mutate: the scope's software root
    /// (which contains its Add/Remove Programs root, its classes and its
    /// `App Paths`) or its environment key (which holds `Path` and the
    /// environment variables).
    pub fn allows_key(&self, key: &KeyPath) -> bool {
        key.is_under(&self.software_root)
            || key.is_under(&self.uninstall_root)
            || key.is_under(&self.environment_key)
    }

    /// Refuses a stored key that lies outside this scope's roots.
    pub fn confine_key(&self, key: &KeyPath) -> Result<()> {
        if self.allows_key(key) {
            return Ok(());
        }
        Err(outside(format!(
            "stored registry key {key} is outside {}, {} and {}",
            self.software_root, self.uninstall_root, self.environment_key
        )))
    }

    /// Whether a stored registry key or value is one this scope may mutate:
    /// any key of the scope's own hive, because a product value may be
    /// declared at an explicit location outside `Software` and the rows of
    /// an installed version must stay usable by the version that replaces
    /// it, whatever that version declares.
    pub fn allows_registry_key(&self, key: &KeyPath) -> bool {
        key.hive == self.hive
    }

    /// Refuses a stored registry key or value that lies in the other hive.
    pub fn confine_registry_key(&self, key: &KeyPath) -> Result<()> {
        if self.allows_registry_key(key) {
            return Ok(());
        }
        Err(outside(format!(
            "stored registry key {key} is outside the {} hive",
            self.hive.as_str()
        )))
    }

    /// Refuses a stored shortcut path that lies outside this scope's Start
    /// Menu Programs and desktop folders.
    pub fn confine_shortcut(&self, link: &Path) -> Result<()> {
        let folders = self.shortcut_folders()?;
        if is_inside(link, &folders) {
            return Ok(());
        }
        let names: Vec<String> = folders.iter().map(|f| f.display().to_string()).collect();
        Err(outside(format!(
            "stored shortcut {} is outside {}",
            link.display(),
            names.join(" and ")
        )))
    }

    /// Refuses everything in `owned` that this scope may not mutate, before
    /// a plan is made from it: the registry keys and values (confined to the
    /// hive), the PATH hives, the environment variables and the registration
    /// key (confined to the scope's roots). A hive does not move, so a
    /// stored key outside this scope means the database is wrong about the
    /// machine and the run must not continue.
    ///
    /// Shortcuts are deliberately not refused here. Their folders *do* move
    /// under a live installation — OneDrive's Known Folder Move relocates the
    /// desktop, policy can redirect the Start Menu — and refusing the run
    /// would leave the product uninstallable. The planner preserves and
    /// reports such a link instead, and the executor still refuses to act on
    /// one, so nothing outside the scope is touched either way.
    ///
    /// Stored file and directory paths are confined to the install root by
    /// `plan::absolute` where they are used.
    pub fn confine_owned(&self, owned: &Owned) -> Result<()> {
        for key in owned
            .registry_keys
            .iter()
            .map(|k| &k.key)
            .chain(owned.registry_values.iter().map(|v| &v.key))
        {
            self.confine_registry_key(&KeyPath::parse(key)?)?;
        }
        let mut keys: Vec<String> = Vec::new();
        keys.extend(owned.path_entries.iter().map(|e| e.hive_key.clone()));
        keys.extend(
            owned
                .environment_variables
                .iter()
                .map(|v| v.hive_key.clone()),
        );
        keys.extend(owned.registration_key.clone());
        for key in &keys {
            self.confine_key(&KeyPath::parse(key)?)?;
        }
        Ok(())
    }

    /// Gives the machine-scope state directory its own owner and access
    /// control list, and repairs one that has drifted, before anything is
    /// written into it. A user-scope directory keeps the profile's own
    /// protection.
    ///
    /// Both halves are checked, because a directory can carry the right list
    /// and the wrong owner: `%ProgramData%` lets any user create a
    /// subdirectory, and the creator owns what it creates.
    pub fn protect_state_directory(&self, state_dir: &Path) -> Result<Protection> {
        if self.scope != Scope::Machine {
            return Ok(Protection::NotApplicable);
        }
        if !crate::elevation::is_elevated() {
            return Ok(Protection::Unprivileged);
        }
        let owner = acl::read_owner(state_dir).unwrap_or_default();
        let owned_by_us = TRUSTED_OWNERS.contains(&owner.to_ascii_uppercase().as_str());
        if owned_by_us
            && let Ok(current) = acl::read_dacl(state_dir)
            && let Some(current) = acl::Dacl::parse(&current)
            && current.is(&expected_state_directory_dacl())
        {
            return Ok(Protection::Intact);
        }
        // Taking ownership is the point of the write when somebody else got
        // here first: without it the previous owner could hand the rights
        // straight back to itself through WRITE_DAC.
        acl::set_dacl(state_dir, MACHINE_STATE_DIRECTORY_DACL)?;
        match owned_by_us {
            true => Ok(Protection::Written),
            false => Ok(Protection::Claimed),
        }
    }
}

/// What [`Locations::protect_state_directory`] did with the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// User scope: the state directory is inside the user's own profile,
    /// which already keeps every other user out of it.
    NotApplicable,
    /// Machine scope without an administrator token. Production never
    /// reaches here — a machine-scope run elevates first — and the folder
    /// seams that do reach it put the directory inside a tree the caller
    /// owns, where no access control list protects anything from the
    /// caller.
    Unprivileged,
    /// The list was already the one this engine writes.
    Intact,
    /// The list was written: the directory was new, or its list had drifted
    /// and was repaired.
    Written,
    /// The directory already existed and somebody else owned it — a standard
    /// user may create it under `%ProgramData%` before any install. Its
    /// ownership and its list were taken over.
    Claimed,
}

impl Protection {
    /// The stable code a run logs for this result, or `None` where there is
    /// nothing worth saying.
    pub fn code(self) -> Option<&'static str> {
        match self {
            Protection::NotApplicable | Protection::Intact => None,
            Protection::Unprivileged => Some("state_directory_protection_skipped"),
            Protection::Written => Some("state_directory_protected"),
            Protection::Claimed => Some("state_directory_ownership_claimed"),
        }
    }
}

/// Whether a link lies in one of `folders`. The planner and the executor ask
/// the same question at different moments and must get the same answer, so
/// they ask it here: Windows paths compare case-insensitively, which
/// `Path::starts_with` does not.
pub fn is_inside(link: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|folder| under(link, folder))
}

fn outside(message: String) -> Error {
    Error::new("path_outside_root", message)
}

/// Whether `path` is `folder` or lies below it, comparing case-insensitively
/// as Windows paths do.
fn under(path: &Path, folder: &Path) -> bool {
    let folder = folder
        .display()
        .to_string()
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let path = path.display().to_string().to_ascii_lowercase();
    !folder.is_empty() && path.starts_with(&format!("{folder}\\"))
}

/// [`MACHINE_STATE_DIRECTORY_DACL`] parsed, for comparison with what a
/// directory carries.
fn expected_state_directory_dacl() -> acl::Dacl {
    acl::Dacl::parse(MACHINE_STATE_DIRECTORY_DACL)
        .expect("the state directory access control list is valid SDDL")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::installation::{
        OwnedPathEntry, OwnedRegistryKey, OwnedRegistryValue, OwnedShortcut,
    };

    #[test]
    fn user_and_machine_scope_differ_only_in_their_locations() {
        let user = locations(Scope::User);
        assert_eq!(user.environment_key.to_string(), "HKCU\\Environment");
        assert_eq!(
            user.registration_key("IT-Tiger.TestApp").to_string(),
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.TestApp"
        );
        assert_eq!(
            user.software_key("IT Tiger\\TestApp").to_string(),
            "HKCU\\Software\\IT Tiger\\TestApp"
        );
        assert_eq!(
            user.key_chain_root(&user.software_key("Classes\\.tigertest\\OpenWithProgids")),
            user.software_key("Classes")
        );
        assert_eq!(
            user.key_chain_root(
                &user.software_key("Microsoft\\Windows\\CurrentVersion\\App Paths\\app.exe")
            ),
            user.software_key("Microsoft\\Windows\\CurrentVersion\\App Paths")
        );
        assert_eq!(
            user.key_chain_root(&user.software_key("IT Tiger\\TestApp\\Capabilities")),
            user.software_root
        );
        assert_eq!(user.programs_folder, "%PROGRAMS%");
        let machine = locations(Scope::Machine);
        assert_eq!(machine.hive, Hive::LocalMachine);
        assert!(machine.environment_key.subkey.contains("Session Manager"));
        assert_eq!(machine.desktop_folder, "%COMMONDESKTOP%");
        assert_eq!(
            machine.state_directory_template("X"),
            "%PROGRAMDATA%\\TigerSetup\\X"
        );
        assert_eq!(
            machine.registration_key("IT-Tiger.TestApp").to_string(),
            "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.TestApp"
        );
    }

    #[test]
    fn a_scope_confines_registry_keys_to_its_own_hive_and_roots() {
        let user = locations(Scope::User);
        for allowed in [
            "HKCU\\Software\\IT Tiger\\TestApp",
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IT-Tiger.TestApp",
            "HKCU\\Environment",
        ] {
            user.confine_key(&KeyPath::parse(allowed).unwrap()).unwrap();
        }
        for refused in [
            "HKLM\\Software\\IT Tiger\\TestApp",
            "HKCU\\Console",
            "HKCU\\SOFTWARE2\\IT Tiger",
            "HKCU\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        ] {
            let err = user
                .confine_key(&KeyPath::parse(refused).unwrap())
                .unwrap_err();
            assert_eq!(err.code, "path_outside_root", "{refused}");
        }
        let machine = locations(Scope::Machine);
        machine
            .confine_key(&KeyPath::parse("HKLM\\Software\\IT Tiger").unwrap())
            .unwrap();
        machine
            .confine_key(
                &KeyPath::parse(
                    "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(
            machine
                .confine_key(&KeyPath::parse("HKCU\\Software\\IT Tiger").unwrap())
                .unwrap_err()
                .code,
            "path_outside_root"
        );
    }

    /// A registry key or value row may name any key of the scope's hive —
    /// a product value at an explicit location outside `Software` — and
    /// never a key of the other hive.
    #[test]
    fn a_registry_row_is_confined_to_the_hive_and_an_explicit_key_stops_at_its_top_level_key() {
        let machine = locations(Scope::Machine);
        let explicit =
            KeyPath::parse("HKLM\\SYSTEM\\CurrentControlSet\\Control\\FileSystem").unwrap();
        machine.confine_registry_key(&explicit).unwrap();
        assert_eq!(
            machine
                .confine_registry_key(&KeyPath::parse("HKCU\\SYSTEM\\Anything").unwrap())
                .unwrap_err()
                .code,
            "path_outside_root"
        );
        assert_eq!(
            machine.key_chain_root(&explicit),
            KeyPath::parse("HKLM\\SYSTEM").unwrap(),
            "the hive's top-level key is Windows's and is never owned"
        );
        assert_eq!(
            machine.key_chain_root(&KeyPath::parse("HKLM\\Software\\Policies\\Vendor").unwrap()),
            machine.software_root,
            "an explicit key under Software stops where every product key does"
        );
        let mut owned = Owned::default();
        owned.registry_values.push(OwnedRegistryValue {
            key: explicit.to_string(),
            name: "LongPathsEnabled".into(),
            kind: "dword".into(),
            data: "1".into(),
            pre_existed: true,
            previous_kind: Some("dword".into()),
            previous_data: Some("0".into()),
        });
        owned.registry_keys.push(OwnedRegistryKey {
            key: "HKLM\\SYSTEM\\TigerSetupTest".into(),
            created: true,
        });
        machine.confine_owned(&owned).unwrap();
        assert_eq!(
            locations(Scope::User)
                .confine_owned(&owned)
                .unwrap_err()
                .code,
            "path_outside_root"
        );
    }

    #[test]
    fn a_tampered_owned_row_is_refused_before_anything_is_planned() {
        let user = locations(Scope::User);
        user.confine_owned(&Owned::default()).unwrap();

        let mut owned = Owned::default();
        owned.registry_values.push(OwnedRegistryValue {
            key: "HKLM\\SYSTEM\\CurrentControlSet\\Services\\Anything".into(),
            name: "ImagePath".into(),
            kind: "string".into(),
            data: "x".into(),
            pre_existed: false,
            previous_kind: None,
            previous_data: None,
        });
        assert_eq!(
            user.confine_owned(&owned).unwrap_err().code,
            "path_outside_root"
        );

        let mut owned = Owned::default();
        owned.path_entries.push(OwnedPathEntry {
            hive_key: "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"
                .into(),
            raw: "C:\\x".into(),
            normalized: "c:\\x".into(),
            pre_existed: false,
            added: true,
        });
        assert_eq!(
            user.confine_owned(&owned).unwrap_err().code,
            "path_outside_root",
            "a user-scope installation may not touch the machine PATH"
        );

        // A shortcut is deliberately not refused here. Its folder can move
        // under a live installation — OneDrive's Known Folder Move relocates
        // the desktop — and refusing would leave the product uninstallable.
        // The planner preserves and reports such a link instead, and
        // `confine_shortcut` still refuses to act on one.
        let mut owned = Owned::default();
        owned.shortcuts.push(OwnedShortcut {
            path: "C:\\Windows\\System32\\anything.lnk".into(),
            target: "C:\\x\\app.exe".into(),
        });
        user.confine_owned(&owned).unwrap();
        assert_eq!(
            user.confine_shortcut(Path::new("C:\\Windows\\System32\\anything.lnk"))
                .unwrap_err()
                .code,
            "path_outside_root"
        );
    }

    #[test]
    fn a_shortcut_in_the_scopes_own_folder_is_allowed() {
        let user = locations(Scope::User);
        let folders = user.shortcut_folders().unwrap();
        assert_eq!(folders.len(), 4, "programs, desktop, startup and send to");
        assert_eq!(
            locations(Scope::Machine).shortcut_folders().unwrap().len(),
            3,
            "machine scope has no shared Send To folder"
        );
        assert_eq!(
            locations(Scope::Machine).shortcut_folder(ShortcutLocation::SendTo),
            None
        );
        assert_eq!(
            user.shortcut_folder(ShortcutLocation::Startup),
            Some("%STARTUP%")
        );
        let link = folders[0].join("TestApp.lnk");
        user.confine_shortcut(&link).unwrap();
        // The folder itself is not a link path, and a sibling folder whose
        // name merely starts the same way is outside.
        assert!(user.confine_shortcut(&folders[0]).is_err());
        let sibling = PathBuf::from(format!("{}X\\TestApp.lnk", folders[0].display()));
        assert_eq!(
            user.confine_shortcut(&sibling).unwrap_err().code,
            "path_outside_root"
        );
    }

    #[test]
    fn the_state_directory_list_is_recognised_only_when_it_is_intact() {
        let wanted = expected_state_directory_dacl();
        let read = |text: &str| acl::Dacl::parse(text).unwrap().is(&wanted);
        assert!(read(
            "D:P(A;OICI;FA;;;SY)(A;OICI;FRFX;;;BU)(A;OICI;FA;;;BA)"
        ));
        // Inheritance still on, an entry missing, an inherited entry
        // present, or a standard user granted more than read and execute:
        // each means the list has to be written again.
        assert!(!read(
            "D:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)"
        ));
        assert!(!read("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"));
        assert!(!read(
            "D:PAI(A;OICIID;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)"
        ));
        assert!(!read("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;BU)"));
    }

    /// A user-scope run leaves the directory's protection alone; the
    /// machine-scope list is written only by a run that has the
    /// administrator token machine scope needs anyway.
    #[test]
    fn the_state_directorys_protection_belongs_to_elevated_machine_scope() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            locations(Scope::User)
                .protect_state_directory(dir.path())
                .unwrap(),
            Protection::NotApplicable
        );
        let machine = locations(Scope::Machine)
            .protect_state_directory(dir.path())
            .unwrap();
        if crate::elevation::is_elevated() {
            assert_eq!(machine, Protection::Written);
            let read = acl::Dacl::parse(&acl::read_dacl(dir.path()).unwrap()).unwrap();
            assert!(read.is(&expected_state_directory_dacl()), "{read:?}");
            assert_eq!(
                locations(Scope::Machine)
                    .protect_state_directory(dir.path())
                    .unwrap(),
                Protection::Intact,
                "a list already in place is left alone"
            );
        } else {
            assert_eq!(machine, Protection::Unprivileged);
            assert_eq!(
                machine.code(),
                Some("state_directory_protection_skipped"),
                "this test process is not elevated, so it cannot write the list"
            );
        }
    }
}
