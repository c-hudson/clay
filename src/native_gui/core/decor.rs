//! One stored line as the output shows it: app.js `renderOutput`'s per-line steps, in its
//! order, around the styling in `style`:
//!
//! 1. the MUD tag stripped (unless tags are shown, F2);
//! 2. `✨ ` before a line Clay itself wrote;
//! 3. temperatures converted (while tags are shown and the setting is on);
//! 4. the text styled (`style::style_line`);
//! 5. in front: the timestamp (while tags are shown), `▶ ` on a line this client owns
//!    (or, with New Background, the line flagged for the glow the output paints),
//!    `🛢️ ` on archived text;
//! 6. a `/highlight` colour behind the whole line, or else the F8 action highlight behind
//!    its text.

use std::sync::OnceLock;

use regex::Regex;

use super::style::{Palette, Rgb, Run, StyleOptions, StyledLine, strip_mud_tag, style_line};

/// How a world's lines are decorated right now.
#[derive(Clone, Debug, PartialEq)]
pub struct DecorOptions {
    pub show_tags: bool,
    /// New Indicator is the ▶ character and no TF `%textdiv` (`Settings::nli_prefix`).
    pub new_markers: bool,
    /// New Indicator is the background glow and no `%textdiv` (`Settings::nli_glow`): the
    /// line is flagged (`StyledLine::new_glow`) for the output to paint (src/newbg.rs).
    pub new_background: bool,
    pub temp_convert: bool,
    /// The theme's accent colour (timestamps).
    pub accent: Rgb,
    /// The theme's hover background (the F8 highlight).
    pub hover_bg: Rgb,
}

/// The parts of a stored line its decoration depends on.
pub struct LineFacts<'a> {
    pub text: &'a str,
    pub ts: u64,
    pub from_server: bool,
    pub from_archive: bool,
    pub archive_sourced: bool,
    /// Carries this client's ▶.
    pub is_new: bool,
    pub highlight: Option<&'a str>,
    /// F8 is on and an enabled action matches the line.
    pub action_match: bool,
}

const NEW_MARKER: Rgb = [0x00, 0xff, 0x00];
pub const MAX_LINE_CHARS: usize = 10_000;
const TRUNCATED: &str = "\x1b[0m\x1b[33m... [truncated]\x1b[0m";

pub fn decorate(f: &LineFacts, palette: &Palette, style: &StyleOptions, opts: &DecorOptions) -> StyledLine {
    let mut body = if opts.show_tags { f.text.to_string() } else { strip_mud_tag(f.text).into_owned() };
    if !f.from_server && !f.archive_sourced && !visually_empty(&body) {
        body = format!("✨ {body}");
    }
    if opts.show_tags && opts.temp_convert {
        body = convert_temperatures(&body);
    }
    let mut line = style_line(&body, palette, style);
    let plain = Run { len: 0, fg: palette.fg, bg: None, bold: false, italic: false, underline: false, blink: false, link: false, cell: None, emoji: None };
    let mut prefix: Vec<(String, Rgb)> = Vec::new();
    if opts.show_tags && f.ts > 0 {
        prefix.push((timestamp(f.ts), opts.accent));
    }
    if opts.new_markers && f.is_new {
        prefix.push(("▶".into(), NEW_MARKER));
        prefix.push((" ".into(), palette.fg));
    }
    line.new_glow = opts.new_background && f.is_new;
    if f.from_archive || f.archive_sourced {
        prefix.push(("🛢️ ".into(), palette.fg));
    }
    if !prefix.is_empty() {
        let text: String = prefix.iter().map(|(t, _)| t.as_str()).collect();
        let shift = text.len();
        let mut runs: Vec<Run> = prefix.into_iter().map(|(t, fg)| Run { len: t.len(), fg, ..plain.clone() }).collect();
        runs.append(&mut line.runs);
        line.runs = runs;
        line.text = text + &line.text;
        line.breaks.iter_mut().for_each(|b| *b += shift);
    }
    match f.highlight {
        Some(h) => line.line_bg = Some(highlight_colour(h)),
        None if f.action_match => {
            for r in &mut line.runs {
                if r.bg.is_none() {
                    r.bg = Some(opts.hover_bg);
                }
            }
        }
        None => {}
    }
    line
}

/// F8's test: does an enabled action, for this world, match the line (app.js
/// `lineMatchesAction`: escapes stripped, case-insensitive, wildcards anchored)?
#[derive(Default)]
pub struct ActionMatcher {
    actions: Vec<(String, Vec<Regex>)>,
}

impl ActionMatcher {
    pub fn new(actions: &[crate::actions::Action]) -> Self {
        let actions = actions
            .iter()
            .filter(|a| a.enabled)
            .map(|a| {
                let patterns: Vec<String> = if a.patterns.is_empty() {
                    vec![a.pattern.clone()]
                } else {
                    a.patterns.iter().map(|p| p.pattern.clone()).collect()
                };
                let regexes = patterns
                    .iter()
                    .filter(|p| !p.trim().is_empty())
                    .filter_map(|p| {
                        let pat = if a.match_type == crate::actions::MatchType::Wildcard { crate::actions::wildcard_to_regex(p) } else { p.clone() };
                        regex::RegexBuilder::new(&pat).case_insensitive(true).build().ok()
                    })
                    .collect();
                (a.world.clone(), regexes)
            })
            .collect();
        ActionMatcher { actions }
    }

    pub fn matches(&self, text: &str, world: &str) -> bool {
        if self.actions.is_empty() {
            return false;
        }
        let plain = strip_escapes(text).to_lowercase();
        self.actions
            .iter()
            .filter(|(w, _)| crate::actions::action_matches_world(w, world))
            .any(|(_, res)| res.iter().any(|r| r.is_match(&plain)))
    }
}

/// Cut an over-long line as app.js does when it arrives (`truncateIfNeeded`).
pub fn truncate(text: &str) -> std::borrow::Cow<'_, str> {
    if text.chars().count() > MAX_LINE_CHARS {
        let cut: String = text.chars().take(MAX_LINE_CHARS).collect();
        std::borrow::Cow::Owned(cut + TRUNCATED)
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

fn visually_empty(text: &str) -> bool {
    strip_escapes(text).trim().is_empty()
}

fn strip_escapes(text: &str) -> String {
    static ESC: OnceLock<Regex> = OnceLock::new();
    ESC.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap()).replace_all(text, "").into_owned()
}

/// `MM/DD HH:MM> ` in local time (app.js `formatTimestamp`).
pub fn timestamp(ts: u64) -> String {
    let t = crate::util::local_time_from_epoch(ts as i64);
    format!("{:02}/{:02} {:02}:{:02}> ", t.month, t.day, t.hour, t.minute)
}

/// app.js `convertTemperatures`, but on the text between escape sequences only: a cursor
/// movement like `ESC[5C` must not become "5C (41F)".
pub fn convert_temperatures(text: &str) -> String {
    static TEMP: OnceLock<Regex> = OnceLock::new();
    static ESC: OnceLock<Regex> = OnceLock::new();
    let temp = TEMP.get_or_init(|| Regex::new(r#"(-?\d+(?:\.\d+)?)(\s?)([CcFf])([\s.,;:!?\])"']|$)"#).unwrap());
    let esc = ESC.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").unwrap());
    let convert = |piece: &str| {
        temp.replace_all(piece, |c: &regex::Captures| {
            let Ok(n) = c[1].parse::<f64>() else { return c[0].to_string() };
            let unit = &c[3];
            let (converted, new_unit) =
                if unit.eq_ignore_ascii_case("c") { ((n * 9.0 / 5.0) + 32.0, 'F') } else { ((n - 32.0) * 5.0 / 9.0, 'C') };
            format!("{}{}{} ({}{}){}", &c[1], &c[2], unit, js_round(converted), new_unit, &c[4])
        })
        .into_owned()
    };
    let mut out = String::with_capacity(text.len() + 16);
    let mut at = 0;
    for m in esc.find_iter(text) {
        out.push_str(&convert(&text[at..m.start()]));
        out.push_str(m.as_str());
        at = m.end();
    }
    out.push_str(&convert(&text[at..]));
    out
}

/// JavaScript's `Math.round` (halves round up, towards +infinity).
fn js_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// app.js `colorNameToCss`: a muted named colour, an xterm 0-255 number, `r,g,b` /
/// `r;g;b`, or the default dark cyan.
pub fn highlight_colour(name: &str) -> Rgb {
    const DEFAULT: Rgb = [0x1a, 0x3a, 0x3a];
    let c = name.trim().to_lowercase();
    let named = match c.as_str() {
        "" => Some(DEFAULT),
        "red" => Some([0x4a, 0x15, 0x15]),
        "green" => Some([0x15, 0x3a, 0x15]),
        "blue" => Some([0x15, 0x15, 0x3a]),
        "yellow" => Some([0x3a, 0x3a, 0x15]),
        "cyan" => Some([0x1a, 0x3a, 0x3a]),
        "magenta" | "purple" => Some([0x3a, 0x15, 0x3a]),
        "orange" => Some([0x4a, 0x2a, 0x10]),
        "pink" => Some([0x4a, 0x15, 0x30]),
        "white" => Some([0xc0, 0xc0, 0xc0]),
        "black" => Some([0x1a, 0x1a, 0x1a]),
        "gray" | "grey" => Some([0x3a, 0x3a, 0x3a]),
        _ => None,
    };
    if let Some(rgb) = named {
        return rgb;
    }
    // An r,g,b (or r;g;b) triple first. app.js tries parseInt first, which reads
    // "10,20,30" as xterm colour 10, so there the triple only worked when its first value
    // was over 255 (deferred bug B14).
    let parts: Vec<&str> = if c.contains(',') { c.split(',').collect() } else { c.split(';').collect() };
    if parts.len() == 3 {
        let v: Vec<Option<u8>> = parts.iter().map(|p| p.trim().parse::<u8>().ok()).collect();
        if let [Some(r), Some(g), Some(b)] = v[..] {
            return [r, g, b];
        }
    }
    // parseInt: leading digits.
    let digits: String = c.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if let Ok(n) = digits.parse::<u32>() {
        if n <= 255 {
            return xterm256(n);
        }
    }
    DEFAULT
}

/// app.js `xterm256ToRgb` (its own table: standard 16, a 51-step cube, greys from 8).
fn xterm256(n: u32) -> Rgb {
    const STANDARD: [Rgb; 16] = [
        [0, 0, 0], [0x80, 0, 0], [0, 0x80, 0], [0x80, 0x80, 0], [0, 0, 0x80], [0x80, 0, 0x80], [0, 0x80, 0x80], [0xc0, 0xc0, 0xc0],
        [0x80, 0x80, 0x80], [0xff, 0, 0], [0, 0xff, 0], [0xff, 0xff, 0], [0, 0, 0xff], [0xff, 0, 0xff], [0, 0xff, 0xff], [0xff, 0xff, 0xff],
    ];
    match n {
        0..=15 => STANDARD[n as usize],
        16..=231 => {
            let c = n - 16;
            [((c / 36) * 51) as u8, (((c % 36) / 6) * 51) as u8, ((c % 6) * 51) as u8]
        }
        _ => {
            let g = ((n - 232) * 10 + 8) as u8;
            [g, g, g]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::from_theme(&crate::theme::ThemeColors::dark_default())
    }

    fn opts() -> DecorOptions {
        DecorOptions { show_tags: false, new_markers: true, new_background: false, temp_convert: true, accent: [1, 2, 3], hover_bg: [9, 9, 9] }
    }

    fn facts(text: &str) -> LineFacts<'_> {
        LineFacts { text, ts: 0, from_server: true, from_archive: false, archive_sourced: false, is_new: false, highlight: None, action_match: false }
    }

    #[test]
    fn prefixes_come_in_the_web_clients_order() {
        let p = palette();
        let mut f = facts("hello");
        f.is_new = true;
        f.archive_sourced = true;
        f.from_server = false;
        f.ts = 1_700_000_000;
        let o = DecorOptions { show_tags: true, ..opts() };
        let l = decorate(&f, &p, &StyleOptions::default(), &o);
        // timestamp, ▶, 🛢️, then the body (archive_sourced: no ✨).
        assert!(l.text.ends_with("> ▶ 🛢️ hello"), "{:?}", l.text);
        assert_eq!(l.runs[0].fg, [1, 2, 3], "the timestamp in the accent colour");
        let mark = l.runs.iter().find(|r| r.fg == NEW_MARKER).expect("a green ▶");
        assert_eq!(mark.len, "▶".len());
    }

    #[test]
    fn client_lines_get_sparkles_unless_empty() {
        let p = palette();
        let mut f = facts("notice");
        f.from_server = false;
        assert_eq!(decorate(&f, &p, &StyleOptions::default(), &opts()).text, "✨ notice");
        let mut e = facts("\x1b[0m  ");
        e.from_server = false;
        assert!(!decorate(&e, &p, &StyleOptions::default(), &opts()).text.contains('✨'));
    }

    #[test]
    fn markers_need_the_indicator_and_the_line_to_be_ours() {
        let p = palette();
        let mut f = facts("x");
        f.is_new = true;
        let off = DecorOptions { new_markers: false, ..opts() };
        let none = decorate(&f, &p, &StyleOptions::default(), &off);
        assert_eq!(none.text, "x");
        assert!(!none.new_glow);
        let character = decorate(&f, &p, &StyleOptions::default(), &opts());
        assert_eq!(character.text, "▶ x");
        assert!(!character.new_glow, "the character style paints no glow");
        // New Background: no prefix (nothing to wrap), the line flagged for the glow.
        let glow = DecorOptions { new_markers: false, new_background: true, ..opts() };
        let background = decorate(&f, &p, &StyleOptions::default(), &glow);
        assert_eq!(background.text, "x");
        assert!(background.new_glow);
        let mut old = facts("y");
        old.is_new = false;
        assert!(!decorate(&old, &p, &StyleOptions::default(), &glow).new_glow, "only a line this client owns");
    }

    #[test]
    fn highlights() {
        let p = palette();
        let mut f = facts("x");
        f.highlight = Some("red");
        assert_eq!(decorate(&f, &p, &StyleOptions::default(), &opts()).line_bg, Some([0x4a, 0x15, 0x15]));
        let mut g = facts("\x1b[44mblue\x1b[0m plain");
        g.action_match = true;
        let l = decorate(&g, &p, &StyleOptions::default(), &opts());
        assert_eq!(l.line_bg, None);
        assert!(l.runs.iter().all(|r| r.bg.is_some()), "F8 fills every run without a background of its own");
        assert_ne!(l.runs[0].bg, Some([9, 9, 9]), "a run's own background stays");
        assert_eq!(highlight_colour("196"), [255, 0, 0]);
        assert_eq!(highlight_colour("10, 20,30"), [10, 20, 30]);
        assert_eq!(highlight_colour("nonsense"), [0x1a, 0x3a, 0x3a]);
    }

    #[test]
    fn temperatures_convert_but_never_inside_an_escape() {
        assert_eq!(convert_temperatures("It is 20C outside."), "It is 20C (68F) outside.");
        assert_eq!(convert_temperatures("98.6 F"), "98.6 F (37C)");
        assert_eq!(convert_temperatures("-40F"), "-40F (-40C)");
        assert_eq!(convert_temperatures("go \x1b[5Chere"), "go \x1b[5Chere");
        assert_eq!(convert_temperatures("cold: 5C"), "cold: 5C (41F)");
    }

    #[test]
    fn long_lines_are_cut_with_a_marker() {
        let long = "x".repeat(MAX_LINE_CHARS + 5);
        let t = truncate(&long);
        assert!(t.ends_with(TRUNCATED));
        assert_eq!(t.chars().filter(|&c| c == 'x').count(), MAX_LINE_CHARS);
        assert!(matches!(truncate("short"), std::borrow::Cow::Borrowed(_)));
    }
}
