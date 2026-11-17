//! The size popup: while a window is resized, a small box in its middle says how many
//! characters the output holds, `80x24` (user request 2026-11-15: "similar to how
//! xfce4-terminal displays things").
//!
//! What xfce4-terminal shows is the window manager's doing (xfwm4's `poswin`), and only
//! for a window that publishes a cell size, which Clay's don't: so every interface with
//! a window draws its own - the native GUI (`native_gui::ui::sizepopup`) and the web
//! page (app.js `noteWindowResize`, which is `ResizeWatch::observe` ported). The consoles
//! have none, by the request: a terminal's size is its window manager's to show.
//!
//! The rules, each a test here:
//!
//! - the label is the output's columns and rows, the pair that interface reports to the
//!   server (`UpdateViewState`); while the output is split (`/split`) it is the output as
//!   if it weren't - the two panes and the bar between them as one pane - never the
//!   focused pane's own (user request 2026-11-15; each drawer measures it);
//! - it shows when the window's **size** changes: never for a move, never for the
//!   program's own resize (the native GUI's zoom - xfwm4 shows nothing for a programmatic
//!   resize either), never for a change of scale (another monitor, a browser's zoom);
//! - nothing shows while a window is starting (`START_QUIET_TICKS`): a window manager may
//!   size a new window more than once before the user has touched it;
//! - nothing says when a drag ends, so the popup stays `LINGER` past the last change.
//!
//! This file knows nothing of GPUI (plain logical pixels), so the rules are unit-tested
//! in every build and `src/tests.rs` pins app.js to them.

use std::time::Duration;

/// How long the popup outlives the last size change. app.js `SIZE_POPUP_LINGER_MS`.
pub const LINGER: Duration = Duration::from_millis(1000);
/// Ticks of the view's 250 ms clock a new window stays quiet for: its first second.
/// Counted in ticks, not wall time, because a tick only runs once the event loop does:
/// a slow first frame can't use the quiet period up before the window manager's first
/// resizes are read. app.js `SIZE_POPUP_QUIET_MS` is the same second.
pub const START_QUIET_TICKS: u8 = 4;

/// The popup's text: columns, an `x`, rows. app.js `sizePopupLabel`.
pub fn label(cols: usize, rows: usize) -> String {
    format!("{cols}x{rows}")
}

/// Tells a resize the popup shows from everything else a window reports about its bounds.
#[derive(Clone, Copy, Debug)]
pub struct ResizeWatch {
    /// The window's content size as last recorded, in logical pixels.
    w: f32,
    h: f32,
    scale: f32,
    /// Ticks left of the start-up quiet period.
    quiet: u8,
}

impl ResizeWatch {
    /// The window as it was built: the size every later report is compared with.
    pub fn new(w: f32, h: f32, scale: f32) -> Self {
        Self { w, h, scale, quiet: START_QUIET_TICKS }
    }

    /// The view's clock (250 ms): the quiet period runs down.
    pub fn tick(&mut self) {
        self.quiet = self.quiet.saturating_sub(1);
    }

    /// The window reported its bounds: is this a resize to show the popup for?
    /// `own`: the program's own resize (the zoom's), recorded but never shown - so the
    /// reports that follow it at the same size (a window manager sends several) are no
    /// change either.
    pub fn observe(&mut self, w: f32, h: f32, scale: f32, own: bool) -> bool {
        if (scale - self.scale).abs() > 0.001 {
            // Another monitor's scale (or a browser's zoom): the size in logical pixels
            // moves with it and nobody resized anything.
            *self = Self { w, h, scale, quiet: self.quiet };
            return false;
        }
        // A move, or rounding. The record is kept as it was, so a drag slower than the
        // tolerance still adds up to a change.
        let tolerance = crate::zoom::SIZE_TOLERANCE;
        if (w - self.w).abs() <= tolerance && (h - self.h).abs() <= tolerance {
            return false;
        }
        self.w = w;
        self.h = h;
        !own && self.quiet == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window past its first second.
    fn settled(w: f32, h: f32) -> ResizeWatch {
        let mut watch = ResizeWatch::new(w, h, 1.0);
        for _ in 0..START_QUIET_TICKS {
            watch.tick();
        }
        watch
    }

    #[test]
    fn the_label_is_columns_x_rows() {
        assert_eq!(label(80, 24), "80x24");
        assert_eq!(label(132, 50), "132x50");
    }

    #[test]
    fn a_move_is_not_a_resize() {
        let mut watch = settled(800.0, 600.0);
        // GPUI reports a move through the same observer, with the size as it was.
        assert!(!watch.observe(800.0, 600.0, 1.0, false));
        assert!(!watch.observe(800.0, 600.0, 1.0, false));
    }

    #[test]
    fn a_changed_size_shows_and_every_step_of_a_drag_does() {
        let mut watch = settled(800.0, 600.0);
        assert!(watch.observe(820.0, 600.0, 1.0, false), "wider");
        assert!(watch.observe(820.0, 640.0, 1.0, false), "taller");
        assert!(watch.observe(700.0, 500.0, 1.0, false), "smaller");
        assert!(!watch.observe(700.0, 500.0, 1.0, false), "the same again is no change");
    }

    #[test]
    fn a_starting_window_is_quiet_and_what_it_became_is_the_baseline() {
        let mut watch = ResizeWatch::new(800.0, 600.0, 1.0);
        // A tiling window manager sizes the new window before the user has seen it.
        assert!(!watch.observe(1280.0, 700.0, 1.0, false));
        for _ in 0..START_QUIET_TICKS - 1 {
            watch.tick();
            assert!(!watch.observe(1280.0, 701.0 + f32::from(watch.quiet) * 10.0, 1.0, false), "still quiet");
        }
        watch.tick();
        // The quiet period is over, and the size it ended at is not news.
        let (w, h) = (watch.w, watch.h);
        assert!(!watch.observe(w, h, 1.0, false));
        assert!(watch.observe(w + 40.0, h, 1.0, false));
    }

    #[test]
    fn the_programs_own_resize_is_recorded_and_never_shown() {
        let mut watch = settled(800.0, 600.0);
        // A zoom step: ours, then the window manager's repeats of it, which the zoom no
        // longer calls its own.
        assert!(!watch.observe(960.0, 720.0, 1.0, true));
        assert!(!watch.observe(960.0, 720.0, 1.0, false), "the echo is no change");
        assert!(!watch.observe(960.5, 719.5, 1.0, false), "nor is its rounding");
        // The user's drag after it shows.
        assert!(watch.observe(1000.0, 720.0, 1.0, false));
    }

    #[test]
    fn rounding_is_not_a_resize_but_a_slow_drag_adds_up() {
        let mut watch = settled(800.0, 600.0);
        assert!(!watch.observe(801.0, 600.0, 1.0, false));
        assert!(!watch.observe(801.5, 599.0, 1.0, false));
        // A pixel a report: each is within the tolerance of the one before, not of the
        // size recorded.
        assert!(watch.observe(802.0, 600.0, 1.0, false));
        assert!(!watch.observe(803.0, 600.0, 1.0, false));
        assert!(watch.observe(804.0, 600.0, 1.0, false));
    }

    #[test]
    fn a_change_of_scale_is_not_a_resize() {
        let mut watch = settled(800.0, 600.0);
        // Dragged to a 2x monitor: the logical size halves with the scale.
        assert!(!watch.observe(400.0, 300.0, 2.0, false));
        assert!(!watch.observe(400.0, 300.0, 2.0, false));
        // That size is the record now.
        assert!(watch.observe(500.0, 300.0, 2.0, false));
    }
}
