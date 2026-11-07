//! `CLAY_NATIVE_PERF=1`: a timing trace of the native GUI's output path, one line a
//! second in `~/.clay/debug.log`, for diagnosing "the GUI lags behind" without a
//! profiler on the user's machine. Counted: the messages handled per wake of the event
//! loop and what handling them cost (the client core, `apply_settings`, `follow_world`),
//! the frames drawn and what `prepare`/`paint` cost, how many lines the shown world holds
//! and how many were laid out, and the latency from a message's arrival off the socket to
//! the first frame drawn after it was handled. Nothing here runs unless the variable is
//! set: every call site checks `enabled()` first (one cached load).

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CLAY_NATIVE_PERF").is_some_and(|v| !v.is_empty() && v != "0"))
}

#[derive(Default)]
struct Stats {
    messages: u64,
    batches: u64,
    /// The largest batch (messages handled in one wake).
    batch_max: u64,
    handle: Duration,
    settings: Duration,
    settings_calls: u64,
    follow: Duration,
    frames: u64,
    prepare: Duration,
    prepare_max: Duration,
    paint: Duration,
    /// The shown world's line count and the lines laid out, at the last frame.
    lines: usize,
    laid: usize,
    /// Arrival times of messages not yet handled (the pump pushes, the view pops).
    queued: VecDeque<Instant>,
    /// The earliest arrival among the messages handled since the last frame.
    unshown_since: Option<Instant>,
    latency_max: Duration,
    latency_sum: Duration,
    latency_n: u64,
    last_report: Option<Instant>,
}

fn stats() -> &'static Mutex<Stats> {
    static STATS: OnceLock<Mutex<Stats>> = OnceLock::new();
    STATS.get_or_init(|| Mutex::new(Stats::default()))
}

fn with(f: impl FnOnce(&mut Stats)) {
    if let Ok(mut s) = stats().lock() {
        f(&mut s);
    }
}

/// A message came off the socket (the pump, before it is queued for the view).
pub fn received() {
    with(|s| {
        s.queued.push_back(Instant::now());
        if s.queued.len() > 100_000 {
            s.queued.pop_front();
        }
    });
}

/// The view handled `n` messages in one wake, in `took` all told.
pub fn handled(n: u64, took: Duration) {
    with(|s| {
        s.messages += n;
        s.batches += 1;
        s.batch_max = s.batch_max.max(n);
        s.handle += took;
        for _ in 0..n {
            if let Some(at) = s.queued.pop_front() {
                // An arrival more than a few seconds old is a message the view never
                // handled through its loop (the login's, before the first frame): not a
                // latency, or one reading would put twelve seconds on the first second.
                if at.elapsed() < Duration::from_secs(5) {
                    s.unshown_since = Some(s.unshown_since.map_or(at, |u| u.min(at)));
                }
            }
        }
    });
}

pub fn settings_applied(took: Duration) {
    with(|s| {
        s.settings += took;
        s.settings_calls += 1;
    });
}

pub fn world_followed(took: Duration) {
    with(|s| s.follow += took);
}

/// A frame's layout is done: `lines` in the shown world, `laid` of them laid out.
pub fn frame_prepared(took: Duration, lines: usize, laid: usize) {
    with(|s| {
        s.frames += 1;
        s.prepare += took;
        s.prepare_max = s.prepare_max.max(took);
        s.lines = lines;
        s.laid = laid;
        if let Some(since) = s.unshown_since.take() {
            let lat = since.elapsed();
            s.latency_max = s.latency_max.max(lat);
            s.latency_sum += lat;
            s.latency_n += 1;
        }
    });
}

pub fn frame_painted(took: Duration) {
    with(|s| s.paint += took);
}

/// From the view's clock: one line a second while anything happened.
pub fn report() {
    let line = {
        let Ok(mut s) = stats().lock() else { return };
        let now = Instant::now();
        let since = s.last_report.map_or(Duration::MAX, |t| now.duration_since(t));
        if since < Duration::from_secs(1) {
            return;
        }
        s.last_report = Some(now);
        if s.messages == 0 && s.frames == 0 {
            return;
        }
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let per = |d: Duration, n: u64| if n == 0 { 0.0 } else { ms(d) / n as f64 };
        let line = format!(
            "native-gui perf: {} msgs in {} wakes (max {}) handle {:.1} ms ({:.2}/msg; settings {:.1} ms x{}, follow {:.1} ms); {} frames prepare {:.1} ms ({:.2}/frame, max {:.2}) paint {:.1} ms ({:.2}/frame); lines {} laid {}; latency recv->frame mean {:.1} ms max {:.1} ms over {}; queued {}",
            s.messages,
            s.batches,
            s.batch_max,
            ms(s.handle),
            per(s.handle, s.messages),
            ms(s.settings),
            s.settings_calls,
            ms(s.follow),
            s.frames,
            ms(s.prepare),
            per(s.prepare, s.frames),
            ms(s.prepare_max),
            ms(s.paint),
            per(s.paint, s.frames),
            s.lines,
            s.laid,
            per(s.latency_sum, s.latency_n),
            ms(s.latency_max),
            s.latency_n,
            s.queued.len(),
        );
        let (queued, unshown, last) = (std::mem::take(&mut s.queued), s.unshown_since, s.last_report);
        *s = Stats { queued, unshown_since: unshown, last_report: last, ..Default::default() };
        line
    };
    crate::debug_log(true, &line);
}
