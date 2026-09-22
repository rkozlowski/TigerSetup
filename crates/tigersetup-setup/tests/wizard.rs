//! The interactive client, driven the way an automated run drives it.
//!
//! Each test starts the real installer on the interactive desktop, finds its
//! window by class and process, and works the wizard through window messages
//! — the same surface the lab's driver reaches through UI Automation. The
//! control identifiers below are the wizard's published ones: they are its
//! UI Automation ids, so a test and a lab row address exactly the same
//! controls.
//!
//! A session without a desktop cannot show a window; every test here says so
//! and stops rather than failing, and the report records that it skipped.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    LICENSE_2026, LICENSE_2027, Machine, Started, VersionFixture, build_licensed_package, fixture,
    license_sha256,
};
use windows_sys::Win32::Foundation::{CloseHandle, GlobalFree, HWND, LPARAM, RECT};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32, Process32First, Process32Next, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_CLICK, BM_GETCHECK, EnumChildWindows, EnumWindows, GetClassNameW, GetClientRect, GetDlgItem,
    GetSystemMetrics, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    SM_CXSCREEN, SendMessageW, WM_CLOSE, WM_GETTEXT, WM_GETTEXTLENGTH, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_SETTEXT, WM_SYSCHAR,
};

/// The wizard's window class, and the class of every dialog belonging to it.
const WIZARD_CLASS: &str = "TigerSetupWizard";
const QUESTION_CLASS: &str = "TigerSetupQuestion";

// The wizard's control identifiers, which are also its UI Automation ids.
const ID_PRIMARY: i32 = 1;
const ID_CANCEL: i32 = 2;
const ID_NEXT: i32 = 101;
const ID_SCOPE_BODY: i32 = 110;
const ID_SCOPE_USER: i32 = 111;
const ID_SCOPE_MACHINE: i32 = 112;
const ID_DESTINATION_EDIT: i32 = 133;
const ID_OPTIONS_BODY: i32 = 140;
const ID_READY_SUMMARY: i32 = 151;
const ID_PROGRESS_BAR: i32 = 161;
const ID_FINISH_BODY: i32 = 170;
const ID_FINISH_LAUNCH: i32 = 171;
const ID_FINISH_LOG: i32 = 172;
const ID_CONFIRM_BODY: i32 = 180;
const ID_CHOICE_FIRST: i32 = 400;

/// The declared options of the package under test, in declaration order:
/// the PATH mode is the first, a choice, so its values are radio buttons.
const OPTION_PATH_MODE_NONE: i32 = ID_CHOICE_FIRST;
/// The package's options take two pages; this is how many.
const OPTION_PAGES: i32 = 2;

const APPEARS_WITHIN: Duration = Duration::from_secs(20);
const FINISHES_WITHIN: Duration = Duration::from_secs(180);

/// Whether this session can show a window at all. A service session has no
/// display, so the interactive tests have nothing to drive.
fn has_a_desktop() -> bool {
    unsafe { GetSystemMetrics(SM_CXSCREEN) > 0 }
}

fn skip_without_desktop() -> bool {
    if has_a_desktop() {
        return false;
    }
    eprintln!("SKIPPED: this session has no interactive desktop, so no window can be shown");
    true
}

/// Every process and its parent, as one snapshot.
fn parents() -> std::collections::HashMap<u32, u32> {
    let mut map = std::collections::HashMap::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() {
            return map;
        }
        let mut entry: PROCESSENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;
        let mut more = Process32First(snapshot, &mut entry);
        while more != 0 {
            map.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            more = Process32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    map
}

/// Whether `process` is `root` or was started by it, directly or through one
/// intermediate process. The uninstaller in the state directory cannot
/// delete itself, so it runs from a temporary copy: the window belongs to
/// that copy, not to the process the test started.
fn descends_from(process: u32, root: u32, parents: &std::collections::HashMap<u32, u32>) -> bool {
    let mut current = process;
    for _ in 0..4 {
        if current == root {
            return true;
        }
        match parents.get(&current) {
            Some(parent) if *parent != current => current = *parent,
            _ => return false,
        }
    }
    false
}

struct Search {
    process: u32,
    parents: std::collections::HashMap<u32, u32>,
    class: &'static str,
    found: HWND,
}

unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> i32 {
    unsafe {
        let search = &mut *(lparam as *mut Search);
        let mut owner = 0u32;
        GetWindowThreadProcessId(hwnd, &mut owner);
        if class_of(hwnd) == search.class
            && IsWindowVisible(hwnd) != 0
            && descends_from(owner, search.process, &search.parents)
        {
            search.found = hwnd;
            return 0;
        }
        1
    }
}

fn class_of(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

/// The visible top-level window of `class` belonging to `process`.
fn window_of(process: u32, class: &'static str) -> Option<HWND> {
    let mut search = Search {
        process,
        parents: parents(),
        class,
        found: std::ptr::null_mut(),
    };
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    (!search.found.is_null()).then_some(search.found)
}

fn wait_for_window(run: &mut Started, class: &'static str) -> HWND {
    let process = run.id();
    let deadline = Instant::now() + APPEARS_WITHIN;
    while Instant::now() < deadline {
        if let Some(hwnd) = window_of(process, class) {
            return hwnd;
        }
        assert!(!run.ended(), "the installer exited before showing a window");
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("no {class} window appeared within {APPEARS_WITHIN:?}");
}

/// The caption of a window or the label of a control, across processes.
/// `WM_GETTEXT` is marshalled by the window manager, which `GetWindowTextW`
/// is not for another process's child controls.
fn text_of(hwnd: HWND) -> String {
    let length = unsafe { SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0) } as usize;
    let mut buffer = vec![0u16; length + 2];
    let written =
        unsafe { SendMessageW(hwnd, WM_GETTEXT, buffer.len(), buffer.as_mut_ptr() as isize) }
            as usize;
    // A link control counts the terminator it wrote; the text ends before it.
    let text = &buffer[..written.min(buffer.len())];
    let end = text.iter().position(|c| *c == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..end])
}

/// The window caption, which is what an automated run matches a wizard and
/// its dialogs by.
fn title_of(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];
    let length = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

/// A label as an automation client reads it, with the mnemonic marker gone.
fn name_of(hwnd: HWND) -> String {
    tigersetup_engine::i18n::strip_mnemonics(&text_of(hwnd))
}

/// Types a path into a control the way a person would, across processes.
fn set_text_of(hwnd: HWND, text: &str) {
    let buffer: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { SendMessageW(hwnd, WM_SETTEXT, 0, buffer.as_ptr() as isize) };
}

/// The Unicode text on the clipboard, if any. The clipboard is a shared
/// resource of the desktop, so a test that touches it puts back what it
/// found (see [`ClipboardGuard`]).
fn clipboard_text() -> Option<String> {
    unsafe {
        for _ in 0..20 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                let handle = GetClipboardData(CF_UNICODETEXT as u32);
                let text = if handle.is_null() {
                    None
                } else {
                    let memory = GlobalLock(handle) as *const u16;
                    let text = (!memory.is_null()).then(|| {
                        let mut length = 0;
                        while *memory.add(length) != 0 {
                            length += 1;
                        }
                        String::from_utf16_lossy(std::slice::from_raw_parts(memory, length))
                    });
                    GlobalUnlock(handle);
                    text
                };
                CloseClipboard();
                return text;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the clipboard could not be opened");
    }
}

fn set_clipboard_text(text: &str) {
    unsafe {
        for _ in 0..20 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                EmptyClipboard();
                let buffer: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let handle = GlobalAlloc(GMEM_MOVEABLE, buffer.len() * 2);
                let memory = GlobalLock(handle) as *mut u16;
                std::ptr::copy_nonoverlapping(buffer.as_ptr(), memory, buffer.len());
                GlobalUnlock(handle);
                if SetClipboardData(CF_UNICODETEXT as u32, handle).is_null() {
                    GlobalFree(handle);
                }
                CloseClipboard();
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the clipboard could not be opened");
    }
}

/// Puts the text the clipboard held before the test back when the test is
/// over, whichever way it ends.
struct ClipboardGuard(Option<String>);

impl ClipboardGuard {
    fn take() -> ClipboardGuard {
        ClipboardGuard(clipboard_text())
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        if let Some(text) = &self.0 {
            set_clipboard_text(text);
        }
    }
}

/// Presses Alt+`letter` the way the wizard's keyboard walk does: the
/// mnemonic reaches the message loop, which activates the control that
/// carries it whichever control has the focus.
fn press_mnemonic(window: HWND, letter: char) {
    unsafe { PostMessageW(window, WM_SYSCHAR, letter as usize, 0) };
}

/// Clicks a link control with the mouse: the button messages a click sends,
/// aimed just inside its text.
fn click_link(link: HWND) {
    unsafe {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetClientRect(link, &mut rect);
        let point = ((rect.bottom / 2) << 16 | 4) as isize;
        PostMessageW(link, WM_LBUTTONDOWN, 1, point);
        PostMessageW(link, WM_LBUTTONUP, 0, point);
    }
}

fn control(window: HWND, id: i32) -> HWND {
    let control = unsafe { GetDlgItem(window, id) };
    assert!(!control.is_null(), "the window has no control {id}");
    control
}

fn visible(window: HWND, id: i32) -> bool {
    let control = unsafe { GetDlgItem(window, id) };
    !control.is_null() && unsafe { IsWindowVisible(control) != 0 }
}

fn enabled(window: HWND, id: i32) -> bool {
    let control = unsafe { GetDlgItem(window, id) };
    !control.is_null() && unsafe { IsWindowEnabled(control) != 0 }
}

fn checked(window: HWND, id: i32) -> bool {
    unsafe { SendMessageW(control(window, id), BM_GETCHECK, 0, 0) == 1 }
}

/// Clicks a control. The message is posted rather than sent: a button may
/// open a modal question, and a sent message would block this thread inside
/// the wizard's own modal loop with nobody left to answer it.
fn click(window: HWND, id: i32) {
    unsafe { PostMessageW(control(window, id), BM_CLICK, 0, 0) };
}

fn close(window: HWND) {
    unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
}

fn wait_until(run: &mut Started, what: &str, within: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if ready() {
            return;
        }
        assert!(!run.ended(), "the installer exited before {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{what} did not happen within {within:?}");
}

/// Waits for a page, recognised by the control only that page shows.
fn wait_for_page(run: &mut Started, window: HWND, marker: i32, what: &str) {
    wait_until(run, what, APPEARS_WITHIN, || visible(window, marker));
}

/// Every visible child control's label, as an automation client reads them.
fn visible_labels(window: HWND) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
        unsafe {
            if IsWindowVisible(hwnd) != 0 {
                let labels = &mut *(lparam as *mut Vec<String>);
                let label = text_of(hwnd);
                if !label.trim().is_empty() {
                    labels.push(label);
                }
            }
            1
        }
    }
    unsafe {
        EnumChildWindows(
            window,
            Some(collect),
            &mut labels as *mut Vec<String> as LPARAM,
        )
    };
    labels
}

/// Starts an interactive run of `version` on `machine`.
fn start(machine: &mut Machine, version: &VersionFixture, extra: &[&str]) -> Started {
    let mut args = vec!["install", "--scope", "user"];
    args.extend_from_slice(extra);
    let installer = version.installer.clone();
    machine.start(Path::new(&installer), &args)
}

/// Walks the destination and options pages of a fresh install, leaving the
/// wizard on the Ready page.
fn advance_to_ready(run: &mut Started, window: HWND, options: &[(i32, bool)]) {
    wait_for_page(run, window, ID_DESTINATION_EDIT, "the destination page");
    assert!(
        !text_of(control(window, ID_DESTINATION_EDIT)).is_empty(),
        "the destination page offers a default install root"
    );
    click(window, ID_NEXT);
    advance_options(run, window, options);
}

/// Walks every options page, setting each requested control where it is
/// visible — a check box is toggled, a radio button selected — and leaves
/// the wizard on the Ready page.
fn advance_options(run: &mut Started, window: HWND, options: &[(i32, bool)]) {
    for page in 0..OPTION_PAGES {
        wait_for_page(run, window, ID_OPTIONS_BODY + page, "an options page");
        for (id, wanted) in options {
            if visible(window, *id) && checked(window, *id) != *wanted {
                click(window, *id);
                wait_until(run, "the option to change", APPEARS_WITHIN, || {
                    checked(window, *id) == *wanted
                });
            }
        }
        click(window, ID_NEXT);
    }
    wait_for_page(run, window, ID_READY_SUMMARY, "the ready page");
}

#[test]
fn an_interactive_install_with_an_option_turned_off_installs_verifies_and_uninstalls() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::new("wizard-install");

    // The hold keeps the run on its busy page long enough to be observed:
    // a synthetic install finishes in well under a second, and a reader in
    // another process can otherwise catch the page before its buttons are
    // updated, or already the finish page.
    let mut run = start(
        &mut machine,
        &fixture.a,
        &["--fault", "after_prepare@20:hold:2"],
    );
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    assert_eq!(
        title_of(window),
        format!("{} Setup", common::PRODUCT_NAME),
        "the window title is what an automated run matches on"
    );

    advance_to_ready(&mut run, window, &[(OPTION_PATH_MODE_NONE, true)]);
    assert_eq!(
        name_of(control(window, ID_NEXT)),
        "Install",
        "the ready page's forward button is named for what it does"
    );
    click(window, ID_NEXT);

    wait_for_page(&mut run, window, ID_PROGRESS_BAR, "the progress page");
    // The page's buttons are updated after the page is shown, so the
    // contract is waited for, as the cancel test waits for Cancel, rather
    // than read the instant the page appears. The run is held, so a button
    // that stays enabled is a violation and not the finish page arriving.
    wait_until(
        &mut run,
        "the busy page to keep its forward button visible but disabled",
        APPEARS_WITHIN,
        || visible(window, ID_NEXT) && !enabled(window, ID_NEXT),
    );

    wait_until(&mut run, "the run to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    assert_eq!(name_of(control(window, ID_NEXT)), "Finish");
    assert!(
        !visible(window, ID_CANCEL),
        "a finished run cannot be cancelled"
    );
    assert!(
        !visible(window, ID_FINISH_LAUNCH),
        "a package that declares no launch offers nothing to start, Start Menu shortcut or not"
    );
    assert!(
        visible(window, ID_FINISH_LOG),
        "a run that wrote a log offers its path"
    );
    let mut seen = std::collections::BTreeMap::new();
    for label in visible_labels(window) {
        if let Some(letter) = tigersetup_engine::i18n::mnemonic_of(&label)
            && let Some(earlier) = seen.insert(letter, label.clone())
        {
            panic!("completion page: {earlier:?} and {label:?} both answer Alt+{letter}");
        }
    }
    click(window, ID_NEXT);

    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert_eq!(result.json()["outcome"], "installed");
    assert!(
        result.json()["launch"].is_null(),
        "nothing was offered, so the outcome reports no launch"
    );
    let resolved: Vec<String> = result
        .log_text()
        .lines()
        .filter(|line| line.contains("[options_resolved]"))
        .map(str::to_string)
        .collect();

    machine.assert_verified(&fixture.a);
    let report = machine.inspect(&fixture.a).json();
    assert_eq!(
        report["owned"]["options"]["path-mode"], "none",
        "the option the person turned off is the one recorded: {resolved:?}"
    );
    assert_eq!(
        machine.path_entry_count(),
        0,
        "an option turned off owns nothing"
    );

    let mut removal = machine.start(&machine.uninstaller().clone(), &["uninstall"]);
    let window = wait_for_window(&mut removal, WIZARD_CLASS);
    assert_eq!(
        title_of(window),
        format!("{} Uninstall", common::PRODUCT_NAME)
    );
    wait_for_page(
        &mut removal,
        window,
        ID_CONFIRM_BODY,
        "the confirmation page",
    );
    assert_eq!(name_of(control(window, ID_NEXT)), "Yes");
    assert_eq!(name_of(control(window, ID_CANCEL)), "No");
    click(window, ID_NEXT);

    wait_until(
        &mut removal,
        "the removal to finish",
        FINISHES_WITHIN,
        || visible(window, ID_FINISH_BODY),
    );
    click(window, ID_NEXT);
    let result = removal.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);

    machine.assert_absent(&fixture.a);
}

#[test]
fn cancelling_while_operations_are_applying_rolls_the_installation_back() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::new("wizard-cancel");

    // The hold makes the applying phase long enough to catch: the run stops
    // inside one operation's prepare step while the window stays live.
    let mut run = start(
        &mut machine,
        &fixture.a,
        &["--fault", "after_prepare@20:hold:5"],
    );
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    advance_to_ready(&mut run, window, &[]);
    click(window, ID_NEXT);

    wait_for_page(&mut run, window, ID_PROGRESS_BAR, "the progress page");
    // The test is about cancelling the transaction: a click that lands
    // while the prerequisite is still being installed cancels the
    // dependency phase instead, so the transaction is waited for first.
    let log = run.log.clone();
    wait_until(&mut run, "the transaction to open", FINISHES_WITHIN, || {
        std::fs::read_to_string(&log).is_ok_and(|text| text.contains("[transaction_started]"))
    });
    wait_until(
        &mut run,
        "cancel to become available",
        APPEARS_WITHIN,
        || enabled(window, ID_CANCEL),
    );
    click(window, ID_CANCEL);

    // Cancelling asks first, in the installer's own words, under the
    // wizard's own title.
    let question = wait_for_window(&mut run, QUESTION_CLASS);
    assert_eq!(
        title_of(question),
        title_of(window),
        "a dialog carries its wizard's title, because that is how it is found"
    );
    assert_eq!(name_of(control(question, ID_PRIMARY)), "Yes");
    assert_eq!(name_of(control(question, ID_CANCEL)), "No");
    click(question, ID_PRIMARY);

    wait_until(&mut run, "the rollback to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    click(window, ID_NEXT);

    let result = run.finish();
    assert_eq!(result.exit_code, Some(5), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["outcome"], "cancelled", "{outcome}");
    assert_eq!(outcome["code"], "cancelled", "{outcome}");
    machine.assert_absent(&fixture.a);
}

#[test]
fn the_wizard_speaks_the_language_it_was_asked_for() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::new("wizard-polish");

    let mut run = start(&mut machine, &fixture.a, &["--lang", "pl-PL"]);
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    assert_eq!(
        title_of(window),
        format!("Instalator — {}", common::PRODUCT_NAME)
    );
    wait_for_page(
        &mut run,
        window,
        ID_DESTINATION_EDIT,
        "the destination page",
    );
    assert_eq!(
        name_of(control(window, ID_NEXT)),
        "Dalej",
        "an automated run advances by the button's visible name"
    );
    assert_eq!(name_of(control(window, ID_CANCEL)), "Anuluj");

    close(window);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(5), "closing the window cancels");
    assert!(
        !machine.install_root().exists(),
        "a wizard closed before it installed leaves nothing behind"
    );
}

#[test]
fn no_two_controls_on_a_page_share_an_alt_mnemonic() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    for lang in tigersetup_engine::i18n::LANGUAGES {
        let mut machine = Machine::new("wizard-mnemonics");
        let mut run = start(&mut machine, &fixture.a, &["--lang", lang]);
        let window = wait_for_window(&mut run, WIZARD_CLASS);

        for (marker, page) in [
            (ID_DESTINATION_EDIT, "destination"),
            (ID_OPTIONS_BODY, "options"),
            (ID_OPTIONS_BODY + 1, "options 2"),
            (ID_READY_SUMMARY, "ready"),
        ] {
            wait_for_page(&mut run, window, marker, page);
            let mut seen = std::collections::BTreeMap::new();
            for label in visible_labels(window) {
                let Some(letter) = tigersetup_engine::i18n::mnemonic_of(&label) else {
                    continue;
                };
                if let Some(earlier) = seen.insert(letter, label.clone()) {
                    panic!(
                        "{lang} {page} page: {earlier:?} and {label:?} both answer Alt+{letter}"
                    );
                }
            }
            if marker != ID_READY_SUMMARY {
                click(window, ID_NEXT);
            }
        }

        close(window);
        let result = run.finish();
        assert_eq!(result.exit_code, Some(5));
    }
}

/// The scope page takes the choice it is given.
///
/// This is the half of the machine-scope question that can be settled without
/// an administrator: whether clicking "install for all users" selects it. What
/// the engine then does with machine scope is covered by `machine_scope.rs`,
/// which drives it through the command line under the test seams.
///
/// The run is cancelled at the scope page rather than carried through, because
/// going further asks the machine for elevation and nothing here can answer a
/// UAC prompt.
///
/// The page only appears when the command line named no scope, which is the
/// double-click case.
#[test]
fn the_scope_page_takes_the_choice_it_is_given() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::new("wizard-scope");
    let installer = fixture.a.installer.clone();
    let mut run = machine.start(Path::new(&installer), &["install"]);
    let window = wait_for_window(&mut run, WIZARD_CLASS);

    wait_for_page(&mut run, window, ID_SCOPE_MACHINE, "the scope page");
    assert!(
        checked(window, ID_SCOPE_USER) && !checked(window, ID_SCOPE_MACHINE),
        "the package declares user scope first, so it is the offered default"
    );

    click(window, ID_SCOPE_MACHINE);
    wait_until(
        &mut run,
        "the machine radio to take the choice",
        APPEARS_WITHIN,
        || checked(window, ID_SCOPE_MACHINE) && !checked(window, ID_SCOPE_USER),
    );

    click(window, ID_SCOPE_USER);
    wait_until(&mut run, "the choice to go back", APPEARS_WITHIN, || {
        checked(window, ID_SCOPE_USER) && !checked(window, ID_SCOPE_MACHINE)
    });

    close(window);
    let completed = run.finish();
    assert_eq!(
        completed.exit_code,
        Some(exit_cancelled()),
        "closing the wizard cancels: {}",
        completed.stdout
    );
    assert!(!machine.install_root().exists(), "nothing was installed");
}

/// The destination follows the scope, and stops following once it is edited.
///
/// The scope page and the destination page describe one decision. A wizard
/// that lets them disagree installs "for all users" into one user's profile,
/// which is what an installer is expected never to do — and a destination the
/// person typed themselves is their answer, so the scope must not overwrite
/// it afterwards.
#[test]
fn the_destination_follows_the_scope_until_it_is_edited() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::new("wizard-scope-destination");
    let installer = fixture.a.installer.clone();
    let mut run = machine.start(Path::new(&installer), &["install"]);
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    wait_for_page(&mut run, window, ID_SCOPE_MACHINE, "the scope page");

    let destination = control(window, ID_DESTINATION_EDIT);
    let user_root = text_of(destination);
    assert!(
        !user_root.is_empty(),
        "the destination starts at the offered scope's default"
    );

    click(window, ID_SCOPE_MACHINE);
    wait_until(
        &mut run,
        "the destination to follow machine scope",
        APPEARS_WITHIN,
        || text_of(destination) != user_root,
    );
    let machine_root = text_of(destination);

    click(window, ID_SCOPE_USER);
    wait_until(
        &mut run,
        "the destination to follow user scope back",
        APPEARS_WITHIN,
        || text_of(destination) == user_root,
    );

    // A path the person chose outranks the scope's default from then on.
    let chosen = machine.chosen_destination();
    set_text_of(destination, &chosen);
    click(window, ID_SCOPE_MACHINE);
    wait_until(
        &mut run,
        "the machine radio to take the choice",
        APPEARS_WITHIN,
        || checked(window, ID_SCOPE_MACHINE),
    );
    assert_eq!(
        text_of(destination),
        chosen,
        "a destination the person chose is not replaced by the scope's default \
         (it would have become {machine_root})"
    );

    close(window);
    let completed = run.finish();
    assert_eq!(completed.exit_code, Some(exit_cancelled()));
    assert!(!machine.install_root().exists(), "nothing was installed");
}

/// The engine's exit code for a cancelled run.
fn exit_cancelled() -> i32 {
    5
}

/// The wizard's scope page for a product the machine already holds: it
/// names the installation the run continues with and offers no choice that
/// would create a second one, then the upgrade lands in that installation
/// whatever the package's default scope says.
#[test]
fn a_rerun_on_an_installed_product_continues_with_that_installation() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::in_scope(
        "wizard-existing",
        tigersetup_engine::format::identity::Scope::Machine,
    );
    let installed = machine.install(&fixture.a);
    assert_eq!(installed.exit_code, Some(0), "{}", installed.stdout);

    // No scope on the command line: the package's default is user scope, and
    // the product is installed for the machine.
    let installer = fixture.b.installer.clone();
    let mut run = machine.start(Path::new(&installer), &["install"]);
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    wait_for_page(
        &mut run,
        window,
        ID_SCOPE_BODY,
        "the existing-installation page",
    );
    assert!(
        unsafe { GetDlgItem(window, ID_SCOPE_USER) }.is_null()
            && unsafe { GetDlgItem(window, ID_SCOPE_MACHINE) }.is_null(),
        "an existing installation is stated, not chosen between"
    );
    let body = text_of(control(window, ID_SCOPE_BODY));
    assert!(
        body.contains(common::VERSION_A) && body.contains("for all users"),
        "the page names the installation: {body:?}"
    );
    click(window, ID_NEXT);

    advance_options(&mut run, window, &[]);
    let summary = text_of(control(window, ID_READY_SUMMARY));
    assert!(
        summary.contains("For all users"),
        "the summary says which installation is updated: {summary:?}"
    );
    click(window, ID_NEXT);
    wait_until(&mut run, "the upgrade to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    click(window, ID_NEXT);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let document = result.json();
    assert_eq!(document["installation"]["scope"], "machine");
    assert_eq!(document["transaction"]["kind"], "upgrade");
    machine.assert_verified(&fixture.b);
    assert!(
        !machine
            .localappdata
            .join("TigerSetup")
            .join(common::PRODUCT_ID)
            .exists(),
        "no per-user installation was created"
    );
}

/// Both scopes hold the product: the scope page names the two installations
/// and the person picks one; the run then continues in a new process for
/// that scope, whose outcome the first one reports as its own, and the other
/// installation is left exactly as it was.
#[test]
fn two_installations_are_told_apart_on_the_scope_page() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::new("wizard-select");
    let user = machine.run(
        &fixture.a.installer,
        &["install", "--quiet", "--scope", "user"],
    );
    assert_eq!(user.exit_code, Some(0), "{}", user.stdout);
    let other = machine.run(
        &fixture.a.installer,
        &["install", "--quiet", "--scope", "machine"],
    );
    assert_eq!(other.exit_code, Some(2), "{}", other.stdout);
    assert_eq!(
        other.json()["code"],
        "scope_conflict",
        "the synthetic package keeps the default policy; the second installation is made below with the parallel one"
    );
    // A second installation of the same product, the way a machine gets one
    // in practice: an earlier installer that did not follow the rule.
    let parallel = common::build_small_package(
        &common::scratch("wizard-select-parallel"),
        &format!(
            "[package]\nid = \"{}\"\nname = \"{}\"\nversion = \"{}\"\npublisher = \"IT Tiger\"\n\n[install]\nscopes = [\"user\", \"machine\"]\nexisting_scope = \"allow-parallel\"\n\n[[files]]\nsource = \"payload/**\"\n",
            common::PRODUCT_ID,
            common::PRODUCT_NAME,
            common::VERSION_A
        ),
    );
    let second = machine.run(&parallel, &["install", "--quiet", "--scope", "machine"]);
    assert_eq!(second.exit_code, Some(0), "{}", second.stdout);

    let installer = fixture.b.installer.clone();
    let mut run = machine.start(Path::new(&installer), &["install"]);
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    wait_for_page(&mut run, window, ID_SCOPE_MACHINE, "the selection page");
    let user_label = text_of(control(window, ID_SCOPE_USER));
    let machine_label = text_of(control(window, ID_SCOPE_MACHINE));
    assert!(
        user_label.contains(common::VERSION_A)
            && user_label.contains(&machine.install_root().display().to_string()),
        "each choice names its installation: {user_label:?}"
    );
    assert!(
        machine_label.contains(common::VERSION_A)
            && machine_label.contains(
                &machine
                    .programfiles
                    .join(common::PRODUCT_NAME)
                    .display()
                    .to_string()
            ),
        "{machine_label:?}"
    );
    assert!(
        checked(window, ID_SCOPE_USER),
        "the default scope is offered first"
    );

    click(window, ID_SCOPE_MACHINE);
    wait_until(&mut run, "the machine choice", APPEARS_WITHIN, || {
        checked(window, ID_SCOPE_MACHINE)
    });
    click(window, ID_NEXT);

    // The run continues in a child process for the chosen scope; this window
    // steps aside for it, and the child's wizard starts at its next page.
    wait_until(
        &mut run,
        "the first window to step aside",
        APPEARS_WITHIN,
        || unsafe { IsWindowVisible(window) == 0 },
    );
    let child = wait_for_window(&mut run, WIZARD_CLASS);
    assert_ne!(child, window);
    advance_options(&mut run, child, &[]);
    click(child, ID_NEXT);
    wait_until(
        &mut run,
        "the child's upgrade to finish",
        FINISHES_WITHIN,
        || visible(child, ID_FINISH_BODY),
    );
    click(child, ID_NEXT);

    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let document = result.json();
    assert_eq!(
        document["installation"]["scope"], "machine",
        "the first process reports the child's run as its own"
    );
    assert_eq!(document["installation"]["version"], common::VERSION_B);

    // The machine installation is the new version; the user one is untouched.
    let upgraded = machine.run(&fixture.b.installer, &["verify", "--scope", "machine"]);
    assert_eq!(upgraded.json()["status"], "ok", "{}", upgraded.stdout);
    assert_eq!(
        upgraded.json()["installation"]["version"],
        common::VERSION_B
    );
    machine.assert_verified(&fixture.a);
}

/// A run the engine refuses — here an explicit scope beside an existing
/// installation under the default policy — opens on its completion page
/// with the refusal, starts nothing, and exits as the refusal says.
#[test]
fn a_refused_scope_is_shown_rather_than_walked_through() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::in_scope(
        "wizard-refused",
        tigersetup_engine::format::identity::Scope::Machine,
    );
    let installed = machine.install(&fixture.a);
    assert_eq!(installed.exit_code, Some(0), "{}", installed.stdout);

    let mut run = start(&mut machine, &fixture.b, &[]);
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    wait_for_page(&mut run, window, ID_FINISH_BODY, "the completion page");
    let body = text_of(control(window, ID_FINISH_BODY));
    assert!(
        body.contains("installed for all users"),
        "the page says why nothing was done: {body:?}"
    );
    assert!(!visible(window, ID_FINISH_LAUNCH), "nothing to launch");
    assert!(
        !visible(window, ID_FINISH_LOG),
        "a refusal wrote no log, so the page offers no log path to copy"
    );
    click(window, ID_NEXT);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(2), "{}", result.stdout);
    assert_eq!(result.json()["code"], "scope_conflict");
    assert!(result.json()["log"].is_null(), "{}", result.stdout);
    machine.assert_verified(&fixture.a);
    assert!(
        !machine
            .localappdata
            .join("TigerSetup")
            .join(common::PRODUCT_ID)
            .exists()
    );
}

/// The completion page offers the log's path to the clipboard rather than
/// showing it: a link, reached by Alt+C and by the mouse, that puts the exact
/// path on the clipboard and says so only once it is there. The path itself
/// is on no control of the page.
#[test]
fn the_completion_page_copies_the_log_path_instead_of_showing_it() {
    if skip_without_desktop() {
        return;
    }
    let fixture = fixture();
    let mut machine = Machine::new("wizard-log");
    let _restore = ClipboardGuard::take();

    let mut run = start(&mut machine, &fixture.a, &[]);
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    advance_to_ready(&mut run, window, &[]);
    click(window, ID_NEXT);
    wait_until(&mut run, "the run to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    wait_until(
        &mut run,
        "the log link to be offered",
        APPEARS_WITHIN,
        || visible(window, ID_FINISH_LOG),
    );

    let link = control(window, ID_FINISH_LOG);
    assert_eq!(
        class_of(link),
        "SysLink",
        "the offer is the native link control: focusable, invokable, themed"
    );
    assert_eq!(name_of(link), "<a>Copy log path</a>");
    assert!(
        enabled(window, ID_FINISH_LOG),
        "the offer is live while the page is"
    );
    for label in visible_labels(window) {
        assert!(
            !label.contains(".log") && !label.contains(":\\"),
            "the path is offered, not shown: {label:?}"
        );
    }

    // Alt+C, the keyboard walk's way. The confirmation appears only once
    // the clipboard holds the path.
    set_clipboard_text("sentinel: not the log path");
    press_mnemonic(window, 'c');
    wait_until(
        &mut run,
        "the link to confirm the copy",
        APPEARS_WITHIN,
        || name_of(link) == "<a>Log path copied</a>",
    );
    let copied = clipboard_text().expect("the clipboard holds Unicode text");
    assert!(
        copied.ends_with(".log") && Path::new(&copied).is_absolute(),
        "the clipboard holds a log path: {copied:?}"
    );
    assert!(
        Path::new(&copied).is_file(),
        "the copied path is the log that was written: {copied:?}"
    );

    // The mouse, and the link's own notification: a second copy replaces a
    // clipboard that was changed in between.
    set_clipboard_text("sentinel: changed since");
    click_link(link);
    wait_until(&mut run, "the second copy to land", APPEARS_WITHIN, || {
        clipboard_text().as_deref() == Some(copied.as_str())
    });
    assert_eq!(name_of(link), "<a>Log path copied</a>");

    // Finish behaves as it always did, and the outcome names the very path
    // the clipboard was given.
    assert_eq!(name_of(control(window, ID_NEXT)), "Finish");
    click(window, ID_NEXT);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["outcome"], "installed");
    assert_eq!(
        outcome["log"].as_str(),
        Some(copied.as_str()),
        "the exact path the run reported is what was copied"
    );
    machine.assert_verified(&fixture.a);
}

// ---------------------------------------------------------------------------
// The licence page: asked once per licence text, by the person, and never by
// an unattended run.

const ID_LICENSE_TEXT: i32 = 121;
const ID_LICENSE_ACCEPT: i32 = 122;

/// Whether the flow this wizard shows has a licence page at all. A page's
/// controls exist only when its flow has the page, so a flow that skips the
/// licence has no accept button anywhere, hidden or otherwise.
fn has_licence_page(window: HWND) -> bool {
    !unsafe { GetDlgItem(window, ID_LICENSE_ACCEPT) }.is_null()
}

/// Waits for the licence page, checks it holds the run until the text is
/// accepted, accepts it and moves on.
fn accept_licence(run: &mut Started, window: HWND) {
    wait_for_page(run, window, ID_LICENSE_TEXT, "the licence page");
    assert!(
        !enabled(window, ID_NEXT),
        "nothing continues until the text is accepted"
    );
    click(window, ID_LICENSE_ACCEPT);
    wait_until(run, "acceptance to enable Next", APPEARS_WITHIN, || {
        enabled(window, ID_NEXT)
    });
    click(window, ID_NEXT);
}

/// From the Ready page: starts the run, waits it out, closes the wizard
/// and returns what it printed.
fn install_from_ready(mut run: Started, window: HWND) -> common::Run {
    wait_for_page(&mut run, window, ID_READY_SUMMARY, "the ready page");
    click(window, ID_NEXT);
    wait_until(&mut run, "the run to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    click(window, ID_NEXT);
    run.finish()
}

/// The installation as `inspect` describes it: `null` once it is gone.
fn installation_of(machine: &mut Machine, installer: &Path) -> serde_json::Value {
    let run = machine.run(installer, &["inspect", "--scope", "user"]);
    assert_eq!(run.exit_code, Some(0), "{}", run.stdout);
    run.json()["installation"].clone()
}

fn interactive(machine: &mut Machine, installer: &Path, extra: &[&str]) -> (Started, HWND) {
    let mut args = vec!["install", "--scope", "user"];
    args.extend_from_slice(extra);
    let mut run = machine.start(installer, &args);
    let window = wait_for_window(&mut run, WIZARD_CLASS);
    (run, window)
}

#[test]
fn the_licence_page_asks_once_per_licence_text() {
    if skip_without_desktop() {
        return;
    }
    let dir = common::scratch("wizard-licence-flow");
    let a = build_licensed_package(&dir.join("1.0.0"), "1.0.0", LICENSE_2026);
    let b = build_licensed_package(&dir.join("1.1.0"), "1.1.0", LICENSE_2026);
    let c = build_licensed_package(&dir.join("1.2.0"), "1.2.0", LICENSE_2027);
    let accepted_2026 = license_sha256(LICENSE_2026);
    let accepted_2027 = license_sha256(LICENSE_2027);
    assert_ne!(
        accepted_2026, accepted_2027,
        "one changed byte is a new text"
    );
    let mut machine = Machine::new("wizard-licence-flow");

    // A first install asks, and a successful accepted install records the
    // exact text the person accepted.
    let (mut run, window) = interactive(&mut machine, &a, &[]);
    accept_licence(&mut run, window);
    wait_for_page(
        &mut run,
        window,
        ID_DESTINATION_EDIT,
        "the destination page",
    );
    click(window, ID_NEXT);
    let result = install_from_ready(run, window);
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert_eq!(result.json()["outcome"], "installed");
    let installed = installation_of(&mut machine, &a);
    assert_eq!(installed["version"], "1.0.0");
    assert_eq!(
        installed["accepted_license_sha256"], accepted_2026,
        "the accepted licence is the hash of its exact bytes: {installed}"
    );

    // The same text again — a reinstall, then an upgrade — is not asked for.
    let (mut run, window) = interactive(&mut machine, &a, &[]);
    wait_for_page(&mut run, window, ID_READY_SUMMARY, "the ready page");
    assert!(
        !has_licence_page(window),
        "a reinstall under the accepted text skips the licence page"
    );
    let result = install_from_ready(run, window);
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert_eq!(
        installation_of(&mut machine, &a)["accepted_license_sha256"],
        accepted_2026
    );

    let (mut run, window) = interactive(&mut machine, &b, &[]);
    wait_for_page(&mut run, window, ID_READY_SUMMARY, "the ready page");
    assert!(
        !has_licence_page(window),
        "an upgrade under the accepted text skips the licence page"
    );
    let result = install_from_ready(run, window);
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let installed = installation_of(&mut machine, &b);
    assert_eq!(installed["version"], "1.1.0");
    assert_eq!(installed["accepted_license_sha256"], accepted_2026);

    // A changed text asks again. An upgrade that fails after the person
    // accepted it rolls back to the old version with the old acceptance:
    // the new text was accepted, but not for anything that got installed.
    let (mut run, window) = interactive(&mut machine, &c, &["--fault", "before_commit:fail"]);
    accept_licence(&mut run, window);
    let result = install_from_ready(run, window);
    assert_eq!(result.exit_code, Some(1), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["outcome"], "rolled_back", "{outcome}");
    assert_eq!(outcome["transaction"]["kind"], "upgrade", "{outcome}");
    let installed = installation_of(&mut machine, &b);
    assert_eq!(installed["version"], "1.1.0", "{installed}");
    assert_eq!(
        installed["accepted_license_sha256"], accepted_2026,
        "a rolled-back upgrade keeps the previous acceptance: {installed}"
    );

    // The same upgrade succeeding records the new text — and only then.
    let (mut run, window) = interactive(&mut machine, &c, &[]);
    accept_licence(&mut run, window);
    let result = install_from_ready(run, window);
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let installed = installation_of(&mut machine, &c);
    assert_eq!(installed["version"], "1.2.0", "{installed}");
    assert_eq!(installed["accepted_license_sha256"], accepted_2027);

    let (mut run, window) = interactive(&mut machine, &c, &[]);
    wait_for_page(&mut run, window, ID_READY_SUMMARY, "the ready page");
    assert!(
        !has_licence_page(window),
        "the new text, once accepted, is not asked for again"
    );
    close(window);
    let result = run.finish();
    assert_eq!(
        result.exit_code,
        Some(exit_cancelled()),
        "{}",
        result.stdout
    );

    // Removing the product asks for confirmation, never for a licence.
    let mut removal = machine.start(&c, &["uninstall", "--scope", "user"]);
    let window = wait_for_window(&mut removal, WIZARD_CLASS);
    wait_for_page(
        &mut removal,
        window,
        ID_CONFIRM_BODY,
        "the confirmation page",
    );
    assert!(
        !has_licence_page(window),
        "an uninstall has no licence page"
    );
    click(window, ID_NEXT);
    wait_until(
        &mut removal,
        "the removal to finish",
        FINISHES_WITHIN,
        || visible(window, ID_FINISH_BODY),
    );
    click(window, ID_NEXT);
    let result = removal.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert_eq!(installation_of(&mut machine, &c), serde_json::Value::Null);
}

/// The three facts stay apart: the package carries a licence, a person
/// accepted this text, and the run may proceed unattended. An installation
/// nobody accepted a licence for — a quiet install, or one made before the
/// engine recorded acceptance at all — is asked once, interactively; a quiet
/// upgrade to a changed text runs through and leaves the earlier acceptance
/// exactly as it was, for the next interactive run to ask about.
#[test]
fn an_unattended_run_leaves_the_licence_for_the_next_person_to_accept() {
    if skip_without_desktop() {
        return;
    }
    let dir = common::scratch("wizard-licence-quiet");
    let a = build_licensed_package(&dir.join("1.0.0"), "1.0.0", LICENSE_2026);
    let c = build_licensed_package(&dir.join("1.2.0"), "1.2.0", LICENSE_2027);
    let accepted_2026 = license_sha256(LICENSE_2026);
    let accepted_2027 = license_sha256(LICENSE_2027);
    let mut machine = Machine::new("wizard-licence-quiet");

    let quiet = machine.run(&a, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(quiet.exit_code, Some(0), "{}", quiet.stdout);
    let installed = installation_of(&mut machine, &a);
    assert_eq!(installed["version"], "1.0.0");
    assert_eq!(
        installed["accepted_license_sha256"],
        serde_json::Value::Null,
        "a quiet install accepts nothing on anyone's behalf: {installed}"
    );

    // An installation with no recorded acceptance is asked once: a
    // same-version rerun with nothing else to change still commits the
    // acceptance, and the next rerun no longer asks.
    let (mut run, window) = interactive(&mut machine, &a, &[]);
    accept_licence(&mut run, window);
    let result = install_from_ready(run, window);
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert_eq!(outcome["transaction"]["kind"], "reinstall", "{outcome}");
    assert_eq!(
        installation_of(&mut machine, &a)["accepted_license_sha256"],
        accepted_2026
    );
    let (mut run, window) = interactive(&mut machine, &a, &[]);
    wait_for_page(&mut run, window, ID_READY_SUMMARY, "the ready page");
    assert!(!has_licence_page(window));
    close(window);
    let result = run.finish();
    assert_eq!(
        result.exit_code,
        Some(exit_cancelled()),
        "{}",
        result.stdout
    );

    // A package manager's upgrade to a changed text: no window, no wait,
    // and the recorded acceptance is still the 2026 text.
    let quiet = machine.run(&c, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(quiet.exit_code, Some(0), "{}", quiet.stdout);
    assert_eq!(quiet.json()["transaction"]["kind"], "upgrade");
    let installed = installation_of(&mut machine, &c);
    assert_eq!(installed["version"], "1.2.0", "{installed}");
    assert_eq!(
        installed["accepted_license_sha256"], accepted_2026,
        "a quiet upgrade neither records the new text nor loses the old acceptance: {installed}"
    );

    // The next interactive run asks for the 2027 text, once.
    let (mut run, window) = interactive(&mut machine, &c, &[]);
    accept_licence(&mut run, window);
    let result = install_from_ready(run, window);
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert_eq!(
        installation_of(&mut machine, &c)["accepted_license_sha256"],
        accepted_2027
    );
    let (mut run, window) = interactive(&mut machine, &c, &[]);
    wait_for_page(&mut run, window, ID_READY_SUMMARY, "the ready page");
    assert!(!has_licence_page(window));
    close(window);
    let result = run.finish();
    assert_eq!(
        result.exit_code,
        Some(exit_cancelled()),
        "{}",
        result.stdout
    );
}

// Launch after install (`TigerSetup-Design.md` §11.7). The program is the
// controlled `TigerSetupTestLaunch.exe`, which writes down how it was started
// — its process, token, arguments and working directory — to the report its
// own arguments name, under the machine's `%PROGRAMDATA%`.

/// Arguments chosen to break any command line that is joined or split
/// carelessly: spaces, quotes, backslashes before a quote and at the end,
/// an empty argument, non-ASCII text, a bare percent and a placeholder.
const LAUNCH_ARGUMENTS: &[&str] = &[
    "plain",
    "two words",
    "a \"quoted\" word",
    "trailing\\",
    "both \\\" kinds\\\\",
    "",
    "zażółć gęślą jaźń",
    "100%",
    "v%VERSION%",
];

/// Where the launched program writes its report on `machine`.
fn launch_report(machine: &Machine) -> PathBuf {
    machine
        .programdata
        .join("TigerSetupTestLaunch")
        .join("report.json")
}

/// A one-scope package of the test product whose `[launch]` offers
/// `program` (the bytes of `bin/TigerSetupTestLaunch.exe`) with
/// [`LAUNCH_ARGUMENTS`], in the installed `data` directory.
fn build_launch_package(dir: &Path, version: &str, checked: bool, program: &[u8]) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let mut arguments: Vec<String> = [
        "--report",
        "%PROGRAMDATA%\\TigerSetupTestLaunch\\report.json",
        "--observe-ms",
        "1500",
        "--exit-after-ms",
        "5000",
        "--",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    arguments.extend(LAUNCH_ARGUMENTS.iter().map(|a| a.to_string()));
    let manifest = format!(
        r#"[package]
id = "{}"
name = "{}"
version = "{version}"
publisher = "IT Tiger"

[install]
scopes = ["user"]

[[files]]
source = "payload/**"

[launch]
executable = "bin/TigerSetupTestLaunch.exe"
arguments = {}
working_directory = "data"
checked = {checked}
"#,
        common::PRODUCT_ID,
        common::PRODUCT_NAME,
        serde_json::to_string(&arguments).unwrap()
    );
    common::build_package(
        dir,
        &manifest,
        &[
            ("bin/TigerSetupTestLaunch.exe", program),
            ("data/readme.txt", b"the launched program runs here"),
        ],
    )
}

/// What a launched program reported, once it has.
fn wait_for_launch_report(machine: &Machine) -> serde_json::Value {
    let report = launch_report(machine);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&report) {
            return serde_json::from_str(&text).unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "the launched program wrote no report at {}",
        report.display()
    );
}

/// No program was started: the report does not appear, even given the
/// time a started one takes to write it.
fn assert_nothing_launched(machine: &Machine, why: &str) {
    std::thread::sleep(Duration::from_secs(3));
    assert!(!launch_report(machine).exists(), "{why}");
}

/// A first interactive install of `installer`, walked to its completion
/// page.
fn install_to_completion(
    machine: &mut Machine,
    installer: &Path,
    extra: &[&str],
) -> (Started, HWND) {
    let (mut run, window) = interactive(machine, installer, extra);
    // A page is seen before its buttons are updated, and a click posted to a
    // Next that is still disabled is dropped (LESSONS_LEARNED.md), so each
    // click waits for the button it presses.
    for (marker, what) in [
        (ID_DESTINATION_EDIT, "the destination page"),
        (ID_READY_SUMMARY, "the ready page"),
    ] {
        wait_for_page(&mut run, window, marker, what);
        wait_until(&mut run, "Next to be enabled", APPEARS_WITHIN, || {
            enabled(window, ID_NEXT)
        });
        click(window, ID_NEXT);
    }
    wait_until(&mut run, "the run to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    (run, window)
}

fn assert_launched_as_declared(machine: &Machine, outcome: &serde_json::Value, version: &str) {
    let launch = &outcome["launch"];
    assert_eq!(launch["status"], "started", "{outcome}");
    assert_eq!(
        launch["method"], "own_token",
        "an unelevated wizard starts the program with its own token: {outcome}"
    );
    let report = wait_for_launch_report(machine);
    assert_eq!(
        report["pid"], launch["pid"],
        "the outcome names the process that ran: {report}"
    );
    let arguments: Vec<&str> = report["arguments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .skip_while(|a| *a != "--")
        .skip(1)
        .collect();
    let expected: Vec<String> = LAUNCH_ARGUMENTS
        .iter()
        .map(|a| a.replace("%VERSION%", version))
        .collect();
    assert_eq!(
        arguments, expected,
        "every declared argument arrives as exactly one argument, expanded and otherwise untouched: {report}"
    );
    let root = machine.install_root();
    assert!(
        Path::new(report["working_directory"].as_str().unwrap()).eq(&root.join("data")),
        "it runs in the declared working directory, under {}: {report}",
        root.display()
    );
    assert_eq!(report["elevated"], false, "{report}");
    assert_eq!(report["administrators_enabled"], false, "{report}");
    assert_eq!(report["window_shown"], true, "{report}");
}

/// The completion page offers the declared program, checked as declared;
/// Finish starts it with exactly the declared arguments in the declared
/// directory, never elevated, and the outcome and the log say so. An
/// interactive upgrade offers it again; a repair and an uninstall never do.
#[test]
fn the_completion_page_starts_the_declared_program_as_declared() {
    if skip_without_desktop() {
        return;
    }
    let dir = common::scratch("wizard-launch");
    let program = std::fs::read(common::launch_executable()).unwrap();
    let a = build_launch_package(&dir.join("1.0.0"), "1.0.0", true, &program);
    let b = build_launch_package(&dir.join("1.1.0"), "1.1.0", true, &program);
    let mut machine = Machine::new("wizard-launch");

    // The installer describes its offer wherever the package is described.
    let declared = machine.run(&a, &["inspect", "--scope", "user"]).json();
    let launch = &declared["package"]["launch"];
    assert_eq!(
        launch["executable"], "bin/TigerSetupTestLaunch.exe",
        "{declared}"
    );
    assert_eq!(launch["working_directory"], "data", "{declared}");
    assert_eq!(launch["checked"], true, "{declared}");
    assert_eq!(
        launch["arguments"].as_array().unwrap().len(),
        7 + LAUNCH_ARGUMENTS.len()
    );

    let (run, window) = install_to_completion(&mut machine, &a, &[]);
    assert!(
        visible(window, ID_FINISH_LAUNCH) && checked(window, ID_FINISH_LAUNCH),
        "the offer is shown, checked as the package declares"
    );
    assert_eq!(
        name_of(control(window, ID_FINISH_LAUNCH)),
        "Launch TigerSetupTestApp"
    );
    let mut seen = std::collections::BTreeMap::new();
    for label in visible_labels(window) {
        if let Some(letter) = tigersetup_engine::i18n::mnemonic_of(&label)
            && let Some(earlier) = seen.insert(letter, label.clone())
        {
            panic!("completion page: {earlier:?} and {label:?} both answer Alt+{letter}");
        }
    }
    click(window, ID_NEXT);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert_launched_as_declared(&machine, &outcome, "1.0.0");
    assert!(result.log_has("[launch_started]"), "{}", result.log_text());

    // An interactive upgrade shows the same completion page, so it makes
    // the same offer, and %VERSION% is the new version.
    std::fs::remove_file(launch_report(&machine)).unwrap();
    let (mut run, window) = interactive(&mut machine, &b, &[]);
    wait_for_page(&mut run, window, ID_READY_SUMMARY, "the ready page");
    wait_until(&mut run, "Next to be enabled", APPEARS_WITHIN, || {
        enabled(window, ID_NEXT)
    });
    click(window, ID_NEXT);
    wait_until(&mut run, "the upgrade to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    assert!(visible(window, ID_FINISH_LAUNCH) && checked(window, ID_FINISH_LAUNCH));
    click(window, ID_NEXT);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["transaction"]["kind"], "upgrade", "{outcome}");
    assert_launched_as_declared(&machine, &outcome, "1.1.0");

    // A repair is not a moment to start the product.
    std::fs::remove_file(launch_report(&machine)).unwrap();
    let mut repair = machine.start(&b, &["repair", "--scope", "user"]);
    let window = wait_for_window(&mut repair, WIZARD_CLASS);
    wait_for_page(&mut repair, window, ID_READY_SUMMARY, "the ready page");
    wait_until(&mut repair, "Next to be enabled", APPEARS_WITHIN, || {
        enabled(window, ID_NEXT)
    });
    click(window, ID_NEXT);
    wait_until(&mut repair, "the repair to finish", FINISHES_WITHIN, || {
        visible(window, ID_FINISH_BODY)
    });
    assert!(
        !visible(window, ID_FINISH_LAUNCH),
        "a repair offers nothing to start"
    );
    click(window, ID_NEXT);
    let result = repair.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert!(result.json()["launch"].is_null(), "{}", result.stdout);

    // Nor is an uninstall.
    let mut removal = machine.start(&machine.uninstaller().clone(), &["uninstall"]);
    let window = wait_for_window(&mut removal, WIZARD_CLASS);
    wait_for_page(
        &mut removal,
        window,
        ID_CONFIRM_BODY,
        "the confirmation page",
    );
    click(window, ID_NEXT);
    wait_until(
        &mut removal,
        "the removal to finish",
        FINISHES_WITHIN,
        || visible(window, ID_FINISH_BODY),
    );
    assert!(
        !visible(window, ID_FINISH_LAUNCH),
        "an uninstall offers nothing to start"
    );
    click(window, ID_NEXT);
    let result = removal.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert!(result.json()["launch"].is_null(), "{}", result.stdout);
    assert_nothing_launched(
        &machine,
        "neither a repair nor an uninstall starts the program",
    );
}

/// A package can offer the program unchecked; the person's choice is what
/// Finish acts on, and a cleared box starts nothing — the outcome says it
/// was declined.
#[test]
fn an_unchecked_offer_starts_nothing() {
    if skip_without_desktop() {
        return;
    }
    let dir = common::scratch("wizard-launch-unchecked");
    let program = std::fs::read(common::launch_executable()).unwrap();
    let installer = build_launch_package(&dir, "1.0.0", false, &program);
    let mut machine = Machine::new("wizard-launch-unchecked");

    let (run, window) = install_to_completion(&mut machine, &installer, &[]);
    assert!(
        visible(window, ID_FINISH_LAUNCH) && !checked(window, ID_FINISH_LAUNCH),
        "the offer starts unchecked, as the package declares"
    );
    click(window, ID_NEXT);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["launch"]["status"], "declined", "{outcome}");
    assert!(outcome["launch"]["pid"].is_null(), "{outcome}");
    assert!(result.log_has("[launch_declined]"), "{}", result.log_text());
    assert_nothing_launched(&machine, "a cleared box starts nothing");
}

/// A program that cannot start is reported to the person as a program that
/// did not start. The installation that already committed stands: the run
/// still ends installed with exit 0, and the installation verifies.
#[test]
fn a_program_that_will_not_start_is_reported_and_the_installation_stands() {
    if skip_without_desktop() {
        return;
    }
    let dir = common::scratch("wizard-launch-failed");
    let installer = build_launch_package(&dir, "1.0.0", true, b"not a program");
    let mut machine = Machine::new("wizard-launch-failed");

    let (mut run, window) = install_to_completion(&mut machine, &installer, &[]);
    assert!(checked(window, ID_FINISH_LAUNCH));
    click(window, ID_NEXT);
    // The person is told, in a box of the wizard's own that only informs:
    // its text is painted, so the words are the catalogue's
    // (`ui.error.launch_failed`), and what is checked here is that the box
    // is there and asks nothing.
    let question = wait_for_window(&mut run, QUESTION_CLASS);
    assert_eq!(title_of(question), title_of(window));
    assert_eq!(name_of(control(question, ID_PRIMARY)), "OK");
    assert!(!visible(question, ID_CANCEL), "a report, not a question");
    click(question, ID_PRIMARY);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    let outcome = result.json();
    assert_eq!(outcome["outcome"], "installed", "{outcome}");
    assert_eq!(outcome["code"], "ok", "{outcome}");
    assert_eq!(outcome["launch"]["status"], "failed", "{outcome}");
    assert_eq!(outcome["launch"]["code"], "launch_failed", "{outcome}");
    assert!(result.log_has("[launch_failed]"), "{}", result.log_text());
    let verify = machine.run(&installer, &["verify", "--scope", "user"]);
    assert_eq!(verify.exit_code, Some(0), "{}", verify.stdout);
}

/// Nothing but a successful interactive run ever starts the program: not a
/// run that rolled back, not one cancelled before it began, and never a
/// quiet one, whose unattended caller did not ask for a window.
#[test]
fn a_failed_cancelled_or_quiet_run_never_starts_the_program() {
    if skip_without_desktop() {
        return;
    }
    let dir = common::scratch("wizard-launch-never");
    let program = std::fs::read(common::launch_executable()).unwrap();
    let installer = build_launch_package(&dir, "1.0.0", true, &program);
    let mut machine = Machine::new("wizard-launch-never");

    let (run, window) =
        install_to_completion(&mut machine, &installer, &["--fault", "before_commit:fail"]);
    assert!(
        !visible(window, ID_FINISH_LAUNCH),
        "a run that rolled back offers nothing to start"
    );
    click(window, ID_NEXT);
    let result = run.finish();
    assert_eq!(result.exit_code, Some(1), "{}", result.stdout);
    assert_eq!(result.json()["outcome"], "rolled_back", "{}", result.stdout);
    assert!(result.json()["launch"].is_null(), "{}", result.stdout);

    let (mut run, window) = interactive(&mut machine, &installer, &[]);
    wait_for_page(
        &mut run,
        window,
        ID_DESTINATION_EDIT,
        "the destination page",
    );
    close(window);
    let result = run.finish();
    assert_eq!(
        result.exit_code,
        Some(exit_cancelled()),
        "{}",
        result.stdout
    );

    let result = machine.run(&installer, &["install", "--quiet", "--scope", "user"]);
    assert_eq!(result.exit_code, Some(0), "{}", result.stdout);
    assert_eq!(result.json()["outcome"], "installed", "{}", result.stdout);
    assert!(result.json()["launch"].is_null(), "{}", result.stdout);
    assert_nothing_launched(
        &machine,
        "a rolled-back, a cancelled and a quiet run start nothing",
    );
}
