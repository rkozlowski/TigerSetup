//! `TigerSetupTestLaunch.exe`: the program the launch-after-install tests
//! offer on the completion page (`TigerSetup-Design.md` §11.7). It stands in
//! for an application a person would start, so that what the wizard does —
//! which token it starts it with, which arguments reach it, where it runs,
//! and whether its window ends up in front — is observed by the program
//! itself rather than guessed from outside.
//!
//! ```text
//! TigerSetupTestLaunch.exe --report <path> [--observe-ms <n>] [--exit-after-ms <n>]
//!                          [--] <anything>...
//! ```
//!
//! It opens one ordinary top-level window, titled `TigerSetupTestLaunch`,
//! and never asks for the foreground itself: whether it gets it is exactly
//! what is being tested. For `--observe-ms` (5000 by default) it samples the
//! foreground window every 50 ms, then writes one JSON document to
//! `--report` (its directories created, written beside and renamed into
//! place so a reader never sees half of it):
//!
//! ```text
//! pid, parent_pid, command_line (raw), arguments (every argument after the
//! program, as the C runtime split them), working_directory, user,
//! elevated, elevation_type (default | full | limited), integrity_rid,
//! administrators_enabled, window_shown, foreground (reached it while
//! observed), foreground_after_ms, foreground_at_end
//! ```
//!
//! It then stays up until its window is closed, or until `--exit-after-ms`
//! from its start. Everything else on the command line, `--` included, is
//! only recorded, so a test can pass arbitrary text and compare it exactly.

#![windows_subsystem = "windows"]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use serde_json::json;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH, UpdateWindow};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, EqualSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    LookupAccountSidW, SECURITY_MAX_SID_SIZE, SID_NAME_USE, TOKEN_ELEVATION, TOKEN_ELEVATION_TYPE,
    TOKEN_GROUPS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER, TokenElevation,
    TokenElevationType, TokenGroups, TokenIntegrityLevel, TokenUser, WinBuiltinAdministratorsSid,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Environment::GetCommandLineW;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::{SE_GROUP_ENABLED, SE_GROUP_USE_FOR_DENY_ONLY};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcessToken,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetForegroundWindow, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassW,
    SW_SHOWDEFAULT, ShowWindow, TranslateMessage, WM_DESTROY, WM_QUIT, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};

const TITLE: &str = "TigerSetupTestLaunch";
const SAMPLE: Duration = Duration::from_millis(50);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Options {
    report: Option<PathBuf>,
    observe: Duration,
    exit_after: Option<Duration>,
}

fn options(arguments: &[String]) -> Options {
    let mut options = Options {
        report: None,
        observe: Duration::from_millis(5000),
        exit_after: None,
    };
    let mut i = 0;
    while i < arguments.len() {
        let value = arguments.get(i + 1);
        match (arguments[i].as_str(), value) {
            ("--", _) => break,
            ("--report", Some(value)) => options.report = Some(PathBuf::from(value)),
            ("--observe-ms", Some(value)) => {
                options.observe = Duration::from_millis(value.parse().unwrap_or(5000))
            }
            ("--exit-after-ms", Some(value)) => {
                options.exit_after = value.parse().ok().map(Duration::from_millis)
            }
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    options
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_DESTROY {
            PostQuitMessage(0);
            return 0;
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

fn create_window() -> HWND {
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let class = wide(TITLE);
        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&window_class);
        let title = wide(TITLE);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            480,
            240,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if !hwnd.is_null() {
            // SW_SHOWDEFAULT follows the STARTUPINFO the starter gave, as a
            // real application's first window does. Nothing here asks for
            // the foreground.
            ShowWindow(hwnd, SW_SHOWDEFAULT);
            UpdateWindow(hwnd);
        }
        hwnd
    }
}

/// Pumps this thread's messages until `until`, or until the window is gone.
/// Returns false once a quit message was seen.
fn pump_until(until: Instant) -> bool {
    unsafe {
        let mut message: MSG = std::mem::zeroed();
        let mut running = true;
        while Instant::now() < until {
            while PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    running = false;
                    break;
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if !running {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        running
    }
}

fn token_information(token: HANDLE, class: i32) -> Option<Vec<u64>> {
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

/// What this process's token says about who it runs as and how.
fn token_facts() -> serde_json::Value {
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return json!({ "token_error": std::io::Error::last_os_error().to_string() });
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let mut kind: TOKEN_ELEVATION_TYPE = 0;
        GetTokenInformation(
            token,
            TokenElevationType,
            &mut kind as *mut TOKEN_ELEVATION_TYPE as *mut c_void,
            size_of::<TOKEN_ELEVATION_TYPE>() as u32,
            &mut returned,
        );
        let integrity = token_information(token, TokenIntegrityLevel).map(|label| {
            let label = &*(label.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            let count = *GetSidSubAuthorityCount(label.Label.Sid);
            match count {
                0 => 0,
                n => *GetSidSubAuthority(label.Label.Sid, n as u32 - 1),
            }
        });
        let mut administrators = [0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut size = administrators.len() as u32;
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            ptr::null_mut(),
            administrators.as_mut_ptr() as *mut c_void,
            &mut size,
        );
        let administrators_enabled = token_information(token, TokenGroups).map(|groups| {
            let groups = &*(groups.as_ptr() as *const TOKEN_GROUPS);
            std::slice::from_raw_parts(groups.Groups.as_ptr(), groups.GroupCount as usize)
                .iter()
                .any(|group| {
                    EqualSid(group.Sid, administrators.as_mut_ptr() as *mut c_void) != 0
                        && group.Attributes & SE_GROUP_ENABLED as u32 != 0
                        && group.Attributes & SE_GROUP_USE_FOR_DENY_ONLY as u32 == 0
                })
        });
        let user = token_information(token, TokenUser).and_then(|user| {
            let user = &*(user.as_ptr() as *const TOKEN_USER);
            let mut name = [0u16; 256];
            let mut domain = [0u16; 256];
            let mut name_length = name.len() as u32;
            let mut domain_length = domain.len() as u32;
            let mut sid_use: SID_NAME_USE = 0;
            (LookupAccountSidW(
                ptr::null(),
                user.User.Sid,
                name.as_mut_ptr(),
                &mut name_length,
                domain.as_mut_ptr(),
                &mut domain_length,
                &mut sid_use,
            ) != 0)
                .then(|| {
                    format!(
                        "{}\\{}",
                        String::from_utf16_lossy(&domain[..domain_length as usize]),
                        String::from_utf16_lossy(&name[..name_length as usize])
                    )
                })
        });
        CloseHandle(token);
        json!({
            "user": user,
            "elevated": elevation.TokenIsElevated != 0,
            "elevation_type": match kind {
                2 => "full",
                3 => "limited",
                _ => "default",
            },
            "integrity_rid": integrity,
            "administrators_enabled": administrators_enabled,
        })
    }
}

fn parent_pid() -> Option<u32> {
    let own = unsafe { GetCurrentProcessId() };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry);
        let mut parent = None;
        while more != 0 {
            if entry.th32ProcessID == own {
                parent = Some(entry.th32ParentProcessID);
                break;
            }
            more = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        parent
    }
}

fn command_line() -> String {
    unsafe {
        let line = GetCommandLineW();
        let mut length = 0;
        while *line.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(line, length))
    }
}

/// Written beside and renamed into place, so a reader polling for the file
/// never reads half of it.
fn write_report(path: &Path, report: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let partial = path.with_extension("partial");
    if std::fs::write(
        &partial,
        serde_json::to_string_pretty(report).unwrap_or_default(),
    )
    .is_ok()
    {
        let _ = std::fs::rename(&partial, path);
    }
}

fn main() {
    let started = Instant::now();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let options = options(&arguments);
    let window = create_window();

    // The foreground, sampled while observed; the program never asks for it.
    let mut foreground_after = None;
    let mut running = true;
    let observe_until = started + options.observe;
    while running && Instant::now() < observe_until {
        if foreground_after.is_none()
            && !window.is_null()
            && unsafe { GetForegroundWindow() } == window
        {
            foreground_after = Some(started.elapsed().as_millis() as u64);
        }
        running = pump_until(Instant::now() + SAMPLE);
    }
    let foreground_at_end = !window.is_null() && unsafe { GetForegroundWindow() } == window;

    if let Some(report) = &options.report {
        let mut document = json!({
            "pid": unsafe { GetCurrentProcessId() },
            "parent_pid": parent_pid(),
            "command_line": command_line(),
            "arguments": arguments,
            "working_directory": std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            "window_shown": !window.is_null(),
            "foreground": foreground_after.is_some(),
            "foreground_after_ms": foreground_after,
            "foreground_at_end": foreground_at_end,
        });
        if let (Some(target), serde_json::Value::Object(token)) =
            (document.as_object_mut(), token_facts())
        {
            target.extend(token);
        }
        write_report(report, &document);
    }

    while running {
        let until = match options.exit_after {
            Some(after) => started + after,
            None => Instant::now() + Duration::from_secs(3600),
        };
        running = pump_until(until) && options.exit_after.is_none();
    }
}
