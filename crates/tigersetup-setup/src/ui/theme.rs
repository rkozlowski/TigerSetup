//! Light and dark presentation.
//!
//! Windows does not repaint a plain Win32 application when the person chooses
//! the dark app theme: `GetSysColor` keeps answering with the light palette
//! whatever the setting says, because the classic system colours are what
//! high contrast owns and Windows will not move them underneath every old
//! program at once. Following the theme is therefore the application's own
//! work, and it has three separate parts:
//!
//! - **the palette**, which this module owns and the window paints with;
//! - **the title bar**, which the Desktop Window Manager draws, and which is
//!   asked for the dark frame through `DWMWA_USE_IMMERSIVE_DARK_MODE`;
//! - **the common controls**, which draw themselves and are moved onto their
//!   dark visual style with `SetWindowTheme`.
//!
//! Two settings decide the answer, in this order. **High contrast wins**: a
//! person using it has told Windows exactly which colours they can see, and
//! an installer that paints its own instead is unreadable to precisely the
//! person who most needs it — so under high contrast the palette is the
//! system's own colours and nothing here overrides them. Otherwise
//! `AppsUseLightTheme` under `Themes\Personalize` is the preference, and its
//! absence means light, as it does everywhere else in Windows.

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{COLORREF, HWND};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_3DFACE, COLOR_3DSHADOW, COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, CreateSolidBrush,
    DeleteObject, GetSysColor, HBRUSH,
};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SystemParametersInfoW};

/// Which presentation the window is drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
    /// The person's own high-contrast scheme, whatever it is. The system
    /// colours are the answer and the palette does not invent one.
    HighContrast,
}

impl Mode {
    /// What Windows is currently set to.
    pub fn current() -> Mode {
        if high_contrast_is_on() {
            Mode::HighContrast
        } else if apps_use_light_theme() {
            Mode::Light
        } else {
            Mode::Dark
        }
    }

    /// Whether the controls and the frame should be asked for their dark
    /// drawing. High contrast is neither: it is the system's own scheme.
    pub fn is_dark(self) -> bool {
        self == Mode::Dark
    }
}

/// Every colour the wizard paints with. One place, so that a page cannot
/// quietly use a colour the rest of the window does not know about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colours {
    /// The "window" surface: the header band behind the page title and the
    /// product icon, and the read-only text boxes that sit on the same
    /// colour rather than on the content band's face. One colour, because
    /// they are one surface as far as a person looking at the wizard is
    /// concerned.
    pub header: COLORREF,
    /// The content band the pages sit on.
    pub body: COLORREF,
    /// The hairline between the bands.
    pub rule: COLORREF,
    pub text: COLORREF,
    /// Secondary text: the brand line, hints, disabled explanations.
    pub dim_text: COLORREF,
}

/// Windows' own dark surfaces, taken from the dark common-control palette so
/// that a control drawing itself and the window painting behind it agree.
/// These are constants rather than system colours precisely because the
/// system colours do not move with the setting.
const DARK_HEADER: COLORREF = rgb(0x20, 0x20, 0x20);
const DARK_BODY: COLORREF = rgb(0x2B, 0x2B, 0x2B);
const DARK_RULE: COLORREF = rgb(0x3D, 0x3D, 0x3D);
const DARK_TEXT: COLORREF = rgb(0xF0, 0xF0, 0xF0);
const DARK_DIM_TEXT: COLORREF = rgb(0xA0, 0xA0, 0xA0);

/// `COLORREF` is `0x00bbggrr`.
const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as COLORREF) | ((g as COLORREF) << 8) | ((b as COLORREF) << 16)
}

impl Colours {
    pub fn of(mode: Mode) -> Colours {
        match mode {
            Mode::Dark => Colours {
                header: DARK_HEADER,
                body: DARK_BODY,
                rule: DARK_RULE,
                text: DARK_TEXT,
                dim_text: DARK_DIM_TEXT,
            },
            // Light and high contrast are both the system's colours. Light
            // because they are already right; high contrast because they are
            // the person's own choice and must not be second-guessed.
            Mode::Light | Mode::HighContrast => Colours {
                header: system(COLOR_WINDOW),
                body: system(COLOR_3DFACE),
                rule: system(COLOR_3DSHADOW),
                text: system(COLOR_WINDOWTEXT),
                dim_text: system(COLOR_GRAYTEXT),
            },
        }
    }
}

fn system(index: i32) -> COLORREF {
    unsafe { GetSysColor(index) }
}

/// The palette in use, with the brushes the window fills with. Brushes are
/// owned here and replaced as a set when the theme changes, so no repaint can
/// ever mix a new colour with an old brush.
pub struct Theme {
    pub mode: Mode,
    pub colours: Colours,
    header_brush: HBRUSH,
    body_brush: HBRUSH,
    rule_brush: HBRUSH,
}

impl Theme {
    pub fn current() -> Theme {
        Theme::of(Mode::current())
    }

    pub fn of(mode: Mode) -> Theme {
        let colours = Colours::of(mode);
        Theme {
            mode,
            colours,
            header_brush: unsafe { CreateSolidBrush(colours.header) },
            body_brush: unsafe { CreateSolidBrush(colours.body) },
            rule_brush: unsafe { CreateSolidBrush(colours.rule) },
        }
    }

    pub fn header_brush(&self) -> HBRUSH {
        self.header_brush
    }

    pub fn body_brush(&self) -> HBRUSH {
        self.body_brush
    }

    pub fn rule_brush(&self) -> HBRUSH {
        self.rule_brush
    }

    /// Adopts the current Windows setting, reporting whether anything moved.
    /// A window that repaints only on a real change does not flicker every
    /// time Windows broadcasts a setting it does not care about.
    pub fn refresh(&mut self) -> bool {
        let mode = Mode::current();
        if mode == self.mode {
            return false;
        }
        let replacement = Theme::of(mode);
        self.release();
        *self = replacement;
        true
    }

    fn release(&mut self) {
        unsafe {
            DeleteObject(self.header_brush as _);
            DeleteObject(self.body_brush as _);
            DeleteObject(self.rule_brush as _);
        }
    }
}

impl Drop for Theme {
    fn drop(&mut self) {
        self.release();
    }
}

/// Whether a `WM_SETTINGCHANGE` is the one that announces a theme change.
/// Windows broadcasts that message constantly; only this string means the
/// colours moved.
pub fn is_colour_setting_change(lparam: isize) -> bool {
    if lparam == 0 {
        return false;
    }
    let pointer = lparam as *const u16;
    let mut length = 0usize;
    // The string is a NUL-terminated area name; a broadcast carries a short
    // one, so the bound is generous rather than tight.
    while length < 64 {
        if unsafe { *pointer.add(length) } == 0 {
            break;
        }
        length += 1;
    }
    let name = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer, length) });
    name == "ImmersiveColorSet"
}

/// Asks the Desktop Window Manager for the dark title bar and frame.
///
/// The frame is not the application's to paint, so a window that turns its
/// own client area dark and leaves this out gets a white caption bar over a
/// dark page, which looks like a defect rather than a theme.
pub fn apply_to_frame(hwnd: HWND, mode: Mode) {
    /// `DWMWA_USE_IMMERSIVE_DARK_MODE`. Windows 10 1809 — the platform
    /// baseline — carries it at 19, and 20 from 20H1 onwards. Both are asked
    /// for and a refusal is simply the older or newer name not being the one
    /// this build knows; neither is a failure worth reporting to anyone.
    const DARK_MODE_ATTRIBUTES: [u32; 2] = [19, 20];
    let dark: i32 = mode.is_dark() as i32;
    for attribute in DARK_MODE_ATTRIBUTES {
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                attribute,
                &dark as *const i32 as *const std::ffi::c_void,
                size_of::<i32>() as u32,
            );
        }
    }
}

/// Moves one control onto the matching visual style, so that the control
/// draws its own parts — a check box's tick, a button's border, an edit
/// box's frame — in the theme the window is painting.
///
/// The window can colour the text and the background behind a control, but
/// not the pieces the control draws itself; only its visual style can.
pub fn apply_to_control(hwnd: HWND, class: &str, mode: Mode) {
    // `DarkMode_CFD` is the dark style for the framed controls Windows draws
    // a border on; `DarkMode_Explorer` is the one for the rest.
    let style = if !mode.is_dark() {
        None
    } else if class.eq_ignore_ascii_case("EDIT") || class.eq_ignore_ascii_case("COMBOBOX") {
        Some("DarkMode_CFD")
    } else {
        Some("DarkMode_Explorer")
    };
    let Some(style) = style else {
        return;
    };
    let name: Vec<u16> = style.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SetWindowTheme(hwnd, name.as_ptr(), std::ptr::null());
    }
}

fn high_contrast_is_on() -> bool {
    unsafe {
        let mut info: HIGHCONTRASTW = zeroed();
        info.cbSize = size_of::<HIGHCONTRASTW>() as u32;
        let ok = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            size_of::<HIGHCONTRASTW>() as u32,
            &mut info as *mut _ as *mut std::ffi::c_void,
            0,
        );
        ok != 0 && (info.dwFlags & HCF_HIGHCONTRASTON) != 0
    }
}

/// `AppsUseLightTheme`, where Windows records the app theme preference. An
/// unreadable or absent value means light, which is what Windows itself
/// assumes.
fn apps_use_light_theme() -> bool {
    registry_dword(
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "AppsUseLightTheme",
    )
    .unwrap_or(1)
        != 0
}

fn registry_dword(subkey: &str, name: &str) -> Option<u32> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RRF_RT_REG_DWORD, RegCloseKey, RegGetValueW,
        RegOpenKeyExW,
    };

    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(subkey).as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let mut value: u32 = 0;
        let mut size = size_of::<u32>() as u32;
        let status = RegGetValueW(
            key,
            std::ptr::null(),
            wide(name).as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut std::ffi::c_void,
            &mut size,
        );
        RegCloseKey(key);
        (status == ERROR_SUCCESS).then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_do_not_share_a_surface() {
        let light = Colours::of(Mode::Light);
        let dark = Colours::of(Mode::Dark);
        assert_ne!(light.body, dark.body);
        assert_ne!(light.text, dark.text);
    }

    #[test]
    fn dark_text_and_dark_surfaces_are_far_apart() {
        // A palette is only usable if its text and its background differ
        // enough to read. The measure is the perceived-luminance difference,
        // which is what a person's eye actually sees.
        let dark = Colours::of(Mode::Dark);
        for surface in [dark.header, dark.body] {
            let contrast = luminance(dark.text).abs_diff(luminance(surface));
            assert!(
                contrast > 120,
                "dark text on a dark surface has too little contrast: {contrast}"
            );
        }
        let dim = luminance(dark.dim_text).abs_diff(luminance(dark.body));
        assert!(dim > 40, "dimmed dark text is not readable: {dim}");
    }

    #[test]
    fn high_contrast_keeps_the_system_scheme() {
        // The person has told Windows which colours they can see. Whatever
        // those are, they are what high contrast must show.
        assert_eq!(Colours::of(Mode::HighContrast), Colours::of(Mode::Light));
        assert!(!Mode::HighContrast.is_dark());
    }

    /// Rec. 601 luma of a `COLORREF`, 0-255.
    fn luminance(colour: COLORREF) -> u32 {
        let r = colour & 0xff;
        let g = (colour >> 8) & 0xff;
        let b = (colour >> 16) & 0xff;
        (r * 299 + g * 587 + b * 114) / 1000
    }
}
