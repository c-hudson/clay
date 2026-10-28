//! The "New Background" style of the new-text indicator (`NewIndicator::Background`): a
//! green glow behind the first cells of a line this viewer owns, fading to the theme's
//! background over [`CELLS`] cells (user request 2026-10-28: "the fading should happen in 4
//! characters"). Which lines are marked, and when the marks go, is exactly as for the `▶ `
//! character (`OutputLine::display_id`, Ctrl+L, leaving the world); only the drawing
//! differs, and since it adds no text it changes no wrapping.
//!
//! The colours come from here for every interface: [`colour`] gives cell `i`'s, the console
//! prints it with [`glow_row`] (24-bit SGR on the first cells of every wrapped row, fading
//! to black or white by the console theme, [`console_target`]), the native GUI paints a
//! quad per cell (`ui/output.rs`) and app.js builds the same four hard stops into
//! `--new-glow` (`newGlowGradient`), both fading to the theme's own background. A line's own
//! background (ANSI art) stays: the glow goes only under cells that have none, and a
//! `/highlight` colour covers it.

use crate::encoding::Theme;
use crate::reveal::{sgr_colors, Rgb};
use crate::tf::attrs::{pieces, sgr_resets, Piece};

/// Cells the glow spans: green at the first, the theme's background from the one after.
pub const CELLS: usize = 4;
/// The glow's colour at cell 0: a dark green (#006400, the user's pick 2026-10-28 from
/// #00aa00, "a little too bright"), which text stays readable on (the ▶'s #00ff00 does not
/// keep white text readable).
pub const GREEN: Rgb = Rgb(0, 100, 0);

/// A colour scheme for "New Background - Custom" (`NewIndicator::Custom`): the colour of the
/// glow's first cell, which fades to the theme's background as the green does. User request
/// 2026-10-28: "10 or so options to choose from", shown with a four-letter word. Each is about
/// as dark as [`GREEN`], so white text stays readable on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scheme {
    pub key: &'static str,
    pub label: &'static str,
    pub start: Rgb,
}

/// The schemes every Settings window offers, in order (app.js `NEW_GLOW_SCHEMES`, pinned to
/// this by a test). Keys are what settings.dat keeps (`custom:<key>`).
pub const SCHEMES: [Scheme; 10] = [
    Scheme { key: "teal", label: "Teal", start: Rgb(0, 95, 95) },
    Scheme { key: "cyan", label: "Cyan", start: Rgb(0, 105, 140) },
    Scheme { key: "blue", label: "Blue", start: Rgb(20, 60, 165) },
    Scheme { key: "indigo", label: "Indigo", start: Rgb(60, 40, 150) },
    Scheme { key: "purple", label: "Purple", start: Rgb(100, 30, 140) },
    Scheme { key: "magenta", label: "Magenta", start: Rgb(135, 0, 105) },
    Scheme { key: "red", label: "Red", start: Rgb(145, 20, 20) },
    Scheme { key: "orange", label: "Orange", start: Rgb(170, 75, 0) },
    Scheme { key: "gold", label: "Gold", start: Rgb(145, 115, 0) },
    Scheme { key: "slate", label: "Slate", start: Rgb(70, 85, 105) },
];

/// The word each picker draws a scheme's glow behind (its four letters are the four cells).
pub const SAMPLE_WORD: &str = "Clay";

/// The scheme called `key`, as its index in [`SCHEMES`].
pub fn scheme_index(key: &str) -> Option<usize> {
    SCHEMES.iter().position(|s| s.key.eq_ignore_ascii_case(key))
}

/// Cell `i`'s colour: `i / CELLS` of the way from `start` (the scheme's first cell) to `to`.
pub fn colour(i: usize, start: Rgb, to: Rgb) -> Rgb {
    start.lerp(to, i as f32 / CELLS as f32)
}

/// The four cells of a glow starting at `start` and fading to `to`: what the pickers draw.
pub fn swatch(start: Rgb, to: Rgb) -> [Rgb; CELLS] {
    std::array::from_fn(|i| colour(i, start, to))
}

/// Black or white, whichever reads better on `bg` (the pickers' sample word).
pub fn text_on(bg: Rgb) -> Rgb {
    let lum = 0.2126 * bg.0 as f32 + 0.7152 * bg.1 as f32 + 0.0722 * bg.2 as f32;
    if lum > 140.0 { Rgb(0, 0, 0) } else { Rgb(255, 255, 255) }
}

/// What the console's glow fades to: the terminal's background is unknown, so black on the
/// Dark theme and white on Light (the user's words).
pub fn console_target(theme: Theme) -> Rgb {
    match theme {
        Theme::Dark => Rgb(0, 0, 0),
        Theme::Light => Rgb(255, 255, 255),
    }
}

/// One wrapped console row with the glow behind its first [`CELLS`] cells: before each such
/// cell that has no background of its own, its colour as a 24-bit SGR; after the last, the
/// background reset (`49`); a row shorter than that is padded with glow spaces. The row's own
/// escapes are copied through (a reset clears the glow too, which the next cell re-sets), so
/// the visible text and its width are exactly what they were - the wrap, `display_rows`,
/// paging and more-mode never see this.
pub fn glow_row(row: &str, start: Rgb, to: Rgb) -> String {
    let mut out = String::with_capacity(row.len() + CELLS * 20);
    let mut col = 0usize;
    let mut own_bg = false;
    // Whether the glow's SGR is in effect (set by us, not cleared since).
    let mut glowing = false;
    for piece in pieces(row) {
        match piece {
            Piece::Escape(esc) => {
                out.push_str(esc);
                if let Some(resets) = sgr_resets(esc) {
                    if resets {
                        own_bg = false;
                        glowing = false;
                    }
                    if let Some(bg) = sgr_colors(&esc[2..esc.len() - 1]).bg {
                        own_bg = bg;
                        // The row set or cleared its own background: ours is gone either way.
                        glowing = false;
                    }
                }
            }
            Piece::Char(ch) => {
                let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if col < CELLS && width > 0 {
                    if !own_bg {
                        out.push_str(&colour(col, start, to).sgr_bg());
                        glowing = true;
                    }
                    out.push(ch);
                    col += width;
                    if col >= CELLS && glowing {
                        out.push_str("\x1b[49m");
                        glowing = false;
                    }
                } else {
                    out.push(ch);
                    if width > 0 {
                        col += width;
                    }
                }
            }
        }
    }
    while col < CELLS {
        if !own_bg {
            out.push_str(&colour(col, start, to).sgr_bg());
            glowing = true;
        }
        out.push(' ');
        col += 1;
    }
    if glowing {
        out.push_str("\x1b[49m");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_width(s: &str) -> usize {
        unicode_width::UnicodeWidthStr::width(plain(s).as_str())
    }

    const BLACK: Rgb = Rgb(0, 0, 0);

    fn bg(i: usize) -> String {
        colour(i, GREEN, BLACK).sgr_bg()
    }

    fn glow_row_green(row: &str, to: Rgb) -> String {
        glow_row(row, GREEN, to)
    }

    #[test]
    fn every_scheme_is_dark_enough_for_white_text_and_found_by_key() {
        for (i, s) in SCHEMES.iter().enumerate() {
            assert_eq!(scheme_index(s.key), Some(i));
            assert_eq!(text_on(s.start), Rgb(255, 255, 255), "{} reads with white text", s.key);
            let sw = swatch(s.start, Rgb(0, 0, 0));
            assert_eq!(sw[0], s.start);
            assert!(sw[3].0 <= s.start.0 && sw[3].1 <= s.start.1 && sw[3].2 <= s.start.2, "{} fades towards black", s.key);
        }
        assert_eq!(scheme_index("TEAL"), Some(0));
        assert_eq!(scheme_index("plaid"), None);
        assert_eq!(SAMPLE_WORD.chars().count(), CELLS);
        assert_eq!(text_on(Rgb(240, 240, 240)), Rgb(0, 0, 0));
    }

    #[test]
    fn a_scheme_starts_the_glow_with_its_own_colour() {
        let blue = SCHEMES[scheme_index("blue").unwrap()].start;
        let row = glow_row("abcd", blue, Rgb(0, 0, 0));
        assert!(row.starts_with(&blue.sgr_bg()), "{row:?}");
    }

    fn plain(s: &str) -> String {
        pieces(s).into_iter().filter_map(|p| if let Piece::Char(c) = p { Some(c) } else { None }).collect()
    }

    #[test]
    fn the_four_cells_step_from_green_to_the_target() {
        assert_eq!(colour(0, GREEN, BLACK), GREEN);
        assert_eq!(colour(1, GREEN, BLACK), Rgb(0, 75, 0));
        assert_eq!(colour(2, GREEN, BLACK), Rgb(0, 50, 0));
        assert_eq!(colour(3, GREEN, BLACK), Rgb(0, 25, 0));
        assert_eq!(colour(4, GREEN, BLACK), BLACK);
        assert_eq!(colour(2, GREEN, Rgb(255, 255, 255)), Rgb(128, 178, 128), "on the light theme the fade heads for white");
        assert_eq!(console_target(Theme::Dark), BLACK);
        assert_eq!(console_target(Theme::Light), Rgb(255, 255, 255));
    }

    #[test]
    fn a_plain_row_gets_four_glow_cells_then_the_reset() {
        let row = glow_row_green("hello world", BLACK);
        assert_eq!(row, format!("{}h{}e{}l{}l\x1b[49mo world", bg(0), bg(1), bg(2), bg(3)));
        assert_eq!(plain(&row), "hello world", "the text is untouched");
        assert_eq!(visible_width(&row), 11, "and so is its width");
    }

    #[test]
    fn a_short_row_is_padded_to_the_glow_width() {
        assert_eq!(glow_row_green("hi", BLACK), format!("{}h{}i{} {} \x1b[49m", bg(0), bg(1), bg(2), bg(3)));
        assert_eq!(glow_row_green("", BLACK), format!("{} {} {} {} \x1b[49m", bg(0), bg(1), bg(2), bg(3)));
    }

    #[test]
    fn the_rows_own_background_is_left_alone() {
        // A picture row: its first cells carry their own background.
        let row = glow_row_green("\x1b[44mab\x1b[49mcdef", BLACK);
        assert_eq!(row, format!("\x1b[44mab\x1b[49m{}c{}d\x1b[49mef", bg(2), bg(3)));
        // The glow is a background: a row set entirely in its own colour keeps it.
        assert_eq!(glow_row_green("\x1b[48;5;17mpixels\x1b[0m", BLACK), "\x1b[48;5;17mpixels\x1b[0m");
    }

    #[test]
    fn a_reset_inside_the_glow_is_followed_by_the_next_cells_colour() {
        let row = glow_row_green("\x1b[31ma\x1b[0mbcd", BLACK);
        assert_eq!(row, format!("\x1b[31m{}a\x1b[0m{}b{}c{}d\x1b[49m", bg(0), bg(1), bg(2), bg(3)));
        // The foreground is the row's: copied through and still active after the glow.
        let row = glow_row_green("\x1b[1;32mabcdef\x1b[0m", BLACK);
        assert_eq!(row, format!("\x1b[1;32m{}a{}b{}c{}d\x1b[49mef\x1b[0m", bg(0), bg(1), bg(2), bg(3)));
    }

    #[test]
    fn a_wide_character_takes_two_cells() {
        let row = glow_row_green("日本語", BLACK);
        assert_eq!(row, format!("{}日{}本\x1b[49m語", bg(0), bg(2)));
        assert_eq!(visible_width(&row), 6);
        // A zero-width character is copied, not counted.
        let row = glow_row_green("e\u{301}abc", BLACK);
        assert_eq!(row, format!("{}e\u{301}{}a{}b{}c\x1b[49m", bg(0), bg(1), bg(2), bg(3)));
    }

    #[test]
    fn an_osc_hyperlink_is_copied_through() {
        let row = glow_row_green("\x1b]8;;http://x\x1b\\ab\x1b]8;;\x1b\\cd", BLACK);
        assert_eq!(plain(&row), "abcd");
        assert!(row.starts_with("\x1b]8;;http://x\x1b\\"));
        assert_eq!(row.matches("\x1b[48;2;").count(), 4);
    }
}
