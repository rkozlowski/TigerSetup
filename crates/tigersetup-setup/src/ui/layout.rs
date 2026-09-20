//! The wizard layout, written once in device-independent units (96 dpi) and
//! scaled to the window's current dpi. Keeping every position in one table is
//! what makes a dpi change a re-scale rather than a rewrite, and it is what
//! lets the same numbers be reasoned about at 100, 125, 150 and 200 %.
//!
//! The client height is chosen so that the frame still fits the work area of
//! a 1920x1080 display at 200 %: 440 device-independent units are 880 pixels
//! there, and the caption and borders add about 70 more.

/// Client area of the wizard window, in device-independent units.
pub const CLIENT_W: i32 = 640;
pub const CLIENT_H: i32 = 440;

/// The white header band. It is tall enough for a two-line subtitle, because
/// the Polish subtitles do not fit on one line at this width.
pub const HEADER_H: i32 = 78;
pub const FOOTER_H: i32 = 56;
pub const FOOTER_TOP: i32 = CLIENT_H - FOOTER_H;

/// Left margin of the content band, and the width of a full-width control.
pub const MARGIN: i32 = 24;
pub const CONTENT_W: i32 = CLIENT_W - 2 * MARGIN;
/// First line of the content band.
pub const CONTENT_TOP: i32 = HEADER_H + 14;

pub const HEADER_TITLE: Rect = Rect::new(MARGIN, 12, 500, 22);
pub const HEADER_SUBTITLE: Rect = Rect::new(MARGIN + 20, 38, 500, 34);
pub const HEADER_IMAGE: Rect = Rect::new(CLIENT_W - 72, 14, 48, 48);

/// The secondary branding: the small TigerSetup icon and the name beside it.
/// The icon is nominally 24 dip; what is drawn is the embedded image
/// nearest to that at the window's dpi, at its own pixels, centred on this
/// rectangle's centre line, and the text starts [`BRAND_GAP`] after its
/// actual right edge (`icon::brand_native`).
pub const BRAND_ICON: Rect = Rect::new(MARGIN, FOOTER_TOP + 16, 24, 24);
pub const BRAND_GAP: i32 = 8;
pub const BRAND_TEXT: Rect = Rect::new(MARGIN + 24 + BRAND_GAP, FOOTER_TOP + 18, 240, 20);

const BUTTON_W: i32 = 100;
const BUTTON_H: i32 = 26;
const BUTTON_Y: i32 = FOOTER_TOP + 15;

pub const BTN_CANCEL: Rect = Rect::new(CLIENT_W - MARGIN - BUTTON_W, BUTTON_Y, BUTTON_W, BUTTON_H);
pub const BTN_NEXT: Rect = Rect::new(BTN_CANCEL.x - 10 - BUTTON_W, BUTTON_Y, BUTTON_W, BUTTON_H);
pub const BTN_BACK: Rect = Rect::new(BTN_NEXT.x - 4 - BUTTON_W, BUTTON_Y, BUTTON_W, BUTTON_H);

// Scope.
pub const SCOPE_BODY: Rect = Rect::new(MARGIN, CONTENT_TOP, CONTENT_W, 68);
pub const SCOPE_USER: Rect = Rect::new(MARGIN + 16, CONTENT_TOP + 80, CONTENT_W - 16, 22);
pub const SCOPE_MACHINE: Rect = Rect::new(MARGIN + 16, CONTENT_TOP + 110, CONTENT_W - 16, 22);

// Licence.
pub const LICENSE_BODY: Rect = Rect::new(MARGIN, CONTENT_TOP, CONTENT_W, 34);
pub const LICENSE_TEXT: Rect = Rect::new(MARGIN, CONTENT_TOP + 40, CONTENT_W, 168);
pub const LICENSE_ACCEPT: Rect = Rect::new(MARGIN, CONTENT_TOP + 216, CONTENT_W, 20);
pub const LICENSE_DECLINE: Rect = Rect::new(MARGIN, CONTENT_TOP + 240, CONTENT_W, 20);

// Destination.
pub const DESTINATION_ICON: Rect = Rect::new(MARGIN, CONTENT_TOP, 20, 20);
pub const DESTINATION_BODY: Rect = Rect::new(MARGIN + 28, CONTENT_TOP + 2, CONTENT_W - 28, 20);
pub const DESTINATION_HINT: Rect = Rect::new(MARGIN, CONTENT_TOP + 32, CONTENT_W, 34);
pub const DESTINATION_EDIT: Rect = Rect::new(MARGIN, CONTENT_TOP + 74, CONTENT_W - 112, 24);
pub const DESTINATION_BROWSE: Rect = Rect::new(CLIENT_W - MARGIN - 100, CONTENT_TOP + 73, 100, 26);
pub const DESTINATION_SPACE: Rect = Rect::new(MARGIN, CONTENT_TOP + 118, CONTENT_W, 40);

// Options: a body paragraph and one row per check box, or a heading row
// plus one radio row per choice for a choice option. A page holds
// `OPTION_ROWS` rows; the options take as many pages as they need.
pub const OPTIONS_BODY: Rect = Rect::new(MARGIN, CONTENT_TOP, CONTENT_W, 40);
pub const OPTIONS_FIRST_TOP: i32 = CONTENT_TOP + 50;
pub const OPTION_STEP: i32 = 26;
pub const OPTION_H: i32 = 22;
/// The rows that fit between the body paragraph and the footer.
pub const OPTION_ROWS: usize = ((FOOTER_TOP - 4 - OPTIONS_FIRST_TOP) / OPTION_STEP) as usize;
/// How far a choice's radio buttons sit in from their heading.
pub const CHOICE_INDENT: i32 = 20;

// Ready.
pub const READY_BODY: Rect = Rect::new(MARGIN, CONTENT_TOP, CONTENT_W, 34);
pub const READY_SUMMARY: Rect = Rect::new(MARGIN, CONTENT_TOP + 40, CONTENT_W, 244);

// Progress.
pub const PROGRESS_STATUS: Rect = Rect::new(MARGIN, CONTENT_TOP + 8, CONTENT_W, 20);
pub const PROGRESS_BAR: Rect = Rect::new(MARGIN, CONTENT_TOP + 36, CONTENT_W, 18);

// Completion.
/// The outcome glyph, beside the sentence that says the same thing in words.
/// It is drawn rather than placed in a control, because it follows the theme's
/// foreground colour and scales with the dpi like the text does.
pub const FINISH_GLYPH: Rect = Rect::new(MARGIN, CONTENT_TOP, 32, 32);
pub const FINISH_BODY: Rect = Rect::new(MARGIN + 44, CONTENT_TOP, CONTENT_W - 44, 96);
pub const FINISH_LAUNCH: Rect = Rect::new(MARGIN, CONTENT_TOP + 108, CONTENT_W, 22);
/// The "Copy log path" link sits at the foot of the content band, away from
/// the sentence and the launch offer: a diagnostic affordance, not a step.
/// The control is fitted to its text at run time; this is its room.
pub const FINISH_LOG: Rect = Rect::new(MARGIN, FOOTER_TOP - 36, CONTENT_W, 20);

// Uninstall confirmation.
pub const CONFIRM_BODY: Rect = Rect::new(MARGIN, CONTENT_TOP + 8, CONTENT_W, 96);

/// The check box, or the heading, on one row of an options page.
pub fn option_row(index: usize) -> Rect {
    Rect::new(
        MARGIN + 4,
        OPTIONS_FIRST_TOP + OPTION_STEP * index as i32,
        CONTENT_W - 4,
        OPTION_H,
    )
}

/// One choice's radio button on one row of an options page, indented under
/// its heading.
pub fn choice_row(index: usize) -> Rect {
    Rect::new(
        MARGIN + 4 + CHOICE_INDENT,
        OPTIONS_FIRST_TOP + OPTION_STEP * index as i32,
        CONTENT_W - 4 - CHOICE_INDENT,
        OPTION_H,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    /// The first row below this rectangle.
    #[cfg(test)]
    pub const fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// Scales device-independent units to physical pixels for `dpi`.
    pub fn scaled(self, dpi: u32) -> Rect {
        Rect {
            x: scale(self.x, dpi),
            y: scale(self.y, dpi),
            w: scale(self.w, dpi),
            h: scale(self.h, dpi),
        }
    }
}

pub fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing on any page may reach into the footer band, in any language:
    /// the layout is the same at every scale, so one check at 96 dpi covers
    /// 100, 125, 150 and 200 %.
    #[test]
    fn no_page_control_overlaps_the_footer() {
        assert_eq!(OPTION_ROWS, 9, "the page holds nine option rows");
        let rows: Vec<Rect> = (0..OPTION_ROWS)
            .map(option_row)
            .chain((0..OPTION_ROWS).map(choice_row))
            .collect();
        let mut all = vec![
            SCOPE_BODY,
            SCOPE_USER,
            SCOPE_MACHINE,
            LICENSE_BODY,
            LICENSE_TEXT,
            LICENSE_ACCEPT,
            LICENSE_DECLINE,
            DESTINATION_ICON,
            DESTINATION_BODY,
            DESTINATION_HINT,
            DESTINATION_EDIT,
            DESTINATION_BROWSE,
            DESTINATION_SPACE,
            OPTIONS_BODY,
            READY_BODY,
            READY_SUMMARY,
            PROGRESS_STATUS,
            PROGRESS_BAR,
            FINISH_BODY,
            FINISH_LAUNCH,
            FINISH_LOG,
            CONFIRM_BODY,
        ];
        all.extend(rows);
        for rect in all {
            assert!(
                rect.bottom() <= FOOTER_TOP - 4,
                "{rect:?} reaches the footer at {FOOTER_TOP}"
            );
            assert!(rect.y >= HEADER_H, "{rect:?} reaches the header band");
            assert!(
                rect.x + rect.w <= CLIENT_W - MARGIN + 1,
                "{rect:?} reaches past the right margin"
            );
        }
    }

    #[test]
    fn the_navigation_buttons_do_not_overlap() {
        let row = [BTN_BACK, BTN_NEXT, BTN_CANCEL];
        for pair in row.windows(2) {
            assert!(
                pair[0].x + pair[0].w <= pair[1].x,
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(
            row[0].x > BRAND_TEXT.x + BRAND_TEXT.w,
            "the buttons overlap the branding"
        );
        assert_eq!(BTN_CANCEL.x + BTN_CANCEL.w, CLIENT_W - MARGIN);
    }

    #[test]
    fn scaling_rounds_to_the_nearest_pixel() {
        assert_eq!(scale(100, 96), 100);
        assert_eq!(scale(100, 120), 125);
        assert_eq!(scale(100, 144), 150);
        assert_eq!(scale(100, 192), 200);
    }
}
