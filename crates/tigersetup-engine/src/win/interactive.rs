//! Starting a program as the person signed in to this desktop, never
//! elevated, and handing it the foreground: launch after install
//! (`TigerSetup-Design.md` §11.7).
//!
//! **Which token.** The decision is the token this process holds, not the
//! scope of the run:
//!
//! - a process that is not elevated starts the program itself, with
//!   `CreateProcessW` and its own token, which is the signed-in user's;
//! - an elevated process — the wizard's elevated child after a UAC prompt,
//!   or an installer started elevated — asks the desktop's shell to start
//!   it. `ShellWindows` is registered to run as the interactive user, so
//!   from any account, the over-the-shoulder administrator included, it
//!   reaches the Explorer of this desktop; its desktop folder view hands out
//!   `Shell.Application`, and `IShellDispatch2::ShellExecute` there makes
//!   Explorer the program's parent, with Explorer's token and environment.
//!
//! Duplicating Explorer's token instead would not work across accounts: an
//! interactive logon's token grants the Administrators group `TOKEN_QUERY`
//! and nothing more, so an administrator who elevated over a standard
//! user's shoulder cannot duplicate it. Querying is what this module does
//! with it: before asking the shell, it reads the shell's token and refuses
//! a shell that is itself elevated — User Account Control off, or the
//! built-in Administrator — because then no non-elevated context exists on
//! this desktop. Nothing ever falls back to this process's own elevated
//! token.
//!
//! **Foreground.** The wizard is the foreground window when the person
//! presses Finish, and that is the right Windows lets it pass on. The
//! program is created suspended and `AllowSetForegroundWindow` names it
//! before its first instruction runs; through the shell, Explorer is named
//! before it is asked, and the program once it appears. The caller then
//! waits, still in the foreground, for the program's window and puts it
//! there with `SetForegroundWindow` ([`Started::wait_for_window`],
//! [`bring_to_foreground`]) — so the outcome does not depend on how Windows
//! happens to arbitrate a window that appears after its creator is gone.

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, SysAllocString, SysFreeString, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, EqualSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    SECURITY_MAX_SID_SIZE, TOKEN_ELEVATION, TOKEN_GROUPS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    TokenElevation, TokenGroups, TokenIntegrityLevel, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::SystemServices::{SE_GROUP_ENABLED, SE_GROUP_USE_FOR_DENY_ONLY};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetCurrentProcess, OpenProcess,
    OpenProcessToken, PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW,
    ResumeThread, STARTF_USESHOWWINDOW, STARTUPINFOW, WaitForInputIdle, WaitForSingleObject,
};
use windows_sys::Win32::System::Variant::{VARIANT, VT_BSTR, VT_EMPTY, VT_I4};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, EnumWindows, GW_OWNER, GWL_EXSTYLE, GetForegroundWindow,
    GetShellWindow, GetWindow, GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow, WS_EX_TOOLWINDOW,
};
use windows_sys::core::{GUID, HRESULT};

use super::process::join_arguments;

const CLSID_SHELL_WINDOWS: GUID = GUID::from_u128(0x9ba05972_f6a8_11cf_a442_00a0c90a8f39);
const IID_ISHELL_WINDOWS: GUID = GUID::from_u128(0x85cb6900_4d95_11cf_960c_0080c7f4ee85);
const IID_ISERVICE_PROVIDER: GUID = GUID::from_u128(0x6d5140c1_7436_11ce_8034_00aa006009fa);
const SID_STOP_LEVEL_BROWSER: GUID = GUID::from_u128(0x4c96be40_915c_11cf_99d3_00aa004ae837);
const IID_ISHELL_BROWSER: GUID = GUID::from_u128(0x000214e2_0000_0000_c000_000000000046);
const IID_IDISPATCH: GUID = GUID::from_u128(0x00020400_0000_0000_c000_000000000046);
const IID_ISHELL_FOLDER_VIEW_DUAL: GUID = GUID::from_u128(0xe7a1af80_4d96_11cf_960c_0080c7f4ee85);
const IID_ISHELL_DISPATCH2: GUID = GUID::from_u128(0xa4c6892c_3ba9_11d2_9dea_00c04fb16162);
/// `CSIDL_DESKTOP`: the desktop, which `FindWindowSW` names by location.
const CSIDL_DESKTOP: i32 = 0;
/// `SWC_DESKTOP`: the desktop window, not a browser window.
const SWC_DESKTOP: i32 = 0x8;
/// `SWFO_NEEDDISPATCH`: return the window's `IDispatch`.
const SWFO_NEEDDISPATCH: i32 = 0x1;
/// `SVGIO_BACKGROUND`: the view's background object, `IShellFolderViewDual`.
const SVGIO_BACKGROUND: u32 = 0;
/// `SECURITY_MANDATORY_HIGH_RID`: the integrity level of an elevated token.
const HIGH_INTEGRITY: u32 = 0x3000;
/// `RPC_E_CHANGED_MODE`: COM was already initialised with another model;
/// the apartment is usable all the same.
const RPC_E_CHANGED_MODE: HRESULT = 0x8001_0106u32 as i32;

/// How long a program started through the shell may take to appear in the
/// process list before the start is reported as unobserved.
const APPEARS_WITHIN: Duration = Duration::from_secs(10);
/// How long a program that has reached its message loop may still take to
/// show a window before it is taken to have none — a tray application.
const IDLE_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);

/// How the program was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `CreateProcessW` with this process's own token, which is not elevated.
    OwnToken,
    /// Explorer started it for this elevated process.
    Shell,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::OwnToken => "own_token",
            Method::Shell => "shell",
        }
    }
}

/// Why a program was not started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// No non-elevated context exists on this desktop to start it in, so it
    /// was not started at all: this process is elevated, and there is no
    /// shell, or the shell is elevated too, or its token cannot be read.
    Unavailable(String),
    /// Starting it was attempted and failed.
    Failed(String),
}

/// A program started as the interactive user.
#[derive(Debug)]
pub struct Started {
    pub method: Method,
    /// The process, when it was seen. A program started through the shell
    /// is found by name among the processes that were not there before; one
    /// that ends before it is seen — a second instance handing over to the
    /// first — leaves this empty.
    pub pid: Option<u32>,
    /// A handle to wait on, where this process may open one.
    process: Option<OwnedHandle>,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> std::io::Error {
    std::io::Error::last_os_error()
}

/// Starts `program` with `arguments` in `directory` as the signed-in user,
/// never elevated (see the module documentation for how).
pub fn start(
    program: &Path,
    arguments: &[String],
    directory: &Path,
) -> Result<Started, StartError> {
    match this_process_is_elevated() {
        false => start_with_own_token(program, arguments, directory),
        true => start_through_shell(program, arguments, directory),
    }
}

/// Whether this process's token is elevated in any of the ways that matter
/// here ([`token_is_elevated`]). A token that cannot be read counts as
/// elevated, which only ever sends the start through the shell.
pub fn this_process_is_elevated() -> bool {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return true;
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token as _) };
    token_is_elevated(token.as_raw_handle() as HANDLE).unwrap_or(true)
}

/// Whether a token is elevated: UAC says so, its integrity level is high or
/// above, or its Administrators group is enabled — the last is how a token
/// looks when User Account Control is off, where Windows reports no
/// elevation at all. `None` when the token cannot be read.
fn token_is_elevated(token: HANDLE) -> Option<bool> {
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    if ok == 0 {
        return None;
    }
    if elevation.TokenIsElevated != 0 {
        return Some(true);
    }
    let label = token_information(token, TokenIntegrityLevel)?;
    let level = unsafe {
        let label = &*(label.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        match count {
            0 => 0,
            n => *GetSidSubAuthority(sid, n as u32 - 1),
        }
    };
    if level >= HIGH_INTEGRITY {
        return Some(true);
    }
    let groups = token_information(token, TokenGroups)?;
    let mut administrators = [0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut size = administrators.len() as u32;
    if unsafe {
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            ptr::null_mut(),
            administrators.as_mut_ptr() as *mut c_void,
            &mut size,
        )
    } == 0
    {
        return None;
    }
    let enabled_administrator = unsafe {
        let groups = &*(groups.as_ptr() as *const TOKEN_GROUPS);
        let entries =
            std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize);
        entries.iter().any(|group| {
            EqualSid(group.Sid, administrators.as_mut_ptr() as *mut c_void) != 0
                && group.Attributes & SE_GROUP_ENABLED as u32 != 0
                && group.Attributes & SE_GROUP_USE_FOR_DENY_ONLY as u32 == 0
        })
    };
    Some(enabled_administrator)
}

/// One `GetTokenInformation` class into a buffer aligned for the structure
/// it holds.
fn token_information(
    token: HANDLE,
    class: windows_sys::Win32::Security::TOKEN_INFORMATION_CLASS,
) -> Option<Vec<u64>> {
    let mut needed = 0u32;
    unsafe { GetTokenInformation(token, class, ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return None;
    }
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    let ok = unsafe {
        GetTokenInformation(
            token,
            class,
            buffer.as_mut_ptr() as *mut c_void,
            (buffer.len() * 8) as u32,
            &mut needed,
        )
    };
    (ok != 0).then_some(buffer)
}

/// The fresh environment of the user `token` belongs to, as `CreateProcessW`
/// takes it: the one Windows would give a program the person starts now,
/// with whatever the installation just added to `PATH`, rather than this
/// process's copy from before the run.
struct EnvironmentBlock(*mut c_void);

impl EnvironmentBlock {
    fn of_this_process() -> Option<EnvironmentBlock> {
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let mut block: *mut c_void = ptr::null_mut();
        let ok = unsafe { CreateEnvironmentBlock(&mut block, token, 0) };
        unsafe { CloseHandle(token) };
        (ok != 0 && !block.is_null()).then_some(EnvironmentBlock(block))
    }
}

impl Drop for EnvironmentBlock {
    fn drop(&mut self) {
        unsafe { DestroyEnvironmentBlock(self.0) };
    }
}

fn start_with_own_token(
    program: &Path,
    arguments: &[String],
    directory: &Path,
) -> Result<Started, StartError> {
    let program_w = wide(&program.display().to_string());
    let mut line_w = wide(&super::process::command_line(program, arguments));
    let directory_w = wide(&directory.display().to_string());
    let environment = EnvironmentBlock::of_this_process();
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESHOWWINDOW;
    startup.wShowWindow = SW_SHOWNORMAL as u16;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            program_w.as_ptr(),
            line_w.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_SUSPENDED
                | match environment {
                    Some(_) => CREATE_UNICODE_ENVIRONMENT,
                    None => 0,
                },
            environment
                .as_ref()
                .map_or(ptr::null(), |block| block.0 as *const c_void),
            directory_w.as_ptr(),
            &startup,
            &mut info,
        )
    };
    if ok == 0 {
        return Err(StartError::Failed(format!(
            "cannot start {}: {}",
            program.display(),
            last_error()
        )));
    }
    // The right to take the foreground is the wizard's, and it is passed on
    // before the program runs a single instruction, so however quickly its
    // first window appears it may come to the front.
    unsafe {
        AllowSetForegroundWindow(info.dwProcessId);
        ResumeThread(info.hThread);
        CloseHandle(info.hThread);
    }
    Ok(Started {
        method: Method::OwnToken,
        pid: Some(info.dwProcessId),
        process: Some(unsafe { OwnedHandle::from_raw_handle(info.hProcess as _) }),
    })
}

fn start_through_shell(
    program: &Path,
    arguments: &[String],
    directory: &Path,
) -> Result<Started, StartError> {
    let shell = unsafe { GetShellWindow() };
    if shell.is_null() {
        return Err(StartError::Unavailable(
            "no desktop shell is running to start the program as the signed-in user".into(),
        ));
    }
    let mut shell_pid = 0u32;
    unsafe { GetWindowThreadProcessId(shell, &mut shell_pid) };
    match shell_token_is_elevated(shell_pid) {
        Some(false) => {}
        Some(true) => {
            return Err(StartError::Unavailable(
                "the desktop shell itself runs elevated, so there is no non-elevated context to start the program in".into(),
            ));
        }
        None => {
            return Err(StartError::Unavailable(format!(
                "the desktop shell's token cannot be read: {}",
                last_error()
            )));
        }
    }

    let file_name = program
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let before: Vec<u32> = processes_named(&file_name)
        .into_iter()
        .map(|(pid, _)| pid)
        .collect();
    unsafe { AllowSetForegroundWindow(shell_pid) };
    shell_execute(program, &join_arguments(arguments), directory).map_err(StartError::Failed)?;

    // Found by name among the processes that were not there before, the
    // one the shell parented first; its image path decides where it can be
    // read. A program that ends before it is seen is reported as started
    // but unobserved.
    let deadline = Instant::now() + APPEARS_WITHIN;
    let pid = loop {
        let mut candidates: Vec<(u32, u32)> = processes_named(&file_name)
            .into_iter()
            .filter(|(pid, _)| !before.contains(pid))
            .filter(|(pid, _)| image_path(*pid).is_none_or(|path| same_path(&path, program)))
            .collect();
        candidates.sort_by_key(|(_, parent)| *parent != shell_pid);
        if let Some((pid, _)) = candidates.first() {
            break Some(*pid);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(POLL);
    };
    let process = pid.and_then(|pid| {
        unsafe { AllowSetForegroundWindow(pid) };
        let handle =
            unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_INFORMATION, 0, pid) };
        (!handle.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(handle as _) })
    });
    Ok(Started {
        method: Method::Shell,
        pid,
        process,
    })
}

/// Reads the shell's token with the one right an interactive token grants
/// the Administrators group, `TOKEN_QUERY`.
fn shell_token_is_elevated(shell_pid: u32) -> Option<bool> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, shell_pid) };
    if process.is_null() {
        return None;
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process as _) };
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(process.as_raw_handle() as HANDLE, TOKEN_QUERY, &mut token) } == 0
    {
        return None;
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token as _) };
    token_is_elevated(token.as_raw_handle() as HANDLE)
}

/// Every running process whose executable is named `file_name`, with its
/// parent's id.
fn processes_named(file_name: &str) -> Vec<(u32, u32)> {
    let mut found = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return found;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry);
        while more != 0 {
            let length = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if name.eq_ignore_ascii_case(file_name) {
                found.push((entry.th32ProcessID, entry.th32ParentProcessID));
            }
            more = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    found
}

/// Whether a process id is still running.
fn is_running(pid: u32) -> bool {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return true;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry);
        let mut running = false;
        while more != 0 {
            if entry.th32ProcessID == pid {
                running = true;
                break;
            }
            more = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        running
    }
}

/// A process's image path, where this process may read it.
fn image_path(pid: u32) -> Option<PathBuf> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process as _) };
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle() as HANDLE,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.display()
        .to_string()
        .eq_ignore_ascii_case(&b.display().to_string())
}

impl Started {
    /// Waits for the program's main window: a visible top-level window of
    /// its own that no other window owns and that is not a tool window.
    /// Gives up — `None` — when the program has ended, when it has reached
    /// its message loop and still shows nothing a moment later, which is a
    /// tray application's shape, or at `within`. The window is returned as
    /// a plain integer so that it can cross to the thread that shows it.
    pub fn wait_for_window(&self, within: Duration) -> Option<isize> {
        let pid = self.pid?;
        let deadline = Instant::now() + within;
        let mut idle_since: Option<Instant> = None;
        loop {
            if let Some(window) = main_window_of(pid) {
                return Some(window as isize);
            }
            match &self.process {
                Some(process) => {
                    let handle = process.as_raw_handle() as HANDLE;
                    if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                        return None;
                    }
                    if idle_since.is_none() && unsafe { WaitForInputIdle(handle, 0) } == 0 {
                        idle_since = Some(Instant::now());
                    }
                }
                None => {
                    if !is_running(pid) {
                        return None;
                    }
                }
            }
            if idle_since.is_some_and(|since| since.elapsed() >= IDLE_GRACE)
                || Instant::now() >= deadline
            {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }
}

struct WindowSearch {
    pid: u32,
    found: HWND,
}

unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
    unsafe {
        let search = &mut *(lparam as *mut WindowSearch);
        let mut owner_pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut owner_pid);
        if owner_pid == search.pid
            && IsWindowVisible(hwnd) != 0
            && GetWindow(hwnd, GW_OWNER).is_null()
            && GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW == 0
        {
            search.found = hwnd;
            return 0;
        }
        1
    }
}

/// The main window of a process, as [`Started::wait_for_window`] means it.
pub fn main_window_of(pid: u32) -> Option<HWND> {
    let mut search = WindowSearch {
        pid,
        found: ptr::null_mut(),
    };
    unsafe { EnumWindows(Some(visit), &mut search as *mut WindowSearch as LPARAM) };
    (!search.found.is_null()).then_some(search.found)
}

/// Puts `window` in the foreground and says whether it is there. Called by
/// the process that holds the foreground — the wizard, on its own thread,
/// before it closes.
pub fn bring_to_foreground(window: isize) -> bool {
    let window = window as HWND;
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window);
        GetForegroundWindow() == window
    }
}

// The shell's automation objects. `windows-sys` declares no COM interfaces,
// so the vtables used here are declared by slot, checked against the
// Windows SDK headers: `IShellWindows::FindWindowSW` is slot 15,
// `IServiceProvider::QueryService` 3, `IShellBrowser::QueryActiveShellView`
// 15, `IShellView::GetItemObject` 15, `IShellFolderViewDual::get_Application`
// 7 and `IShellDispatch2::ShellExecute` 31.

type Slot = unsafe extern "system" fn();

#[repr(C)]
struct Vtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

type FindWindowSw = unsafe extern "system" fn(
    *mut c_void,
    *const VARIANT,
    *const VARIANT,
    i32,
    *mut i32,
    i32,
    *mut *mut c_void,
) -> HRESULT;
type QueryService =
    unsafe extern "system" fn(*mut c_void, *const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type QueryActiveShellView = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
type GetItemObject =
    unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT;
type GetApplication = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
type ShellExecute = unsafe extern "system" fn(
    *mut c_void,
    *const u16,
    VARIANT,
    VARIANT,
    VARIANT,
    VARIANT,
) -> HRESULT;

/// A COM interface pointer, released on drop.
struct Com(*mut c_void);

impl Com {
    /// The method in vtable slot `index`, as the type the caller names.
    unsafe fn slot<T: Copy>(&self, index: usize) -> T {
        unsafe {
            let vtable = *(self.0 as *const *const Slot);
            let entry = vtable.add(index);
            std::mem::transmute_copy::<Slot, T>(&*entry)
        }
    }

    fn query(&self, iid: &GUID, what: &str) -> Result<Com, String> {
        let mut out: *mut c_void = ptr::null_mut();
        let hr =
            unsafe { ((**(self.0 as *const *const Vtbl)).query_interface)(self.0, iid, &mut out) };
        checked(hr, out, what)
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { ((**(self.0 as *const *const Vtbl)).release)(self.0) };
    }
}

fn checked(hr: HRESULT, out: *mut c_void, what: &str) -> Result<Com, String> {
    if hr < 0 || out.is_null() {
        return Err(format!("{what} failed: HRESULT 0x{:08x}", hr as u32));
    }
    Ok(Com(out))
}

/// A COM apartment for the current thread, left on drop. Declared before
/// the interfaces it serves so that it is dropped after them.
struct Apartment {
    uninitialise: bool,
}

impl Apartment {
    fn enter() -> Result<Apartment, String> {
        let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Apartment {
                uninitialise: false,
            });
        }
        if hr < 0 {
            return Err(format!(
                "cannot initialise COM: HRESULT 0x{:08x}",
                hr as u32
            ));
        }
        Ok(Apartment { uninitialise: true })
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.uninitialise {
            unsafe { CoUninitialize() };
        }
    }
}

/// A `BSTR`, freed on drop.
struct Bstr(*const u16);

impl Bstr {
    fn new(text: &str) -> Bstr {
        let text = wide(text);
        Bstr(unsafe { SysAllocString(text.as_ptr()) })
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) };
        }
    }
}

fn variant_i4(value: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    variant.Anonymous.Anonymous.vt = VT_I4;
    variant.Anonymous.Anonymous.Anonymous.lVal = value;
    variant
}

/// A `VT_BSTR` that borrows `text`: the caller keeps the `Bstr` alive.
fn variant_bstr(text: &Bstr) -> VARIANT {
    let mut variant = VARIANT::default();
    variant.Anonymous.Anonymous.vt = VT_BSTR;
    variant.Anonymous.Anonymous.Anonymous.bstrVal = text.0;
    variant
}

fn variant_empty() -> VARIANT {
    let mut variant = VARIANT::default();
    variant.Anonymous.Anonymous.vt = VT_EMPTY;
    variant
}

/// Asks this desktop's Explorer to start the program: its `ShellWindows`
/// hands out the desktop window, whose top-level browser's active view
/// exposes `Shell.Application`, and `IShellDispatch2::ShellExecute` there
/// runs in Explorer, as Explorer.
fn shell_execute(program: &Path, arguments: &str, directory: &Path) -> Result<(), String> {
    let _apartment = Apartment::enter()?;
    let mut windows: *mut c_void = ptr::null_mut();
    let hr = unsafe {
        CoCreateInstance(
            &CLSID_SHELL_WINDOWS,
            ptr::null_mut(),
            CLSCTX_LOCAL_SERVER,
            &IID_ISHELL_WINDOWS,
            &mut windows,
        )
    };
    let windows = checked(hr, windows, "reaching the desktop shell (ShellWindows)")?;

    let location = variant_i4(CSIDL_DESKTOP);
    let root = variant_empty();
    let mut desktop_hwnd = 0i32;
    let mut desktop: *mut c_void = ptr::null_mut();
    let hr = unsafe {
        windows.slot::<FindWindowSw>(15)(
            windows.0,
            &location,
            &root,
            SWC_DESKTOP,
            &mut desktop_hwnd,
            SWFO_NEEDDISPATCH,
            &mut desktop,
        )
    };
    let desktop = checked(hr, desktop, "finding the desktop window")?;
    let provider = desktop.query(
        &IID_ISERVICE_PROVIDER,
        "asking the desktop for its services",
    )?;

    let mut browser: *mut c_void = ptr::null_mut();
    let hr = unsafe {
        provider.slot::<QueryService>(3)(
            provider.0,
            &SID_STOP_LEVEL_BROWSER,
            &IID_ISHELL_BROWSER,
            &mut browser,
        )
    };
    let browser = checked(hr, browser, "reaching the desktop's browser")?;

    let mut view: *mut c_void = ptr::null_mut();
    let hr = unsafe { browser.slot::<QueryActiveShellView>(15)(browser.0, &mut view) };
    let view = checked(hr, view, "reaching the desktop's view")?;

    let mut background: *mut c_void = ptr::null_mut();
    let hr = unsafe {
        view.slot::<GetItemObject>(15)(view.0, SVGIO_BACKGROUND, &IID_IDISPATCH, &mut background)
    };
    let background = checked(hr, background, "reaching the desktop view's automation")?;
    let folder_view = background.query(&IID_ISHELL_FOLDER_VIEW_DUAL, "reaching the folder view")?;

    let mut application: *mut c_void = ptr::null_mut();
    let hr = unsafe { folder_view.slot::<GetApplication>(7)(folder_view.0, &mut application) };
    let application = checked(hr, application, "reaching Shell.Application")?;
    let dispatch = application.query(&IID_ISHELL_DISPATCH2, "reaching IShellDispatch2")?;

    let file = Bstr::new(&program.display().to_string());
    let arguments = Bstr::new(arguments);
    let directory = Bstr::new(&directory.display().to_string());
    let verb = Bstr::new("open");
    let hr = unsafe {
        dispatch.slot::<ShellExecute>(31)(
            dispatch.0,
            file.0,
            variant_bstr(&arguments),
            variant_bstr(&directory),
            variant_bstr(&verb),
            variant_i4(SW_SHOWNORMAL),
        )
    };
    if hr < 0 {
        return Err(format!(
            "the desktop shell could not start {}: HRESULT 0x{:08x}",
            program.display(),
            hr as u32
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system32(file: &str) -> PathBuf {
        PathBuf::from(std::env::var("SystemRoot").unwrap())
            .join("System32")
            .join(file)
    }

    /// This process's own token reads the way `whoami /groups` would put
    /// it, and the answer matches the elevation check the rest of the
    /// engine uses wherever User Account Control is on.
    #[test]
    fn this_process_token_is_read() {
        let mut token: HANDLE = ptr::null_mut();
        assert_ne!(
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) },
            0
        );
        let token = unsafe { OwnedHandle::from_raw_handle(token as _) };
        let elevated = token_is_elevated(token.as_raw_handle() as HANDLE);
        assert!(elevated.is_some());
        if super::super::process::is_elevated() {
            assert_eq!(elevated, Some(true));
        }
    }

    /// A program started with this process's token runs in the directory it
    /// was given, and a windowless console program is reported as having no
    /// window once it has ended. (`cmd.exe` parses its own command line, so
    /// the arguments here need no quoting; exact argument passing is proven
    /// with a C-runtime program by the process-level launch tests.)
    #[test]
    fn an_unelevated_start_runs_in_the_directory_it_was_given() {
        if this_process_is_elevated() {
            eprintln!("SKIPPED: this test starts a program with its own, unelevated token");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        let arguments: Vec<String> = ["/d", "/c", "cd>out.txt"]
            .iter()
            .map(|a| a.to_string())
            .collect();
        let started = start(&system32("cmd.exe"), &arguments, dir.path()).unwrap();
        assert_eq!(started.method, Method::OwnToken);
        assert!(started.pid.is_some());
        assert_eq!(started.wait_for_window(Duration::from_secs(20)), None);
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(
            same_path(Path::new(written.trim()), dir.path()),
            "{written:?} vs {}",
            dir.path().display()
        );
    }

    #[test]
    fn a_program_that_cannot_start_is_a_failure_not_an_unavailability() {
        if this_process_is_elevated() {
            eprintln!("SKIPPED: this test starts a program with its own, unelevated token");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let not_a_program = dir.path().join("app.exe");
        std::fs::write(&not_a_program, b"not a program").unwrap();
        assert!(matches!(
            start(&not_a_program, &[], dir.path()),
            Err(StartError::Failed(_))
        ));
        assert!(matches!(
            start(&dir.path().join("missing.exe"), &[], dir.path()),
            Err(StartError::Failed(_))
        ));
    }
}
