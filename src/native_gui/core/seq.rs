//! Which seqs of a world's output this client holds (app.js `_seenRanges`).
//!
//! The server numbers every line of a world (`seq`), and a client dedups by exact
//! membership: a line is dropped only when its own seq has already arrived. The record is
//! a sorted list of merged inclusive ranges, so the usual case (everything since connect)
//! is one range however long the session runs.
//!
//! Not a high-water mark: the web client's old `_max_seq` ratchet, pushed past the
//! client's real buffer by any server-side ordering slip, then ate that world's output
//! permanently and re-dropped its own repair (PROTOCOL-ROADMAP.md Phase C).

/// More than this many separate ranges and the two oldest are merged (app.js
/// `MAX_SEEN_SEQ_RANGES`): giving up on the oldest hole bounds the record's size.
pub const MAX_RANGES: usize = 512;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeenRanges {
    /// Inclusive, sorted, non-overlapping and non-adjacent.
    ranges: Vec<(u64, u64)>,
}

impl SeenRanges {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `start..=end` as delivered (app.js `markSeqRangeSeen`). Merges with any range
    /// it overlaps or touches.
    pub fn mark(&mut self, start: u64, end: u64) {
        if end < start {
            return;
        }
        let first = self.ranges.partition_point(|r| r.1.saturating_add(1) < start);
        let mut last = first;
        let (mut s, mut e) = (start, end);
        while last < self.ranges.len() && self.ranges[last].0 <= e.saturating_add(1) {
            s = s.min(self.ranges[last].0);
            e = e.max(self.ranges[last].1);
            last += 1;
        }
        self.ranges.splice(first..last, [(s, e)]);
        if self.ranges.len() > MAX_RANGES {
            self.ranges[1].0 = self.ranges[0].0;
            self.ranges.remove(0);
        }
    }

    pub fn contains(&self, seq: u64) -> bool {
        let i = self.ranges.partition_point(|r| r.1 < seq);
        i < self.ranges.len() && self.ranges[i].0 <= seq
    }

    /// Whether one range holds all of `start..=end` (app.js `hasSeenRange`).
    pub fn contains_range(&self, start: u64, end: u64) -> bool {
        let i = self.ranges.partition_point(|r| r.1 < start);
        i < self.ranges.len() && self.ranges[i].0 <= start && end <= self.ranges[i].1
    }

    /// The top of the first range: every seq from where this client's record starts up to
    /// here has arrived (app.js `contiguousFrontier`). What the client acks and resumes
    /// from. 0 when nothing has arrived.
    pub fn frontier(&self) -> u64 {
        self.ranges.first().map_or(0, |r| r.1)
    }

    /// The highest seq held (app.js `maxSeenSeq`).
    pub fn max_seen(&self) -> u64 {
        self.ranges.last().map_or(0, |r| r.1)
    }

    /// The first missing run above the frontier, if there is one.
    pub fn oldest_hole(&self) -> Option<(u64, u64)> {
        (self.ranges.len() >= 2).then(|| (self.ranges[0].1 + 1, self.ranges[1].0 - 1))
    }

    /// Give up on the oldest hole: treat it as delivered, so the frontier moves past it
    /// (app.js `closeOldestSeqHole`). Returns whether there was one.
    pub fn close_oldest_hole(&mut self) -> bool {
        if self.ranges.len() < 2 {
            return false;
        }
        self.ranges[1].0 = self.ranges[0].0;
        self.ranges.remove(0);
        true
    }

    /// Anything delivered at all (seq 0 is a real seq).
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn ranges(&self) -> &[(u64, u64)] {
        &self.ranges
    }

    pub fn clear(&mut self) {
        self.ranges.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// A small deterministic generator, so a failure reproduces.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    fn model_ranges(set: &BTreeSet<u64>) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        for &s in set {
            match out.last_mut() {
                Some(r) if r.1 + 1 == s => r.1 = s,
                _ => out.push((s, s)),
            }
        }
        out
    }

    #[test]
    fn matches_a_set_of_seqs_whatever_the_order() {
        for seed in 0..200 {
            let mut rng = Lcg(seed);
            let mut seen = SeenRanges::new();
            let mut model = BTreeSet::new();
            for _ in 0..60 {
                let s = rng.next(300);
                let e = s + rng.next(6);
                seen.mark(s, e);
                model.extend(s..=e);
                assert_eq!(seen.ranges(), model_ranges(&model).as_slice(), "seed {seed}");
                for probe in 0..310 {
                    assert_eq!(seen.contains(probe), model.contains(&probe));
                }
                let (a, b) = (rng.next(300), rng.next(8));
                assert_eq!(seen.contains_range(a, a + b), (a..=a + b).all(|x| model.contains(&x)));
            }
        }
    }

    #[test]
    fn frontier_hole_and_closing_it() {
        let mut s = SeenRanges::new();
        assert_eq!((s.frontier(), s.max_seen(), s.oldest_hole()), (0, 0, None));
        s.mark(0, 9);
        s.mark(15, 20);
        s.mark(30, 30);
        assert_eq!(s.frontier(), 9);
        assert_eq!(s.max_seen(), 30);
        assert_eq!(s.oldest_hole(), Some((10, 14)));
        assert!(s.close_oldest_hole());
        assert_eq!(s.frontier(), 20);
        assert_eq!(s.oldest_hole(), Some((21, 29)));
        // Filling the hole merges the ranges.
        s.mark(21, 29);
        assert_eq!(s.ranges(), &[(0, 30)]);
        assert!(!s.close_oldest_hole());
    }

    #[test]
    fn a_reversed_span_records_nothing() {
        let mut s = SeenRanges::new();
        s.mark(5, 4);
        assert!(s.is_empty());
    }

    #[test]
    fn too_many_ranges_gives_up_the_oldest_hole() {
        let mut s = SeenRanges::new();
        for k in 0..(MAX_RANGES as u64 + 1) {
            s.mark(k * 2, k * 2);
        }
        assert_eq!(s.ranges().len(), MAX_RANGES);
        assert_eq!(s.ranges()[0], (0, 2));
    }
}
