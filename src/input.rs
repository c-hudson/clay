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
        // Use display width instead of character count (handles zero-width chars)
        let width_before = display_width(&self.buffer[..self.cursor_position]);
        let width = self.width as usize;

        // Account for prompt taking up space on the first line
        // First line capacity is reduced by prompt_len
        let first_line_capacity = width.saturating_sub(self.prompt_len);

        if first_line_capacity == 0 {
            // Prompt fills entire first line, so all input is on subsequent lines
            // Add 1 for the prompt line, then calculate remaining lines
            1 + width_before / width
        } else if width_before < first_line_capacity {
            // Still on first line
            0
        } else {
            // Past first line: subtract first line display width, then divide by width
            let remaining_width = width_before - first_line_capacity;
            1 + remaining_width / width
        }
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
        self.buffer = new_before + after_cursor;
        self.adjust_viewport();
    }

    pub fn clear(&mut self) {
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

    /// Get the column position within the current line (0-indexed, in display width)
    fn cursor_column(&self) -> usize {
        if self.width == 0 {
            return 0;
        }
        let width_before = display_width(&self.buffer[..self.cursor_position]);
        let width = self.width as usize;
        let first_line_capacity = width.saturating_sub(self.prompt_len);

        if first_line_capacity == 0 {
            // Prompt fills entire first line
            width_before % width
        } else if width_before < first_line_capacity {
            // Still on first line
            width_before
        } else {
            // Past first line
            let remaining_width = width_before - first_line_capacity;
            remaining_width % width
        }
    }

    /// Move cursor up one line, maintaining column position if possible
    pub fn move_cursor_up(&mut self) {
        let current_line = self.cursor_line();
        if current_line == 0 {
            // Already on first line, move to start
            self.cursor_position = 0;
            self.adjust_viewport();
            return;
        }

        let current_col = self.cursor_column();
        let width = self.width as usize;
        let first_line_capacity = width.saturating_sub(self.prompt_len);

        // Calculate target character position
        let target_char_pos = if current_line == 1 {
            // Moving to first line (which has prompt)
            current_col.min(first_line_capacity.saturating_sub(1))
        } else {
            // Moving to a non-first line
            let chars_before_target_line = if current_line == 1 {
                0
            } else {
                first_line_capacity + (current_line - 2) * width
            };
            chars_before_target_line + current_col.min(width - 1)
        };

        // Convert character position to byte position
        let mut byte_pos = 0;
        for (i, c) in self.buffer.chars().enumerate() {
            if i >= target_char_pos {
                break;
            }
            byte_pos += c.len_utf8();
        }
        self.cursor_position = byte_pos.min(self.buffer.len());
        self.adjust_viewport();
    }

    /// Move cursor down one line, maintaining column position if possible
    pub fn move_cursor_down(&mut self) {
        let current_line = self.cursor_line();
        let current_col = self.cursor_column();
        let width = self.width as usize;
        let first_line_capacity = width.saturating_sub(self.prompt_len);
        let total_chars = self.buffer.chars().count();

        // Calculate total lines
        let total_lines = if first_line_capacity == 0 {
            1 + (total_chars + width - 1) / width
        } else if total_chars <= first_line_capacity {
            1
        } else {
            1 + (total_chars - first_line_capacity + width - 1) / width
        };

        if current_line >= total_lines.saturating_sub(1) {
            // Already on last line, move to end
            self.cursor_position = self.buffer.len();
            self.adjust_viewport();
            return;
        }

        // Calculate target character position on next line
        let target_char_pos = if current_line == 0 {
            // Moving from first line to second line
            first_line_capacity + current_col.min(width - 1)
        } else {
            // Moving from non-first line to next line
            let chars_before_next_line = first_line_capacity + current_line * width;
            chars_before_next_line + current_col.min(width - 1)
        };

        // Convert character position to byte position, clamping to buffer length
        let mut byte_pos = 0;
        for (i, c) in self.buffer.chars().enumerate() {
            if i >= target_char_pos {
                break;
            }
            byte_pos += c.len_utf8();
        }
        self.cursor_position = byte_pos.min(self.buffer.len());
        self.adjust_viewport();
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
