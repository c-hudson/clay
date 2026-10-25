//! The console's text-reveal effect: tfr's `terminal_reveal` look, applied to the rows
//! the console is about to paint.
//!
//! A newly arrived line is typed onto the screen left to right at a "baud rate": the
//! characters past the frontier are concealed (spaces of the same width), the few just
//! behind it are a glitching run of random glyphs in bold accent colour, the few behind
//! those fade from the accent colour into the character's own foreground, and everything
//! further back is the line as it is. Settled characters occasionally glitch again for a
//! moment while the line is still filling (the inline glitch). One reveal per line,
//! anchored at the moment the console came to show it - its arrival, a switch to its world
//! or more-mode letting it through, whichever came last (`World::console_reveal_anchor`) -
//! so a line restored by a reload or read from the archive is simply drawn whole.
//!
//! A wrapped line's rows all reveal at once, each behind its own frontier, so a long line
//! takes as long as its longest row (user ruling 2026-10-24).
//!
//! Everything here works on the wrapped row strings `wrap_ansi_line` returns, inside
//! `render_output_crossterm`, and nowhere else: the line's data, the wrap, the row
//! counts, paging and more-mode never see it. Every transform keeps a row's visible
//! width (a concealed wide character is two spaces, a zero-width one is dropped, a glitch
//! glyph is one cell for a one-cell character), so the padding and column math of the
//! renderer stay right. The row's own SGR codes are kept: an escape is copied through,
//! and a styled run is closed with a reset plus the row's codes active at that point - the
//! idiom `tf::attrs::apply_to_line` uses.
//!
//! ANSI art (FANSI pictures) is cells, not words: a space with a background colour is a
//! pixel, and a half-block carries two colours. So a concealed cell is painted with the
//! colours reset (a copied-through background would show the picture anyway), and a
//! space is left alone by the glitch run only when it has no background - a coloured one
//! glitches like any glyph (user report 2026-10-24: the text revealed, the pictures did not).
//!
//! The console draws through `Reveal::apply` (ANSI row strings); the native GUI asks
//! `Frame::draw` per character at paint time; app.js carries the same algorithm.
//! (Console only at first, by user ruling 2026-10-24; every interface since 2026-10-25.)

use std::time::{Duration, SystemTime};

use crate::encoding::Theme;
use crate::tf::attrs::{pieces, sgr_resets, Piece};

/// Frames a second while a reveal runs (tfr's ceiling).
pub const FPS: f32 = 30.0;
/// Characters in the glitching run just behind the frontier.
pub const GLITCH_WIDTH: usize = 3;
/// Characters fading from the accent colour to their own, behind the glitch.
pub const SETTLE_WIDTH: usize = 3;
/// Share of settled characters that glitch once more while the line still fills.
pub const INLINE_GLITCH_CHANCE: f32 = 0.35;
/// Per-line speed jitter: each line runs at `1 ± this` times the set speed.
pub const SPEED_VARIATION: f32 = 0.25;
/// A line's reveal never lasts longer than this; a long line is sped up to fit.
pub const MAX_SECS: f32 = 10.0;
/// A line with more visible characters than this is drawn whole.
pub const MAX_CHARS: usize = 4096;
/// The glitch glyphs: visible ASCII (letters, digits, punctuation), every one 1 cell.
pub const GLITCH_GLYPHS: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn array(self) -> [u8; 3] {
        [self.0, self.1, self.2]
    }

    pub fn from_array(c: [u8; 3]) -> Rgb {
        Rgb(c[0], c[1], c[2])
    }

    fn lerp(self, to: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(mix(self.0, to.0), mix(self.1, to.1), mix(self.2, to.2))
    }

    fn sgr_fg(self) -> String {
        format!("38;2;{};{};{}", self.0, self.1, self.2)
    }
}

/// How the effect looks on one console: the speed and the theme's colours. The widths and
/// chances are tfr's defaults; tests set their own.
#[derive(Clone, Copy, Debug)]
pub struct Reveal {
    /// Characters revealed a second (before the per-line jitter).
    pub cps: f32,
    pub fps: f32,
    pub glitch_width: usize,
    pub settle_width: usize,
    pub inline_glitch_chance: f32,
    pub speed_variation: f32,
    /// The glitch colour (bold).
    pub accent: Rgb,
    /// The fade target for a character with no colour of its own.
    pub base: Rgb,
    pub glyphs: &'static str,
}

impl Reveal {
    /// The effect at `cps` characters a second for a console drawn in `theme`: white
    /// glitches fading to light grey on the dark theme, black fading to dark grey on the
    /// light one (bold white on a light terminal is invisible).
    pub fn for_console(cps: f32, theme: Theme) -> Reveal {
        let (accent, base) = match theme {
            Theme::Dark => (Rgb(255, 255, 255), Rgb(215, 215, 215)),
            Theme::Light => (Rgb(0, 0, 0), Rgb(68, 68, 68)),
        };
        Reveal {
            cps,
            fps: FPS,
            glitch_width: GLITCH_WIDTH,
            settle_width: SETTLE_WIDTH,
            inline_glitch_chance: INLINE_GLITCH_CHANCE,
            speed_variation: SPEED_VARIATION,
            accent,
            base,
            glyphs: GLITCH_GLYPHS,
        }
    }

    /// The effect for a GUI window: the same look, chosen by whether the theme is light.
    pub fn for_gui(cps: f32, light: bool) -> Reveal {
        Reveal::for_console(cps, if light { Theme::Light } else { Theme::Dark })
    }

    /// One line's reveal `elapsed` after its anchor, as rows of `row_lens` characters (the
    /// layout's rows; every row reveals at once behind its own frontier, user ruling
    /// 2026-10-24: a three-row line typed on row after row was slow, so a wrapped line takes
    /// as long as its longest row). `None` once it is over, before it starts, when the
    /// setting is off, or for a line too long to run (over `MAX_CHARS` characters).
    pub fn frame(&self, seed: u64, elapsed: Duration, row_lens: &[usize]) -> Option<Frame> {
        if self.cps <= 0.0 || self.fps <= 0.0 {
            return None;
        }
        let n: usize = row_lens.iter().sum();
        if n == 0 || n > MAX_CHARS {
            return None;
        }
        let elapsed = elapsed.as_secs_f64();
        let longest = row_lens.iter().copied().max().unwrap_or(0);
        let fps = self.fps as f64;
        let span = longest + self.glitch_width + self.settle_width;
        let jitter = 1.0 + self.speed_variation as f64 * (2.0 * unit(mix(seed, 0xBEEF, 0)) - 1.0);
        let interval = (1.0 / (self.cps as f64 * jitter)).min(MAX_SECS as f64 / span as f64);
        let total = interval * span as f64;
        // tfr's tolerances: a frame boundary or the end reached to within float noise counts.
        if elapsed >= total - 1e-9 {
            return None;
        }
        let frame = ((elapsed + 1e-9) * fps) as u64;
        let t = frame as f64 / fps;
        let frontier = (t / interval + 1e-9) as usize;
        let next = ((frame + 1) as f64 / fps - elapsed).min(total - elapsed).max(0.001);
        // Rows differ in their glitch glyphs and inline pulses, as lines do.
        let rows = row_lens
            .iter()
            .enumerate()
            .map(|(r, &len)| (seed ^ (r as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15), len))
            .collect();
        Some(Frame { reveal: *self, rows, interval, frame, t, frontier, next: Duration::from_secs_f64(next) })
    }

    /// Draw one line's wrapped `rows` (ANSI row strings, the console's) as they look at
    /// `now` for a line anchored at `timestamp` (`seed` tells lines apart). `Some(delay)` -
    /// the rows were changed and the next frame is due in `delay` - while the reveal runs;
    /// `None`, rows untouched, otherwise (see `frame`; a timestamp in the future - another
    /// machine's clock - is "before it starts").
    pub fn apply(
        &self,
        seed: u64,
        timestamp: SystemTime,
        now: SystemTime,
        rows: &mut [String],
    ) -> Option<Duration> {
        let elapsed = now.duration_since(timestamp).ok()?;
        let counts: Vec<usize> =
            rows.iter().map(|r| pieces(r).iter().filter(|p| matches!(p, Piece::Char(_))).count()).collect();
        let frame = self.frame(seed, elapsed, &counts)?;
        for (row_idx, row) in rows.iter_mut().enumerate() {
            *row = frame.draw_row(row, row_idx);
        }
        Some(frame.next())
    }
}

/// What to do with one character this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Draw {
    /// Not reached yet: blank of the same width (a GUI paints nothing there).
    Hide,
    /// As the line has it.
    Keep,
    /// A random glyph (`Frame::glitch_glyph`) in bold accent colour.
    Glitch,
    /// The character itself, `0.0` = accent colour .. `1.0` = its own colour (`Frame::fade`).
    Fade(f32),
}

/// One line at one frame: each row's seed and length, the frame's frontier. Shared by the
/// console (`Reveal::apply`) and the native GUI, which asks `draw` per character at paint.
pub struct Frame {
    reveal: Reveal,
    rows: Vec<(u64, usize)>,
    interval: f64,
    frame: u64,
    t: f64,
    frontier: usize,
    next: Duration,
}

impl Frame {
    /// The delay to this line's next frame.
    pub fn next(&self) -> Duration {
        self.next
    }

    /// The glitch colour.
    pub fn accent(&self) -> Rgb {
        self.reveal.accent
    }

    /// A fading character's colour: `progress` of the way from the accent to its own
    /// (`own_fg`; the theme's base when it has none).
    pub fn fade(&self, progress: f32, own_fg: Option<Rgb>) -> Rgb {
        self.reveal.accent.lerp(own_fg.unwrap_or(self.reveal.base), progress)
    }

    /// The glitch glyph for character `i` of `row` this frame (for a one-cell character;
    /// a wider one keeps itself in the accent colour).
    pub fn glitch_glyph(&self, row: usize, i: usize) -> char {
        let glyphs = self.reveal.glyphs.as_bytes();
        let seed = self.rows.get(row).map_or(0, |r| r.0);
        glyphs[mix(seed, self.frame, i as u64) as usize % glyphs.len()] as char
    }

    /// What to do with character `i` of `row`. `blank`: whitespace with no background - a
    /// gap between words, never glitched or coloured; a space on a background is a picture
    /// cell and is treated like any glyph.
    pub fn draw(&self, row: usize, i: usize, blank: bool) -> Draw {
        let r = &self.reveal;
        let Some(&(seed, n)) = self.rows.get(row) else { return Draw::Keep };
        let revealed = n.min(self.frontier);
        if i >= revealed {
            return Draw::Hide;
        }
        let trail = self.frontier - 1 - i;
        if trail >= r.glitch_width + r.settle_width || blank {
            let Some(pulse) = self.inline_glitch_phase(seed, n, i) else { return Draw::Keep };
            if blank {
                return Draw::Keep;
            }
            if pulse < self.interval {
                return Draw::Glitch;
            }
            let fade = r.settle_width as f64 * self.interval;
            if fade <= 0.0 {
                return Draw::Keep;
            }
            return Draw::Fade(((pulse - self.interval) / fade) as f32);
        }
        if trail >= r.glitch_width {
            let settle = trail - r.glitch_width;
            if r.settle_width <= 1 || settle + 1 >= r.settle_width {
                return Draw::Keep;
            }
            return Draw::Fade(settle as f32 / (r.settle_width - 1) as f32);
        }
        Draw::Glitch
    }

    /// tfr's `_inline_glitch_phase`: a settled character chosen for an inline glitch
    /// pulses once, at a time of its own, while its row is still filling.
    fn inline_glitch_phase(&self, seed: u64, n: usize, i: usize) -> Option<f64> {
        let r = &self.reveal;
        let interval = self.interval;
        let fill = n as f64 * interval;
        if r.inline_glitch_chance <= 0.0 || self.t >= fill {
            return None;
        }
        if unit(mix(seed ^ 0xA5A5_A5A5, 0, i as u64)) >= r.inline_glitch_chance as f64 {
            return None;
        }
        let stable_at = (i + 1 + r.glitch_width + r.settle_width) as f64 * interval;
        let pulse = (1 + r.settle_width) as f64 * interval;
        let latest_start = fill - pulse;
        if latest_start <= stable_at {
            return None;
        }
        let position = unit(mix(seed ^ 0x5A5A_5A5A, 0, i as u64));
        let start = stable_at + position * (latest_start - stable_at);
        let phase = self.t - start;
        (0.0..pulse).contains(&phase).then_some(phase)
    }

    /// One ANSI row redrawn for this frame (the console's way: hidden characters become
    /// blanks of the same width, a glitch or fade is a 24-bit SGR, the row's own codes are
    /// copied through and restored after a styled run).
    fn draw_row(&self, row: &str, row_idx: usize) -> String {
        let r = &self.reveal;
        let mut out = String::with_capacity(row.len() + 64);
        // The row's own SGR escapes since its last reset, and the colours they set: what a
        // styled run is closed with, what a fade fades to, and whether a space is a pixel.
        let mut active: Vec<&str> = Vec::new();
        let mut own_fg: Option<Rgb> = None;
        let mut own_bg = false;
        // The style of the styled run being written, if one is open. A concealed run on a
        // coloured row is one too, styled as a bare reset: the cells are blank, not painted
        // in the row's background.
        let mut open: Option<String> = None;
        const HIDDEN: &str = "\x1b[0m";
        let restore = |out: &mut String, active: &[&str]| {
            out.push_str("\x1b[0m");
            for code in active {
                out.push_str(code);
            }
        };
        let mut index = 0usize;
        for piece in pieces(row) {
            match piece {
                Piece::Escape(esc) => {
                    if open.take().is_some() && sgr_resets(esc) != Some(true) {
                        restore(&mut out, &active);
                    }
                    out.push_str(esc);
                    if let Some(resets) = sgr_resets(esc) {
                        if resets {
                            active.clear();
                            own_fg = None;
                            own_bg = false;
                        }
                        let params = &esc[2..esc.len() - 1];
                        let colors = sgr_colors(params);
                        if let Some(fg) = colors.fg {
                            own_fg = fg;
                        }
                        if let Some(bg) = colors.bg {
                            own_bg = bg;
                        }
                        if !(params.is_empty() || params == "0") {
                            active.push(esc);
                        }
                    }
                }
                Piece::Char(ch) => {
                    let i = index;
                    index += 1;
                    let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                    let blank = ch.is_whitespace() && !own_bg;
                    let (style, glyph) = match self.draw(row_idx, i, blank) {
                        Draw::Hide => {
                            if active.is_empty() {
                                if open.take().is_some() {
                                    restore(&mut out, &active);
                                }
                            } else if open.as_deref() != Some(HIDDEN) {
                                // A reset is all a hidden run needs, whatever run was open.
                                out.push_str(HIDDEN);
                                open = Some(HIDDEN.to_string());
                            }
                            for _ in 0..width {
                                out.push(' ');
                            }
                            continue;
                        }
                        Draw::Keep => {
                            if open.take().is_some() {
                                restore(&mut out, &active);
                            }
                            out.push(ch);
                            continue;
                        }
                        Draw::Glitch => {
                            let glyph = if width == 1 { self.glitch_glyph(row_idx, i) } else { ch };
                            (format!("\x1b[1;{}m", r.accent.sgr_fg()), glyph)
                        }
                        Draw::Fade(progress) => (format!("\x1b[{}m", self.fade(progress, own_fg).sgr_fg()), ch),
                    };
                    if open.as_deref() != Some(style.as_str()) {
                        if open.take().is_some() {
                            restore(&mut out, &active);
                        }
                        out.push_str(&style);
                        open = Some(style);
                    }
                    out.push(glyph);
                }
            }
        }
        if open.is_some() {
            restore(&mut out, &active);
        }
        out
    }
}

/// What an SGR parameter list says about the colours: `fg` is `Some(Some(rgb))` for a
/// colour, `Some(None)` for the default (39, or a reset), `None` when it says nothing;
/// `bg` is `Some(true)` when a background is set, `Some(false)` when cleared (49, or a
/// reset), `None` when it says nothing.
#[derive(Debug, Default, PartialEq)]
struct SgrColors {
    fg: Option<Option<Rgb>>,
    bg: Option<bool>,
}

fn sgr_colors(params: &str) -> SgrColors {
    let nums: Vec<Option<u16>> = params.split(';').map(|p| p.parse::<u16>().ok()).collect();
    let mut fg: Option<Option<Rgb>> = None;
    let mut bg: Option<bool> = None;
    let mut k = 0;
    while k < nums.len() {
        match nums[k] {
            Some(0) => {
                fg = Some(None);
                bg = Some(false);
            }
            Some(39) => fg = Some(None),
            Some(49) => bg = Some(false),
            Some(40..=47) | Some(100..=107) => bg = Some(true),
            Some(48) => match nums.get(k + 1).copied().flatten() {
                Some(5) => {
                    bg = Some(true);
                    k += 2;
                }
                Some(2) => {
                    bg = Some(true);
                    k += 4;
                }
                _ => {}
            },
            Some(c @ 30..=37) => fg = Some(Some(ansi_palette((c - 30) as u8))),
            Some(c @ 90..=97) => fg = Some(Some(ansi_palette((c - 90 + 8) as u8))),
            Some(38) => match (nums.get(k + 1).copied().flatten(), nums.get(k + 2).copied().flatten()) {
                (Some(5), Some(idx)) if idx < 256 => {
                    fg = Some(Some(ansi_palette(idx as u8)));
                    k += 2;
                }
                (Some(2), Some(r)) => {
                    let g = nums.get(k + 3).copied().flatten();
                    let b = nums.get(k + 4).copied().flatten();
                    if let (Some(g), Some(b)) = (g, b) {
                        if r < 256 && g < 256 && b < 256 {
                            fg = Some(Some(Rgb(r as u8, g as u8, b as u8)));
                        }
                    }
                    k += 4;
                }
                _ => {}
            },
            _ => {}
        }
        k += 1;
    }
    SgrColors { fg, bg }
}

#[cfg(test)]
fn sgr_foreground(params: &str) -> Option<Option<Rgb>> {
    sgr_colors(params).fg
}

/// xterm's 256-colour palette (what the fade converges on; terminal themes vary, and a
/// few steps of colour either way at the end of a fade are not visible).
fn ansi_palette(idx: u8) -> Rgb {
    const BASIC: [Rgb; 16] = [
        Rgb(0, 0, 0), Rgb(205, 0, 0), Rgb(0, 205, 0), Rgb(205, 205, 0),
        Rgb(0, 0, 238), Rgb(205, 0, 205), Rgb(0, 205, 205), Rgb(229, 229, 229),
        Rgb(127, 127, 127), Rgb(255, 0, 0), Rgb(0, 255, 0), Rgb(255, 255, 0),
        Rgb(92, 92, 255), Rgb(255, 0, 255), Rgb(0, 255, 255), Rgb(255, 255, 255),
    ];
    match idx {
        0..=15 => BASIC[idx as usize],
        16..=231 => {
            let c = idx - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            Rgb(level(c / 36), level((c / 6) % 6), level(c % 6))
        }
        232..=255 => {
            let g = 8 + 10 * (idx - 232);
            Rgb(g, g, g)
        }
    }
}

/// tfr's `_seed_value`: a small deterministic mix, so a frame's glitch glyphs are the same
/// wherever the row is drawn and change from frame to frame.
fn mix(seed: u64, frame: u64, index: u64) -> u32 {
    let mut v = (seed as u32) ^ (frame as u32).wrapping_mul(0x9E37_79B1) ^ (index as u32).wrapping_mul(0x85EB_CA77);
    v ^= v >> 16;
    v = v.wrapping_mul(0x7FEB_352D);
    v ^= v >> 15;
    v
}

fn unit(v: u32) -> f64 {
    v as f64 / 4_294_967_296.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(row: &str) -> String {
        pieces(row).iter().filter_map(|p| if let Piece::Char(c) = p { Some(*c) } else { None }).collect()
    }

    fn cols(row: &str) -> usize {
        pieces(row)
            .iter()
            .filter_map(|p| if let Piece::Char(c) = p { unicode_width::UnicodeWidthChar::width(*c) } else { None })
            .sum()
    }

    fn escapes(row: &str) -> Vec<String> {
        pieces(row).iter().filter_map(|p| if let Piece::Escape(e) = p { Some(e.to_string()) } else { None }).collect()
    }

    /// tfr's `alice` fixture: interval 0.1 s, a glitch run of 2 drawn as `#`, no settle.
    fn alice() -> Reveal {
        Reveal {
            cps: 10.0,
            fps: 30.0,
            glitch_width: 2,
            settle_width: 0,
            inline_glitch_chance: 0.0,
            speed_variation: 0.0,
            accent: Rgb(255, 255, 255),
            base: Rgb(215, 215, 215),
            glyphs: "#",
        }
    }

    fn at(reveal: &Reveal, rows: &[&str], secs: f64) -> (Vec<String>, Option<Duration>) {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut rows: Vec<String> = rows.iter().map(|s| s.to_string()).collect();
        let next = reveal.apply(7, t0, t0 + Duration::from_secs_f64(secs), &mut rows);
        (rows, next)
    }

    #[test]
    fn frontier_glitches_then_settles() {
        let r = alice();
        let text = |secs: f64| plain(&at(&r, &["alice"], secs).0[0]);
        assert_eq!(text(0.0), "     ");
        assert_eq!(text(0.25), "##   ");
        assert_eq!(text(0.299), "##   ");
        assert_eq!(text(0.35), "a##  ");
        assert_eq!(text(0.55), "ali##");
        assert_eq!(text(0.7), "alice");
        let (_, next) = at(&r, &["alice"], 0.35);
        let next = next.expect("still revealing").as_secs_f32();
        assert!((next - (11.0 / 30.0 - 0.35)).abs() < 0.002, "next frame in {next}");
        assert_eq!(at(&r, &["alice"], 0.7).1, None, "over: rows untouched, no next frame");
    }

    #[test]
    fn a_future_timestamp_or_a_long_line_is_drawn_whole() {
        let r = alice();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut rows = vec!["alice".to_string()];
        assert_eq!(r.apply(1, t0 + Duration::from_secs(5), t0, &mut rows), None);
        assert_eq!(rows[0], "alice");
        let mut long = vec!["x".repeat(MAX_CHARS + 1)];
        assert_eq!(r.apply(1, t0, t0, &mut long), None);
        assert_eq!(long[0].len(), MAX_CHARS + 1);
        let mut fits = vec!["x".repeat(MAX_CHARS)];
        assert!(r.apply(1, t0, t0, &mut fits).is_some());
    }

    #[test]
    fn a_long_line_is_sped_up_to_fit_the_cap() {
        let r = Reveal { cps: 1.0, ..alice() };
        // 100 chars at 1 cps would take 102 s; the cap makes it MAX_SECS.
        let text = "y".repeat(100);
        let (rows, _) = at(&r, &[&text], MAX_SECS as f64 - 0.05);
        assert!(plain(&rows[0]).contains('#') || plain(&rows[0]).contains("yy"), "still running near the cap");
        assert_eq!(at(&r, &[&text], MAX_SECS as f64).1, None);
    }

    #[test]
    fn glitch_glyphs_are_bold_accent_and_restore_the_rows_own_codes() {
        let r = Reveal { glyphs: "#", ..Reveal::for_console(10.0, Theme::Dark) };
        let r = Reveal { speed_variation: 0.0, settle_width: 0, glitch_width: 2, inline_glitch_chance: 0.0, ..r };
        let row = "\x1b[31malice\x1b[0m";
        let (rows, _) = at(&r, &[row], 0.35);
        assert_eq!(plain(&rows[0]), "a##  ");
        assert_eq!(
            rows[0],
            "\x1b[31ma\x1b[1;38;2;255;255;255m##\x1b[0m  \x1b[0m",
            "one bold-white run for the two glitch glyphs; the hidden cells after it need only a reset"
        );
        // A kept character after a styled run (an inline glitch pulse that just ended) gets
        // the row's own codes back.
        let r = Reveal { inline_glitch_chance: 1.0, ..r };
        let row = "\x1b[31maaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\x1b[0m";
        let restored = (10..120).any(|frame| at(&r, &[row], frame as f64 / 30.0 + 0.001).0[0].contains("\x1b[0m\x1b[31ma"));
        assert!(restored, "some frame has a glitched char followed by a kept one in the row's red");
    }

    #[test]
    fn a_coloured_char_fades_back_to_its_own_colour() {
        let r = Reveal {
            cps: 10.0,
            speed_variation: 0.0,
            inline_glitch_chance: 0.0,
            glitch_width: 1,
            settle_width: 3,
            glyphs: "#",
            ..Reveal::for_console(10.0, Theme::Dark)
        };
        // interval 0.1; at frame 12 (0.4 s) frontier = 4: i=3 glitch, i=2 fade 0, i=1 fade 0.5, i=0 keep
        let (rows, _) = at(&r, &["\x1b[31mabcd\x1b[0m"], 0.405);
        assert_eq!(plain(&rows[0]), "abc#");
        let esc = escapes(&rows[0]);
        assert_eq!(esc[0], "\x1b[31m");
        assert!(esc.contains(&"\x1b[38;2;230;128;128m".to_string()), "half-way between white and red: {esc:?}");
        assert!(esc.contains(&"\x1b[38;2;255;255;255m".to_string()), "the first fade step is the accent: {esc:?}");
        assert!(esc.iter().filter(|e| *e == "\x1b[31m").count() >= 2, "the row's red is restored after a styled run");
        // 256-colour and 24-bit foregrounds are read too.
        let (rows, _) = at(&r, &["\x1b[38;5;196mabcd\x1b[0m"], 0.405);
        assert!(escapes(&rows[0]).contains(&"\x1b[38;2;255;128;128m".to_string()), "{:?}", escapes(&rows[0]));
        let (rows, _) = at(&r, &["\x1b[38;2;0;0;255mabcd\x1b[0m"], 0.405);
        assert!(escapes(&rows[0]).contains(&"\x1b[38;2;128;128;255m".to_string()), "{:?}", escapes(&rows[0]));
        // No colour of its own: the fade heads for the theme's base.
        let (rows, _) = at(&r, &["abcd"], 0.405);
        assert!(escapes(&rows[0]).contains(&"\x1b[38;2;235;235;235m".to_string()), "{:?}", escapes(&rows[0]));
    }

    #[test]
    fn every_frame_keeps_the_row_width_and_its_escapes() {
        let r = Reveal::for_console(60.0, Theme::Dark);
        let row = "\x1b[1;32mHP:\x1b[0m 12 \x1b]8;;https://x.test/\x07link\x1b]8;;\x07 \x1b[38;5;208m界e\u{301}\x1b[0m        end\x1b[0m";
        let before_cols = cols(row);
        let before_esc = escapes(row);
        for step in 0..60 {
            let secs = step as f64 * 0.02;
            let (rows, _) = at(&r, &[row], secs);
            assert_eq!(cols(&rows[0]), before_cols, "width at {secs}s: {:?}", rows[0]);
            let after = escapes(&rows[0]);
            let mut it = after.iter();
            for e in &before_esc {
                assert!(it.any(|a| a == e), "escape {e:?} missing or out of order at {secs}s: {after:?}");
            }
        }
        let (rows, next) = at(&r, &[row], 20.0);
        assert_eq!(rows[0], row);
        assert_eq!(next, None);
    }

    #[test]
    fn wrapped_rows_reveal_together() {
        let r = alice();
        let (rows, _) = at(&r, &["abc", "def"], 0.25); // frontier 2 in every row
        assert_eq!(plain(&rows[0]), "## ");
        assert_eq!(plain(&rows[1]), "## ");
        let (rows, next) = at(&r, &["abc", "def"], 0.45); // frontier 4: each row's last char glitches
        assert_eq!(plain(&rows[0]), "ab#");
        assert_eq!(plain(&rows[1]), "de#");
        assert!(next.is_some());
        // Three rows take as long as one: total = interval * (longest + glitch width).
        assert_eq!(at(&r, &["abc", "def", "g"], 0.5).1, None);
        let (rows, _) = at(&r, &["abc", "def", "g"], 0.45);
        assert_eq!(plain(&rows[2]), "g", "a short row is done before the long ones");
    }

    #[test]
    fn spaces_are_never_glitched_and_hidden_text_keeps_its_cells() {
        let r = Reveal { glitch_width: 5, ..alice() };
        let (rows, _) = at(&r, &["a b 界x\u{301}"], 0.45); // frontier 4: all of "a b " behind it
        let drawn = plain(&rows[0]);
        assert_eq!(&drawn[..4], "# # ", "spaces stay spaces inside the glitch run");
        assert_eq!(cols(&rows[0]), cols("a b 界x\u{301}"));
        let (rows, _) = at(&r, &["a b 界x\u{301}"], 0.0);
        assert_eq!(rows[0], "       ", "a wide char hides as two cells, a combining mark as none");
    }

    #[test]
    fn inline_glitch_is_deterministic_and_only_while_filling() {
        let r = Reveal {
            cps: 20.0,
            speed_variation: 0.0,
            inline_glitch_chance: 1.0,
            glitch_width: 3,
            settle_width: 3,
            glyphs: "#",
            ..Reveal::for_console(20.0, Theme::Dark)
        };
        let text = "a".repeat(40);
        let mut saw_pulse = false;
        for frame in 11..60 {
            let secs = frame as f64 / 30.0 + 0.001; // one sample per frame, so no pulse is skipped
            let (a, _) = at(&r, &[&text], secs);
            let (b, _) = at(&r, &[&text], secs);
            assert_eq!(a, b, "the same seed and time draw the same row");
            if plain(&a[0]).starts_with('#') {
                saw_pulse = true;
            }
        }
        assert!(saw_pulse, "the first (long settled) char glitched once during the fill");
        let (rows, _) = at(&r, &[&text], 2.25);
        assert_eq!(plain(&rows[0]), text, "after the fill nothing glitches (the tail only fades)");
    }

    #[test]
    fn a_picture_cell_hides_without_its_background_and_glitches_like_a_glyph() {
        let r = alice();
        // Three background-coloured spaces (a FANSI picture row) at the start: blank, not blue.
        let (rows, _) = at(&r, &["\x1b[44m   \x1b[0m"], 0.0);
        assert_eq!(rows[0], "\x1b[44m\x1b[0m   \x1b[0m");
        // Half-blocks carrying both colours hide the same way.
        let (rows, _) = at(&r, &["\x1b[31;44m▀▄▀\x1b[0m"], 0.0);
        assert_eq!(rows[0], "\x1b[31;44m\x1b[0m   \x1b[0m");
        // Reached by the frontier, a coloured space glitches (a glyph on its background)
        // while a plain space between words stays a space.
        let (rows, _) = at(&r, &["\x1b[44m   \x1b[0m"], 0.25); // frontier 2
        assert_eq!(plain(&rows[0]), "## ");
        assert!(rows[0].starts_with("\x1b[44m\x1b[1;38;2;255;255;255m##"), "{:?}", rows[0]);
        let (rows, _) = at(&r, &["a b"], 0.25);
        assert_eq!(plain(&rows[0]), "#  ", "a word gap is never glitched");
        // 256-colour and 24-bit backgrounds count, 49 clears one.
        let (rows, _) = at(&r, &["\x1b[48;5;21m  \x1b[49m  \x1b[0m"], 0.0);
        assert_eq!(rows[0], "\x1b[48;5;21m\x1b[0m  \x1b[0m\x1b[48;5;21m\x1b[49m\x1b[0m  \x1b[0m");
        let (rows, _) = at(&r, &["\x1b[48;2;1;2;3m  \x1b[0m"], 0.25);
        assert_eq!(plain(&rows[0]), "##");
        // Settled, the picture is exactly itself again.
        let (rows, _) = at(&r, &["\x1b[31;44m▀▄▀\x1b[0m"], 5.0);
        assert_eq!(rows[0], "\x1b[31;44m▀▄▀\x1b[0m");
    }

    /// The paint-time API the GUI uses agrees with the console's row transform.
    #[test]
    fn frame_draw_matches_the_row_transform() {
        let r = alice();
        let frame = r.frame(7, Duration::from_secs_f64(0.35), &[5]).expect("revealing");
        let draws: Vec<Draw> = (0..5).map(|i| frame.draw(0, i, false)).collect();
        assert_eq!(draws, [Draw::Keep, Draw::Glitch, Draw::Glitch, Draw::Hide, Draw::Hide]);
        assert_eq!(frame.glitch_glyph(0, 1), '#');
        assert!((frame.next().as_secs_f64() - (11.0 / 30.0 - 0.35)).abs() < 0.002);
        assert_eq!(frame.fade(0.5, Some(Rgb(205, 0, 0))), Rgb(230, 128, 128));
        assert_eq!(frame.fade(0.0, None), Rgb(255, 255, 255));
        assert!(r.frame(7, Duration::from_secs_f64(0.7), &[5]).is_none(), "over");
        assert!(r.frame(7, Duration::ZERO, &[]).is_none(), "nothing to reveal");
        assert!(Reveal { cps: 0.0, ..alice() }.frame(7, Duration::ZERO, &[5]).is_none(), "off");
        // A blank (a word gap) is kept wherever the frontier has passed it.
        assert_eq!(frame.draw(0, 1, true), Draw::Keep);
        assert_eq!(Reveal::for_gui(60.0, true).accent, Rgb(0, 0, 0));
        assert_eq!(Reveal::for_gui(60.0, false).accent, Rgb(255, 255, 255));
    }

    #[test]
    fn off_or_zero_speed_does_nothing() {
        let r = Reveal { cps: 0.0, ..alice() };
        let (rows, next) = at(&r, &["alice"], 0.0);
        assert_eq!(rows[0], "alice");
        assert_eq!(next, None);
    }

    #[test]
    fn the_palette_and_foreground_parser_agree_with_xterm() {
        assert_eq!(ansi_palette(196), Rgb(255, 0, 0));
        assert_eq!(ansi_palette(21), Rgb(0, 0, 255));
        assert_eq!(ansi_palette(244), Rgb(128, 128, 128));
        assert_eq!(sgr_foreground("1;31"), Some(Some(Rgb(205, 0, 0))));
        assert_eq!(sgr_foreground("38;5;21"), Some(Some(Rgb(0, 0, 255))));
        assert_eq!(sgr_foreground("38;2;1;2;3;1"), Some(Some(Rgb(1, 2, 3))));
        assert_eq!(sgr_foreground("1"), None);
        assert_eq!(sgr_foreground("0;1"), Some(None));
        assert_eq!(sgr_foreground("39"), Some(None));
    }
}
