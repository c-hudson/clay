//! Configurable keyboard bindings with TinyFugue defaults.
//!
//! Maps canonical key names (e.g. "^A", "Esc-b", "F1", "PageUp") to action IDs
//! (e.g. "cursor_home", "cursor_word_left", "help", "scroll_page_up").
//!
//! The binding system has two layers:
//! 1. Action bindings (this module) - maps keys to built-in UI actions
//! 2. TF /bind bindings (tf::hooks) - maps keys to TF commands (checked first)

use std::collections::HashMap;
use std::io;
use std::path::Path;

/// All known action IDs with metadata for the web editor.
pub struct ActionInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
}

/// Complete list of bindable actions.
pub const ACTIONS: &[ActionInfo] = &[
    // Cursor Movement
    ActionInfo { id: "cursor_left", name: "Cursor Left", category: "Cursor" },
    ActionInfo { id: "cursor_right", name: "Cursor Right", category: "Cursor" },
    ActionInfo { id: "cursor_word_left", name: "Word Left", category: "Cursor" },
    ActionInfo { id: "cursor_word_right", name: "Word Right", category: "Cursor" },
    ActionInfo { id: "cursor_home", name: "Home", category: "Cursor" },
    ActionInfo { id: "cursor_end", name: "End", category: "Cursor" },
    ActionInfo { id: "cursor_up", name: "Cursor Up", category: "Cursor" },
    ActionInfo { id: "cursor_down", name: "Cursor Down", category: "Cursor" },

    // Editing
    ActionInfo { id: "delete_backward", name: "Delete Backward", category: "Editing" },
    ActionInfo { id: "delete_forward", name: "Delete Forward", category: "Editing" },
    ActionInfo { id: "delete_word_backward", name: "Delete Word Backward", category: "Editing" },
    ActionInfo { id: "delete_word_forward", name: "Delete Word Forward", category: "Editing" },
    ActionInfo { id: "delete_word_backward_punct", name: "Delete Word Back (Punct)", category: "Editing" },
    ActionInfo { id: "kill_to_end", name: "Kill to End", category: "Editing" },
    ActionInfo { id: "clear_line", name: "Clear Line", category: "Editing" },
    ActionInfo { id: "transpose_chars", name: "Transpose Chars", category: "Editing" },
    ActionInfo { id: "literal_next", name: "Literal Next", category: "Editing" },
    ActionInfo { id: "capitalize_word", name: "Capitalize Word", category: "Editing" },
    ActionInfo { id: "lowercase_word", name: "Lowercase Word", category: "Editing" },
    ActionInfo { id: "uppercase_word", name: "Uppercase Word", category: "Editing" },
    ActionInfo { id: "collapse_spaces", name: "Collapse Spaces", category: "Editing" },
    ActionInfo { id: "goto_matching_bracket", name: "Goto Matching Bracket", category: "Editing" },
    ActionInfo { id: "insert_last_arg", name: "Insert Last Arg", category: "Editing" },
    ActionInfo { id: "yank", name: "Yank (Paste Kill Ring)", category: "Editing" },

    // History
    ActionInfo { id: "history_prev", name: "History Previous", category: "History" },
    ActionInfo { id: "history_next", name: "History Next", category: "History" },
    ActionInfo { id: "history_search_backward", name: "History Search Back", category: "History" },
    ActionInfo { id: "history_search_forward", name: "History Search Forward", category: "History" },

    // Scrollback
    ActionInfo { id: "scroll_page_up", name: "Page Up", category: "Scrollback" },
    ActionInfo { id: "scroll_page_down", name: "Page Down", category: "Scrollback" },
    ActionInfo { id: "scroll_half_page", name: "Half Page Scroll", category: "Scrollback" },
    ActionInfo { id: "flush_output", name: "Flush Output", category: "Scrollback" },
    ActionInfo { id: "selective_flush", name: "Selective Flush", category: "Scrollback" },
    ActionInfo { id: "tab_key", name: "Tab Key", category: "Scrollback" },

    // World
    ActionInfo { id: "world_next", name: "Next Active World", category: "World" },
    ActionInfo { id: "world_prev", name: "Previous Active World", category: "World" },
    ActionInfo { id: "world_all_next", name: "Next World (All)", category: "World" },
    ActionInfo { id: "world_all_prev", name: "Previous World (All)", category: "World" },
    ActionInfo { id: "world_activity", name: "World With Activity", category: "World" },
    ActionInfo { id: "world_previous", name: "Switch to Previous", category: "World" },
    ActionInfo { id: "world_forward", name: "Switch Forward", category: "World" },

    // System
    ActionInfo { id: "help", name: "Help", category: "System" },
    ActionInfo { id: "redraw", name: "Redraw Screen", category: "System" },
    ActionInfo { id: "reload", name: "Reload", category: "System" },
    ActionInfo { id: "quit", name: "Quit", category: "System" },
    ActionInfo { id: "suspend", name: "Suspend", category: "System" },
    ActionInfo { id: "bell", name: "Bell", category: "System" },
    ActionInfo { id: "spell_check", name: "Spell Check", category: "System" },

    // Clay Extensions
    ActionInfo { id: "toggle_tags", name: "Toggle Tags (F2)", category: "Clay" },
    ActionInfo { id: "filter_popup", name: "Find (F4)", category: "Clay" },
    ActionInfo { id: "search_popup", name: "Search History (F5)", category: "Clay" },
    ActionInfo { id: "toggle_action_highlight", name: "Toggle Highlights (F8)", category: "Clay" },
    ActionInfo { id: "toggle_gmcp_media", name: "Toggle GMCP Media (F9)", category: "Clay" },
    ActionInfo { id: "input_grow", name: "Grow Input Area", category: "Clay" },
    ActionInfo { id: "input_shrink", name: "Shrink Input Area", category: "Clay" },
];

/// Keyboard binding map: canonical key name -> action ID.
#[derive(Clone)]
pub struct KeyBindings {
    pub bindings: HashMap<String, String>,
}

impl KeyBindings {
    /// Create bindings with TinyFugue defaults.
    pub fn tf_defaults() -> Self {
        let mut b = HashMap::new();

        // Cursor Movement (TF defaults: ^B/^F = char, Esc-b/Esc-f = word)
        b.insert("^A".into(), "cursor_home".into());
        b.insert("^B".into(), "cursor_left".into());
        b.insert("^E".into(), "cursor_end".into());
        b.insert("^F".into(), "cursor_right".into());
        b.insert("Left".into(), "cursor_left".into());
        b.insert("Right".into(), "cursor_right".into());
        b.insert("Home".into(), "cursor_home".into());
        b.insert("End".into(), "cursor_end".into());
        b.insert("Esc-b".into(), "cursor_word_left".into());
        b.insert("Esc-f".into(), "cursor_word_right".into());

        // Editing
        b.insert("Backspace".into(), "delete_backward".into());
        b.insert("Delete".into(), "delete_forward".into());
        b.insert("^D".into(), "delete_forward".into());
        b.insert("^K".into(), "kill_to_end".into());
        b.insert("^U".into(), "clear_line".into());
        b.insert("^W".into(), "delete_word_backward".into());
        b.insert("^T".into(), "transpose_chars".into());
        b.insert("^V".into(), "literal_next".into());
        b.insert("^Y".into(), "yank".into());
        b.insert("Esc-c".into(), "capitalize_word".into());
        b.insert("Esc-d".into(), "delete_word_forward".into());
        b.insert("Esc-l".into(), "lowercase_word".into());
        b.insert("Esc-u".into(), "uppercase_word".into());
        b.insert("Esc-Space".into(), "collapse_spaces".into());
        b.insert("Esc-Backspace".into(), "delete_word_backward_punct".into());
        b.insert("Esc--".into(), "goto_matching_bracket".into());
        b.insert("Esc-.".into(), "insert_last_arg".into());
        b.insert("Esc-_".into(), "insert_last_arg".into());

        // History (TF defaults: ^P/^N = history, Up/Down = cursor movement)
        b.insert("^P".into(), "history_prev".into());
        b.insert("^N".into(), "history_next".into());
        b.insert("Up".into(), "cursor_up".into());
        b.insert("Down".into(), "cursor_down".into());
        b.insert("Esc-p".into(), "history_search_backward".into());
        b.insert("Esc-n".into(), "history_search_forward".into());

        // Scrollback
        b.insert("PageUp".into(), "scroll_page_up".into());
        b.insert("PageDown".into(), "scroll_page_down".into());
        // Esc-v in TF toggles insert mode; not implemented in Clay
        b.insert("Esc-j".into(), "flush_output".into());
        b.insert("Esc-J".into(), "selective_flush".into());
        b.insert("Esc-h".into(), "scroll_half_page".into());
        b.insert("Tab".into(), "tab_key".into());

        // World (Clay additions - non-conflicting with TF)
        b.insert("Ctrl-Up".into(), "world_next".into());
        b.insert("Ctrl-Down".into(), "world_prev".into());
        b.insert("Shift-Up".into(), "world_all_next".into());
        b.insert("Shift-Down".into(), "world_all_prev".into());
        b.insert("Esc-w".into(), "world_activity".into());

        // System
        b.insert("F1".into(), "help".into());
        b.insert("^L".into(), "redraw".into());
        b.insert("^R".into(), "reload".into());
        b.insert("^G".into(), "bell".into());
        b.insert("^Z".into(), "suspend".into());
        b.insert("^Q".into(), "spell_check".into());

        // Clay Extensions
        b.insert("F2".into(), "toggle_tags".into());
        b.insert("F4".into(), "filter_popup".into());
        b.insert("F5".into(), "search_popup".into());
        b.insert("F8".into(), "toggle_action_highlight".into());
        b.insert("F9".into(), "toggle_gmcp_media".into());
        b.insert("Alt-Up".into(), "input_grow".into());
        b.insert("Alt-Down".into(), "input_shrink".into());

        Self { bindings: b }
    }

    /// Get the action bound to a key, if any.
    pub fn get_action(&self, key_name: &str) -> Option<&str> {
        self.bindings.get(key_name).map(|s| s.as_str())
    }

    /// Set a binding (key -> action).
    pub fn set_binding(&mut self, key: &str, action: &str) {
        self.bindings.insert(key.to_string(), action.to_string());
    }

    /// Remove a binding for a key.
    pub fn remove_binding(&mut self, key: &str) {
        self.bindings.remove(key);
    }

    /// Find all keys bound to a given action.
    pub fn keys_for_action(&self, action: &str) -> Vec<String> {
        self.bindings.iter()
            .filter(|(_, v)| v.as_str() == action)
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Serialize bindings to JSON.
    pub fn to_json(&self) -> String {
        let mut entries: Vec<(&String, &String)> = self.bindings.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        let mut json = String::from("{");
        for (i, (key, action)) in entries.iter().enumerate() {
            if i > 0 { json.push(','); }
            json.push('"');
            json.push_str(&key.replace('\\', "\\\\").replace('"', "\\\""));
            json.push_str("\":\"");
            json.push_str(&action.replace('\\', "\\\\").replace('"', "\\\""));
            json.push('"');
        }
        json.push('}');
        json
    }

    /// Deserialize bindings from JSON.
    pub fn from_json(json: &str) -> Self {
        let mut bindings = HashMap::new();
        // Simple JSON object parser for {"key":"value",...}
        let trimmed = json.trim();
        if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
            return Self { bindings };
        }
        let inner = &trimmed[1..trimmed.len()-1];
        let mut chars = inner.chars().peekable();

        loop {
            // Skip whitespace
            while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
                chars.next();
            }
            if chars.peek().is_none() { break; }

            // Parse key
            if let Some(key) = parse_json_string(&mut chars) {
                // Skip colon
                while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ':') {
                    chars.next();
                }
                // Parse value
                if let Some(value) = parse_json_string(&mut chars) {
                    bindings.insert(key, value);
                }
            } else {
                break;
            }
        }

        Self { bindings }
    }

    /// Load from INI file, merging with TF defaults.
    /// Accepts files with or without a [bindings] section header.
    pub fn load(path: &Path) -> Self {
        let mut kb = Self::tf_defaults();

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return kb,
        };

        // If file has no [bindings] section at all, treat all lines as bindings
        let has_section = content.lines().any(|l| l.trim() == "[bindings]");
        let mut in_section = !has_section; // start active if no sections exist
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[bindings]" {
                in_section = true;
                continue;
            }
            if line.starts_with('[') {
                in_section = false;
                continue;
            }
            if !in_section {
                continue;
            }
            if let Some((key, action)) = line.split_once('=') {
                let key = key.trim().to_string();
                let action = action.trim().to_string();
                if action == "UNBOUND" {
                    // Explicit unbind: remove default binding
                    kb.bindings.remove(&key);
                } else if !key.is_empty() && !action.is_empty() {
                    kb.bindings.insert(key, action);
                }
            }
        }

        kb
    }

    /// Save to INI file. Only saves bindings that differ from TF defaults.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let defaults = Self::tf_defaults();
        let mut lines = vec![
            "# Clay Keyboard Bindings".to_string(),
            "# Format: key = action".to_string(),
            "# Only modified bindings are saved (defaults are built-in)".to_string(),
            "# Use UNBOUND to remove a default binding".to_string(),
            String::new(),
            "[bindings]".to_string(),
        ];

        // Find bindings that differ from defaults
        let mut entries: Vec<(&String, &String)> = self.bindings.iter()
            .filter(|(key, action)| {
                defaults.bindings.get(*key).map(|d| d != *action).unwrap_or(true)
            })
            .collect();
        entries.sort_by_key(|(k, _)| k.as_str());

        for (key, action) in &entries {
            lines.push(format!("{} = {}", key, action));
        }

        // Find default bindings that were removed
        let mut removed: Vec<&String> = defaults.bindings.keys()
            .filter(|key| !self.bindings.contains_key(*key))
            .collect();
        removed.sort();

        for key in &removed {
            lines.push(format!("{} = UNBOUND", key));
        }

        std::fs::write(path, lines.join("\n") + "\n")
    }

    /// Serialize action metadata to JSON for the web editor.
    pub fn actions_json() -> String {
        let mut json = String::from("[");
        for (i, action) in ACTIONS.iter().enumerate() {
            if i > 0 { json.push(','); }
            json.push_str(&format!(
                "{{\"id\":\"{}\",\"name\":\"{}\",\"category\":\"{}\"}}",
                action.id, action.name, action.category
            ));
        }
        json.push(']');
        json
    }
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self::tf_defaults()
    }
}

/// Convert a crossterm KeyEvent to canonical key name.
///
/// Returns None if the key event doesn't map to a bindable name
/// (e.g. bare modifier keys, unknown codes).
pub fn key_event_to_name(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Option<String> {
    use crossterm::event::{KeyCode, KeyModifiers};

    match code {
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(format!("^{}", c.to_ascii_uppercase()))
        }
        // Alt+char is handled via recent_escape pattern, not here
        // (crossterm sends Alt modifier for some terminals though)
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::ALT) => {
            Some(format!("Esc-{}", c))
        }
        KeyCode::F(n) => Some(format!("F{}", n)),
        KeyCode::Up => {
            if modifiers.contains(KeyModifiers::SHIFT) {
                Some("Shift-Up".into())
            } else if modifiers.contains(KeyModifiers::CONTROL) {
                Some("Ctrl-Up".into())
            } else if modifiers.contains(KeyModifiers::ALT) {
                Some("Alt-Up".into())
            } else {
                Some("Up".into())
            }
        }
        KeyCode::Down => {
            if modifiers.contains(KeyModifiers::SHIFT) {
                Some("Shift-Down".into())
            } else if modifiers.contains(KeyModifiers::CONTROL) {
                Some("Ctrl-Down".into())
            } else if modifiers.contains(KeyModifiers::ALT) {
                Some("Alt-Down".into())
            } else {
                Some("Down".into())
            }
        }
        KeyCode::Left => {
            if modifiers.contains(KeyModifiers::SHIFT) {
                Some("Shift-Left".into())
            } else if modifiers.contains(KeyModifiers::CONTROL) {
                Some("Ctrl-Left".into())
            } else if modifiers.contains(KeyModifiers::ALT) {
                Some("Alt-Left".into())
            } else {
                Some("Left".into())
            }
        }
        KeyCode::Right => {
            if modifiers.contains(KeyModifiers::SHIFT) {
                Some("Shift-Right".into())
            } else if modifiers.contains(KeyModifiers::CONTROL) {
                Some("Ctrl-Right".into())
            } else if modifiers.contains(KeyModifiers::ALT) {
                Some("Alt-Right".into())
            } else {
                Some("Right".into())
            }
        }
        KeyCode::PageUp => Some("PageUp".into()),
        KeyCode::PageDown => Some("PageDown".into()),
        KeyCode::Home => Some("Home".into()),
        KeyCode::End => Some("End".into()),
        KeyCode::Insert => Some("Insert".into()),
        KeyCode::Delete => Some("Delete".into()),
        KeyCode::Backspace => {
            if modifiers.contains(KeyModifiers::ALT) {
                Some("Esc-Backspace".into())
            } else {
                Some("Backspace".into())
            }
        }
        KeyCode::Tab => Some("Tab".into()),
        KeyCode::Enter => Some("Enter".into()),
        KeyCode::Esc => Some("Escape".into()),
        _ => None,
    }
}

/// Convert an Escape+key sequence to canonical "Esc-X" name.
/// Called when recent_escape is true and a key arrives.
pub fn escape_key_to_name(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Option<String> {
    use crossterm::event::KeyCode;
    let _ = modifiers; // Escape sequences don't combine with other modifiers

    match code {
        KeyCode::Char(' ') => Some("Esc-Space".into()),
        KeyCode::Char(c) => {
            // Preserve case: Esc-j vs Esc-J
            Some(format!("Esc-{}", c))
        }
        KeyCode::Backspace => Some("Esc-Backspace".into()),
        _ => None,
    }
}

/// Helper: parse a JSON string value from a char iterator.
fn parse_json_string(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
    // Skip to opening quote
    while chars.peek().is_some_and(|c| *c != '"') {
        chars.next();
    }
    chars.next(); // consume opening "

    let mut s = String::new();
    loop {
        match chars.next() {
            Some('"') => return Some(s),
            Some('\\') => {
                match chars.next() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some(c) => { s.push('\\'); s.push(c); }
                    None => return Some(s),
                }
            }
            Some(c) => s.push(c),
            None => return Some(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tf_defaults() {
        let kb = KeyBindings::tf_defaults();
        assert_eq!(kb.get_action("^A"), Some("cursor_home"));
        assert_eq!(kb.get_action("Up"), Some("cursor_up"));
        assert_eq!(kb.get_action("^B"), Some("cursor_left"));
        assert_eq!(kb.get_action("^F"), Some("cursor_right"));
        assert_eq!(kb.get_action("Esc-b"), Some("cursor_word_left"));
        assert_eq!(kb.get_action("Esc-f"), Some("cursor_word_right"));
        assert_eq!(kb.get_action("^Y"), Some("yank"));
        assert_eq!(kb.get_action("F1"), Some("help"));
    }

    #[test]
    fn test_json_roundtrip() {
        let kb = KeyBindings::tf_defaults();
        let json = kb.to_json();
        let kb2 = KeyBindings::from_json(&json);
        assert_eq!(kb.bindings.len(), kb2.bindings.len());
        for (key, action) in &kb.bindings {
            assert_eq!(kb2.get_action(key), Some(action.as_str()));
        }
    }

    #[test]
    fn test_save_load_only_diffs() {
        let mut kb = KeyBindings::tf_defaults();
        // Modify one binding
        kb.set_binding("Up", "world_next");
        // Remove one binding
        kb.remove_binding("^Z");

        let dir = std::env::temp_dir().join("clay_test_keybindings");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.key.dat");
        kb.save(&path).unwrap();

        let loaded = KeyBindings::load(&path);
        assert_eq!(loaded.get_action("Up"), Some("world_next"));
        assert_eq!(loaded.get_action("^Z"), None);
        // Default bindings should still be present
        assert_eq!(loaded.get_action("^A"), Some("cursor_home"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_keys_for_action() {
        let kb = KeyBindings::tf_defaults();
        let keys = kb.keys_for_action("cursor_left");
        assert!(keys.contains(&"Left".to_string()));
        assert!(keys.contains(&"Left".to_string()));
    }

    #[test]
    fn test_key_event_to_name() {
        use crossterm::event::{KeyCode, KeyModifiers};
        assert_eq!(key_event_to_name(KeyCode::Char('a'), KeyModifiers::CONTROL), Some("^A".into()));
        assert_eq!(key_event_to_name(KeyCode::F(1), KeyModifiers::NONE), Some("F1".into()));
        assert_eq!(key_event_to_name(KeyCode::Up, KeyModifiers::SHIFT), Some("Shift-Up".into()));
        assert_eq!(key_event_to_name(KeyCode::Up, KeyModifiers::CONTROL), Some("Ctrl-Up".into()));
        assert_eq!(key_event_to_name(KeyCode::Up, KeyModifiers::NONE), Some("Up".into()));
        assert_eq!(key_event_to_name(KeyCode::PageUp, KeyModifiers::NONE), Some("PageUp".into()));
        assert_eq!(key_event_to_name(KeyCode::Backspace, KeyModifiers::NONE), Some("Backspace".into()));
        assert_eq!(key_event_to_name(KeyCode::Tab, KeyModifiers::NONE), Some("Tab".into()));
    }
}
