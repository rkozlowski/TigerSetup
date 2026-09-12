//! Standard output for a windowed program.
//!
//! The installer is a GUI-subsystem executable so that a double-click opens
//! a wizard and never a console window. Windows therefore gives it no
//! console of its own, and a command such as `Setup.exe inspect --json` run
//! from a terminal would have nowhere to write.
//!
//! Two cases have to keep working and they are distinguished by the handles
//! the process was started with:
//!
//! * **Redirected** — a pipe or a file, which is how the tests, the lab and
//!   a package manager run the installer. The handles are already valid;
//!   nothing is done and the bytes go where the caller asked.
//! * **A terminal** — the parent has a console but did not hand one over.
//!   The process attaches to the parent's console and re-opens standard
//!   output and standard error on it.
//!
//! One consequence is inherent to a GUI-subsystem program and is not a
//! defect: the shell does not wait for it, so the prompt returns before the
//! output appears. Every GUI-subsystem installer behaves this way.

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    SetStdHandle,
};

fn missing(handle: HANDLE) -> bool {
    handle.is_null() || handle == INVALID_HANDLE_VALUE
}

fn standard_handle(id: u32) -> HANDLE {
    unsafe { GetStdHandle(id) }
}

/// Gives this process somewhere to print when it was started from a terminal
/// rather than with redirected output. Does nothing when output is already
/// redirected, and nothing when there is no parent console.
pub fn attach_to_parent() {
    let out_missing = missing(standard_handle(STD_OUTPUT_HANDLE));
    let error_missing = missing(standard_handle(STD_ERROR_HANDLE));
    if !out_missing && !error_missing {
        return;
    }
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }
    if out_missing && let Some(handle) = open_console() {
        unsafe { SetStdHandle(STD_OUTPUT_HANDLE, handle) };
    }
    if error_missing && let Some(handle) = open_console() {
        unsafe { SetStdHandle(STD_ERROR_HANDLE, handle) };
    }
}

/// Opens the attached console's screen buffer.
fn open_console() -> Option<HANDLE> {
    let name: Vec<u16> = "CONOUT$".encode_utf16().chain(std::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    (!missing(handle)).then_some(handle)
}
