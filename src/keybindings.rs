//! Configurable keyboard bindings with TinyFugue defaults.
//!
//! Maps canonical key names (e.g. "^A", "Esc-b", "F1", "PageUp") to action IDs
//! (e.g. "cursor_home", "cursor_word_left", "help", "scroll_page_up"). The
//! canonical grammar itself - what a key name is allowed to look like, and
//! how to parse TF's raw spellings into it - lives in `crate::keynames`;
//! every insert and lookup in this module goes through `canonicalize` so a
//! `keybindings.dat` entry written in any accepted spelling ends up stored
//! (and found again) under the exact same key.
//!
//! The binding system has two layers:
//! 1. Action bindings (this module) - maps keys to built-in UI actions
//! 2. TF /bind bindings (tf::hooks) - maps keys to TF commands (checked first)

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::keynames::{self, KeySeq, KeyToken, Modifier, NamedKey};

/// Canonicalise a key name through the shared grammar (`crate::keynames`),
/// falling back to the original string unchanged if it doesn't parse - so an
/// already-broken or not-yet-supported spelling already sitting in a user's
/// `keybindings.dat` is preserved rather than silently dropped. Every
/// `KeyBindings` insert/lookup routes through this (plan finding A: this is
/// what makes `Esc-j` and `Alt-J` land on two distinct, stable keys instead
/// of colliding under an ad hoc one-way translation).
fn canonicalize(key: &str) -> String {
    if keynames::is_canonical(key) {
        return key.to_string();
    }
    keynames::parse_key_name(key)
        .map(|seq| seq.canonical())
        .unwrap_or_else(|_| key.to_string())
}

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
    // TF-parity plan Job 19 (P2.3): kill_to_start is TF's real ^U (`kb_backward_kill_line`) -
    // Clay's own ^U has always meant "clear the whole line" (still `clear_line`/TF `DLINE`).
    ActionInfo { id: "kill_to_start", name: "Kill to Start", category: "Editing" },
    // TF `kb_expand_line` (`/eval /grab $(/recall -i 1)`): substitute %var/$[]/$() in the
    // current input line in place.
    ActionInfo { id: "expand_line", name: "Expand Line", category: "Editing" },
    // Tab completion, factored out of Tab's own more-mode-priority handling so it can also
    // be bound standalone (Job 22 defaults this to Esc-Tab).
    ActionInfo { id: "completion", name: "Command Completion", category: "Editing" },

    // History
    ActionInfo { id: "history_prev", name: "History Previous", category: "History" },
    ActionInfo { id: "history_next", name: "History Next", category: "History" },
    ActionInfo { id: "history_search_backward", name: "History Search Back", category: "History" },
    ActionInfo { id: "history_search_forward", name: "History Search Forward", category: "History" },
    // TF RECALLBEG/RECALLEND: jump straight to the oldest/newest history entry.
    ActionInfo { id: "recall_begin", name: "Recall First", category: "History" },
    ActionInfo { id: "recall_end", name: "Recall Last", category: "History" },

    // Scrollback
    ActionInfo { id: "scroll_page_up", name: "Page Up", category: "Scrollback" },
    ActionInfo { id: "scroll_page_down", name: "Page Down", category: "Scrollback" },
    // TF PAGEBACK: alias of scroll_page_up (same behavior, separate id so /dokey PAGEBACK
    // and a `key_pageback`-style binding have their own name to bind).
    ActionInfo { id: "scroll_page_back", name: "Page Back", category: "Scrollback" },
    ActionInfo { id: "scroll_half_page", name: "Half Page Forward", category: "Scrollback" },
    ActionInfo { id: "scroll_half_page_back", name: "Half Page Back", category: "Scrollback" },
    // TF LINE/LINEBACK: scroll by exactly one output line.
    ActionInfo { id: "scroll_line_forward", name: "Line Forward", category: "Scrollback" },
    ActionInfo { id: "scroll_line_back", name: "Line Back", category: "Scrollback" },
    ActionInfo { id: "flush_output", name: "Flush Output", category: "Scrollback" },
    ActionInfo { id: "selective_flush", name: "Selective Flush", category: "Scrollback" },
    ActionInfo { id: "tab_key", name: "Tab Key", category: "Scrollback" },
    // TF CLEAR: empty the view without dropping any lines (scrollback refills it).
    ActionInfo { id: "clear_screen", name: "Clear Screen", category: "Scrollback" },
    // TF PAUSE: pause the current world so new output queues as pending (more-mode).
    ActionInfo { id: "pause_output", name: "Pause Output", category: "Scrollback" },
    // TF-parity plan Job 22a (P2.6, ruling table `Esc-L`): toggle the F4 filter popup
    // between "no limit" and "the last limit applied" (`/unlimit`/`/relimit`).
    ActionInfo { id: "toggle_limit", name: "Toggle Limit", category: "Scrollback" },

    // World
    ActionInfo { id: "world_next", name: "Next Active World", category: "World" },
    ActionInfo { id: "world_prev", name: "Previous Active World", category: "World" },
    ActionInfo { id: "world_all_next", name: "Next World (All)", category: "World" },
    ActionInfo { id: "world_all_prev", name: "Previous World (All)", category: "World" },
    ActionInfo { id: "world_activity", name: "World With Activity", category: "World" },
    ActionInfo { id: "world_previous", name: "Switch to Previous", category: "World" },
    ActionInfo { id: "world_forward", name: "Switch Forward", category: "World" },
    ActionInfo { id: "recent_worlds", name: "Recent Worlds", category: "World" },
    // TF SOCKETB/SOCKETF (`/fg -<`/`/fg ->`): cycle CONNECTED worlds only, in world-list
    // order - distinct from world_prev/world_next, which cycle "active" worlds (connected,
    // or with unseen/pending output) using world_switch_mode's alphabetical/unseen-first rules.
    ActionInfo { id: "world_socket_prev", name: "Previous Connected World", category: "World" },
    ActionInfo { id: "world_socket_next", name: "Next Connected World", category: "World" },
    // TF /bg (= /fg -n): background every world / no foreground. A no-op in Clay's
    // single-pane console (see `App::cycle_connected_world`'s doc comment and `cmd_fg`'s own
    // -n handling in tf/parser.rs) - kept as a real action id for binding/scripting parity.
    ActionInfo { id: "bg_all_worlds", name: "Background All Worlds", category: "World" },

    // System
    ActionInfo { id: "help", name: "Help", category: "System" },
    ActionInfo { id: "redraw", name: "Redraw Screen", category: "System" },
    ActionInfo { id: "reload", name: "Reload", category: "System" },
    ActionInfo { id: "quit", name: "Quit", category: "System" },
    ActionInfo { id: "suspend", name: "Suspend", category: "System" },
    ActionInfo { id: "bell", name: "Bell", category: "System" },
    ActionInfo { id: "spell_check", name: "Spell Check", category: "System" },
    // TF REFRESH: plain repaint, no filtering (Job 22 ruling: ^L becomes this).
    ActionInfo { id: "refresh_line", name: "Refresh Screen", category: "System" },
    // Clay's own historical ^L behavior: repaint AND drop client-generated lines
    // (becomes unbound by default in Job 22, still available to rebind).
    ActionInfo { id: "redraw_server_only", name: "Redraw (Server Lines Only)", category: "System" },
    // TF-parity plan Job 22a (`^X^V`): same text as a typed `/version`.
    ActionInfo { id: "show_version", name: "Show Version", category: "System" },

    // Clay Extensions
    ActionInfo { id: "toggle_tags", name: "Toggle Tags (F2)", category: "Clay" },
    ActionInfo { id: "filter_popup", name: "Find (F4)", category: "Clay" },
    ActionInfo { id: "search_popup", name: "Search History (F5)", category: "Clay" },
    ActionInfo { id: "toggle_action_highlight", name: "Toggle Highlights (F8)", category: "Clay" },
    ActionInfo { id: "toggle_gmcp_media", name: "Toggle GMCP Media (F9)", category: "Clay" },
    ActionInfo { id: "input_grow", name: "Grow Input Area", category: "Clay" },
    ActionInfo { id: "input_shrink", name: "Shrink Input Area", category: "Clay" },

    // TF-parity plan Job 20 (P2.4): overwrite/insert toggle (TF `Insert` key / `Esc-v`,
    // `/@test insert := !insert`). Default binding added in Job 22 (three-UI rule).
    ActionInfo { id: "toggle_insert", name: "Toggle Insert/Overwrite", category: "Editing" },

    // Numeric prefix (TF `%kbnum`, tf-help #kbnum): `Esc-0`..`Esc-9` build up a repeat
    // count/motion magnitude one digit at a time (`InputArea::kbnum_digit`), `Esc--`
    // starts a negative one (`/set kbnum=-` in kbbind.tf). Default bindings added in
    // Job 22 (three-UI rule) - `Esc--` was already `goto_matching_bracket` pre-parity and
    // moves there per the plan's ruling table (finding A).
    ActionInfo { id: "kbnum_0", name: "Numeric Prefix 0", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_1", name: "Numeric Prefix 1", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_2", name: "Numeric Prefix 2", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_3", name: "Numeric Prefix 3", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_4", name: "Numeric Prefix 4", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_5", name: "Numeric Prefix 5", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_6", name: "Numeric Prefix 6", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_7", name: "Numeric Prefix 7", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_8", name: "Numeric Prefix 8", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_9", name: "Numeric Prefix 9", category: "Numeric prefix" },
    ActionInfo { id: "kbnum_negative", name: "Numeric Prefix Negative", category: "Numeric prefix" },
];

/// Keyboard binding map: canonical key name -> action ID.
#[derive(Clone)]
pub struct KeyBindings {
    pub bindings: HashMap<String, String>,
}

impl KeyBindings {
    /// Create bindings with TinyFugue defaults (TF-parity plan Phase 2 step P2.6 - finding
    /// A's ruling table is the spec this implements exactly). Old name kept as an alias
    /// below since it's referenced throughout the codebase (daemon.rs, main.rs, tests);
    /// `defaults()` is the name to use in new code.
    pub fn defaults() -> Self {
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
        b.insert("Ctrl-Left".into(), "cursor_word_left".into());
        b.insert("Ctrl-Right".into(), "cursor_word_right".into());

        // Editing
        b.insert("Backspace".into(), "delete_backward".into());
        b.insert("Delete".into(), "delete_forward".into());
        b.insert("^D".into(), "delete_forward".into());
        b.insert("^K".into(), "kill_to_end".into());
        // TF's real ^U is "kill to start of line" (kill ring kept) - Clay's own historical
        // "clear whole line" meaning moves to no default key (still bindable as
        // `clear_line`, TF's own `/dokey DLINE`). Ruling table.
        b.insert("^U".into(), "kill_to_start".into());
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
        // Esc-= is TF's own "goto matching bracket" (Esc-- moves to kbnum_negative below,
        // matching TF - ruling table).
        b.insert("Esc-=".into(), "goto_matching_bracket".into());
        b.insert("Esc-.".into(), "insert_last_arg".into());
        b.insert("Esc-_".into(), "insert_last_arg".into());
        b.insert("Esc-^H".into(), "delete_word_backward_punct".into());
        b.insert("^X^?".into(), "delete_word_backward_punct".into());
        // TF `Insert`/`Esc-v`: overwrite/insert toggle (Job 20/P2.4).
        b.insert("Insert".into(), "toggle_insert".into());
        b.insert("Esc-v".into(), "toggle_insert".into());
        // TF kb_expand_line.
        b.insert("Esc-^E".into(), "expand_line".into());
        // Tab completion, standalone (Tab itself keeps its historical more-mode-priority
        // paging behavior - see `tab_key`/`perform_completion`'s own doc comments).
        b.insert("Esc-Tab".into(), "completion".into());

        // Numeric prefix (TF `%kbnum`, tf-help #kbnum): Esc-- starts a negative one,
        // Esc-0..Esc-9 build up the magnitude one digit at a time - ruling table (this is
        // where TF's own `Esc--`/`Esc-=` meanings land, swapped from Clay's pre-parity
        // defaults above).
        b.insert("Esc--".into(), "kbnum_negative".into());
        b.insert("Esc-0".into(), "kbnum_0".into());
        b.insert("Esc-1".into(), "kbnum_1".into());
        b.insert("Esc-2".into(), "kbnum_2".into());
        b.insert("Esc-3".into(), "kbnum_3".into());
        b.insert("Esc-4".into(), "kbnum_4".into());
        b.insert("Esc-5".into(), "kbnum_5".into());
        b.insert("Esc-6".into(), "kbnum_6".into());
        b.insert("Esc-7".into(), "kbnum_7".into());
        b.insert("Esc-8".into(), "kbnum_8".into());
        b.insert("Esc-9".into(), "kbnum_9".into());

        // History (TF defaults: ^P/^N = history, Up/Down = cursor movement). Ctrl-Up/Down
        // move here from world-switching (ruling table: TF wins, real TF's own Ctrl-Up/Down
        // recall history) - world switching moves to Esc-Left/Right and Esc-{/} below.
        b.insert("^P".into(), "history_prev".into());
        b.insert("^N".into(), "history_next".into());
        b.insert("Up".into(), "cursor_up".into());
        b.insert("Down".into(), "cursor_down".into());
        b.insert("Esc-p".into(), "history_search_backward".into());
        b.insert("Esc-n".into(), "history_search_forward".into());
        b.insert("Ctrl-Up".into(), "history_prev".into());
        b.insert("Ctrl-Down".into(), "history_next".into());
        // TF RECALLBEG/RECALLEND.
        b.insert("Ctrl-Home".into(), "recall_begin".into());
        b.insert("Ctrl-End".into(), "recall_end".into());
        b.insert("Esc-<".into(), "recall_begin".into());
        b.insert("Esc->".into(), "recall_end".into());

        // Scrollback
        b.insert("PageUp".into(), "scroll_page_up".into());
        b.insert("PageDown".into(), "scroll_page_down".into());
        b.insert("Esc-j".into(), "flush_output".into());
        b.insert("Esc-J".into(), "selective_flush".into());
        b.insert("Esc-h".into(), "scroll_half_page".into());
        b.insert("Tab".into(), "tab_key".into());
        b.insert("Ctrl-PageDown".into(), "flush_output".into());
        // TF LINE/LINEBACK, CLEAR, PAUSE.
        b.insert("Esc-^N".into(), "scroll_line_forward".into());
        b.insert("Esc-^P".into(), "scroll_line_back".into());
        b.insert("Esc-^L".into(), "clear_screen".into());
        b.insert("^S".into(), "pause_output".into());
        // TF's own `^X` prefix keymap (kbbind.tf): half-page/page scroll.
        b.insert("^X[".into(), "scroll_half_page_back".into());
        b.insert("^X]".into(), "scroll_half_page".into());
        b.insert("^X{".into(), "scroll_page_back".into());
        b.insert("^X}".into(), "scroll_page_down".into());

        // World. Esc-Left/Right and Esc-{/} take over world-switching from Ctrl-Up/Down
        // (ruling table). TF binds all four to SOCKETB/SOCKETF; Clay keeps Esc-Left/Right
        // as those (connected worlds only) and gives the redundant Esc-{/} pair to its own
        // world_prev/world_next (the alphabetical/unseen-first cycling from the "World
        // Switching" setting), so both styles keep a default key. Shift-Up/Down unchanged.
        b.insert("Shift-Up".into(), "world_all_next".into());
        b.insert("Shift-Down".into(), "world_all_prev".into());
        b.insert("Esc-w".into(), "world_activity".into());
        b.insert("Esc-Left".into(), "world_socket_prev".into());
        b.insert("Esc-Right".into(), "world_socket_next".into());
        b.insert("Esc-{".into(), "world_prev".into());
        b.insert("Esc-}".into(), "world_next".into());
        // TF /bg (= /fg -n).
        b.insert("^]".into(), "bg_all_worlds".into());

        // System. ^L is TF's real REFRESH (plain repaint) - Clay's own historical "repaint
        // AND drop client lines" meaning becomes `redraw_server_only`, unbound by default
        // (ruling table).
        b.insert("F1".into(), "help".into());
        b.insert("^L".into(), "refresh_line".into());
        b.insert("^R".into(), "reload".into());
        b.insert("^G".into(), "bell".into());
        b.insert("^Z".into(), "suspend".into());
        b.insert("^Q".into(), "spell_check".into());
        // TF's own `^X` prefix keymap continued: reload (hot reload, Clay's own reflex for
        // TF's ".tfrc" meaning) and Clay's own `show_version` addition.
        b.insert("^X^R".into(), "reload".into());
        b.insert("^X^V".into(), "show_version".into());
        // Clay's own toggle_limit convenience binding (no single TF dokey for this).
        b.insert("Esc-L".into(), "toggle_limit".into());

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

    /// Alias of [`Self::defaults`] kept for existing call sites (daemon.rs, main.rs, tests)
    /// - `defaults()` is the name new code should use.
    pub fn tf_defaults() -> Self {
        Self::defaults()
    }

    /// Get the action bound to a key, if any.
    pub fn get_action(&self, key_name: &str) -> Option<&str> {
        self.bindings.get(&canonicalize(key_name)).map(|s| s.as_str())
    }

    /// Set a binding (key -> action).
    pub fn set_binding(&mut self, key: &str, action: &str) {
        self.bindings.insert(canonicalize(key), action.to_string());
    }

    /// Remove a binding for a key.
    pub fn remove_binding(&mut self, key: &str) {
        self.bindings.remove(&canonicalize(key));
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
                    bindings.insert(canonicalize(&key), value);
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
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Self::defaults(),
        };
        Self::from_dat_string(&content)
    }

    /// Same parsing `load` does, from an in-memory string instead of a file. Used by `load`
    /// and by the `/import` merge path (a `keybindings_dat` string arrives over the wire,
    /// never touching disk until after the remote-wins merge is applied).
    pub fn from_dat_string(content: &str) -> Self {
        let mut kb = Self::defaults();

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
                let key = canonicalize(key.trim());
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
        std::fs::write(path, self.to_dat_string())
    }

    /// Merges another instance's *effective* binding set into `self`: every key `other` has
    /// an opinion on (its own customizations, plus whatever TF defaults it didn't explicitly
    /// UNBOUND — `from_dat_string` starts from `tf_defaults()`) overwrites `self`'s binding
    /// for that key; a key only `self` has customized — one `other` never touched — is left
    /// alone. Note: if `other` explicitly UNBOUND'd a default, that removal doesn't
    /// propagate here (the key is simply absent from `other.bindings`, indistinguishable
    /// from "other never considered this key") — an accepted gap for now, not a full
    /// diff-aware merge. Used by `/import`'s `merge_keybindings_dat` (persistence.rs; plan
    /// `i-d-like-to-make-snuggly-rain.md`).
    pub fn merge_remote(&mut self, remote_keybindings_dat: &str) {
        let remote = Self::from_dat_string(remote_keybindings_dat);
        for (key, action) in remote.bindings {
            self.bindings.insert(key, action);
        }
    }

    /// Same INI text `save` writes to `keybindings.dat`, as a `String`. Only bindings that
    /// differ from TF defaults are included. Used by `save` and by the `/import` export
    /// path (`RequestSettingsExport` handler) to send this instance's keybindings to an
    /// importer without going through the filesystem.
    pub fn to_dat_string(&self) -> String {
        let defaults = Self::defaults();
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

        lines.join("\n") + "\n"
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
        Self::defaults()
    }
}

/// A named-key token for `code`, honoring a Shift/Ctrl/Alt modifier as a real
/// `Modified` token (shared by every arrow direction in `key_event_to_name` -
/// they all follow the same Shift-then-Ctrl-then-Alt priority TF and Clay
/// have always used for these).
fn modified_or_named(key: NamedKey, modifiers: crossterm::event::KeyModifiers) -> KeyToken {
    use crossterm::event::KeyModifiers;
    if modifiers.contains(KeyModifiers::SHIFT) {
        KeyToken::Modified(Modifier::Shift, key)
    } else if modifiers.contains(KeyModifiers::CONTROL) {
        KeyToken::Modified(Modifier::Ctrl, key)
    } else if modifiers.contains(KeyModifiers::ALT) {
        KeyToken::Modified(Modifier::Alt, key)
    } else {
        KeyToken::Named(key)
    }
}

/// The raw token for one physical, top-level keystroke (no chord in
/// progress) - `None` for anything that isn't a candidate key name at all
/// (bare modifier keys, unknown codes, or a plain unmodified character:
/// those are ordinary typed input). Shared by [`key_event_to_name`] (wraps
/// and canonicalizes) and `chords::resolve_key_name` (feeds it straight into
/// `ChordState::push`, since at the top level - no chord buffered yet -
/// every keystroke is a candidate first token of one).
pub(crate) fn key_event_to_token(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Option<KeyToken> {
    use crossterm::event::{KeyCode, KeyModifiers};

    Some(match code {
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) => {
            KeyToken::Ctrl(c.to_ascii_uppercase())
        }
        // Alt+char is handled via chords::resolve_key_name's Esc-prefix
        // buffering, not here (crossterm sends Alt modifier for some
        // terminals though) - it means the same "Esc-<char>" token either
        // way (case preserved), per keynames's Alt-x == Esc-x rule.
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::ALT) => {
            KeyToken::Esc(Box::new(KeyToken::Char(c)))
        }
        KeyCode::F(n) => KeyToken::Named(NamedKey::F(n)),
        KeyCode::Up => modified_or_named(NamedKey::Up, modifiers),
        KeyCode::Down => modified_or_named(NamedKey::Down, modifiers),
        KeyCode::Left => modified_or_named(NamedKey::Left, modifiers),
        KeyCode::Right => modified_or_named(NamedKey::Right, modifiers),
        // Finding 41: these used to ignore modifiers outright, so
        // `Ctrl-Home`/`Ctrl-End`/`Ctrl-PageDown` (new TF-parity defaults)
        // could never fire and `Ctrl-Delete` could never be bound - fixed by
        // routing through the same `modified_or_named` the arrows already
        // use.
        KeyCode::PageUp => modified_or_named(NamedKey::PageUp, modifiers),
        KeyCode::PageDown => modified_or_named(NamedKey::PageDown, modifiers),
        KeyCode::Home => modified_or_named(NamedKey::Home, modifiers),
        KeyCode::End => modified_or_named(NamedKey::End, modifiers),
        KeyCode::Insert => modified_or_named(NamedKey::Insert, modifiers),
        KeyCode::Delete => modified_or_named(NamedKey::Delete, modifiers),
        KeyCode::Backspace if modifiers.contains(KeyModifiers::ALT) => {
            KeyToken::Esc(Box::new(KeyToken::Named(NamedKey::Backspace)))
        }
        KeyCode::Backspace => KeyToken::Named(NamedKey::Backspace),
        KeyCode::Tab => modified_or_named(NamedKey::Tab, modifiers),
        // crossterm reports Shift-Tab as its own code, never `Tab` with the
        // Shift modifier bit set - route it through the same `Modified`
        // token `modified_or_named(Tab, SHIFT)` would have produced, so
        // `Shift-Tab` is bindable like any other modified named key
        // (finding 41).
        KeyCode::BackTab => KeyToken::Modified(Modifier::Shift, NamedKey::Tab),
        KeyCode::Enter => modified_or_named(NamedKey::Enter, modifiers),
        KeyCode::Esc => KeyToken::Named(NamedKey::Escape),
        _ => return None,
    })
}

/// Convert a crossterm KeyEvent to canonical key name.
///
/// Returns None if the key event doesn't map to a bindable name
/// (e.g. bare modifier keys, unknown codes, or a plain unmodified character -
/// those are ordinary typed input, not a candidate for a keybinding lookup).
pub fn key_event_to_name(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Option<String> {
    key_event_to_token(code, modifiers).map(|token| KeySeq(vec![token]).canonical())
}

/// The raw *inner* token for one physical keystroke arriving while a bare
/// Escape is already buffered - `None` for anything the `Esc-` grammar
/// doesn't cover. Unlike [`key_event_to_token`] this deliberately handles
/// plain characters too (`Esc-a` is a real, distinct binding from typing
/// `a`), preserving case (`Esc-j` != `Esc-J`, finding A bug 1). Shared by
/// [`escape_key_to_name`] (wraps in `KeyToken::Esc` and canonicalizes) and
/// `chords::resolve_key_name` (which does that same wrapping itself, inside
/// `ChordState::push`, so a chord can keep extending past one Esc-pair).
pub(crate) fn escape_key_to_token(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Option<KeyToken> {
    use crossterm::event::{KeyCode, KeyModifiers};

    Some(match code {
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) => {
            KeyToken::Ctrl(c.to_ascii_uppercase())
        }
        KeyCode::Char(' ') => KeyToken::Named(NamedKey::Space),
        // Preserve case: Esc-j vs Esc-J (finding A, bug 1).
        KeyCode::Char(c) => KeyToken::Char(c),
        KeyCode::Backspace => KeyToken::Named(NamedKey::Backspace),
        // Finding 41: honour a real terminal modifier on these the same way
        // the top-level `key_event_to_token` now does. `Esc-Ctrl-...` isn't
        // one of TF's own documented forms (TF only has `key_esc_ctrl_pgup`-
        // style names reached via the `key_<name>` macro layer, never a raw
        // `Esc-` chord over a modified key) - the simplest, most predictable
        // rule is to keep the `Esc-` wrapper and nest the same `Modified`
        // token inside it, so e.g. physically pressing Escape and then
        // Ctrl-PageDown names `Esc-Ctrl-PageDown` rather than silently
        // dropping the Ctrl bit or refusing the combination outright.
        KeyCode::Tab => modified_or_named(NamedKey::Tab, modifiers),
        KeyCode::BackTab => KeyToken::Modified(Modifier::Shift, NamedKey::Tab),
        KeyCode::Enter => modified_or_named(NamedKey::Enter, modifiers),
        KeyCode::Esc => KeyToken::Named(NamedKey::Escape),
        KeyCode::Up => KeyToken::Named(NamedKey::Up),
        KeyCode::Down => KeyToken::Named(NamedKey::Down),
        KeyCode::Left => KeyToken::Named(NamedKey::Left),
        KeyCode::Right => KeyToken::Named(NamedKey::Right),
        KeyCode::PageUp => modified_or_named(NamedKey::PageUp, modifiers),
        KeyCode::PageDown => modified_or_named(NamedKey::PageDown, modifiers),
        KeyCode::Home => modified_or_named(NamedKey::Home, modifiers),
        KeyCode::End => modified_or_named(NamedKey::End, modifiers),
        KeyCode::Insert => modified_or_named(NamedKey::Insert, modifiers),
        KeyCode::Delete => modified_or_named(NamedKey::Delete, modifiers),
        KeyCode::F(n) => KeyToken::Named(NamedKey::F(n)),
        _ => return None,
    })
}

/// Convert an Escape+key sequence to a canonical "Esc-<token>" name. Names
/// every token TF's `Esc-` grammar covers - arrows and other named keys
/// (`Esc-Left`, `Esc-Delete`), a following Ctrl chord (`Esc-^N`), digits and
/// punctuation (`Esc-0`, `Esc-{`) - not just plain characters and Backspace.
/// Kept as a public convenience (and for its own direct tests); the chord
/// machinery itself (`chords::resolve_key_name`) calls
/// [`escape_key_to_token`] directly so it can fold the result into a
/// `KeyToken::Esc` itself as part of a longer buffered sequence.
pub fn escape_key_to_name(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Option<String> {
    escape_key_to_token(code, modifiers)
        .map(|inner| KeySeq(vec![KeyToken::Esc(Box::new(inner))]).canonical())
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

    /// Every (key, action) pair `tf_defaults()` produces today, pinned exactly. Phase 2 step
    /// P2.6 of the TinyFugue-parity plan deliberately rewrites this table (new default keys,
    /// `Ctrl-Up/Down` retargeted, `^L`/`^U`/`Esc--` reruled, etc.) — this constant forces that
    /// rewrite to touch this test explicitly instead of silently drifting the defaults out
    /// from under `to_dat_string`'s diff-against-defaults logic and every saved
    /// `keybindings.dat` on disk.
    const PINNED_DEFAULTS: &[(&str, &str)] = &[
        // Cursor Movement
        ("^A", "cursor_home"),
        ("^B", "cursor_left"),
        ("^E", "cursor_end"),
        ("^F", "cursor_right"),
        ("Left", "cursor_left"),
        ("Right", "cursor_right"),
        ("Home", "cursor_home"),
        ("End", "cursor_end"),
        ("Esc-b", "cursor_word_left"),
        ("Esc-f", "cursor_word_right"),
        ("Ctrl-Left", "cursor_word_left"),
        ("Ctrl-Right", "cursor_word_right"),
        // Editing
        ("Backspace", "delete_backward"),
        ("Delete", "delete_forward"),
        ("^D", "delete_forward"),
        ("^K", "kill_to_end"),
        ("^U", "kill_to_start"),
        ("^W", "delete_word_backward"),
        ("^T", "transpose_chars"),
        ("^V", "literal_next"),
        ("^Y", "yank"),
        ("Esc-c", "capitalize_word"),
        ("Esc-d", "delete_word_forward"),
        ("Esc-l", "lowercase_word"),
        ("Esc-u", "uppercase_word"),
        ("Esc-Space", "collapse_spaces"),
        ("Esc-Backspace", "delete_word_backward_punct"),
        ("Esc-=", "goto_matching_bracket"),
        ("Esc-.", "insert_last_arg"),
        ("Esc-_", "insert_last_arg"),
        ("Esc-^H", "delete_word_backward_punct"),
        ("^X^?", "delete_word_backward_punct"),
        ("Insert", "toggle_insert"),
        ("Esc-v", "toggle_insert"),
        ("Esc-^E", "expand_line"),
        ("Esc-Tab", "completion"),
        // Numeric prefix
        ("Esc--", "kbnum_negative"),
        ("Esc-0", "kbnum_0"),
        ("Esc-1", "kbnum_1"),
        ("Esc-2", "kbnum_2"),
        ("Esc-3", "kbnum_3"),
        ("Esc-4", "kbnum_4"),
        ("Esc-5", "kbnum_5"),
        ("Esc-6", "kbnum_6"),
        ("Esc-7", "kbnum_7"),
        ("Esc-8", "kbnum_8"),
        ("Esc-9", "kbnum_9"),
        // History
        ("^P", "history_prev"),
        ("^N", "history_next"),
        ("Up", "cursor_up"),
        ("Down", "cursor_down"),
        ("Esc-p", "history_search_backward"),
        ("Esc-n", "history_search_forward"),
        ("Ctrl-Up", "history_prev"),
        ("Ctrl-Down", "history_next"),
        ("Ctrl-Home", "recall_begin"),
        ("Ctrl-End", "recall_end"),
        ("Esc-<", "recall_begin"),
        ("Esc->", "recall_end"),
        // Scrollback
        ("PageUp", "scroll_page_up"),
        ("PageDown", "scroll_page_down"),
        ("Esc-j", "flush_output"),
        ("Esc-J", "selective_flush"),
        ("Esc-h", "scroll_half_page"),
        ("Tab", "tab_key"),
        ("Ctrl-PageDown", "flush_output"),
        ("Esc-^N", "scroll_line_forward"),
        ("Esc-^P", "scroll_line_back"),
        ("Esc-^L", "clear_screen"),
        ("^S", "pause_output"),
        ("^X[", "scroll_half_page_back"),
        ("^X]", "scroll_half_page"),
        ("^X{", "scroll_page_back"),
        ("^X}", "scroll_page_down"),
        // World
        ("Shift-Up", "world_all_next"),
        ("Shift-Down", "world_all_prev"),
        ("Esc-w", "world_activity"),
        ("Esc-Left", "world_socket_prev"),
        ("Esc-Right", "world_socket_next"),
        ("Esc-{", "world_prev"),
        ("Esc-}", "world_next"),
        ("^]", "bg_all_worlds"),
        // System
        ("F1", "help"),
        ("^L", "refresh_line"),
        ("^R", "reload"),
        ("^G", "bell"),
        ("^Z", "suspend"),
        ("^Q", "spell_check"),
        ("^X^R", "reload"),
        ("^X^V", "show_version"),
        ("Esc-L", "toggle_limit"),
        // Clay Extensions
        ("F2", "toggle_tags"),
        ("F4", "filter_popup"),
        ("F5", "search_popup"),
        ("F8", "toggle_action_highlight"),
        ("F9", "toggle_gmcp_media"),
        ("Alt-Up", "input_grow"),
        ("Alt-Down", "input_shrink"),
    ];

    #[test]
    fn test_default_table_pinned() {
        let kb = KeyBindings::tf_defaults();

        let pinned: HashMap<&str, &str> = PINNED_DEFAULTS.iter().copied().collect();
        assert_eq!(pinned.len(), PINNED_DEFAULTS.len(), "PINNED_DEFAULTS has a duplicate key");

        let mut missing: Vec<String> = Vec::new(); // in PINNED_DEFAULTS, absent from tf_defaults()
        let mut extra: Vec<String> = Vec::new();   // in tf_defaults(), absent from PINNED_DEFAULTS
        let mut different: Vec<String> = Vec::new(); // present in both, different action

        for (key, action) in &pinned {
            match kb.get_action(key) {
                None => missing.push(format!("{key} -> {action}")),
                Some(actual) if actual != *action => {
                    different.push(format!("{key}: pinned={action}, actual={actual}"))
                }
                _ => {}
            }
        }
        for (key, action) in &kb.bindings {
            if !pinned.contains_key(key.as_str()) {
                extra.push(format!("{key} -> {action}"));
            }
        }
        missing.sort();
        extra.sort();
        different.sort();

        assert!(missing.is_empty() && extra.is_empty() && different.is_empty(),
            "tf_defaults() no longer matches the pinned table (expected — Phase 2 step P2.6 \
             changes this deliberately; update PINNED_DEFAULTS to match and move on).\n\
             missing from tf_defaults() (in PINNED_DEFAULTS but not produced): {:#?}\n\
             extra in tf_defaults() (produced but not in PINNED_DEFAULTS): {:#?}\n\
             different action for the same key: {:#?}",
            missing, extra, different);

        // Every action id the table uses must be a real, known action.
        let known_actions: std::collections::HashSet<&str> = ACTIONS.iter().map(|a| a.id).collect();
        let mut unknown_actions: Vec<&str> = PINNED_DEFAULTS.iter()
            .map(|(_, action)| *action)
            .filter(|action| !known_actions.contains(action))
            .collect();
        unknown_actions.sort();
        unknown_actions.dedup();
        assert!(unknown_actions.is_empty(),
            "PINNED_DEFAULTS references action id(s) not present in ACTIONS: {:#?}",
            unknown_actions);
    }

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
    fn test_to_dat_string_matches_save() {
        let mut kb = KeyBindings::tf_defaults();
        kb.set_binding("Up", "world_next");
        kb.remove_binding("^Z");

        // to_dat_string (used by the /import export path) must be byte-identical to what
        // save() writes to keybindings.dat, since it's the same content taking a different
        // exit (a WsMessage instead of a file).
        let dir = std::env::temp_dir().join("clay_test_keybindings_dat_string");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.key.dat");
        kb.save(&path).unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(kb.to_dat_string(), on_disk);

        // And it round-trips through the same parser as the on-disk file.
        let loaded = KeyBindings::from_dat_string(&kb.to_dat_string());
        assert_eq!(loaded.get_action("Up"), Some("world_next"));
        assert_eq!(loaded.get_action("^Z"), None);
        assert_eq!(loaded.get_action("^A"), Some("cursor_home"));
    }

    #[test]
    fn test_chord_and_raw_form_round_trip_through_dat_string() {
        // plan P2.1: a chord name, and a non-canonical raw spelling of an
        // ordinary one, both survive a keybindings.dat round-trip under
        // their canonical form. Both rebindings deliberately differ from the
        // ruling-table defaults for these exact keys (`^X^R` -> reload,
        // `Ctrl-Up` -> history_prev as of Job 22a/P2.6) so `to_dat_string`'s
        // diff-against-defaults logic actually has something to list.
        let mut kb = KeyBindings::tf_defaults();
        kb.set_binding("^X^R", "quit");
        kb.set_binding("ctrl+up", "world_next"); // non-canonical spelling in

        let dat = kb.to_dat_string();
        assert!(dat.contains("^X^R = quit"), "chord should be listed canonically:\n{dat}");

        let loaded = KeyBindings::from_dat_string(&dat);
        assert_eq!(loaded.get_action("^X^R"), Some("quit"));
        // Looking it up via a differently-spelled but equivalent name still finds it.
        assert_eq!(loaded.get_action("Ctrl-Up"), Some("world_next"));
        assert_eq!(loaded.get_action("ctrl+up"), Some("world_next"));
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

    /// Finding 41 / Job 22c: `PageUp/PageDown/Home/End/Insert/Delete/Tab/Enter` used to drop
    /// every modifier (only the arrows honored Shift/Ctrl/Alt), so the new
    /// `Ctrl-Home`/`Ctrl-End`/`Ctrl-PageDown` defaults could never fire and `Shift-Tab`/
    /// `Ctrl-Delete` could never be bound at all. Table-driven so every one of the 8 affected
    /// codes gets the same Shift/Ctrl/Alt/none coverage the arrows already had.
    #[test]
    fn test_key_event_to_name_modified_named_keys() {
        use crossterm::event::{KeyCode, KeyModifiers};

        let codes: &[(KeyCode, &str)] = &[
            (KeyCode::PageUp, "PageUp"),
            (KeyCode::PageDown, "PageDown"),
            (KeyCode::Home, "Home"),
            (KeyCode::End, "End"),
            (KeyCode::Insert, "Insert"),
            (KeyCode::Delete, "Delete"),
            (KeyCode::Tab, "Tab"),
            (KeyCode::Enter, "Enter"),
        ];
        for &(code, base) in codes {
            assert_eq!(key_event_to_name(code, KeyModifiers::NONE), Some(base.to_string()),
                "{base} with no modifier should stay bare");
            assert_eq!(key_event_to_name(code, KeyModifiers::CONTROL), Some(format!("Ctrl-{base}")),
                "{base} + Ctrl should produce a Modified token");
            assert_eq!(key_event_to_name(code, KeyModifiers::SHIFT), Some(format!("Shift-{base}")),
                "{base} + Shift should produce a Modified token");
            assert_eq!(key_event_to_name(code, KeyModifiers::ALT), Some(format!("Alt-{base}")),
                "{base} + Alt should produce a Modified token");
        }

        // crossterm reports Shift-Tab as its own `BackTab` code, never `Tab` with the Shift
        // modifier bit set - it must still land on the same `Shift-Tab` canonical name.
        assert_eq!(key_event_to_name(KeyCode::BackTab, KeyModifiers::NONE), Some("Shift-Tab".into()));

        // The specific plan-cited regressions: the three new defaults must now be nameable,
        // and Ctrl-Delete (previously unbindable) resolves too.
        assert_eq!(key_event_to_name(KeyCode::Home, KeyModifiers::CONTROL), Some("Ctrl-Home".into()));
        assert_eq!(key_event_to_name(KeyCode::End, KeyModifiers::CONTROL), Some("Ctrl-End".into()));
        assert_eq!(key_event_to_name(KeyCode::PageDown, KeyModifiers::CONTROL), Some("Ctrl-PageDown".into()));
        assert_eq!(key_event_to_name(KeyCode::Delete, KeyModifiers::CONTROL), Some("Ctrl-Delete".into()));
    }

    /// Same fix, after an Escape prefix. Ruling: `Esc-` followed by a modified named key keeps
    /// the `Esc-` wrapper and nests the real `Modified` token inside it (`Esc-Ctrl-PageDown`) -
    /// TF has no raw `Esc-Ctrl-...` form of its own (only `key_esc_ctrl_pgdn`-style names via
    /// the `key_<name>` macro layer), so this is Clay's own simplest, most predictable choice
    /// rather than a documented TF spelling.
    #[test]
    fn test_escape_key_to_name_modified_named_keys() {
        use crossterm::event::{KeyCode, KeyModifiers};

        assert_eq!(escape_key_to_name(KeyCode::PageDown, KeyModifiers::CONTROL),
            Some("Esc-Ctrl-PageDown".into()));
        assert_eq!(escape_key_to_name(KeyCode::Home, KeyModifiers::CONTROL),
            Some("Esc-Ctrl-Home".into()));
        assert_eq!(escape_key_to_name(KeyCode::End, KeyModifiers::CONTROL),
            Some("Esc-Ctrl-End".into()));
        assert_eq!(escape_key_to_name(KeyCode::Delete, KeyModifiers::SHIFT),
            Some("Esc-Shift-Delete".into()));
        assert_eq!(escape_key_to_name(KeyCode::Insert, KeyModifiers::ALT),
            Some("Esc-Alt-Insert".into()));
        assert_eq!(escape_key_to_name(KeyCode::Tab, KeyModifiers::NONE), Some("Esc-Tab".into()));
        assert_eq!(escape_key_to_name(KeyCode::BackTab, KeyModifiers::NONE), Some("Esc-Shift-Tab".into()));
        assert_eq!(escape_key_to_name(KeyCode::Enter, KeyModifiers::CONTROL), Some("Esc-Ctrl-Enter".into()));
        // Unmodified named keys after Esc are unaffected by this fix (regression guard).
        assert_eq!(escape_key_to_name(KeyCode::PageUp, KeyModifiers::NONE), Some("Esc-PageUp".into()));
    }

    /// TF-parity plan Job 22a (P2.6): an old `keybindings.dat` written before the ruling
    /// table's `^L` retarget (`redraw` -> `refresh_line`) explicitly customized `^L` back to
    /// `redraw` still loads and wins - `from_dat_string` always applies every explicit line
    /// on top of the current `defaults()`, so a pre-existing customization survives a
    /// default-table rewrite even when the customization's action id happens to match the
    /// key's *old* default rather than its new one.
    #[test]
    fn test_old_dat_line_overriding_a_retargeted_default_still_loads_and_wins() {
        let content = "[bindings]\n^L = redraw\n";
        let loaded = KeyBindings::from_dat_string(content);
        assert_eq!(loaded.get_action("^L"), Some("redraw"),
            "an explicit ^L = redraw line must win over the new default (refresh_line)");
        // Sanity: the new default itself is indeed different, so this test would catch a
        // regression where `from_dat_string` silently dropped the override.
        assert_eq!(KeyBindings::defaults().get_action("^L"), Some("refresh_line"));
        // Everything else the file didn't mention still comes from the new defaults.
        assert_eq!(loaded.get_action("^U"), Some("kill_to_start"));
    }

    #[test]
    fn test_defaults_alias_matches_defaults() {
        // `tf_defaults()` is kept as a plain alias of `defaults()` for existing callers.
        let a = KeyBindings::defaults();
        let b = KeyBindings::tf_defaults();
        assert_eq!(a.bindings, b.bindings);
    }

    /// TF-parity plan Job 23 (P2.8, docs): `docs/markdown/07-keyboard-shortcuts.md`'s
    /// "Appendix: Every Default Binding" table is meant to be the human-readable mirror of
    /// `PINNED_DEFAULTS` above - this parses that exact table out of the checked-in markdown
    /// file (via `env!("CARGO_MANIFEST_DIR")`, same recipe `script_tests.rs` uses for its own
    /// fixtures) and asserts set equality with `PINNED_DEFAULTS` in both directions, so a
    /// future default-table change that updates one but not the other fails the build instead
    /// of silently drifting the docs out from under the code (exactly the class of bug finding
    /// A called out: "`docs/markdown/07-keyboard-shortcuts.md` contradicts the code").
    #[test]
    fn test_docs_key_table_matches_defaults() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // TINYFUGUE-COMPAT.md is tracked in git, so this drift guard runs in every clone.
        assert_key_table_matches(&root.join("TINYFUGUE-COMPAT.md"));
        // docs/ is gitignored (generated-PDF sources), so the user-facing chapter carries
        // the same marked table but is only checked on a machine that actually has it.
        let chapter = root.join("docs/markdown/07-keyboard-shortcuts.md");
        if chapter.exists() {
            assert_key_table_matches(&chapter);
        }
    }

    /// Parse the "DEFAULT KEY TABLE" markdown block out of `path` and assert it is exactly
    /// `PINNED_DEFAULTS`, in both directions and in sorted-by-key order.
    fn assert_key_table_matches(path: &std::path::Path) {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));

        const BEGIN: &str = "<!-- BEGIN DEFAULT KEY TABLE -->";
        const END: &str = "<!-- END DEFAULT KEY TABLE -->";
        let start = content.find(BEGIN)
            .unwrap_or_else(|| panic!("{} missing {:?} marker", path.display(), BEGIN))
            + BEGIN.len();
        let end = content[start..].find(END)
            .unwrap_or_else(|| panic!("{} missing {:?} marker", path.display(), END))
            + start;
        let table = &content[start..end];

        // Row shape: "| `Key` | `action_id` |" - skip the header/separator lines, which
        // don't have backtick-quoted cells.
        let mut doc_pairs: Vec<(String, String)> = Vec::new();
        for line in table.lines() {
            let line = line.trim();
            if !line.starts_with('|') {
                continue;
            }
            let cells: Vec<&str> = line.split('|').map(|c| c.trim()).collect();
            // split('|') on "| `Key` | `action_id` |" yields ["", "`Key`", "`action_id`", ""]
            if cells.len() != 4 {
                continue;
            }
            let unquote = |c: &str| -> Option<String> {
                c.strip_prefix('`').and_then(|c| c.strip_suffix('`')).map(|c| c.to_string())
            };
            if let (Some(key), Some(action)) = (unquote(cells[1]), unquote(cells[2])) {
                doc_pairs.push((key, action));
            }
        }

        assert!(!doc_pairs.is_empty(), "parsed zero rows out of the appendix table in {}", path.display());

        let mut doc_keys: Vec<String> = Vec::new();
        for (k, _) in &doc_pairs {
            assert!(!doc_keys.contains(k), "appendix table has a duplicate key: {k}");
            doc_keys.push(k.clone());
        }
        // The task asks for the appendix sorted by key - verify it actually is, since a
        // human hand-editing this table is exactly how it could quietly go out of order.
        let mut sorted_keys = doc_keys.clone();
        sorted_keys.sort();
        assert_eq!(doc_keys, sorted_keys, "appendix table rows must be sorted by key");

        let pinned: std::collections::HashMap<&str, &str> = PINNED_DEFAULTS.iter().copied().collect();
        let doc: std::collections::HashMap<&str, &str> = doc_pairs.iter()
            .map(|(k, v)| (k.as_str(), v.as_str())).collect();

        let mut missing_from_docs: Vec<String> = Vec::new(); // in PINNED_DEFAULTS, absent from the doc table
        let mut extra_in_docs: Vec<String> = Vec::new();     // in the doc table, absent from PINNED_DEFAULTS
        let mut different: Vec<String> = Vec::new();

        for (key, action) in &pinned {
            match doc.get(key) {
                None => missing_from_docs.push(format!("{key} -> {action}")),
                Some(doc_action) if doc_action != action => {
                    different.push(format!("{key}: code={action}, docs={doc_action}"))
                }
                _ => {}
            }
        }
        for (key, action) in &doc {
            if !pinned.contains_key(key) {
                extra_in_docs.push(format!("{key} -> {action}"));
            }
        }
        missing_from_docs.sort();
        extra_in_docs.sort();
        different.sort();

        assert!(missing_from_docs.is_empty() && extra_in_docs.is_empty() && different.is_empty(),
            "{}'s appendix table no longer matches \
             PINNED_DEFAULTS - update whichever one is stale.\n\
             in PINNED_DEFAULTS but missing from the docs table: {:#?}\n\
             in the docs table but not in PINNED_DEFAULTS: {:#?}\n\
             same key, different action: {:#?}",
            path.display(), missing_from_docs, extra_in_docs, different);
    }
}
