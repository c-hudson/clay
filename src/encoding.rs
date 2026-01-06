use ratatui::style::Color;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Encoding {
    #[default]
    Utf8,
    Latin1,
    Fansi,
}

impl Encoding {
    pub fn decode(&self, bytes: &[u8]) -> String {
        let result = match self {
            Encoding::Utf8 => String::from_utf8_lossy(bytes).to_string(),
            Encoding::Latin1 => {
                // ISO-8859-1: each byte maps directly to its Unicode codepoint
                bytes.iter().map(|&b| b as char).collect()
            }
            Encoding::Fansi => {
                // FANSI: Similar to CP437 with some MUD-specific extensions
                // For simplicity, use a basic CP437-like mapping for high bytes
                bytes
                    .iter()
                    .map(|&b| {
                        if b < 128 {
                            b as char
                        } else {
                            // CP437 high characters - common box drawing and symbols
                            match b {
                                128 => '\u{00C7}', // Ç
                                129 => '\u{00FC}', // ü
                                130 => '\u{00E9}', // é
                                131 => '\u{00E2}', // â
                                132 => '\u{00E4}', // ä
                                133 => '\u{00E0}', // à
                                134 => '\u{00E5}', // å
                                135 => '\u{00E7}', // ç
                                176 => '\u{2591}', // ░
                                177 => '\u{2592}', // ▒
                                178 => '\u{2593}', // ▓
                                179 => '\u{2502}', // │
                                180 => '\u{2524}', // ┤
                                191 => '\u{2510}', // ┐
                                192 => '\u{2514}', // └
                                193 => '\u{2534}', // ┴
                                194 => '\u{252C}', // ┬
                                195 => '\u{251C}', // ├
                                196 => '\u{2500}', // ─
                                197 => '\u{253C}', // ┼
                                217 => '\u{2518}', // ┘
                                218 => '\u{250C}', // ┌
                                219 => '\u{2588}', // █
                                220 => '\u{2584}', // ▄
                                221 => '\u{258C}', // ▌
                                222 => '\u{2590}', // ▐
                                223 => '\u{2580}', // ▀
                                _ => b as char, // Fallback
                            }
                        }
                    })
                    .collect()
            }
        };
        // Strip control characters that could cause rendering issues
        // Keep: \t (tab), \n (newline), \x1b (escape for ANSI sequences), \x07 (BEL for OSC termination)
        // Also strip DEL (0x7F) and other high control chars
        let filtered: String = result
            .chars()
            .filter(|&c| (c >= ' ' && c != '\x7f') || c == '\t' || c == '\n' || c == '\x1b' || c == '\x07')
            .collect();

        // Strip non-color ANSI sequences (cursor movement, erase, etc.)
        // Keep only SGR sequences (those ending with 'm' for colors/styles)
        let result = strip_non_sgr_sequences(&filtered);

        // Strip any remaining control characters (BEL, carriage returns, etc.)
        result.chars().filter(|&c| (c >= ' ' && c != '\x7f') || c == '\t' || c == '\n' || c == '\x1b').collect()
    }

    pub fn name(&self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf8",
            Encoding::Latin1 => "latin1",
            Encoding::Fansi => "fansi",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Encoding::Utf8 => Encoding::Latin1,
            Encoding::Latin1 => Encoding::Fansi,
            Encoding::Fansi => Encoding::Utf8,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Encoding::Utf8 => Encoding::Fansi,
            Encoding::Latin1 => Encoding::Utf8,
            Encoding::Fansi => Encoding::Latin1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn name(&self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "light" => Theme::Light,
            _ => Theme::Dark,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        }
    }

    // Theme colors for TUI rendering
    pub fn bg(&self) -> Color {
        match self {
            Theme::Dark => Color::Reset,  // Use terminal default (usually black)
            Theme::Light => Color::White,
        }
    }

    pub fn fg(&self) -> Color {
        match self {
            Theme::Dark => Color::White,
            Theme::Light => Color::Black,
        }
    }

    pub fn fg_dim(&self) -> Color {
        match self {
            Theme::Dark => Color::DarkGray,
            Theme::Light => Color::Gray,
        }
    }

    pub fn fg_accent(&self) -> Color {
        match self {
            Theme::Dark => Color::Cyan,
            Theme::Light => Color::Blue,
        }
    }

    pub fn fg_highlight(&self) -> Color {
        match self {
            Theme::Dark => Color::Yellow,
            Theme::Light => Color::Rgb(180, 100, 0),  // Darker orange/gold for light theme
        }
    }

    pub fn fg_success(&self) -> Color {
        match self {
            Theme::Dark => Color::Green,
            Theme::Light => Color::Rgb(0, 128, 0),  // Darker green for light theme
        }
    }

    pub fn fg_error(&self) -> Color {
        match self {
            Theme::Dark => Color::Red,
            Theme::Light => Color::Rgb(180, 0, 0),  // Darker red for light theme
        }
    }

    pub fn popup_border(&self) -> Color {
        match self {
            Theme::Dark => Color::Cyan,
            Theme::Light => Color::Blue,
        }
    }

    pub fn popup_bg(&self) -> Color {
        match self {
            Theme::Dark => Color::Reset,
            Theme::Light => Color::Rgb(245, 245, 245),  // Light gray background
        }
    }

    pub fn button_selected_fg(&self) -> Color {
        match self {
            Theme::Dark => Color::Black,
            Theme::Light => Color::White,
        }
    }

    pub fn button_selected_bg(&self) -> Color {
        match self {
            Theme::Dark => Color::White,
            Theme::Light => Color::Blue,
        }
    }
}

/// World switching mode for Up/Down arrow cycling
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum WorldSwitchMode {
    #[default]
    UnseenFirst,
    Alphabetical,
}

impl WorldSwitchMode {
    pub fn name(&self) -> &'static str {
        match self {
            WorldSwitchMode::UnseenFirst => "Unseen First",
            WorldSwitchMode::Alphabetical => "Alphabetical",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "alphabetical" => WorldSwitchMode::Alphabetical,
            _ => WorldSwitchMode::UnseenFirst,
        }
    }

    pub fn next(&self) -> Self {
        match self {
            WorldSwitchMode::UnseenFirst => WorldSwitchMode::Alphabetical,
            WorldSwitchMode::Alphabetical => WorldSwitchMode::UnseenFirst,
        }
    }
}

/// Strip non-SGR ANSI escape sequences (cursor movement, erase, etc.)
/// Keep only SGR sequences (CSI ... m) for colors and styles
/// For cursor positioning sequences, insert appropriate separators
pub fn strip_non_sgr_sequences(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    // Check if a character is a CSI final byte (0x40-0x7E: @ through ~)
    fn is_csi_final_byte(c: char) -> bool {
        ('@'..='~').contains(&c)
    }

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Start of escape sequence
            match chars.peek() {
                Some(&'[') => {
                    // CSI sequence: ESC [ <params> <final byte>
                    // Final byte is 0x40-0x7E (@ through ~)
                    chars.next(); // consume '['
                    let mut seq = String::new();
                    seq.push('\x1b');
                    seq.push('[');
                    while let Some(&sc) = chars.peek() {
                        if is_csi_final_byte(sc) {
                            // End of CSI sequence
                            chars.next();
                            seq.push(sc);
                            // Only keep SGR sequences (ending with 'm')
                            if sc == 'm' {
                                result.push_str(&seq);
                            } else {
                                // For cursor positioning sequences, add a space or newline
                                // to prevent text from different positions from concatenating
                                match sc {
                                    'H' | 'f' => {
                                        // Cursor position (row;col) - add newline
                                        if !result.ends_with('\n') && !result.is_empty() {
                                            result.push('\n');
                                        }
                                    }
                                    'A' | 'B' | 'E' | 'F' => {
                                        // Cursor up/down/next line/prev line - add newline
                                        if !result.ends_with('\n') && !result.is_empty() {
                                            result.push('\n');
                                        }
                                    }
                                    'G' | 'C' | 'D' | '`' => {
                                        // Cursor column/forward/back - add space
                                        if !result.ends_with(' ') && !result.ends_with('\n') && !result.is_empty() {
                                            result.push(' ');
                                        }
                                    }
                                    _ => {
                                        // Other sequences (J, K, @, etc.) - just discard
                                    }
                                }
                            }
                            break;
                        } else {
                            chars.next();
                            seq.push(sc);
                        }
                    }
                }
                Some(&']') => {
                    // OSC sequence: ESC ] ... (terminated by BEL \x07 or ST ESC \)
                    chars.next(); // consume ']'
                    while let Some(&sc) = chars.peek() {
                        chars.next();
                        if sc == '\x07' {
                            // BEL terminates OSC
                            break;
                        } else if sc == '\x1b' {
                            // Check for ST (ESC \)
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                }
                Some(&'P') | Some(&'^') | Some(&'_') | Some(&'X') => {
                    // DCS, PM, APC, SOS sequences: terminated by ST (ESC \)
                    chars.next(); // consume the type character
                    while let Some(&sc) = chars.peek() {
                        chars.next();
                        if sc == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(&'N') | Some(&'O') => {
                    // SS2, SS3: followed by one character
                    chars.next(); // consume N or O
                    chars.next(); // consume the following character
                }
                Some(&c2) if c2.is_ascii_alphabetic() => {
                    // Simple escape sequence like ESC M (reverse line feed)
                    chars.next(); // consume the letter
                }
                _ => {
                    // Unknown escape sequence, just skip the ESC
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Check if a line is visually empty (contains only ANSI codes and/or whitespace)
pub fn is_visually_empty(s: &str) -> bool {
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            // Wait for end of escape sequence (alphabetic char or ~)
            if c.is_alphabetic() || c == '~' {
                in_escape = false;
            }
        } else if !c.is_whitespace() {
            // Found visible content
            return false;
        }
    }
    true
}
