//! A keystroke to what it does, resolved the way every Clay client resolves it
//! (CLAUDE.md "Key-name grammar and dispatch order"): the canonical key name
//! (`keybindings::key_event_to_token`, chords through `chords::ChordState`), then
//!
//! 1. a key the *server* has bound (`/bind`, `/def -b`, a `key_<name>` macro) — TF state
//!    lives only there, so the keystroke goes back as `RunKeyBinding`;
//! 2. the action table (the server's `keybindings_json`, defaults included);
//! 3. otherwise it's typed input.
//!
//! The same order and grammar as the console and the SSH remote console, because the same
//! code names the keys; only the binding tables come from the server here.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};

use crate::chords::{ChordResult, ChordState, DEFAULT_CHORD_WINDOW};
use crate::keybindings::{KeyBindings, escape_key_to_token, key_event_to_token};
use crate::keynames::{self, KeySeq};
use crate::websocket::GlobalSettingsMsg;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// Part of a chord: wait for the next keystroke.
    Pending,
    /// The server has this key bound: send `RunKeyBinding`.
    Server(String),
    /// A built-in action.
    Action { key: String, action: String },
    /// Not a binding (typed input, or a key nothing uses).
    NotBound,
}

pub struct Keys {
    bindings: KeyBindings,
    server_bound: HashSet<String>,
    chord: ChordState,
    window: Duration,
    /// What the tables were built from, to rebuild only on a change.
    source: (String, String),
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            bindings: KeyBindings::defaults(),
            server_bound: HashSet::new(),
            chord: ChordState::new(),
            window: DEFAULT_CHORD_WINDOW,
            source: (String::new(), String::new()),
        }
    }
}

impl Keys {
    /// Take the server's tables from its settings (when they've changed).
    pub fn update(&mut self, settings: &GlobalSettingsMsg) {
        let source = (settings.keybindings_json.clone(), settings.tf_bound_keys_json.clone());
        if source == self.source {
            return;
        }
        if !source.0.is_empty() {
            self.bindings = KeyBindings::from_json(&source.0);
        }
        self.server_bound = serde_json::from_str::<Vec<String>>(&source.1).unwrap_or_default().into_iter().collect();
        self.source = source;
    }

    pub fn action_for(&self, key: &str) -> Option<&str> {
        self.bindings.get_action(key)
    }

    fn lookup(&self, name: String) -> Resolved {
        if self.server_bound.contains(&name) {
            return Resolved::Server(name);
        }
        match self.bindings.get_action(&name) {
            Some(action) => Resolved::Action { action: action.to_string(), key: name },
            None => Resolved::NotBound,
        }
    }

    /// One physical keystroke.
    pub fn resolve(&mut self, code: KeyCode, mods: KeyModifiers, now: Instant) -> Resolved {
        let action_keys: Vec<String> = self.bindings.bindings.keys().cloned().collect();
        let server_keys: Vec<String> = self.server_bound.iter().cloned().collect();
        let is_prefix = |c: &KeySeq| {
            keynames::is_prefix_of_any(action_keys.iter().map(String::as_str), c)
                || keynames::is_prefix_of_any(server_keys.iter().map(String::as_str), c)
        };
        let has_binding = |c: &KeySeq| {
            let name = c.canonical();
            action_keys.contains(&name) || server_keys.contains(&name)
        };
        if self.chord.is_pending() {
            if let Some(fired) = self.chord.expired(now, self.window, has_binding) {
                return self.lookup(fired.canonical());
            }
        }
        let mid_chord = self.chord.is_pending();
        let token = if mid_chord { escape_key_to_token(code, mods) } else { key_event_to_token(code, mods) };
        let Some(token) = token else {
            if mid_chord {
                self.chord.cancel();
            }
            return Resolved::NotBound;
        };
        if mid_chord && token == keynames::KeyToken::Ctrl('G') {
            self.chord.cancel();
            return Resolved::NotBound;
        }
        match self.chord.push(token, now, is_prefix, has_binding) {
            ChordResult::Pending => Resolved::Pending,
            ChordResult::Complete(seq) => self.lookup(seq.canonical()),
            ChordResult::Abandon { prefix, .. } => {
                if has_binding(&prefix) {
                    return self.lookup(prefix.canonical());
                }
                match key_event_to_token(code, mods) {
                    None => Resolved::NotBound,
                    Some(token) => match self.chord.push(token, now, is_prefix, has_binding) {
                        ChordResult::Pending => Resolved::Pending,
                        ChordResult::Complete(seq) => self.lookup(seq.canonical()),
                        ChordResult::Abandon { .. } => Resolved::NotBound,
                    },
                }
            }
        }
    }

    /// A chord left waiting fires its own binding once its window passes (`Esc` alone
    /// never does).
    pub fn tick(&mut self, now: Instant) -> Option<Resolved> {
        if !self.chord.is_pending() {
            return None;
        }
        let action_keys: Vec<String> = self.bindings.bindings.keys().cloned().collect();
        let server_keys: Vec<String> = self.server_bound.iter().cloned().collect();
        let fired = self.chord.expired(now, self.window, |c: &KeySeq| {
            let name = c.canonical();
            action_keys.contains(&name) || server_keys.contains(&name)
        })?;
        Some(self.lookup(fired.canonical()))
    }

    pub fn chord_pending(&self) -> bool {
        self.chord.is_pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(r: Resolved) -> String {
        match r {
            Resolved::Action { action, .. } => action,
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn default_keys_resolve_to_their_actions() {
        let mut k = Keys::default();
        let now = Instant::now();
        let none = KeyModifiers::NONE;
        assert_eq!(act(k.resolve(KeyCode::PageUp, none, now)), "scroll_page_up");
        assert_eq!(act(k.resolve(KeyCode::Up, KeyModifiers::SHIFT, now)), "world_all_next");
        assert_eq!(act(k.resolve(KeyCode::F(2), none, now)), "toggle_tags");
        assert_eq!(act(k.resolve(KeyCode::Char('l'), KeyModifiers::CONTROL, now)), "redraw");
        // Alt-w is Esc-w.
        assert_eq!(act(k.resolve(KeyCode::Char('w'), KeyModifiers::ALT, now)), "world_activity");
        // Escape then j: a chord.
        assert_eq!(k.resolve(KeyCode::Esc, none, now), Resolved::Pending);
        assert_eq!(act(k.resolve(KeyCode::Char('j'), none, now)), "flush_output");
        // ^X ^R.
        assert_eq!(k.resolve(KeyCode::Char('x'), KeyModifiers::CONTROL, now), Resolved::Pending);
        assert_eq!(act(k.resolve(KeyCode::Char('r'), KeyModifiers::CONTROL, now)), "reload");
        // A plain letter is typed input.
        assert_eq!(k.resolve(KeyCode::Char('a'), none, now), Resolved::NotBound);
    }

    #[test]
    fn a_key_the_server_has_bound_goes_to_the_server_first() {
        let mut k = Keys::default();
        let mut s = crate::App::new().build_global_settings_msg();
        s.tf_bound_keys_json = r#"["F2","Esc-x"]"#.into();
        k.update(&s);
        let now = Instant::now();
        assert_eq!(k.resolve(KeyCode::F(2), KeyModifiers::NONE, now), Resolved::Server("F2".into()));
        assert_eq!(k.resolve(KeyCode::Esc, KeyModifiers::NONE, now), Resolved::Pending);
        assert_eq!(k.resolve(KeyCode::Char('x'), KeyModifiers::NONE, now), Resolved::Server("Esc-x".into()));
    }

    #[test]
    fn a_lone_escape_times_out_silently() {
        let mut k = Keys::default();
        let now = Instant::now();
        assert_eq!(k.resolve(KeyCode::Esc, KeyModifiers::NONE, now), Resolved::Pending);
        assert_eq!(k.tick(now + Duration::from_secs(1)), None);
        assert!(!k.chord_pending());
    }
}
