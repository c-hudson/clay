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

/// The word at `at`, for suggestions.
pub fn word_at(text: &str, at: usize) -> Option<Range<usize>> {
    words(text).into_iter().find(|w| w.start <= at && at <= w.end)
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
        assert_eq!(misspelled(t, t.len(), &[8..11], &valid), vec![0..3, 8..11]);
        assert_eq!(words("don't stop"), vec![0..5, 6..10]);
        assert_eq!(misspelled("don't xyzzy.", 0, &[], &valid), vec![6..11]);
        assert_eq!(word_at("the catt sat", 6), Some(4..8));
    }
}
