//! The loader: the small executable every generated `Setup.exe` begins with.
//!
//! A generated installer is `[loader][compressed engine][payload][metadata]
//! [footer]` (`tigersetup-format`). The loader's whole job is to get the
//! real engine running against the file it came from:
//!
//! 1. locate and validate the footer of its own file;
//! 2. decompress the engine block into a fresh temporary file, checking
//!    its length and SHA-256 against the footer before anything is
//!    executed;
//! 3. start that engine with this process's own command line, verbatim,
//!    plus `--package <this file>`, so the engine reads the metadata and
//!    the payload from the original `Setup.exe` and relaunches *that* when
//!    it needs an elevated run or a temporary uninstaller copy;
//! 4. wait, propagate the engine's exit code, and remove the temporary.
//!
//! Nothing else lives here: no metadata, no payload, no state, no
//! transaction. The loader does not decide anything about the run — the
//! engine parses the command line and the engine asks for elevation — so a
//! machine-scope run is an elevated `Setup.exe` (this loader again) that
//! extracts its own engine under a system-owned root nobody else can write.
//!
//! **Where the engine is extracted.** Unelevated: `%TEMP%\TigerSetup\<pid>-
//! <tick>\<Setup.exe's own file name>`, which is the invoking user's own
//! folder. Elevated: a fresh directory under `%SystemRoot%\Temp`, created
//! with an access control list that grants SYSTEM and Administrators alone,
//! owner Administrators, atomically at creation — an executable about to
//! run elevated must never sit in a folder the invoking user can write
//! (`LESSONS_LEARNED.md`). A name that already exists is never adopted.
//! The file keeps the package's own name, so Task Manager, the Restart
//! Manager and a crash dialog all say which installer is running.
//!
//! **Failure.** A file that does not carry a valid engine — no footer, a
//! hash that does not match, a block that does not decompress — is reported
//! as a damaged package: on standard error, and in a message box when the
//! loader was started with no arguments at all (a double-click), and exits
//! 2, the engine's own code for an invalid package. Nothing half-extracted
//! is left behind: the temporary is removed on every path.

#![windows_subsystem = "windows"]

#[path = "console.rs"]
mod console;

use std::fs::File;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tigersetup_format::FormatError;
use tigersetup_format::footer::Footer;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
use windows_sys::Win32::System::Environment::GetCommandLineW;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

/// The engine's exit code for an invalid package, which is what a file the
/// loader cannot bootstrap is.
const INVALID: i32 = 2;

/// The argument the engine reads the package path from.
const PACKAGE_ARGUMENT: &str = "--package";

/// The access control list of an elevated extraction directory: owner
/// Administrators, protected, full control for SYSTEM and Administrators
/// and nobody else — the same list the engine puts on a machine-scope
/// state directory.
const ELEVATED_DACL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    // Load system DLLs from System32 only, before anything is loaded on
    // demand: an installer runs from a download folder, which is exactly the
    // application directory a planted DLL would be searched in first.
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    console::attach_to_parent();

    let tail = command_line_tail();
    match bootstrap(&tail) {
        Ok(code) => code,
        Err(err) => {
            let package = std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "this installer".into());
            let message = format!("{package} cannot start: {}", err.message);
            let _ = writeln!(std::io::stderr(), "error: {message}");
            if tail.trim().is_empty() {
                message_box(&message);
            }
            INVALID
        }
    }
}

/// Extracts the engine, runs it and cleans up. The exit code is the
/// engine's.
fn bootstrap(tail: &str) -> Result<i32, FormatError> {
    let package = std::env::current_exe().map_err(|err| {
        FormatError::new(
            "package_unreadable",
            format!("cannot locate this executable: {err}"),
        )
    })?;
    let file = File::open(&package).map_err(|err| {
        FormatError::new(
            "package_unreadable",
            format!("cannot open {}: {err}", package.display()),
        )
    })?;
    let footer = tigersetup_format::installer::read_footer(&file)?;

    let directory = extraction_directory()?;
    let result = run_engine(&package, &file, &footer, &directory, tail);
    remove_directory(&directory);
    result
}

fn run_engine(
    package: &Path,
    file: &File,
    footer: &Footer,
    directory: &Path,
    tail: &str,
) -> Result<i32, FormatError> {
    let file_name = package
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "Setup.exe".into());
    let engine = directory.join(&file_name);
    let partial = directory.join(format!("{}.partial", file_name.to_string_lossy()));
    {
        let mut out = File::create(&partial).map_err(|err| {
            FormatError::new(
                "io_error",
                format!("cannot create {}: {err}", partial.display()),
            )
        })?;
        tigersetup_format::installer::extract_engine(file, footer, &mut out)?;
        out.flush()?;
    }
    // The file is the engine only once it is complete and verified; a
    // crash before this rename leaves a `.partial` that nothing executes.
    std::fs::rename(&partial, &engine).map_err(|err| {
        FormatError::new(
            "io_error",
            format!("cannot place {}: {err}", engine.display()),
        )
    })?;

    // The engine gets this process's command line exactly as it was given
    // — every argument, every quote, every non-ASCII character — with the
    // package appended. Standard handles are inherited, so redirected
    // output goes where the caller sent it and a console attached above is
    // the engine's too.
    let mut command = Command::new(&engine);
    if !tail.trim().is_empty() {
        command.raw_arg(tail);
    }
    command.arg(PACKAGE_ARGUMENT).arg(package);
    let status = command.status().map_err(|err| {
        FormatError::new(
            "engine_unavailable",
            format!("cannot start {}: {err}", engine.display()),
        )
    })?;
    Ok(status.code().unwrap_or(INVALID))
}

/// A fresh directory of this launch's own, for the engine to run from.
fn extraction_directory() -> Result<PathBuf, FormatError> {
    let elevated = is_elevated();
    let root = if elevated {
        system_root().join("Temp")
    } else {
        std::env::temp_dir().join("TigerSetup")
    };
    if !elevated {
        std::fs::create_dir_all(&root).map_err(|err| {
            FormatError::new(
                "io_error",
                format!("cannot create {}: {err}", root.display()),
            )
        })?;
        sweep_stale(&root);
    }
    let pid = std::process::id();
    let mut attempt = 0u32;
    loop {
        let tick = unsafe { GetTickCount64() };
        let name = if elevated {
            format!("TigerSetup-{pid}-{tick}-{attempt}")
        } else {
            format!("{pid}-{tick}-{attempt}")
        };
        let directory = root.join(name);
        match create_directory(&directory, elevated) {
            Ok(()) => return Ok(directory),
            Err(err) if err.raw_os_error() == Some(ERROR_ALREADY_EXISTS as i32) && attempt < 16 => {
                attempt += 1;
            }
            Err(err) => {
                return Err(FormatError::new(
                    "io_error",
                    format!("cannot create {}: {err}", directory.display()),
                ));
            }
        }
    }
}

/// Creates `directory`, which must not exist. An elevated run creates it
/// with [`ELEVATED_DACL`] in the same call, so there is no moment at which
/// it exists with a weaker list.
fn create_directory(directory: &Path, elevated: bool) -> std::io::Result<()> {
    if !elevated {
        return std::fs::create_dir(directory);
    }
    let sddl: Vec<u16> = ELEVATED_DACL
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor: *mut std::ffi::c_void = std::ptr::null_mut();
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let wide: Vec<u16> = directory
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let created = unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) };
    let error = std::io::Error::last_os_error();
    unsafe { LocalFree(descriptor) };
    if created == 0 { Err(error) } else { Ok(()) }
}

/// Removes the extraction directory and everything in it. An antivirus may
/// still hold the engine open for a moment after it exited, so the removal
/// is retried briefly; what cannot be removed is left, harmless, in a
/// folder of this launch's own.
fn remove_directory(directory: &Path) {
    for attempt in 0..10 {
        if std::fs::remove_dir_all(directory).is_ok() || !directory.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100 * (attempt + 1)));
    }
}

/// Removes extraction directories a killed launch left under the user's
/// root, once they are a day old: young ones may belong to a launch still
/// running.
fn sweep_stale(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let stale = Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let path = entry.path();
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > stale);
        if old && path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// This process's command line after its own program name, verbatim.
fn command_line_tail() -> String {
    let raw = unsafe { GetCommandLineW() };
    if raw.is_null() {
        return String::new();
    }
    let mut length = 0usize;
    while unsafe { *raw.add(length) } != 0 {
        length += 1;
    }
    let full = unsafe { std::slice::from_raw_parts(raw, length) };
    String::from_utf16_lossy(skip_program(full))
        .trim_start()
        .to_string()
}

/// The command line without its first token, by the rule the C runtime
/// parses the program name with: a quoted token ends at the closing quote,
/// an unquoted one at the first space or tab.
fn skip_program(full: &[u16]) -> &[u16] {
    let quote = u16::from(b'"');
    let space = u16::from(b' ');
    let tab = u16::from(b'\t');
    let mut index = 0;
    if full.first() == Some(&quote) {
        index = 1;
        while index < full.len() && full[index] != quote {
            index += 1;
        }
        if index < full.len() {
            index += 1;
        }
    } else {
        while index < full.len() && full[index] != space && full[index] != tab {
            index += 1;
        }
    }
    &full[index..]
}

/// Whether this process runs with an elevated token. The engine has the
/// same question in its Win32 layer; the loader asks it itself because it
/// links no part of the engine.
fn is_elevated() -> bool {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    unsafe { CloseHandle(token) };
    ok != 0 && elevation.TokenIsElevated != 0
}

fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
}

fn message_box(message: &str) {
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "Setup".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn the_program_token_is_skipped_by_the_c_runtime_rule() {
        let tail = |line: &str| String::from_utf16_lossy(skip_program(&wide(line)));
        assert_eq!(
            tail(r#""C:\a b\Setup.exe" install --quiet"#),
            " install --quiet"
        );
        assert_eq!(tail(r"Setup.exe install"), " install");
        assert_eq!(tail(r"Setup.exe"), "");
        assert_eq!(tail(r#""C:\Setup.exe""#), "");
        assert_eq!(
            tail(r#""Setup.exe" --option "name" "zażółć gęślą""#),
            r#" --option "name" "zażółć gęślą""#
        );
    }
}
