use unicode_width::UnicodeWidthChar;

/// Calculate display width of a string (handles zero-width characters and wide chars)
pub fn display_width(s: &str) -> usize {
    s.chars().map(|c| UnicodeWidthChar::width(c).unwrap_or(0)).sum()
}

/// Calculate display width of a char slice
pub fn display_width_chars(chars: &[char]) -> usize {
    chars.iter().map(|c| UnicodeWidthChar::width(*c).unwrap_or(0)).sum()
}

/// Find the character index where display width reaches or exceeds the target width.
/// Returns (char_index, actual_display_width_up_to_that_point)
pub fn chars_for_display_width(chars: &[char], target_width: usize) -> (usize, usize) {
    let mut width = 0;
    for (i, c) in chars.iter().enumerate() {
        let char_width = UnicodeWidthChar::width(*c).unwrap_or(0);
        if width + char_width > target_width {
            return (i, width);
        }
        width += char_width;
    }
    (chars.len(), width)
}

pub struct InputArea {
    pub buffer: String,
    pub cursor_position: usize,
    pub viewport_start_line: usize,
    pub visible_height: u16,
    pub width: u16,
    pub prompt_len: usize, // Length of prompt (reduces first line capacity)
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub temp_input: String,
    pub search_prefix: Option<String>,  // Prefix being searched (set on first ^[p/^[n)
    pub search_index: Option<usize>,    // Position in history during search
    pub kill_ring: Vec<String>,         // Killed text history (for ^Y yank)
}

impl InputArea {
    pub fn new(visible_height: u16) -> Self {
        Self {
            buffer: String::new(),
            cursor_position: 0,
            viewport_start_line: 0,
            visible_height,
            width: 80,
            prompt_len: 0,
            history: Vec::new(),
            history_index: None,
            temp_input: String::new(),
            search_prefix: None,
            search_index: None,
            kill_ring: Vec::new(),
        }
    }

    pub fn set_dimensions(&mut self, width: u16, height: u16) {
        self.width = width.max(1);
        self.visible_height = height;
        self.adjust_viewport();
    }

    pub fn cursor_line(&self) -> usize {
        if self.width == 0 {
            return 0;
        }
        let width = self.width as usize;
        let first_line_capacity = width.saturating_sub(self.prompt_len);
        let text_before_cursor = &self.buffer[..self.cursor_position];

        // Walk through lines, splitting at newlines and width-based wraps
        let mut line = 0;
        let mut col_width = 0; // display width consumed on current line
        let mut is_first_line = true;

        for c in text_before_cursor.chars() {
            if c == '\n' {
                line += 1;
                col_width = 0;
                is_first_line = false;
                continue;
            }
            let cw = display_width(&c.to_string());
            let capacity = if is_first_line { first_line_capacity } else { width };
            col_width += cw;
            if capacity > 0 && col_width >= capacity {
                // Line is full — if exactly full, cursor wraps to next line
                // (col_width resets so next char starts fresh)
                if col_width == capacity {
                    line += 1;
                    col_width = 0;
                    is_first_line = false;
                } else {
                    // Overflows — char itself is on next line
                    line += 1;
                    col_width = cw;
                    is_first_line = false;
                }
            }
        }
        line
    }

    pub fn adjust_viewport(&mut self) {
        let cursor_line = self.cursor_line();
        let visible = self.visible_height as usize;

        if cursor_line < self.viewport_start_line {
            self.viewport_start_line = cursor_line;
        } else if cursor_line >= self.viewport_start_line + visible {
            self.viewport_start_line = cursor_line.saturating_sub(visible - 1);
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            // Find the previous character boundary
            let mut new_pos = self.cursor_position - 1;
            while new_pos > 0 && !self.buffer.is_char_boundary(new_pos) {
                new_pos -= 1;
            }
            self.cursor_position = new_pos;
            self.adjust_viewport();
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.buffer.len() {
            // Find the next character boundary
            let mut new_pos = self.cursor_position + 1;
            while new_pos < self.buffer.len() && !self.buffer.is_char_boundary(new_pos) {
                new_pos += 1;
            }
            self.cursor_position = new_pos;
            self.adjust_viewport();
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor_position, c);
        self.cursor_position += c.len_utf8();
        self.adjust_viewport();
        self.history_index = None;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            // Find the previous character boundary
            let mut new_pos = self.cursor_position - 1;
            while new_pos > 0 && !self.buffer.is_char_boundary(new_pos) {
                new_pos -= 1;
            }
            self.buffer.remove(new_pos);
            self.cursor_position = new_pos;
            self.adjust_viewport();
        }
    }

    pub fn delete_char_forward(&mut self) {
        if self.cursor_position < self.buffer.len() {
            self.buffer.remove(self.cursor_position);
        }
    }

    pub fn delete_word_before_cursor(&mut self) {
        if self.cursor_position == 0 {
            return;
        }

        let old_pos = self.cursor_position;
        // Work with characters, not bytes
        let before_cursor: String = self.buffer[..self.cursor_position].to_string();
        let mut chars: Vec<char> = before_cursor.chars().collect();

        // Skip whitespace immediately before cursor
        while !chars.is_empty() && chars.last().is_some_and(|c| c.is_whitespace()) {
            chars.pop();
        }

        // Delete word characters until we hit whitespace or start
        while !chars.is_empty() && chars.last().is_some_and(|c| !c.is_whitespace()) {
            chars.pop();
        }

        // Reconstruct the string
        let new_before: String = chars.into_iter().collect();
        let after_cursor = &self.buffer[self.cursor_position..];
        self.cursor_position = new_before.len();

        // Push killed text to kill ring
        let killed = self.buffer[self.cursor_position..old_pos].to_string();
        if !killed.is_empty() {
            self.kill_ring.push(killed);
        }

        self.buffer = new_before + after_cursor;
        self.adjust_viewport();
    }

    pub fn clear(&mut self) {
        // Push to kill ring if there's content
        if !self.buffer.is_empty() {
            self.kill_ring.push(self.buffer.clone());
        }
        self.buffer.clear();
        self.cursor_position = 0;
        self.viewport_start_line = 0;
        self.history_index = None;
    }

    pub fn take_input(&mut self) -> String {
        let input = self.buffer.clone();
        if !input.is_empty() {
            self.history.push(input.clone());
        }
        self.clear();
        input
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }

        match self.history_index {
            None => {
                self.temp_input = self.buffer.clone();
                self.history_index = Some(self.history.len() - 1);
            }
            Some(idx) if idx > 0 => {
                self.history_index = Some(idx - 1);
            }
            _ => return,
        }

        if let Some(idx) = self.history_index {
            self.buffer = self.history[idx].clone();
            self.cursor_position = self.buffer.len();
            self.adjust_viewport();
        }
    }

    pub fn history_next(&mut self) {
        match self.history_index {
            Some(idx) if idx < self.history.len() - 1 => {
                self.history_index = Some(idx + 1);
                self.buffer = self.history[idx + 1].clone();
            }
            Some(_) => {
                self.history_index = None;
                self.buffer = self.temp_input.clone();
            }
            None => return,
        }
        self.cursor_position = self.buffer.len();
        self.adjust_viewport();
    }

    pub fn home(&mut self) {
        self.cursor_position = 0;
        self.adjust_viewport();
    }

    pub fn end(&mut self) {
        self.cursor_position = self.buffer.len();
        self.adjust_viewport();
    }

    /// Delete from cursor position to end of line (Ctrl+K)
    pub fn kill_to_end(&mut self) {
        let killed = self.buffer[self.cursor_position..].to_string();
        if !killed.is_empty() {
            self.kill_ring.push(killed);
        }
        self.buffer.truncate(self.cursor_position);
    }

    /// Delete forward to end of next word (Esc+D).
    /// Deletes non-word chars, then word chars, stopping before the first
    /// alphanumeric character of the next word.
    pub fn delete_word_forward(&mut self) {
        if self.cursor_position >= self.buffer.len() {
            return;
        }
        let after: Vec<char> = self.buffer[self.cursor_position..].chars().collect();
        let mut i = 0;
        // Skip non-word characters first
        while i < after.len() && !after[i].is_alphanumeric() {
            i += 1;
        }
        // Then skip word characters
        while i < after.len() && after[i].is_alphanumeric() {
            i += 1;
        }
        // Calculate byte offset to delete
        let byte_offset: usize = after[..i].iter().map(|c| c.len_utf8()).sum();
        let end = self.cursor_position + byte_offset;
        // Push killed text to kill ring
        let killed = self.buffer[self.cursor_position..end].to_string();
        if !killed.is_empty() {
            self.kill_ring.push(killed);
        }
        self.buffer.replace_range(self.cursor_position..end, "");
    }

    /// Move cursor to end of next word, converting characters to lowercase (Esc+L).
    /// Word characters are A-Z, a-z, 0-9. Skips trailing spaces.
    pub fn lowercase_word(&mut self) {
        self.transform_word(|_, c| c.to_lowercase().next().unwrap_or(c))
    }

    /// Move cursor to end of next word, converting characters to uppercase (Esc+U).
    /// Word characters are A-Z, a-z, 0-9. Skips trailing spaces.
    pub fn uppercase_word(&mut self) {
        self.transform_word(|_, c| c.to_uppercase().next().unwrap_or(c))
    }

    /// Move cursor to end of next word, capitalizing first letter of each word (Esc+C).
    /// Word characters are A-Z, a-z, 0-9. Skips trailing spaces.
    pub fn capitalize_word(&mut self) {
        self.transform_word(|is_start, c| {
            if is_start {
                c.to_uppercase().next().unwrap_or(c)
            } else {
                c.to_lowercase().next().unwrap_or(c)
            }
        })
    }

    /// Helper: transform characters from cursor to end of next word, then skip trailing spaces.
    /// The closure receives (is_word_start, char) and returns the replacement char.
    fn transform_word<F>(&mut self, transform: F)
    where
        F: Fn(bool, char) -> char,
    {
        if self.cursor_position >= self.buffer.len() {
            return;
        }
        let before = self.buffer[..self.cursor_position].to_string();
        let after: Vec<char> = self.buffer[self.cursor_position..].chars().collect();
        let mut i = 0;
        let mut result = String::new();
        let mut at_word_start = true;

        // Skip leading non-word characters (pass through unchanged)
        while i < after.len() && !after[i].is_alphanumeric() {
            result.push(after[i]);
            i += 1;
        }
        // Transform word characters
        while i < after.len() && after[i].is_alphanumeric() {
            result.push(transform(at_word_start, after[i]));
            at_word_start = false;
            i += 1;
        }
        // Skip trailing spaces (pass through unchanged, but cursor moves past them)
        while i < after.len() && after[i] == ' ' {
            result.push(after[i]);
            i += 1;
        }

        let rest: String = after[i..].iter().collect();
        self.cursor_position = before.len() + result.len();
        self.buffer = before + &result + &rest;
        self.adjust_viewport();
    }

    /// Get the column position within the current line (0-indexed, in display width)
    fn cursor_column(&self) -> usize {
        if self.width == 0 {
            return 0;
        }
        let width = self.width as usize;
        let first_line_capacity = width.saturating_sub(self.prompt_len);
        let text_before_cursor = &self.buffer[..self.cursor_position];

        let mut col_width = 0usize;
        let mut is_first_line = true;

        for c in text_before_cursor.chars() {
            if c == '\n' {
                col_width = 0;
                is_first_line = false;
                continue;
            }
            let cw = UnicodeWidthChar::width(c).unwrap_or(0);
            let capacity = if is_first_line { first_line_capacity } else { width };
            col_width += cw;
            if capacity > 0 && col_width >= capacity {
                if col_width == capacity {
                    col_width = 0;
                } else {
                    col_width = cw;
                }
                is_first_line = false;
            }
        }
        col_width
    }

    /// Build a map of (line_index -> byte_offset_of_line_start) for the buffer,
    /// accounting for both newlines and width-based wrapping.
    fn line_starts(&self) -> Vec<usize> {
        let width = self.width as usize;
        let first_line_capacity = width.saturating_sub(self.prompt_len);
        let mut starts = vec![0usize]; // line 0 starts at byte 0
        let mut col_width = 0usize;
        let mut is_first_line = true;
        let mut byte_pos = 0;

        for c in self.buffer.chars() {
            if c == '\n' {
                byte_pos += c.len_utf8();
                starts.push(byte_pos);
                col_width = 0;
                is_first_line = false;
                continue;
            }
            let cw = UnicodeWidthChar::width(c).unwrap_or(0);
            let capacity = if is_first_line { first_line_capacity } else { width };
            col_width += cw;
            if capacity > 0 && col_width >= capacity {
                byte_pos += c.len_utf8();
                if col_width == capacity {
                    // Exact fill — next char starts a new line
                    starts.push(byte_pos);
                    col_width = 0;
                } else {
                    // Overflow — this char starts a new line
                    starts.push(byte_pos - c.len_utf8());
                    col_width = cw;
                }
                is_first_line = false;
                continue;
            }
            byte_pos += c.len_utf8();
        }
        starts
    }

    /// Move cursor up one line, maintaining column position if possible
    /// Move cursor up one line. Returns true if already at the top line
    /// (caller should trigger history_prev).
    pub fn move_cursor_up(&mut self) -> bool {
        let current_line = self.cursor_line();
        if current_line == 0 {
            return true; // At top — caller should navigate history
        }

        let current_col = self.cursor_column();
        let starts = self.line_starts();
        let target_line = current_line - 1;

        // Target column: maintain screen column, adjusted for prompt on first line
        let target_col = if target_line == 0 {
            // Moving to first line — current_col might include prompt offset visually,
            // but cursor_column returns text column without prompt
            current_col.min(self.prompt_len + current_col).saturating_sub(self.prompt_len)
        } else {
            current_col
        };

        if target_line < starts.len() {
            let line_start = starts[target_line];
            // Walk from line_start to find the byte position at target_col display width
            let mut col = 0;
            let mut pos = line_start;
            for c in self.buffer[line_start..].chars() {
                if c == '\n' { break; }
                let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                if col + cw > target_col { break; }
                col += cw;
                pos += c.len_utf8();
                // Stop at line boundary (next line start)
                if target_line + 1 < starts.len() && pos >= starts[target_line + 1] {
                    pos = starts[target_line + 1];
                    break;
                }
            }
            self.cursor_position = pos.min(self.buffer.len());
        }
        self.adjust_viewport();
        false
    }

    /// Move cursor down one line. Returns true if already at the bottom line
    /// (caller should trigger history_next).
    pub fn move_cursor_down(&mut self) -> bool {
        let current_line = self.cursor_line();
        let current_col = self.cursor_column();
        let starts = self.line_starts();
        let total_lines = starts.len();

        if current_line >= total_lines.saturating_sub(1) {
            return true; // At bottom — caller should navigate history
        }

        let target_line = current_line + 1;
        // Maintain column position
        let target_col = if current_line == 0 {
            // Moving from first line — screen column includes prompt,
            // but on subsequent lines there's no prompt offset
            let screen_col = self.prompt_len + current_col;
            screen_col.min(self.width as usize - 1)
        } else {
            current_col
        };

        if target_line < starts.len() {
            let line_start = starts[target_line];
            let mut col = 0;
            let mut pos = line_start;
            for c in self.buffer[line_start..].chars() {
                if c == '\n' { break; }
                let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                if col + cw > target_col { break; }
                col += cw;
                pos += c.len_utf8();
                // Stop at line boundary
                if target_line + 1 < starts.len() && pos >= starts[target_line + 1] {
                    pos = starts[target_line + 1];
                    break;
                }
            }
            self.cursor_position = pos.min(self.buffer.len());
        }
        self.adjust_viewport();
        false
    }

    /// Check if a character at the given position should be part of a word.
    /// Includes alphabetic characters and apostrophes between alphabetic characters.
    fn is_word_char(chars: &[char], pos: usize) -> bool {
        if pos >= chars.len() {
            return false;
        }
        let c = chars[pos];
        if c.is_alphabetic() {
            return true;
        }
        // Include apostrophe if it's between alphabetic characters (contractions like "didn't")
        if c == '\'' {
            let has_alpha_before = pos > 0 && chars[pos - 1].is_alphabetic();
            let has_alpha_after = pos + 1 < chars.len() && chars[pos + 1].is_alphabetic();
            return has_alpha_before && has_alpha_after;
        }
        false
    }

    pub fn current_word(&self) -> Option<(usize, usize, String)> {
        if self.buffer.is_empty() {
            return None;
        }

        let chars: Vec<char> = self.buffer.chars().collect();
        // Convert byte position to character position
        let char_pos = self.buffer[..self.cursor_position].chars().count();
        let pos = char_pos.min(chars.len());

        let mut start = pos;
        while start > 0 && Self::is_word_char(&chars, start - 1) {
            start -= 1;
        }

        let mut end = pos;
        while end < chars.len() && Self::is_word_char(&chars, end) {
            end += 1;
        }

        if start == end {
            if start > 0 && !chars[start - 1].is_alphabetic() {
                let mut prev_end = start - 1;
                while prev_end > 0 && !chars[prev_end - 1].is_alphabetic() {
                    prev_end -= 1;
                }
                if prev_end == 0 && !chars[0].is_alphabetic() {
                    return None;
                }
                end = prev_end;
                start = prev_end;
                while start > 0 && Self::is_word_char(&chars, start - 1) {
                    start -= 1;
                }
            } else {
                return None;
            }
        }

        let word: String = chars[start..end].iter().collect();
        Some((start, end, word))
    }

    /// Transpose the two characters before the cursor (Ctrl+T).
    /// If at end, swap last two chars. Otherwise swap char before cursor with char at cursor, advance cursor.
    pub fn transpose_chars(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        if chars.len() < 2 {
            return;
        }
        let char_pos = self.buffer[..self.cursor_position].chars().count();
        if char_pos == 0 {
            return;
        }
        let (a, b) = if char_pos >= chars.len() {
            // At end: swap last two chars
            (chars.len() - 2, chars.len() - 1)
        } else {
            // Swap char before cursor with char at cursor, advance cursor
            (char_pos - 1, char_pos)
        };
        let mut new_chars = chars;
        new_chars.swap(a, b);
        self.buffer = new_chars.into_iter().collect();
        // Position cursor after the swapped pair
        let new_char_pos = b + 1;
        self.cursor_position = self.buffer.chars().take(new_char_pos).map(|c| c.len_utf8()).sum();
        self.adjust_viewport();
    }

    /// Collapse multiple spaces around cursor to a single space (Esc+Space).
    pub fn collapse_spaces(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let char_pos = self.buffer[..self.cursor_position].chars().count();
        if chars.is_empty() {
            return;
        }
        // Find extent of space run around cursor
        let mut start = char_pos;
        while start > 0 && chars[start - 1] == ' ' {
            start -= 1;
        }
        let mut end = char_pos;
        while end < chars.len() && chars[end] == ' ' {
            end += 1;
        }
        if end - start <= 1 {
            return; // 0 or 1 space, nothing to collapse
        }
        // Replace run with single space
        let before: String = chars[..start].iter().collect();
        let after: String = chars[end..].iter().collect();
        self.buffer = format!("{} {}", before, after);
        self.cursor_position = before.len() + 1; // After the single space
        self.adjust_viewport();
    }

    /// Insert last word from previous history entry (Esc+. / Esc+_).
    pub fn last_argument(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let prev = &self.history[self.history.len() - 1];
        if let Some(word) = prev.split_whitespace().last() {
            let word = word.to_string();
            self.buffer.insert_str(self.cursor_position, &word);
            self.cursor_position += word.len();
            self.adjust_viewport();
        }
    }

    /// Move cursor to matching bracket (Esc+-).
    pub fn goto_matching_bracket(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let char_pos = self.buffer[..self.cursor_position].chars().count();
        if char_pos >= chars.len() {
            return;
        }
        let ch = chars[char_pos];
        let (open, close, forward) = match ch {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => return,
        };
        let mut depth = 0i32;
        if forward {
            for i in char_pos..chars.len() {
                if chars[i] == open { depth += 1; }
                if chars[i] == close { depth -= 1; }
                if depth == 0 {
                    self.cursor_position = chars[..i].iter().map(|c| c.len_utf8()).sum();
                    self.adjust_viewport();
                    return;
                }
            }
        } else {
            for i in (0..=char_pos).rev() {
                if chars[i] == close { depth += 1; }
                if chars[i] == open { depth -= 1; }
                if depth == 0 {
                    self.cursor_position = chars[..i].iter().map(|c| c.len_utf8()).sum();
                    self.adjust_viewport();
                    return;
                }
            }
        }
    }

    /// Delete word backward stopping at punctuation boundaries (Esc+Backspace).
    /// Like delete_word_before_cursor but treats non-alphanumeric as word boundaries.
    pub fn backward_kill_word_punctuation(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        let old_pos = self.cursor_position;
        let before_cursor: String = self.buffer[..self.cursor_position].to_string();
        let mut chars: Vec<char> = before_cursor.chars().collect();

        // Skip whitespace immediately before cursor
        while !chars.is_empty() && chars.last().is_some_and(|c| c.is_whitespace()) {
            chars.pop();
        }

        // Delete until we hit whitespace or a different character class
        // If starting on punctuation, delete punctuation. If on alnum, delete alnum.
        if let Some(&last) = chars.last() {
            if last.is_alphanumeric() {
                while !chars.is_empty() && chars.last().is_some_and(|c| c.is_alphanumeric()) {
                    chars.pop();
                }
            } else {
                // Punctuation: delete one run of non-alnum non-whitespace
                while !chars.is_empty() && chars.last().is_some_and(|c| !c.is_alphanumeric() && !c.is_whitespace()) {
                    chars.pop();
                }
            }
        }

        let new_before: String = chars.into_iter().collect();
        let after_cursor = &self.buffer[self.cursor_position..];
        self.cursor_position = new_before.len();

        // Push killed text to kill ring
        let killed = self.buffer[self.cursor_position..old_pos].to_string();
        if !killed.is_empty() {
            self.kill_ring.push(killed);
        }

        self.buffer = new_before + after_cursor;
        self.adjust_viewport();
    }

    /// Yank (paste) the most recent entry from the kill ring (Ctrl+Y).
    pub fn yank(&mut self) {
        if let Some(text) = self.kill_ring.last().cloned() {
            self.buffer.insert_str(self.cursor_position, &text);
            self.cursor_position += text.len();
            self.adjust_viewport();
        }
    }

    /// Move cursor backward one word (Esc+b / Ctrl+Left).
    pub fn word_left(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        let before: Vec<char> = self.buffer[..self.cursor_position].chars().collect();
        let mut i = before.len();
        // Skip whitespace/non-word chars
        while i > 0 && !before[i - 1].is_alphanumeric() {
            i -= 1;
        }
        // Skip word chars
        while i > 0 && before[i - 1].is_alphanumeric() {
            i -= 1;
        }
        self.cursor_position = before[..i].iter().map(|c| c.len_utf8()).sum();
        self.adjust_viewport();
    }

    /// Move cursor forward one word (Esc+f / Ctrl+Right).
    pub fn word_right(&mut self) {
        if self.cursor_position >= self.buffer.len() {
            return;
        }
        let after: Vec<char> = self.buffer[self.cursor_position..].chars().collect();
        let mut i = 0;
        // Skip non-word chars
        while i < after.len() && !after[i].is_alphanumeric() {
            i += 1;
        }
        // Skip word chars
        while i < after.len() && after[i].is_alphanumeric() {
            i += 1;
        }
        let byte_offset: usize = after[..i].iter().map(|c| c.len_utf8()).sum();
        self.cursor_position += byte_offset;
        self.adjust_viewport();
    }

    /// Search history backward for entries starting with current prefix (Esc+p).
    pub fn history_search_backward(&mut self) {
        if self.history.is_empty() {
            return;
        }
        // On first call, save current input as search prefix
        if self.search_prefix.is_none() {
            self.search_prefix = Some(self.buffer.clone());
            self.search_index = Some(self.history.len()); // Start past end
        }
        let prefix = self.search_prefix.clone().unwrap_or_default();
        let start = self.search_index.unwrap_or(self.history.len());
        // Scan backward from start-1
        if start == 0 {
            return; // Already at beginning
        }
        for i in (0..start).rev() {
            if self.history[i].starts_with(&prefix) {
                self.search_index = Some(i);
                self.buffer = self.history[i].clone();
                self.cursor_position = self.buffer.len();
                self.adjust_viewport();
                return;
            }
        }
    }

    /// Search history forward for entries starting with current prefix (Esc+n).
    pub fn history_search_forward(&mut self) {
        if self.search_prefix.is_none() {
            return; // No active search
        }
        let prefix = self.search_prefix.clone().unwrap_or_default();
        let start = self.search_index.unwrap_or(0);
        // Scan forward from start+1
        for i in (start + 1)..self.history.len() {
            if self.history[i].starts_with(&prefix) {
                self.search_index = Some(i);
                self.buffer = self.history[i].clone();
                self.cursor_position = self.buffer.len();
                self.adjust_viewport();
                return;
            }
        }
        // Past end: restore original input
        self.buffer = prefix;
        self.cursor_position = self.buffer.len();
        self.search_index = Some(self.history.len());
        self.adjust_viewport();
    }

    pub fn replace_word(&mut self, start: usize, end: usize, new_word: &str) {
        // start and end are character indices
        let before: String = self.buffer.chars().take(start).collect();
        let after: String = self.buffer.chars().skip(end).collect();
        self.buffer = format!("{}{}{}", before, new_word, after);
        // cursor_position needs to be a byte index
        self.cursor_position = before.len() + new_word.len();
        self.adjust_viewport();
    }
}

impl Default for InputArea {
    fn default() -> Self {
        Self::new(3)
    }
}
