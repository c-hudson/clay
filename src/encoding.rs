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
                // FANSI: CP437 encoding used by many MUDs
                // Full CP437 mapping to avoid C1 control character issues
                bytes
                    .iter()
                    .map(|&b| {
                        if b < 128 {
                            b as char
                        } else {
                            // Complete CP437 high byte mapping (128-255)
                            match b {
                                // 0x80-0x9F: Accented letters and currency symbols
                                // IMPORTANT: These MUST be mapped to avoid C1 control chars
                                128 => '\u{00C7}', // Ç
                                129 => '\u{00FC}', // ü
                                130 => '\u{00E9}', // é
                                131 => '\u{00E2}', // â
                                132 => '\u{00E4}', // ä
                                133 => '\u{00E0}', // à
                                134 => '\u{00E5}', // å
                                135 => '\u{00E7}', // ç
                                136 => '\u{00EA}', // ê
                                137 => '\u{00EB}', // ë
                                138 => '\u{00E8}', // è
                                139 => '\u{00EF}', // ï
                                140 => '\u{00EE}', // î
                                141 => '\u{00EC}', // ì
                                142 => '\u{00C4}', // Ä
                                143 => '\u{00C5}', // Å
                                144 => '\u{00C9}', // É
                                145 => '\u{00E6}', // æ
                                146 => '\u{00C6}', // Æ
                                147 => '\u{00F4}', // ô
                                148 => '\u{00F6}', // ö
                                149 => '\u{00F2}', // ò
                                150 => '\u{00FB}', // û
                                151 => '\u{00F9}', // ù
                                152 => '\u{00FF}', // ÿ
                                153 => '\u{00D6}', // Ö
                                154 => '\u{00DC}', // Ü
                                155 => '\u{00A2}', // ¢ (cent sign - was causing CSI bug!)
                                156 => '\u{00A3}', // £ (pound sign)
                                157 => '\u{00A5}', // ¥ (yen sign)
                                158 => '\u{20A7}', // ₧ (peseta sign)
                                159 => '\u{0192}', // ƒ (florin sign)
                                // 0xA0-0xAF: More accented letters and symbols
                                160 => '\u{00E1}', // á
                                161 => '\u{00ED}', // í
                                162 => '\u{00F3}', // ó
                                163 => '\u{00FA}', // ú
                                164 => '\u{00F1}', // ñ
                                165 => '\u{00D1}', // Ñ
                                166 => '\u{00AA}', // ª
                                167 => '\u{00BA}', // º
                                168 => '\u{00BF}', // ¿
                                169 => '\u{2310}', // ⌐
                                170 => '\u{00AC}', // ¬
                                171 => '\u{00BD}', // ½
                                172 => '\u{00BC}', // ¼
                                173 => '\u{00A1}', // ¡
                                174 => '\u{00AB}', // «
                                175 => '\u{00BB}', // »
                                // 0xB0-0xDF: Box drawing characters
                                176 => '\u{2591}', // ░
                                177 => '\u{2592}', // ▒
                                178 => '\u{2593}', // ▓
                                179 => '\u{2502}', // │
                                180 => '\u{2524}', // ┤
                                181 => '\u{2561}', // ╡
                                182 => '\u{2562}', // ╢
                                183 => '\u{2556}', // ╖
                                184 => '\u{2555}', // ╕
                                185 => '\u{2563}', // ╣
                                186 => '\u{2551}', // ║
                                187 => '\u{2557}', // ╗
                                188 => '\u{255D}', // ╝
                                189 => '\u{255C}', // ╜
                                190 => '\u{255B}', // ╛
                                191 => '\u{2510}', // ┐
                                192 => '\u{2514}', // └
                                193 => '\u{2534}', // ┴
                                194 => '\u{252C}', // ┬
                                195 => '\u{251C}', // ├
                                196 => '\u{2500}', // ─
                                197 => '\u{253C}', // ┼
                                198 => '\u{255E}', // ╞
                                199 => '\u{255F}', // ╟
                                200 => '\u{255A}', // ╚
                                201 => '\u{2554}', // ╔
                                202 => '\u{2569}', // ╩
                                203 => '\u{2566}', // ╦
                                204 => '\u{2560}', // ╠
                                205 => '\u{2550}', // ═
                                206 => '\u{256C}', // ╬
                                207 => '\u{2567}', // ╧
                                208 => '\u{2568}', // ╨
                                209 => '\u{2564}', // ╤
                                210 => '\u{2565}', // ╥
                                211 => '\u{2559}', // ╙
                                212 => '\u{2558}', // ╘
                                213 => '\u{2552}', // ╒
                                214 => '\u{2553}', // ╓
                                215 => '\u{256B}', // ╫
                                216 => '\u{256A}', // ╪
                                217 => '\u{2518}', // ┘
                                218 => '\u{250C}', // ┌
                                219 => '\u{2588}', // █
                                220 => '\u{2584}', // ▄
                                221 => '\u{258C}', // ▌
                                222 => '\u{2590}', // ▐
                                223 => '\u{2580}', // ▀
                                // 0xE0-0xEF: Greek letters and math symbols
                                224 => '\u{03B1}', // α
                                225 => '\u{00DF}', // ß
                                226 => '\u{0393}', // Γ
                                227 => '\u{03C0}', // π
                                228 => '\u{03A3}', // Σ
                                229 => '\u{03C3}', // σ
                                230 => '\u{00B5}', // µ
                                231 => '\u{03C4}', // τ
                                232 => '\u{03A6}', // Φ
                                233 => '\u{0398}', // Θ
                                234 => '\u{03A9}', // Ω
                                235 => '\u{03B4}', // δ
                                236 => '\u{221E}', // ∞
                                237 => '\u{03C6}', // φ
                                238 => '\u{03B5}', // ε
                                239 => '\u{2229}', // ∩
                                // 0xF0-0xFF: Math symbols and special chars
                                240 => '\u{2261}', // ≡
                                241 => '\u{00B1}', // ±
                                242 => '\u{2265}', // ≥
                                243 => '\u{2264}', // ≤
                                244 => '\u{2320}', // ⌠
                                245 => '\u{2321}', // ⌡
                                246 => '\u{00F7}', // ÷
                                247 => '\u{2248}', // ≈
                                248 => '\u{00B0}', // °
                                249 => '\u{2219}', // ∙
                                250 => '\u{00B7}', // ·
                                251 => '\u{221A}', // √
                                252 => '\u{207F}', // ⁿ
                                253 => '\u{00B2}', // ²
                                254 => '\u{25A0}', // ■
                                255 => '\u{00A0}', // NBSP
                                _ => b as char, // Should never reach here
                            }
                        }
                    })
                    .collect()
            }
        };
        // Strip control characters that could cause rendering issues
        // Keep: \t (tab), \n (newline), \x1b (escape for ANSI sequences), \x07 (BEL for OSC termination)
        // Keep: \x0e (Ctrl-N, ANSI music terminator)
        // Also strip DEL (0x7F) and other high control chars
        let filtered: String = result
            .chars()
            .filter(|&c| (c >= ' ' && c != '\x7f') || c == '\t' || c == '\n' || c == '\x1b' || c == '\x07' || c == '\x0e')
            .collect();

        // Strip non-color ANSI sequences (cursor movement, erase, etc.)
        // Keep only SGR sequences (those ending with 'm' for colors/styles)
        let result = strip_non_sgr_sequences(&filtered);

        // Strip any remaining control characters (BEL, carriage returns, etc.)
        // Keep Ctrl-N for ANSI music terminator
        result.chars().filter(|&c| (c >= ' ' && c != '\x7f') || c == '\t' || c == '\n' || c == '\x1b' || c == '\x0e').collect()
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

                    // Check for ANSI music sequence (ESC [ M or ESC [ N followed by music content)
                    // Preserve these entirely so music parser can process them
                    if chars.peek() == Some(&'M') || chars.peek() == Some(&'N') {
                        let music_prefix = chars.next().unwrap(); // consume M or N
                        // Preserve the sequence - add ESC [ M/N and continue collecting until terminator
                        result.push('\x1b');
                        result.push('[');
                        result.push(music_prefix);
                        // Collect rest of music sequence (terminated by Ctrl-N, newline, or another ESC)
                        while let Some(&sc) = chars.peek() {
                            if sc == '\x0e' || sc == '\n' || sc == '\r' {
                                // Include terminator
                                result.push(sc);
                                chars.next();
                                break;
                            } else if sc == '\x1b' {
                                // Another escape sequence starts - stop here
                                break;
                            } else {
                                result.push(sc);
                                chars.next();
                            }
                        }
                        continue;
                    }

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

/// Check if a line contains ANSI codes but no visible content.
/// Returns true for lines that should be filtered (ANSI-only garbage like cursor control).
/// Returns false for legitimate blank lines, lines with content, or lines with background colors.
pub fn is_ansi_only_line(s: &str) -> bool {
    let mut has_ansi = false;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            has_ansi = true;
            in_escape = true;
        } else if in_escape {
            // Wait for end of escape sequence (alphabetic char or ~)
            if c.is_alphabetic() || c == '~' {
                in_escape = false;
            }
        } else if !c.is_whitespace() {
            // Found visible content - not ANSI-only
            return false;
        }
    }
    // Only filter if it had ANSI codes but no visible content
    // BUT don't filter lines with background colors - those are visually meaningful
    has_ansi && !has_background_color(s)
}

/// Check if a line has ANSI background color codes.
/// Returns true if the line contains background color sequences like:
/// - [48;5;Nm (256-color background)
/// - [48;2;R;G;Bm (true color background)
/// - [4Xm where X is 0-7 (standard 8-color background)
/// - [10Xm where X is 0-7 (bright background)
pub fn has_background_color(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.next() == Some('[') {
                // Collect the CSI parameters until we hit 'm' or non-numeric
                let mut params = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch == 'm' {
                        chars.next();
                        break;
                    } else if ch.is_ascii_digit() || ch == ';' {
                        params.push(ch);
                        chars.next();
                    } else {
                        // Not a color code
                        break;
                    }
                }
                // Check for background color codes in parameters
                for param in params.split(';') {
                    // Standard background colors: 40-47
                    // Bright background colors: 100-107
                    if let Ok(n) = param.parse::<u32>() {
                        if (40..=47).contains(&n) || (100..=107).contains(&n) {
                            return true;
                        }
                    }
                    // 48;5;N or 48;2;R;G;B
                    if param == "48" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Wrap URLs with OSC 8 hyperlink escape sequences for terminal clickability.
/// This makes URLs clickable in terminals that support OSC 8 (xfce4-terminal, gnome-terminal, etc.)
/// and ensures the full URL is accessible even when the visible text wraps across lines.
/// Format: \x1b]8;;URL\x07VISIBLE_TEXT\x1b]8;;\x07 (using BEL terminator for compatibility)
pub fn wrap_urls_with_osc8(s: &str) -> String {
    // Quick check - if no "http" in string, return as-is
    if !s.contains("http") {
        return s.to_string();
    }

    let mut result = String::with_capacity(s.len() * 2);
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Check for URL start (http:// or https://)
        let remaining: String = chars[i..].iter().collect();
        if remaining.starts_with("http://") || remaining.starts_with("https://") {
            // Find the end of the URL (whitespace, quotes, or certain punctuation at end)
            let url_start = i;
            let mut url_end = i;

            // URL can contain most characters, ends at whitespace or certain delimiters
            while url_end < chars.len() {
                let c = chars[url_end];
                // End URL at whitespace or common text delimiters
                if c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '>'
                   || c == '[' || c == ']' || c == '(' || c == ')' || c == '{' || c == '}' {
                    break;
                }
                url_end += 1;
            }

            // Strip trailing punctuation that's likely not part of the URL
            while url_end > url_start {
                let c = chars[url_end - 1];
                if c == '.' || c == ',' || c == ';' || c == ':' || c == '!' || c == '?' {
                    url_end -= 1;
                } else {
                    break;
                }
            }

            if url_end > url_start {
                let url: String = chars[url_start..url_end].iter().collect();
                // Clean URL for OSC 8 parameter - strip zero-width spaces that may have
                // been inserted for word breaking
                let clean_url = url.replace('\u{200B}', "");
                // OSC 8 format: \x1b]8;;URL\x07VISIBLE_TEXT\x1b]8;;\x07
                // Using BEL (0x07) as terminator for better terminal compatibility
                // URL parameter uses clean URL, visible text preserves original (may have ZWSP for breaking)
                result.push_str("\x1b]8;;");
                result.push_str(&clean_url);
                result.push('\x07');
                result.push_str(&url);
                result.push_str("\x1b]8;;\x07");
                i = url_end;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Replace colored square emoji with ANSI-colored block characters for console display
/// This ensures emoji like 🟩🟨 display in their proper colors in terminals
/// (Emoji fonts typically ignore ANSI colors, so we use block characters instead)
pub fn colorize_square_emojis(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if let Some((r, g, b)) = colored_square_rgb(c) {
            // Replace emoji with two colored block characters (emoji are typically 2 cells wide)
            // Use ANSI true-color (24-bit) foreground code with FULL BLOCK (█)
            result.push_str(&format!("\x1b[38;2;{};{};{}m██\x1b[0m", r, g, b));
        } else {
            result.push(c);
        }
    }
    result
}

/// Get RGB color for a colored square emoji, if it is one
fn colored_square_rgb(c: char) -> Option<(u8, u8, u8)> {
    match c {
        '🟥' => Some((0xDD, 0x2E, 0x44)), // Red
        '🟧' => Some((0xF4, 0x90, 0x0C)), // Orange
        '🟨' => Some((0xFD, 0xCB, 0x58)), // Yellow
        '🟩' => Some((0x78, 0xB1, 0x59)), // Green
        '🟦' => Some((0x55, 0xAC, 0xEE)), // Blue
        '🟪' => Some((0xAA, 0x8E, 0xD6)), // Purple
        '🟫' => Some((0xA0, 0x6A, 0x42)), // Brown
        '⬛' => Some((0x31, 0x37, 0x3D)), // Black
        '⬜' => Some((0xE6, 0xE7, 0xE8)), // White
        _ => None,
    }
}

/// Convert Discord custom emoji tags to Unicode or :name: fallback
/// Format: <:name:id> or <a:name:id> (animated)
pub fn convert_discord_emojis(s: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;

    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"<a?:([^:]+):\d+>").unwrap()
    });

    re.replace_all(s, |caps: &regex::Captures| {
        let name = &caps[1];
        // Try to map to Unicode emoji
        emoji_name_to_unicode(name).unwrap_or_else(|| format!(":{}:", name))
    }).to_string()
}

/// Map common emoji names to Unicode characters
fn emoji_name_to_unicode(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let emoji = match lower.as_str() {
        // Smileys & Emotion
        "smile" | "smiley" | "happy" => "😊",
        "grin" | "grinning" => "😀",
        "joy" | "laughing" | "lol" => "😂",
        "rofl" => "🤣",
        "wink" | "winking" => "😉",
        "blush" => "😊",
        "heart_eyes" | "hearteyes" => "😍",
        "kissing_heart" | "kissingheart" => "😘",
        "kiss" => "💋",
        "yum" | "delicious" => "😋",
        "stuck_out_tongue" | "tongue" => "😛",
        "crazy" | "zany" => "🤪",
        "thinking" | "think" | "hmm" => "🤔",
        "shush" | "shushing" => "🤫",
        "neutral" | "meh" => "😐",
        "expressionless" => "😑",
        "unamused" => "😒",
        "roll_eyes" | "rolleyes" | "eyeroll" => "🙄",
        "grimace" | "grimacing" => "😬",
        "relieved" => "😌",
        "pensive" => "😔",
        "sleepy" => "😪",
        "sleeping" | "zzz" => "😴",
        "sick" | "ill" => "🤢",
        "vomit" | "puke" => "🤮",
        "sneeze" | "sneezing" => "🤧",
        "hot" | "overheated" => "🥵",
        "cold" | "freezing" => "🥶",
        "woozy" | "dizzy" => "🥴",
        "exploding_head" | "mindblown" => "🤯",
        "cowboy" => "🤠",
        "sunglasses" | "cool" => "😎",
        "nerd" => "🤓",
        "monocle" => "🧐",
        "confused" => "😕",
        "worried" => "😟",
        "frown" | "frowning" | "sad" => "☹️",
        "cry" | "crying" => "😢",
        "sob" | "sobbing" => "😭",
        "angry" | "mad" => "😠",
        "rage" | "furious" => "😡",
        "skull" | "dead" => "💀",
        "poop" | "poo" | "shit" => "💩",
        "clown" => "🤡",
        "ghost" => "👻",
        "alien" => "👽",
        "robot" => "🤖",
        "cat" | "smiley_cat" => "😺",
        "heart_eyes_cat" => "😻",
        "scream_cat" => "🙀",
        "crying_cat" => "😿",
        "pouting_cat" => "😾",
        "devil" | "imp" => "😈",
        "angel" => "😇",

        // Gestures & Body
        "wave" | "waving" => "👋",
        "raised_hand" | "hand" => "✋",
        "ok_hand" | "ok" => "👌",
        "thumbs_up" | "thumbsup" | "+1" | "like" => "👍",
        "thumbs_down" | "thumbsdown" | "-1" | "dislike" => "👎",
        "clap" | "clapping" => "👏",
        "handshake" => "🤝",
        "pray" | "praying" | "please" | "thanks" => "🙏",
        "muscle" | "flex" | "strong" => "💪",
        "middle_finger" | "fu" => "🖕",
        "point_up" => "☝️",
        "point_down" => "👇",
        "point_left" => "👈",
        "point_right" => "👉",
        "fist" | "punch" => "👊",
        "raised_fist" => "✊",
        "v" | "peace" | "victory" => "✌️",
        "fingers_crossed" | "crossed_fingers" => "🤞",
        "love_you" | "ily" => "🤟",
        "metal" | "rock" | "horns" => "🤘",
        "call_me" | "shaka" => "🤙",
        "eyes" => "👀",
        "eye" => "👁️",
        "brain" => "🧠",

        // Hearts & Love
        "heart" | "love" | "red_heart" => "❤️",
        "orange_heart" => "🧡",
        "yellow_heart" => "💛",
        "green_heart" => "💚",
        "blue_heart" => "💙",
        "purple_heart" => "💜",
        "black_heart" => "🖤",
        "white_heart" => "🤍",
        "broken_heart" => "💔",
        "sparkling_heart" => "💖",
        "heartbeat" => "💓",
        "heartpulse" => "💗",
        "two_hearts" => "💕",
        "revolving_hearts" => "💞",
        "cupid" => "💘",
        "gift_heart" => "💝",

        // Nature & Animals
        "dog" | "puppy" => "🐕",
        "cat2" | "kitty" => "🐈",
        "mouse" => "🐁",
        "hamster" => "🐹",
        "rabbit" | "bunny" => "🐰",
        "fox" => "🦊",
        "bear" => "🐻",
        "panda" => "🐼",
        "koala" => "🐨",
        "tiger" => "🐯",
        "lion" => "🦁",
        "cow" => "🐄",
        "pig" => "🐷",
        "frog" => "🐸",
        "monkey" => "🐒",
        "chicken" | "hen" => "🐔",
        "penguin" => "🐧",
        "bird" => "🐦",
        "eagle" => "🦅",
        "duck" => "🦆",
        "owl" => "🦉",
        "bat" => "🦇",
        "wolf" => "🐺",
        "horse" => "🐴",
        "unicorn" => "🦄",
        "bee" => "🐝",
        "bug" | "beetle" => "🐛",
        "butterfly" => "🦋",
        "snail" => "🐌",
        "shell" => "🐚",
        "crab" => "🦀",
        "shrimp" => "🦐",
        "squid" => "🦑",
        "octopus" => "🐙",
        "fish" => "🐟",
        "dolphin" => "🐬",
        "whale" => "🐳",
        "shark" => "🦈",
        "crocodile" | "alligator" => "🐊",
        "snake" => "🐍",
        "turtle" => "🐢",
        "dragon" => "🐉",
        "dragon_face" => "🐲",
        "t_rex" | "trex" | "dinosaur" => "🦖",

        // Food & Drink
        "apple" => "🍎",
        "banana" => "🍌",
        "orange" => "🍊",
        "lemon" => "🍋",
        "watermelon" => "🍉",
        "grapes" => "🍇",
        "strawberry" => "🍓",
        "peach" => "🍑",
        "cherry" | "cherries" => "🍒",
        "pizza" => "🍕",
        "hamburger" | "burger" => "🍔",
        "fries" | "french_fries" => "🍟",
        "hotdog" | "hot_dog" => "🌭",
        "taco" => "🌮",
        "burrito" => "🌯",
        "popcorn" => "🍿",
        "icecream" | "ice_cream" => "🍦",
        "donut" | "doughnut" => "🍩",
        "cookie" => "🍪",
        "cake" | "birthday" => "🎂",
        "pie" => "🥧",
        "chocolate" => "🍫",
        "candy" => "🍬",
        "coffee" | "cafe" => "☕",
        "tea" => "🍵",
        "beer" => "🍺",
        "beers" => "🍻",
        "wine" | "wine_glass" => "🍷",
        "cocktail" | "martini" => "🍸",
        "champagne" => "🍾",

        // Activities & Sports
        "soccer" | "football" => "⚽",
        "basketball" => "🏀",
        "baseball" => "⚾",
        "tennis" => "🎾",
        "volleyball" => "🏐",
        "golf" => "⛳",
        "bowling" => "🎳",
        "trophy" | "winner" => "🏆",
        "medal" | "gold_medal" => "🥇",
        "silver_medal" => "🥈",
        "bronze_medal" => "🥉",
        "video_game" | "gaming" | "controller" => "🎮",
        "dice" | "game_die" => "🎲",
        "dart" | "bullseye" => "🎯",

        // Objects
        "phone" | "iphone" | "mobile" => "📱",
        "computer" | "laptop" | "pc" => "💻",
        "keyboard" => "⌨️",
        "mouse2" | "computer_mouse" => "🖱️",
        "printer" => "🖨️",
        "camera" => "📷",
        "tv" | "television" => "📺",
        "radio" => "📻",
        "bulb" | "lightbulb" | "idea" => "💡",
        "flashlight" | "torch" => "🔦",
        "book" => "📖",
        "books" => "📚",
        "money" | "cash" | "dollar" => "💵",
        "credit_card" => "💳",
        "gem" | "diamond" => "💎",
        "hammer" => "🔨",
        "wrench" => "🔧",
        "gear" | "cog" | "settings" => "⚙️",
        "lock" | "locked" => "🔒",
        "unlock" | "unlocked" => "🔓",
        "key" => "🔑",
        "bell" => "🔔",
        "gift" | "present" => "🎁",
        "balloon" | "balloons" => "🎈",
        "tada" | "party" | "celebration" => "🎉",
        "confetti" => "🎊",

        // Symbols & Misc
        "check" | "checkmark" | "yes" => "✅",
        "x" | "cross" | "no" => "❌",
        "warning" | "warn" => "⚠️",
        "question" | "?" => "❓",
        "exclamation" | "!" => "❗",
        "100" | "hundred" => "💯",
        "fire" | "lit" | "hot2" => "🔥",
        "star" | "stars" => "⭐",
        "sparkles" | "sparkle" => "✨",
        "boom" | "explosion" => "💥",
        "zap" | "lightning" | "thunder" => "⚡",
        "rainbow" => "🌈",
        "sun" | "sunny" => "☀️",
        "moon" | "crescent_moon" => "🌙",
        "cloud" | "cloudy" => "☁️",
        "rain" | "rainy" => "🌧️",
        "snow" | "snowy" | "snowflake" => "❄️",
        "earth" | "globe" | "world" => "🌍",
        "rocket" => "🚀",
        "airplane" | "plane" => "✈️",
        "car" | "automobile" => "🚗",
        "bus" => "🚌",
        "train" => "🚃",
        "bike" | "bicycle" => "🚲",
        "crown" | "king" | "queen" => "👑",
        "ring" => "💍",
        "clock" | "time" => "🕐",
        "hourglass" => "⏳",
        "alarm" | "alarm_clock" => "⏰",
        "music" | "musical_note" => "🎵",
        "notes" | "musical_notes" => "🎶",
        "microphone" | "mic" => "🎤",
        "headphones" | "headphone" => "🎧",
        "art" | "palette" => "🎨",
        "movie" | "film" => "🎬",
        "mask" | "theater" => "🎭",
        "flag" => "🚩",
        "white_flag" => "🏳️",
        "skull_crossbones" | "danger" => "☠️",

        // Arrows & Shapes
        "arrow_up" | "up" => "⬆️",
        "arrow_down" | "down" => "⬇️",
        "arrow_left" | "left" => "⬅️",
        "arrow_right" | "right" => "➡️",
        "arrows_counterclockwise" | "refresh" | "reload" => "🔄",
        "plus" | "add" => "➕",
        "minus" | "subtract" => "➖",

        _ => return None,
    };
    Some(emoji.to_string())
}
