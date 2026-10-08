//! More-mode as the web client does it on its side (app.js `handleIncomingLine`,
//! `releaseLines`): once a screenful (less two rows) of new lines has arrived since the
//! user last caught up, further lines are held back, unseen, until released — a
//! screenful at a time, a few rows, or all at once. The server keeps its own count of
//! lines it holds (`World::pending_count`); the More badge shows both.
//!
//! Held lines stay in the client's buffer; they just aren't drawn. Switching world drops
//! the hold (the lines then show), as app.js does.

use std::collections::{HashSet, VecDeque};

use crate::native_gui::core::client::World;

#[derive(Default)]
pub struct MoreMode {
    pub paused: bool,
    held: VecDeque<u64>,
    held_set: HashSet<u64>,
    since_pause: usize,
    /// The newest line id already seen at the tail of the shown world.
    last_tail: Option<u64>,
    /// Wheel movement past the bottom not yet worth a whole row.
    pub wheel_px: f32,
}

impl MoreMode {
    /// Start over for a world (a switch, a new InitialState): nothing held, and what's
    /// there now counts as seen.
    pub fn reset(&mut self, world: Option<&World>) {
        self.paused = false;
        self.held.clear();
        self.held_set.clear();
        self.since_pause = 0;
        self.wheel_px = 0.0;
        self.last_tail = world.and_then(|w| w.lines.last()).map(|l| l.id);
    }

    pub fn is_held(&self, id: u64) -> bool {
        self.held_set.contains(&id)
    }

    pub fn held(&self) -> usize {
        self.held.len()
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Lines that arrived at the tail of `world` since the last call. Returns whether
    /// output paused just now (the view then goes to the bottom, as app.js does).
    pub fn arrivals(&mut self, world: &World, enabled: bool, threshold: usize) -> bool {
        let since = self.last_tail;
        let start = world.lines.iter().rposition(|l| Some(l.id) == since).map_or_else(
            // The last seen line is gone (a flush): everything is new, but nothing to hold.
            || world.lines.len(),
            |p| p + 1,
        );
        self.last_tail = world.lines.last().map(|l| l.id).or(since);
        let mut paused_now = false;
        for line in &world.lines[start..] {
            // Only server lines with a real seq go through more-mode (app.js: released
            // and client lines are appended directly); gagged lines don't count.
            if !line.from_server || line.seq.is_none() || line.gagged {
                continue;
            }
            if self.paused {
                self.hold(line.id);
            } else if enabled && self.since_pause >= threshold.max(1) {
                self.paused = true;
                paused_now = true;
                self.hold(line.id);
            } else {
                self.since_pause += 1;
            }
        }
        paused_now
    }

    fn hold(&mut self, id: u64) {
        if self.held_set.insert(id) {
            self.held.push_back(id);
        }
    }

    /// Show up to `n` held lines; returns how many.
    pub fn release(&mut self, n: usize) -> usize {
        let mut k = 0;
        while k < n {
            let Some(id) = self.held.pop_front() else { break };
            self.held_set.remove(&id);
            k += 1;
        }
        if self.held.is_empty() {
            self.paused = false;
            self.since_pause = 0;
        }
        k
    }

    pub fn release_all(&mut self) {
        self.release(usize::MAX);
    }

    /// The user caught up (released output, sent a command): count afresh.
    pub fn caught_up(&mut self) {
        self.since_pause = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_gui::core::client::Line;

    fn line(id: u64) -> Line {
        Line {
            id,
            text: format!("l{id}"),
            ts: 0,
            seq: Some(id),
            from_server: true,
            gagged: false,
            highlight: None,
            viewed: true,
            display_id: None,
            archive_sourced: false,
            from_archive: false,
        }
    }

    #[test]
    fn holds_after_a_screenful_and_releases_in_order() {
        let mut w = World::default();
        w.lines = (1..=3).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((4..=10).map(line));
        let paused = m.arrivals(&w, true, 5);
        assert!(paused);
        // 5 shown, 6..=10 held... the 6th arrival paused.
        assert_eq!(m.held(), 2);
        assert!(m.is_held(9) && m.is_held(10) && !m.is_held(8));
        assert_eq!(m.release(1), 1);
        assert!(!m.is_held(9) && m.is_held(10));
        m.release_all();
        assert!(!m.paused && m.held() == 0);
    }

    #[test]
    fn off_means_nothing_is_held_and_gap_fills_never_count() {
        let mut w = World::default();
        w.lines = (1..=3).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((4..=50).map(line));
        assert!(!m.arrivals(&w, false, 5));
        assert_eq!(m.held(), 0);
        // A line inserted mid-buffer (a gap fill) isn't an arrival.
        w.lines.insert(10, line(99));
        assert!(!m.arrivals(&w, true, 1));
        assert_eq!(m.held(), 0);
    }
}
