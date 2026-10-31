//! Which words of the input to underline as misspelled: the console's rules
//! (`App::find_misspelled_words_in_window`), over the whole line.
//!
//! - A word is letters, with apostrophes inside it ("don't").
//! - The word the cursor is in is being typed: it isn't judged.
//! - A word is judged once it's finished: a separator follows it, or it ends the line.
//!   A word at the very end with the cursor right after it keeps its last verdict, so it
//!   doesn't flicker while being typed or backspaced through.
//! - Anything else (a word run into punctuation that isn't a separator) isn't judged.

use std::ops::Range;

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}' | '"' | '%' | '@' | '#' | '$' | '^' | '&' | '*' | '(' | '[' | '{')
}

/// The words of `text` (byte ranges), with apostrophes kept only between letters.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let in_word = |k: usize| {
        let c = chars[k].1;
        c.is_alphabetic() || (c == '\'' && k > 0 && chars[k - 1].1.is_alphabetic() && chars.get(k + 1).is_some_and(|n| n.1.is_alphabetic()))
    };
    let mut out = Vec::new();
    let mut k = 0;
    while k < chars.len() {
        if !chars[k].1.is_alphabetic() {
            k += 1;
            continue;
        }
        let start = k;
        while k < chars.len() && in_word(k) {
            k += 1;
        }
        let end = chars.get(k).map_or(text.len(), |c| c.0);
        out.push(chars[start].0..end);
    }
    out
}

/// The misspelled words, given the last verdicts (`cached`) and a dictionary.
pub fn misspelled(text: &str, cursor: usize, cached: &[Range<usize>], valid: &dyn Fn(&str) -> bool) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for w in words(text) {
        if cursor >= w.start && cursor < w.end {
            continue;
        }
        let at_end = w.end == text.len();
        let flagged = if at_end && cursor == w.end {
            cached.iter().any(|c| w.start < c.end && w.end > c.start)
        } else if at_end || text[w.end..].chars().next().is_some_and(is_separator) {
            !valid(&text[w.clone()])
        } else {
            false
        };
        if flagged {
            out.push(w);
        }
    }
    out
}

/// The word Ctrl+Q works on (the console's `InputArea::current_word`): the one the cursor
/// is in or at the end of, else the nearest one before it ("teh, |" is "teh").
pub fn word_at(text: &str, at: usize) -> Option<Range<usize>> {
    words(text).into_iter().take_while(|w| w.start <= at).last()
}

/// A Ctrl+Q round (the console's `SpellState`): the word's suggestions put in its place one
/// press at a time, the word as typed last, then round again.
#[derive(Debug)]
pub struct Round {
    /// Where the word is now, and what the last press put there.
    word: Range<usize>,
    current: String,
    /// The suggestions, then the word as typed.
    list: Vec<String>,
    next: usize,
}

impl Round {
    pub fn new(word: Range<usize>, typed: String, mut suggestions: Vec<String>) -> Round {
        suggestions.push(typed.clone());
        Round { word, current: typed, list: suggestions, next: 0 }
    }

    /// Whether a press now carries the round on: the word is still the one it last put
    /// there, with no letters run onto it, and the cursor is in it, at its end, or one
    /// character past it (the console's `check_word_ended`, for a press after the space
    /// that follows the word).
    pub fn applies(&self, text: &str, cursor: usize) -> bool {
        if text.get(self.word.clone()) != Some(self.current.as_str()) {
            return false;
        }
        let (before, after) = (text[..self.word.start].chars().next_back(), text[self.word.end..].chars().next());
        if before.is_some_and(char::is_alphabetic) || after.is_some_and(char::is_alphabetic) {
            return false;
        }
        (self.word.start..=self.word.end + after.map_or(0, char::len_utf8)).contains(&cursor)
    }

    /// The next word in: the range to replace and what goes there.
    pub fn advance(&mut self) -> (Range<usize>, String) {
        let replace = self.word.clone();
        let word = self.list[self.next].clone();
        self.word.end = self.word.start + word.len();
        self.current = word.clone();
        self.next = (self.next + 1) % self.list.len();
        (replace, word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(w: &str) -> bool {
        ["the", "cat", "don't", "sat"].contains(&w.to_lowercase().as_str())
    }

    #[test]
    fn finished_words_are_judged_and_the_one_being_typed_is_not() {
        let t = "teh cat sta";
        // The cursor at the end, in "sta": it's being typed (no verdict yet).
        assert_eq!(misspelled(t, t.len(), &[], &valid), vec![0..3]);
        // Moved away: the last word is finished and judged.
        assert_eq!(misspelled(t, 0, &[], &valid), vec![8..11]);
        // Back at its end, the earlier verdict holds.
        let earlier = [Range { start: 8, end: 11 }];
        assert_eq!(misspelled(t, t.len(), &earlier, &valid), vec![0..3, 8..11]);
        assert_eq!(words("don't stop"), vec![0..5, 6..10]);
        assert_eq!(misspelled("don't xyzzy.", 0, &[], &valid), vec![6..11]);
        assert_eq!(word_at("the catt sat", 6), Some(4..8));
    }

    #[test]
    fn ctrl_q_takes_the_word_at_or_before_the_cursor() {
        let t = "say teh, then";
        assert_eq!(word_at(t, 5), Some(4..7)); // in it
        assert_eq!(word_at(t, 7), Some(4..7)); // at its end
        assert_eq!(word_at(t, 8), Some(4..7)); // past the comma
        assert_eq!(word_at(t, 9), Some(9..13)); // at the next one's start
        assert_eq!(word_at(t, 4), Some(4..7)); // at its start
        assert_eq!(word_at(t, t.len()), Some(9..13));
        assert_eq!(word_at("  teh", 1), None); // nothing before the cursor
        assert_eq!(word_at("", 0), None);
    }

    /// Apply one press of `round` to `text`, as the GUI does.
    fn press(round: &mut Round, text: &mut String) -> usize {
        let (range, word) = round.advance();
        text.replace_range(range.clone(), &word);
        range.start + word.len()
    }

    #[test]
    fn a_round_cycles_the_suggestions_then_the_word_as_typed() {
        let mut text = "teh cat".to_string();
        let mut round = Round::new(0..3, "teh".into(), vec!["the".into(), "tea".into(), "ten".into()]);
        let mut seen = Vec::new();
        let mut cursor = 3;
        for _ in 0..5 {
            assert!(round.applies(&text, cursor));
            cursor = press(&mut round, &mut text);
            seen.push(text.clone());
        }
        assert_eq!(seen, ["the cat", "tea cat", "ten cat", "teh cat", "the cat"]);
        // A longer suggestion moves the word's end with it.
        let mut text = "I ned it".to_string();
        let mut round = Round::new(2..5, "ned".into(), vec!["need".into()]);
        assert!(round.applies(&text, 6)); // after the space that follows it
        let cursor = press(&mut round, &mut text);
        assert_eq!((text.as_str(), cursor), ("I need it", 6));
        assert!(round.applies(&text, cursor));
        press(&mut round, &mut text);
        assert_eq!(text, "I ned it");
    }

    #[test]
    fn a_round_ends_when_the_word_is_edited_or_left() {
        let round = Round::new(0..3, "teh".into(), vec!["the".into()]);
        assert!(round.applies("teh cat", 3));
        assert!(round.applies("teh cat", 4)); // one non-letter past it
        assert!(!round.applies("teh cat", 5)); // in the next word
        assert!(!round.applies("tehx cat", 3)); // a letter run onto it
        assert!(!round.applies("xteh cat", 1));
        assert!(!round.applies("ten cat", 3)); // the word itself changed
        assert!(!round.applies("te", 2)); // gone
        let at_end = Round::new(0..3, "teh".into(), vec!["the".into()]);
        assert!(at_end.applies("teh", 3));
    }
}
