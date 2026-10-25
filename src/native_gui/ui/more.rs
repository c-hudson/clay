//! More-mode as the web client does it on its side (app.js `handleIncomingLine`,
//! `releaseLines`): once a screenful (less two rows) of new output has arrived since the
//! user last caught up, further lines are held back, unseen, until released — a
//! screenful at a time, a few rows, or all at once. The server keeps its own count of
//! lines it holds (`World::pending_count`); the More badge shows both.
//!
//! - Held lines stay in the client's buffer; they just aren't drawn. Nor is anything
//!   after the first of them (`ClayView::visible_lines`), so the output never shows a
//!   line below one it's holding back: released lines always come out at the bottom.
//!   Once paused, every line that arrives is held, the client's own (an action's
//!   `/echo`) as much as the MUD's, as app.js queues them. Holding only the MUD's left
//!   the others drawn below the hold, where they stayed while Page Down released lines
//!   in above them: the bottom of the view stuck, half a window moving at a time.
//! - Counted in rows, as the console counts (`World::add_output`): a screenful of
//!   wrapped prose is fewer lines than rows, and releasing a screenful of lines skipped
//!   the rows that didn't fit.
//! - Lines from a scrollback reply (`Line::scrollback`) are history, never arrivals:
//!   they neither count nor pause. A cold start from the disk cache catches up by gap
//!   fill, appended at the tail like live output; counted, the catch-up paused after a
//!   screenful and held the rest, and the window opened hundreds of lines back.
//!
//! One per world (`ClayView::more`), as the console pages every world on its own
//! (`World::add_output`): a world not on screen counts and holds its arrivals too, a
//! switch shows the world's own held state, and leaving a world counts afresh only when it
//! holds nothing (`left`, the console's `switch_world`). app.js does the same. Before this
//! a switch dropped the hold, so a backlog came out all at once.

use std::collections::{HashSet, VecDeque};

use crate::native_gui::core::client::{Line, World};

#[derive(Default)]
pub struct MoreMode {
    pub paused: bool,
    /// Held lines, oldest first, with the rows each takes.
    held: VecDeque<(u64, usize)>,
    held_set: HashSet<u64>,
    /// Rows shown since the user last caught up.
    since_pause: usize,
    /// The newest line id already seen at the tail of the shown world.
    last_tail: Option<u64>,
    /// Wheel movement past the bottom not yet worth a whole row.
    pub wheel_px: f32,
    /// Lines `arrivals` found shown at once (not held) since `take_arrived`, and lines let
    /// through since `take_released`: the text-reveal effect types both on from that moment
    /// (`ClayView::reveal_stamp`).
    arrived: Vec<u64>,
    released: Vec<u64>,
}

impl MoreMode {
    /// Start over: nothing held, and what's there now counts as seen (the tests' starting
    /// point; a world's state is made fresh by `ClayView::more` on its first arrivals).
    #[cfg(test)]
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

    /// How many lines are held.
    pub fn held(&self) -> usize {
        self.held.len()
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Lines that arrived at the tail of `world` since the last call. `threshold` is in
    /// rows, and `rows` says how many a line takes (0: it isn't drawn). Returns whether
    /// output paused just now (the view then goes to the bottom, as app.js does).
    pub fn arrivals(&mut self, world: &World, enabled: bool, threshold: usize, rows: &mut dyn FnMut(&Line) -> usize) -> bool {
        let since = self.last_tail;
        let start = world.lines.iter().rposition(|l| Some(l.id) == since).map_or_else(
            // The last seen line is gone (a flush): everything is new, but nothing to hold.
            || world.lines.len(),
            |p| p + 1,
        );
        self.last_tail = world.lines.last().map(|l| l.id).or(since);
        let mut paused_now = false;
        for line in &world.lines[start..] {
            // Gagged lines don't count (app.js); while paused, a line not held is still
            // kept from view by the held line before it.
            if line.gagged {
                continue;
            }
            // Not paused: history caught up on, and lines the server never numbered
            // (released ones, from an older server, and this client's own notices),
            // aren't arrivals; with more-mode off, nothing is counted. Shown at once all
            // the same, unless they are history.
            if !self.paused && (line.scrollback || line.seq.is_none() || !enabled) {
                if !line.scrollback {
                    self.arrived.push(line.id);
                }
                continue;
            }
            let cost = rows(line);
            if cost == 0 {
                continue;
            }
            if self.paused {
                self.hold(line.id, cost);
            } else if self.since_pause > 0 && self.since_pause + cost > threshold.max(1) {
                self.paused = true;
                paused_now = true;
                self.hold(line.id, cost);
            } else {
                self.since_pause += cost;
                self.arrived.push(line.id);
            }
        }
        paused_now
    }

    fn hold(&mut self, id: u64, rows: usize) {
        if self.held_set.insert(id) {
            self.held.push_back((id, rows));
        }
    }

    fn pop(&mut self) -> Option<usize> {
        let (id, rows) = self.held.pop_front()?;
        self.held_set.remove(&id);
        self.released.push(id);
        Some(rows)
    }

    /// The lines let through since the last call (`pop`), oldest first.
    pub fn take_released(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.released)
    }

    /// The lines `arrivals` has shown at once since the last call, oldest first.
    pub fn take_arrived(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.arrived)
    }

    fn drained(&mut self) {
        if self.held.is_empty() {
            self.paused = false;
            self.since_pause = 0;
        }
    }

    /// Show held lines, oldest first, as many as fit in `budget` rows (at least one, so
    /// a line taller than the budget still comes out). Returns the rows shown.
    pub fn release_rows(&mut self, budget: usize) -> usize {
        let mut used = 0;
        while let Some(&(_, rows)) = self.held.front() {
            if used > 0 && used + rows > budget {
                break;
            }
            used += self.pop().unwrap_or(0);
        }
        self.drained();
        used
    }

    /// Show up to `n` held lines; returns how many.
    pub fn release(&mut self, n: usize) -> usize {
        let mut k = 0;
        while k < n && self.pop().is_some() {
            k += 1;
        }
        self.drained();
        k
    }

    pub fn release_all(&mut self) {
        self.release(usize::MAX);
    }

    /// The user caught up (released output, sent a command): count afresh.
    pub fn caught_up(&mut self) {
        self.since_pause = 0;
    }

    /// The user switched away: count afresh unless lines are held, which keep their
    /// pause for the next visit (the console's `switch_world`).
    pub fn left(&mut self) {
        if self.held.is_empty() {
            self.paused = false;
            self.since_pause = 0;
        }
        self.wheel_px = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            scrollback: false,
        }
    }

    /// A line from a scrollback reply (a gap fill).
    fn caught_up_line(id: u64) -> Line {
        Line { scrollback: true, ..line(id) }
    }

    /// A client line (an action's /echo): numbered by the server, not from the MUD.
    fn client_line(id: u64) -> Line {
        Line { from_server: false, ..line(id) }
    }

    fn one_row(_: &Line) -> usize {
        1
    }

    #[test]
    fn holds_after_a_screenful_and_releases_in_order() {
        let mut w = World::default();
        w.lines = (1..=3).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((4..=10).map(line));
        let paused = m.arrivals(&w, true, 5, &mut one_row);
        assert!(paused);
        // 4..=8 shown (5 rows); the 6th arrival paused, so 9 and 10 are held.
        assert_eq!(m.held(), 2);
        assert!(m.is_held(9) && m.is_held(10) && !m.is_held(8));
        assert_eq!(m.release(1), 1);
        assert!(!m.is_held(9) && m.is_held(10));
        m.release_all();
        assert!(!m.paused && m.held() == 0);
    }

    /// The text-reveal effect is told what was shown at once and what was let through.
    #[test]
    fn arrivals_shown_at_once_and_releases_are_reported() {
        let mut w = World::default();
        w.lines = (1..=3).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        assert!(m.take_arrived().is_empty(), "the backlog is not an arrival");
        w.lines.extend((4..=8).map(line));
        w.lines.push(caught_up_line(20));
        assert!(!m.arrivals(&w, true, 5, &mut one_row));
        assert_eq!(m.take_arrived(), vec![4, 5, 6, 7, 8], "shown at once; a gap fill is history");
        w.lines.extend((9..=12).map(line));
        assert!(m.arrivals(&w, true, 5, &mut one_row));
        assert!(m.take_arrived().is_empty(), "held lines wait for their release");
        assert_eq!(m.release(1), 1);
        assert_eq!(m.take_released(), vec![9]);
        m.release_all();
        assert_eq!(m.take_released(), vec![10, 11, 12]);
        assert!(m.take_released().is_empty());
    }

    #[test]
    fn off_means_nothing_is_held_and_gap_fills_never_count() {
        let mut w = World::default();
        w.lines = (1..=3).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((4..=50).map(line));
        assert!(!m.arrivals(&w, false, 5, &mut one_row));
        assert_eq!(m.held(), 0);
        // A line inserted mid-buffer (a gap fill) isn't an arrival.
        w.lines.insert(10, line(99));
        assert!(!m.arrivals(&w, true, 1, &mut one_row));
        assert_eq!(m.held(), 0);
    }

    /// The cold start: the cached buffer, then the gap fill appended at its tail - far
    /// more than a screenful, none of it an arrival.
    #[test]
    fn a_catch_up_at_the_tail_is_never_held_and_never_pauses() {
        let mut w = World::default();
        w.lines = (1..=20).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((21..=320).map(caught_up_line));
        assert!(!m.arrivals(&w, true, 22, &mut one_row));
        assert_eq!(m.held(), 0);
        assert!(!m.paused);
        // Live output after it counts from where the count stood: 22 rows show, the
        // 23rd pauses.
        w.lines.extend((321..=330).map(line));
        assert!(!m.arrivals(&w, true, 22, &mut one_row));
        w.lines.extend((331..=345).map(line));
        assert!(m.arrivals(&w, true, 22, &mut one_row));
        assert_eq!(m.held(), 3);
        assert!(m.is_held(343) && !m.is_held(342));
    }

    /// A gap fill landing while paused is newer than what's held: it waits behind it.
    #[test]
    fn a_catch_up_while_paused_waits_behind_the_hold() {
        let mut w = World::default();
        w.lines = (1..=2).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((3..=8).map(line));
        assert!(m.arrivals(&w, true, 4, &mut one_row));
        w.lines.extend((9..=10).map(caught_up_line));
        m.arrivals(&w, true, 4, &mut one_row);
        assert!(m.is_held(9) && m.is_held(10));
        assert_eq!(m.held(), 4);
    }

    /// Once paused, the client's own lines are held too, so none is drawn below a held
    /// line (where Page Down's releases would go in above it).
    #[test]
    fn while_paused_client_lines_are_held_in_order() {
        let mut w = World::default();
        w.lines = (1..=2).map(line).collect();
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((3..=6).map(line));
        w.lines.push(client_line(7));
        w.lines.push(line(8));
        w.lines.push(client_line(9));
        assert!(m.arrivals(&w, true, 3, &mut one_row));
        // 3, 4, 5 shown; 6 paused; the rest waits behind it, in order.
        for id in 6..=9 {
            assert!(m.is_held(id), "{id} held");
        }
        assert_eq!(m.release(2), 2);
        assert!(!m.is_held(7) && m.is_held(8));
    }

    /// Client lines count for the screenful as the MUD's do (app.js counts them).
    #[test]
    fn client_lines_count_toward_the_screenful() {
        let mut w = World::default();
        w.lines = vec![line(1)];
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend([client_line(2), line(3), client_line(4)]);
        assert!(m.arrivals(&w, true, 2, &mut one_row));
        assert!(m.is_held(4) && !m.is_held(3));
    }

    /// Rows, not lines: three two-row lines fill a six-row screenful.
    #[test]
    fn the_screenful_is_counted_in_rows() {
        let mut w = World::default();
        w.lines = vec![line(1)];
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((2..=10).map(line));
        let mut two_rows = |_: &Line| 2;
        assert!(m.arrivals(&w, true, 6, &mut two_rows));
        // 2, 3, 4 shown (6 rows); 5 on held.
        assert!(m.is_held(5) && !m.is_held(4));
        // A release of six rows shows three of them; a release always shows at least
        // one line, however tall.
        assert_eq!(m.release_rows(6), 6);
        assert!(!m.is_held(7) && m.is_held(8));
        assert_eq!(m.release_rows(1), 2);
        assert!(!m.is_held(8) && m.is_held(9));
        // A single line taller than the screenful still shows before pausing.
        let mut w = World::default();
        w.lines = vec![line(1)];
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend([line(2), line(3)]);
        let mut tall = |_: &Line| 30;
        assert!(m.arrivals(&w, true, 22, &mut tall));
        assert!(!m.is_held(2) && m.is_held(3));
    }

    /// Lines that aren't drawn (gagged, or `rows` 0) are neither counted nor held.
    /// A world not on screen holds its backlog too, and keeps it through a switch away and
    /// back; leaving counts afresh only when nothing is held.
    #[test]
    fn leaving_keeps_a_hold_and_resets_only_an_empty_count() {
        let mut w = World::default();
        w.lines = vec![line(1)];
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((2..=4).map(line));
        assert!(!m.arrivals(&w, true, 5, &mut one_row));
        // Three rows shown of five: leaving with nothing held starts the count over.
        m.left();
        w.lines.extend((5..=10).map(line));
        assert!(m.arrivals(&w, true, 5, &mut one_row), "six more rows: five fit, the sixth pauses");
        assert_eq!(m.held(), 1);
        // Held lines survive a switch away and back.
        m.left();
        assert!(m.paused && m.held() == 1 && m.is_held(10));
        w.lines.extend((11..=13).map(line));
        assert!(!m.arrivals(&w, true, 5, &mut one_row), "already paused: held, not paused anew");
        assert_eq!(m.held(), 4);
        m.release_all();
        assert!(!m.paused);
    }

    #[test]
    fn undrawn_lines_dont_count() {
        let mut w = World::default();
        w.lines = vec![line(1)];
        let mut m = MoreMode::default();
        m.reset(Some(&w));
        w.lines.extend((2..=9).map(|id| Line { gagged: id % 2 == 0, ..line(id) }));
        assert!(!m.arrivals(&w, true, 4, &mut one_row));
        let mut hidden = |l: &Line| usize::from(l.id != 11);
        w.lines.extend([line(10), line(11), line(12)]);
        assert!(m.arrivals(&w, true, 4, &mut hidden));
        assert!(m.is_held(10) && !m.is_held(11) && m.is_held(12));
        assert_eq!(m.held(), 2);
    }
}
