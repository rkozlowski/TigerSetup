//! Shell completion (`TigerSetup-Design.md` §5.15): which PowerShell profile
//! blocks a package wants on this machine, and for whom.
//!
//! The package carries each completion script as an ordinary file and
//! declares which command it completes and under which predicate — the one of
//! the PATH entry that puts the command on PATH, because completion is
//! registered by command name and works only for a command PATH finds. The
//! engine activates a script with a marked block (`resource::profile`) in the
//! CurrentUserAllHosts profile of Windows PowerShell 5.1 and of PowerShell 7.
//!
//! **Whose profiles.** Only the installing user's, edited from a process
//! running as that user, never another account's and never an all-users
//! profile under `$PSHOME`. A process that is not elevated is its own user. An
//! elevated one — a machine-scope run — edits profiles only when it is the
//! account signed in to this desktop: the administrator who consented to the
//! prompt. An administrator who elevated over a standard user's shoulder, a
//! service account and session 0 have no profile here to activate, and the
//! run says so instead.
//!
//! **What may be loaded.** A profile runs its scripts as its user, so a
//! script that any other account could change would let that account run
//! code as this one. A script whose file or nearest existing directory
//! another account may change — an install root outside `%ProgramFiles%` and
//! `%LOCALAPPDATA%`, a drive root that lets authenticated users add to it —
//! is not activated, and the run says so.
//!
//! **What may not be done.** The execution policy is never changed. Windows
//! PowerShell 5.1 on a Windows client runs no profile at all until somebody
//! allows scripts, so a run that activates completion where the effective
//! policy would not run the profile says so, and changes nothing about it.

use std::path::{Path, PathBuf};

use tigersetup_format::Metadata;

use crate::Result;
use crate::plan::DesiredFile;
use crate::report::Finding;
use crate::resource::predicate::{self, Options};
use crate::resource::profile;
use crate::win::registry::{self as winreg, Data, KeyPath, Roots};

/// One profile block a run wants: in `profile`, named by `marker`, loading
/// the script, for the account `user` (a SID).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredProfileBlock {
    pub profile: PathBuf,
    pub marker: String,
    pub block: String,
    pub user: String,
}

/// Whose profiles this run may edit, or why it may edit none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    Profiles { user: String, documents: PathBuf },
    Unreachable { code: &'static str, detail: String },
}

/// The SIDs of the service accounts, which have no interactive profile.
const SERVICE_ACCOUNTS: [&str; 3] = ["S-1-5-18", "S-1-5-19", "S-1-5-20"];

impl Reach {
    /// The profiles of the account this process runs as, where that account
    /// is the one installing on this desktop.
    pub fn resolve() -> Reach {
        let Some(user) = crate::win::interactive::this_process_user_sid() else {
            return Reach::unreachable(
                "completion_user_unknown",
                "the account this run is made as cannot be read",
            );
        };
        if SERVICE_ACCOUNTS.contains(&user.as_str()) {
            return Reach::unreachable(
                "completion_service_account",
                format!("{user} is a service account and has no PowerShell profile to activate"),
            );
        }
        if crate::elevation::is_elevated() && !Reach::documents_redirected() {
            match crate::win::interactive::desktop_user_sid() {
                None => {
                    return Reach::unreachable(
                        "completion_no_desktop_user",
                        "nobody is signed in to this desktop: only the installing user's profiles are activated",
                    );
                }
                Some(desktop) if !desktop.eq_ignore_ascii_case(&user) => {
                    return Reach::unreachable(
                        "completion_installer_not_desktop_user",
                        format!(
                            "this run is elevated as {user}, not as {desktop}, who is signed in: another account's profiles are never edited"
                        ),
                    );
                }
                Some(_) => {}
            }
        }
        // A test build that relocates the registry and not Documents would
        // reach the real profiles of whoever runs the tests.
        #[cfg(any(test, feature = "test-seams"))]
        if Roots::from_env().is_relocated() && !Reach::documents_redirected() {
            return Reach::unreachable(
                "completion_documents_unavailable",
                "a relocated machine has no Documents folder of its own",
            );
        }
        match crate::win::env::known_folder("DOCUMENTS") {
            Some(documents) => Reach::Profiles {
                user,
                documents: PathBuf::from(documents),
            },
            None => Reach::unreachable(
                "completion_documents_unavailable",
                "the user's Documents folder cannot be resolved",
            ),
        }
    }

    /// [`Reach::resolve`] for a run that may touch a profile — the package
    /// declares a completion, or the installation owns a block — so a run of
    /// any other package asks Windows nothing about its desktop.
    pub fn for_run(declared: bool, owned: bool) -> Reach {
        if !declared && !owned {
            return Reach::unreachable(
                "completion_not_declared",
                "the package declares no PowerShell completion",
            );
        }
        Reach::resolve()
    }

    fn unreachable(code: &'static str, detail: impl Into<String>) -> Reach {
        Reach::Unreachable {
            code,
            detail: detail.into(),
        }
    }

    /// Whether the test seam stands in for Documents.
    fn documents_redirected() -> bool {
        #[cfg(any(test, feature = "test-seams"))]
        return std::env::var_os(format!("{}DOCUMENTS", crate::win::env::TEST_FOLDER_PREFIX))
            .is_some();
        #[cfg(not(any(test, feature = "test-seams")))]
        false
    }

    /// The account whose profiles this run may edit.
    pub fn user(&self) -> Option<&str> {
        match self {
            Reach::Profiles { user, .. } => Some(user),
            Reach::Unreachable { .. } => None,
        }
    }
}

/// The completions the effective options enable, one per command: those
/// whose predicate holds and whose script is among the files the run
/// installs, as `(command, install-relative script)`.
pub fn enabled<'a>(
    metadata: &'a Metadata,
    options: &Options,
    files: &[DesiredFile],
) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for completion in &metadata.shell_completions {
        let script = completion.script.replace('/', "\\");
        if !predicate::enabled(completion.when.as_ref(), "", options)
            || !files.iter().any(|f| f.path.eq_ignore_ascii_case(&script))
            || out
                .iter()
                .any(|(command, _)| command.eq_ignore_ascii_case(&completion.command))
        {
            continue;
        }
        out.push((&completion.command, &completion.script));
    }
    out
}

/// The profile blocks the run wants, and what it found worth saying about
/// the ones it does not create.
pub fn desired(
    metadata: &Metadata,
    options: &Options,
    install_root: &Path,
    files: &[DesiredFile],
    reach: &Reach,
    roots: &Roots,
) -> Result<(Vec<DesiredProfileBlock>, Vec<Finding>)> {
    let enabled = enabled(metadata, options, files);
    let mut blocks = Vec::new();
    let mut findings = Vec::new();
    if enabled.is_empty() {
        return Ok((blocks, findings));
    }
    let (user, documents) = match reach {
        Reach::Profiles { user, documents } => (user, documents),
        Reach::Unreachable { code, detail } => {
            findings.push(Finding::plain(code, detail.clone()));
            return Ok((blocks, findings));
        }
    };
    let product_id = &metadata.package().id;
    for (command, script) in enabled {
        let script = crate::plan::absolute(install_root, &crate::plan::to_relative(script))?;
        if let Some(writer) = untrusted_writer(&script, user) {
            findings.push(Finding {
                code: "completion_script_writable_by_others",
                path: Some(script.display().to_string()),
                detail: Some(format!(
                    "{writer} can change it, so it is not loaded into {user}'s PowerShell sessions"
                )),
            });
            continue;
        }
        let marker = profile::marker(command, product_id);
        let text = profile::block(&marker, &script);
        for path in profile::profiles(documents) {
            blocks.push(DesiredProfileBlock {
                profile: path,
                marker: marker.clone(),
                block: text.clone(),
                user: user.clone(),
            });
        }
    }
    if !blocks.is_empty()
        && let Some(policy) = windows_powershell_blocks_profiles(roots)
    {
        findings.push(Finding::plain(
            "windows_powershell_profiles_not_run",
            format!(
                "Windows PowerShell's execution policy is {policy}, so it runs no profile and completion works there only once scripts are allowed; TigerSetup never changes the policy"
            ),
        ));
    }
    Ok((blocks, findings))
}

/// Another account that may change the script — the file, or the nearest
/// directory above it that exists — or `None`. An access control list that
/// cannot be read counts as a writer: nothing unverified is loaded.
fn untrusted_writer(script: &Path, user: &str) -> Option<String> {
    let mut candidates: Vec<&Path> = Vec::new();
    if script.exists() {
        candidates.push(script);
    }
    if let Some(directory) = script.ancestors().skip(1).find(|p| p.is_dir()) {
        candidates.push(directory);
    }
    for candidate in candidates {
        match crate::win::acl::untrusted_writer(candidate, user) {
            Ok(None) => {}
            Ok(Some(writer)) => return Some(writer),
            Err(err) => return Some(format!("an unreadable access list ({})", err.message)),
        }
    }
    None
}

/// The Windows PowerShell 5.1 policy that keeps it from running an unsigned
/// profile, when that is the effective one: group policy for the machine,
/// then for the user, then the user's and the machine's own settings, then
/// Windows' default — `Restricted` on a client, `RemoteSigned` on a server.
/// Read only, to say so: TigerSetup never writes a policy.
fn windows_powershell_blocks_profiles(roots: &Roots) -> Option<String> {
    let read = |key: &str, name: &str| -> Option<String> {
        let key = KeyPath::parse(key).ok()?;
        match winreg::read_value(roots, &key, name).ok()?? {
            Data::String(text) | Data::ExpandString(text) => Some(text),
            Data::Dword(value) => Some(value.to_string()),
            _ => None,
        }
    };
    let policy_key = "SOFTWARE\\Policies\\Microsoft\\Windows\\PowerShell";
    let shell_key = "SOFTWARE\\Microsoft\\PowerShell\\1\\ShellIds\\Microsoft.PowerShell";
    let by_policy = |hive: &str| -> Option<String> {
        let key = format!("{hive}\\{policy_key}");
        match read(&key, "EnableScripts").as_deref() {
            Some("1") => read(&key, "ExecutionPolicy"),
            Some(_) => Some("Restricted".into()),
            None => None,
        }
    };
    let effective = by_policy("HKLM")
        .or_else(|| by_policy("HKCU"))
        .or_else(|| read(&format!("HKCU\\{shell_key}"), "ExecutionPolicy"))
        .or_else(|| read(&format!("HKLM\\{shell_key}"), "ExecutionPolicy"))
        .filter(|policy| !policy.eq_ignore_ascii_case("Undefined"))
        .unwrap_or_else(|| {
            let server = read(
                "HKLM\\SYSTEM\\CurrentControlSet\\Control\\ProductOptions",
                "ProductType",
            )
            .is_some_and(|kind| !kind.eq_ignore_ascii_case("WinNT"));
            if server { "RemoteSigned" } else { "Restricted" }.to_string()
        });
    ["Restricted", "AllSigned"]
        .iter()
        .any(|blocking| blocking.eq_ignore_ascii_case(&effective))
        .then_some(effective)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::metadata::{Predicate, ShellCompletion};

    fn metadata(completions: Vec<ShellCompletion>) -> Metadata {
        Metadata {
            package: Some(tigersetup_format::metadata::Package {
                id: "Vendor.Tool".into(),
                ..Default::default()
            }),
            shell_completions: completions,
            ..Default::default()
        }
    }

    fn completion(command: &str, script: &str, equals: Option<&str>) -> ShellCompletion {
        ShellCompletion {
            command: command.into(),
            script: script.into(),
            when: equals.map(|value| Predicate {
                option: "path-mode".into(),
                equals: value.into(),
            }),
        }
    }

    fn files(paths: &[&str]) -> Vec<DesiredFile> {
        paths
            .iter()
            .map(|p| DesiredFile {
                path: p.replace('/', "\\"),
                entry: p.to_string(),
                size: 1,
            })
            .collect()
    }

    fn options(mode: &str) -> Options {
        let mut options = Options::new();
        options.insert(
            "path-mode".into(),
            tigersetup_format::metadata::OptionValue::Choice(mode.into()),
        );
        options
    }

    #[test]
    fn a_completion_follows_the_predicate_of_its_path_entry_and_its_script() {
        // Declared twice, as a command on PATH under two values of a choice
        // option is: one completion when either holds, none otherwise.
        let metadata = metadata(vec![
            completion("tool", "completion/tool.ps1", Some("command")),
            completion("tool", "completion/tool.ps1", Some("tools")),
        ]);
        let installed = files(&["bin/tool.exe", "completion/tool.ps1"]);
        assert_eq!(
            enabled(&metadata, &options("command"), &installed),
            vec![("tool", "completion/tool.ps1")]
        );
        assert_eq!(enabled(&metadata, &options("tools"), &installed).len(), 1);
        assert!(enabled(&metadata, &options("none"), &installed).is_empty());
        // A script the run does not install is not activated.
        assert!(enabled(&metadata, &options("command"), &files(&["bin/tool.exe"])).is_empty());
    }

    #[test]
    fn the_installing_users_two_profiles_get_a_block_each() {
        let root = tempfile::tempdir().unwrap();
        let documents = tempfile::tempdir().unwrap();
        let me = crate::win::interactive::this_process_user_sid().unwrap();
        crate::win::acl::set_dacl(
            root.path(),
            &format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{me})"),
        )
        .unwrap();
        let metadata = metadata(vec![completion("tool", "completion/tool.ps1", None)]);
        let reach = Reach::Profiles {
            user: me.clone(),
            documents: documents.path().to_path_buf(),
        };
        let roots = Roots::relocated("Software\\TigerSetupTests\\completion-none");
        let (blocks, _) = desired(
            &metadata,
            &Options::new(),
            root.path(),
            &files(&["completion/tool.ps1"]),
            &reach,
            &roots,
        )
        .unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks.iter().map(|b| b.profile.clone()).collect::<Vec<_>>(),
            profile::profiles(documents.path())
        );
        assert!(
            blocks
                .iter()
                .all(|b| b.user == me && b.marker.starts_with("tool ("))
        );
        // Nobody to activate for: nothing wanted, and the run says why.
        let (blocks, findings) = desired(
            &metadata,
            &Options::new(),
            root.path(),
            &files(&["completion/tool.ps1"]),
            &Reach::Unreachable {
                code: "completion_no_desktop_user",
                detail: "x".into(),
            },
            &roots,
        )
        .unwrap();
        assert!(blocks.is_empty());
        assert_eq!(findings[0].code, "completion_no_desktop_user");
        // A script another account may change is not loaded.
        crate::win::acl::set_dacl(
            root.path(),
            &format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{me})(A;OICI;0x1301bf;;;AU)"),
        )
        .unwrap();
        let (blocks, findings) = desired(
            &metadata,
            &Options::new(),
            root.path(),
            &files(&["completion/tool.ps1"]),
            &reach,
            &roots,
        )
        .unwrap();
        assert!(blocks.is_empty());
        assert_eq!(findings[0].code, "completion_script_writable_by_others");
    }

    #[test]
    fn windows_powershells_policy_is_read_never_written() {
        let prefix = format!(
            "Software\\TigerSetupTests\\policy-{}-{}",
            std::process::id(),
            crate::report::unique_id()
        );
        let roots = Roots::relocated(&prefix);
        // No setting at all: a client's default runs no profile.
        let product =
            KeyPath::parse("HKLM\\SYSTEM\\CurrentControlSet\\Control\\ProductOptions").unwrap();
        winreg::create_key(&roots, &product).unwrap();
        winreg::write_value(
            &roots,
            &product,
            "ProductType",
            &Data::String("WinNT".into()),
        )
        .unwrap();
        assert_eq!(
            windows_powershell_blocks_profiles(&roots).as_deref(),
            Some("Restricted")
        );
        winreg::write_value(
            &roots,
            &product,
            "ProductType",
            &Data::String("ServerNT".into()),
        )
        .unwrap();
        assert_eq!(windows_powershell_blocks_profiles(&roots), None);
        // The user's own setting, then group policy over it.
        let user = KeyPath::parse(
            "HKCU\\SOFTWARE\\Microsoft\\PowerShell\\1\\ShellIds\\Microsoft.PowerShell",
        )
        .unwrap();
        winreg::create_key(&roots, &user).unwrap();
        winreg::write_value(
            &roots,
            &user,
            "ExecutionPolicy",
            &Data::String("AllSigned".into()),
        )
        .unwrap();
        assert_eq!(
            windows_powershell_blocks_profiles(&roots).as_deref(),
            Some("AllSigned")
        );
        winreg::write_value(
            &roots,
            &user,
            "ExecutionPolicy",
            &Data::String("RemoteSigned".into()),
        )
        .unwrap();
        assert_eq!(windows_powershell_blocks_profiles(&roots), None);
        let policy =
            KeyPath::parse("HKLM\\SOFTWARE\\Policies\\Microsoft\\Windows\\PowerShell").unwrap();
        winreg::create_key(&roots, &policy).unwrap();
        winreg::write_value(&roots, &policy, "EnableScripts", &Data::Dword(0)).unwrap();
        assert_eq!(
            windows_powershell_blocks_profiles(&roots).as_deref(),
            Some("Restricted")
        );
        let _ = std::process::Command::new("reg.exe")
            .args(["delete", &format!("HKCU\\{prefix}"), "/f"])
            .output();
    }
}
