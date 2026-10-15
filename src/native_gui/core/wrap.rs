//! Wrapping one output line into rows, the way the WebView's `white-space: pre-wrap;
//! word-wrap: break-word` does it.
//!
//! - Rows break where a browser would: between two printable ASCII characters, by the
//!   table WebKit and Blink share (only before an opening bracket, after '-' and '?'), and
//!   elsewhere by Unicode's line-break rules (UAX #14). Plus the extra points
//!   `insertWordBreaks` adds inside long words ([`super::style::StyledLine::breaks`]).
//! - Spaces are kept, and the ones at the end of a row hang past the edge rather than
//!   forcing a wrap.
//! - A word too long for a row on its own is broken between graphemes.
//!
//! Widths come from the caller as the x position of every byte boundary, so this has no
//! GUI dependency and the same code serves any renderer.

use std::ops::Range;
use std::sync::OnceLock;

use unicode_segmentation::UnicodeSegmentation;

const ASCII_FIRST: u8 = b'!';
const ASCII_LAST: u8 = 0x7F;
const ASCII_N: usize = (ASCII_LAST - ASCII_FIRST + 1) as usize;

/// WebKit's and Blink's line-break table for pairs of printable ASCII characters,
/// built the way WebKit's CharacterPropertyDataGenerator builds it (`fillASCII`): later
/// rules override earlier ones.
fn ascii_table() -> &'static [[bool; ASCII_N]; ASCII_N] {
    static TABLE: OnceLock<[[bool; ASCII_N]; ASCII_N]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [[false; ASCII_N]; ASCII_N];
        const ALL: (u8, u8) = (ASCII_FIRST, ASCII_LAST);
        let mut set = |first: (u8, u8), second: (u8, u8), value: bool| {
            for a in first.0..=first.1 {
                for b in second.0..=second.1 {
                    t[(a - ASCII_FIRST) as usize][(b - ASCII_FIRST) as usize] = value;
                }
            }
        };
        let one = |c: u8| (c, c);
        for c in *b"(<[{" {
            set(ALL, one(c), true);
        }
        set(one(b'-'), ALL, true);
        set(one(b'?'), ALL, true);
        set(ALL, one(b'!'), false);
        set(one(b'-'), one(b'!'), true);
        set(one(b'?'), one(b'"'), false);
        set(one(b'?'), one(b'\''), false);
        set(ALL, one(b')'), false);
        set(one(b'-'), one(b')'), true);
        set(ALL, one(b','), false);
        set(ALL, one(b'.'), false);
        set(ALL, one(b'/'), false);
        set(one(b'-'), one(b'/'), true);
        set(one(b'-'), (b'0', b'9'), false);
        for c in *b":;?]}" {
            set(ALL, one(c), false);
            set(one(b'-'), one(c), true);
        }
        for first in [one(b'$'), one(b'\''), one(b'('), one(b'/'), (b'0', b'9'), one(b'<'), one(b'@'),
            (b'A', b'Z'), one(b'['), (b'^', b'`'), (b'a', b'z'), one(b'{'), one(0x7F)]
        {
            set(first, ALL, false);
        }
        t
    })
}

fn is_printable_ascii(c: char) -> bool {
    (ASCII_FIRST as u32..=ASCII_LAST as u32).contains(&(c as u32))
}

/// Byte offsets where a row may end (the next row starting there), as a browser decides.
fn break_opportunities(text: &str, extra: &[usize]) -> Vec<usize> {
    let uax14: Vec<usize> = unicode_linebreak::linebreaks(text).map(|(pos, _)| pos).collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out: Vec<usize> = Vec::new();
    for k in 1..chars.len() {
        let (i, c) = chars[k];
        let p = chars[k - 1].1;
        let allowed = if is_printable_ascii(p) && is_printable_ascii(c) {
            if p == '-' && c.is_ascii_digit() {
                // Hard-coded in Blink/WebKit: "-5" stays together unless the hyphen joins
                // two words ("pre-1990").
                k >= 2 && chars[k - 2].1.is_ascii_alphanumeric()
            } else {
                ascii_table()[(p as u8 - ASCII_FIRST) as usize][(c as u8 - ASCII_FIRST) as usize]
            }
        } else {
            uax14.binary_search(&i).is_ok()
        };
        if allowed {
            out.push(i);
        }
    }
    out.extend(extra.iter().copied().filter(|&p| p > 0 && p < text.len()));
    out.push(text.len());
    out.sort_unstable();
    out.dedup();
    out
}

/// Wrap `text` into rows no wider than `width`.
///
/// `x(i)` is the x position of byte boundary `i` (0 at the start of the line), and is
/// only asked about char boundaries. `indent` is how far continuation rows are pushed
/// right (wrapspace's hanging indent); the first row starts at 0.
#[allow(clippy::single_range_in_vec_init)] // one empty row for an empty line
pub fn wrap_rows(
    text: &str,
    breaks: &[usize],
    width: f32,
    indent: f32,
    x: &dyn Fn(usize) -> f32,
) -> Vec<Range<usize>> {
    if text.is_empty() {
        return vec![0..0];
    }
    let opportunities = break_opportunities(text, breaks);

    let mut rows = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let avail = if rows.is_empty() { width } else { (width - indent).max(1.0) };
        // The width a row ending at `end` needs: trailing whitespace hangs.
        let needs = |end: usize| {
            let trimmed = start + text[start..end].trim_end_matches(char::is_whitespace).len();
            x(trimmed) - x(start)
        };
        let mut end = None;
        for &o in opportunities.iter().filter(|&&o| o > start) {
            if needs(o) <= avail + 0.01 {
                end = Some(o);
            } else {
                break;
            }
        }
        let end = match end {
            Some(e) => e,
            // Nothing fits: break the word between graphemes, at least one per row.
            None => {
                let mut fit = None;
                for (i, g) in text[start..].grapheme_indices(true) {
                    let after = start + i + g.len();
                    if x(after) - x(start) <= avail + 0.01 || fit.is_none() {
                        fit = Some(after);
                    } else {
                        break;
                    }
                }
                fit.unwrap_or(text.len())
            }
        };
        rows.push(start..end);
        start = end;
    }
    rows
}

/// How many rows `text` wraps to in a grid `cols` columns wide, continuation rows
/// pushed right by `indent` columns: [`wrap_rows`] with every character as wide as a
/// terminal draws it (a tab to the next eight-column stop). The output's font is
/// monospaced, so this is the layout's count, give or take a wide glyph, for a line it
/// hasn't laid out yet (more-mode's budget for lines it holds back).
pub fn rows_in_columns(text: &str, breaks: &[usize], cols: usize, indent: usize) -> usize {
    // No character but a tab is more columns wide than it has bytes, so a line that
    // fits in bytes fits (most do): the scroll bar asks this of every line in the buffer.
    if text.len() <= cols && !text.contains('\t') {
        return 1;
    }
    let mut x = vec![0.0f32; text.len() + 1];
    let mut at = 0.0f32;
    for (i, c) in text.char_indices() {
        x[i..i + c.len_utf8()].fill(at);
        at += if c == '\t' { 8.0 - at % 8.0 } else { unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as f32 };
    }
    x[text.len()] = at;
    let cols = cols.max(1) as f32;
    if at <= cols {
        return 1;
    }
    wrap_rows(text, breaks, cols, indent as f32, &|i| x[i]).len().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_in_columns_counts_the_wrap() {
        assert_eq!(rows_in_columns("", &[], 10, 0), 1);
        assert_eq!(rows_in_columns("hello world", &[], 11, 0), 1);
        // Broken at the space, as wrap_rows does: "hello " / "world".
        assert_eq!(rows_in_columns("hello world", &[], 8, 0), 2);
        // Continuation rows are narrower by the indent.
        assert_eq!(rows_in_columns("aa bb cc dd", &[], 6, 3), 3);
        // Wide characters take two columns, a tab runs to the next stop.
        assert_eq!(rows_in_columns("日本語日本語", &[], 6, 0), 2);
        assert_eq!(rows_in_columns("a\tb", &[], 9, 0), 1);
        assert_eq!(rows_in_columns("a\tb", &[], 8, 0), 2);
        // A line no longer in bytes than the row is one row, wide characters and all.
        assert_eq!(rows_in_columns("日本語日本語", &[], 18, 0), 1);
    }

    /// Every char one unit wide.
    fn mono(text: &str) -> impl Fn(usize) -> f32 + '_ {
        move |i| text[..i].chars().count() as f32
    }

    fn rows<'a>(text: &'a str, breaks: &[usize], width: f32) -> Vec<&'a str> {
        let x = mono(text);
        wrap_rows(text, breaks, width, 0.0, &x).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn fits_on_one_row() {
        assert_eq!(rows("hello world", &[], 20.0), vec!["hello world"]);
    }

    #[test]
    fn wraps_at_spaces_and_trailing_spaces_hang() {
        assert_eq!(rows("hello world again", &[], 11.0), vec!["hello world ", "again"]);
        // "hello " is 6 wide, but its space hangs, so it fits in 5.
        assert_eq!(rows("hello world", &[], 5.0), vec!["hello ", "world"]);
    }

    #[test]
    fn long_words_break_between_graphemes() {
        assert_eq!(rows("abcdefgh", &[], 3.0), vec!["abc", "def", "gh"]);
        // An emoji ZWJ sequence is never split.
        let cat = "🐈\u{200d}⬛";
        let text = format!("{cat}{cat}");
        let x = |i: usize| text[..i].graphemes(true).count() as f32;
        let r = wrap_rows(&text, &[], 1.0, 0.0, &x);
        assert_eq!(r.len(), 2);
        assert_eq!(&text[r[0].clone()], cat);
    }

    #[test]
    fn extra_breaks_are_used() {
        // A path with no spaces: breaks only where insertWordBreaks put them.
        let text = "aaaa/bbbb/cccc";
        assert_eq!(rows(text, &[5, 10], 10.0), vec!["aaaa/bbbb/", "cccc"]);
        assert_eq!(rows(text, &[5, 10], 9.0), vec!["aaaa/", "bbbb/cccc"]);
    }

    #[test]
    fn ascii_pairs_follow_the_browser_table() {
        // No break after the slashes of a URL...
        assert_eq!(rows("x https://example", &[], 12.0), vec!["x ", "https://exam", "ple"]);
        // ...but after a hyphen, and before an opening bracket.
        assert_eq!(rows("well-known", &[], 6.0), vec!["well-", "known"]);
        assert_eq!(rows("ab.(cd", &[], 4.0), vec!["ab.", "(cd"]);
        // A letter before '(' cancels that break (a later rule in WebKit's table), so a
        // call like this only breaks where it must.
        assert_eq!(rows("func(arg)", &[], 5.0), vec!["func(", "arg)"]);
        // "-5" stays together; "pre-1990" may break after the hyphen.
        assert_eq!(rows("ab -5000", &[], 4.0), vec!["ab ", "-500", "0"]);
        assert_eq!(rows("pre-1990", &[], 5.0), vec!["pre-", "1990"]);
    }

    #[test]
    fn continuation_rows_are_narrower_by_the_indent() {
        let text = "aa bb cc dd";
        let x = mono(text);
        let r: Vec<&str> = wrap_rows(text, &[], 6.0, 3.0, &x).into_iter().map(|r| &text[r]).collect();
        assert_eq!(r, vec!["aa bb ", "cc ", "dd"]);
    }
}
