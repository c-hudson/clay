//! TF's `%textdiv`: a divider between the text this client had already shown of a world
//! and what arrived after, drawn when the user comes back to it (app.js
//! `clearLeavingWorldState`/`arriveTextdiv`, the console's `App::track_console_world`).
//!
//! - "on": only when something arrived since the world was last shown, above the first
//!   such line.
//! - "always": every visit, above the first new line or after the last.
//! - "clear": the same place, a blank as tall as the output, which takes the old text out
//!   of view the way TF clears it (scrolling back still shows it).
//!
//! The divider is not a line: it's never copied, and its rows are pseudo-lines with ids
//! from [`DIVIDER_ID`] up.

use std::collections::HashMap;

use crate::native_gui::core::client::World;

/// The divider's rows use ids from here up (line ids never get near it).
pub const DIVIDER_ID: u64 = u64::MAX - (1 << 20);

pub fn is_divider(id: u64) -> bool {
    id >= DIVIDER_ID
}

/// The seq of a world's newest line (not prepended history), if any.
pub fn last_seq(w: &World) -> Option<u64> {
    w.lines.iter().rev().filter(|l| !l.from_archive).find_map(|l| l.seq)
}

#[derive(Default)]
pub struct Textdiv {
    /// Per world (by name): the newest seq this client had shown when it last left it.
    seen: HashMap<String, Option<u64>>,
    /// This visit's divider: below seq (`None`: above every line), and the mode.
    pub visit: Option<(Option<u64>, String)>,
}

impl Textdiv {
    /// What a world holds when this client first sees it counts as shown.
    pub fn note_worlds(&mut self, worlds: &[World]) {
        for w in worlds {
            if !self.seen.contains_key(&w.name) {
                self.seen.insert(w.name.clone(), last_seq(w));
            }
        }
    }

    /// Leaving a world: shown up to here, and this visit's divider goes.
    pub fn leave(&mut self, w: &World) {
        if let Some(seq) = last_seq(w) {
            self.seen.insert(w.name.clone(), Some(seq));
        }
        self.visit = None;
    }

    /// Arriving at a world with `%textdiv` set to `mode`.
    pub fn arrive(&mut self, w: &World, mode: &str) {
        self.visit = None;
        if !matches!(mode, "on" | "always" | "clear") {
            return;
        }
        let after = self.seen.get(&w.name).copied().flatten();
        let last = last_seq(w);
        let has_new = last.is_some_and(|l| after.is_none_or(|a| l > a));
        if mode != "on" || has_new {
            self.visit = Some((after, mode.to_string()));
        }
    }

    /// Where the divider goes among `seqs` (each shown line's seq, `None` for client and
    /// prepended lines): before index i, or `None` for nowhere.
    pub fn place(&self, seqs: &[Option<u64>]) -> Option<usize> {
        let (after, mode) = self.visit.as_ref()?;
        match seqs.iter().position(|s| s.is_some_and(|s| after.is_none_or(|a| s > a))) {
            Some(i) => Some(i),
            None if mode != "on" => Some(seqs.len()),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_gui::core::client::Line;

    fn world(name: &str, seqs: std::ops::Range<u64>) -> World {
        let mut w = World::default();
        w.name = name.into();
        w.lines = seqs
            .map(|s| Line {
                id: s,
                text: format!("l{s}"),
                ts: 0,
                seq: Some(s),
                from_server: true,
                gagged: false,
                highlight: None,
                viewed: true,
                display_id: None,
                archive_sourced: false,
                from_archive: false,
                scrollback: false,
            })
            .collect();
        w
    }

    #[test]
    fn on_shows_only_after_new_text_and_always_shows_every_visit() {
        let mut t = Textdiv::default();
        let mut w = world("a", 0..5);
        t.note_worlds(std::slice::from_ref(&w));
        t.arrive(&w, "on");
        assert_eq!(t.visit, None, "nothing new since it was first seen");
        t.leave(&w);
        w = world("a", 0..8);
        t.arrive(&w, "on");
        let seqs: Vec<Option<u64>> = w.lines.iter().map(|l| l.seq).collect();
        assert_eq!(t.place(&seqs), Some(5), "above the first new line");
        t.leave(&w);
        t.arrive(&w, "always");
        assert_eq!(t.place(&seqs), Some(8), "nothing new: after the last line");
        t.arrive(&w, "");
        assert_eq!(t.visit, None);
    }
}
