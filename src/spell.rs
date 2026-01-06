use std::collections::HashSet;
use strsim::levenshtein;

// System dictionary paths to try (in order of preference)
const SYSTEM_DICT_PATHS: &[&str] = &[
    "/usr/share/dict/words",
    "/usr/share/dict/american-english",
    "/usr/share/dict/british-english",
];

pub struct SpellChecker {
    words: HashSet<String>,
}

impl SpellChecker {
    pub fn new() -> Self {
        // Try to load system dictionary
        let mut words = HashSet::new();
        for path in SYSTEM_DICT_PATHS {
            if let Ok(content) = std::fs::read_to_string(path) {
                words = content
                    .lines()
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_alphabetic()))
                    .collect();
                break;
            }
        }
        Self { words }
    }

    pub fn is_valid(&self, word: &str) -> bool {
        if word.is_empty() {
            return true;
        }
        if word.starts_with('/') || word.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
        self.words.contains(&word.to_lowercase())
    }

    pub fn suggestions(&self, word: &str, count: usize) -> Vec<String> {
        let word_lower = word.to_lowercase();
        let mut candidates: Vec<(String, usize)> = self
            .words
            .iter()
            .filter(|w| {
                let len_diff = (w.len() as i32 - word_lower.len() as i32).abs();
                len_diff <= 3
            })
            .map(|w| {
                let dist = levenshtein(&word_lower, w);
                (w.clone(), dist)
            })
            .filter(|(_, dist)| *dist <= 3)
            .collect();

        candidates.sort_by_key(|(_, dist)| *dist);
        candidates.truncate(count);
        candidates.into_iter().map(|(w, _)| w).collect()
    }
}

impl Default for SpellChecker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SpellState {
    pub suggestions: Vec<String>,  // Includes original word at the end for cycling
    pub suggestion_index: usize,
    pub word_start: usize,
    pub word_end: usize,
    pub original_word: String,
    pub showing_suggestions: bool,
}

impl SpellState {
    pub fn new() -> Self {
        Self {
            suggestions: Vec::new(),
            suggestion_index: 0,
            word_start: 0,
            word_end: 0,
            original_word: String::new(),
            showing_suggestions: false,
        }
    }

    pub fn reset(&mut self) {
        self.suggestions.clear();
        self.suggestion_index = 0;
        self.original_word.clear();
        self.showing_suggestions = false;
    }
}

impl Default for SpellState {
    fn default() -> Self {
        Self::new()
    }
}
