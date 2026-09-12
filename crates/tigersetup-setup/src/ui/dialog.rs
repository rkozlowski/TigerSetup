//! The wizard's own modal question box.
//!
//! `MessageBoxW` would be shorter, but it draws its buttons in the *system*
//! UI language rather than the installer's, and an installer that shows a
//! Polish question above an English `Yes` is not localized. This box owns
//! every string it shows, carries the same window title as the wizard it
//! belongs to — which is how the lab's driver finds it — and exposes its
//! buttons to UI Automation the same way the wizard's do.

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_LEFT, DT_NOPREFIX, DT_TOP, DT_WORDBREAK, DeleteObject, EndPaint, FillRect, HDC,
    HFONT, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::layout::{Rect, scale};
use super::theme::{self, Theme};
use super::win::{activate_mnemonic, draw_text, message_fonts, wide, wrapped_height};

const CLASS_NAME: &str = "TigerSetupQuestion";

const ID_PRIMARY: i32 = IDOK;
const ID_SECONDARY: i32 = IDCANCEL;

/// Device-independent geometry of the box.
const WIDTH: i32 = 420;
const MARGIN: i32 = 20;
const BUTTON_W: i32 = 104;
const BUTTON_H: i32 = 26;

struct State {
    answer: Option<bool>,
    message: String,
    controls: Vec<HWND>,
    font: HFONT,
    single: bool,
    /// A question raised by the wizard is part of the same window, so it
    /// follows the same theme. A light message box over a dark wizard reads
    /// as another application's dialog.
    theme: Theme,
}

/// Shows `message` under `title` with one or two buttons and waits for an
/// answer. Returns whether the first button was chosen; closing the box or
/// pressing Escape is the second answer, or the only one when there is no
/// second button.
pub fn ask(
    owner: HWND,
    title: &str,
    message: &str,
    primary: &str,
    secondary: Option<&str>,
) -> bool {
    unsafe {
        register_class();
        let instance = GetModuleHandleW(std::ptr::null());
        let dpi = GetDpiForWindow(owner);
        let (font, _bold) = message_fonts(dpi);
        DeleteObject(_bold as _);

        let text_w = scale(WIDTH - 2 * MARGIN, dpi);
        let text_h = wrapped_height(owner, font, message, text_w).max(scale(36, dpi));
        let client_w = scale(WIDTH, dpi);
        let client_h = scale(MARGIN, dpi) + text_h + scale(MARGIN + BUTTON_H + MARGIN, dpi);

        let mut frame = RECT {
            left: 0,
            top: 0,
            right: client_w,
            bottom: client_h,
        };
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        AdjustWindowRectExForDpi(&mut frame, style, 0, WS_EX_DLGMODALFRAME, dpi);
        let width = frame.right - frame.left;
        let height = frame.bottom - frame.top;

        let mut owner_rect: RECT = zeroed();
        GetWindowRect(owner, &mut owner_rect);
        let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - width) / 2;
        let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - height) / 2;

        let class = wide(CLASS_NAME);
        let title_w = wide(title);
        let mut state = Box::new(State {
            answer: None,
            message: message.to_string(),
            controls: Vec::new(),
            font,
            single: secondary.is_none(),
            theme: Theme::current(),
        });
        let state_ptr: *mut State = &mut *state;

        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            title_w.as_ptr(),
            style,
            x,
            y,
            width,
            height,
            owner,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            DeleteObject(font as _);
            return secondary.is_none();
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        theme::apply_to_frame(hwnd, state.theme.mode);

        let button_y = client_h - scale(MARGIN + BUTTON_H, dpi);
        let button_w = scale(BUTTON_W, dpi);
        let gap = scale(10, dpi);
        let right = client_w - scale(MARGIN, dpi);
        let mut buttons = vec![(ID_PRIMARY, primary, BS_DEFPUSHBUTTON as u32)];
        if let Some(second) = secondary {
            buttons.push((ID_SECONDARY, second, BS_PUSHBUTTON as u32));
        }
        let total = buttons.len() as i32;
        for (index, (id, label, kind)) in buttons.into_iter().enumerate() {
            let index = index as i32;
            let left = right - (total - index) * button_w - (total - 1 - index) * gap;
            let label_w = wide(label);
            let button_class = wide("BUTTON");
            let control = CreateWindowExW(
                0,
                button_class.as_ptr(),
                label_w.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_GROUP | kind,
                left,
                button_y,
                button_w,
                scale(BUTTON_H, dpi),
                hwnd,
                id as isize as HMENU,
                instance,
                std::ptr::null(),
            );
            SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
            theme::apply_to_control(control, "BUTTON", state.theme.mode);
            state.controls.push(control);
        }

        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        if let Some(first) = state.controls.first() {
            SetFocus(*first);
        }

        let mut msg: MSG = zeroed();
        while state.answer.is_none() && GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.message == WM_SYSCHAR && activate_mnemonic(&state.controls, msg.wParam as u32) {
                continue;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let answer = state.answer.unwrap_or(state.single);

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        EnableWindow(owner, 1);
        SetActiveWindow(owner);
        DestroyWindow(hwnd);
        DeleteObject(font as _);
        answer
    }
}

unsafe fn register_class() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(proc_of),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: std::ptr::null_mut(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            // The background is painted from the theme instead, because a
            // class brush is fixed when the class is registered and the theme
            // is not.
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
    });
}

unsafe extern "system" fn proc_of(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if state.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let state = &mut *state;
        match message {
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let dpi = GetDpiForWindow(hwnd);
                let mut client: RECT = zeroed();
                GetClientRect(hwnd, &mut client);
                let margin = scale(MARGIN, dpi);
                let rect = Rect::new(
                    margin,
                    margin,
                    client.right - 2 * margin,
                    client.bottom - margin - scale(MARGIN + BUTTON_H, dpi),
                );
                FillRect(hdc, &client, state.theme.body_brush());
                let previous = SelectObject(hdc, state.font as _);
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, state.theme.colours.text);
                draw_text(
                    hdc,
                    &state.message,
                    rect,
                    DT_LEFT | DT_TOP | DT_WORDBREAK | DT_NOPREFIX,
                );
                SelectObject(hdc, previous);
                EndPaint(hwnd, &ps);
                0
            }
            WM_CTLCOLORBTN | WM_CTLCOLORSTATIC => {
                SetTextColor(wparam as HDC, state.theme.colours.text);
                SetBkMode(wparam as HDC, TRANSPARENT as i32);
                state.theme.body_brush() as LRESULT
            }
            DM_GETDEFID => ((DC_HASDEFID as isize) << 16) | ID_PRIMARY as isize,
            WM_COMMAND => {
                match (wparam & 0xffff) as i32 {
                    ID_PRIMARY => state.answer = Some(true),
                    ID_SECONDARY => state.answer = Some(false),
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                state.answer = Some(state.single);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
