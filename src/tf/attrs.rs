//! TF display attributes (`/help attributes`).
//!
//! An attribute list is any of "n" (none), "x" (exclusive), "g" (gag), "G" (nohistory),
//! "L" (nolog), "A" (noactivity), "u" (underline), "r" (reverse), "B" (bold), "b"
//! (bell), "h" (hilite), "E" (error), "W" (warning), "f"/"d" (accepted, ignored) and
//! "C<color>", with commas optional between single letters ("BuCred,Cbgyellow"). Clay's
//! older long names ("gag", "bold", "hilite:red", ...) are still accepted.
//!
//! "h", "E" and "W" stand for the attributes in %hiliteattr, %error_attr and
//! %warning_attr; `expand` replaces them, which TF does when a macro is defined
//! (`/def -ah` lists as `-aB`). `canonical` prints TF's own order, verified against
//! real tf: `-aBu` lists as `-auB`, `-aCbgblue,Cyellow,B` as `-aBCyellow,Cbgblue`, and
//! a numeric color as TF's name for it (`C200` lists as `Crgb504`).

/// One attribute list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TfAttributes {
    /// "x": earlier attributes are turned off before these apply (see `merge`).
    pub exclusive: bool,
    pub gag: bool,
    /// "G": the line is not recorded in history.
    pub norecord: bool,
    /// "L": the line is not written to a log file.
    pub nolog: bool,
    /// "A": the line causes no activity.
    pub noactivity: bool,
    pub underline: bool,
    pub reverse: bool,
    pub bold: bool,
    pub bell: bool,
    /// "h"/"E"/"W" not yet replaced by %hiliteattr/%error_attr/%warning_attr.
    pub hilite: bool,
    pub error: bool,
    pub warning: bool,
    /// Foreground and background color, by TF's name for them ("red", "brightred",
    /// "rgb504", "gray12").
    pub fg: Option<String>,
    pub bg: Option<String>,
}

const BASIC_COLORS: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

impl TfAttributes {
    /// Parse an attribute list.
    pub fn parse(text: &str) -> Result<TfAttributes, String> {
        let mut out = TfAttributes::default();
        for item in text.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let lower = item.to_lowercase();
            if let Some(color) = lower.strip_prefix("hilite:") {
                // Clay's older long form: a hilite color
                if color == "hilite" {
                    out.hilite = true;
                } else {
                    out.set_color(color)?;
                }
                continue;
            }
            match lower.as_str() {
                "gag" => { out.gag = true; continue; }
                "norecord" | "nohistory" => { out.norecord = true; continue; }
                "nolog" => { out.nolog = true; continue; }
                "noactivity" => { out.noactivity = true; continue; }
                "bold" => { out.bold = true; continue; }
                "underline" => { out.underline = true; continue; }
                "reverse" => { out.reverse = true; continue; }
                "flash" | "dim" => continue,
                "bell" => { out.bell = true; continue; }
                "hilite" => { out.hilite = true; continue; }
                "none" | "normal" => continue,
                _ => {}
            }
            let chars: Vec<char> = item.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                match chars[i] {
                    'n' | 'f' | 'd' => {}
                    'x' => out.exclusive = true,
                    'g' => out.gag = true,
                    'G' => out.norecord = true,
                    'L' => out.nolog = true,
                    'A' => out.noactivity = true,
                    'u' => out.underline = true,
                    'r' => out.reverse = true,
                    'B' => out.bold = true,
                    'b' => out.bell = true,
                    'h' => out.hilite = true,
                    'E' => out.error = true,
                    'W' => out.warning = true,
                    'C' => {
                        // A color name runs to the end of this comma-separated item
                        let name: String = chars[i + 1..].iter().collect();
                        out.set_color(&name)?;
                        i = chars.len();
                        continue;
                    }
                    c => return Err(format!("invalid display attribute '{}'", c)),
                }
                i += 1;
            }
        }
        Ok(out)
    }

    /// Set the foreground ("red") or background ("bgred") color.
    fn set_color(&mut self, name: &str) -> Result<(), String> {
        let (is_bg, base) = match name.strip_prefix("bg") {
            Some(rest) => (true, rest),
            None => (false, name),
        };
        let canonical = canonical_color(base).ok_or_else(|| format!("invalid color name '{}'", name))?;
        if is_bg {
            self.bg = Some(canonical);
        } else {
            self.fg = Some(canonical);
        }
        Ok(())
    }

    /// Nothing set at all.
    pub fn is_empty(&self) -> bool {
        *self == TfAttributes::default()
    }

    /// Combine `other` into these attributes: TF combines them, except that an
    /// exclusive ("x") list turns the earlier display attributes off first.
    pub fn merge(&mut self, other: &TfAttributes) {
        if other.exclusive {
            self.underline = false;
            self.reverse = false;
            self.bold = false;
            self.fg = None;
            self.bg = None;
        }
        self.exclusive |= other.exclusive;
        self.gag |= other.gag;
        self.norecord |= other.norecord;
        self.nolog |= other.nolog;
        self.noactivity |= other.noactivity;
        self.underline |= other.underline;
        self.reverse |= other.reverse;
        self.bold |= other.bold;
        self.bell |= other.bell;
        self.hilite |= other.hilite;
        self.error |= other.error;
        self.warning |= other.warning;
        if other.fg.is_some() {
            self.fg = other.fg.clone();
        }
        if other.bg.is_some() {
            self.bg = other.bg.clone();
        }
    }

    /// Replace "h", "E" and "W" with the attributes in %hiliteattr, %error_attr and
    /// %warning_attr (unparseable values contribute nothing).
    pub fn expand(&mut self, hiliteattr: &str, error_attr: &str, warning_attr: &str) {
        for (flag, value) in [(self.hilite, hiliteattr), (self.error, error_attr), (self.warning, warning_attr)] {
            if flag {
                if let Ok(extra) = TfAttributes::parse(value) {
                    let mut extra = extra;
                    extra.hilite = false;
                    extra.error = false;
                    extra.warning = false;
                    self.merge(&extra);
                }
            }
        }
        self.hilite = false;
        self.error = false;
        self.warning = false;
    }

    /// TF's own spelling of these attributes, as `/list` shows them: letters in TF's
    /// order, then the foreground color, then the background color.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        for (on, c) in [
            (self.exclusive, 'x'), (self.gag, 'g'), (self.norecord, 'G'), (self.nolog, 'L'),
            (self.noactivity, 'A'), (self.underline, 'u'), (self.reverse, 'r'), (self.bold, 'B'),
            (self.bell, 'b'), (self.hilite, 'h'), (self.error, 'E'), (self.warning, 'W'),
        ] {
            if on {
                out.push(c);
            }
        }
        if let Some(fg) = &self.fg {
            out.push('C');
            out.push_str(fg);
        }
        if let Some(bg) = &self.bg {
            if self.fg.is_some() {
                out.push(',');
            }
            out.push_str("Cbg");
            out.push_str(bg);
        }
        out
    }

    /// The ANSI SGR sequence that shows these attributes ("" when they show nothing).
    pub fn to_sgr(&self) -> String {
        let mut codes: Vec<String> = Vec::new();
        if self.bold {
            codes.push("1".to_string());
        }
        if self.underline {
            codes.push("4".to_string());
        }
        if self.reverse {
            codes.push("7".to_string());
        }
        if let Some(fg) = &self.fg {
            if let Some(c) = color_sgr(fg, false) {
                codes.push(c);
            }
        }
        if let Some(bg) = &self.bg {
            if let Some(c) = color_sgr(bg, true) {
                codes.push(c);
            }
        }
        if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        }
    }
}

/// One piece of raw MUD text: an escape sequence (kept whole) or a visible character.
enum Piece<'a> {
    Escape(&'a str),
    Char(char),
}

/// Split raw text into escape sequences and characters: CSI (`ESC [ ... final`), OSC
/// (`ESC ] ... BEL` or `ESC \`), or any other two-character `ESC x`.
fn pieces(raw: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut iter = raw.char_indices().peekable();
    while let Some((start, c)) = iter.next() {
        if c != '\x1b' {
            out.push(Piece::Char(c));
            continue;
        }
        let mut end = start + 1;
        match iter.peek().map(|&(_, n)| n) {
            Some('[') => {
                iter.next();
                end += 1;
                for (i, n) in iter.by_ref() {
                    end = i + n.len_utf8();
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
            }
            Some(']') => {
                iter.next();
                end += 1;
                let mut prev_esc = false;
                for (i, n) in iter.by_ref() {
                    end = i + n.len_utf8();
                    if n == '\x07' || (prev_esc && n == '\\') {
                        break;
                    }
                    prev_esc = n == '\x1b';
                }
            }
            Some(n) => {
                iter.next();
                end += n.len_utf8();
            }
            None => {}
        }
        out.push(Piece::Escape(&raw[start..end]));
    }
    out
}

/// What an SGR escape does to the color state it is applied to: None if `seq` is not an
/// SGR at all, Some(true) if it resets everything first (`ESC[m`, `ESC[0m`, `ESC[0;...m`).
fn sgr_resets(seq: &str) -> Option<bool> {
    let params = seq.strip_prefix("\x1b[")?.strip_suffix('m')?;
    Some(params.is_empty() || params == "0" || params.starts_with("0;"))
}

/// Show `raw` (MUD text, with its own ANSI codes) with TF display attributes: `line`'s
/// over all of it, and each partial's - `(start, end)` in visible characters - over
/// that part, later ones winning where they overlap (TF's -P). The line's own colors
/// stay where nothing is laid over them, and come back after a part that was.
pub fn apply_to_line(raw: &str, line: &TfAttributes, partials: &[(usize, usize, TfAttributes)]) -> String {
    let overlay_at = |i: usize| -> String {
        let mut attrs = line.clone();
        for (start, end, part) in partials {
            if (*start..*end).contains(&i) {
                attrs.merge(part);
            }
        }
        attrs.to_sgr()
    };
    let mut out = String::with_capacity(raw.len() + 16);
    // The line's own SGR codes since its last reset: what to restore after an overlay.
    let mut own_sgr = String::new();
    let mut active = String::new();
    let mut visible = 0;
    for piece in pieces(raw) {
        match piece {
            Piece::Escape(seq) => {
                out.push_str(seq);
                match sgr_resets(seq) {
                    Some(true) => own_sgr = seq.to_string(),
                    Some(false) => own_sgr.push_str(seq),
                    None => continue,
                }
                // The line changed its colors under the overlay: lay it back on top.
                out.push_str(&active);
            }
            Piece::Char(c) => {
                let wanted = overlay_at(visible);
                if wanted != active {
                    if !active.is_empty() {
                        out.push_str("\x1b[0m");
                        out.push_str(&own_sgr);
                    }
                    out.push_str(&wanted);
                    active = wanted;
                }
                out.push(c);
                visible += 1;
            }
        }
    }
    if !active.is_empty() {
        out.push_str("\x1b[0m");
        out.push_str(&own_sgr);
    }
    out
}

/// TF's name for a color: one of the 8 basic names, "bright" + one of them, "rgbRGB"
/// (each 0-5), "grayN" (0-23), or a number 0-255 converted to one of those. None when
/// it names no color.
pub fn canonical_color(name: &str) -> Option<String> {
    let lower = name.trim().to_lowercase();
    if BASIC_COLORS.contains(&lower.as_str()) {
        return Some(lower);
    }
    if let Some(rest) = lower.strip_prefix("bright") {
        if BASIC_COLORS.contains(&rest) {
            return Some(lower);
        }
    }
    if let Some(rest) = lower.strip_prefix("rgb") {
        let digits: Vec<u32> = rest.chars().filter_map(|c| c.to_digit(10)).collect();
        if rest.len() == 3 && digits.len() == 3 && digits.iter().all(|d| *d <= 5) {
            return Some(lower);
        }
        return None;
    }
    for prefix in ["gray", "grey"] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return match rest.parse::<u32>() {
                Ok(n) if n <= 23 => Some(format!("gray{}", n)),
                _ => None,
            };
        }
    }
    if let Ok(n) = lower.parse::<u32>() {
        return match n {
            0..=7 => Some(BASIC_COLORS[n as usize].to_string()),
            8..=15 => Some(format!("bright{}", BASIC_COLORS[(n - 8) as usize])),
            16..=231 => {
                let i = n - 16;
                Some(format!("rgb{}{}{}", i / 36, (i / 6) % 6, i % 6))
            }
            232..=255 => Some(format!("gray{}", n - 232)),
            _ => None,
        };
    }
    None
}

/// The SGR parameter for a color name from `canonical_color`.
fn color_sgr(name: &str, bg: bool) -> Option<String> {
    let base = if bg { 40 } else { 30 };
    if let Some(i) = BASIC_COLORS.iter().position(|c| *c == name) {
        return Some((base + i).to_string());
    }
    if let Some(rest) = name.strip_prefix("bright") {
        if let Some(i) = BASIC_COLORS.iter().position(|c| *c == rest) {
            return Some((base + 60 + i).to_string());
        }
    }
    let index = if let Some(rest) = name.strip_prefix("rgb") {
        let d: Vec<u32> = rest.chars().filter_map(|c| c.to_digit(10)).collect();
        if d.len() != 3 {
            return None;
        }
        16 + d[0] * 36 + d[1] * 6 + d[2]
    } else if let Some(rest) = name.strip_prefix("gray") {
        232 + rest.parse::<u32>().ok()?
    } else {
        return None;
    };
    Some(format!("{};5;{}", if bg { 48 } else { 38 }, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(text: &str) -> TfAttributes {
        TfAttributes::parse(text).unwrap()
    }

    /// A trigger's attributes over a whole line, over the line's own colors (put back
    /// after its resets), and -P parts over pieces of it - multi-byte text included.
    #[test]
    fn test_apply_to_line() {
        assert_eq!(apply_to_line("hello", &attrs("B"), &[]), "\x1b[1mhello\x1b[0m");
        assert_eq!(apply_to_line("hello", &TfAttributes::default(), &[]), "hello");
        // The server's own reset mid-line doesn't drop the overlay.
        assert_eq!(apply_to_line("a\x1b[0mb", &attrs("u"), &[]), "\x1b[4ma\x1b[0m\x1b[4mb\x1b[0m\x1b[0m");
        // A part: only "é2" (chars 1..3) is red; the server's green comes back after.
        let out = apply_to_line("\x1b[32mxé2y", &TfAttributes::default(), &[(1, 3, attrs("Cred"))]);
        assert_eq!(out, "\x1b[32mx\x1b[31mé2\x1b[0m\x1b[32my");
        // Overlapping: the later part wins, a line attribute stays under both.
        let out = apply_to_line("abc", &attrs("u"), &[(0, 2, attrs("Cred")), (1, 3, attrs("Cgreen"))]);
        assert_eq!(out, "\x1b[4;31ma\x1b[0m\x1b[4;32mbc\x1b[0m");
        // Non-SGR escapes (an OSC link) pass through untouched.
        let link = "\x1b]8;;http://x\x07link\x1b]8;;\x07";
        assert_eq!(apply_to_line(link, &attrs("B"), &[]), format!("\x1b]8;;http://x\x07\x1b[1mlink\x1b]8;;\x07\x1b[0m"));
    }

    #[test]
    fn test_parse_and_canonical_match_tf() {
        // Each pair verified against real tf's /list output.
        for (input, listed) in [
            ("bBrLuAGgx", "xgGLAurBb"),
            ("Bu", "uB"),
            ("Cbgblue,Cyellow,B", "BCyellow,Cbgblue"),
            ("Cred,Cbggreen,u", "uCred,Cbggreen"),
            ("Crgb123", "Crgb123"),
            ("Cgray12", "Cgray12"),
            ("C200", "Crgb504"),
            ("Cbrightred", "Cbrightred"),
            ("Cbgred,Cblue,Cgreen", "Cgreen,Cbgred"),
            ("n", ""),
            ("df", ""),
        ] {
            assert_eq!(TfAttributes::parse(input).unwrap().canonical(), listed, "input {input}");
        }
    }

    #[test]
    fn test_expand_and_merge() {
        let mut a = TfAttributes::parse("h").unwrap();
        a.expand("B", "", "");
        assert_eq!(a.canonical(), "B");
        let mut e = TfAttributes::parse("E,u").unwrap();
        e.expand("B", "Cred", "");
        assert_eq!(e.canonical(), "uCred");
        let mut base = TfAttributes::parse("uCred").unwrap();
        base.merge(&TfAttributes::parse("xr").unwrap());
        assert_eq!(base.canonical(), "xr");
    }

    #[test]
    fn test_long_names_still_accepted() {
        let a = TfAttributes::parse("gag,bold,hilite:red").unwrap();
        assert!(a.gag && a.bold);
        assert_eq!(a.fg.as_deref(), Some("red"));
    }

    #[test]
    fn test_sgr() {
        assert_eq!(TfAttributes::parse("BuCred").unwrap().to_sgr(), "\x1b[1;4;31m");
        assert_eq!(TfAttributes::parse("Cbgbrightblue").unwrap().to_sgr(), "\x1b[104m");
        assert_eq!(TfAttributes::parse("Crgb504").unwrap().to_sgr(), "\x1b[38;5;200m");
        assert_eq!(TfAttributes::parse("g").unwrap().to_sgr(), "");
    }
}
