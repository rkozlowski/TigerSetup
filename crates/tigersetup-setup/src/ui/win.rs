//! The thin layer between the wizard and the Windows API: string conversion,
//! the system message font at a given dpi, text measurement, mnemonic
//! activation, the folder picker, free-space enquiry, and launching an
//! installed application. Nothing here knows about pages or the engine.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DrawTextW, GetDC, HDC, HFONT, LOGFONTW, ReleaseDC, SelectObject,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{IsWindowEnabled, SetFocus};
use windows_sys::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
    SHBrowseForFolderW, SHELLEXECUTEINFOW, SHGSI_ICON, SHGetPathFromIDListW, SHGetStockIconInfo,
    SHSTOCKICONINFO, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_CLICK, GetClassNameW, GetWindowTextW, IsWindowVisible, NONCLIENTMETRICSW,
    SPI_GETNONCLIENTMETRICS, STM_SETICON, SW_SHOWNORMAL, SendMessageW, SetWindowTextW,
};

/// A NUL-terminated UTF-16 copy of `text`, as every `W` entry point wants.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The system message font for `dpi`, and a bold variant of it. The wizard
/// never chooses a typeface: it uses the one the user's Windows is set to.
pub fn message_fonts(dpi: u32) -> (HFONT, HFONT) {
    unsafe {
        let mut metrics: NONCLIENTMETRICSW = zeroed();
        metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS,
            size_of::<NONCLIENTMETRICSW>() as u32,
            &mut metrics as *mut _ as *mut c_void,
            0,
            dpi,
        );
        let regular: LOGFONTW = metrics.lfMessageFont;
        let mut bold = regular;
        bold.lfWeight = 700;
        (CreateFontIndirectW(&regular), CreateFontIndirectW(&bold))
    }
}

/// The height `text` needs when word-wrapped into `width` pixels.
pub fn wrapped_height(hwnd: HWND, font: HFONT, text: &str, width: i32) -> i32 {
    const DT_CALCRECT: u32 = 0x0400;
    const DT_WORDBREAK: u32 = 0x0010;
    const DT_NOPREFIX: u32 = 0x0800;
    unsafe {
        let hdc = GetDC(hwnd);
        let previous = SelectObject(hdc, font as _);
        let mut buffer: Vec<u16> = text.encode_utf16().collect();
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: 0,
        };
        DrawTextW(
            hdc,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            &mut rect,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(hdc, previous);
        ReleaseDC(hwnd, hdc);
        rect.bottom - rect.top
    }
}

/// Draws `text` inside `rect` with `flags`.
pub fn draw_text(hdc: HDC, text: &str, rect: crate::ui::layout::Rect, flags: u32) {
    let mut buffer: Vec<u16> = text.encode_utf16().collect();
    let mut r = RECT {
        left: rect.x,
        top: rect.y,
        right: rect.x + rect.w,
        bottom: rect.y + rect.h,
    };
    unsafe { DrawTextW(hdc, buffer.as_mut_ptr(), buffer.len() as i32, &mut r, flags) };
}

pub fn set_text(hwnd: HWND, text: &str) {
    let buffer = wide(text);
    unsafe { SetWindowTextW(hwnd, buffer.as_ptr()) };
}

pub fn text_of(hwnd: HWND) -> String {
    let mut buffer = [0u16; 1024];
    let length = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

fn class_of(hwnd: HWND) -> String {
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

/// Activates whichever of `controls` carries the Alt mnemonic `character`: a
/// button is clicked, anything else takes the focus. Returns whether one
/// matched.
///
/// Windows resolves mnemonics for a dialog box; this window is not one, and
/// `IsDialogMessageW` would only move the focus. Activation is the contract
/// the keyboard walk and the lab's automation rely on, so it is done here.
pub fn activate_mnemonic(controls: &[HWND], character: u32) -> bool {
    let Some(wanted) = char::from_u32(character).map(|c| c.to_lowercase().to_string()) else {
        return false;
    };
    for &control in controls {
        unsafe {
            if IsWindowVisible(control) == 0 || IsWindowEnabled(control) == 0 {
                continue;
            }
        }
        let label = text_of(control);
        if tigersetup_engine::i18n::mnemonic_of(&label).map(|c| c.to_string())
            != Some(wanted.clone())
        {
            continue;
        }
        unsafe { SetFocus(control) };
        if class_of(control).eq_ignore_ascii_case("Button") {
            unsafe { SendMessageW(control, BM_CLICK, 0, 0) };
        }
        return true;
    }
    false
}

/// Puts one of the shell's stock icons — the elevation shield, the folder —
/// into a static control.
pub fn set_stock_icon(control: HWND, stock: i32) {
    unsafe {
        let mut info: SHSTOCKICONINFO = zeroed();
        info.cbSize = size_of::<SHSTOCKICONINFO>() as u32;
        if SHGetStockIconInfo(stock, SHGSI_ICON, &mut info) == 0 {
            SendMessageW(control, STM_SETICON, info.hIcon as usize, 0);
        }
    }
}

/// The shell's folder picker, rooted at the desktop.
pub fn browse_for_folder(owner: HWND, title: &str) -> Option<String> {
    unsafe {
        let title = wide(title);
        let mut display = [0u16; 260];
        let info = BROWSEINFOW {
            hwndOwner: owner,
            pidlRoot: std::ptr::null_mut(),
            pszDisplayName: display.as_mut_ptr(),
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            lpfn: None,
            lParam: 0,
            iImage: 0,
        };
        let idlist = SHBrowseForFolderW(&info);
        if idlist.is_null() {
            return None;
        }
        let mut path = [0u16; 260];
        let chosen = (SHGetPathFromIDListW(idlist, path.as_mut_ptr()) != 0).then(|| {
            let length = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            String::from_utf16_lossy(&path[..length])
        });
        CoTaskMemFree(idlist as *const c_void);
        chosen
    }
}

/// Free bytes available on the volume that would hold `path`, walking up to
/// the nearest ancestor that exists. `None` when the volume cannot be read.
pub fn free_space(path: &Path) -> Option<u64> {
    let mut candidate = path;
    loop {
        if candidate.exists() {
            break;
        }
        candidate = candidate.parent()?;
    }
    let wide_path = wide(&candidate.display().to_string());
    let mut free: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide_path.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// The volume a path lives on, as a person names it: `C:\`.
pub fn volume_of(path: &Path) -> String {
    let text = path.display().to_string();
    match text.find(':') {
        Some(colon) => format!("{}:\\", &text[..colon]),
        None => text,
    }
}

/// Starts an installed application the way the shell would, without waiting
/// for it. Best effort and silent: a completion page must not fail, and must
/// never turn into a shell error dialog, because a target has moved or
/// cannot be run.
pub fn launch(target: &Path) -> bool {
    unsafe {
        let file = wide(&target.display().to_string());
        let directory = target
            .parent()
            .map(|parent| wide(&parent.display().to_string()));
        let mut info: SHELLEXECUTEINFOW = zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        info.lpFile = file.as_ptr();
        info.lpDirectory = directory
            .as_ref()
            .map(|d| d.as_ptr())
            .unwrap_or(std::ptr::null());
        info.nShow = SW_SHOWNORMAL;
        ShellExecuteExW(&mut info) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_is_named_the_way_a_person_writes_it() {
        assert_eq!(volume_of(Path::new("C:\\Programs\\Sample")), "C:\\");
        assert_eq!(volume_of(Path::new("relative")), "relative");
    }

    #[test]
    fn free_space_walks_up_to_an_existing_ancestor() {
        let root = std::env::temp_dir();
        let missing = root.join("tigersetup-no-such-folder").join("nor-this-one");
        assert!(free_space(&missing).is_some());
        assert!(free_space(Path::new("\\\\?\\no-such-volume\\x")).is_none());
    }
}
