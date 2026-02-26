// Theme system for Clay MUD client
// Loads theme colors from ~/clay.theme.dat and provides them to all renderers

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A single RGB color value
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl ThemeColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse a hex color string like "#RRGGBB" or "RRGGBB"
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }

    /// Convert to #RRGGBB hex string
    pub fn to_css(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Convert to ratatui Color::Rgb
    pub fn to_ratatui(&self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(self.r, self.g, self.b)
    }

    /// Convert to egui Color32 (only available with GUI features)
    #[cfg(feature = "remote-gui")]
    pub fn to_egui(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.r, self.g, self.b)
    }

    /// Convert to (r, g, b) tuple
    pub fn to_tuple(&self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

/// All theme color variables for a single theme
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeColors {
    // Background hierarchy
    pub bg: ThemeColor,
    pub bg_deep: ThemeColor,
    pub bg_surface: ThemeColor,
    pub bg_elevated: ThemeColor,
    pub bg_hover: ThemeColor,

    // Foreground hierarchy
    pub fg: ThemeColor,
    pub fg_secondary: ThemeColor,
    pub fg_muted: ThemeColor,
    pub fg_dim: ThemeColor,

    // Semantic colors
    pub accent: ThemeColor,
    pub accent_dim: ThemeColor,
    pub highlight: ThemeColor,
    pub success: ThemeColor,
    pub error: ThemeColor,
    pub error_dim: ThemeColor,

    // UI elements
    pub status_bar_bg: ThemeColor,
    pub menu_bar_bg: ThemeColor,
    pub selection_bg: ThemeColor,
    pub link: ThemeColor,
    pub prompt: ThemeColor,
    pub border_subtle: ThemeColor,
    pub border_medium: ThemeColor,
    pub button_selected_bg: ThemeColor,
    pub button_selected_fg: ThemeColor,
    pub more_indicator_bg: ThemeColor,
    pub activity_bg: ThemeColor,

    // ANSI palette (16 standard colors)
    pub ansi: [ThemeColor; 16],
}

impl ThemeColors {
    /// Default dark theme - imported from ~/clay.theme.dat
    pub fn dark_default() -> Self {
        Self {
            // Background hierarchy
            bg: ThemeColor::new(0x13, 0x19, 0x26),      // #131926
            bg_deep: ThemeColor::new(0x13, 0x19, 0x26),  // #131926
            bg_surface: ThemeColor::new(0x1c, 0x17, 0x22), // #1c1722
            bg_elevated: ThemeColor::new(0x1f, 0x1f, 0x1f), // #1f1f1f
            bg_hover: ThemeColor::new(0x2c, 0x25, 0x35),  // #2c2535

            // Foreground hierarchy
            fg: ThemeColor::new(0xe8, 0xe4, 0xec),        // #e8e4ec
            fg_secondary: ThemeColor::new(0xa8, 0x9f, 0xb4), // #a89fb4
            fg_muted: ThemeColor::new(0x6e, 0x64, 0x79),  // #6e6479
            fg_dim: ThemeColor::new(0x4a, 0x42, 0x55),    // #4a4255

            // Semantic colors
            accent: ThemeColor::new(0x26, 0x57, 0xba),    // #2657ba
            accent_dim: ThemeColor::new(0x00, 0x40, 0x80), // #004080
            highlight: ThemeColor::new(0xe8, 0xc4, 0x6a),  // #e8c46a
            success: ThemeColor::new(0x7e, 0xcf, 0x8b),    // #7ecf8b
            error: ThemeColor::new(0xb1, 0x0a, 0x0a),      // #b10a0a
            error_dim: ThemeColor::new(0x5f, 0x00, 0x00),   // #5f0000

            // UI elements
            status_bar_bg: ThemeColor::new(0x28, 0x4b, 0x63), // #284b63
            menu_bar_bg: ThemeColor::new(0x15, 0x2b, 0x3a),    // #152b3a
            selection_bg: ThemeColor::new(0x00, 0x40, 0x80),   // #004080
            link: ThemeColor::new(0x8c, 0xb4, 0xe0),        // #8cb4e0
            prompt: ThemeColor::new(0xd4, 0x84, 0x5a),       // #d4845a
            border_subtle: ThemeColor::new(0x22, 0x1c, 0x2b),  // #221c2b
            border_medium: ThemeColor::new(0x2e, 0x27, 0x38),  // #2e2738
            button_selected_bg: ThemeColor::new(0xe8, 0xe4, 0xec), // #e8e4ec
            button_selected_fg: ThemeColor::new(0x13, 0x19, 0x26), // #131926
            more_indicator_bg: ThemeColor::new(0x5f, 0x00, 0x00),  // #5f0000
            activity_bg: ThemeColor::new(0xf5, 0xf0, 0xd8),       // #f5f0d8

            // ANSI palette - Xubuntu Dark
            ansi: [
                ThemeColor::new(0, 0, 0),         // 0: black
                ThemeColor::new(170, 0, 0),       // 1: red
                ThemeColor::new(68, 170, 68),     // 2: green
                ThemeColor::new(170, 85, 0),      // 3: yellow
                ThemeColor::new(0, 57, 170),      // 4: blue
                ThemeColor::new(170, 34, 170),    // 5: magenta
                ThemeColor::new(26, 146, 170),    // 6: cyan
                ThemeColor::new(170, 170, 170),   // 7: white
                ThemeColor::new(119, 119, 119),   // 8: bright black
                ThemeColor::new(255, 135, 135),   // 9: bright red
                ThemeColor::new(76, 230, 76),     // 10: bright green
                ThemeColor::new(222, 216, 44),    // 11: bright yellow
                ThemeColor::new(41, 95, 204),     // 12: bright blue
                ThemeColor::new(204, 88, 204),    // 13: bright magenta
                ThemeColor::new(76, 204, 230),    // 14: bright cyan
                ThemeColor::new(255, 255, 255),   // 15: bright white
            ],
        }
    }

    /// Default light theme - matches current GUI2 light values
    pub fn light_default() -> Self {
        Self {
            // Background hierarchy
            bg: ThemeColor::new(232, 232, 232),        // #e8e8e8
            bg_deep: ThemeColor::new(232, 232, 232),   // #e8e8e8
            bg_surface: ThemeColor::new(179, 179, 179), // #b3b3b3
            bg_elevated: ThemeColor::new(240, 240, 240), // #f0f0f0
            bg_hover: ThemeColor::new(210, 210, 210),   // #d2d2d2

            // Foreground hierarchy
            fg: ThemeColor::new(29, 29, 31),           // #1d1d1f
            fg_secondary: ThemeColor::new(99, 99, 102), // #636366
            fg_muted: ThemeColor::new(142, 142, 147),   // #8e8e93
            fg_dim: ThemeColor::new(174, 174, 178),     // #aeaeb2

            // Semantic colors
            accent: ThemeColor::new(184, 107, 63),      // #b86b3f
            accent_dim: ThemeColor::new(160, 90, 50),   // #a05a32
            highlight: ThemeColor::new(224, 134, 0),     // #e08600
            success: ThemeColor::new(52, 199, 89),       // #34c759
            error: ThemeColor::new(255, 59, 48),         // #ff3b30
            error_dim: ThemeColor::new(95, 0, 0),        // #5f0000

            // UI elements
            status_bar_bg: ThemeColor::new(155, 155, 155), // #9b9b9b
            menu_bar_bg: ThemeColor::new(220, 220, 220),    // #dcdcdc
            selection_bg: ThemeColor::new(180, 200, 230),   // #b4c8e6
            link: ThemeColor::new(0, 122, 255),             // #007aff
            prompt: ThemeColor::new(184, 107, 63),          // #b86b3f (same as accent)
            border_subtle: ThemeColor::new(192, 192, 192),  // #c0c0c0
            border_medium: ThemeColor::new(176, 176, 176),  // #b0b0b0
            button_selected_bg: ThemeColor::new(29, 29, 31),  // #1d1d1f (same as fg)
            button_selected_fg: ThemeColor::new(232, 232, 232), // #e8e8e8 (same as bg)
            more_indicator_bg: ThemeColor::new(95, 0, 0),       // #5f0000
            activity_bg: ThemeColor::new(252, 186, 3),          // #fcba03

            // ANSI palette - Xubuntu with light theme adjustments
            ansi: [
                ThemeColor::new(0, 0, 0),         // 0: black
                ThemeColor::new(170, 0, 0),       // 1: red
                ThemeColor::new(68, 170, 68),     // 2: green
                ThemeColor::new(128, 64, 0),      // 3: yellow (darker for light bg)
                ThemeColor::new(0, 57, 170),      // 4: blue
                ThemeColor::new(170, 34, 170),    // 5: magenta
                ThemeColor::new(26, 146, 170),    // 6: cyan
                ThemeColor::new(80, 80, 80),      // 7: white (dark gray for light bg)
                ThemeColor::new(119, 119, 119),   // 8: bright black
                ThemeColor::new(255, 135, 135),   // 9: bright red
                ThemeColor::new(76, 230, 76),     // 10: bright green
                ThemeColor::new(167, 163, 33),    // 11: bright yellow (darker for light bg)
                ThemeColor::new(41, 95, 204),     // 12: bright blue
                ThemeColor::new(204, 88, 204),    // 13: bright magenta
                ThemeColor::new(76, 204, 230),    // 14: bright cyan
                ThemeColor::new(40, 40, 40),      // 15: bright white (near black for light bg)
            ],
        }
    }

    /// Set a named color variable from a key-value pair
    fn set_var(&mut self, key: &str, color: ThemeColor) -> bool {
        match key {
            "bg" => self.bg = color,
            "bg_deep" => self.bg_deep = color,
            "bg_surface" => self.bg_surface = color,
            "bg_elevated" => self.bg_elevated = color,
            "bg_hover" => self.bg_hover = color,
            "fg" => self.fg = color,
            "fg_secondary" => self.fg_secondary = color,
            "fg_muted" => self.fg_muted = color,
            "fg_dim" => self.fg_dim = color,
            "accent" => self.accent = color,
            "accent_dim" => self.accent_dim = color,
            "highlight" => self.highlight = color,
            "success" => self.success = color,
            "error" => self.error = color,
            "error_dim" => self.error_dim = color,
            "status_bar.bg" => self.status_bar_bg = color,
            "menu_bar.bg" => self.menu_bar_bg = color,
            "selection.bg" => self.selection_bg = color,
            "link" => self.link = color,
            "prompt" => self.prompt = color,
            "border_subtle" => self.border_subtle = color,
            "border_medium" => self.border_medium = color,
            "button.selected_bg" => self.button_selected_bg = color,
            "button.selected_fg" => self.button_selected_fg = color,
            "more_indicator.bg" => self.more_indicator_bg = color,
            "activity.bg" => self.activity_bg = color,
            _ => {
                // Handle ansi.0 through ansi.15
                if let Some(rest) = key.strip_prefix("ansi.") {
                    if let Ok(idx) = rest.parse::<usize>() {
                        if idx < 16 {
                            self.ansi[idx] = color;
                            return true;
                        }
                    }
                }
                return false;
            }
        }
        true
    }

    // Console-compatible convenience methods (same names as encoding::Theme methods)
    // These allow `let theme = app.theme_colors()` to be a drop-in replacement for
    // `let theme = app.settings.theme` in rendering code.

    pub fn bg(&self) -> ratatui::style::Color { self.bg.to_ratatui() }
    pub fn fg(&self) -> ratatui::style::Color { self.fg.to_ratatui() }
    pub fn fg_dim(&self) -> ratatui::style::Color { self.fg_dim.to_ratatui() }
    pub fn fg_accent(&self) -> ratatui::style::Color { self.accent.to_ratatui() }
    pub fn fg_highlight(&self) -> ratatui::style::Color { self.highlight.to_ratatui() }
    pub fn fg_success(&self) -> ratatui::style::Color { self.success.to_ratatui() }
    pub fn fg_error(&self) -> ratatui::style::Color { self.error.to_ratatui() }
    pub fn popup_border(&self) -> ratatui::style::Color { self.accent.to_ratatui() }
    pub fn popup_bg(&self) -> ratatui::style::Color { self.bg_elevated.to_ratatui() }
    pub fn button_selected_fg(&self) -> ratatui::style::Color { self.button_selected_fg.to_ratatui() }
    pub fn button_selected_bg(&self) -> ratatui::style::Color { self.button_selected_bg.to_ratatui() }
    pub fn selection_bg(&self) -> ratatui::style::Color { self.selection_bg.to_ratatui() }

    /// Generate CSS custom properties for all theme variables
    pub fn to_css_vars(&self) -> String {
        let mut css = String::new();
        css.push_str(&format!("--theme-bg: {};\n", self.bg.to_css()));
        css.push_str(&format!("--theme-bg-deep: {};\n", self.bg_deep.to_css()));
        css.push_str(&format!("--theme-bg-surface: {};\n", self.bg_surface.to_css()));
        css.push_str(&format!("--theme-bg-elevated: {};\n", self.bg_elevated.to_css()));
        css.push_str(&format!("--theme-bg-hover: {};\n", self.bg_hover.to_css()));
        css.push_str(&format!("--theme-fg: {};\n", self.fg.to_css()));
        css.push_str(&format!("--theme-fg-secondary: {};\n", self.fg_secondary.to_css()));
        css.push_str(&format!("--theme-fg-muted: {};\n", self.fg_muted.to_css()));
        css.push_str(&format!("--theme-fg-dim: {};\n", self.fg_dim.to_css()));
        css.push_str(&format!("--theme-accent: {};\n", self.accent.to_css()));
        css.push_str(&format!("--theme-accent-dim: {};\n", self.accent_dim.to_css()));
        css.push_str(&format!("--theme-highlight: {};\n", self.highlight.to_css()));
        css.push_str(&format!("--theme-success: {};\n", self.success.to_css()));
        css.push_str(&format!("--theme-error: {};\n", self.error.to_css()));
        css.push_str(&format!("--theme-error-dim: {};\n", self.error_dim.to_css()));
        css.push_str(&format!("--theme-status-bar-bg: {};\n", self.status_bar_bg.to_css()));
        css.push_str(&format!("--theme-menu-bar-bg: {};\n", self.menu_bar_bg.to_css()));
        css.push_str(&format!("--theme-selection-bg: {};\n", self.selection_bg.to_css()));
        css.push_str(&format!("--theme-link: {};\n", self.link.to_css()));
        css.push_str(&format!("--theme-prompt: {};\n", self.prompt.to_css()));
        css.push_str(&format!("--theme-border-subtle: {};\n", self.border_subtle.to_css()));
        css.push_str(&format!("--theme-border-medium: {};\n", self.border_medium.to_css()));
        css.push_str(&format!("--theme-button-selected-bg: {};\n", self.button_selected_bg.to_css()));
        css.push_str(&format!("--theme-button-selected-fg: {};\n", self.button_selected_fg.to_css()));
        css.push_str(&format!("--theme-more-indicator-bg: {};\n", self.more_indicator_bg.to_css()));
        css.push_str(&format!("--theme-activity-bg: {};\n", self.activity_bg.to_css()));
        for i in 0..16 {
            css.push_str(&format!("--theme-ansi-{}: {};\n", i, self.ansi[i].to_css()));
        }
        css
    }

    /// Generate the theme file content for this theme with comments
    pub fn to_theme_file_section(&self) -> String {
        let mut s = String::new();

        s.push_str("# Background hierarchy\n");
        s.push_str(&format!("bg = {}\n", self.bg.to_css()));
        s.push_str(&format!("bg_deep = {}\n", self.bg_deep.to_css()));
        s.push_str(&format!("bg_surface = {}\n", self.bg_surface.to_css()));
        s.push_str(&format!("bg_elevated = {}\n", self.bg_elevated.to_css()));
        s.push_str(&format!("bg_hover = {}\n", self.bg_hover.to_css()));
        s.push_str("\n# Foreground hierarchy\n");
        s.push_str(&format!("fg = {}\n", self.fg.to_css()));
        s.push_str(&format!("fg_secondary = {}\n", self.fg_secondary.to_css()));
        s.push_str(&format!("fg_muted = {}\n", self.fg_muted.to_css()));
        s.push_str(&format!("fg_dim = {}\n", self.fg_dim.to_css()));
        s.push_str("\n# Semantic colors\n");
        s.push_str(&format!("accent = {}\n", self.accent.to_css()));
        s.push_str(&format!("accent_dim = {}\n", self.accent_dim.to_css()));
        s.push_str(&format!("highlight = {}\n", self.highlight.to_css()));
        s.push_str(&format!("success = {}\n", self.success.to_css()));
        s.push_str(&format!("error = {}\n", self.error.to_css()));
        s.push_str(&format!("error_dim = {}\n", self.error_dim.to_css()));
        s.push_str("\n# UI elements\n");
        s.push_str(&format!("status_bar.bg = {}\n", self.status_bar_bg.to_css()));
        s.push_str(&format!("menu_bar.bg = {}\n", self.menu_bar_bg.to_css()));
        s.push_str(&format!("selection.bg = {}\n", self.selection_bg.to_css()));
        s.push_str(&format!("link = {}\n", self.link.to_css()));
        s.push_str(&format!("prompt = {}\n", self.prompt.to_css()));
        s.push_str(&format!("border_subtle = {}\n", self.border_subtle.to_css()));
        s.push_str(&format!("border_medium = {}\n", self.border_medium.to_css()));
        s.push_str(&format!("button.selected_bg = {}\n", self.button_selected_bg.to_css()));
        s.push_str(&format!("button.selected_fg = {}\n", self.button_selected_fg.to_css()));
        s.push_str(&format!("more_indicator.bg = {}\n", self.more_indicator_bg.to_css()));
        s.push_str(&format!("activity.bg = {}\n", self.activity_bg.to_css()));
        s.push_str("\n# ANSI palette (16 standard colors)\n");
        let ansi_names = [
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
            "bright black", "bright red", "bright green", "bright yellow",
            "bright blue", "bright magenta", "bright cyan", "bright white",
        ];
        for (i, name) in ansi_names.iter().enumerate() {
            s.push_str(&format!("ansi.{} = {}  # {}\n", i, self.ansi[i].to_css(), name));
        }
        s
    }

    /// Serialize to JSON string (for WebSocket transport)
    pub fn to_json(&self) -> String {
        let mut map = serde_json::Map::new();
        let mut add = |key: &str, c: &ThemeColor| {
            map.insert(key.to_string(), serde_json::Value::String(c.to_css()));
        };
        add("bg", &self.bg);
        add("bg_deep", &self.bg_deep);
        add("bg_surface", &self.bg_surface);
        add("bg_elevated", &self.bg_elevated);
        add("bg_hover", &self.bg_hover);
        add("fg", &self.fg);
        add("fg_secondary", &self.fg_secondary);
        add("fg_muted", &self.fg_muted);
        add("fg_dim", &self.fg_dim);
        add("accent", &self.accent);
        add("accent_dim", &self.accent_dim);
        add("highlight", &self.highlight);
        add("success", &self.success);
        add("error", &self.error);
        add("error_dim", &self.error_dim);
        add("status_bar.bg", &self.status_bar_bg);
        add("menu_bar.bg", &self.menu_bar_bg);
        add("selection.bg", &self.selection_bg);
        add("link", &self.link);
        add("prompt", &self.prompt);
        add("border_subtle", &self.border_subtle);
        add("border_medium", &self.border_medium);
        add("button.selected_bg", &self.button_selected_bg);
        add("button.selected_fg", &self.button_selected_fg);
        add("more_indicator.bg", &self.more_indicator_bg);
        add("activity.bg", &self.activity_bg);
        for i in 0..16 {
            add(&format!("ansi.{}", i), &self.ansi[i]);
        }
        serde_json::Value::Object(map).to_string()
    }

    /// Deserialize from JSON string, starting from defaults
    pub fn from_json(json: &str, base: &ThemeColors) -> Self {
        let mut theme = base.clone();
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(json) {
            for (key, value) in &map {
                if let Some(hex) = value.as_str() {
                    if let Some(color) = ThemeColor::from_hex(hex) {
                        theme.set_var(key, color);
                    }
                }
            }
        }
        theme
    }
}

/// Container for all themes loaded from the theme file
#[derive(Clone, Debug)]
pub struct ThemeFile {
    pub themes: HashMap<String, ThemeColors>,
}

impl ThemeFile {
    /// Create a new ThemeFile with just the built-in dark and light defaults
    pub fn with_defaults() -> Self {
        let mut themes = HashMap::new();
        themes.insert("dark".to_string(), ThemeColors::dark_default());
        themes.insert("light".to_string(), ThemeColors::light_default());
        Self { themes }
    }

    /// Load themes from a file path, merging with defaults.
    /// Missing variables fall back to the built-in defaults.
    /// Returns with_defaults() if the file doesn't exist or can't be parsed.
    pub fn load(path: &Path) -> Self {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::with_defaults(),
        };
        Self::parse(&content)
    }

    /// Parse theme file content
    pub fn parse(content: &str) -> Self {
        let mut file = Self::with_defaults();
        let mut current_theme: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Section header: [theme:name]
            if line.starts_with('[') && line.ends_with(']') {
                let inner = &line[1..line.len() - 1];
                if let Some(name) = inner.strip_prefix("theme:") {
                    let name = name.trim().to_string();
                    // If this theme doesn't exist yet, start from dark defaults
                    if !file.themes.contains_key(&name) {
                        file.themes.insert(name.clone(), ThemeColors::dark_default());
                    }
                    current_theme = Some(name);
                }
                continue;
            }

            // Key = value pair
            if let Some(ref theme_name) = current_theme {
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();
                    // Strip inline comments: find second # (first # is hex prefix)
                    let value = if let Some(rest) = value.strip_prefix('#') {
                        // Skip the leading #, find next # for comment
                        if let Some(pos) = rest.find('#') {
                            value[..1 + pos].trim()
                        } else {
                            value
                        }
                    } else {
                        value
                    };
                    if let Some(color) = ThemeColor::from_hex(value) {
                        if let Some(theme) = file.themes.get_mut(theme_name) {
                            theme.set_var(key, color);
                        }
                    }
                }
            }
        }

        file
    }

    /// Get a theme by name, falling back to dark default
    pub fn get(&self, name: &str) -> &ThemeColors {
        self.themes.get(name).unwrap_or_else(|| {
            self.themes.get("dark").expect("dark theme must exist")
        })
    }

    /// Serialize all themes as JSON: {"dark": {...}, "light": {...}, ...}
    pub fn to_json_all(&self) -> String {
        let mut map = serde_json::Map::new();
        for (name, colors) in &self.themes {
            if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(&colors.to_json()) {
                map.insert(name.clone(), serde_json::Value::Object(obj));
            }
        }
        serde_json::Value::Object(map).to_string()
    }

    /// Generate complete .ini file content from current state
    pub fn generate_file_content(&self) -> String {
        let mut s = String::new();
        s.push_str("# Clay Theme Configuration\n");
        s.push_str("# Colors are #RRGGBB hex values. Lines starting with # are comments.\n");
        s.push_str("# Edit colors and restart Clay (or /reload) to apply changes.\n");
        s.push_str("# Delete this file to regenerate defaults.\n");

        let mut names: Vec<&String> = self.themes.keys().collect();
        names.sort();
        for name in names {
            s.push_str(&format!("\n[theme:{}]\n", name));
            s.push_str(&self.themes[name].to_theme_file_section());
        }
        s
    }

    /// Add or update a theme
    pub fn set_theme(&mut self, name: &str, colors: ThemeColors) {
        self.themes.insert(name.to_string(), colors);
    }

    /// Remove a theme (refuses if it's the last one). Returns true if removed.
    pub fn remove_theme(&mut self, name: &str) -> bool {
        if self.themes.len() <= 1 {
            return false;
        }
        self.themes.remove(name).is_some()
    }

    /// Generate a complete default theme file with comments
    pub fn generate_default_file() -> String {
        let mut s = String::new();
        s.push_str("# Clay Theme Configuration\n");
        s.push_str("# Colors are #RRGGBB hex values. Lines starting with # are comments.\n");
        s.push_str("# Edit colors and restart Clay (or /reload) to apply changes.\n");
        s.push_str("# Delete this file to regenerate defaults.\n");
        s.push_str("\n[theme:dark]\n");
        s.push_str(&ThemeColors::dark_default().to_theme_file_section());
        s.push_str("\n[theme:light]\n");
        s.push_str(&ThemeColors::light_default().to_theme_file_section());
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_color_from_hex() {
        assert_eq!(ThemeColor::from_hex("#ff0000"), Some(ThemeColor::new(255, 0, 0)));
        assert_eq!(ThemeColor::from_hex("00ff00"), Some(ThemeColor::new(0, 255, 0)));
        assert_eq!(ThemeColor::from_hex("#131926"), Some(ThemeColor::new(19, 25, 38)));
        assert_eq!(ThemeColor::from_hex("invalid"), None);
        assert_eq!(ThemeColor::from_hex("#fff"), None); // too short
        assert_eq!(ThemeColor::from_hex(""), None);
    }

    #[test]
    fn test_theme_color_to_css() {
        assert_eq!(ThemeColor::new(19, 25, 38).to_css(), "#131926");
        assert_eq!(ThemeColor::new(255, 0, 0).to_css(), "#ff0000");
        assert_eq!(ThemeColor::new(0, 0, 0).to_css(), "#000000");
    }

    #[test]
    fn test_theme_color_to_ratatui() {
        let c = ThemeColor::new(19, 25, 38);
        assert_eq!(c.to_ratatui(), ratatui::style::Color::Rgb(19, 25, 38));
    }

    #[test]
    fn test_dark_defaults_exist() {
        let dark = ThemeColors::dark_default();
        assert_eq!(dark.bg, ThemeColor::new(0x12, 0x12, 0x12));
        assert_eq!(dark.fg, ThemeColor::new(0xe8, 0xe4, 0xec));
        assert_eq!(dark.accent, ThemeColor::new(0x26, 0x57, 0xba));
        assert_eq!(dark.ansi[0], ThemeColor::new(0, 0, 0));
        assert_eq!(dark.ansi[15], ThemeColor::new(255, 255, 255));
    }

    #[test]
    fn test_light_defaults_exist() {
        let light = ThemeColors::light_default();
        assert_eq!(light.bg, ThemeColor::new(232, 232, 232));
        assert_eq!(light.fg, ThemeColor::new(29, 29, 31));
        assert_eq!(light.ansi[3], ThemeColor::new(128, 64, 0)); // darker yellow
        assert_eq!(light.ansi[15], ThemeColor::new(40, 40, 40)); // near black
    }

    #[test]
    fn test_parse_theme_file() {
        let content = r#"
# Test theme file
[theme:dark]
bg = #ff0000
fg = #00ff00

[theme:light]
bg = #0000ff

[theme:custom]
bg = #112233
accent = #aabbcc
ansi.0 = #111111
ansi.15 = #eeeeee
"#;
        let file = ThemeFile::parse(content);

        // Dark theme should have overridden bg and fg
        let dark = file.get("dark");
        assert_eq!(dark.bg, ThemeColor::new(255, 0, 0));
        assert_eq!(dark.fg, ThemeColor::new(0, 255, 0));
        // accent should still be default since not overridden
        assert_eq!(dark.accent, ThemeColor::new(0x26, 0x57, 0xba));

        // Light theme should have overridden bg
        let light = file.get("light");
        assert_eq!(light.bg, ThemeColor::new(0, 0, 255));
        // fg should still be light default
        assert_eq!(light.fg, ThemeColor::new(29, 29, 31));

        // Custom theme should exist, starting from dark defaults
        let custom = file.get("custom");
        assert_eq!(custom.bg, ThemeColor::new(0x11, 0x22, 0x33));
        assert_eq!(custom.accent, ThemeColor::new(0xaa, 0xbb, 0xcc));
        assert_eq!(custom.ansi[0], ThemeColor::new(0x11, 0x11, 0x11));
        assert_eq!(custom.ansi[15], ThemeColor::new(0xee, 0xee, 0xee));
        // Non-overridden ansi colors should be dark defaults
        assert_eq!(custom.ansi[1], ThemeColor::new(170, 0, 0));
    }

    #[test]
    fn test_parse_with_inline_comments() {
        let content = "[theme:dark]\nbg = #aabbcc  # main background\n";
        let file = ThemeFile::parse(content);
        let dark = file.get("dark");
        assert_eq!(dark.bg, ThemeColor::new(0xaa, 0xbb, 0xcc));
    }

    #[test]
    fn test_parse_empty_file() {
        let file = ThemeFile::parse("");
        // Should still have defaults
        let dark = file.get("dark");
        assert_eq!(dark.bg, ThemeColor::new(0x12, 0x12, 0x12));
    }

    #[test]
    fn test_parse_invalid_colors_ignored() {
        let content = "[theme:dark]\nbg = not_a_color\nfg = #00ff00\n";
        let file = ThemeFile::parse(content);
        let dark = file.get("dark");
        // bg should be default (invalid color ignored)
        assert_eq!(dark.bg, ThemeColor::new(0x12, 0x12, 0x12));
        // fg should be overridden
        assert_eq!(dark.fg, ThemeColor::new(0, 255, 0));
    }

    #[test]
    fn test_get_nonexistent_theme_falls_back() {
        let file = ThemeFile::with_defaults();
        let theme = file.get("nonexistent");
        // Should fall back to dark
        assert_eq!(theme.bg, ThemeColor::new(0x12, 0x12, 0x12));
    }

    #[test]
    fn test_set_var_dotted_keys() {
        let mut theme = ThemeColors::dark_default();
        assert!(theme.set_var("status_bar.bg", ThemeColor::new(1, 2, 3)));
        assert_eq!(theme.status_bar_bg, ThemeColor::new(1, 2, 3));
        assert!(theme.set_var("selection.bg", ThemeColor::new(4, 5, 6)));
        assert_eq!(theme.selection_bg, ThemeColor::new(4, 5, 6));
        assert!(theme.set_var("button.selected_bg", ThemeColor::new(7, 8, 9)));
        assert_eq!(theme.button_selected_bg, ThemeColor::new(7, 8, 9));
        assert!(theme.set_var("ansi.5", ThemeColor::new(10, 11, 12)));
        assert_eq!(theme.ansi[5], ThemeColor::new(10, 11, 12));
        // Invalid keys
        assert!(!theme.set_var("ansi.16", ThemeColor::new(0, 0, 0)));
        assert!(!theme.set_var("nonexistent", ThemeColor::new(0, 0, 0)));
    }

    #[test]
    fn test_generate_default_file() {
        let content = ThemeFile::generate_default_file();
        assert!(content.contains("[theme:dark]"));
        assert!(content.contains("[theme:light]"));
        assert!(content.contains("bg = #121212"));
        assert!(content.contains("ansi.0 = #000000"));
        // Should be re-parseable
        let file = ThemeFile::parse(&content);
        let dark = file.get("dark");
        assert_eq!(dark.bg, ThemeColor::new(0x12, 0x12, 0x12));
    }

    #[test]
    fn test_to_css_vars() {
        let theme = ThemeColors::dark_default();
        let css = theme.to_css_vars();
        assert!(css.contains("--theme-bg: #121212;"));
        assert!(css.contains("--theme-fg: #e8e4ec;"));
        assert!(css.contains("--theme-ansi-0: #000000;"));
        assert!(css.contains("--theme-ansi-15: #ffffff;"));
    }

    #[test]
    fn test_roundtrip_theme_file() {
        let original = ThemeColors::dark_default();
        let content = format!("[theme:test]\n{}", original.to_theme_file_section());
        let file = ThemeFile::parse(&content);
        let parsed = file.get("test");

        assert_eq!(parsed.bg, original.bg);
        assert_eq!(parsed.fg, original.fg);
        assert_eq!(parsed.accent, original.accent);
        assert_eq!(parsed.error, original.error);
        assert_eq!(parsed.ansi[0], original.ansi[0]);
        assert_eq!(parsed.ansi[15], original.ansi[15]);
        assert_eq!(parsed.status_bar_bg, original.status_bar_bg);
        assert_eq!(parsed.button_selected_bg, original.button_selected_bg);
    }
}
