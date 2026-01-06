pub struct InputArea {
    pub buffer: String,
    pub cursor_position: usize,
    pub viewport_start_line: usize,
    pub visible_height: u16,
    pub width: u16,
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
        // Count characters before cursor, not bytes
        let chars_before = self.buffer[..self.cursor_position].chars().count();
        chars_before / self.width as usize
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

    pub fn current_word(&self) -> Option<(usize, usize, String)> {
        if self.buffer.is_empty() {
            return None;
        }

        let chars: Vec<char> = self.buffer.chars().collect();
        // Convert byte position to character position
        let char_pos = self.buffer[..self.cursor_position].chars().count();
        let pos = char_pos.min(chars.len());

        let mut start = pos;
        while start > 0 && chars[start - 1].is_alphabetic() {
            start -= 1;
        }

        let mut end = pos;
        while end < chars.len() && chars[end].is_alphabetic() {
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
                while start > 0 && chars[start - 1].is_alphabetic() {
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
