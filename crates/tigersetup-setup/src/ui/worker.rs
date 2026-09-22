//! The engine runs on its own thread; the window thread only draws.
//!
//! The sink turns every engine event the wizard can show into a posted
//! window message and hands ownership of the boxed event to the window
//! thread, which frees it. That keeps the engine free of any knowledge of
//! the UI and keeps the message loop responsive while a run is in progress:
//! the window stays paintable, and Cancel stays clickable.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tigersetup_engine::report::{Answer, Event, EventSink, Outcome, Progress, Question, exit};
use tigersetup_engine::{Package, RunOptions};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, SendMessageW, WM_APP};

use crate::Operation;

/// One engine event, owned by the window thread once it arrives.
pub const WM_ENGINE_EVENT: u32 = WM_APP + 1;
/// The run finished; the outcome document comes with it.
pub const WM_ENGINE_DONE: u32 = WM_APP + 2;

/// Sent, not posted: the engine thread is waiting for the answer. `lparam`
/// carries a [`QuestionRequest`] the window thread reads and answers in
/// place; the reply is the message's return value, non-zero to go ahead.
pub const WM_ENGINE_QUESTION: u32 = WM_APP + 3;

/// An elevated relaunch finished; what the elevated child reported comes with
/// it. Posted from the elevation thread so the window thread never blocks the
/// message pump waiting for the child (`elevate`).
pub const WM_ELEVATION_DONE: u32 = WM_APP + 4;
/// The prompt was answered and the elevated child is running: from here on
/// the child shows the wizard, and this window steps aside for it. Never
/// posted for a refused or failed prompt, which comes as
/// [`WM_ELEVATION_DONE`] alone.
pub const WM_ELEVATION_STARTED: u32 = WM_APP + 5;
/// The launch after install has started the program and its window has
/// appeared, or it has been given up on; a [`LaunchDone`] comes with it.
pub const WM_LAUNCH_DONE: u32 = WM_APP + 6;

/// What the launch after install did, owned by the window thread once it
/// arrives. The window, when there is one, is the program's main window as
/// a plain integer: the window thread puts it in the foreground.
pub struct LaunchDone {
    pub info: tigersetup_engine::report::LaunchInfo,
    pub window: Option<isize>,
}

/// The result of an elevated relaunch, owned by the window thread once it
/// arrives: either what the elevated child reported, or why no prompt could
/// be answered.
pub struct ElevationDone {
    pub result: tigersetup_engine::Result<tigersetup_engine::elevation::Elevated>,
}

/// A question put to the person, crossing to the window thread by pointer
/// because the asking thread is blocked until it returns.
pub struct QuestionRequest {
    /// The applications holding the product's files, already described.
    pub holders: Vec<String>,
}

/// What the window needs from an engine event.
pub struct EngineEvent {
    pub code: &'static str,
    pub progress: Option<Progress>,
}

/// Events without progress that still move the status line. Everything else
/// goes to the log only, so a run over thousands of files posts about one
/// message per operation rather than several.
const SHOWN_CODES: &[&str] = &[
    "recovery_started",
    "transaction_rolling_back",
    "transaction_committed",
];

struct WindowSink {
    /// The window handle as an integer, because a raw pointer is not `Send`
    /// and a handle is not a pointer to anything this thread owns.
    hwnd: isize,
}

impl EventSink for WindowSink {
    /// Puts the engine's question to the person. `SendMessageW` blocks this
    /// thread while the window thread shows the box, which is exactly what
    /// is wanted: the engine must not touch the machine until it is
    /// answered. A window that has gone away answers for the unattended
    /// case, which is what the trait's own default does.
    fn question(&mut self, question: &Question<'_>) -> Answer {
        let Question::CloseApplications(holders) = question;
        let request = QuestionRequest {
            holders: holders.iter().map(|holder| holder.name.clone()).collect(),
        };
        let answered = unsafe {
            SendMessageW(
                self.hwnd as HWND,
                WM_ENGINE_QUESTION,
                0,
                &request as *const QuestionRequest as isize,
            )
        };
        match answered != 0 {
            true => Answer::Close,
            false => Answer::Cancel,
        }
    }

    fn event(&mut self, event: &Event) {
        if event.progress.is_none() && !SHOWN_CODES.contains(&event.code) {
            return;
        }
        let boxed = Box::new(EngineEvent {
            code: event.code,
            progress: event.progress.clone(),
        });
        let raw = Box::into_raw(boxed);
        let posted =
            unsafe { PostMessageW(self.hwnd as HWND, WM_ENGINE_EVENT, 0, raw as isize) != 0 };
        if !posted {
            // The window is gone; nobody will free this.
            drop(unsafe { Box::from_raw(raw) });
        }
    }
}

/// Starts `operation` on a worker thread. The thread opens its own view of
/// the package — the metadata block only, no payload bytes are read until
/// the transaction needs them — so that nothing has to be shared with the
/// window thread but the handle and the cancellation flag.
pub fn start(
    hwnd: HWND,
    exe: PathBuf,
    operation: Operation,
    options: RunOptions,
) -> std::thread::JoinHandle<()> {
    let hwnd = hwnd as isize;
    std::thread::spawn(move || {
        let mut sink = WindowSink { hwnd };
        let outcome = match Package::open(&exe) {
            Ok(package) => match operation {
                Operation::Uninstall => tigersetup_engine::uninstall(&package, &options, &mut sink),
                Operation::Repair => tigersetup_engine::repair(&package, &options, &mut sink),
                _ => tigersetup_engine::install(&package, &options, &mut sink),
            },
            Err(err) => Outcome::new("failed", err.code, err.exit_code(), err.message),
        };
        let raw = Box::into_raw(Box::new(outcome));
        let posted = unsafe { PostMessageW(hwnd as HWND, WM_ENGINE_DONE, 0, raw as isize) != 0 };
        if !posted {
            drop(unsafe { Box::from_raw(raw) });
        }
    })
}

/// Runs the relaunch — elevated through the prompt, or plainly with this
/// process's own token — on its own thread, posts [`WM_ELEVATION_STARTED`]
/// once an elevated child is running, and posts [`WM_ELEVATION_DONE`] when
/// it finishes. The window thread keeps pumping messages the whole time,
/// so the wizard stays responsive while the UAC prompt is up and while the
/// child does the work — the message loop is never blocked on the child
/// (`process::run_elevated` waits on it here, off the window thread).
pub fn start_relaunch(
    hwnd: HWND,
    exe: PathBuf,
    arguments: Vec<String>,
    elevated: bool,
) -> std::thread::JoinHandle<()> {
    let hwnd = hwnd as isize;
    std::thread::spawn(move || {
        let result = if elevated {
            tigersetup_engine::elevation::relaunch_elevated_observed(&exe, &arguments, || {
                unsafe { PostMessageW(hwnd as HWND, WM_ELEVATION_STARTED, 0, 0) };
            })
        } else {
            tigersetup_engine::elevation::relaunch_plain(&exe, &arguments)
        };
        let raw = Box::into_raw(Box::new(ElevationDone { result }));
        let posted = unsafe { PostMessageW(hwnd as HWND, WM_ELEVATION_DONE, 0, raw as isize) != 0 };
        if !posted {
            // The window is gone; nobody will free this.
            drop(unsafe { Box::from_raw(raw) });
        }
    })
}

/// Starts the program the completion page offered, as the signed-in user,
/// on its own thread — through the shell this is a COM conversation with
/// Explorer, and either way it waits for the program's window — and posts
/// [`WM_LAUNCH_DONE`]. The window thread keeps pumping messages, and
/// keeps the foreground, the whole time.
pub fn start_launch(
    hwnd: HWND,
    target: tigersetup_engine::launch::Target,
) -> std::thread::JoinHandle<()> {
    let hwnd = hwnd as isize;
    std::thread::spawn(move || {
        let (info, started) = target.start();
        let window = started
            .and_then(|started| started.wait_for_window(tigersetup_engine::launch::WINDOW_WITHIN));
        let raw = Box::into_raw(Box::new(LaunchDone { info, window }));
        let posted = unsafe { PostMessageW(hwnd as HWND, WM_LAUNCH_DONE, 0, raw as isize) != 0 };
        if !posted {
            // The window is gone; nobody will free this.
            drop(unsafe { Box::from_raw(raw) });
        }
    })
}

/// The outcome a wizard reports when the engine could not even be started.
pub fn start_failed(message: String) -> Outcome {
    Outcome::new("failed", "engine_unavailable", exit::ROLLED_BACK, message)
}

/// A fresh cancellation flag for one run.
pub fn cancel_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
