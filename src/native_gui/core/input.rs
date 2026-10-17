//! The input line's text and everything the editing keys do to it: app.js's
//! `dispatchActionImpl` editing actions and their helpers (`deleteWordBackwardKill`,
//! `transformWordForward`, the history and kill-ring functions, `completeCommand`,
//! `checkTempConversion`, `tfHistorySubstitute`), as one model with no GPUI in it.
//!
//! The one deliberate difference: app.js counts positions in UTF-16 code units, so a
//! cursor step or a Backspace can land inside an emoji. Here every step that moves over
//! or deletes "a character" moves over or deletes a whole grapheme cluster (a ZWJ family,
//! a flag, a letter with its accents), as a native text field does. Word rules keep
//! app.js's: words are separated by spaces, and the case-changing actions work on ASCII
//! letters and digits.
//!
//! Positions are byte offsets into `text`, always on a char boundary.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

/// app.js keeps this many history entries, and this many kills.
const HISTORY_MAX: usize = 1000;
const KILL_RING_MAX: usize = 100;

#[derive(Default, Debug)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
    /// The other end of a selection, when there is one.
    pub anchor: Option<usize>,
    /// Text the input method is still composing (shown underlined, not yet committed).
    pub marked: Option<Range<usize>>,
    pub history: Vec<String>,
    history_index: Option<usize>,
    /// A history prefix search in progress: the prefix and where it is.
    search: Option<(String, usize)>,
    pub kill_ring: Vec<String>,
    /// TF's %insert: off means a typed character replaces the one under the cursor.
    pub overwrite: bool,
    /// The last completion: what was being completed and which match it showed.
    completion: Option<(String, usize)>,
    /// A temperature conversion the user undid, not to be redone.
    temp_skip: Option<String>,
    /// The last change was a deletion (it may have undone a conversion).
    pub last_was_delete: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

impl Editor {
    // ---- positions ----

    fn prev_boundary(&self, i: usize) -> usize {
        self.text.grapheme_indices(true).rev().find_map(|(at, _)| (at < i).then_some(at)).unwrap_or(0)
    }

    fn next_boundary(&self, i: usize) -> usize {
        self.text.grapheme_indices(true).find_map(|(at, _)| (at > i).then_some(at)).unwrap_or(self.text.len())
    }

    /// The selection, start to end, if it isn't empty.
    pub fn selection(&self) -> Option<Range<usize>> {
        let a = self.anchor?;
        let (s, e) = if a < self.cursor { (a, self.cursor) } else { (self.cursor, a) };
        (s < e).then_some(s..e)
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.text[r])
    }

    /// Put the cursor at `pos` (snapped to a char boundary), extending the selection or
    /// dropping it.
    pub fn set_cursor(&mut self, pos: usize, extend: bool) {
        let mut pos = pos.min(self.text.len());
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = pos;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    /// Replace `range` with `with`, leaving the cursor after it.
    fn splice(&mut self, range: Range<usize>, with: &str) {
        self.text.replace_range(range.clone(), with);
        self.cursor = range.start + with.len();
        self.anchor = None;
        self.marked = None;
    }

    fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some(r) => {
                self.splice(r, "");
                true
            }
            None => false,
        }
    }

    // ---- typing ----

    /// Typed, pasted or committed text: replaces the selection, else goes in at the cursor.
    pub fn insert(&mut self, s: &str) {
        let range = self.selection().unwrap_or(self.cursor..self.cursor);
        self.splice(range, s);
        self.last_was_delete = false;
        self.completion = None;
    }

    /// Import Picture's links: in at the cursor without touching the text around them
    /// (`picture::link_insertion`; selected text stays, the links go in at its end), the
    /// cursor left at the first link's start.
    pub fn insert_link(&mut self, link: &str) {
        if let Some(sel) = self.selection() {
            self.cursor = sel.end;
        }
        self.anchor = None;
        let before = self.text[..self.cursor].chars().next_back();
        let after = self.text[self.cursor..].chars().next();
        let (text, at) = crate::picture::link_insertion(before, after, link);
        let start = self.cursor + at;
        self.insert(&text);
        self.cursor = start;
    }

    /// A typed character with %insert off: replaces the character under the cursor.
    pub fn overwrite(&mut self, s: &str) {
        let end = if self.selection().is_some() { None } else { Some(self.next_boundary(self.cursor)) };
        match end {
            Some(end) => self.splice(self.cursor..end, s),
            None => self.insert(s),
        }
        self.last_was_delete = false;
        self.completion = None;
    }

    /// One step of the `InputOps` a TF command this client ran sends (`/input`, `/grab`, the
    /// kb* functions), on this line, as the console applies its own. Positions and counts
    /// are in characters. False for a step that isn't text (an action, Submit): the
    /// window runs those.
    pub fn apply_input_op(&mut self, op: &crate::websocket::InputOp) -> bool {
        use crate::websocket::InputOp;
        let byte_at = |text: &str, chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b);
        let here = self.text[..self.cursor.min(self.text.len())].chars().count();
        match op {
            InputOp::Goto { pos } => self.set_cursor(byte_at(&self.text, *pos), false),
            InputOp::Delete { count } if *count > 0 => {
                let end = byte_at(&self.text, here + *count as usize);
                self.splice(self.cursor..end, "");
            }
            InputOp::Delete { count } => {
                let start = byte_at(&self.text, here.saturating_sub(count.unsigned_abs() as usize));
                self.splice(start..self.cursor, "");
            }
            InputOp::WordLeft => self.word_left(1),
            InputOp::WordRight => self.word_right(1),
            InputOp::Insert { text, insert: true } => self.splice(self.cursor..self.cursor, text),
            // %insert off: what follows the cursor is overwritten.
            InputOp::Insert { text, insert: false } => {
                let end = byte_at(&self.text, here + text.chars().count());
                self.splice(self.cursor..end, text);
            }
            InputOp::Action { .. } | InputOp::Submit => return false,
        }
        true
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
        self.marked = None;
    }

    // ---- cursor (n: TF's numeric prefix; negative reverses) ----

    pub fn cursor_left(&mut self, n: i64) {
        for _ in 0..n.unsigned_abs() {
            self.cursor = if n >= 0 { self.prev_boundary(self.cursor) } else { self.next_boundary(self.cursor) };
        }
        self.anchor = None;
    }

    pub fn cursor_right(&mut self, n: i64) {
        self.cursor_left(-n);
    }

    pub fn word_left(&mut self, n: i64) {
        for _ in 0..n.unsigned_abs() {
            self.cursor = if n >= 0 { word_start_before(&self.text, self.cursor) } else { word_end_after(&self.text, self.cursor) };
        }
        self.anchor = None;
    }

    pub fn word_right(&mut self, n: i64) {
        self.word_left(-n);
    }

    pub fn home(&mut self) {
        self.set_cursor(0, false);
    }

    pub fn end(&mut self) {
        self.set_cursor(self.text.len(), false);
    }

    // ---- deleting ----

    pub fn delete_backward(&mut self, n: i64) {
        self.last_was_delete = true;
        if n.abs() == 1 && self.delete_selection() {
            return;
        }
        self.anchor = None;
        for _ in 0..n.unsigned_abs() {
            let r = if n >= 0 { self.prev_boundary(self.cursor)..self.cursor } else { self.cursor..self.next_boundary(self.cursor) };
            if !r.is_empty() {
                self.splice(r.clone(), "");
                self.cursor = r.start;
            }
        }
    }

    pub fn delete_forward(&mut self, n: i64) {
        self.delete_backward(-n);
    }

    fn kill(&mut self, range: Range<usize>) {
        let killed = self.text[range.clone()].to_string();
        if !killed.is_empty() {
            self.kill_ring.push(killed);
            if self.kill_ring.len() > KILL_RING_MAX {
                self.kill_ring.remove(0);
            }
        }
        self.splice(range.clone(), "");
        self.cursor = range.start;
        self.last_was_delete = true;
    }

    /// ^W: back over spaces, then over the word (app.js `deleteWordBackwardKill`).
    pub fn delete_word_backward(&mut self, n: i64) {
        for _ in 0..n.unsigned_abs() {
            let r = if n >= 0 { word_start_before(&self.text, self.cursor)..self.cursor } else { self.cursor..word_end_after(&self.text, self.cursor) };
            self.kill(r);
        }
    }

    pub fn delete_word_forward(&mut self, n: i64) {
        self.delete_word_backward(-n);
    }

    /// Esc-Backspace: back over spaces, then one punctuation mark or a run of letters and
    /// digits (app.js `backwardKillWordPunctuationKill`). Negative: a word forward.
    pub fn delete_word_backward_punct(&mut self, n: i64) {
        for _ in 0..n.unsigned_abs() {
            if n < 0 {
                let r = self.cursor..word_end_after(&self.text, self.cursor);
                self.kill(r);
                continue;
            }
            let before: Vec<(usize, char)> = self.text[..self.cursor].char_indices().collect();
            let mut k = before.len();
            while k > 0 && before[k - 1].1 == ' ' {
                k -= 1;
            }
            if k > 0 && !before[k - 1].1.is_ascii_alphanumeric() {
                k -= 1;
            } else {
                while k > 0 && before[k - 1].1.is_ascii_alphanumeric() {
                    k -= 1;
                }
            }
            let start = before.get(k).map_or(self.cursor, |&(i, _)| i);
            self.kill(start..self.cursor);
        }
    }

    pub fn kill_to_end(&mut self) {
        self.kill(self.cursor..self.text.len());
    }

    /// ^U (TF's backward-kill-line): from the start to the cursor.
    pub fn kill_to_start(&mut self) {
        self.kill(0..self.cursor);
    }

    /// The whole line, into the kill ring.
    pub fn clear_line(&mut self) {
        if !self.text.is_empty() {
            self.kill(0..self.text.len());
        }
        self.history_index = None;
    }

    pub fn yank(&mut self) {
        if let Some(t) = self.kill_ring.last().cloned() {
            self.anchor = None;
            self.insert(&t);
        }
    }

    // ---- changing ----

    /// ^T: swap the two characters before the cursor, or around it (app.js
    /// `transposeChars`), by grapheme.
    pub fn transpose(&mut self) {
        let g: Vec<(usize, &str)> = self.text.grapheme_indices(true).collect();
        if g.len() < 2 || self.cursor == 0 {
            return;
        }
        let at = g.iter().position(|(i, _)| *i >= self.cursor).unwrap_or(g.len());
        let (a, b) = if at >= g.len() { (g.len() - 2, g.len() - 1) } else { (at - 1, at) };
        let (sa, ta) = g[a];
        let (sb, tb) = g[b];
        let swapped = format!("{tb}{ta}");
        let end = sb + tb.len();
        self.text.replace_range(sa..end, &swapped);
        self.cursor = end;
        self.anchor = None;
    }

    /// Esc-c / Esc-l / Esc-u: the next word, capitalised, lower- or upper-cased, the cursor
    /// left after it and its trailing spaces (app.js `transformWordForward`).
    pub fn transform_word(&mut self, how: Case) {
        let rest: Vec<char> = self.text[self.cursor..].chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < rest.len() && !is_word_char(rest[i]) {
            out.push(rest[i]);
            i += 1;
        }
        let mut first = true;
        while i < rest.len() && is_word_char(rest[i]) {
            let c = rest[i];
            out.push(match how {
                Case::Capitalize if first => c.to_ascii_uppercase(),
                Case::Upper => c.to_ascii_uppercase(),
                _ => c.to_ascii_lowercase(),
            });
            first = false;
            i += 1;
        }
        while i < rest.len() && rest[i] == ' ' {
            out.push(' ');
            i += 1;
        }
        let consumed: usize = rest[..i].iter().map(|c| c.len_utf8()).sum();
        let start = self.cursor;
        self.text.replace_range(start..start + consumed, &out);
        self.cursor = start + out.len();
        self.anchor = None;
    }

    /// Esc-Space: the spaces around the cursor become one.
    pub fn collapse_spaces(&mut self) {
        let b = self.text.as_bytes();
        let (mut s, mut e) = (self.cursor, self.cursor);
        while s > 0 && b[s - 1] == b' ' {
            s -= 1;
        }
        while e < b.len() && b[e] == b' ' {
            e += 1;
        }
        if e - s > 1 {
            self.splice(s..e, " ");
        }
    }

    /// Esc-m: each word's inner letters shuffled (`input::scramble_words_with`, which the
    /// console and app.js follow too).
    pub fn scramble(&mut self) {
        let chars_before = self.text[..self.cursor].chars().count();
        let out = crate::input::scramble_words_with(&self.text, &mut crate::input::Scrambler::from_entropy());
        if out != self.text {
            self.text = out;
            self.cursor = self.text.char_indices().nth(chars_before).map_or(self.text.len(), |(i, _)| i);
            self.anchor = None;
        }
    }

    /// Esc-=: to the bracket matching the one under the cursor.
    pub fn goto_matching_bracket(&mut self) {
        let b = self.text.as_bytes();
        let Some(&c) = b.get(self.cursor) else { return };
        let (open, close, forward) = match c {
            b'(' => (b'(', b')', true),
            b'[' => (b'[', b']', true),
            b'{' => (b'{', b'}', true),
            b')' => (b'(', b')', false),
            b']' => (b'[', b']', false),
            b'}' => (b'{', b'}', false),
            _ => return,
        };
        let mut depth = 0i64;
        let mut step = |i: usize| {
            if b[i] == open {
                depth += 1;
            }
            if b[i] == close {
                depth -= 1;
            }
            depth == 0
        };
        let found = if forward { (self.cursor..b.len()).find(|&i| step(i)) } else { (0..=self.cursor).rev().find(|&i| step(i)) };
        if let Some(i) = found {
            self.set_cursor(i, false);
        }
    }

    /// Esc-. / Esc-_: the last word of the previous command, at the cursor.
    pub fn insert_last_arg(&mut self) {
        let Some(word) = self.history.last().and_then(|h| h.split_whitespace().last()).map(str::to_string) else { return };
        self.anchor = None;
        self.insert(&word);
    }

    // ---- history ----

    /// After a command is sent: remembered, unless it was typed at a password prompt.
    pub fn remember(&mut self, cmd: &str, masked: bool) {
        if !cmd.is_empty() && !masked {
            self.history.push(cmd.to_string());
            if self.history.len() > HISTORY_MAX {
                self.history.remove(0);
            }
        }
        self.history_index = None;
    }

    fn show(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.anchor = None;
        self.marked = None;
    }

    fn history_prev_once(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let i = match self.history_index {
            None => self.history.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.history_index = Some(i);
        self.show(self.history[i].clone());
    }

    fn history_next_once(&mut self) {
        let Some(i) = self.history_index else { return };
        if i + 1 < self.history.len() {
            self.history_index = Some(i + 1);
            self.show(self.history[i + 1].clone());
        } else {
            self.history_index = None;
            self.show(String::new());
        }
    }

    pub fn history_prev(&mut self, n: i64) {
        for _ in 0..n.unsigned_abs() {
            if n >= 0 { self.history_prev_once() } else { self.history_next_once() }
        }
    }

    pub fn history_next(&mut self, n: i64) {
        self.history_prev(-n);
    }

    fn history_goto(&mut self, i: usize) {
        self.history_index = Some(i);
        self.show(self.history[i].clone());
    }

    /// The n-th entry from the oldest end (n = 1 is the first command).
    pub fn recall_begin(&mut self, n: i64) {
        if self.history.is_empty() {
            return;
        }
        if n < 0 {
            return self.recall_end(-n);
        }
        let last = self.history.len() as i64 - 1;
        self.history_goto((n - 1).clamp(0, last) as usize);
    }

    /// The n-th entry from the newest end.
    pub fn recall_end(&mut self, n: i64) {
        if self.history.is_empty() {
            return;
        }
        if n < 0 {
            return self.recall_begin(-n);
        }
        let last = self.history.len() as i64 - 1;
        self.history_goto((last - (n - 1)).clamp(0, last) as usize);
    }

    /// Esc-p: back to the next older command starting with what was typed.
    pub fn search_backward(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let (prefix, from) = self.search.clone().unwrap_or_else(|| (self.text.clone(), self.history.len()));
        if let Some(i) = (0..from).rev().find(|&i| self.history[i].starts_with(&prefix)) {
            self.show(self.history[i].clone());
            self.search = Some((prefix, i));
        } else {
            self.search = Some((prefix, from));
        }
    }

    /// Esc-n: forward again; past the newest, back to what was typed.
    pub fn search_forward(&mut self) {
        let Some((prefix, from)) = self.search.clone() else { return };
        if let Some(i) = (from + 1..self.history.len()).find(|&i| self.history[i].starts_with(&prefix)) {
            self.show(self.history[i].clone());
            self.search = Some((prefix, i));
        } else {
            self.show(prefix.clone());
            self.search = Some((prefix, self.history.len()));
        }
    }

    pub fn end_search(&mut self) {
        self.search = None;
    }

    pub fn search_backward_n(&mut self, n: i64) {
        for _ in 0..n.unsigned_abs() {
            if n >= 0 { self.search_backward() } else { self.search_forward() }
        }
    }

    pub fn search_forward_n(&mut self, n: i64) {
        self.search_backward_n(-n);
    }

    // ---- completion ----

    /// Tab on a `/command`: the next command starting with what's typed, cycling on
    /// repeated Tabs; the arguments after it stay (app.js `completeCommand` /
    /// `performCompletion`). False when there's nothing to complete.
    pub fn complete(&mut self, commands: &[String]) -> bool {
        if !self.text.starts_with('/') {
            return false;
        }
        let space = self.text.find(' ');
        if space.is_some_and(|s| self.cursor > s) {
            return false;
        }
        let (partial, args) = self.text.split_at(space.unwrap_or(self.text.len()));
        let lower = partial.to_lowercase();
        let mut matches: Vec<&String> = commands.iter().filter(|c| c.to_lowercase().starts_with(&lower)).collect();
        if matches.is_empty() {
            return false;
        }
        matches.sort();
        matches.dedup();
        let next = match &self.completion {
            Some((prefix, i)) if prefix.to_lowercase() == lower => (i + 1) % matches.len(),
            _ => matches.iter().position(|m| m.to_lowercase() == lower).map_or(0, |i| (i + 1) % matches.len()),
        };
        let completed = format!("{}{}", matches[next], args);
        self.completion = Some((partial.to_string(), next));
        self.text = completed;
        self.cursor = self.text.find(' ').unwrap_or(self.text.len());
        self.anchor = None;
        true
    }

    pub fn reset_completion(&mut self) {
        self.completion = None;
    }

    // ---- temperature conversion ----

    /// Typing a temperature and then a separator ("32F ") puts its conversion before the
    /// separator ("32F(0C) "), when the cursor is at the end (app.js
    /// `checkTempConversion`). A conversion the user undoes isn't redone.
    pub fn convert_temperature(&mut self) {
        if self.text.is_empty() || self.last_was_delete || self.cursor != self.text.len() {
            return;
        }
        let last = self.text.chars().last().expect("not empty");
        if !(last.is_whitespace() || ".,!?;:)]}".contains(last)) {
            self.temp_skip = None;
            return;
        }
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"(-?(?:\d+\.?\d*|\.\d+))(°?[FfCc])$").expect("valid"));
        let before = &self.text[..self.text.len() - last.len_utf8()];
        let Some(m) = re.captures(before) else { return };
        let whole = m.get(0).expect("match");
        if before[..whole.start()].chars().last().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            return;
        }
        let temp = whole.as_str().to_string();
        if self.temp_skip.as_deref() == Some(temp.as_str()) {
            return;
        }
        let Ok(value) = m[1].parse::<f64>() else { return };
        let fahrenheit = m[2].to_ascii_uppercase().ends_with('F');
        let (converted, unit) = if fahrenheit { ((value - 32.0) * 5.0 / 9.0, 'C') } else { (value * 9.0 / 5.0 + 32.0, 'F') };
        let shown = if (converted - converted.round()).abs() < 0.05 { format!("({}{unit})", converted.round() as i64) } else { format!("({converted:.1}{unit})") };
        self.temp_skip = Some(temp);
        self.text = format!("{before}{shown}{last}");
        self.cursor = self.text.len();
    }

    // ---- UTF-16 offsets, for the input method ----

    pub fn to_utf16(&self, offset: usize) -> usize {
        self.text[..offset.min(self.text.len())].encode_utf16().count()
    }

    pub fn from_utf16(&self, offset: usize) -> usize {
        let mut units = 0;
        for (i, c) in self.text.char_indices() {
            if units >= offset {
                return i;
            }
            units += c.len_utf16();
        }
        self.text.len()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Case {
    Capitalize,
    Lower,
    Upper,
}

/// Back over spaces, then over the word before them.
fn word_start_before(text: &str, from: usize) -> usize {
    let b = text.as_bytes();
    let mut i = from;
    while i > 0 && b[i - 1] == b' ' {
        i -= 1;
    }
    while i > 0 && b[i - 1] != b' ' {
        i -= 1;
    }
    i
}

/// Forward over spaces, then over the word after them.
fn word_end_after(text: &str, from: usize) -> usize {
    let b = text.as_bytes();
    let mut i = from;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    while i < b.len() && b[i] != b' ' {
        i += 1;
    }
    i
}

/// TF's `^old^new`: the newest command containing `old`, its first `old` made `new`.
/// `None` when the line isn't one; `Some(Err(()))` when nothing in history matches.
pub fn history_substitute(cmd: &str, history: &[String]) -> Option<Result<String, ()>> {
    let rest = cmd.strip_prefix('^')?;
    let (old, new) = rest.split_once('^')?;
    Some(history.iter().rev().find_map(|h| h.find(old).map(|at| format!("{}{}{}", &h[..at], new, &h[at + old.len()..]))).ok_or(()))
}

/// The commands Tab completes: app.js's `INTERNAL_COMMANDS` (read from the web client
/// the server already embeds, so the two lists can't drift), as `/name`.
pub fn internal_commands() -> &'static [String] {
    static LIST: OnceLock<Vec<String>> = OnceLock::new();
    LIST.get_or_init(|| crate::http::internal_command_names().iter().map(|name| format!("/{name}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str, cursor: usize) -> Editor {
        Editor { text: text.into(), cursor, ..Default::default() }
    }

    #[test]
    fn steps_and_deletes_whole_graphemes() {
        // A ZWJ family, a flag and an accented letter are one step each.
        let family = "👨\u{200d}👩\u{200d}👧";
        let mut e = ed(&format!("a{family}🇺🇸e\u{301}"), 0);
        let n = e.text.len();
        e.end();
        e.delete_backward(1);
        assert_eq!(e.text, format!("a{family}🇺🇸"), "e + combining accent goes as one");
        e.delete_backward(1);
        assert_eq!(e.text, format!("a{family}"), "a flag goes as one");
        e.cursor_left(1);
        assert_eq!(e.cursor, 1, "over the whole family");
        e.delete_forward(1);
        assert_eq!(e.text, "a");
        assert!(n > 0);
    }

    #[test]
    fn a_numeric_prefix_repeats_and_a_negative_one_reverses() {
        let mut e = ed("abcdef", 3);
        e.cursor_left(2);
        assert_eq!(e.cursor, 1);
        e.cursor_left(-3);
        assert_eq!(e.cursor, 4);
        e.delete_backward(-2);
        assert_eq!(e.text, "abcd");
    }

    #[test]
    fn kills_go_to_the_ring_and_yank_brings_the_last_back() {
        let mut e = ed("say hello there", 15);
        e.delete_word_backward(1);
        assert_eq!(e.text, "say hello ");
        e.kill_to_start();
        assert_eq!((e.text.as_str(), e.cursor), ("", 0));
        e.yank();
        assert_eq!(e.text, "say hello ");
        assert_eq!(e.kill_ring, vec!["there".to_string(), "say hello ".to_string()]);
        let mut e = ed("go north, then", 9);
        e.delete_word_backward_punct(1);
        assert_eq!(e.text, "go north then", "one punctuation mark");
        e.delete_word_backward_punct(1);
        assert_eq!(e.text, "go  then");
    }

    #[test]
    fn words_change_case_and_characters_transpose() {
        let mut e = ed("hello WORLD", 0);
        e.transform_word(Case::Capitalize);
        assert_eq!((e.text.as_str(), e.cursor), ("Hello WORLD", 6));
        e.transform_word(Case::Lower);
        assert_eq!(e.text, "Hello world");
        let mut e = ed("ab🇺🇸", 7);
        e.transpose();
        assert_eq!(e.text, "a🇺🇸b", "by grapheme, at the end the last two");
        let mut e = ed("a    b", 3);
        e.collapse_spaces();
        assert_eq!((e.text.as_str(), e.cursor), ("a b", 2));
        let mut e = ed("f(a[b])", 1);
        e.goto_matching_bracket();
        assert_eq!(e.cursor, 6);
    }

    #[test]
    fn history_walks_searches_and_substitutes() {
        let mut e = Editor::default();
        for c in ["north", "say hi", "south"] {
            e.remember(c, false);
        }
        e.remember("secret", true);
        assert_eq!(e.history.len(), 3, "not at a password prompt");
        e.history_prev(1);
        assert_eq!(e.text, "south");
        e.history_prev(2);
        assert_eq!(e.text, "north");
        e.history_next(5);
        assert_eq!(e.text, "", "past the newest: empty");
        e.text = "s".into();
        e.search_backward();
        assert_eq!(e.text, "south");
        e.search_backward();
        assert_eq!(e.text, "say hi");
        e.search_forward();
        e.search_forward();
        assert_eq!(e.text, "s", "past the newest: what was typed");
        e.recall_begin(1);
        assert_eq!(e.text, "north");
        assert_eq!(history_substitute("^hi^bye", &e.history), Some(Ok("say bye".into())));
        assert_eq!(history_substitute("^zz^y", &e.history), Some(Err(())));
        assert_eq!(history_substitute("hello", &e.history), None);
        e.clear();
        e.insert_last_arg();
        assert_eq!(e.text, "south");
    }

    #[test]
    fn tab_completes_and_cycles_keeping_the_arguments() {
        let cmds: Vec<String> = ["/connect", "/connections", "/cd", "/dc"].iter().map(|s| s.to_string()).collect();
        let mut e = ed("/con world", 2);
        assert!(e.complete(&cmds));
        assert_eq!((e.text.as_str(), e.cursor), ("/connect world", 8));
        assert!(e.complete(&cmds));
        assert_eq!(e.text, "/connections world");
        assert!(e.complete(&cmds));
        assert_eq!(e.text, "/connections world", "narrowed to what's now typed, as app.js does");
        let mut e = ed("/dc x", 5);
        assert!(!e.complete(&cmds), "the cursor is in the arguments");
        assert!(!ed("say", 3).complete(&cmds));
        let list = internal_commands();
        assert!(list.contains(&"/connect".to_string()) && list.contains(&"/nogag".to_string()), "{} commands", list.len());
    }

    #[test]
    fn temperatures_convert_once_and_not_after_an_undo() {
        let mut e = ed("it is 32F ", 10);
        e.convert_temperature();
        assert_eq!(e.text, "it is 32F(0C) ");
        let mut e = ed("x -10.5c,", 9);
        e.convert_temperature();
        assert_eq!(e.text, "x -10.5c(13.1F),");
        let mut e = ed("abc32F ", 7);
        e.convert_temperature();
        assert_eq!(e.text, "abc32F ", "part of a word");
        // Undone (the user deleted it back to "32F "): not converted again.
        let mut e = ed("32F ", 4);
        e.convert_temperature();
        e.text = "32F ".into();
        e.cursor = 4;
        e.convert_temperature();
        assert_eq!(e.text, "32F ");
    }

    #[test]
    fn typing_replaces_a_selection_and_overwrite_replaces_a_character() {
        let mut e = ed("hello", 5);
        e.set_cursor(1, false);
        e.set_cursor(4, true);
        assert_eq!(e.selected_text(), Some("ell"));
        e.insert("ipp");
        assert_eq!(e.text, "hippo");
        let mut e = ed("abc", 1);
        e.overwrite("X");
        assert_eq!((e.text.as_str(), e.cursor), ("aXc", 2));
        assert_eq!(e.to_utf16(e.text.len()), 3);
        let e = ed("a😀b", 0);
        assert_eq!(e.to_utf16(5), 3, "the emoji is two UTF-16 units");
        assert_eq!(e.from_utf16(3), 5);
    }
}
