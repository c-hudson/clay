use std::collections::HashSet;
use std::process::Command;
use strsim::levenshtein;

// System dictionary paths to try (in order of preference)
const SYSTEM_DICT_PATHS: &[&str] = &[
    "/usr/share/dict/words",
    "/usr/share/dict/american-english",
    "/usr/share/dict/british-english",
    // Termux (Android) paths
    "/data/data/com.termux/files/usr/share/dict/words",
    "/data/data/com.termux/files/usr/share/dict/american-english",
    "/data/data/com.termux/files/usr/share/dict/british-english",
];

// Parse a dictionary line, handling Hunspell .dic format (word/FLAGS)
// Returns the word portion lowercased, or None if invalid
fn parse_dict_word(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Skip Hunspell .dic header (first line is a number = word count)
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // Strip Hunspell affix flags after '/' (e.g., "word/MS" -> "word")
    let word = if let Some(slash_pos) = trimmed.find('/') {
        &trimmed[..slash_pos]
    } else {
        trimmed
    };
    let lower = word.to_lowercase();
    if !lower.is_empty() && lower.chars().all(|c| c.is_alphabetic()) {
        Some(lower)
    } else {
        None
    }
}

// Find LibreOffice Hunspell dictionaries on Windows
#[cfg(target_os = "windows")]
fn find_hunspell_dict_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    // Program Files locations
    for base in &[
        r"C:\Program Files\LibreOffice\share\extensions",
        r"C:\Program Files (x86)\LibreOffice\share\extensions",
    ] {
        let base_path = std::path::Path::new(base);
        if base_path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(base_path) {
                for entry in entries.flatten() {
                    let dir = entry.path();
                    if dir.is_dir() {
                        // Look for en_US.dic or en_GB.dic in subdirectories
                        for name in &["en_US.dic", "en_GB.dic"] {
                            let dic = dir.join(name);
                            if dic.exists() {
                                paths.push(dic);
                            }
                        }
                    }
                }
            }
        }
    }
    // Also check next to the executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let words = exe_dir.join("words");
            if words.exists() {
                paths.push(words);
            }
            let dic = exe_dir.join("en_US.dic");
            if dic.exists() {
                paths.push(dic);
            }
        }
    }
    paths
}

#[cfg(not(target_os = "windows"))]
fn find_hunspell_dict_paths() -> Vec<std::path::PathBuf> {
    Vec::new()
}

// Common contraction suffixes (after apostrophe)
// If a word ends with 'suffix, check if the base word is valid
// Note: "n't" must come before "'t" so we match the longer suffix first
const CONTRACTION_SUFFIXES: &[&str] = &[
    "n't",   // didn't, wouldn't, couldn't, shouldn't, don't, won't, isn't, aren't, wasn't, weren't, hasn't, haven't, hadn't
    "'t",    // can't, shan't (base already ends in 'n')
    "'s",    // he's, she's, it's, that's, what's, who's, there's, here's, let's
    "'re",   // you're, we're, they're
    "'ve",   // I've, you've, we've, they've, could've, would've, should've
    "'ll",   // I'll, you'll, he'll, she'll, it'll, we'll, they'll
    "'d",    // I'd, you'd, he'd, she'd, we'd, they'd
    "'m",    // I'm
];

pub struct SpellChecker {
    words: HashSet<String>,
}

impl SpellChecker {
    pub fn new(custom_path: &str) -> Self {
        let mut words = HashSet::new();

        // Try custom dictionary path first
        if !custom_path.is_empty() {
            if let Ok(content) = std::fs::read_to_string(custom_path) {
                words = content.lines().filter_map(parse_dict_word).collect();
            }
        }

        // Try system dictionary paths
        if words.is_empty() {
            for path in SYSTEM_DICT_PATHS {
                if let Ok(content) = std::fs::read_to_string(path) {
                    words = content.lines().filter_map(parse_dict_word).collect();
                    break;
                }
            }
        }

        // Try Hunspell dictionaries (LibreOffice on Windows)
        if words.is_empty() {
            for path in find_hunspell_dict_paths() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    words = content.lines().filter_map(parse_dict_word).collect();
                    if !words.is_empty() {
                        break;
                    }
                }
            }
        }

        // If no dictionary file found, try aspell (works with aspell-en on Termux)
        if words.is_empty() {
            if let Ok(output) = Command::new("aspell")
                .args(["dump", "master"])
                .output()
            {
                if output.status.success() {
                    if let Ok(content) = String::from_utf8(output.stdout) {
                        words = content.lines().filter_map(parse_dict_word).collect();
                    }
                }
            }
        }

        Self { words }
    }

    pub fn has_dictionary(&self) -> bool {
        !self.words.is_empty()
    }

    pub fn is_valid(&self, word: &str) -> bool {
        // If no dictionary loaded, consider all words valid (don't mark anything as misspelled)
        if self.words.is_empty() {
            return true;
        }
        if word.is_empty() {
            return true;
        }
        if word.starts_with('/') || word.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
        let word_lower = word.to_lowercase();

        // Check if word is in dictionary
        if self.words.contains(&word_lower) {
            return true;
        }

        // Check for contractions: if word contains apostrophe, check base word
        if word_lower.contains('\'') {
            // Special case for irregular contractions
            if word_lower == "won't" {
                return self.words.contains("will");
            }

            for suffix in CONTRACTION_SUFFIXES {
                if word_lower.ends_with(suffix) {
                    let base = &word_lower[..word_lower.len() - suffix.len()];
                    if !base.is_empty() && self.words.contains(base) {
                        return true;
                    }
                }
            }
        }

        false
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
        Self::new("")
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
