//! The wizard window: one top-level window that owns every control of every
//! page, showing one page's controls and hiding the rest.
//!
//! It decides only what the person asked for — scope, destination, options,
//! accept, cancel — and shows only what the engine reports. There is no
//! installation logic here: no file, registry or PATH code, no planning. The
//! engine runs on a worker thread and reaches this window as posted events.

use std::collections::BTreeMap;
use std::mem::{size_of, zeroed};
use std::path::PathBuf;

use tigersetup_engine::format::identity::Scope;
use tigersetup_engine::report::{Outcome, Phase, exit};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_TOP, DT_WORDBREAK,
    DeleteObject, EndPaint, FillRect, HDC, HFONT, InvalidateRect, PAINTSTRUCT, SelectObject,
    SetBkColor, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::{
    SS_CENTERIMAGE, SS_ICON, SS_LEFT, SS_PATHELLIPSIS, SS_REALSIZECONTROL,
};
use windows_sys::Win32::UI::Controls::{
    BCM_SETSHIELD, BST_CHECKED, ICC_LINK_CLASS, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, NM_CLICK, NM_RETURN, NMHDR, PBM_SETPOS,
    PBM_SETRANGE32, PBS_SMOOTH,
};
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, SetFocus, VK_SHIFT, VK_TAB,
};
use windows_sys::Win32::UI::Shell::SIID_FOLDER;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::dialog;
use super::glyph::{self, Glyph};
use super::icon;
use super::layout::{self, Rect, scale};
use super::session::{self, FlowKind, Page, ScopePage, Session};
use super::text::{BRAND, Text};
use super::theme::{self, Theme};
use super::win::{self, wide};
use super::worker::{
    self, EngineEvent, WM_ELEVATION_DONE, WM_ELEVATION_STARTED, WM_ENGINE_DONE, WM_ENGINE_EVENT,
    WM_ENGINE_QUESTION,
};
use crate::Operation;

const CLASS_NAME: &str = "TigerSetupWizard";

// Control identifiers. They double as UI Automation ids for these
// window-backed controls, so they are stable and grouped by page rather than
// arbitrary. The lab's driver addresses controls by name; a test addresses
// them by these.
const ID_BACK: i32 = 100;
const ID_NEXT: i32 = 101;
const ID_BROWSE: i32 = 102;
/// Escape maps to the system's cancel identifier, so the Cancel button
/// carries it.
const ID_CANCEL: i32 = IDCANCEL;

const ID_SCOPE_BODY: i32 = 110;
const ID_SCOPE_USER: i32 = 111;
const ID_SCOPE_MACHINE: i32 = 112;

const ID_LICENSE_BODY: i32 = 120;
const ID_LICENSE_TEXT: i32 = 121;
const ID_LICENSE_ACCEPT: i32 = 122;
const ID_LICENSE_DECLINE: i32 = 123;

const ID_DESTINATION_ICON: i32 = 130;
const ID_DESTINATION_BODY: i32 = 131;
const ID_DESTINATION_HINT: i32 = 132;
const ID_DESTINATION_EDIT: i32 = 133;
const ID_DESTINATION_SPACE: i32 = 134;

const ID_OPTIONS_BODY: i32 = 140;

const ID_READY_BODY: i32 = 150;
const ID_READY_SUMMARY: i32 = 151;

const ID_PROGRESS_STATUS: i32 = 160;
const ID_PROGRESS_BAR: i32 = 161;

const ID_FINISH_BODY: i32 = 170;
const ID_FINISH_LAUNCH: i32 = 171;
/// The "Copy log path" link. A log path is long, and static text cannot be
/// selected, so the page offers the path to the clipboard instead of
/// showing it.
const ID_FINISH_LOG: i32 = 172;

const ID_CONFIRM_BODY: i32 = 180;

/// The first declared option's check box; the rest follow it in order.
const ID_OPTION_FIRST: i32 = 200;

const PROGRESS_RANGE: i32 = 1000;

struct Control {
    id: i32,
    hwnd: HWND,
    /// `None` for a control that belongs to the frame rather than a page.
    page: Option<Page>,
    rect: Rect,
    /// Read-only text boxes keep the window background; everything else sits
    /// on the face colour of the content band.
    window_coloured: bool,
    multiline: bool,
}

/// Where a run is, as the window sees it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunState {
    NotStarted,
    Running,
    Finished,
}

pub struct Wizard {
    hwnd: HWND,
    instance: HINSTANCE,
    dpi: u32,
    font: HFONT,
    font_bold: HFONT,
    product_icon: HICON,
    brand_icon: HICON,
    controls: Vec<Control>,
    session: Session,
    text: Text,
    /// The palette the window paints with, which follows the person's Windows
    /// theme. Owned here because a plain Win32 application is not repainted
    /// for the dark preference by Windows itself.
    theme: Theme,
    page: usize,
    accepted: bool,
    run: RunState,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// True while an elevated relaunch is in flight: the UAC prompt is up or
    /// the elevated child is running. The window keeps pumping messages
    /// throughout — the wait for the child is on `elevation_worker`, never on
    /// this thread — so it stays responsive rather than "Not Responding".
    elevating: bool,
    elevation_worker: Option<std::thread::JoinHandle<()>>,
    outcome: Option<Outcome>,
    /// The outcome document an elevated child wrote, kept verbatim: it is
    /// that run's own report, and re-serialising it here could only lose
    /// something.
    elevated_document: Option<String>,
    status: String,
    exit_code: i32,
}

/// What the wizard reports back to the command-line client.
pub struct Completed {
    pub exit_code: i32,
    pub outcome: Option<Outcome>,
    /// Set instead of `outcome` when an elevated child did the work.
    pub elevated_document: Option<String>,
}

/// Shows the wizard for `session` and returns once the window has closed.
pub fn show(session: Session) -> Completed {
    unsafe {
        let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let icc = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_PROGRESS_CLASS | ICC_LINK_CLASS,
        };
        InitCommonControlsEx(&icc);

        let instance = GetModuleHandleW(std::ptr::null());
        register_class(instance);

        let text = Text::new(session.lang(), &session.product_name);
        let title = text.get(session.title_key());
        let title_w = wide(&title);
        let class = wide(CLASS_NAME);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title_w.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            layout::CLIENT_W,
            layout::CLIENT_H,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Completed {
                exit_code: exit::ROLLED_BACK,
                outcome: Some(worker::start_failed(
                    "the installer window could not be created".into(),
                )),
                elevated_document: None,
            };
        }

        // A run the engine refused before it could start is the whole
        // outcome already: the wizard opens on its completion page with it.
        let refusal = session.refusal.clone();
        let dpi = GetDpiForWindow(hwnd);
        let mut wizard = Box::new(Wizard {
            hwnd,
            instance,
            dpi,
            font: std::ptr::null_mut(),
            font_bold: std::ptr::null_mut(),
            product_icon: std::ptr::null_mut(),
            brand_icon: std::ptr::null_mut(),
            controls: Vec::new(),
            text,
            session,
            theme: Theme::current(),
            page: 0,
            accepted: false,
            run: match refusal {
                Some(_) => RunState::Finished,
                None => RunState::NotStarted,
            },
            cancel: None,
            worker: None,
            elevating: false,
            elevation_worker: None,
            exit_code: refusal
                .as_ref()
                .map_or(exit::CANCELLED, |outcome| outcome.exit_code),
            outcome: refusal,
            elevated_document: None,
            status: String::new(),
        });
        theme::apply_to_frame(hwnd, wizard.theme.mode);
        wizard.create_fonts_and_icons();
        wizard.create_controls();
        let wizard_ptr = Box::into_raw(wizard);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, wizard_ptr as isize);

        let wizard = &mut *wizard_ptr;
        wizard.size_window_to_client();
        wizard.apply_layout();
        wizard.enter_page(0);

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            // Alt+letter is resolved here rather than left to
            // `IsDialogMessageW`, which only moves the focus for a window
            // that is not a dialog box. A mnemonic must activate its
            // control, because that is what the keyboard walk relies on. A
            // button activates itself; the link's activation is this
            // window's own.
            if msg.message == WM_SYSCHAR {
                let handles: Vec<HWND> = wizard.page_controls();
                if let Some(control) = win::activate_mnemonic(&handles, msg.wParam as u32) {
                    if control == wizard.control(ID_FINISH_LOG) {
                        wizard.copy_log_path();
                    }
                    continue;
                }
            }
            // A multiline edit control asks for every key, Tab included, when
            // its parent is not a dialog box, which traps the focus inside
            // the licence and summary boxes. Move it on here instead.
            if msg.message == WM_KEYDOWN
                && msg.wParam as u32 == VK_TAB as u32
                && wizard.move_focus_out_of_text_box()
            {
                continue;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let wizard = Box::from_raw(wizard_ptr);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let completed = Completed {
            exit_code: wizard.exit_code,
            outcome: wizard.outcome.clone(),
            elevated_document: wizard.elevated_document.clone(),
        };
        if let Some(worker) = wizard.worker {
            let _ = worker.join();
        }
        // The window only closes on an elevated relaunch once its result has
        // arrived and been consumed, so this thread has already finished; the
        // join just reclaims it.
        if let Some(worker) = wizard.elevation_worker {
            let _ = worker.join();
        }
        DeleteObject(wizard.font as _);
        DeleteObject(wizard.font_bold as _);
        completed
    }
}

/// The markup a link control wants: the label, and nothing else, is the
/// link. The mnemonic marker stays, because the control reads it as a
/// button does.
fn link_markup(label: &str) -> String {
    format!("<a>{label}</a>")
}

unsafe fn register_class(instance: HINSTANCE) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let class = wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(instance, icon::EXECUTABLE_ICON_ID as usize as *const u16),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
    });
}

impl Wizard {
    fn page(&self) -> Page {
        self.session.pages[self.page]
    }

    fn control(&self, id: i32) -> HWND {
        self.controls
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.hwnd)
            .unwrap_or(std::ptr::null_mut())
    }

    /// Every control the current page shows, plus the frame's buttons, in
    /// tab order.
    fn page_controls(&self) -> Vec<HWND> {
        let page = self.page();
        self.controls
            .iter()
            .filter(|c| c.page.is_none_or(|p| p == page))
            .map(|c| c.hwnd)
            .collect()
    }

    unsafe fn create_fonts_and_icons(&mut self) {
        unsafe {
            if !self.font.is_null() {
                DeleteObject(self.font as _);
                DeleteObject(self.font_bold as _);
                DestroyIcon(self.product_icon);
                DestroyIcon(self.brand_icon);
            }
            let (font, bold) = win::message_fonts(self.dpi);
            self.font = font;
            self.font_bold = bold;

            let header_px = scale(layout::HEADER_IMAGE.w, self.dpi);
            self.product_icon = icon::product(&self.session.icon_bytes, self.instance, header_px);
            self.brand_icon = icon::brand(self.instance, scale(layout::BRAND_ICON.w, self.dpi));

            // The title bar and the taskbar show the executable's icon, the
            // same one Explorer shows for the file.
            let big = GetSystemMetricsForDpi(SM_CXICON, self.dpi);
            let small = GetSystemMetricsForDpi(SM_CXSMICON, self.dpi);
            let icon_big = icon::executable(self.instance, big);
            let icon_small = icon::executable(self.instance, small);
            SendMessageW(
                self.hwnd,
                WM_SETICON,
                ICON_BIG as WPARAM,
                icon_big as LPARAM,
            );
            SendMessageW(
                self.hwnd,
                WM_SETICON,
                ICON_SMALL as WPARAM,
                icon_small as LPARAM,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn add(
        &mut self,
        class: &str,
        label: &str,
        style: u32,
        ex_style: u32,
        id: i32,
        page: Option<Page>,
        rect: Rect,
        window_coloured: bool,
    ) -> HWND {
        unsafe {
            let multiline = class.eq_ignore_ascii_case("EDIT") && style & ES_MULTILINE as u32 != 0;
            let class_w = wide(class);
            let label_w = wide(label);
            let hwnd = CreateWindowExW(
                ex_style,
                class_w.as_ptr(),
                label_w.as_ptr(),
                WS_CHILD | style,
                0,
                0,
                10,
                10,
                self.hwnd,
                id as isize as HMENU,
                self.instance,
                std::ptr::null(),
            );
            theme::apply_to_control(hwnd, class, self.theme.mode);
            self.controls.push(Control {
                id,
                hwnd,
                page,
                rect,
                window_coloured,
                multiline,
            });
            hwnd
        }
    }

    unsafe fn create_controls(&mut self) {
        unsafe {
            const STATIC: &str = "STATIC";
            const BUTTON: &str = "BUTTON";
            const EDIT: &str = "EDIT";
            const PROGRESS: &str = "msctls_progress32";
            const LINK: &str = "SysLink";

            // Every control starts a new group except the second member of a
            // radio pair, so that the arrow keys move within a radio group
            // and stop at its end instead of running into the next page.
            let label: u32 = SS_LEFT | WS_GROUP;
            let icon_label: u32 = SS_ICON | SS_REALSIZECONTROL | SS_CENTERIMAGE | WS_GROUP;
            // A path is one long word that no line break can help, so the
            // controls that show one shorten it in the middle instead.
            let path_label: u32 = SS_LEFT | SS_PATHELLIPSIS | WS_GROUP;
            let radio_first: u32 =
                (BS_AUTORADIOBUTTON | BS_MULTILINE) as u32 | WS_GROUP | WS_TABSTOP;
            let radio_next: u32 = (BS_AUTORADIOBUTTON | BS_MULTILINE) as u32;
            let check: u32 = (BS_AUTOCHECKBOX | BS_MULTILINE) as u32 | WS_GROUP | WS_TABSTOP;
            let push: u32 = BS_PUSHBUTTON as u32 | WS_GROUP | WS_TABSTOP;
            let default_push: u32 = BS_DEFPUSHBUTTON as u32 | WS_GROUP | WS_TABSTOP;
            let text_box: u32 =
                (ES_MULTILINE | ES_READONLY) as u32 | WS_VSCROLL | WS_GROUP | WS_TABSTOP;
            let path_box: u32 = ES_AUTOHSCROLL as u32 | WS_GROUP | WS_TABSTOP;
            let link: u32 = WS_GROUP | WS_TABSTOP;
            let sunken: u32 = WS_EX_CLIENTEDGE;

            if let Some(scope_page) = self.session.scope_page.clone() {
                let page = Some(Page::Scope);
                // The page says one of three things: choose a scope for a
                // first install, this is the installation the run continues
                // with, or choose between the two installations found. Only
                // the choices carry radio buttons.
                let (body, radios) = match &scope_page {
                    ScopePage::Choose => (
                        self.text.get("ui.scope.body"),
                        Some((
                            self.text.get("ui.scope.user"),
                            self.text.get("ui.scope.machine"),
                        )),
                    ),
                    ScopePage::Existing(existing) => (
                        self.describe_installation(
                            match existing.scope() {
                                Scope::Machine => "ui.scope.existing.machine",
                                Scope::User => "ui.scope.existing.user",
                            },
                            existing,
                        ),
                        None,
                    ),
                    ScopePage::Select(installations) => {
                        let label = |scope: Scope, key: &str| {
                            installations
                                .iter()
                                .find(|i| i.scope() == scope)
                                .map(|i| self.describe_installation(key, i))
                                .unwrap_or_default()
                        };
                        (
                            self.text.get("ui.scope.select.body"),
                            Some((
                                label(Scope::User, "ui.scope.select.user"),
                                label(Scope::Machine, "ui.scope.select.machine"),
                            )),
                        )
                    }
                };
                self.add(
                    STATIC,
                    &body,
                    label,
                    0,
                    ID_SCOPE_BODY,
                    page,
                    layout::SCOPE_BODY,
                    false,
                );
                if let Some((user, machine)) = radios {
                    self.add(
                        BUTTON,
                        &user,
                        radio_first,
                        0,
                        ID_SCOPE_USER,
                        page,
                        layout::SCOPE_USER,
                        false,
                    );
                    self.add(
                        BUTTON,
                        &machine,
                        radio_next,
                        0,
                        ID_SCOPE_MACHINE,
                        page,
                        layout::SCOPE_MACHINE,
                        false,
                    );
                    // The package's default scope is the one already chosen,
                    // so the page can be passed with Next alone.
                    let default_scope = match self.session.scope {
                        Scope::Machine => ID_SCOPE_MACHINE,
                        Scope::User => ID_SCOPE_USER,
                    };
                    SendMessageW(
                        self.control(default_scope),
                        BM_SETCHECK,
                        BST_CHECKED as WPARAM,
                        0,
                    );
                }
                // The elevation affordance is the native shield Windows draws
                // on the button that actually raises the prompt — the Next
                // button — not a drawn glyph beside the radio. `update_buttons`
                // places it from the selected scope, so nothing is done here.
            }

            if self.session.has(Page::License) {
                let page = Some(Page::License);
                let body = self.text.get("ui.license.body");
                let accept = self.text.get("ui.license.accept");
                let decline = self.text.get("ui.license.decline");
                let licence = self
                    .session
                    .license_text
                    .replace("\r\n", "\n")
                    .replace('\n', "\r\n");
                self.add(
                    STATIC,
                    &body,
                    label,
                    0,
                    ID_LICENSE_BODY,
                    page,
                    layout::LICENSE_BODY,
                    false,
                );
                self.add(
                    EDIT,
                    &licence,
                    text_box,
                    sunken,
                    ID_LICENSE_TEXT,
                    page,
                    layout::LICENSE_TEXT,
                    true,
                );
                self.add(
                    BUTTON,
                    &accept,
                    radio_first,
                    0,
                    ID_LICENSE_ACCEPT,
                    page,
                    layout::LICENSE_ACCEPT,
                    false,
                );
                self.add(
                    BUTTON,
                    &decline,
                    radio_next,
                    0,
                    ID_LICENSE_DECLINE,
                    page,
                    layout::LICENSE_DECLINE,
                    false,
                );
                SendMessageW(
                    self.control(ID_LICENSE_DECLINE),
                    BM_SETCHECK,
                    BST_CHECKED as WPARAM,
                    0,
                );
            }

            if self.session.has(Page::Destination) {
                let page = Some(Page::Destination);
                let body = self.text.get("ui.destination.body");
                let hint = self.text.get("ui.destination.hint");
                let browse = self.text.get("ui.button.browse");
                let root = self.session.install_root.display().to_string();
                self.add(
                    STATIC,
                    "",
                    icon_label,
                    0,
                    ID_DESTINATION_ICON,
                    page,
                    layout::DESTINATION_ICON,
                    false,
                );
                self.add(
                    STATIC,
                    &body,
                    label,
                    0,
                    ID_DESTINATION_BODY,
                    page,
                    layout::DESTINATION_BODY,
                    false,
                );
                self.add(
                    STATIC,
                    &hint,
                    label,
                    0,
                    ID_DESTINATION_HINT,
                    page,
                    layout::DESTINATION_HINT,
                    false,
                );
                self.add(
                    EDIT,
                    &root,
                    path_box,
                    sunken,
                    ID_DESTINATION_EDIT,
                    page,
                    layout::DESTINATION_EDIT,
                    true,
                );
                self.add(
                    BUTTON,
                    &browse,
                    push,
                    0,
                    ID_BROWSE,
                    page,
                    layout::DESTINATION_BROWSE,
                    false,
                );
                self.add(
                    STATIC,
                    "",
                    label,
                    0,
                    ID_DESTINATION_SPACE,
                    page,
                    layout::DESTINATION_SPACE,
                    false,
                );
                win::set_stock_icon(self.control(ID_DESTINATION_ICON), SIID_FOLDER);
            }

            if self.session.has(Page::Options) {
                let page = Some(Page::Options);
                let body = self.text.get("ui.options.body");
                self.add(
                    STATIC,
                    &body,
                    label,
                    0,
                    ID_OPTIONS_BODY,
                    page,
                    layout::OPTIONS_BODY,
                    false,
                );
                for index in 0..self.session.options.len() {
                    let option = &self.session.options[index];
                    let (label_text, checked) = (option.label.clone(), option.checked);
                    let control = self.add(
                        BUTTON,
                        &label_text,
                        check,
                        0,
                        ID_OPTION_FIRST + index as i32,
                        page,
                        layout::option_row(index),
                        false,
                    );
                    if checked {
                        SendMessageW(control, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
                    }
                }
            }

            if self.session.has(Page::Ready) {
                let page = Some(Page::Ready);
                let body = self.ready_body();
                self.add(
                    STATIC,
                    &body,
                    label,
                    0,
                    ID_READY_BODY,
                    page,
                    layout::READY_BODY,
                    false,
                );
                self.add(
                    EDIT,
                    "",
                    text_box,
                    sunken,
                    ID_READY_SUMMARY,
                    page,
                    layout::READY_SUMMARY,
                    true,
                );
            }

            if self.session.has(Page::Confirm) {
                let page = Some(Page::Confirm);
                let body = self.text.get("ui.uninstall.confirm");
                self.add(
                    STATIC,
                    &body,
                    label,
                    0,
                    ID_CONFIRM_BODY,
                    page,
                    layout::CONFIRM_BODY,
                    false,
                );
            }

            let page = Some(Page::Progress);
            self.add(
                STATIC,
                "",
                path_label,
                0,
                ID_PROGRESS_STATUS,
                page,
                layout::PROGRESS_STATUS,
                false,
            );
            self.add(
                PROGRESS,
                "",
                PBS_SMOOTH,
                0,
                ID_PROGRESS_BAR,
                page,
                layout::PROGRESS_BAR,
                false,
            );
            SendMessageW(
                self.control(ID_PROGRESS_BAR),
                PBM_SETRANGE32,
                0,
                PROGRESS_RANGE as LPARAM,
            );

            let page = Some(Page::Finish);
            self.add(
                STATIC,
                "",
                label,
                0,
                ID_FINISH_BODY,
                page,
                layout::FINISH_BODY,
                false,
            );
            if self.session.start_menu_target.is_some() {
                let launch = self.text.get("ui.finish.launch");
                let control = self.add(
                    BUTTON,
                    &launch,
                    check,
                    0,
                    ID_FINISH_LAUNCH,
                    page,
                    layout::FINISH_LAUNCH,
                    false,
                );
                SendMessageW(control, BM_SETCHECK, BST_CHECKED as WPARAM, 0);
            }
            // The link is created with its text so that its width can be
            // fitted with the layout; whether the page shows it is decided
            // by the outcome, which has a log path or has not.
            let copy_log = link_markup(&self.text.get("ui.finish.copy_log"));
            self.add(
                LINK,
                &copy_log,
                link,
                0,
                ID_FINISH_LOG,
                page,
                layout::FINISH_LOG,
                false,
            );

            // The navigation buttons come last so that they come last in the
            // tab order, after whichever page's controls are visible.
            let back = self.text.get("ui.button.back");
            let next = self.text.get("ui.button.next");
            let cancel = self.text.get("ui.button.cancel");
            self.add(
                BUTTON,
                &back,
                push,
                0,
                ID_BACK,
                None,
                layout::BTN_BACK,
                false,
            );
            self.add(
                BUTTON,
                &next,
                default_push,
                0,
                ID_NEXT,
                None,
                layout::BTN_NEXT,
                false,
            );
            self.add(
                BUTTON,
                &cancel,
                push,
                0,
                ID_CANCEL,
                None,
                layout::BTN_CANCEL,
                false,
            );
        }
    }

    /// Resizes the frame so that the client area is exactly the design size
    /// at the current dpi, and centres it on the work area.
    unsafe fn size_window_to_client(&self) {
        unsafe {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: scale(layout::CLIENT_W, self.dpi),
                bottom: scale(layout::CLIENT_H, self.dpi),
            };
            let style = GetWindowLongPtrW(self.hwnd, GWL_STYLE) as u32;
            let ex_style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            AdjustWindowRectExForDpi(&mut rect, style, 0, ex_style, self.dpi);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            let mut work: RECT = zeroed();
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                &mut work as *mut _ as *mut std::ffi::c_void,
                0,
            );
            let x = work.left + ((work.right - work.left) - width) / 2;
            let y = work.top + ((work.bottom - work.top) - height) / 2;
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                x.max(work.left),
                y.max(work.top),
                width,
                height,
                SWP_NOZORDER,
            );
        }
    }

    /// Places every control at its scaled position and gives it the current
    /// font.
    /// Repaints everything the theme reaches after the person changed it
    /// while the wizard was open: the frame, every control's own drawing, and
    /// the window's own painting.
    unsafe fn apply_theme(&mut self) {
        unsafe {
            theme::apply_to_frame(self.hwnd, self.theme.mode);
            for control in &self.controls {
                let mut class = [0u16; 32];
                let length = GetClassNameW(control.hwnd, class.as_mut_ptr(), class.len() as i32);
                let class = String::from_utf16_lossy(&class[..length.max(0) as usize]);
                theme::apply_to_control(control.hwnd, &class, self.theme.mode);
                InvalidateRect(control.hwnd, std::ptr::null(), 1);
            }
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
            UpdateWindow(self.hwnd);
        }
    }

    unsafe fn apply_layout(&mut self) {
        unsafe {
            for control in &self.controls {
                let r = control.rect.scaled(self.dpi);
                SetWindowPos(
                    control.hwnd,
                    std::ptr::null_mut(),
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                SendMessageW(control.hwnd, WM_SETFONT, self.font as WPARAM, 1);
            }
            self.fit_log_link();
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
        }
    }

    /// Sizes the log link to its text at the current dpi, so that the
    /// control is exactly the link.
    unsafe fn fit_log_link(&self) {
        let control = self.control(ID_FINISH_LOG);
        if !control.is_null() {
            let rect = layout::FINISH_LOG.scaled(self.dpi);
            win::fit_link(control, rect.w, rect.h);
        }
    }

    /// Gives the log link its text: the catalog entry as link markup. New
    /// text is a new link item, which starts in the control's default
    /// colours, so the theme is applied to it again, and the control is
    /// refitted to the new width.
    fn set_log_link_text(&self, key: &str) {
        let link = self.control(ID_FINISH_LOG);
        win::set_text(link, &link_markup(&self.text.get(key)));
        theme::apply_to_control(link, "SysLink", self.theme.mode);
        unsafe { self.fit_log_link() };
    }

    unsafe fn move_focus_out_of_text_box(&self) -> bool {
        unsafe {
            let focus = GetFocus();
            let inside = self
                .controls
                .iter()
                .any(|c| c.hwnd == focus && c.window_coloured && c.multiline);
            if !inside {
                return false;
            }
            let backwards = GetKeyState(VK_SHIFT as i32) < 0;
            let next = GetNextDlgTabItem(self.hwnd, focus, backwards as i32);
            if !next.is_null() {
                SetFocus(next);
            }
            true
        }
    }

    fn is_checked(&self, id: i32) -> bool {
        unsafe { SendMessageW(self.control(id), BM_GETCHECK, 0, 0) as u32 == BST_CHECKED }
    }

    unsafe fn enter_page(&mut self, index: usize) {
        unsafe {
            self.page = index;
            let page = self.page();
            for control in &self.controls {
                if let Some(owner) = control.page {
                    ShowWindow(control.hwnd, if owner == page { SW_SHOW } else { SW_HIDE });
                }
            }

            match page {
                Page::Destination => self.update_free_space(),
                Page::Ready => {
                    let summary = self.summary_text();
                    win::set_text(self.control(ID_READY_SUMMARY), &summary);
                }
                Page::Progress => {
                    self.status = self.text.get("ui.progress.preparing");
                    win::set_text(self.control(ID_PROGRESS_STATUS), &self.status);
                    SendMessageW(self.control(ID_PROGRESS_BAR), PBM_SETPOS, 0, 0);
                    self.start_run();
                }
                Page::Finish => self.show_outcome(),
                _ => {}
            }

            self.update_buttons();
            InvalidateRect(self.hwnd, std::ptr::null(), 1);

            let focus = match page {
                Page::Scope => ID_SCOPE_USER,
                Page::License => ID_LICENSE_DECLINE,
                Page::Destination => ID_DESTINATION_EDIT,
                Page::Options => ID_OPTION_FIRST,
                _ => ID_NEXT,
            };
            let control = self.control(focus);
            SetFocus(if control.is_null() {
                self.control(ID_NEXT)
            } else {
                control
            });
        }
    }

    unsafe fn update_buttons(&self) {
        unsafe {
            let page = self.page();
            let back = self.control(ID_BACK);
            let next = self.control(ID_NEXT);
            let cancel = self.control(ID_CANCEL);

            let first_interactive = self.page == 0;
            let back_visible = !first_interactive
                && !matches!(page, Page::Progress | Page::Finish | Page::Confirm);
            ShowWindow(back, if back_visible { SW_SHOW } else { SW_HIDE });
            ShowWindow(next, SW_SHOW);
            let cancel_visible = page != Page::Finish;
            ShowWindow(cancel, if cancel_visible { SW_SHOW } else { SW_HIDE });

            let next_key = match page {
                Page::Ready => "ui.button.install",
                Page::Confirm => "ui.button.yes",
                Page::Finish => "ui.button.finish",
                _ => "ui.button.next",
            };
            win::set_text(next, &self.text.get(next_key));
            let cancel_key = match page {
                Page::Confirm => "ui.button.no",
                _ => "ui.button.cancel",
            };
            win::set_text(cancel, &self.text.get(cancel_key));

            // A busy page keeps its forward button visible but disabled, so
            // that an automated run waits for it instead of racing a control
            // that appears late. An elevated relaunch in flight is busy in the
            // same way: the page's controls are frozen until the elevated
            // child reports back, and only Cancel of the *child* answers it.
            let next_enabled = !self.elevating
                && match page {
                    Page::License => self.accepted,
                    Page::Progress => false,
                    _ => true,
                };
            EnableWindow(next, next_enabled as i32);
            EnableWindow(
                cancel,
                (!self.elevating && (page != Page::Progress || self.run == RunState::Running))
                    as i32,
            );
            if back_visible {
                EnableWindow(back, (!self.elevating) as i32);
            }
            self.scope_controls_enabled(!self.elevating);
            self.set_next_shield(page == Page::Scope && self.needs_elevation());
            InvalidateRect(next, std::ptr::null(), 1);
        }
    }

    /// Puts the native Windows elevation shield on the Next button, or takes
    /// it off. The control that raises the UAC prompt is Next — selecting a
    /// scope decides nothing on its own — so the shield lives here, drawn by
    /// the themed button itself. Because it is the button's own state, it
    /// survives hover, focus, repaint, a theme change and a DPI change without
    /// any painting of ours, which a drawn glyph beside the radio did not.
    unsafe fn set_next_shield(&self, on: bool) {
        unsafe {
            SendMessageW(self.control(ID_NEXT), BCM_SETSHIELD, 0, on as LPARAM);
        }
    }

    /// Enables or disables the scope page's radio buttons, so they cannot be
    /// changed while an elevated relaunch they started is still in flight.
    unsafe fn scope_controls_enabled(&self, enabled: bool) {
        unsafe {
            for id in [ID_SCOPE_USER, ID_SCOPE_MACHINE] {
                let control = self.control(id);
                if !control.is_null() {
                    EnableWindow(control, enabled as i32);
                }
            }
        }
    }

    /// The chosen scope: the radio the person checked where the scope page
    /// offers a choice, the installation it names where it does not, and
    /// the session's own scope where there is no scope page at all.
    fn chosen_scope(&self) -> Scope {
        match &self.session.scope_page {
            Some(ScopePage::Existing(existing)) => existing.scope(),
            Some(ScopePage::Choose | ScopePage::Select(_)) => {
                if self.is_checked(ID_SCOPE_MACHINE) {
                    Scope::Machine
                } else {
                    Scope::User
                }
            }
            None => self.session.scope,
        }
    }

    /// The text that names an installation: its version and root, in the
    /// wording `key` gives that scope.
    fn describe_installation(
        &self,
        key: &str,
        installation: &tigersetup_engine::target::ExistingInstallation,
    ) -> String {
        self.text.fill(
            key,
            &[
                ("version", &Text::literal(&installation.version)),
                ("root", &Text::literal(&installation.install_root)),
            ],
        )
    }

    fn chosen_root(&self) -> PathBuf {
        if self.session.has(Page::Destination) {
            PathBuf::from(win::text_of(self.control(ID_DESTINATION_EDIT)).trim())
        } else {
            self.session.install_root.clone()
        }
    }

    /// Moves the destination to the chosen scope's default folder, unless the
    /// person has put their own path there.
    ///
    /// The two pages describe one decision, and "for all users" landing in a
    /// single user's profile is the kind of disagreement a person is entitled
    /// to assume cannot happen. Only a destination that is still one of the
    /// scope defaults is replaced: anything typed or browsed to is the
    /// person's answer and outranks the scope's default.
    unsafe fn follow_scope_in_destination(&mut self) {
        if !self.session.has(Page::Destination) || !self.session.has(Page::Scope) {
            return;
        }
        let Some(wanted) =
            session::default_root_of(&self.session.default_root_for_scope, self.chosen_scope())
        else {
            return;
        };
        let current = PathBuf::from(win::text_of(self.control(ID_DESTINATION_EDIT)).trim());
        if current == wanted {
            return;
        }
        let is_a_default = self
            .session
            .default_root_for_scope
            .iter()
            .any(|(_, root)| *root == current);
        if !is_a_default && !current.as_os_str().is_empty() {
            return;
        }
        win::set_text(
            self.control(ID_DESTINATION_EDIT),
            &wanted.display().to_string(),
        );
        unsafe { self.update_free_space() };
    }

    fn chosen_options(&self) -> BTreeMap<String, bool> {
        let mut options = self.session.explicit_options.clone();
        if self.session.has(Page::Options) {
            for (index, option) in self.session.options.iter().enumerate() {
                options.insert(
                    option.name.clone(),
                    self.is_checked(ID_OPTION_FIRST + index as i32),
                );
            }
        }
        options
    }

    unsafe fn update_free_space(&self) {
        let root = self.chosen_root();
        let required = self.session.estimated_size;
        let message = match win::free_space(&root) {
            Some(free) => self.text.fill(
                "ui.destination.space",
                &[
                    ("required", &self.text.size(required)),
                    ("free", &self.text.size(free)),
                    ("volume", &win::volume_of(&root)),
                ],
            ),
            None => self.text.fill(
                "ui.destination.space_required",
                &[("required", &self.text.size(required))],
            ),
        };
        win::set_text(self.control(ID_DESTINATION_SPACE), &message);
    }

    fn ready_body(&self) -> String {
        match self.session.kind {
            FlowKind::Upgrade => self.text.fill(
                "ui.ready.body.upgrade",
                &[
                    (
                        "from",
                        self.session.installed_version.as_deref().unwrap_or(""),
                    ),
                    ("to", &self.session.product_version),
                ],
            ),
            FlowKind::Reinstall => self.text.fill(
                "ui.ready.body.reinstall",
                &[("version", &self.session.product_version)],
            ),
            FlowKind::Repair => self.text.fill(
                "ui.ready.body.repair",
                &[("version", &self.session.product_version)],
            ),
            _ => self.text.get("ui.ready.body.install"),
        }
    }

    fn summary_text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if self.session.has(Page::Destination) || self.session.kind == FlowKind::Install {
            lines.push(self.text.get("ui.summary.destination"));
            lines.push(format!("    {}", self.chosen_root().display()));
            lines.push(String::new());
        }
        if self.session.has(Page::Scope) {
            lines.push(self.text.get("ui.summary.scope"));
            let key = match self.chosen_scope() {
                Scope::Machine => "ui.summary.scope.machine",
                Scope::User => "ui.summary.scope.user",
            };
            lines.push(format!("    {}", self.text.get(key)));
            lines.push(String::new());
        }
        if !self.session.options.is_empty() {
            lines.push(self.text.get("ui.summary.options"));
            let chosen = self.chosen_options();
            let mut any = false;
            for option in &self.session.options {
                if chosen.get(&option.name).copied().unwrap_or(option.checked) {
                    any = true;
                    lines.push(format!(
                        "    {}",
                        tigersetup_engine::i18n::strip_mnemonics(&option.label)
                    ));
                }
            }
            if !any {
                lines.push(format!("    {}", self.text.get("ui.summary.none")));
            }
            lines.push(String::new());
        }
        if !self.session.absent_dependencies.is_empty() {
            lines.push(self.text.get("ui.summary.dependencies"));
            for dependency in &self.session.absent_dependencies {
                lines.push(format!("    {dependency}"));
            }
        }
        lines.join("\r\n")
    }

    unsafe fn start_run(&mut self) {
        let cancel = worker::cancel_flag();
        let mut options = self.session.base.clone();
        options.scope = self.chosen_scope();
        options.quiet = false;
        options.cancel = Some(cancel.clone());
        options.options = self.chosen_options();
        if self.session.operation == Operation::Install {
            options.install_root = Some(self.chosen_root());
        }
        self.cancel = Some(cancel);
        self.run = RunState::Running;
        self.worker = Some(worker::start(
            self.hwnd,
            self.session.exe.clone(),
            self.session.operation,
            options,
        ));
        unsafe { self.update_buttons() };
    }

    /// Turns one engine event into the status line and the progress bar.
    unsafe fn on_engine_event(&mut self, event: &EngineEvent) {
        let Some(progress) = &event.progress else {
            self.status = match event.code {
                "transaction_rolling_back" => self.text.get("ui.progress.rolling_back"),
                "recovery_started" => self.text.get("ui.progress.recovering"),
                "transaction_committed" => self.text.get("ui.progress.finishing"),
                _ => return,
            };
            win::set_text(self.control(ID_PROGRESS_STATUS), &self.status);
            return;
        };

        self.status = match progress.phase {
            Phase::Dependencies if progress.total > 0 => self.text.fill(
                "ui.progress.dependency_download",
                &[
                    ("dependency", &progress.target),
                    ("done", &self.text.size(progress.done)),
                    ("total", &self.text.size(progress.total)),
                ],
            ),
            Phase::Dependencies => self.text.fill(
                "ui.progress.dependency_install",
                &[("dependency", &progress.target)],
            ),
            Phase::RollingBack => self.text.get("ui.progress.rolling_back"),
            Phase::Recovery => self.text.get("ui.progress.recovering"),
            Phase::Finishing => self.text.get("ui.progress.finishing"),
            _ => {
                let key = match self.session.operation {
                    Operation::Uninstall => "ui.progress.applying.uninstall",
                    _ => "ui.progress.applying.install",
                };
                self.text
                    .fill(key, &[("target", &Text::literal(&progress.target))])
            }
        };
        win::set_text(self.control(ID_PROGRESS_STATUS), &self.status);

        if let Some(position) =
            (progress.done.min(progress.total) * PROGRESS_RANGE as u64).checked_div(progress.total)
        {
            unsafe {
                SendMessageW(
                    self.control(ID_PROGRESS_BAR),
                    PBM_SETPOS,
                    position as WPARAM,
                    0,
                )
            };
        }
    }

    unsafe fn on_engine_done(&mut self, outcome: Outcome) {
        self.run = RunState::Finished;
        self.exit_code = outcome.exit_code;
        self.outcome = Some(outcome);
        let finish = self
            .session
            .pages
            .iter()
            .position(|page| *page == Page::Finish)
            .unwrap_or(self.page);
        unsafe { self.enter_page(finish) };
    }

    /// Fills the completion page from the outcome the engine reported.
    unsafe fn show_outcome(&mut self) {
        let outcome = self.outcome.clone();
        let body = match &outcome {
            Some(outcome) if outcome.outcome == "installed" => {
                let mut body = self.text.fill(
                    "ui.finish.body.installed",
                    &[("version", &self.session.product_version)],
                );
                if outcome.reboot_required || outcome.exit_code == exit::REBOOT_REQUIRED {
                    body.push_str("\r\n\r\n");
                    body.push_str(&self.text.get("ui.finish.reboot"));
                }
                body
            }
            Some(outcome) if outcome.outcome == "uninstalled" => {
                self.text.get("ui.finish.body.uninstalled")
            }
            Some(outcome) => Text::literal(&outcome.message),
            None => self.text.get("ui.finish.subtitle.failed"),
        };
        win::set_text(self.control(ID_FINISH_BODY), &body);

        // The log is offered, never shown: the link puts the exact path on
        // the clipboard. A run that wrote no log offers nothing.
        let link = self.control(ID_FINISH_LOG);
        self.set_log_link_text("ui.finish.copy_log");
        unsafe {
            ShowWindow(
                link,
                if self.log_path().is_some() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }

        let launch = self.control(ID_FINISH_LAUNCH);
        if !launch.is_null() {
            // Never offered from an elevated run: starting the product from
            // here would hand it this process's administrator token, and the
            // application would run elevated for the rest of the session.
            let installed = outcome
                .as_ref()
                .is_some_and(|outcome| outcome.outcome == "installed")
                && !self.session.elevated;
            unsafe { ShowWindow(launch, if installed { SW_SHOW } else { SW_HIDE }) };
        }
    }

    /// The log the outcome names, when it names one.
    fn log_path(&self) -> Option<&str> {
        self.outcome
            .as_ref()
            .and_then(|outcome| outcome.log.as_deref())
            .filter(|path| !path.is_empty())
    }

    /// Puts the exact log path on the clipboard and says so on the link —
    /// only once the clipboard has it. A copy that failed leaves the link
    /// as it was, offering to try again, rather than claiming anything.
    fn copy_log_path(&self) {
        let Some(path) = self.log_path() else {
            return;
        };
        if win::set_clipboard_text(self.hwnd, path) {
            self.set_log_link_text("ui.finish.log_copied");
        }
    }

    /// Starts the application the package's Start Menu shortcut points at.
    /// Only an installation that succeeded has anything to launch, so a run
    /// that failed or was cancelled starts nothing whatever the check box
    /// was left at.
    fn launch_product(&self) {
        // An elevated run must not start the product: under same-account
        // elevation the child would inherit the administrator token and the
        // application would run elevated for the whole session. The check box
        // is hidden in that case; this is the second guard.
        if self.session.elevated {
            return;
        }
        let Some(target) = &self.session.start_menu_target else {
            return;
        };
        let Some(installation) = self
            .outcome
            .as_ref()
            .filter(|outcome| outcome.outcome == "installed")
            .and_then(|outcome| outcome.installation.as_ref())
        else {
            return;
        };
        let launch = self.control(ID_FINISH_LAUNCH);
        if launch.is_null() || !self.is_checked(ID_FINISH_LAUNCH) {
            return;
        }
        let root = PathBuf::from(&installation.install_root);
        win::launch(&root.join(target.replace('/', "\\")));
    }

    fn title(&self) -> String {
        self.text.get(self.session.title_key())
    }

    /// Asks the person to confirm, in the installer's language, with the
    /// wizard's own title so that an automated run can find the box.
    /// Asks whether the applications holding the product's files may be
    /// closed. The engine will not touch anything until this is answered,
    /// which is the whole reason it is a question and not an event.
    unsafe fn ask_to_close(&self, holders: &[String]) -> bool {
        let title = self.title();
        let message = self.text.fill(
            "ui.quiescence.close_confirm",
            &[("applications", &Text::literal(&holders.join(", ")))],
        );
        let close = self.text.get("ui.button.close_applications");
        let cancel = self.text.get("ui.button.cancel");
        dialog::ask(self.hwnd, &title, &message, &close, Some(&cancel))
    }

    unsafe fn confirm_cancel(&self) -> bool {
        let title = self.title();
        let message = self.text.get("ui.progress.cancel_confirm");
        let yes = self.text.get("ui.button.yes");
        let no = self.text.get("ui.button.no");
        dialog::ask(self.hwnd, &title, &message, &yes, Some(&no))
    }

    unsafe fn report(&self, message: &str) {
        let title = self.title();
        let ok = self.text.get("ui.button.ok");
        dialog::ask(self.hwnd, &title, message, &ok, None);
    }

    /// Refuses a destination that is not a full path or does not fit.
    unsafe fn destination_is_usable(&self) -> bool {
        let root = self.chosen_root();
        if root.as_os_str().is_empty() || !root.is_absolute() {
            unsafe { self.report(&self.text.get("ui.destination.not_absolute")) };
            return false;
        }
        let required = self.session.estimated_size;
        if let Some(free) = win::free_space(&root)
            && free < required
        {
            let message = self.text.fill(
                "ui.destination.not_enough_space",
                &[
                    ("volume", &win::volume_of(&root)),
                    ("free", &self.text.size(free)),
                    ("required", &self.text.size(required)),
                ],
            );
            unsafe { self.report(&message) };
            return false;
        }
        true
    }

    /// Whether the run the user has chosen needs an administrator. The
    /// engine's elevation module decided this when the session was built;
    /// the wizard does not decide it again.
    fn needs_elevation(&self) -> bool {
        self.chosen_scope() == Scope::Machine && self.session.machine_needs_elevation
    }

    /// Hands the run to a new copy of this executable for `scope`, on a
    /// worker thread, and freezes the page until it reports back. The copy
    /// is elevated when that scope needs an administrator, and otherwise
    /// started plainly, which is how a choice between two installations
    /// continues: the child is told the scope and is the run from there.
    ///
    /// The relaunch itself waits for the child, so it must not run on the
    /// window thread: doing so would stop the message pump for the whole
    /// life of the UAC prompt and the elevated install, which is exactly the
    /// "Not Responding" the wizard must never show. The worker posts
    /// [`worker::WM_ELEVATION_DONE`] when the child exits, and
    /// `on_elevation_done` finishes the transition on the window thread.
    unsafe fn begin_relaunch(&mut self, scope: Scope) {
        if self.elevating {
            return;
        }
        let elevated = scope == Scope::Machine && self.session.machine_needs_elevation;
        let mut arguments = self.session.relaunch_arguments.clone();
        arguments.push("--scope".into());
        arguments.push(scope.as_str().into());
        arguments.push("--lang".into());
        arguments.push(self.text.lang().to_string());
        self.elevating = true;
        // Freeze the page's controls while the child runs, and repaint so the
        // shield and the disabled state show at once. The child shows the
        // same wizard from its next page on, so this window steps aside for
        // it rather than sitting frozen behind it: at once for a plain child,
        // and for an elevated one the moment the prompt has been answered
        // (`on_elevation_started`) — while the prompt is up this window is
        // what the person came from and what a refusal returns them to.
        unsafe {
            self.update_buttons();
            if !elevated {
                ShowWindow(self.hwnd, SW_HIDE);
            }
        }
        self.elevation_worker = Some(worker::start_relaunch(
            self.hwnd,
            self.session.exe.clone(),
            arguments,
            elevated,
        ));
    }

    /// The prompt was answered and the elevated child is running, so from
    /// here on the child's wizard is the one the person works in; this
    /// window steps aside for it, as it does at once for a plain child.
    unsafe fn on_elevation_started(&mut self) {
        if self.elevating {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
        }
    }

    /// Finishes an elevated relaunch on the window thread once the worker has
    /// posted its result.
    ///
    /// A child that ran is this run's whole outcome — it collected the
    /// choices, did the work and reported it — so the wizard closes and hands
    /// that report up. A prompt that could not be answered leaves the wizard
    /// exactly where it was, usable: the person can pick "for me only"
    /// instead, or close. Nothing was started on this side, so there is no
    /// half-finished transaction to undo either way.
    unsafe fn on_elevation_done(&mut self, done: worker::ElevationDone) {
        self.elevating = false;
        if let Some(worker) = self.elevation_worker.take() {
            let _ = worker.join();
        }
        match done.result {
            Ok(elevated) => {
                self.exit_code = elevated.exit_code;
                // The elevated child did the work and wrote what it did, so
                // its document is this run's outcome rather than a second
                // report of the same installation.
                self.elevated_document = elevated.document;
                unsafe { PostQuitMessage(0) };
            }
            Err(err) => {
                self.exit_code = err.exit_code();
                let message = self
                    .text
                    .fill("ui.error.elevation", &[("reason", &err.message)]);
                unsafe {
                    ShowWindow(self.hwnd, SW_SHOW);
                    self.report(&message);
                    self.update_buttons();
                    SetFocus(self.control(ID_NEXT));
                }
            }
        }
    }

    unsafe fn on_next(&mut self) {
        unsafe {
            match self.page() {
                Page::Scope => {
                    // A choice between two installations always continues in
                    // a new process for the chosen one, so that process
                    // works out its own flow from that installation. Otherwise,
                    // whether this run needs elevation is the engine's
                    // decision, not the wizard's: the answer is not "the
                    // scope says machine" but "can this process write where
                    // that scope keeps its state and its files". A relaunch
                    // runs on a worker thread and the page advances only when
                    // its result arrives, so this returns either way.
                    let chosen = self.chosen_scope();
                    if matches!(self.session.scope_page, Some(ScopePage::Select(_)))
                        || self.needs_elevation()
                    {
                        self.begin_relaunch(chosen);
                        return;
                    }
                }
                Page::Destination => {
                    if !self.destination_is_usable() {
                        return;
                    }
                }
                Page::Finish => {
                    self.launch_product();
                    PostQuitMessage(0);
                    return;
                }
                _ => {}
            }
            if self.page + 1 < self.session.pages.len() {
                self.enter_page(self.page + 1);
            }
        }
    }

    unsafe fn on_cancel(&mut self) {
        unsafe {
            // While an elevated relaunch is in flight the child owns the run,
            // including its own Cancel; closing the parent now would leave the
            // elevated install running unwatched. The parent stays put — still
            // responsive — until the child reports back.
            if self.elevating {
                return;
            }
            match self.page() {
                Page::Progress => {
                    if self.run != RunState::Running {
                        return;
                    }
                    if !self.confirm_cancel() || self.run != RunState::Running {
                        return;
                    }
                    if let Some(cancel) = &self.cancel {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    EnableWindow(self.control(ID_CANCEL), 0);
                    self.status = self.text.get("ui.progress.rolling_back");
                    win::set_text(self.control(ID_PROGRESS_STATUS), &self.status);
                }
                Page::Finish => {
                    self.launch_product();
                    PostQuitMessage(0);
                }
                _ => {
                    self.exit_code = exit::CANCELLED;
                    PostQuitMessage(0);
                }
            }
        }
    }

    fn header_title(&self) -> String {
        let key = match self.page() {
            Page::Scope => match self.session.scope_page {
                Some(ScopePage::Existing(_)) => "ui.scope.existing.title",
                Some(ScopePage::Select(_)) => "ui.scope.select.title",
                _ => "ui.scope.title",
            },
            Page::License => "ui.license.title",
            Page::Destination => "ui.destination.title",
            Page::Options => "ui.options.title",
            Page::Ready => "ui.ready.title",
            Page::Confirm => "ui.uninstall.title",
            Page::Progress => match self.session.operation {
                Operation::Uninstall => "ui.progress.title.uninstall",
                _ => "ui.progress.title.install",
            },
            Page::Finish => match &self.outcome {
                Some(outcome) if matches!(outcome.outcome, "installed" | "uninstalled") => {
                    "ui.finish.title.ok"
                }
                _ => "ui.finish.title.failed",
            },
        };
        self.text.get(key)
    }

    /// Which outcome glyph the completion page shows, if any. A run whose
    /// outcome is not known yet shows none rather than guessing at one.
    fn finish_glyph(&self) -> Option<Glyph> {
        let outcome = self.outcome.as_ref()?;
        Some(match outcome.outcome {
            "installed" | "uninstalled" | "upgraded" | "repaired" | "already_installed" => {
                Glyph::Succeeded
            }
            _ => Glyph::Failed,
        })
    }

    fn header_subtitle(&self) -> String {
        let key = match self.page() {
            Page::Scope => match self.session.scope_page {
                Some(ScopePage::Existing(_)) => "ui.scope.existing.subtitle",
                Some(ScopePage::Select(_)) => "ui.scope.select.subtitle",
                _ => "ui.scope.subtitle",
            },
            Page::License => "ui.license.subtitle",
            Page::Destination => "ui.destination.subtitle",
            Page::Options => "ui.options.subtitle",
            Page::Ready => match self.session.kind {
                FlowKind::Upgrade | FlowKind::Reinstall => "ui.ready.subtitle.upgrade",
                FlowKind::Repair => "ui.ready.subtitle.repair",
                _ => "ui.ready.subtitle.install",
            },
            Page::Confirm => "ui.uninstall.subtitle",
            Page::Progress => match self.session.operation {
                Operation::Uninstall => "ui.progress.subtitle.uninstall",
                _ => "ui.progress.subtitle.install",
            },
            Page::Finish => match &self.outcome {
                Some(outcome) if outcome.outcome == "installed" => "ui.finish.subtitle.installed",
                Some(outcome) if outcome.outcome == "uninstalled" => {
                    "ui.finish.subtitle.uninstalled"
                }
                _ => "ui.finish.subtitle.failed",
            },
        };
        self.text.get(key)
    }

    unsafe fn paint(&self, hdc: HDC) {
        unsafe {
            let dpi = self.dpi;
            let width = scale(layout::CLIENT_W, dpi);
            let header_h = scale(layout::HEADER_H, dpi);
            let footer_top = scale(layout::FOOTER_TOP, dpi);
            let height = scale(layout::CLIENT_H, dpi);

            let header = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: header_h,
            };
            FillRect(hdc, &header, self.theme.header_brush());
            let body = RECT {
                left: 0,
                top: header_h,
                right: width,
                bottom: height,
            };
            FillRect(hdc, &body, self.theme.body_brush());

            let line = self.theme.rule_brush();
            let top_rule = RECT {
                left: 0,
                top: header_h,
                right: width,
                bottom: header_h + 1,
            };
            FillRect(hdc, &top_rule, line);
            let bottom_rule = RECT {
                left: 0,
                top: footer_top,
                right: width,
                bottom: footer_top + 1,
            };
            FillRect(hdc, &bottom_rule, line);

            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, self.theme.colours.text);

            let previous = SelectObject(hdc, self.font_bold as _);
            win::draw_text(
                hdc,
                &self.header_title(),
                layout::HEADER_TITLE.scaled(dpi),
                DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            SelectObject(hdc, self.font as _);
            win::draw_text(
                hdc,
                &self.header_subtitle(),
                layout::HEADER_SUBTITLE.scaled(dpi),
                DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS,
            );

            let image = layout::HEADER_IMAGE.scaled(dpi);
            DrawIconEx(
                hdc,
                image.x,
                image.y,
                self.product_icon,
                image.w,
                image.h,
                0,
                std::ptr::null_mut(),
                DI_NORMAL,
            );

            // The secondary branding: the mark and the name, never a phrase
            // and never translated.
            let mark = layout::BRAND_ICON.scaled(dpi);
            DrawIconEx(
                hdc,
                mark.x,
                mark.y,
                self.brand_icon,
                mark.w,
                mark.h,
                0,
                std::ptr::null_mut(),
                DI_NORMAL,
            );
            // The completion page says what happened in a sentence; the glyph
            // beside it says the same thing at a glance, in the theme's own
            // foreground colour.
            if self.page() == Page::Finish
                && let Some(mark) = self.finish_glyph()
            {
                glyph::draw(
                    hdc,
                    mark,
                    layout::FINISH_GLYPH.scaled(dpi),
                    self.theme.colours.text,
                );
            }

            SetTextColor(hdc, self.theme.colours.dim_text);
            win::draw_text(
                hdc,
                BRAND,
                layout::BRAND_TEXT.scaled(dpi),
                DT_LEFT | DT_TOP | DT_SINGLELINE | DT_NOPREFIX,
            );

            SelectObject(hdc, previous);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Wizard;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let wizard = &mut *pointer;

        match message {
            WM_ERASEBKGND => 1,

            WM_PAINT => {
                let mut ps: PAINTSTRUCT = zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                wizard.paint(hdc);
                EndPaint(hwnd, &ps);
                0
            }

            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                let hdc = wparam as HDC;
                let control = lparam as HWND;
                let window_coloured = wizard
                    .controls
                    .iter()
                    .any(|c| c.hwnd == control && c.window_coloured);
                // The link takes this colour only where the theme told it
                // to (`theme::apply_to_control`); elsewhere it draws the
                // system's own link colour.
                SetTextColor(
                    hdc,
                    if control == wizard.control(ID_FINISH_LOG) {
                        wizard.theme.colours.link
                    } else {
                        wizard.theme.colours.text
                    },
                );
                if window_coloured {
                    SetBkColor(hdc, wizard.theme.colours.header);
                    wizard.theme.header_brush() as LRESULT
                } else {
                    SetBkMode(hdc, TRANSPARENT as i32);
                    wizard.theme.body_brush() as LRESULT
                }
            }

            // Enter activates the page's forward button, exactly as it does
            // in a dialog. A busy page has no forward action, so it has none.
            DM_GETDEFID => {
                if wizard.page() == Page::Progress {
                    0
                } else {
                    ((DC_HASDEFID as isize) << 16) | ID_NEXT as isize
                }
            }

            WM_ENGINE_EVENT => {
                let event = Box::from_raw(lparam as *mut EngineEvent);
                wizard.on_engine_event(&event);
                0
            }

            WM_ENGINE_DONE => {
                let outcome = Box::from_raw(lparam as *mut Outcome);
                wizard.on_engine_done(*outcome);
                0
            }

            WM_ELEVATION_STARTED => {
                wizard.on_elevation_started();
                0
            }

            WM_ELEVATION_DONE => {
                let done = Box::from_raw(lparam as *mut worker::ElevationDone);
                wizard.on_elevation_done(*done);
                0
            }

            // Sent by the engine thread, which is blocked until this
            // returns; the request is borrowed for the call, never owned.
            WM_ENGINE_QUESTION => {
                let request = &*(lparam as *const worker::QuestionRequest);
                wizard.ask_to_close(&request.holders) as isize
            }

            WM_COMMAND => {
                let id = (wparam & 0xffff) as i32;
                let notification = ((wparam >> 16) & 0xffff) as u32;
                match id {
                    ID_NEXT => wizard.on_next(),
                    ID_BACK => {
                        if wizard.page > 0 {
                            wizard.enter_page(wizard.page - 1);
                        }
                    }
                    ID_CANCEL => wizard.on_cancel(),
                    ID_BROWSE => {
                        let title = wizard.text.get("ui.destination.browse_title");
                        if let Some(folder) = win::browse_for_folder(hwnd, &title) {
                            win::set_text(wizard.control(ID_DESTINATION_EDIT), &folder);
                            wizard.update_free_space();
                        }
                    }
                    ID_DESTINATION_EDIT if notification == EN_CHANGE => {
                        wizard.update_free_space();
                    }
                    ID_LICENSE_ACCEPT | ID_LICENSE_DECLINE if notification == BN_CLICKED => {
                        wizard.accepted = wizard.is_checked(ID_LICENSE_ACCEPT);
                        wizard.update_buttons();
                    }
                    ID_SCOPE_USER | ID_SCOPE_MACHINE if notification == BN_CLICKED => {
                        wizard.follow_scope_in_destination();
                        // The chosen scope decides whether Next carries the
                        // elevation shield, so refresh the buttons with it.
                        wizard.update_buttons();
                    }
                    _ => {}
                }
                0
            }

            // The link says it was activated — clicked, or Enter while it
            // has the focus — and this window does what the link is for.
            WM_NOTIFY => {
                let header = &*(lparam as *const NMHDR);
                if header.idFrom == ID_FINISH_LOG as usize
                    && (header.code == NM_CLICK || header.code == NM_RETURN)
                {
                    wizard.copy_log_path();
                }
                0
            }

            // Windows broadcasts every setting change to every window; only
            // the colour one moves the palette, and only a palette that
            // actually moved is worth repainting the whole wizard for.
            WM_SETTINGCHANGE => {
                if theme::is_colour_setting_change(lparam) && wizard.theme.refresh() {
                    wizard.apply_theme();
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }

            WM_DPICHANGED => {
                wizard.dpi = (wparam & 0xffff) as u32;
                let suggested = lparam as *const RECT;
                wizard.create_fonts_and_icons();
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    (*suggested).left,
                    (*suggested).top,
                    (*suggested).right - (*suggested).left,
                    (*suggested).bottom - (*suggested).top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                wizard.apply_layout();
                0
            }

            // The close button is Cancel, so a run in progress is still
            // confirmed and rolled back rather than abandoned.
            WM_CLOSE => {
                wizard.on_cancel();
                0
            }

            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }

            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
