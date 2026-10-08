//! One line of MUD text -> what the output view paints.
//!
//! A port of the web client's per-line pipeline (`app.js`: `insertWordBreaks` ->
//! `parseAnsi` -> `renderBlockElements` -> `linkifyUrls`), so a line looks the same in
//! the native GUI as in the WebView. app.js is the spec, quirks included: SGR 1 turns an
//! already-set 30-37 colour bright, SGR 22 and 2 (dim) are ignored, and only the 38/48
//! forms app.js understands are honoured.

use std::sync::OnceLock;

use regex::Regex;

use super::boxdraw::is_box_drawing;

pub type Rgb = [u8; 3];

/// The colours a line is styled with: the theme's 16 ANSI colours, its default text and
/// background colours, and the link colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub ansi: [Rgb; 16],
    pub fg: Rgb,
    pub bg: Rgb,
    pub link: Rgb,
}

impl Palette {
    pub fn from_theme(t: &crate::theme::ThemeColors) -> Self {
        let c = |c: &crate::theme::ThemeColor| [c.r, c.g, c.b];
        let mut ansi = [[0u8; 3]; 16];
        for (i, slot) in ansi.iter_mut().enumerate() {
            *slot = c(&t.ansi[i]);
        }
        Palette { ansi, fg: c(&t.fg), bg: c(&t.bg), link: c(&t.link) }
    }
}

/// How a run is drawn when it isn't ordinary text (app.js `renderBlockElements`): as exact
/// cells, one column wide and one row tall each, painted over everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Spaces on a coloured background: plain background cells.
    Space,
    /// A block element (U+2580-U+259F), drawn as its exact shape in the run's colour.
    Block(char),
    /// A box-drawing character (U+2500-U+257F), drawn as exact lines that meet the
    /// neighbouring cells' (see `boxdraw`). The WebView draws these from a fallback font,
    /// whose columns and line height don't match the output's.
    Box(char),
}

/// A Discord custom emoji (`<:name:id>`, or `<a:name:id>` when animated), which app.js
/// `convertDiscordEmojis` turns into an image from Discord's CDN.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiscordEmoji {
    /// Digits only.
    pub id: String,
    /// `[A-Za-z0-9_]` only: what the web shows as the image's alt text, `:name:`.
    pub name: String,
    pub animated: bool,
}

/// Stands in for a Discord emoji in [`StyledLine::text`]: one character, which line
/// breaking treats as an inline object (UAX #14 class CB), as browsers treat an image.
pub const EMOJI_PLACEHOLDER: char = '\u{FFFC}';

/// A styled stretch of [`StyledLine::text`].
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// Length in bytes.
    pub len: usize,
    pub fg: Rgb,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub link: bool,
    /// Drawn as cells rather than glyphs. The characters stay in `text` for copying.
    pub cell: Option<Cell>,
    /// A Discord emoji: the run is its one [`EMOJI_PLACEHOLDER`].
    pub emoji: Option<std::sync::Arc<DiscordEmoji>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyledLine {
    /// The text as displayed and copied: escape sequences removed.
    pub text: String,
    /// Contiguous runs covering all of `text`.
    pub runs: Vec<Run>,
    /// Byte offsets where `insertWordBreaks` would have inserted a zero-width space: extra
    /// wrap opportunities inside long words. Kept out of `text`, so copying needs no
    /// clean-up.
    pub breaks: Vec<usize>,
    /// A background behind the whole line, edge to edge (`/highlight`).
    pub line_bg: Option<Rgb>,
}

/// Settings that change how a line is styled.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleOptions {
    /// `color_offset_percent`: pushes text colours away from a background they nearly
    /// match. 0 is off.
    pub color_offset_percent: u32,
    /// Turn Discord emoji tags into emoji (off while tags are shown, F2).
    pub discord_emoji: bool,
}

impl Default for StyleOptions {
    fn default() -> Self {
        StyleOptions { color_offset_percent: 0, discord_emoji: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Colour {
    Default,
    /// An entry of the 16-colour palette (app.js's `ansi-*` classes).
    Index(u8),
    /// An explicit colour (app.js's inline `rgb()` styles).
    Rgb(Rgb),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Sgr {
    fg: Colour,
    bg: Colour,
    bold: bool,
    italic: bool,
    underline: bool,
    blink: bool,
    reversed: bool,
}

impl Sgr {
    const RESET: Sgr = Sgr {
        fg: Colour::Default,
        bg: Colour::Default,
        bold: false,
        italic: false,
        underline: false,
        blink: false,
        reversed: false,
    };

    fn apply(&mut self, params: &str, palette: &Palette) {
        let codes: Vec<u32> = params.split(';').map(|c| c.parse().unwrap_or(0)).collect();
        let mut i = 0;
        while i < codes.len() {
            match codes[i] {
                0 => *self = Sgr::RESET,
                1 => {
                    self.bold = true;
                    if let Colour::Index(n) = self.fg {
                        if n < 8 {
                            self.fg = Colour::Index(n + 8);
                        }
                    }
                }
                3 => self.italic = true,
                4 => self.underline = true,
                5 | 6 => self.blink = true,
                7 => self.reversed = true,
                27 => self.reversed = false,
                n @ 30..=37 => {
                    let n = (n - 30) as u8;
                    self.fg = Colour::Index(if self.bold { n + 8 } else { n });
                }
                38 => {
                    if let Some(c) = extended_colour(&codes, &mut i, palette) {
                        self.fg = c;
                    }
                }
                39 => self.fg = Colour::Default,
                n @ 40..=47 => self.bg = Colour::Index((n - 40) as u8),
                48 => {
                    if let Some(c) = extended_colour(&codes, &mut i, palette) {
                        self.bg = c;
                    }
                }
                49 => self.bg = Colour::Default,
                n @ 90..=97 => self.fg = Colour::Index((n - 90) as u8 + 8),
                n @ 100..=107 => self.bg = Colour::Index((n - 100) as u8 + 8),
                _ => {}
            }
            i += 1;
        }
    }
}

/// `38;5;N` / `38;2;R;G;B` (and the 48 forms), advancing `i` past what was consumed.
/// Anything else after a 38/48 is left for the main loop, as app.js does.
fn extended_colour(codes: &[u32], i: &mut usize, palette: &Palette) -> Option<Colour> {
    match codes.get(*i + 1) {
        Some(5) if codes.len() > *i + 2 => {
            let rgb = colour_256(codes[*i + 2], palette);
            *i += 2;
            Some(Colour::Rgb(rgb))
        }
        Some(2) if codes.len() > *i + 4 => {
            let c = |v: u32| v.min(255) as u8;
            let rgb = [c(codes[*i + 2]), c(codes[*i + 3]), c(codes[*i + 4])];
            *i += 4;
            Some(Colour::Rgb(rgb))
        }
        _ => None,
    }
}

/// xterm's 256-colour palette: the theme's 16, the 6x6x6 cube on xterm's levels, then
/// 24 greys.
fn colour_256(n: u32, palette: &Palette) -> Rgb {
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match n {
        0..=15 => palette.ansi[n as usize],
        16..=231 => {
            let n = (n - 16) as usize;
            [CUBE[n / 36], CUBE[(n % 36) / 6], CUBE[n % 6]]
        }
        _ => {
            let g = ((n.min(255) - 232) * 10 + 8) as u8;
            [g, g, g]
        }
    }
}

/// The colours a chunk of text is actually drawn with, after reverse video.
struct Effective {
    fg: Rgb,
    bg: Option<Rgb>,
}

fn effective(s: &Sgr, palette: &Palette) -> Effective {
    let resolve = |c: Colour| match c {
        Colour::Default => None,
        Colour::Index(n) => Some(palette.ansi[n as usize]),
        Colour::Rgb(rgb) => Some(rgb),
    };
    let fg = resolve(s.fg).unwrap_or(palette.fg);
    let bg = resolve(s.bg);
    if s.reversed {
        // app.js applyReverseVideo: swap, resolving a default background to the theme's.
        Effective { fg: bg.unwrap_or(palette.bg), bg: Some(fg) }
    } else {
        Effective { fg, bg }
    }
}

/// app.js `adjustFgForContrast`. Note it compares a default background against a fixed
/// dark colour, not the theme's background.
fn adjust_for_contrast(fg: Rgb, bg: Option<Rgb>, percent: u32) -> Rgb {
    if percent == 0 {
        return fg;
    }
    let bg = bg.unwrap_or([13, 17, 23]);
    let distance: u32 = (0..3).map(|i| (fg[i] as i32 - bg[i] as i32).unsigned_abs()).sum();
    let threshold = 150 * percent / 100;
    if distance >= threshold {
        return fg;
    }
    let dark = (bg[0] as u32 + bg[1] as u32 + bg[2] as u32) / 3 < 128;
    let adjustment = (percent * 2).min(200) as i32;
    let shift = |v: u8| {
        if dark {
            (v as i32 + adjustment).min(255) as u8
        } else {
            (v as i32 - adjustment).max(0) as u8
        }
    };
    [shift(fg[0]), shift(fg[1]), shift(fg[2])]
}

fn blend(fg: Rgb, bg: Rgb, fg_weight: f32) -> Rgb {
    let mix = |f: u8, b: u8| (f as f32 * fg_weight + b as f32 * (1.0 - fg_weight)).round() as u8;
    [mix(fg[0], bg[0]), mix(fg[1], bg[1]), mix(fg[2], bg[2])]
}

fn is_block_element(c: char) -> bool {
    ('\u{2580}'..='\u{259F}').contains(&c)
}

/// One element of a line once escape sequences are recognised.
enum Token<'a> {
    Text(&'a str),
    Sgr(&'a str),
}

/// Splits a line into text and SGR sequences, dropping every other CSI sequence, any
/// ESC that doesn't start one, the ␛ control picture, and SGR leftovers whose ESC was
/// lost (`[0m`, `[1;32m`), as app.js's `parseAnsi` does.
fn tokenize(line: &str) -> Vec<Token<'_>> {
    static CSI: OnceLock<Regex> = OnceLock::new();
    let csi = CSI.get_or_init(|| {
        Regex::new(r"\x1b\[([0-9;?]*)([A-Za-z@`~])|\x1b|\x{241b}|\[[0-9;]*m").unwrap()
    });
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut last = 0;
    for m in csi.find_iter(line) {
        let s = m.as_str();
        // A leftover `[..m` only goes when no letter follows, so `[match(` and `[menu]`
        // survive (app.js's negative lookahead).
        if s.starts_with('[') {
            let next = bytes.get(m.end()).copied().unwrap_or(b' ');
            if next.is_ascii_alphabetic() {
                continue;
            }
        }
        if m.start() > last {
            out.push(Token::Text(&line[last..m.start()]));
        }
        last = m.end();
        if let Some(rest) = s.strip_prefix("\x1b[") {
            if let Some(params) = rest.strip_suffix('m') {
                if !params.contains('?') {
                    out.push(Token::Sgr(params));
                }
            }
        }
    }
    if last < line.len() {
        out.push(Token::Text(&line[last..]));
    }
    out
}

/// Where `insertWordBreaks` puts its zero-width spaces: after a break character, once
/// the word it's in has passed 15 characters. Counted over the display text with Discord
/// emoji tags treated as one unit, the same as app.js.
fn word_breaks(text: &str) -> Vec<usize> {
    const BREAK_CHARS: &[char] = &[']', ')', ',', '\\', '/', '-', '_', '&', '=', '?', ';'];
    static DISCORD: OnceLock<Regex> = OnceLock::new();
    let discord = DISCORD.get_or_init(|| Regex::new(r"^<a?:[A-Za-z0-9_]+:\d+>").unwrap());
    let mut breaks = Vec::new();
    let mut word_len = 0usize;
    let mut skip_to = 0usize;
    for (i, c) in text.char_indices() {
        if i < skip_to {
            continue;
        }
        if c == '<' {
            if let Some(m) = discord.find(&text[i..]) {
                skip_to = i + m.end();
                continue;
            }
        }
        if c == EMOJI_PLACEHOLDER {
            // An emoji that replaced its tag: the tag counted nothing either.
            continue;
        }
        if c.is_whitespace() {
            word_len = 0;
        } else {
            word_len += 1;
            if word_len > 15 && BREAK_CHARS.contains(&c) {
                breaks.push(i + c.len_utf8());
            }
        }
    }
    breaks
}

/// URLs app.js `linkifyUrls` would turn into links, as byte ranges of `text`.
fn link_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    static URL: OnceLock<Regex> = OnceLock::new();
    let url = URL.get_or_init(|| {
        Regex::new(
            r#"(?i)(?-u:\b)(?:https?://|www\.)[^\s<>"'\x{201C}\x{201D}\x{2018}\x{2019}]*[^\s<>"'\x{201C}\x{201D}\x{2018}\x{2019}.,;:!?)\]}>]"#,
        )
        .unwrap()
    });
    url.find_iter(text).map(|m| m.range()).collect()
}

/// app.js `stripMudTag`: drop a leading MUD channel tag and its space. Only for an
/// unindented line starting (after any SGR codes) with `[name(content)...] ` or
/// `[name:] `; the SGR codes before the tag are kept. Skipped when tags are shown (F2).
pub fn strip_mud_tag(line: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if line.starts_with(char::is_whitespace) {
        return Cow::Borrowed(line);
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    // Skip leading escape sequences (to the first letter, as app.js does).
    while i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
        let mut j = i + 2;
        while j < bytes.len() && !bytes[j].is_ascii_alphabetic() {
            j += 1;
        }
        i = (j + 1).min(bytes.len());
    }
    let rest = &line[i..];
    let Some(inner) = rest.strip_prefix('[') else { return Cow::Borrowed(line) };
    let Some(end) = inner.find(']') else { return Cow::Borrowed(line) };
    let tag = &inner[..end];
    let is_tag = match tag.find('(') {
        Some(open) if open > 0 => tag[open..].find(')').is_some_and(|close| close > 1),
        _ => tag.chars().count() > 1 && tag.ends_with(':'),
    };
    match inner[end + 1..].strip_prefix(' ') {
        Some(after) if is_tag => Cow::Owned(format!("{}{}", &line[..i], after)),
        _ => Cow::Borrowed(line),
    }
}

/// Style one line of output, as app.js does for the WebView.
pub fn style_line(line: &str, palette: &Palette, opts: &StyleOptions) -> StyledLine {
    // Some servers send the escape as literal text.
    let normalized;
    let line = if line.contains("\\x1b") || line.contains("\\u001b") || line.contains("\\X1B") {
        static LITERAL: OnceLock<Regex> = OnceLock::new();
        let re = LITERAL.get_or_init(|| Regex::new(r"(?i)\\x1b|\\u001b").unwrap());
        normalized = re.replace_all(line, "\x1b").into_owned();
        normalized.as_str()
    } else {
        line
    };

    let mut out = StyledLine::default();
    let mut sgr = Sgr::RESET;
    for token in tokenize(line) {
        match token {
            Token::Sgr(params) => sgr.apply(params, palette),
            Token::Text(chunk) => push_chunk(&mut out, chunk, &sgr, palette, opts),
        }
    }
    out.breaks = word_breaks(&out.text);
    out
}

/// One chunk of text under one SGR state: app.js emits each as its own span, which is
/// also why a URL never continues across a colour change.
fn push_chunk(out: &mut StyledLine, chunk: &str, sgr: &Sgr, palette: &Palette, opts: &StyleOptions) {
    let eff = effective(sgr, palette);
    let text_fg = adjust_for_contrast(eff.fg, eff.bg, opts.color_offset_percent);
    let base = Run {
        len: 0,
        fg: text_fg,
        bg: eff.bg,
        bold: sgr.bold,
        italic: sgr.italic,
        underline: sgr.underline,
        blink: sgr.blink,
        link: false,
        cell: None,
        emoji: None,
    };
    // Links are found first, on the text with its tags (app.js links before it converts
    // emoji; a URL can't contain `<` or `>`, so the two never overlap).
    let links = link_ranges(chunk);
    let emoji = if opts.discord_emoji { discord_tags(chunk) } else { Vec::new() };
    let mut next_emoji = emoji.iter().peekable();

    for (i, c) in chunk.char_indices() {
        while next_emoji.peek().is_some_and(|(r, _)| i >= r.end) {
            next_emoji.next();
        }
        if let Some((range, e)) = next_emoji.peek() {
            if i > range.start {
                continue;
            }
            if i == range.start {
                // An image in the span: its background, but no underline (text decoration
                // isn't drawn across an image) and not the link colour.
                let run = Run {
                    len: EMOJI_PLACEHOLDER.len_utf8(),
                    underline: false,
                    emoji: Some(std::sync::Arc::new(e.clone())),
                    ..base.clone()
                };
                out.text.push(EMOJI_PLACEHOLDER);
                out.runs.push(run);
                continue;
            }
        }
        out.text.push(c);
        let mut run = base.clone();
        run.len = c.len_utf8();
        if links.iter().any(|r| r.contains(&i)) {
            run.link = true;
            run.fg = palette.link;
            run.underline = true;
        }
        if let Some(bg) = eff.bg {
            match c {
                // processShadeChars: on a background, a shade is a solid block in the
                // blend of the two colours (it uses the unadjusted text colour).
                '░' | '▒' | '▓' => {
                    let w = match c { '░' => 0.25, '▒' => 0.5, _ => 0.75 };
                    run.fg = blend(eff.fg, bg, w);
                    run.cell = Some(Cell::Block('█'));
                }
                ' ' => run.cell = Some(Cell::Space),
                c if is_block_element(c) => run.cell = Some(Cell::Block(c)),
                c if is_box_drawing(c) => run.cell = Some(Cell::Box(c)),
                _ => {}
            }
        } else if is_block_element(c) {
            run.cell = Some(Cell::Block(c));
        } else if is_box_drawing(c) {
            run.cell = Some(Cell::Box(c));
        }
        push_run(out, run);
    }
}

/// app.js `convertDiscordEmojis`: every `<:name:id>` / `<a:name:id>`, with its byte range.
fn discord_tags(text: &str) -> Vec<(std::ops::Range<usize>, DiscordEmoji)> {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new(r"<(a?):([A-Za-z0-9_]+):(\d+)>").unwrap());
    tag.captures_iter(text)
        .map(|c| {
            let e = DiscordEmoji { id: c[3].to_string(), name: c[2].to_string(), animated: !c[1].is_empty() };
            (c.get(0).unwrap().range(), e)
        })
        .collect()
}

/// Append a one-character run, merging it into the previous run when nothing differs.
fn push_run(out: &mut StyledLine, run: Run) {
    if let Some(last) = out.runs.last_mut().filter(|l| l.emoji.is_none()) {
        let same = last.fg == run.fg
            && last.bg == run.bg
            && last.bold == run.bold
            && last.italic == run.italic
            && last.underline == run.underline
            && last.blink == run.blink
            && last.link == run.link
            && last.cell == run.cell;
        if same {
            last.len += run.len;
            return;
        }
    }
    out.runs.push(run);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::from_theme(&crate::theme::ThemeColors::dark_default())
    }

    fn style(s: &str) -> StyledLine {
        style_line(s, &palette(), &StyleOptions::default())
    }

    #[test]
    fn plain_text_is_one_default_run() {
        let l = style("hello world");
        assert_eq!(l.text, "hello world");
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].fg, palette().fg);
        assert_eq!(l.runs[0].bg, None);
    }

    #[test]
    fn sgr_colours_and_bold_upgrade() {
        let p = palette();
        let l = style("\x1b[31mred\x1b[1mbright\x1b[0m plain");
        assert_eq!(l.text, "redbright plain");
        assert_eq!(l.runs[0].fg, p.ansi[1]);
        // SGR 1 after 31 turns the red bright, as app.js does.
        assert_eq!(l.runs[1].fg, p.ansi[9]);
        assert!(l.runs[1].bold);
        assert_eq!(l.runs[2].fg, p.fg);
        // 30-37 after bold is set comes out bright.
        assert_eq!(style("\x1b[1;32mx").runs[0].fg, p.ansi[10]);
    }

    #[test]
    fn extended_colours() {
        assert_eq!(style("\x1b[38;5;196mx").runs[0].fg, [255, 0, 0]);
        assert_eq!(style("\x1b[38;5;244mx").runs[0].fg, [128, 128, 128]);
        assert_eq!(style("\x1b[38;2;1;2;3mx").runs[0].fg, [1, 2, 3]);
        assert_eq!(style("\x1b[48;2;9;8;7mx").runs[0].bg, Some([9, 8, 7]));
    }

    #[test]
    fn reverse_video_swaps_with_the_theme_background() {
        let p = palette();
        let l = style("\x1b[7mx");
        assert_eq!(l.runs[0].fg, p.bg);
        assert_eq!(l.runs[0].bg, Some(p.fg));
    }

    #[test]
    fn other_csi_and_stray_escapes_are_dropped() {
        assert_eq!(style("a\x1b[2Jb\x1b[37gc\x1bd\u{241b}e").text, "abcde");
        // An SGR whose ESC was lost is scrubbed, but `[match(` is not.
        assert_eq!(style("x[0m y [match(").text, "x y [match(");
        // ...nor when a letter follows (app.js's negative lookahead).
        assert_eq!(style("x[0my").text, "x[0my");
    }

    #[test]
    fn coloured_spaces_and_blocks_are_cells() {
        let l = style("\x1b[41m  \x1b[0m▀▀x");
        assert_eq!(l.runs[0].cell, Some(Cell::Space));
        assert_eq!(l.runs[0].len, 2);
        assert_eq!(l.runs[1].cell, Some(Cell::Block('▀')));
        assert_eq!(l.runs[2].cell, None);
    }

    #[test]
    fn shade_on_a_background_becomes_a_blended_full_block() {
        let p = palette();
        let l = style("\x1b[37;44m▒");
        assert_eq!(l.runs[0].cell, Some(Cell::Block('█')));
        assert_eq!(l.runs[0].fg, blend(p.ansi[7], p.ansi[4], 0.5));
        // Without a background it stays a shade cell.
        assert_eq!(style("▒").runs[0].cell, Some(Cell::Block('▒')));
    }

    #[test]
    fn urls_are_links_in_the_link_colour() {
        let p = palette();
        let l = style("see https://example.com/a?b=1. ok");
        let link: String = l
            .runs
            .iter()
            .scan(0, |at, r| {
                let s = *at;
                *at += r.len;
                Some((s, r))
            })
            .filter(|(_, r)| r.link)
            .map(|(s, r)| l.text[s..s + r.len].to_string())
            .collect();
        assert_eq!(link, "https://example.com/a?b=1");
        assert!(l.runs.iter().filter(|r| r.link).all(|r| r.fg == p.link && r.underline));
    }

    #[test]
    fn word_breaks_follow_insert_word_breaks() {
        // "aaaaaaaaaaaaaaaa/" is a 17-character word: a break after the slash.
        let l = style("aaaaaaaaaaaaaaaa/bbb short/x");
        assert_eq!(l.breaks, vec![17]);
        // A Discord emoji tag doesn't count toward the word.
        assert!(style("<:a_very_long_emoji_name:123456>").breaks.is_empty());
    }

    #[test]
    fn mud_tags_are_stripped_like_app_js() {
        assert_eq!(strip_mud_tag("[chat(ooc)] Bob: hi"), "Bob: hi");
        assert_eq!(strip_mud_tag("[Newbie:] hi"), "hi");
        assert_eq!(strip_mud_tag("\x1b[36m[chat(ooc)] hi"), "\x1b[36mhi");
        // Not tags: no paren content, no trailing colon, indented, no space after.
        assert_eq!(strip_mud_tag("[Newbie] Alice"), "[Newbie] Alice");
        assert_eq!(strip_mud_tag("[x()] y"), "[x()] y");
        assert_eq!(strip_mud_tag("  [chat(ooc)] hi"), "  [chat(ooc)] hi");
        assert_eq!(strip_mud_tag("[chat(ooc)]hi"), "[chat(ooc)]hi");
    }

    #[test]
    fn colour_offset_lightens_text_close_to_the_background() {
        let opts = StyleOptions { color_offset_percent: 50, ..StyleOptions::default() };
        let l = style_line("\x1b[30mx", &palette(), &opts);
        assert_ne!(l.runs[0].fg, palette().ansi[0]);
    }

    fn emoji_of(l: &StyledLine) -> Vec<(String, String, bool)> {
        l.runs.iter().filter_map(|r| r.emoji.as_ref()).map(|e| (e.name.clone(), e.id.clone(), e.animated)).collect()
    }

    #[test]
    fn discord_tags_become_one_placeholder_each() {
        let l = style("hi <:mmLol:216154654256398347> and <a:b1nzy:392938283556143104>!");
        assert_eq!(l.text, "hi \u{FFFC} and \u{FFFC}!");
        assert_eq!(
            emoji_of(&l),
            vec![
                ("mmLol".into(), "216154654256398347".into(), false),
                ("b1nzy".into(), "392938283556143104".into(), true),
            ]
        );
        // Each placeholder is a run of its own, so two emoji never merge.
        let l = style("<:a:1><:b:2>");
        assert_eq!(l.text, "\u{FFFC}\u{FFFC}");
        assert_eq!(l.runs.len(), 2);
        assert_eq!(emoji_of(&l).len(), 2);
    }

    #[test]
    fn discord_emoji_keep_the_span_background_but_no_underline() {
        let l = style("\x1b[4;44mx <:a:1> y");
        let e = l.runs.iter().find(|r| r.emoji.is_some()).unwrap();
        assert!(!e.underline);
        assert!(e.bg.is_some());
        assert!(l.runs.iter().filter(|r| r.emoji.is_none()).all(|r| r.underline));
    }

    #[test]
    fn only_real_discord_tags_convert() {
        // app.js's name pattern is [A-Za-z0-9_]: anything else stays text.
        for raw in ["<:bad-name:1>", "<:name:12a>", "<name:1>", "<::1>"] {
            let l = style(raw);
            assert_eq!(l.text, raw);
            assert!(emoji_of(&l).is_empty(), "{raw}");
        }
    }

    #[test]
    fn shown_tags_stay_as_text() {
        let opts = StyleOptions { discord_emoji: false, ..StyleOptions::default() };
        let l = style_line("<:a:1>", &palette(), &opts);
        assert_eq!(l.text, "<:a:1>");
        assert!(emoji_of(&l).is_empty());
    }

    #[test]
    fn an_emoji_counts_nothing_toward_a_long_word() {
        // insertWordBreaks skips a tag; the placeholder that replaced it does the same,
        // so the break after "/" comes at the same character count either way.
        let plain = style("aaaaaaaaaaaaaaaa/b");
        let with = style("aaaaaaaa<:a:1>aaaaaaaa/b");
        assert_eq!(plain.breaks.len(), 1);
        assert_eq!(with.breaks, vec![with.text.find('/').unwrap() + 1]);
    }

    #[test]
    fn a_link_ends_before_an_emoji() {
        let l = style("see https://example.com/x<:a:1> now");
        let link: String = l
            .text
            .char_indices()
            .filter(|(i, _)| {
                let mut at = 0;
                l.runs.iter().any(|r| {
                    let hit = r.link && (at..at + r.len).contains(i);
                    at += r.len;
                    hit
                })
            })
            .map(|(_, c)| c)
            .collect();
        assert_eq!(link, "https://example.com/x");
        assert_eq!(emoji_of(&l).len(), 1);
    }

    #[test]
    fn box_drawing_becomes_cells() {
        let l = style("┌─┐ a");
        let cells: Vec<Option<Cell>> = l.runs.iter().map(|r| r.cell).collect();
        assert_eq!(l.runs.len(), 4, "{:?}", l.runs);
        assert_eq!(cells[..3], [Some(Cell::Box('┌')), Some(Cell::Box('─')), Some(Cell::Box('┐'))]);
        assert_eq!(cells[3], None);
    }
}
