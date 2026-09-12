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
//! The evidence that the removal worked is the registration key
//! disappearing, not the exit code alone. Inno Setup's uninstaller
//! re-launches itself from `%TEMP%` and returns immediately, so the key
//! often outlives the process this engine waited for by a moment; the check
//! polls for it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tigersetup_format::metadata::Legacy;

use crate::report::{LegacyInfo, Reporter};
use crate::scope::Locations;
use crate::win::process;
use crate::win::registry::{self as winreg, KeyPath, Roots};
use crate::{Error, Result};

/// How long the registration key may take to disappear after the
/// uninstaller's process has exited.
const KEY_REMOVAL_TIMEOUT: Duration = Duration::from_secs(10);
const KEY_REMOVAL_POLL: Duration = Duration::from_millis(200);

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
    let program_path = PathBuf::from(&program);
    let exit_code = process::run_hidden(&program_path, &arguments).map_err(|err| {
        Error::new(
            "legacy_uninstall_failed",
            format!("{program} could not be run: {err}"),
        )
    })?;
    if exit_code != 0 {
        return Err(Error::new(
            "legacy_uninstall_failed",
            format!("{program} exited with {exit_code}"),
        ));
    }
    if !key_disappears(roots, &key)? {
        return Err(Error::new(
            "legacy_uninstall_failed",
            format!("{program} reported success but {key} is still registered"),
        ));
    }
    reporter.event("legacy_uninstalled", key.to_string());
    Ok(Some(LegacyInfo {
        key: key.to_string(),
        uninstalled: true,
    }))
}

/// Waits for the registration key to go away, because a legacy uninstaller
/// commonly finishes a moment after the process this engine waited for has
/// exited.
fn key_disappears(roots: &Roots, key: &KeyPath) -> Result<bool> {
    let deadline = Instant::now() + KEY_REMOVAL_TIMEOUT;
    loop {
        if !winreg::key_exists(roots, key)? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(KEY_REMOVAL_POLL);
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
