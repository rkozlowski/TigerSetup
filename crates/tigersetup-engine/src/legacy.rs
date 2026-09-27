//! Migration from the installer technology a product used before
//! (`TigerSetup-Design.md` §5.12): uninstall-first, once, through the old
//! installer's own uninstaller.
//!
//! TigerSetup never takes ownership of files another installer put on the
//! machine and never rewrites its bookkeeping. It asks that installer to
//! remove its own installation, checks that it did, and only then installs
//! the product. The phase therefore runs after the dependency phase and
//! before the product transaction, outside it: a migration that fails
//! leaves the machine holding exactly what it held before, and the run
//! stops with `legacy_uninstall_failed` before a single product resource
//! is touched.
//!
//! The uninstall is over when **every process the uninstaller started has
//! ended**, not when the program started here exits and not when its
//! registration key disappears. Inno Setup's `unins000.exe` copies itself
//! to `%TEMP%` and runs the copy as a second phase that does the work: the
//! copy undoes the install in reverse, so the key — written last — goes
//! first; then it tells `unins000.exe` to exit, waits half a second,
//! deletes `unins000.exe` and only then removes the directories it could
//! not remove while that was in them, the install root among them. Either
//! earlier signal releases the install while the old one is still being
//! taken apart, and an install root the old uninstaller had not removed yet
//! is one TigerSetup finds already there and so never owns. The uninstaller
//! therefore runs in a job object (`process::start_tree`) and the migration
//! waits, bounded, until the job is empty. Only then are its results read:
//! its exit code, and the key, which must be gone.
//!
//! That the old uninstaller has finished does not make what it left behind
//! TigerSetup's. A directory it kept — a file it could not delete, one a
//! user added — stays foreign and is recorded as found; the run says so
//! (`legacy_location_remains`) rather than waiting for it to go.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tigersetup_format::metadata::Legacy;

use crate::report::{LegacyInfo, Reporter};
use crate::scope::Locations;
use crate::win::process::{self, TreeState};
use crate::win::registry::{self as winreg, KeyPath, Roots};
use crate::{Error, Result};

/// How long a legacy uninstaller, with everything it started, may take.
/// Generous: an uninstall is normally seconds, and a run that gives up
/// leaves the uninstaller running rather than ending it halfway.
pub const UNINSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// The switches that make each supported legacy uninstaller unattended,
/// used only when it published no `QuietUninstallString` of its own.
fn quiet_switches(installer_type: &str) -> &'static [&'static str] {
    match installer_type {
        "inno" => &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
        _ => &[],
    }
}

/// Where a legacy registration may be: the scope's own Add/Remove Programs
/// root, plus the 32-bit view of it for machine scope, because a 32-bit
/// legacy installer registered under `WOW6432Node`.
fn candidate_keys(locations: &Locations, key_name: &str) -> Vec<KeyPath> {
    let mut keys = vec![locations.registration_key(key_name)];
    if locations.scope == tigersetup_format::identity::Scope::Machine {
        keys.push(KeyPath::new(
            locations.hive,
            format!(
                "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{key_name}"
            ),
        ));
    }
    keys
}

/// The uninstall command a registration publishes: its quiet form, or its
/// interactive form with the type's quiet switches appended.
fn command(
    roots: &Roots,
    key: &KeyPath,
    installer_type: &str,
) -> Result<Option<(String, Vec<String>)>> {
    let read = |name: &str| -> Result<Option<String>> {
        Ok(winreg::read_value(roots, key, name)?
            .and_then(|data| data.as_text().map(str::to_string))
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()))
    };
    if let Some(quiet) = read("QuietUninstallString")? {
        let (program, mut arguments) = split_command(&quiet);
        // A published quiet string is already unattended; the switches are
        // added only where they are missing entirely.
        if arguments.is_empty() {
            arguments = quiet_switches(installer_type)
                .iter()
                .map(|s| s.to_string())
                .collect();
        }
        return Ok(Some((program, arguments)));
    }
    let Some(plain) = read("UninstallString")? else {
        return Ok(None);
    };
    let (program, mut arguments) = split_command(&plain);
    arguments.extend(quiet_switches(installer_type).iter().map(|s| s.to_string()));
    Ok(Some((program, arguments)))
}

/// Splits a published uninstall string into its program and arguments. A
/// quoted program may contain spaces; an unquoted one ends at the first
/// space that is not inside the path of an existing file.
fn split_command(text: &str) -> (String, Vec<String>) {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), split_arguments(&rest[end + 1..]));
        }
        return (rest.to_string(), Vec::new());
    }
    // `C:\Program Files\App\unins000.exe /X` has no quotes in some old
    // registrations: take the longest prefix that is a file on disk.
    let bytes: Vec<usize> = text
        .char_indices()
        .filter(|(_, c)| *c == ' ')
        .map(|(i, _)| i)
        .collect();
    for end in bytes.iter().rev() {
        if PathBuf::from(&text[..*end]).is_file() {
            return (text[..*end].to_string(), split_arguments(&text[*end..]));
        }
    }
    match text.split_once(' ') {
        Some((program, rest)) => (program.to_string(), split_arguments(rest)),
        None => (text.to_string(), Vec::new()),
    }
}

/// Splits an argument tail on whitespace, keeping quoted runs together.
fn split_arguments(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Runs the migration for a package that declares one. Returns what
/// happened, or `None` when the package declares no legacy installation or
/// this machine holds none.
pub fn migrate(
    legacy: Option<&Legacy>,
    locations: &Locations,
    roots: &Roots,
    reporter: &mut Reporter<'_>,
) -> Result<Option<LegacyInfo>> {
    migrate_within(legacy, locations, roots, reporter, UNINSTALL_TIMEOUT)
}

/// [`migrate`], with the uninstaller given `timeout` to finish.
pub fn migrate_within(
    legacy: Option<&Legacy>,
    locations: &Locations,
    roots: &Roots,
    reporter: &mut Reporter<'_>,
    timeout: Duration,
) -> Result<Option<LegacyInfo>> {
    let Some(legacy) = legacy else {
        return Ok(None);
    };
    let Some(key) = candidate_keys(locations, &legacy.registration_key)
        .into_iter()
        .find(|key| winreg::key_exists(roots, key).unwrap_or(false))
    else {
        return Ok(None);
    };
    reporter.event("legacy_found", key.to_string());

    let Some((program, arguments)) = command(roots, &key, &legacy.installer_type)? else {
        return Err(Error::new(
            "legacy_uninstall_failed",
            format!("{key} publishes no uninstall command"),
        ));
    };
    // Read before the uninstall removes it; used only to say what stayed,
    // so a value that cannot be read says nothing rather than stopping the run.
    let location = install_location(roots, &key);
    let mut tree = process::start_tree(Path::new(&program), &arguments).map_err(|err| {
        Error::new(
            "legacy_uninstall_failed",
            format!("{program} could not be run: {err}"),
        )
    })?;
    let state = tree.wait(timeout).map_err(|err| {
        Error::new(
            "legacy_uninstall_failed",
            format!("{program} could not be waited for: {err}"),
        )
    })?;
    let still_registered = winreg::key_exists(roots, &key)?;
    let (processes, root_exited, ended) =
        verdict(&program, &key, state, still_registered, timeout)?;
    reporter.event(
        "legacy_uninstalled",
        format!(
            "{key} processes={processes} root_exit_ms={} ended_ms={}",
            root_exited.as_millis(),
            ended.as_millis()
        ),
    );
    let location_remains = location
        .filter(|path| path.is_dir())
        .map(|path| path.display().to_string());
    if let Some(location) = &location_remains {
        reporter.event("legacy_location_remains", location.clone());
    }
    Ok(Some(LegacyInfo {
        key: key.to_string(),
        uninstalled: true,
        processes,
        location_remains,
    }))
}

/// The directory the legacy registration says the product is installed in,
/// when it says one and can be read.
fn install_location(roots: &Roots, key: &KeyPath) -> Option<PathBuf> {
    winreg::read_value(roots, key, "InstallLocation")
        .ok()
        .flatten()
        .and_then(|data| data.as_text().map(str::to_string))
        .map(|text| text.trim().trim_matches('"').to_string())
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
}

/// Judges a finished wait for the legacy uninstaller: the uninstall is
/// done only when every process of it has ended, it succeeded, and its
/// registration is gone. A key that disappeared while processes were still
/// running does not count. Returns how many processes took part, when the
/// program the registration names exited, and when the last process did.
fn verdict(
    program: &str,
    key: &KeyPath,
    state: TreeState,
    still_registered: bool,
    timeout: Duration,
) -> Result<(u32, Duration, Duration)> {
    match state {
        TreeState::Running {
            root_exit_code,
            running,
        } => {
            let root = match root_exit_code {
                Some(code) => format!("{program} exited with {code}"),
                None => format!("{program} is still running"),
            };
            let running = running
                .iter()
                .map(|member| match &member.image {
                    Some(image) => format!("{} (pid {})", image.display(), member.pid),
                    None => format!("pid {}", member.pid),
                })
                .collect::<Vec<_>>()
                .join(", ");
            Err(Error::new(
                "legacy_uninstall_failed",
                format!(
                    "the legacy uninstaller did not finish within {} s ({root}; still running: {running}); it was left running",
                    timeout.as_secs()
                ),
            ))
        }
        TreeState::Ended { exit_code, .. } if exit_code != 0 => Err(Error::new(
            "legacy_uninstall_failed",
            format!("{program} exited with {exit_code}"),
        )),
        TreeState::Ended { .. } if still_registered => Err(Error::new(
            "legacy_uninstall_failed",
            format!("{program} finished but {key} is still registered"),
        )),
        TreeState::Ended {
            processes,
            root_exited,
            ended,
            ..
        } => Ok((processes, root_exited, ended)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tigersetup_format::identity::Scope;

    #[test]
    fn a_quoted_program_keeps_the_spaces_in_its_path() {
        assert_eq!(
            split_command("\"C:\\Program Files\\App\\unins000.exe\" /VERYSILENT"),
            (
                "C:\\Program Files\\App\\unins000.exe".to_string(),
                vec!["/VERYSILENT".to_string()]
            )
        );
        assert_eq!(
            split_command("\"C:\\App\\unins000.exe\""),
            ("C:\\App\\unins000.exe".to_string(), Vec::new())
        );
        assert_eq!(
            split_command("C:\\App\\unins000.exe /X \"a b\""),
            (
                "C:\\App\\unins000.exe".to_string(),
                vec!["/X".to_string(), "a b".to_string()]
            )
        );
        assert_eq!(
            split_command("C:\\App\\unins000.exe"),
            ("C:\\App\\unins000.exe".to_string(), Vec::new())
        );
    }

    /// An unquoted program whose path contains a space is recognised by
    /// finding the file on disk.
    #[test]
    fn an_unquoted_program_with_a_space_is_found_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("Old App").join("unins000.exe");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(&program, b"exe").unwrap();
        let text = format!("{} /VERYSILENT", program.display());
        assert_eq!(
            split_command(&text),
            (
                program.display().to_string(),
                vec!["/VERYSILENT".to_string()]
            )
        );
    }

    #[test]
    fn inno_gets_its_unattended_switches_when_the_registration_has_no_quiet_string() {
        assert_eq!(
            quiet_switches("inno"),
            ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]
        );
        assert!(quiet_switches("something-else").is_empty());
    }

    #[test]
    fn machine_scope_also_looks_in_the_thirty_two_bit_view() {
        let user = candidate_keys(&crate::scope::locations(Scope::User), "Old_is1");
        assert_eq!(user.len(), 1);
        assert_eq!(
            user[0].to_string(),
            "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Old_is1"
        );
        let machine = candidate_keys(&crate::scope::locations(Scope::Machine), "Old_is1");
        assert_eq!(machine.len(), 2);
        assert_eq!(
            machine[1].to_string(),
            "HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Old_is1"
        );
    }

    fn inno_key() -> KeyPath {
        candidate_keys(&crate::scope::locations(Scope::User), "Old_is1").remove(0)
    }

    fn ended(exit_code: i32) -> TreeState {
        TreeState::Ended {
            exit_code,
            processes: 2,
            root_exited: Duration::from_millis(300),
            ended: Duration::from_millis(1_200),
        }
    }

    /// The race Inno Setup's hand-off makes: `unins000.exe` has exited 0
    /// and the key is already gone, but its second phase is still removing
    /// the install root. That is not a finished uninstall.
    #[test]
    fn a_key_gone_while_the_uninstaller_still_runs_is_not_a_finished_uninstall() {
        let state = TreeState::Running {
            root_exit_code: Some(0),
            running: vec![process::Member {
                pid: 4242,
                image: Some(PathBuf::from(
                    "C:\\Users\\u\\AppData\\Local\\Temp\\is-X-uninstall.tmp\\_unins.tmp",
                )),
            }],
        };
        let err = verdict(
            "C:\\App\\unins000.exe",
            &inno_key(),
            state,
            false,
            Duration::from_secs(600),
        )
        .unwrap_err();
        assert_eq!(err.code, "legacy_uninstall_failed");
        assert!(err.message.contains("within 600 s"), "{}", err.message);
        assert!(err.message.contains("exited with 0"), "{}", err.message);
        assert!(
            err.message.contains("_unins.tmp (pid 4242)"),
            "the diagnostic names what is still running: {}",
            err.message
        );
        assert!(err.message.contains("left running"), "{}", err.message);
    }

    #[test]
    fn a_started_program_that_never_exited_is_named_as_still_running() {
        let state = TreeState::Running {
            root_exit_code: None,
            running: vec![process::Member {
                pid: 7,
                image: None,
            }],
        };
        let err = verdict(
            "unins000.exe",
            &inno_key(),
            state,
            true,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(
            err.message.contains("unins000.exe is still running"),
            "{}",
            err.message
        );
        assert!(err.message.contains("pid 7"), "{}", err.message);
    }

    #[test]
    fn a_finished_tree_with_the_key_gone_is_a_finished_uninstall() {
        assert_eq!(
            verdict(
                "unins000.exe",
                &inno_key(),
                ended(0),
                false,
                UNINSTALL_TIMEOUT
            )
            .unwrap(),
            (2, Duration::from_millis(300), Duration::from_millis(1_200))
        );
    }

    #[test]
    fn a_finished_tree_that_failed_or_left_its_key_stops_the_migration() {
        let err = verdict(
            "unins000.exe",
            &inno_key(),
            ended(5),
            false,
            UNINSTALL_TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(err.code, "legacy_uninstall_failed");
        assert!(err.message.contains("exited with 5"), "{}", err.message);
        let err = verdict(
            "unins000.exe",
            &inno_key(),
            ended(0),
            true,
            UNINSTALL_TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(err.code, "legacy_uninstall_failed");
        assert!(err.message.contains("still registered"), "{}", err.message);
    }

    #[test]
    fn a_package_without_a_legacy_section_migrates_nothing() {
        let mut sink = crate::report::NullSink;
        let mut reporter = Reporter::new(&mut sink);
        assert_eq!(
            migrate(
                None,
                &crate::scope::locations(Scope::User),
                &Roots::real(),
                &mut reporter
            )
            .unwrap(),
            None
        );
    }
}
