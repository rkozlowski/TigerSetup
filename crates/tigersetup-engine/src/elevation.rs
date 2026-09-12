//! Per-machine scope needs an administrator, so a run that mutates machine
//! scope either already has one or asks for one
//! (`TigerSetup-Design.md` §14.1).
//!
//! Every client goes through this module and none implements any of it:
//! the command line and the wizard ask the same questions, express a run
//! the same way, and read the same result.
//!
//! **Asking.** [`is_elevated`] answers whether this process already has the
//! rights, and [`requirement`] answers whether a given package and run would
//! need them on this machine — enough for a wizard to put the shield on the
//! button before the user commits to anything, and for the command line to
//! decide whether to relaunch. The decision is never "the scope says
//! machine": it is whether this process can write where that scope keeps
//! its state, so
//!
//! - the process token is elevated → nothing to do;
//! - this process can already write both roots the run needs, the state
//!   directory and the install root → nothing to do. Both matter:
//!   `%ProgramData%` lets a standard user create files, so a machine-scope
//!   run that looked only at the state directory would start and then fail
//!   in `%ProgramFiles%` halfway through. In production a standard user
//!   fails at least one of the two, so this is only true where the folder
//!   seams redirect the roots into a place the caller owns — which is how
//!   the process-level tests run machine-scope installs unelevated;
//! - otherwise the run has to be relaunched elevated.
//!
//! **Relaunching.** [`arguments_for`] turns an [`Intent`] and the
//! [`RunOptions`] the client assembled — scope, install root, the options
//! the user chose, the language, the log, the dependency policy, any
//! injected faults — into the command line that reproduces exactly that run,
//! so a client never has to compose one and a wizard's choices survive the
//! elevation boundary. [`relaunch`] then runs this same executable through
//! `ShellExecuteExW` with the `runas` verb, passing those arguments and a
//! hidden `--elevated-result <path>`. The elevated child writes its outcome
//! document there as well as printing it, because the parent cannot read
//! the child's standard output across the elevation boundary. The parent
//! reports that document and the child's exit code as its own, so a caller
//! cannot tell which side of the prompt produced them.
//!
//! The child's command line also decides how it is started: a `--quiet`
//! child has nothing to show and is started hidden, and any other child is
//! about to show the wizard from its next page on and is started shown. The
//! distinction is the launch's, not the child's — a process's first window
//! follows the show state it was started with, so a wizard started hidden
//! would wait, invisible, for a click nobody can give, and its parent would
//! wait for it.
//!
//! **Refusal.** A prompt the user declines is `elevation_refused`; a process
//! that cannot raise a prompt at all is `elevation_required`. Both exit 4
//! (`TigerSetup-Design.md` §6.2), and [`refused`] is distinguishable from
//! [`unavailable`] by its code and by [`declined`], so a client can say "you
//! declined the prompt" rather than "something failed". Exit 5 stays what it
//! is: a run that was cancelled, not one that was never allowed to start.

use std::path::{Path, PathBuf};

use tigersetup_format::identity::Scope;

use crate::report::exit;
use crate::win::process::{self, LaunchError};
use crate::{Error, Package, Result, Roots, RunOptions};

/// The argument that tells an elevated child where to leave its outcome
/// document. Hidden from `--help`: it is a detail of one process handing a
/// result to another, not something a person passes.
pub const RESULT_ARGUMENT: &str = "--elevated-result";

/// The mutating intents that can need an administrator. `verify` and
/// `inspect` never do: they only read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Install,
    Uninstall,
    Repair,
}

impl Intent {
    pub fn as_str(self) -> &'static str {
        match self {
            Intent::Install => "install",
            Intent::Uninstall => "uninstall",
            Intent::Repair => "repair",
        }
    }
}

/// What this machine requires of a run before it can change anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// Nothing: user scope, or this process already has the rights.
    Satisfied,
    /// The run has to be relaunched elevated. A client that shows a button
    /// puts the shield on it.
    Elevation,
}

impl Requirement {
    pub fn needs_elevation(self) -> bool {
        self == Requirement::Elevation
    }
}

/// Whether this process runs with an elevated token.
pub fn is_elevated() -> bool {
    process::is_elevated()
}

/// What `package` would require of a run with `options` on this machine.
///
/// Fails only where the run itself could not be resolved at all — an
/// unsupported scope, a known folder this machine cannot name — so a client
/// can call it as soon as it knows the scope, before anything is prepared.
pub fn requirement(package: &Package, options: &RunOptions) -> Result<Requirement> {
    let roots = crate::resolve_roots(package, options)?;
    Ok(match required(options.scope, &roots) {
        true => Requirement::Elevation,
        false => Requirement::Satisfied,
    })
}

/// Whether a mutating run in this scope has to be relaunched elevated.
///
/// Neither root need exist yet; the nearest existing ancestor of each is
/// probed instead.
pub fn required(scope: Scope, roots: &Roots) -> bool {
    scope == Scope::Machine
        && !is_elevated()
        && !(can_write(&roots.state_dir) && can_write(&roots.install_root))
}

/// Whether this process can create a directory where it resolved to: the
/// directory itself when it exists, otherwise the nearest ancestor that
/// does.
fn can_write(directory: &Path) -> bool {
    let mut candidate = Some(directory);
    while let Some(path) = candidate {
        if path.is_dir() {
            return probe(path);
        }
        candidate = path.parent();
    }
    false
}

/// Creates and removes a uniquely named file in `directory`; a standard
/// user cannot do that in a directory it only has read access to.
fn probe(directory: &Path) -> bool {
    let probe = directory.join(format!(
        ".tigersetup-write-probe-{}-{}",
        std::process::id(),
        crate::report::unique_id()
    ));
    match std::fs::File::create(&probe) {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// The command line that reproduces this run: the intent, then every choice
/// in `options` that a fresh process would otherwise not know about.
///
/// A client that wants the elevated run to proceed without a second
/// interface sets `quiet` on the options it passes; one that wants the
/// elevated process to show the interface leaves it clear. Nothing about
/// how the client displays its own result belongs here.
pub fn arguments_for(intent: Intent, options: &RunOptions) -> Vec<String> {
    let mut out = vec![intent.as_str().to_string()];
    if options.quiet {
        out.push("--quiet".into());
    }
    out.push("--scope".into());
    out.push(options.scope.as_str().to_string());
    if let Some(root) = &options.install_root {
        out.push("--install-root".into());
        out.push(root.display().to_string());
    }
    for (name, value) in &options.options {
        out.push("--option".into());
        out.push(name.clone());
        out.push(if *value { "on".into() } else { "off".into() });
    }
    if !options.install_dependencies {
        out.push("--no-dependency-install".into());
    }
    if !options.lang.is_empty() {
        out.push("--lang".into());
        out.push(options.lang.clone());
    }
    if let Some(log) = &options.log_path {
        out.push("--log".into());
        out.push(log.display().to_string());
    }
    for fault in &options.faults {
        out.push("--fault".into());
        out.push(fault.describe());
    }
    out
}

/// What a relaunched child left behind: an elevated one, or one started
/// plainly for the scope the wizard chose among two installations.
pub struct Elevated {
    /// The child's exit code.
    pub exit_code: i32,
    /// The outcome document it wrote, when it wrote one.
    pub document: Option<String>,
}

/// Runs `intent` with `options` again, elevated, and returns what the
/// elevated run reported. The whole path a client needs: express the run,
/// raise the prompt, collect the result.
pub fn relaunch(executable: &Path, intent: Intent, options: &RunOptions) -> Result<Elevated> {
    relaunch_elevated(executable, &arguments_for(intent, options))
}

/// Runs this executable again, elevated, with `arguments` plus
/// `--elevated-result <path>`, and returns what the child reported. The
/// child is started shown unless `arguments` make it quiet
/// ([`show_for`]).
///
/// The result file lives beside the other temporaries this installation
/// uses and is removed before returning, so nothing is left on the machine
/// whether the child succeeded, failed or never started.
pub fn relaunch_elevated(executable: &Path, arguments: &[String]) -> Result<Elevated> {
    relaunch_elevated_observed(executable, arguments, || {})
}

/// [`relaunch_elevated`], calling `started` once the prompt has been
/// answered and the child is running, before the wait for it — so that a
/// client showing a window of its own can step aside for the child's. A
/// refused or failed prompt never reaches `started`.
pub fn relaunch_elevated_observed(
    executable: &Path,
    arguments: &[String],
    started: impl FnOnce(),
) -> Result<Elevated> {
    let show = show_for(arguments);
    relaunch_with(executable, arguments, |program, full| {
        let child = process::start_elevated(program, full, show)?;
        started();
        child.wait()
    })
}

/// How a child started with `arguments` is shown: hidden when it runs
/// quietly, shown when it is going to put up the wizard.
pub fn show_for(arguments: &[String]) -> process::Show {
    match arguments.iter().any(|argument| argument == "--quiet") {
        true => process::Show::Hidden,
        false => process::Show::Shown,
    }
}

/// Runs this executable again with this process's own token — no prompt —
/// and returns what the child reported through the same result file. The
/// wizard uses it when the person chose, among two installations, the one
/// this process may already change: the child is then the run, exactly as
/// an elevated child is.
pub fn relaunch_plain(executable: &Path, arguments: &[String]) -> Result<Elevated> {
    relaunch_with(executable, arguments, process::run_plain)
}

fn relaunch_with(
    executable: &Path,
    arguments: &[String],
    launch: impl FnOnce(&Path, &[String]) -> std::result::Result<i32, LaunchError>,
) -> Result<Elevated> {
    let directory = crate::temp_directory();
    std::fs::create_dir_all(&directory).map_err(|err| {
        Error::new(
            "io_error",
            format!("cannot create {}: {err}", directory.display()),
        )
    })?;
    let result_path = directory.join(format!("elevated-{}.json", crate::report::unique_id()));

    let mut full: Vec<String> = arguments_to_forward(arguments.iter().cloned());
    full.push(RESULT_ARGUMENT.to_string());
    full.push(result_path.display().to_string());

    let launched = launch(executable, &full);
    let document = std::fs::read_to_string(&result_path).ok();
    let _ = std::fs::remove_file(&result_path);
    match launched {
        Ok(exit_code) => Ok(Elevated {
            exit_code,
            document,
        }),
        Err(LaunchError::Refused) => Err(refused()),
        Err(LaunchError::Failed(message)) => Err(unavailable(message)),
    }
}

/// The user answered No to the prompt. Exit 4, not 5: the run never
/// started, so there is nothing that was cancelled.
pub fn refused() -> Error {
    Error::new(
        "elevation_refused",
        "this installation needs administrator rights and the prompt was refused",
    )
}

/// No prompt could be raised: User Account Control is off for this account,
/// or the account may not elevate at all.
pub fn unavailable(reason: impl Into<String>) -> Error {
    Error::new(
        "elevation_required",
        format!(
            "this installation needs administrator rights: {}",
            reason.into()
        ),
    )
}

/// Whether an error is a prompt the user declined, rather than any other
/// way of not having the rights. A client reports the two differently.
pub fn declined(err: &Error) -> bool {
    err.code == "elevation_refused"
}

/// Both elevation failures exit 4.
pub fn exit_code() -> i32 {
    exit::ELEVATION
}

/// Writes the document an elevated child hands back to its parent, next to
/// printing it. A failure to write is not a failure of the run: the parent
/// falls back to reporting the child's exit code alone.
pub fn write_result(path: &Path, document: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, document)
}

/// Arguments without an `--elevated-result` a caller already passed, so
/// that relaunching cannot nest one result file inside another.
pub fn arguments_to_forward(arguments: impl Iterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut skip = false;
    for argument in arguments {
        if skip {
            skip = false;
            continue;
        }
        if argument == RESULT_ARGUMENT {
            skip = true;
            continue;
        }
        out.push(argument);
    }
    out
}

/// The path an `--elevated-result` argument names, for a client that parses
/// its own command line.
pub fn result_path(arguments: &[String]) -> Option<PathBuf> {
    arguments
        .iter()
        .position(|a| a == RESULT_ARGUMENT)
        .and_then(|index| arguments.get(index + 1))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::txn::FaultSpec;
    use std::collections::BTreeMap;

    #[test]
    fn user_scope_never_elevates() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!required(Scope::User, &roots_under(dir.path())));
    }

    /// A pair of roots under `parent`, as a run would resolve them.
    fn roots_under(parent: &Path) -> Roots {
        let state_dir = parent.join("ProgramData").join("TigerSetup").join("P");
        Roots {
            state_db: state_dir.join("state.db"),
            uninstaller: state_dir.join("uninstall.exe"),
            state_dir,
            install_root: parent.join("ProgramFiles").join("P"),
        }
    }

    /// The seams the tests use redirect machine scope into a tree the test
    /// process owns, so an unelevated machine-scope run proceeds there; a
    /// root this process may only read needs an administrator, whatever the
    /// scope says.
    #[test]
    fn machine_scope_elevates_unless_this_process_can_write_both_roots() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!required(Scope::Machine, &roots_under(dir.path())));

        // Only the local system may write here, which is how
        // `%ProgramData%\TigerSetup\<product>` behaves once it carries the
        // access control list `scope.rs` gives it, and how `%ProgramFiles%`
        // behaves for a standard user all along.
        let closed = dir.path().join("closed");
        std::fs::create_dir(&closed).unwrap();
        crate::win::acl::set_dacl(&closed, "D:(A;OICI;FA;;;SY)").unwrap();

        // Either root being out of reach is enough: a run that could write
        // its state but not its files would fail halfway through.
        let mut state_closed = roots_under(dir.path());
        state_closed.state_dir = closed.join("TigerSetup").join("P");
        assert!(required(Scope::Machine, &state_closed));

        let mut files_closed = roots_under(dir.path());
        files_closed.install_root = closed.join("P");
        assert!(required(Scope::Machine, &files_closed));
        assert!(!required(Scope::User, &files_closed));

        // Give it back so the temporary directory can be removed.
        crate::win::acl::set_dacl(&closed, "D:(A;OICI;FA;;;WD)").unwrap();
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe(dir.path()));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(!probe(&dir.path().join("missing")));
    }

    /// Everything the client chose survives the boundary, and nothing else
    /// crosses it.
    #[test]
    fn the_relaunched_command_line_carries_the_runs_choices() {
        let mut options = RunOptions {
            scope: Scope::Machine,
            install_root: Some(PathBuf::from("C:\\Program Files\\TestApp")),
            lang: "pl-PL".into(),
            quiet: false,
            ..RunOptions::default()
        };
        options.options = BTreeMap::from([
            ("desktop-shortcut".to_string(), true),
            ("path".to_string(), false),
        ]);
        assert_eq!(
            arguments_for(Intent::Install, &options),
            [
                "install",
                "--scope",
                "machine",
                "--install-root",
                "C:\\Program Files\\TestApp",
                "--option",
                "desktop-shortcut",
                "on",
                "--option",
                "path",
                "off",
                "--lang",
                "pl-PL",
            ]
        );

        let quiet = RunOptions {
            scope: Scope::Machine,
            quiet: true,
            install_dependencies: false,
            log_path: Some(PathBuf::from("C:\\Temp\\run.log")),
            faults: vec![FaultSpec::parse("after_rename@3:crash:skip_flush").unwrap()],
            ..RunOptions::default()
        };
        assert_eq!(
            arguments_for(Intent::Uninstall, &quiet),
            [
                "uninstall",
                "--quiet",
                "--scope",
                "machine",
                "--no-dependency-install",
                "--lang",
                "en-US",
                "--log",
                "C:\\Temp\\run.log",
                "--fault",
                "after_rename@3:crash:skip_flush",
            ]
        );
        assert_eq!(arguments_for(Intent::Repair, &quiet)[0], "repair");
    }

    /// The wizard relaunches itself without `--quiet` and the command line
    /// relaunches a quiet run with it; only the first has a window to show.
    /// A shown wizard is the whole point: a child started hidden creates its
    /// window invisible and waits there for a click, and its parent waits
    /// for it.
    #[test]
    fn a_wizard_child_is_started_shown_and_a_quiet_one_hidden() {
        let wizard = RunOptions {
            scope: Scope::Machine,
            quiet: false,
            ..RunOptions::default()
        };
        assert_eq!(
            show_for(&arguments_for(Intent::Install, &wizard)),
            process::Show::Shown
        );
        let quiet = RunOptions {
            quiet: true,
            ..wizard.clone()
        };
        assert_eq!(
            show_for(&arguments_for(Intent::Install, &quiet)),
            process::Show::Hidden
        );
        // The wizard's own relaunch: its arguments plus the scope it chose.
        let chosen: Vec<String> = ["install", "--scope", "machine", "--lang", "en-US"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(show_for(&chosen), process::Show::Shown);
    }

    #[test]
    fn a_result_argument_is_never_forwarded_twice() {
        let given: Vec<String> = ["install", "--quiet", "--scope", "machine"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(arguments_to_forward(given.iter().cloned()), given);

        let mut nested = given.clone();
        nested.push(RESULT_ARGUMENT.into());
        nested.push("C:\\Temp\\a.json".into());
        assert_eq!(arguments_to_forward(nested.iter().cloned()), given);
        assert_eq!(
            result_path(&nested),
            Some(PathBuf::from("C:\\Temp\\a.json"))
        );
        assert_eq!(result_path(&given), None);
    }

    #[test]
    fn a_declined_prompt_is_distinguishable_and_both_failures_exit_four() {
        assert_eq!(refused().code, "elevation_refused");
        assert_eq!(unavailable("no prompt").code, "elevation_required");
        assert!(declined(&refused()));
        assert!(!declined(&unavailable("no prompt")));
        assert!(!declined(&Error::new("cancelled", "")));
        assert_eq!(refused().exit_code(), exit_code());
        assert_eq!(unavailable("no prompt").exit_code(), exit_code());
        // Exit 4 is "needs an administrator", exit 5 is "a run was
        // cancelled"; declining the prompt is the first.
        assert_ne!(exit_code(), Error::new("cancelled", "").exit_code());
    }

    #[test]
    fn a_child_writes_its_document_where_the_parent_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("elevated.json");
        write_result(&path, "{\"outcome\":\"installed\"}").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"outcome\":\"installed\"}"
        );
    }
}
