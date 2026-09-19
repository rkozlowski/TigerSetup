//! Running another program and waiting for it: a dependency's installer,
//! hidden and unattended through `CreateProcessW`; or, through
//! `ShellExecuteExW` with the `runas` verb, a program that needs an
//! administrator — a dependency installer that asks for one, or this same
//! executable relaunched for machine scope — so that the UAC prompt is for
//! exactly that program. Every launch waits for the process and returns its
//! exit code.
//!
//! An elevated launch says how the child's window is to be shown, because
//! the child's first `ShowWindow` follows the `STARTUPINFO` it was started
//! with rather than its own argument: a child started [`Show::Hidden`]
//! creates its main window invisible. That is right for a `/quiet`
//! installer and wrong for a child that is about to show a wizard, so the
//! caller states which of the two it is starting.
//!
//! A custom action is run through [`run_captured`]: hidden, with no
//! standard input, its output captured, and killed at a deadline — the
//! envelope an unattended program gets, whoever wrote it. The action and
//! everything it starts live in a job object, so that the deadline ends
//! the whole tree and nothing the action started outlives it: a grandchild
//! left holding the output pipe would otherwise keep the run waiting long
//! after the action itself exited.

use std::ffi::c_void;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    OpenProcessToken, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};

/// `SW_HIDE`.
const SW_HIDE: i32 = 0;
/// `SW_SHOWNORMAL`.
const SW_SHOWNORMAL: i32 = 1;

/// How a launched program's first window is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    /// The child shows an interface of its own: its window comes up as a
    /// window normally does.
    Shown,
    /// The child runs unattended and has nothing to show; a window it
    /// creates anyway stays hidden.
    Hidden,
}

impl Show {
    fn command(self) -> i32 {
        match self {
            Show::Shown => SW_SHOWNORMAL,
            Show::Hidden => SW_HIDE,
        }
    }
}

/// Why a launch did not yield an exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// The user refused the elevation prompt.
    Refused,
    /// The process could not be started or waited for.
    Failed(String),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Refused => write!(f, "the elevation prompt was refused"),
            LaunchError::Failed(message) => write!(f, "{message}"),
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// Quotes one argument the way the C runtime's `CommandLineToArgvW`
/// undoes it: quotes only when needed, so an argument such as `&` reaches
/// a shell unchanged.
pub fn quote_argument(argument: &str) -> String {
    let needs_quotes = argument.is_empty()
        || argument
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '"'));
    if !needs_quotes {
        return argument.to_string();
    }
    let mut out = String::with_capacity(argument.len() + 2);
    out.push('"');
    let mut backslashes = 0;
    for c in argument.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// The arguments joined as one command line.
pub fn join_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|a| quote_argument(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The full command line: the quoted program followed by the arguments.
pub fn command_line(program: &Path, arguments: &[String]) -> String {
    let mut line = quote_argument(&program.display().to_string());
    let rest = join_arguments(arguments);
    if !rest.is_empty() {
        line.push(' ');
        line.push_str(&rest);
    }
    line
}

fn wait_for_exit(process: HANDLE) -> Result<i32, LaunchError> {
    unsafe { WaitForSingleObject(process, INFINITE) };
    let mut code: u32 = 0;
    let ok = unsafe { GetExitCodeProcess(process, &mut code) };
    unsafe { CloseHandle(process) };
    if ok == 0 {
        return Err(LaunchError::Failed(format!(
            "cannot read the exit code: {}",
            last_error()
        )));
    }
    Ok(code as i32)
}

/// Runs the program with no window and waits for it.
pub fn run_hidden(program: &Path, arguments: &[String]) -> Result<i32, LaunchError> {
    let program_w = wide(&program.display().to_string());
    let mut line_w = wide(&command_line(program, arguments));
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESHOWWINDOW;
    startup.wShowWindow = SW_HIDE as u16;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            program_w.as_ptr(),
            line_w.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut info,
        )
    };
    if ok == 0 {
        return Err(LaunchError::Failed(format!(
            "cannot start {}: {}",
            program.display(),
            last_error()
        )));
    }
    unsafe { CloseHandle(info.hThread) };
    wait_for_exit(info.hProcess)
}

/// Runs the program with this process's own token and waits for it. What
/// the child reports comes back through the result file its arguments name,
/// and this process reports it as its own — so the child's standard streams
/// are closed rather than inherited, or a caller would read the same
/// document twice.
pub fn run_plain(program: &Path, arguments: &[String]) -> Result<i32, LaunchError> {
    use std::process::Stdio;
    let status = std::process::Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| LaunchError::Failed(format!("cannot start {}: {err}", program.display())))?;
    Ok(status.code().unwrap_or(crate::report::exit::ROLLED_BACK))
}

/// One captured, bounded, unattended launch.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: PathBuf,
    /// Passed as separate arguments, quoted for the C runtime.
    pub arguments: Vec<String>,
    /// Appended to the command line verbatim, after the arguments, for an
    /// interpreter that parses its own line (`cmd.exe /s /c "..."`).
    pub raw_tail: Option<String>,
    pub working_directory: Option<PathBuf>,
    /// Added to this process's environment for the child.
    pub environment: Vec<(String, String)>,
    /// The child is killed once this passes.
    pub timeout: Duration,
}

/// What a captured launch produced.
#[derive(Debug, Clone, Default)]
pub struct Captured {
    /// The exit code; `None` when the child was killed at the deadline.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

/// A job object every process of one captured launch belongs to, created
/// to kill its processes when it is closed. Dropping it therefore ends
/// whatever the launch left running.
struct Job {
    handle: HANDLE,
}

impl Job {
    fn create() -> Result<Job, LaunchError> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(LaunchError::Failed(format!(
                "cannot create a job object: {}",
                last_error()
            )));
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            let err = last_error();
            unsafe { CloseHandle(handle) };
            return Err(LaunchError::Failed(format!(
                "cannot configure a job object: {err}"
            )));
        }
        Ok(Job { handle })
    }

    fn assign(&self, process: HANDLE) -> Result<(), LaunchError> {
        if unsafe { AssignProcessToJobObject(self.handle, process) } == 0 {
            return Err(LaunchError::Failed(format!(
                "cannot assign the process to its job object: {}",
                last_error()
            )));
        }
        Ok(())
    }

    /// Ends every process still in the job.
    fn terminate(&self) {
        unsafe { TerminateJobObject(self.handle, 1) };
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// Reads one of the child's streams to its end on a thread of its own, so
/// that a child writing more than a pipe holds never blocks on this process
/// while this process waits on it.
fn drain(stream: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stream) = stream {
            let _ = stream.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Runs the program hidden and unattended — no window, no standard input —
/// captures what it writes, waits for it up to the deadline and kills it
/// there. The exit code is the child's own; a deadline is `timed_out` with
/// no code. Everything the program started ends with it, whichever way it
/// ended. A program that cannot be started at all is [`LaunchError::Failed`].
pub fn run_captured(launch: &Launch) -> Result<Captured, LaunchError> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let job = Job::create()?;

    let mut command = Command::new(&launch.program);
    command.args(&launch.arguments);
    if let Some(tail) = &launch.raw_tail {
        command.raw_arg(tail);
    }
    if let Some(directory) = &launch.working_directory {
        command.current_dir(directory);
    }
    for (name, value) in &launch.environment {
        command.env(name, value);
    }
    let started = Instant::now();
    let mut child = command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            LaunchError::Failed(format!("cannot start {}: {err}", launch.program.display()))
        })?;
    if let Err(err) = job.assign(child.as_raw_handle() as HANDLE) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(err);
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let mut timed_out = false;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if started.elapsed() >= launch.timeout {
                    timed_out = true;
                    job.terminate();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                job.terminate();
                return Err(LaunchError::Failed(format!(
                    "cannot wait for {}: {err}",
                    launch.program.display()
                )));
            }
        }
    };
    // The action is over: whatever it left running goes with it, which is
    // also what lets the streams reach their end.
    job.terminate();
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    Ok(Captured {
        exit_code,
        timed_out,
        stdout,
        stderr,
        duration: started.elapsed(),
    })
}

/// A program started elevated and not yet waited for. Dropping it without
/// waiting leaves the program running and only closes the handle.
pub struct Elevated {
    process: HANDLE,
}

impl Elevated {
    /// Waits for the program to exit and returns its exit code.
    pub fn wait(self) -> Result<i32, LaunchError> {
        let process = self.process;
        // `wait_for_exit` closes the handle; the drop below must not.
        std::mem::forget(self);
        wait_for_exit(process)
    }
}

impl Drop for Elevated {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.process) };
    }
}

/// Runs the program elevated (`runas`), with its window shown or hidden as
/// `show` says, and waits for it. A refused UAC prompt is
/// [`LaunchError::Refused`].
pub fn run_elevated(program: &Path, arguments: &[String], show: Show) -> Result<i32, LaunchError> {
    start_elevated(program, arguments, show)?.wait()
}

/// Starts the program elevated (`runas`), with its window shown or hidden as
/// `show` says, and returns once the prompt has been answered and the
/// program is running, so that a caller can act on that before waiting for
/// it. A refused UAC prompt is [`LaunchError::Refused`].
pub fn start_elevated(
    program: &Path,
    arguments: &[String],
    show: Show,
) -> Result<Elevated, LaunchError> {
    let verb = wide("runas");
    let file = wide(&program.display().to_string());
    let parameters = wide(&join_arguments(arguments));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = show.command();
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let code = unsafe { GetLastError() };
        return Err(if code == ERROR_CANCELLED {
            LaunchError::Refused
        } else {
            LaunchError::Failed(format!(
                "cannot start {} elevated: {}",
                program.display(),
                last_error()
            ))
        });
    }
    if info.hProcess.is_null() || info.hProcess == INVALID_HANDLE_VALUE {
        return Err(LaunchError::Failed(format!(
            "{} started elevated but yielded no process handle",
            program.display()
        )));
    }
    Ok(Elevated {
        process: info.hProcess,
    })
}

/// Whether this process runs with an elevated token.
pub fn is_elevated() -> bool {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned: u32 = 0;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    unsafe { CloseHandle(token) };
    ok != 0 && elevation.TokenIsElevated != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cmd() -> PathBuf {
        PathBuf::from(std::env::var("SystemRoot").unwrap())
            .join("System32")
            .join("cmd.exe")
    }

    #[test]
    fn arguments_are_quoted_like_the_c_runtime_expects() {
        assert_eq!(quote_argument("/quiet"), "/quiet");
        assert_eq!(quote_argument("&"), "&");
        assert_eq!(quote_argument(""), "\"\"");
        assert_eq!(
            quote_argument("C:\\Program Files\\x"),
            "\"C:\\Program Files\\x\""
        );
        assert_eq!(quote_argument("a\\"), "a\\");
        assert_eq!(quote_argument("a b\\"), "\"a b\\\\\"");
        assert_eq!(quote_argument("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(
            command_line(Path::new("C:\\x\\setup.exe"), &["/S".into(), "a b".into()]),
            "C:\\x\\setup.exe /S \"a b\""
        );
    }

    #[test]
    fn hidden_processes_report_their_exit_code() {
        let code = run_hidden(&cmd(), &["/c".into(), "exit".into(), "3010".into()]).unwrap();
        assert_eq!(code, 3010);
        let code = run_hidden(&cmd(), &["/c".into(), "exit".into(), "0".into()]).unwrap();
        assert_eq!(code, 0);
        // Negative codes survive the round trip through the u32 exit code.
        let code = run_hidden(&cmd(), &["/c".into(), "exit".into(), "-2147219416".into()]).unwrap();
        assert_eq!(code, -2147219416);
        let err = run_hidden(Path::new("C:\\nope\\missing.exe"), &[]).unwrap_err();
        assert!(matches!(err, LaunchError::Failed(_)));
    }

    #[test]
    fn a_shell_command_with_a_marker_directory_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("marker dir").join("1.0");
        let code = run_hidden(
            &cmd(),
            &[
                "/c".into(),
                "mkdir".into(),
                marker.display().to_string(),
                "&".into(),
                "exit".into(),
                "7".into(),
            ],
        )
        .unwrap();
        assert_eq!(code, 7);
        assert!(marker.is_dir());
    }

    fn cmd_launch(arguments: &[&str], timeout_secs: u64) -> Launch {
        Launch {
            program: cmd(),
            arguments: arguments.iter().map(|a| a.to_string()).collect(),
            raw_tail: None,
            working_directory: None,
            environment: vec![("TIGERSETUP_TEST_PROBE".into(), "probe value".into())],
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// The captured runner returns the child's own exit code and both
    /// streams, sees the environment it was given, runs where it was told
    /// to, and reports a deadline as a timeout rather than as an exit code.
    #[test]
    fn captured_launches_report_streams_exit_codes_and_deadlines() {
        let dir = tempfile::tempdir().unwrap();
        let mut launch = cmd_launch(
            &[
                "/c",
                "echo out %TIGERSETUP_TEST_PROBE% & echo err 1>&2 & cd & exit 7",
            ],
            30,
        );
        launch.working_directory = Some(dir.path().to_path_buf());
        let captured = run_captured(&launch).unwrap();
        assert_eq!(captured.exit_code, Some(7));
        assert!(!captured.timed_out);
        assert!(captured.stdout.contains("out probe value"), "{captured:?}");
        assert!(
            captured
                .stdout
                .to_ascii_lowercase()
                .contains(&dir.path().display().to_string().to_ascii_lowercase()),
            "{captured:?}"
        );
        assert!(captured.stderr.contains("err"), "{captured:?}");

        // The deadline ends the whole tree: cmd's own child, ping, would
        // otherwise hold the output pipe for its full thirty seconds.
        let started = Instant::now();
        let captured = run_captured(&cmd_launch(&["/c", "ping -n 30 127.0.0.1 >nul"], 1)).unwrap();
        assert!(captured.timed_out);
        assert_eq!(captured.exit_code, None);
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the child and its tree were killed"
        );
        // And a program that exits leaving a background child holding the
        // pipe does not hold the run: the tree ends with the program.
        let started = Instant::now();
        let captured = run_captured(&cmd_launch(
            &["/c", "start /b ping -n 30 127.0.0.1 >nul & exit 0"],
            30,
        ))
        .unwrap();
        assert_eq!(captured.exit_code, Some(0), "{captured:?}");
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the orphan was ended with the action"
        );

        let err = run_captured(&Launch {
            program: PathBuf::from("C:\\nope\\missing.exe"),
            ..cmd_launch(&[], 1)
        })
        .unwrap_err();
        assert!(matches!(err, LaunchError::Failed(_)));
    }

    #[test]
    fn elevation_is_a_plain_boolean() {
        // Whatever the test runs as, the call must not fail.
        let _ = is_elevated();
    }
}
