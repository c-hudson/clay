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

/// What this client had shown of one world: its newest seq, in the world's seq space.
struct Mark {
    seq: Option<u64>,
    epoch: u64,
    /// Set by the first switch to the world (or away from it). Until then, what the
    /// client catches up on counts as shown too.
    fixed: bool,
}

#[derive(Default)]
pub struct Textdiv {
    /// Per world (by name): the newest seq this client had shown when it last left it.
    seen: HashMap<String, Mark>,
    /// This visit's divider: below seq (`None`: above every line), and the mode.
    pub visit: Option<(Option<u64>, String)>,
}

impl Textdiv {
    /// What a world holds when this client first sees it counts as shown, and so does
    /// what it catches up on (a cold start's gap fill after the disk cache, the server's
    /// resume replay: `Line::scrollback`) until the first switch to it. Only lines that
    /// arrive live are new. A world whose seq space changed (the server restarted) starts
    /// over from what it holds.
    pub fn note_worlds(&mut self, worlds: &[World]) {
        for w in worlds {
            match self.seen.get_mut(&w.name) {
                Some(m) if m.epoch == w.epoch() => {
                    if !m.fixed {
                        // The tail newer than the mark, newest first.
                        let caught_up = w.lines.iter().rev()
                            .filter(|l| !l.from_archive)
                            .filter_map(|l| l.seq.map(|seq| (seq, l.scrollback)))
                            .take_while(|(seq, _)| m.seq.is_none_or(|s| *seq > s))
                            .filter(|(_, scrollback)| *scrollback)
                            .map(|(seq, _)| seq)
                            .max();
                        if caught_up.is_some() {
                            m.seq = caught_up;
                        }
                    }
                }
                _ => {
                    self.seen.insert(w.name.clone(), Mark { seq: last_seq(w), epoch: w.epoch(), fixed: false });
                }
            }
        }
    }

    /// The newest seq this client had shown of the world named `name` when it last left
    /// it (`None`: nothing, or never seen): what New Only text reveal counts as old.
    pub fn seen(&self, name: &str) -> Option<u64> {
        self.seen.get(name).and_then(|m| m.seq)
    }

    /// Leaving a world: shown up to here, and this visit's divider goes.
    pub fn leave(&mut self, w: &World) {
        let seq = last_seq(w);
        let mark = self.seen.entry(w.name.clone()).or_insert(Mark { seq, epoch: w.epoch(), fixed: true });
        if seq.is_some() {
            (mark.seq, mark.epoch) = (seq, w.epoch());
        }
        mark.fixed = true;
        self.visit = None;
    }

    /// Arriving at a world with `%textdiv` set to `mode`.
    pub fn arrive(&mut self, w: &World, mode: &str) {
        self.visit = None;
        // From here on, only what arrives live is new.
        if let Some(m) = self.seen.get_mut(&w.name) {
            m.fixed = true;
        }
        if !matches!(mode, "on" | "always" | "clear") {
            return;
        }
        let after = self.seen(&w.name);
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

    /// `seqs` appended to `w`, as a catch-up (`scrollback`) or live.
    fn append(w: &mut World, seqs: std::ops::Range<u64>, scrollback: bool) {
        let mut more = world(&w.name, seqs).lines;
        for l in &mut more {
            l.scrollback = scrollback;
        }
        w.lines.extend(more);
    }

    /// A cold start: the disk cache, then the catch-up on what arrived while the GUI was
    /// closed. The catch-up counts as shown until the first switch to the world; a line
    /// that arrives live is new (user report 2026-10-26: New Only typed on whole screens).
    #[test]
    fn a_start_and_its_catch_up_count_as_shown_until_the_first_visit() {
        let mut t = Textdiv::default();
        let mut w = world("a", 0..30);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), Some(29), "the cache");
        append(&mut w, 30..90, true);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), Some(89), "the catch-up is history, not news");
        append(&mut w, 90..95, false);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), Some(89), "a live line is new");
        t.arrive(&w, "");
        append(&mut w, 95..99, true);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), Some(89), "after the first visit only leaving moves the mark");
        t.leave(&w);
        assert_eq!(t.seen("a"), Some(98));
    }

    /// A world that had nothing when first seen takes its catch-up too.
    #[test]
    fn an_empty_world_takes_its_catch_up_as_shown() {
        let mut t = Textdiv::default();
        let mut w = world("a", 0..0);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), None);
        append(&mut w, 0..40, true);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), Some(39));
    }

    /// A new seq space (the server restarted) makes the old mark meaningless: what the
    /// world holds now counts as shown, and its catch-up too.
    #[test]
    fn a_new_epoch_starts_the_mark_over() {
        let mut t = Textdiv::default();
        let mut w = world("a", 0..500);
        w.set_epoch(7);
        t.note_worlds(std::slice::from_ref(&w));
        t.arrive(&w, "");
        t.leave(&w);
        assert_eq!(t.seen("a"), Some(499));
        let mut w = world("a", 0..20);
        w.set_epoch(8);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), Some(19));
        append(&mut w, 20..30, true);
        t.note_worlds(std::slice::from_ref(&w));
        assert_eq!(t.seen("a"), Some(29));
    }
}
