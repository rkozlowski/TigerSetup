//! Closing and restarting the applications that hold a product's files,
//! through the Restart Manager Windows already has
//! (`TigerSetup-Design.md` §5.10). There is no TigerSetup-specific shutdown
//! protocol: an application saves and restores its own state, and comes
//! back afterwards exactly when it asked Windows to by calling
//! `RegisterApplicationRestart`.
//!
//! The sequence around a transaction that replaces or removes files is
//! always the same: start a session, register every file the plan touches,
//! ask who holds them, close them if anyone does, run the transaction,
//! then restart what was closed and end the session. Closing happens
//! *before* the transaction opens, so a run that cannot free the files has
//! mutated nothing at all.
//!
//! `RmShutdown` is always asked for a graceful shutdown — never
//! `RmForceShutdown`. What Windows does with a holder that has no message
//! loop is Windows' business: a console process is terminated rather than
//! asked. Either way the only two acceptable ends are "the files were freed
//! and the transaction ran" and "`package_in_use`, nothing touched".

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, ERROR_SUCCESS, GetLastError,
    WAIT_OBJECT_0, WIN32_ERROR,
};
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
use windows_sys::Win32::System::RestartManager::{
    CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmConsole, RmCritical, RmEndSession, RmExplorer,
    RmGetList, RmMainWindow, RmOtherWindow, RmRegisterResources, RmRestart, RmService, RmShutdown,
    RmStartSession,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
};

use crate::report::{Answer, HolderInfo, HolderKind, Question, Reporter};
use crate::{Error, Result};

/// The failure a run reports when the files could not be freed.
pub const IN_USE_CODE: &str = "package_in_use";

/// How long a holder asked to close is given to finish closing before the run
/// concludes that it will not, when Windows closes it by messaging its windows.
///
/// That is the request that works, so a windowed application is given real
/// time to answer it: it may have a document to save, a window to tear down
/// and, where it hosts one, a browser runtime to end with it, and reporting
/// `package_in_use` against an application that was doing exactly what it was
/// asked is a failed upgrade for no reason. The bound is generous because
/// nothing is spent reaching it — the holders are checked four times a second
/// and the wait ends the moment they are gone, which for a co-operating
/// application is a fraction of a second.
const WINDOWED_SHUTDOWN_GRACE: Duration = Duration::from_secs(60);

/// The same bound for a holder the Restart Manager found no window for.
///
/// A console process with no message loop, or one Windows could not classify
/// at all, is asked in a way it may never answer, so a long wait only delays
/// the `package_in_use` the run is going to report anyway. It is not zero,
/// because a console application *can* answer the control event it is sent and
/// exit.
const OTHER_SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// How long these holders are given, which is decided by what Windows can ask
/// of them rather than by one number for every kind of holder.
fn shutdown_grace(holders: &[HolderInfo]) -> Duration {
    let windowed = holders.iter().any(|holder| {
        matches!(
            holder.kind,
            HolderKind::MainWindow | HolderKind::OtherWindow
        )
    });
    if windowed {
        WINDOWED_SHUTDOWN_GRACE
    } else {
        OTHER_SHUTDOWN_GRACE
    }
}

fn failed(what: &str, status: WIN32_ERROR) -> Error {
    Error::new(
        "restart_manager_failed",
        format!(
            "the Restart Manager could not {what}: {}",
            std::io::Error::from_raw_os_error(status as i32)
        ),
    )
}

/// An open Restart Manager session. Ending it is what releases the
/// applications Windows is holding on behalf of this installation, so it
/// happens on every path out, including a panic-free early return.
pub struct Session {
    handle: u32,
    /// Files registered so far, for the log.
    registered: usize,
}

impl Session {
    pub fn start() -> Result<Session> {
        let mut handle: u32 = 0;
        let mut key = vec![0u16; CCH_RM_SESSION_KEY as usize + 1];
        let status = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
        if status != ERROR_SUCCESS {
            return Err(failed("start a session", status));
        }
        Ok(Session {
            handle,
            registered: 0,
        })
    }

    /// Registers the absolute paths whose holders this session is about.
    pub fn register(&mut self, files: &[PathBuf]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let wide: Vec<Vec<u16>> = files
            .iter()
            .map(|file| {
                file.display()
                    .to_string()
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect()
            })
            .collect();
        let pointers: Vec<*const u16> = wide.iter().map(|f| f.as_ptr()).collect();
        let status = unsafe {
            RmRegisterResources(
                self.handle,
                pointers.len() as u32,
                pointers.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(failed("register the files", status));
        }
        self.registered += files.len();
        Ok(())
    }

    pub fn registered(&self) -> usize {
        self.registered
    }

    /// Who holds the registered files, this process excluded — an installer
    /// never asks Windows to close the installer.
    pub fn holders(&self) -> Result<Vec<HolderInfo>> {
        let mut capacity: u32 = 0;
        let mut infos: Vec<RM_PROCESS_INFO> = Vec::new();
        // The list can grow between the two calls, so ask again while
        // Windows says the buffer was too small.
        for _ in 0..8 {
            let mut needed: u32 = 0;
            let mut count = capacity;
            let mut reasons: u32 = 0;
            let status = unsafe {
                RmGetList(
                    self.handle,
                    &mut needed,
                    &mut count,
                    if capacity == 0 {
                        std::ptr::null_mut()
                    } else {
                        infos.as_mut_ptr()
                    },
                    &mut reasons,
                )
            };
            match status {
                ERROR_SUCCESS => {
                    infos.truncate(count as usize);
                    return Ok(describe(&infos));
                }
                ERROR_MORE_DATA => {
                    capacity = needed.max(1);
                    infos = vec![RM_PROCESS_INFO::default(); capacity as usize];
                }
                other => return Err(failed("list the applications", other)),
            }
        }
        Err(Error::new(
            "restart_manager_failed",
            "the list of applications holding the files kept growing",
        ))
    }

    /// Asks the holders to close, gracefully. Applications that registered
    /// for restart are remembered by Windows until [`Session::restart`].
    pub fn shutdown(&self) -> Result<()> {
        let status = unsafe { RmShutdown(self.handle, 0, None) };
        if status != ERROR_SUCCESS {
            return Err(failed("close the applications", status));
        }
        Ok(())
    }

    /// Waits, briefly, for `holders` to actually go away.
    ///
    /// A graceful shutdown is a request, and a request takes time to honour:
    /// an application asked to close has a window to answer, work to save and
    /// a process to end, and `RmShutdown` can return before any of that has
    /// finished. Concluding immediately therefore reports an application that
    /// is in the middle of doing exactly what it was asked to do as one that
    /// refused, and the run fails with `package_in_use` against a co-operating
    /// application.
    ///
    /// **The question is asked of the machine, not of the Restart Manager.**
    /// `RmGetList` answers with the applications the session was told about
    /// when the resources were registered, and it keeps naming one that has
    /// already exited — so a wait that ends when *that* list empties never
    /// ends, and every holder becomes `package_in_use` however promptly it
    /// closed. Whether a holder is still there is a question about its
    /// process, and it is asked by the identity the Restart Manager itself
    /// uses: the process id together with the moment it started, so an id
    /// reused after the holder exits cannot be mistaken for it.
    ///
    /// The bound is short because this is the pleasant path, not the safe one:
    /// what makes the transaction safe is the journal. A holder that is still
    /// there at the end is reported and the run stops before it mutates
    /// anything, which is the same answer as before — only now it is the
    /// answer to a question the application was given time to answer.
    pub fn wait_for_holders_to_go(
        &self,
        holders: &[HolderInfo],
        within: Duration,
    ) -> Result<Vec<HolderInfo>> {
        const POLL: Duration = Duration::from_millis(250);
        let deadline = Instant::now() + within;
        loop {
            let remaining = still_running(holders);
            if remaining.is_empty() || Instant::now() >= deadline {
                return Ok(remaining);
            }
            std::thread::sleep(POLL);
        }
    }

    /// Restarts what was closed. Best effort by design: an application that
    /// does not come back is reported, never a reason to fail a committed
    /// installation.
    pub fn restart(&self) -> Result<()> {
        let status = unsafe { RmRestart(self.handle, 0, None) };
        if status != ERROR_SUCCESS {
            return Err(failed("restart the applications", status));
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { RmEndSession(self.handle) };
    }
}

fn describe(infos: &[RM_PROCESS_INFO]) -> Vec<HolderInfo> {
    let self_id = std::process::id();
    infos
        .iter()
        .filter(|info| info.Process.dwProcessId != self_id)
        .map(|info| {
            let length = info
                .strAppName
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(info.strAppName.len());
            HolderInfo {
                name: String::from_utf16_lossy(&info.strAppName[..length]),
                process_id: info.Process.dwProcessId,
                restartable: info.bRestartable != 0,
                kind: classify(info.ApplicationType),
                started_at: (u64::from(info.Process.ProcessStartTime.dwHighDateTime) << 32)
                    | u64::from(info.Process.ProcessStartTime.dwLowDateTime),
            }
        })
        .collect()
}

/// Those of `holders` whose process is still running.
///
/// A process id alone is not an identity — Windows reuses one as soon as the
/// process that had it is gone — so a holder is the same holder only while the
/// process wearing its id also started when it did. A handle that cannot be
/// opened because this run may not look at that process is *not* evidence that
/// it ended: an unelevated run has no business concluding that another
/// account's application has closed, so it keeps waiting for it and reports it
/// if it never does.
fn still_running(holders: &[HolderInfo]) -> Vec<HolderInfo> {
    holders
        .iter()
        .filter(|holder| is_running(holder))
        .cloned()
        .collect()
}

fn is_running(holder: &HolderInfo) -> bool {
    let access = PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE;
    let process = unsafe { OpenProcess(access, 0, holder.process_id) };
    if process.is_null() {
        // Only "there is no such process" says the holder has gone. Anything
        // else — access denied above all — means this run cannot tell, and
        // "cannot tell" must not read as "closed".
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    }
    let signalled = unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0;
    let same_process = holder.started_at == 0 || started_at(process) == holder.started_at;
    unsafe { CloseHandle(process) };
    !signalled && same_process
}

/// When a process started, as the 100-nanosecond value Windows keeps, or `0`
/// where it will not say.
fn started_at(process: windows_sys::Win32::Foundation::HANDLE) -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;

    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    let ok = unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) };
    if ok == 0 {
        return 0;
    }
    (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime)
}

/// What the Restart Manager decided a holder is.
///
/// The classification is the Restart Manager's own and is made when the list is
/// taken, so it answers a question no other evidence does: a holder listed as
/// having no window is one a graceful request cannot reach, however long it is
/// given, and that is a different failure from an application that was asked
/// and refused.
fn classify(application_type: i32) -> HolderKind {
    // Compared rather than matched: the Windows constants are not upper case,
    // and a lower-case name in a pattern binds a new variable instead of
    // matching the constant it looks like.
    if application_type == RmMainWindow {
        HolderKind::MainWindow
    } else if application_type == RmOtherWindow {
        HolderKind::OtherWindow
    } else if application_type == RmService {
        HolderKind::Service
    } else if application_type == RmExplorer {
        HolderKind::Explorer
    } else if application_type == RmConsole {
        HolderKind::Console
    } else if application_type == RmCritical {
        HolderKind::Critical
    } else {
        HolderKind::Unknown
    }
}

/// `<name> (<pid>, <what Windows can do with it>)` for every holder, for a log
/// line and a message.
pub fn describe_holders(holders: &[HolderInfo]) -> String {
    holders
        .iter()
        .map(|h| format!("{} ({}, {})", h.name, h.process_id, h.kind.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The session a run keeps around its transaction: `None` when the plan
/// replaces or removes nothing, so an ordinary first install never starts
/// one.
pub struct Quiescence {
    session: Option<Session>,
    closed: Vec<HolderInfo>,
}

impl Quiescence {
    /// No session: nothing the run does can collide with a running
    /// application.
    pub fn none() -> Quiescence {
        Quiescence {
            session: None,
            closed: Vec::new(),
        }
    }

    /// Opens a session, registers `files`, and closes whoever holds them.
    ///
    /// A quiet run closes them without asking; an interactive one asks the
    /// sink first and reports `cancelled` when the answer is to stop. The
    /// files are freed before this returns, so the caller may open its
    /// transaction.
    pub fn acquire(
        files: &[PathBuf],
        quiet: bool,
        reporter: &mut Reporter<'_>,
    ) -> Result<Quiescence> {
        if files.is_empty() {
            return Ok(Quiescence::none());
        }
        // Quiescence is coordination, not a precondition. A Restart Manager
        // that will not start is a reason to proceed without it — a file
        // genuinely held open then fails its own operation and the
        // transaction rolls back, which is recoverable. Refusing to install
        // because the service is unavailable is not, and it is likeliest
        // right after an unclean boot, which is exactly when a recovery run
        // happens.
        let mut session = match Session::start() {
            Ok(session) => session,
            Err(err) => {
                reporter.event("restart_manager_unavailable", err.message);
                return Ok(Quiescence::none());
            }
        };
        if let Err(err) = session.register(files) {
            reporter.event("restart_manager_unavailable", err.message);
            return Ok(Quiescence::none());
        }
        // Same reasoning: a list that cannot be read is not a reason to
        // refuse the run.
        let holders = match session.holders() {
            Ok(holders) => holders,
            Err(err) => {
                reporter.event("restart_manager_unavailable", err.message);
                return Ok(Quiescence::none());
            }
        };
        if holders.is_empty() {
            return Ok(Quiescence {
                session: Some(session),
                closed: Vec::new(),
            });
        }
        reporter.event("restart_manager_holders", describe_holders(&holders));
        if !quiet && reporter.ask(&Question::CloseApplications(&holders)) == Answer::Cancel {
            return Err(Error::new(
                "cancelled",
                format!(
                    "{} is in use and was left running",
                    describe_holders(&holders)
                ),
            ));
        }
        // Whether the run may go on is decided by who still holds the
        // files, never by what the call returned. Windows reports success
        // once it has stopped what it could — an application with no
        // message loop is asked and simply never answers — and it reports
        // failure for a holder that had already gone away by itself.
        let shutdown = session.shutdown();
        let remaining = session.wait_for_holders_to_go(&holders, shutdown_grace(&holders))?;
        if !remaining.is_empty() {
            let why = match &shutdown {
                Ok(()) => String::new(),
                Err(err) => format!(": {}", err.message),
            };
            return Err(Error::new(
                IN_USE_CODE,
                format!("{} is still running{why}", describe_holders(&remaining)),
            ));
        }
        if let Err(err) = shutdown {
            reporter.event("restart_manager_failed", err.message);
        }
        reporter.event("restart_manager_shutdown", describe_holders(&holders));
        Ok(Quiescence {
            session: Some(session),
            closed: holders,
        })
    }

    /// The applications this run closed.
    pub fn closed(&self) -> &[HolderInfo] {
        &self.closed
    }

    /// Restarts what was closed and ends the session, after the transaction
    /// committed or rolled back. Applications that never asked Windows to
    /// restart them simply stay closed.
    pub fn release(self, reporter: &mut Reporter<'_>) {
        let Some(session) = self.session else {
            return;
        };
        if !self.closed.is_empty() {
            let restartable = self.closed.iter().filter(|h| h.restartable).count();
            match session.restart() {
                Ok(()) => reporter.event(
                    "restart_manager_restart",
                    format!(
                        "restarted={restartable} closed={} {}",
                        self.closed.len(),
                        describe_holders(&self.closed)
                    ),
                ),
                Err(err) => reporter.event("restart_manager_failed", err.message),
            }
        }
        drop(session);
    }
}

/// Every file a plan will write over or delete that is on the machine now:
/// the files it replaces or removes and the shortcuts it rewrites, minus
/// the ones it merely keeps and the ones that are not there yet.
///
/// Only a file that exists can be held open, so a first installation
/// registers nothing and opens no session at all.
pub fn files_at_risk(
    operations: &[crate::plan::PlannedOperation],
    install_root: &Path,
) -> Vec<PathBuf> {
    use crate::state::journal::OpKind;

    let mut out: Vec<PathBuf> = Vec::new();
    for op in operations {
        let path = match op.kind {
            OpKind::InstallFile | OpKind::RemoveFile => {
                crate::plan::absolute(install_root, &op.target).ok()
            }
            OpKind::CreateShortcut | OpKind::RemoveShortcut => Some(PathBuf::from(&op.target)),
            _ => None,
        };
        if let Some(path) = path
            && path.is_file()
        {
            out.push(path);
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlannedOperation;
    use crate::report::NullSink;
    use crate::state::journal::OpKind;

    fn planned(kind: OpKind, target: &str) -> PlannedOperation {
        PlannedOperation {
            kind,
            target: target.to_string(),
            ..PlannedOperation::default()
        }
    }

    #[test]
    fn only_the_existing_files_a_plan_writes_over_are_registered() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let menu = root.join("Menu");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("doc")).unwrap();
        std::fs::create_dir_all(&menu).unwrap();
        for relative in ["bin\\app.exe", "bin\\kept.dll", "doc\\old.md"] {
            std::fs::write(root.join(relative), b"x").unwrap();
        }
        let link = menu.join("TestApp.lnk");
        std::fs::write(&link, b"x").unwrap();

        let operations = vec![
            planned(OpKind::InstallFile, "bin\\app.exe"),
            planned(OpKind::KeepFile, "bin\\kept.dll"),
            planned(OpKind::RemoveFile, "doc\\old.md"),
            planned(OpKind::CreateDirectory, "bin"),
            planned(OpKind::CreateShortcut, &link.display().to_string()),
            planned(OpKind::SetRegistryValue, "HKCU\\Software\\TestApp"),
            // A file the plan adds is not on the machine yet.
            planned(OpKind::InstallFile, "bin\\new.dll"),
            // The same file twice is registered once.
            planned(OpKind::InstallFile, "bin\\app.exe"),
        ];
        let mut expected = vec![link, root.join("bin\\app.exe"), root.join("doc\\old.md")];
        expected.sort();
        assert_eq!(files_at_risk(&operations, root), expected);
        assert!(files_at_risk(&[], root).is_empty());
    }

    #[test]
    fn a_plan_that_writes_nothing_opens_no_session() {
        let mut sink = NullSink;
        let mut reporter = Reporter::new(&mut sink);
        let quiescence = Quiescence::acquire(&[], true, &mut reporter).unwrap();
        assert!(quiescence.closed().is_empty());
        quiescence.release(&mut reporter);
    }

    /// A session over files nobody has open finds no holder and closes
    /// A holder that goes away has to be noticed.
    ///
    /// The wait exists to give an application time to close, and it can only
    /// end when the engine sees that it has. That is a question about the
    /// machine, not about what the Restart Manager was told earlier, and this
    /// is the cheapest thing that asks it: a holder that closes the file and
    /// exits on its own, with no shutdown to attribute it to.
    #[test]
    fn a_holder_that_goes_away_by_itself_ends_the_wait() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        // The holder gets a console of its own: the Restart Manager closes a
        // console holder with a console control event, and such an event
        // reaches every process attached to that console.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("app.exe");
        std::fs::write(&file, b"binary").unwrap();
        let script = format!(
            "$f = [System.IO.File]::Open('{}', 'Open', 'Read', 'Read'); \
             Start-Sleep -Seconds 2; $f.Dispose()",
            file.display()
        );
        let mut child = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .expect("powershell.exe starts");

        let started = Instant::now();
        let mut session = Session::start().expect("a Restart Manager session");
        session
            .register(std::slice::from_ref(&file))
            .expect("the file registers");
        // The holder may not have opened it yet; the run only cares about a
        // holder it can see.
        let mut holders = session.holders().unwrap();
        while holders.is_empty() && started.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(100));
            holders = session.holders().unwrap();
        }
        assert!(!holders.is_empty(), "the holder was never listed");

        let remaining = session
            .wait_for_holders_to_go(&holders, Duration::from_secs(30))
            .unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            remaining.is_empty(),
            "a holder that had already exited was still reported as holding the file: {}",
            describe_holders(&remaining)
        );
    }

    /// nothing, which is the ordinary case on every install.
    #[test]
    fn a_file_nobody_holds_has_no_holders() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("app.exe");
        std::fs::write(&file, b"binary").unwrap();
        let mut sink = NullSink;
        let mut reporter = Reporter::new(&mut sink);
        let quiescence =
            Quiescence::acquire(std::slice::from_ref(&file), true, &mut reporter).unwrap();
        assert!(quiescence.closed().is_empty());
        quiescence.release(&mut reporter);
    }

    /// A holder line has to say what Windows can do with the holder, because
    /// "still running after the grace period" reads identically for an
    /// application that refused to close and one that was never asked - the
    /// Restart Manager found no window to ask.
    #[test]
    fn holders_read_as_name_process_id_and_what_windows_can_ask() {
        assert_eq!(describe_holders(&[]), "");
        assert_eq!(
            describe_holders(&[
                HolderInfo {
                    name: "TestApp".into(),
                    process_id: 42,
                    restartable: true,
                    kind: HolderKind::MainWindow,
                    started_at: 0,
                },
                HolderInfo {
                    name: "Other".into(),
                    process_id: 7,
                    restartable: false,
                    kind: HolderKind::Unknown,
                    started_at: 0,
                },
            ]),
            "TestApp (42, main window), Other (7, no window)"
        );
    }

    fn holder(kind: HolderKind) -> HolderInfo {
        HolderInfo {
            name: "Holder".into(),
            process_id: 1,
            restartable: false,
            kind,
            started_at: 0,
        }
    }

    /// The grace period is what Windows can ask of the holder, not one number
    /// for all of them: an application being messaged is answering, and one
    /// Windows found no window for is not going to.
    #[test]
    fn a_windowed_holder_is_given_longer_than_one_windows_cannot_ask() {
        assert_eq!(
            shutdown_grace(&[holder(HolderKind::MainWindow)]),
            WINDOWED_SHUTDOWN_GRACE
        );
        assert_eq!(
            shutdown_grace(&[holder(HolderKind::OtherWindow)]),
            WINDOWED_SHUTDOWN_GRACE
        );
        assert_eq!(
            shutdown_grace(&[holder(HolderKind::Console)]),
            OTHER_SHUTDOWN_GRACE
        );
        assert_eq!(
            shutdown_grace(&[holder(HolderKind::Unknown)]),
            OTHER_SHUTDOWN_GRACE
        );
        // One windowed holder among several is still an application that is
        // answering, so the whole set waits for it.
        assert_eq!(
            shutdown_grace(&[holder(HolderKind::Console), holder(HolderKind::MainWindow)]),
            WINDOWED_SHUTDOWN_GRACE
        );
        assert_eq!(shutdown_grace(&[]), OTHER_SHUTDOWN_GRACE);
    }

    #[test]
    fn every_restart_manager_application_type_reads_back() {
        assert_eq!(classify(RmMainWindow), HolderKind::MainWindow);
        assert_eq!(classify(RmOtherWindow), HolderKind::OtherWindow);
        assert_eq!(classify(RmService), HolderKind::Service);
        assert_eq!(classify(RmExplorer), HolderKind::Explorer);
        assert_eq!(classify(RmConsole), HolderKind::Console);
        assert_eq!(classify(RmCritical), HolderKind::Critical);
        assert_eq!(classify(0), HolderKind::Unknown);
        assert_eq!(classify(-1), HolderKind::Unknown);
    }
}
