//! `tiger-setup shell`: the command prompt the Start Menu's **TigerSetup
//! Shell** opens.
//!
//! The shell is the Windows command interpreter (`%ComSpec%`) with this
//! `tiger-setup.exe`'s directory first on its `PATH`, so `tiger-setup` in that
//! window is the installation the shortcut belongs to — whether or not the
//! installer added it to the persistent `PATH`, and whatever other
//! `tiger-setup.exe` the persistent `PATH` names. Only the child's environment
//! changes; the user's and the machine's `PATH` are never written. The shell
//! opens on the brief landing help (`tiger-setup --help-brief`) and stays
//! open for the user.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The title the shell's console window carries.
pub const TITLE: &str = "TigerSetup Shell";

/// Where TigerSetup's own installer puts the PDF help, relative to
/// `tiger-setup.exe` (`packages/tigersetup/TigerSetup.toml`).
pub const HELP_PDF: &str = "help\\TigerSetup-Help.pdf";

/// How the landing help is written: coloured on a console that shows colour,
/// plain everywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Colour,
}

impl Style {
    /// `text` in the SGR attributes `sgr`, reset afterwards; unchanged when
    /// plain.
    fn paint(self, sgr: &str, text: &str) -> String {
        match self {
            Style::Plain => text.to_string(),
            Style::Colour => format!("\x1b[{sgr}m{text}\x1b[0m"),
        }
    }
}

/// The brief landing help: what TigerSetup is, the one command to start
/// with, and where the rest is. `in_shell` adds TigerSetup Shell's line about
/// `PATH`; `installed_help` names the Start Menu help, which exists only when
/// this `tiger-setup.exe` was installed with it.
pub fn brief_help(
    install_dir: &Path,
    in_shell: bool,
    installed_help: bool,
    style: Style,
) -> String {
    let mut text = format!(
        "{} \u{2014} {}\n\n",
        style.paint("1;96", concat!("TigerSetup ", env!("CARGO_PKG_VERSION"))),
        install_dir.display()
    );
    if in_shell {
        text.push_str(&format!(
            "{} Type exit to close it.\n\n",
            style.paint("92", "tiger-setup is on PATH in this window.")
        ));
    }
    text.push_str(&format!(
        "Build Windows installers from TigerSetup.toml.\n\n\
         Getting started:\n  {}\n\n\
         More:\n  tiger-setup --help\n  tiger-setup build --help\n",
        style.paint("1", "tiger-setup build TigerSetup.toml")
    ));
    if installed_help {
        text.push_str("\nDocumentation:\n  Start > TigerSetup > TigerSetup Help\n");
    }
    text
}

/// Prints the brief landing help, in colour where the console shows it.
pub fn print_brief_help(in_shell: bool) -> std::io::Result<()> {
    let install_dir = install_directory()?;
    let installed_help = install_dir.join(HELP_PDF).is_file();
    let colour = ConsoleColour::enable();
    let style = if colour.is_some() {
        Style::Colour
    } else {
        Style::Plain
    };
    let mut out = std::io::stdout().lock();
    out.write_all(brief_help(&install_dir, in_shell, installed_help, style).as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

/// The directory of this `tiger-setup.exe`.
fn install_directory() -> std::io::Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".")))
}

/// Virtual-terminal processing on standard output while the value lives,
/// and the console's own mode put back afterwards. There is none when output
/// is not a console, when `NO_COLOR` asks for none, or when the console
/// cannot show colour.
struct ConsoleColour {
    handle: windows_sys::Win32::Foundation::HANDLE,
    mode: u32,
}

impl ConsoleColour {
    fn enable() -> Option<Self> {
        use windows_sys::Win32::System::Console::{
            ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE,
            SetConsoleMode,
        };
        if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
            return None;
        }
        // SAFETY: console calls on this process's standard output handle,
        // with a valid pointer for the mode.
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode = 0u32;
            if GetConsoleMode(handle, &mut mode) == 0
                || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) == 0
            {
                return None;
            }
            Some(ConsoleColour { handle, mode })
        }
    }
}

impl Drop for ConsoleColour {
    fn drop(&mut self) {
        // SAFETY: puts back the mode read from the same handle in `enable`.
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleMode(self.handle, self.mode);
        }
    }
}

/// The `PATH` the shell runs with: `install_dir` first, then `current`
/// unchanged. `None` when `current` already begins with `install_dir`, so
/// the shell inherits it as it is.
pub fn path_with_first(install_dir: &Path, current: Option<&OsStr>) -> Option<OsString> {
    let current = current.unwrap_or_default();
    let first = current
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .find(|entry| !entry.is_empty())
        .map(str::to_string);
    if first.is_some_and(|entry| same_directory(Path::new(&entry), install_dir)) {
        return None;
    }
    // A directory whose name holds the separator is quoted, as PATH allows.
    let mut path = if install_dir.to_string_lossy().contains(';') {
        OsString::from(format!("\"{}\"", install_dir.display()))
    } else {
        OsString::from(install_dir.as_os_str())
    };
    if !current.is_empty() {
        path.push(";");
        path.push(current);
    }
    Some(path)
}

/// Whether two directory spellings name the same directory, the way `PATH`
/// entries are compared: case-insensitively, ignoring surrounding quotes and
/// a trailing separator.
fn same_directory(a: &Path, b: &Path) -> bool {
    let normal = |path: &Path| {
        path.to_string_lossy()
            .trim()
            .trim_matches('"')
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    normal(a) == normal(b)
}

/// The command interpreter: `%ComSpec%`, or `cmd.exe` in the system
/// directory when `ComSpec` is unset.
pub fn command_interpreter(comspec: Option<&OsStr>, system_root: Option<&OsStr>) -> PathBuf {
    match comspec.filter(|value| !value.is_empty()) {
        Some(value) => PathBuf::from(value),
        None => Path::new(system_root.unwrap_or(OsStr::new("C:\\Windows")))
            .join("System32")
            .join("cmd.exe"),
    }
}

/// Where the shell starts: where it was asked from, except when that is the
/// install directory itself — which is where a Start Menu shortcut starts a
/// program — in which case the user's profile, where their work is.
pub fn working_directory(
    current: Option<&Path>,
    install_dir: &Path,
    profile: Option<&OsStr>,
) -> Option<PathBuf> {
    let profile = profile
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_dir());
    match current {
        Some(current) if !same_directory(current, install_dir) => Some(current.to_path_buf()),
        _ => profile,
    }
}

/// Runs the shell until the user leaves it and returns its exit code.
pub fn run() -> std::io::Result<i32> {
    use std::os::windows::process::CommandExt;

    let install_dir = install_directory()?;
    let interpreter = command_interpreter(
        std::env::var_os("ComSpec").as_deref(),
        std::env::var_os("SystemRoot").as_deref(),
    );

    print_brief_help(true)?;

    let mut command = Command::new(&interpreter);
    // `/k` runs the command line and keeps the interpreter for the user. The
    // line is passed as it is written: cmd.exe parses its own command line.
    command.arg("/k").raw_arg(format!("title {TITLE}"));
    if let Some(path) = path_with_first(&install_dir, std::env::var_os("PATH").as_deref()) {
        command.env("PATH", path);
    }
    if let Some(directory) = working_directory(
        std::env::current_dir().ok().as_deref(),
        &install_dir,
        std::env::var_os("USERPROFILE").as_deref(),
    ) {
        command.current_dir(directory);
    }

    ignore_console_interrupts();
    let status = command.status().map_err(|err| {
        std::io::Error::new(
            err.kind(),
            format!("cannot start {}: {err}", interpreter.display()),
        )
    })?;
    Ok(status.code().unwrap_or(1))
}

/// Ctrl+C and Ctrl+Break in the shell belong to the shell and what it runs.
/// This process only waits for it, so it lets them pass rather than ending
/// and leaving the interpreter without the program that started it. A
/// handler, not the inherited ignore flag, so the interpreter's own children
/// still receive them.
fn ignore_console_interrupts() {
    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
    };

    unsafe extern "system" fn handler(event: u32) -> windows_sys::core::BOOL {
        if event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT {
            TRUE
        } else {
            FALSE
        }
    }
    // SAFETY: registers a handler with the signature the API requires; it
    // touches no state and stays valid for the life of the process.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), TRUE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_directory_goes_first_unless_it_already_is() {
        let dir = Path::new("C:\\Programs\\TigerSetup");
        assert_eq!(
            path_with_first(dir, Some(OsStr::new("C:\\Windows;C:\\Tools"))),
            Some(OsString::from(
                "C:\\Programs\\TigerSetup;C:\\Windows;C:\\Tools"
            ))
        );
        // Already first, in any spelling PATH allows: nothing changes.
        for current in [
            "C:\\Programs\\TigerSetup;C:\\Windows",
            "c:\\programs\\tigersetup\\;C:\\Windows",
            ";\"C:\\Programs\\TigerSetup\";C:\\Windows",
        ] {
            assert_eq!(
                path_with_first(dir, Some(OsStr::new(current))),
                None,
                "{current}"
            );
        }
        // Present, but behind another directory that may hold another
        // tiger-setup.exe: it goes first all the same.
        assert_eq!(
            path_with_first(dir, Some(OsStr::new("C:\\Other;C:\\Programs\\TigerSetup"))),
            Some(OsString::from(
                "C:\\Programs\\TigerSetup;C:\\Other;C:\\Programs\\TigerSetup"
            ))
        );
        assert_eq!(
            path_with_first(dir, None),
            Some(OsString::from("C:\\Programs\\TigerSetup"))
        );
        assert_eq!(
            path_with_first(Path::new("C:\\A;B"), Some(OsStr::new("C:\\Windows"))),
            Some(OsString::from("\"C:\\A;B\";C:\\Windows"))
        );
    }

    #[test]
    fn the_interpreter_is_comspec_with_the_system_cmd_as_the_fallback() {
        assert_eq!(
            command_interpreter(Some(OsStr::new("D:\\Shells\\cmd.exe")), None),
            PathBuf::from("D:\\Shells\\cmd.exe")
        );
        assert_eq!(
            command_interpreter(Some(OsStr::new("")), Some(OsStr::new("C:\\Win"))),
            PathBuf::from("C:\\Win\\System32\\cmd.exe")
        );
    }

    #[test]
    fn the_brief_help_is_short_and_coloured_only_where_it_shows() {
        let dir = Path::new("C:\\Programs\\TigerSetup");
        let plain = brief_help(dir, true, true, Style::Plain);
        assert!(!plain.contains('\x1b'), "{plain}");
        assert!(plain.starts_with(concat!("TigerSetup ", env!("CARGO_PKG_VERSION"))));
        for line in [
            "tiger-setup is on PATH in this window. Type exit to close it.",
            "  tiger-setup build TigerSetup.toml",
            "  tiger-setup --help",
            "  tiger-setup build --help",
            "  Start > TigerSetup > TigerSetup Help",
        ] {
            assert!(plain.lines().any(|l| l == line), "{line:?} in {plain}");
        }
        // It fits the window a console opens with, with room to spare.
        assert!(plain.lines().count() <= 18, "{plain}");
        assert!(plain.lines().all(|l| l.chars().count() <= 80), "{plain}");

        let coloured = brief_help(dir, true, true, Style::Colour);
        assert!(coloured.contains("\x1b[92mtiger-setup is on PATH in this window.\x1b[0m"));
        assert!(coloured.contains("\x1b[1mtiger-setup build TigerSetup.toml\x1b[0m"));
        // Nothing but the colour differs.
        let stripped: String = coloured
            .split('\x1b')
            .enumerate()
            .map(|(i, part)| {
                if i == 0 {
                    part
                } else {
                    part.split_once('m').unwrap().1
                }
            })
            .collect();
        assert_eq!(stripped, plain);

        let elsewhere = brief_help(dir, false, false, Style::Plain);
        assert!(!elsewhere.contains("on PATH"), "{elsewhere}");
        assert!(!elsewhere.contains("Start >"), "{elsewhere}");
    }

    #[test]
    fn a_shell_started_in_the_install_directory_starts_in_the_profile() {
        let dir = Path::new("C:\\Programs\\TigerSetup");
        let profile = std::env::temp_dir();
        assert_eq!(
            working_directory(
                Some(Path::new("C:\\Programs\\TigerSetup\\")),
                dir,
                Some(profile.as_os_str())
            ),
            Some(profile.clone())
        );
        assert_eq!(
            working_directory(Some(Path::new("D:\\Work")), dir, Some(profile.as_os_str())),
            Some(PathBuf::from("D:\\Work"))
        );
    }
}
