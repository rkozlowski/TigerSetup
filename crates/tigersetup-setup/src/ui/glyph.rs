//! The few Fluent UI System Icons the wizard draws, as vector outlines.
//!
//! These are the *regular* 24-unit glyphs from Microsoft's Fluent UI System
//! Icons, kept as their original path data and rendered here rather than
//! shipped as bitmaps. Three properties follow from that, and all three are
//! why it is worth the code:
//!
//! - **every scale is the right scale.** A wizard that runs at 100, 125, 150
//!   and 200 per cent would otherwise need an image per size, and would still
//!   be wrong at a scale nobody thought of.
//! - **the glyph follows the theme**, because it is filled with the colour
//!   the window is drawing text in rather than with colours baked into an
//!   image.
//! - **nothing is added to the installer's payload.** The data below is a few
//!   hundred bytes of path text.
//!
//! GDI fills a path without antialiasing, which at 16 or 24 pixels looks like
//! a defect rather than an icon. The outline is therefore filled into an
//! oversampled monochrome bitmap and box-filtered down into a coverage mask,
//! which is then blended in the wanted colour. That is the whole reason this
//! module is longer than "fill a path".
//!
//! Provenance and licence are recorded in `THIRD-PARTY-NOTICES.md`; the path
//! data below is redistributed material and its notice travels with it.

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{COLORREF, POINT};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, AlphaBlend, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    BeginPath, CloseFigure, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, EndPath, FillPath, GetDC, HDC, LineTo, MoveToEx, PatBlt, PolyBezierTo,
    ReleaseDC, SelectObject, SetPolyFillMode, WHITENESS, WINDING,
};

use super::layout::Rect;

/// The glyphs the wizard has a use for. Deliberately few: an installer is a
/// familiar Windows dialog, not an icon gallery, and every glyph here has to
/// say something the words beside it do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// The run finished and did what it said it would.
    Succeeded,
    /// The run failed, or was rolled back.
    Failed,
}

impl Glyph {
    fn path(self) -> &'static str {
        match self {
            Glyph::Succeeded => CHECKMARK_CIRCLE_24_REGULAR,
            Glyph::Failed => ERROR_CIRCLE_24_REGULAR,
        }
    }
}

/// Every glyph below is drawn on this square, which is the Fluent 24-pixel
/// grid the outlines were designed on.
const GRID: f64 = 24.0;

/// How many times over the outline is sampled in each direction before the
/// coverage is averaged down. Four is the point where more stops being
/// visible at the sizes a wizard draws.
const OVERSAMPLE: i32 = 4;

/// `assets/Checkmark Circle/SVG/ic_fluent_checkmark_circle_24_regular.svg`
const CHECKMARK_CIRCLE_24_REGULAR: &str = "M12 2C17.5228 2 22 6.47715 22 12C22 17.5228 17.5228 22 12 22C6.47715 22 2 17.5228 2 12C2 6.47715 6.47715 2 12 2ZM12 3.5C7.30558 3.5 3.5 7.30558 3.5 12C3.5 16.6944 7.30558 20.5 12 20.5C16.6944 20.5 20.5 16.6944 20.5 12C20.5 7.30558 16.6944 3.5 12 3.5ZM10.75 13.4393L15.2197 8.96967C15.5126 8.67678 15.9874 8.67678 16.2803 8.96967C16.5466 9.23594 16.5708 9.6526 16.3529 9.94621L16.2803 10.0303L11.2803 15.0303C11.0141 15.2966 10.5974 15.3208 10.3038 15.1029L10.2197 15.0303L7.71967 12.5303C7.42678 12.2374 7.42678 11.7626 7.71967 11.4697C7.98594 11.2034 8.4026 11.1792 8.69621 11.3971L8.78033 11.4697L10.75 13.4393L15.2197 8.96967L10.75 13.4393Z";

/// `assets/Error Circle/SVG/ic_fluent_error_circle_24_regular.svg`
const ERROR_CIRCLE_24_REGULAR: &str = "M12 2C17.5228 2 22 6.47715 22 12C22 17.5228 17.5228 22 12 22C6.47715 22 2 17.5228 2 12C2 6.47715 6.47715 2 12 2ZM12 3.5C7.30558 3.5 3.5 7.30558 3.5 12C3.5 16.6944 7.30558 20.5 12 20.5C16.6944 20.5 20.5 16.6944 20.5 12C20.5 7.30558 16.6944 3.5 12 3.5ZM11.999 14.502C12.5504 14.5021 12.9971 14.9496 12.9971 15.501C12.997 16.0524 12.5504 16.4998 11.999 16.5C11.4475 16.5 11 16.0525 11 15.501C11 14.9494 11.4475 14.502 11.999 14.502ZM11.9941 7C12.3738 6.9997 12.6883 7.2815 12.7383 7.64746L12.7451 7.74902L12.749 12.251C12.7494 12.6652 12.4132 13.0016 11.999 13.002C11.6194 13.0021 11.3058 12.7195 11.2559 12.3535L11.249 12.252L11.2451 7.75098C11.2448 7.33687 11.5801 7.00051 11.9941 7Z";

/// One drawing instruction of an outline, already in absolute coordinates on
/// the 24-unit grid.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Segment {
    Move(f64, f64),
    Line(f64, f64),
    /// Two control points and an end point.
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// Reads the subset of SVG path syntax these glyphs use: absolute and
/// relative move, line, horizontal and vertical line, cubic curve, and close.
///
/// It is deliberately not a general SVG path parser. The data it reads is a
/// handful of constants in this file, so an unsupported command is a mistake
/// made here rather than input to defend against, and the parser simply stops
/// at one — a glyph that would be drawn wrong is not drawn.
fn parse(path: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut numbers = Numbers::new(path);
    let (mut x, mut y) = (0.0, 0.0);
    let (mut start_x, mut start_y) = (0.0, 0.0);
    let mut command = ' ';
    loop {
        // A command letter, or a repeat of the previous one with fresh
        // numbers, which is how SVG shortens a run of curves.
        match numbers.command() {
            Some(letter) => command = letter,
            None if numbers.at_number() && command != ' ' => {
                // An implicit repeat; a repeated move is a line, as SVG says.
                if command == 'M' {
                    command = 'L';
                } else if command == 'm' {
                    command = 'l';
                }
            }
            None => break,
        }
        let relative = command.is_ascii_lowercase();
        let (dx, dy) = if relative { (x, y) } else { (0.0, 0.0) };
        match command.to_ascii_uppercase() {
            'M' => {
                let (nx, ny) = (numbers.next() + dx, numbers.next() + dy);
                x = nx;
                y = ny;
                start_x = nx;
                start_y = ny;
                segments.push(Segment::Move(nx, ny));
            }
            'L' => {
                let (nx, ny) = (numbers.next() + dx, numbers.next() + dy);
                x = nx;
                y = ny;
                segments.push(Segment::Line(nx, ny));
            }
            'H' => {
                x = numbers.next() + dx;
                segments.push(Segment::Line(x, y));
            }
            'V' => {
                y = numbers.next() + dy;
                segments.push(Segment::Line(x, y));
            }
            'C' => {
                let c1 = (numbers.next() + dx, numbers.next() + dy);
                let c2 = (numbers.next() + dx, numbers.next() + dy);
                let end = (numbers.next() + dx, numbers.next() + dy);
                x = end.0;
                y = end.1;
                segments.push(Segment::Cubic(c1.0, c1.1, c2.0, c2.1, end.0, end.1));
            }
            'Z' => {
                x = start_x;
                y = start_y;
                segments.push(Segment::Close);
            }
            _ => break,
        }
        if numbers.exhausted() {
            break;
        }
    }
    segments
}

/// A scanner over the numbers and command letters of a path string.
struct Numbers<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Numbers<'a> {
    fn new(text: &'a str) -> Numbers<'a> {
        Numbers {
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    fn skip_separators(&mut self) {
        while self.at < self.bytes.len() {
            match self.bytes[self.at] {
                b' ' | b',' | b'\t' | b'\r' | b'\n' => self.at += 1,
                _ => break,
            }
        }
    }

    fn exhausted(&mut self) -> bool {
        self.skip_separators();
        self.at >= self.bytes.len()
    }

    fn at_number(&mut self) -> bool {
        self.skip_separators();
        matches!(
            self.bytes.get(self.at),
            Some(b'0'..=b'9' | b'-' | b'.' | b'+')
        )
    }

    /// The next command letter, if one is there.
    fn command(&mut self) -> Option<char> {
        self.skip_separators();
        let byte = *self.bytes.get(self.at)?;
        if byte.is_ascii_alphabetic() {
            self.at += 1;
            return Some(byte as char);
        }
        None
    }

    /// The next number. A malformed path yields zero rather than panicking:
    /// the data is a constant in this file, so the failure mode that matters
    /// is a glyph that looks wrong, not one that ends the installer.
    fn next(&mut self) -> f64 {
        self.skip_separators();
        let start = self.at;
        if matches!(self.bytes.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        while matches!(self.bytes.get(self.at), Some(b'0'..=b'9' | b'.')) {
            self.at += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or(0.0)
    }
}

/// Draws `glyph` filling `rect`, in `colour`, blended onto whatever is there.
///
/// `rect` is device pixels and already scaled for the dpi by the caller, as
/// every other measurement in the wizard is.
pub fn draw(hdc: HDC, glyph: Glyph, rect: Rect, colour: COLORREF) {
    let size = rect.w.min(rect.h);
    if size <= 0 {
        return;
    }
    let Some(mask) = coverage(glyph, size) else {
        return;
    };
    blend(hdc, rect.x, rect.y, size, &mask, colour);
}

/// The glyph's coverage, one byte per pixel, at `size` × `size`.
///
/// GDI fills the outline into a bitmap `OVERSAMPLE` times larger and the
/// result is averaged down, which is what turns a hard-edged path fill into
/// something that looks like an icon.
fn coverage(glyph: Glyph, size: i32) -> Option<Vec<u8>> {
    let big = size * OVERSAMPLE;
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(std::ptr::null_mut(), screen);
        if dc.is_null() {
            return None;
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: big,
            // Negative height is a top-down bitmap, so row 0 is the top row
            // and the rows can simply be read in order.
            biHeight: -big,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous_bitmap = SelectObject(dc, bitmap as _);

        // White ground, black glyph: the ink is then simply the inverse of a
        // channel, and no palette or mask bitmap is involved.
        PatBlt(dc, 0, 0, big, big, WHITENESS);
        let brush = CreateSolidBrush(0);
        let previous_brush = SelectObject(dc, brush as _);
        SetPolyFillMode(dc, WINDING);
        BeginPath(dc);
        let scale = big as f64 / GRID;
        let point = |x: f64, y: f64| POINT {
            x: (x * scale).round() as i32,
            y: (y * scale).round() as i32,
        };
        let mut current = POINT { x: 0, y: 0 };
        for segment in parse(glyph.path()) {
            match segment {
                Segment::Move(x, y) => {
                    current = point(x, y);
                    MoveToEx(dc, current.x, current.y, std::ptr::null_mut());
                }
                Segment::Line(x, y) => {
                    current = point(x, y);
                    LineTo(dc, current.x, current.y);
                }
                Segment::Cubic(c1x, c1y, c2x, c2y, ex, ey) => {
                    let points = [point(c1x, c1y), point(c2x, c2y), point(ex, ey)];
                    PolyBezierTo(dc, points.as_ptr(), points.len() as u32);
                    current = points[2];
                }
                Segment::Close => {
                    CloseFigure(dc);
                }
            }
        }
        let _ = current;
        EndPath(dc);
        FillPath(dc);
        SelectObject(dc, previous_brush);
        DeleteObject(brush as _);

        // Average each OVERSAMPLE × OVERSAMPLE block of the ink channel.
        let pixels = std::slice::from_raw_parts(bits as *const u8, (big * big * 4) as usize);
        let per_block = (OVERSAMPLE * OVERSAMPLE) as u32;
        let mut mask = vec![0u8; (size * size) as usize];
        for y in 0..size {
            for x in 0..size {
                let mut ink = 0u32;
                for sy in 0..OVERSAMPLE {
                    for sx in 0..OVERSAMPLE {
                        let px = x * OVERSAMPLE + sx;
                        let py = y * OVERSAMPLE + sy;
                        let at = ((py * big + px) * 4) as usize;
                        ink += 255 - pixels[at] as u32;
                    }
                }
                mask[(y * size + x) as usize] = (ink / per_block) as u8;
            }
        }

        SelectObject(dc, previous_bitmap);
        DeleteObject(bitmap as _);
        DeleteDC(dc);
        Some(mask)
    }
}

/// Blends `mask` onto `hdc` in `colour`, so the glyph takes the theme's
/// foreground rather than a colour of its own.
fn blend(hdc: HDC, x: i32, y: i32, size: i32, mask: &[u8], colour: COLORREF) {
    let (r, g, b) = (colour & 0xff, (colour >> 8) & 0xff, (colour >> 16) & 0xff);
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        ReleaseDC(std::ptr::null_mut(), screen);
        if dc.is_null() {
            return;
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let bitmap = CreateDIBSection(
            dc,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return;
        }
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        for (index, &alpha) in mask.iter().enumerate() {
            let at = index * 4;
            let a = alpha as u32;
            // `AlphaBlend` wants premultiplied colour.
            pixels[at] = ((b * a) / 255) as u8;
            pixels[at + 1] = ((g * a) / 255) as u8;
            pixels[at + 2] = ((r * a) / 255) as u8;
            pixels[at + 3] = alpha;
        }
        let previous = SelectObject(dc, bitmap as _);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        AlphaBlend(hdc, x, y, size, size, dc, 0, 0, size, size, blend);
        SelectObject(dc, previous);
        DeleteObject(bitmap as _);
        DeleteDC(dc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_reads_as_the_outline_it_describes() {
        let segments = parse("M1 2L3 4C5 6 7 8 9 10Z");
        assert_eq!(
            segments,
            vec![
                Segment::Move(1.0, 2.0),
                Segment::Line(3.0, 4.0),
                Segment::Cubic(5.0, 6.0, 7.0, 8.0, 9.0, 10.0),
                Segment::Close,
            ]
        );
    }

    #[test]
    fn a_repeated_command_may_be_left_out() {
        // SVG lets a run of curves name the command once, which is exactly how
        // the Fluent outlines are written.
        let segments = parse("M0 0C1 1 2 2 3 3 4 4 5 5 6 6");
        assert_eq!(
            segments,
            vec![
                Segment::Move(0.0, 0.0),
                Segment::Cubic(1.0, 1.0, 2.0, 2.0, 3.0, 3.0),
                Segment::Cubic(4.0, 4.0, 5.0, 5.0, 6.0, 6.0),
            ]
        );
    }

    #[test]
    fn relative_commands_follow_the_current_point() {
        let segments = parse("M10 10l5 0h2v3z");
        assert_eq!(
            segments,
            vec![
                Segment::Move(10.0, 10.0),
                Segment::Line(15.0, 10.0),
                Segment::Line(17.0, 10.0),
                Segment::Line(17.0, 13.0),
                Segment::Close,
            ]
        );
    }

    #[test]
    fn every_glyph_parses_and_stays_on_its_grid() {
        for glyph in [Glyph::Succeeded, Glyph::Failed] {
            let segments = parse(glyph.path());
            assert!(
                segments.len() > 8,
                "{glyph:?} produced only {} segment(s)",
                segments.len()
            );
            assert!(
                matches!(segments.first(), Some(Segment::Move(_, _))),
                "{glyph:?} must start by moving to a point"
            );
            assert!(
                matches!(segments.last(), Some(Segment::Close)),
                "{glyph:?} must end closed, or the fill is undefined"
            );
            // An outline that left the grid would be clipped when drawn.
            for segment in segments {
                let points: Vec<(f64, f64)> = match segment {
                    Segment::Move(x, y) | Segment::Line(x, y) => vec![(x, y)],
                    Segment::Cubic(a, b, c, d, e, f) => vec![(a, b), (c, d), (e, f)],
                    Segment::Close => vec![],
                };
                for (x, y) in points {
                    assert!(
                        (0.0..=GRID).contains(&x) && (0.0..=GRID).contains(&y),
                        "{glyph:?} has a point at ({x}, {y}) outside the {GRID}-unit grid"
                    );
                }
            }
        }
    }
}
