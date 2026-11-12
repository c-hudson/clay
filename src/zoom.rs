//! Window zoom: Ctrl+plus / Ctrl+minus / Ctrl+0 in a desktop GUI window, the way
//! xfce4-terminal does it (user request 2026-11-10).
//!
//! A zoom level scales the window's **text** by `STEP` per level, and the **window** with
//! it, in proportion, so the same amount of text keeps fitting. The rules, each a test:
//!
//! - levels run `MIN_LEVEL..=MAX_LEVEL` (xfce4-terminal's seven each way); a step past
//!   either end does nothing;
//! - the window at level 0 is the **base**; every other level is `base × STEP^level`;
//! - a window that would outgrow its screen **becomes the screen** instead: the whole
//!   usable area, frame included (`Frame`), whichever axis overflowed first (user report
//!   2026-11-10: a height clamped alone left the screen's left side empty and the title
//!   bar pushed the bottom off); once a level is clamped, zooming in again **silently
//!   does nothing** (the text stays too);
//! - zooming out from a clamped level returns to the previous level's size; arriving
//!   back at level 0 restores the base exactly, position included;
//! - a window the user has dragged to a new size while zoomed re-bases: later steps
//!   scale the user's size (a pure move re-anchors the origin);
//! - a maximized or fullscreen window changes only its text, and its size is never a base;
//! - everything is in memory, per window: nothing here touches settings, the server or
//!   another window.
//!
//! The struct knows nothing of GPUI (plain logical pixels), so the rules are unit-tested
//! in every build; `native_gui::ui::zoom` is the one caller, feeding it the window's
//! bounds and screen and applying the `Change` it answers with.

use std::time::{Duration, Instant};

/// xfce4-terminal's factor per level (Pango's scale table).
pub const STEP: f32 = 1.2;
/// Seven levels each way, like xfce4-terminal (user ruling 2026-11-10: shrinking below
/// 1:1 is allowed).
pub const MIN_LEVEL: i8 = -7;
pub const MAX_LEVEL: i8 = 7;
/// A size this close to the one we asked for is ours: HiDPI and window-manager rounding
/// is not a user resize.
pub const SIZE_TOLERANCE: f32 = 1.5;
/// A bounds report within this long of our own resize, at a size that is not yet the one
/// asked for, is an intermediate configure of ours, not the user's.
pub const EXPECT_WINDOW: Duration = Duration::from_secs(1);

/// A window's place and **content** size in logical pixels, global origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Whether `self`'s size fits inside `screen`'s (origin not considered).
    fn fits(&self, screen: &Rect) -> bool {
        self.w <= screen.w + SIZE_TOLERANCE && self.h <= screen.h + SIZE_TOLERANCE
    }

    fn same_size(&self, other: &Rect) -> bool {
        (self.w - other.w).abs() <= SIZE_TOLERANCE && (self.h - other.h).abs() <= SIZE_TOLERANCE
    }

    fn same_place(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= SIZE_TOLERANCE && (self.y - other.y).abs() <= SIZE_TOLERANCE
    }
}

/// What the window manager draws around a window's content (title bar, borders), in
/// logical pixels. The zoom works in **content** coordinates - a window is where its
/// content is - and the frame is taken off at the two edges: the screen a window may fill
/// is the work area less the frame (`inset`), and the place the platform is asked to put
/// the frame is the content's place less the frame's top-left (`outer`). Without it every
/// step walked the window down and right by the frame, and a window grown to the screen
/// hung its title bar's height off the bottom (xfwm4, user report 2026-11-10).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Frame {
    /// The part of `area` the content may occupy with the whole frame still inside it.
    pub fn inset(&self, area: Rect) -> Rect {
        Rect::new(
            area.x + self.left,
            area.y + self.top,
            (area.w - self.left - self.right).max(1.0),
            (area.h - self.top - self.bottom).max(1.0),
        )
    }

    /// Where the frame's top-left goes for content at (`x`, `y`).
    pub fn outer(&self, x: f32, y: f32) -> (f32, f32) {
        (x - self.left, y - self.top)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    In,
    Out,
    Reset,
}

impl Step {
    /// The action id's step, for the three ids bound by default to `^+`, `^-` and `^0`.
    pub fn from_action(id: &str) -> Option<Step> {
        match id {
            "zoom_in" => Some(Step::In),
            "zoom_out" => Some(Step::Out),
            "zoom_reset" => Some(Step::Reset),
            _ => None,
        }
    }
}

/// What a step changed: the text scale, and the window's new bounds when the window
/// moves with it (`None` for a maximized or fullscreen window, which keeps its size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub scale: f32,
    pub bounds: Option<Rect>,
}

/// Whose change a bounds report was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observed {
    /// The size we asked for (or one on the way to it).
    Ours,
    /// The user's (or the window manager's) own resize or move.
    Theirs,
}

#[derive(Debug, Default)]
pub struct ZoomState {
    level: i8,
    /// The window at level 0; captured on the first step away from 0, re-derived by a
    /// user's resize or move, dropped on arriving back at 0 (the window *is* the base).
    base: Option<Rect>,
    /// The bounds we asked the platform for and have not seen back yet.
    expected: Option<(Rect, Instant)>,
    /// The bounds the platform last confirmed for a step of ours: a later report of the
    /// same size and place is an echo (X11 reports a resize and a move apart), the same
    /// size elsewhere is the user's move, another size the user's resize.
    applied: Option<Rect>,
}

impl ZoomState {
    pub fn level(&self) -> i8 {
        self.level
    }

    /// The text scale at the current level.
    pub fn scale(&self) -> f32 {
        scale_at(self.level)
    }

    /// Zoomed at all (the text is not its saved size).
    pub fn active(&self) -> bool {
        self.level != 0
    }

    /// Take a step. `current` is the window now, `screen` its monitor's usable area when
    /// known, `resizable` false for a maximized or fullscreen window. `None` means
    /// nothing changes - not the level, not the text, not the window.
    pub fn step(&mut self, step: Step, current: Rect, screen: Option<Rect>, resizable: bool, now: Instant) -> Option<Change> {
        let new = match step {
            Step::Reset => {
                if self.level == 0 {
                    return None;
                }
                self.level = 0;
                let bounds = if resizable { self.base.take() } else { None };
                if let Some(b) = bounds {
                    self.expected = Some((b, now));
                }
                return Some(Change { scale: 1.0, bounds });
            }
            Step::In => self.level.checked_add(1).filter(|l| *l <= MAX_LEVEL)?,
            Step::Out => self.level.checked_sub(1).filter(|l| *l >= MIN_LEVEL)?,
        };

        if !resizable {
            // Only the text: a maximized window keeps the size the window manager gave it,
            // and that size is never a base.
            self.level = new;
            return Some(Change { scale: scale_at(new), bounds: None });
        }

        let base = *self.base.get_or_insert(current);
        let s = scale_at(new);
        let mut want = Rect::new(base.x, base.y, base.w * s, base.h * s);
        // Back at 1:1 the window is as it was, place included, whatever the screen; at
        // any other level it is kept on the screen.
        let screen = if new == 0 { None } else { screen };
        if let Some(screen) = screen {
            if !want.fits(&screen) {
                let here = Rect::new(0.0, 0.0, base.w * scale_at(self.level), base.h * scale_at(self.level));
                if step == Step::In && !here.fits(&screen) {
                    // Already as large as the screen: the user's rule is that this step
                    // silently does nothing at all.
                    return None;
                }
                // The user's rule: a window that would outgrow the screen becomes the
                // screen - all of it, not just the axis that overflowed.
                want = screen;
            } else {
                let (x, y) = fit_origin(base.x, base.y, want.w, want.h, &screen);
                want.x = x;
                want.y = y;
            }
        }

        self.level = new;
        if new == 0 {
            // Back at 1:1 the window is the base again; a later step recaptures it, so a
            // resize made meanwhile counts.
            self.base = None;
        }
        self.expected = Some((want, now));
        Some(Change { scale: s, bounds: Some(want) })
    }

    /// The platform reported the window's bounds (a resize or a move). Ours are consumed;
    /// the user's move the base along, or re-base the zoom so later steps scale the window
    /// they made.
    pub fn observed(&mut self, bounds: Rect, resizable: bool, now: Instant) -> Observed {
        if let Some((want, at)) = self.expected {
            let fresh = now.duration_since(at) < EXPECT_WINDOW;
            if want.same_size(&bounds) {
                // Our size, where the platform has it so far: X11 reports the resize at
                // the old place and the move after it, so the step is done once the place
                // is ours too - or once the window has passed, where a window manager (or
                // Wayland, which has no move) left it elsewhere.
                self.applied = Some(bounds);
                if want.same_place(&bounds) || !fresh {
                    self.expected = None;
                }
                return Observed::Ours;
            }
            if fresh {
                return Observed::Ours;
            }
            self.expected = None;
        }
        if !resizable {
            // A maximize is not a new base; the restore reports again.
            return Observed::Theirs;
        }
        let Some(applied) = self.applied else {
            // Nothing of ours is on the window yet: at level 0 the window is the base; at
            // any other level (zoomed while maximized, now restored) the window shows this
            // level's text, so it is this level's size and the base is it over the scale -
            // the next step then grows it by one step, as the text grows.
            if self.level == 0 {
                self.base = None;
            } else {
                let s = self.scale();
                self.base = Some(Rect::new(bounds.x, bounds.y, bounds.w / s, bounds.h / s));
                self.applied = Some(bounds);
            }
            return Observed::Theirs;
        };
        let (dx, dy) = (bounds.x - applied.x, bounds.y - applied.y);
        if applied.same_size(&bounds) {
            if applied.same_place(&bounds) {
                return Observed::Ours;
            }
            // The user moved it: the base goes along by the same distance.
            if let Some(b) = &mut self.base {
                b.x += dx;
                b.y += dy;
            }
            self.applied = Some(bounds);
            return Observed::Theirs;
        }
        // The user resized it: later steps scale this size (at level 0 the window is the
        // base itself).
        if self.level == 0 {
            self.base = None;
        } else {
            let s = self.scale();
            let (ox, oy) = self.base.map_or((bounds.x, bounds.y), |b| (b.x + dx, b.y + dy));
            self.base = Some(Rect::new(ox, oy, bounds.w / s, bounds.h / s));
        }
        self.applied = Some(bounds);
        Observed::Theirs
    }
}

/// The text scale at `level`: `STEP^level`.
pub fn scale_at(level: i8) -> f32 {
    STEP.powi(level as i32)
}

/// Where a window of `w`×`h` goes to stay on `screen` while keeping its place when it can
/// (the popup windows' `bounds_over` rule): shifted only as far as needed, so a window the
/// size of the screen sits at the screen's origin.
fn fit_origin(x: f32, y: f32, w: f32, h: f32, screen: &Rect) -> (f32, f32) {
    let max_x = (screen.x + screen.w - w).max(screen.x);
    let max_y = (screen.y + screen.h - h).max(screen.y);
    (x.clamp(screen.x, max_x), y.clamp(screen.y, max_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn rect_near(a: Rect, b: Rect) -> bool {
        near(a.x, b.x) && near(a.y, b.y) && near(a.w, b.w) && near(a.h, b.h)
    }

    const BASE: Rect = Rect::new(100.0, 50.0, 800.0, 600.0);

    #[test]
    fn scale_is_1_2_per_level_and_the_ends_are_no_ops() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        assert!(near(z.scale(), 1.0));
        for level in 1..=MAX_LEVEL {
            let c = z.step(Step::In, BASE, None, true, now).expect("in");
            assert!(near(c.scale, 1.2f32.powi(level as i32)), "level {level}");
            assert_eq!(z.level(), level);
        }
        assert!(near(z.scale(), 3.583));
        assert_eq!(z.step(Step::In, BASE, None, true, now), None, "past MAX_LEVEL nothing happens");
        assert_eq!(z.level(), MAX_LEVEL);
        let mut z = ZoomState::default();
        for _ in 0..7 {
            z.step(Step::Out, BASE, None, true, now).expect("out");
        }
        assert_eq!(z.level(), MIN_LEVEL);
        assert_eq!(z.step(Step::Out, BASE, None, true, now), None, "past MIN_LEVEL nothing happens");
    }

    #[test]
    fn in_then_out_restores_the_base_exactly() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::In, BASE, None, true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(100.0, 50.0, 960.0, 720.0)));
        // The platform answers with what we asked (ours), the window is not re-based.
        assert_eq!(z.observed(c.bounds.unwrap(), true, now), Observed::Ours);
        let c = z.step(Step::Out, Rect::new(100.0, 50.0, 960.0, 720.0), None, true, now).unwrap();
        assert_eq!(c.bounds, Some(BASE), "level 0 is the base, position included");
        assert!(near(c.scale, 1.0));
        assert!(!z.active());
    }

    #[test]
    fn the_screen_clamps_and_a_clamped_level_steps_no_further() {
        let screen = Rect::new(0.0, 0.0, 1280.0, 720.0);
        let start = Rect::new(300.0, 200.0, 800.0, 600.0);
        let mut z = ZoomState::default();
        let now = Instant::now();
        // Level 1: 960×720 fits in height exactly; the window moves up to stay on screen.
        let c = z.step(Step::In, start, Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(300.0, 0.0, 960.0, 720.0)), "{:?}", c.bounds);
        // Level 2 would be 1152×864, taller than the screen: the window becomes the
        // screen - the width too, not only the height that overflowed.
        let c = z.step(Step::In, c.bounds.unwrap(), Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(0.0, 0.0, 1280.0, 720.0)), "{:?}", c.bounds);
        assert!(near(c.scale, 1.44));
        // Level 2 is clamped: the next step does nothing at all.
        let at2 = c.bounds.unwrap();
        assert_eq!(z.step(Step::In, at2, Some(screen), true, now), None);
        assert_eq!(z.level(), 2);
        assert!(near(z.scale(), 1.44));
        // Out from the clamped level: the previous level's size and place.
        let c = z.step(Step::Out, at2, Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(300.0, 0.0, 960.0, 720.0)), "{:?}", c.bounds);
        // And back to 1:1: the original bounds, position included - even where the
        // original hung off the screen's bottom, as this one did.
        let c = z.step(Step::Out, c.bounds.unwrap(), Some(screen), true, now).unwrap();
        assert_eq!(c.bounds, Some(start));
    }

    #[test]
    fn a_window_the_size_of_the_screen_sits_at_its_origin() {
        let screen = Rect::new(1920.0, 0.0, 1280.0, 720.0); // a second monitor
        let start = Rect::new(2400.0, 300.0, 1100.0, 650.0);
        let mut z = ZoomState::default();
        let c = z.step(Step::In, start, Some(screen), true, Instant::now()).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(1920.0, 0.0, 1280.0, 720.0)), "{:?}", c.bounds);
    }

    /// The user's screenshot (2026-11-10): a window whose height overflowed first was
    /// clamped in height alone and left 265 px of the screen's left side empty, and the
    /// title bar pushed its bottom off the screen. With the frame taken off the work area
    /// the clamped window is the whole screen, frame included.
    #[test]
    fn a_clamped_window_is_the_whole_screen_frame_included() {
        let frame = Frame { left: 4.0, top: 23.0, right: 4.0, bottom: 4.0 };
        let work_area = Rect::new(0.0, 27.0, 1920.0, 1053.0); // a 27 px panel on top
        let screen = frame.inset(work_area);
        assert_eq!(screen, Rect::new(4.0, 50.0, 1912.0, 1026.0));
        let start = Rect::new(300.0, 200.0, 1100.0, 800.0);
        let mut z = ZoomState::default();
        let now = Instant::now();
        // Level 1: 1320×960 still fits; the window moves up only as far as it must.
        let c = z.step(Step::In, start, Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(300.0, 116.0, 1320.0, 960.0)), "{:?}", c.bounds);
        // Level 2: 1584×1152 is too tall, so the window is the whole screen - and the
        // frame's top-left, where the platform is asked to put it, is the work area's.
        let c = z.step(Step::In, c.bounds.unwrap(), Some(screen), true, now).unwrap();
        let at2 = c.bounds.unwrap();
        assert_eq!(at2, screen);
        assert_eq!(frame.outer(at2.x, at2.y), (0.0, 27.0));
        assert_eq!(z.step(Step::In, at2, Some(screen), true, now), None, "clamped: nothing");
        // Out: the previous level's size, where it was.
        let c = z.step(Step::Out, at2, Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(300.0, 116.0, 1320.0, 960.0)), "{:?}", c.bounds);
    }

    #[test]
    fn the_frame_comes_off_the_screen_and_goes_back_on_the_move() {
        let frame = Frame { left: 1.0, top: 30.0, right: 1.0, bottom: 1.0 };
        assert_eq!(frame.inset(Rect::new(0.0, 0.0, 1280.0, 800.0)), Rect::new(1.0, 30.0, 1278.0, 769.0));
        assert_eq!(frame.outer(1.0, 30.0), (0.0, 0.0));
        // No window manager, no frame: nothing changes.
        assert_eq!(Frame::default().inset(Rect::new(5.0, 6.0, 7.0, 8.0)), Rect::new(5.0, 6.0, 7.0, 8.0));
        assert_eq!(Frame::default().outer(5.0, 6.0), (5.0, 6.0));
        // A frame wider than the area leaves a sliver, never a negative size.
        assert_eq!(frame.inset(Rect::new(0.0, 0.0, 1.0, 1.0)).w, 1.0);
    }

    #[test]
    fn reset_restores_size_and_position_and_is_nothing_at_zero() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        assert_eq!(z.step(Step::Reset, BASE, None, true, now), None);
        z.step(Step::In, BASE, None, true, now).unwrap();
        z.step(Step::In, Rect::new(100.0, 50.0, 960.0, 720.0), None, true, now).unwrap();
        let c = z.step(Step::Reset, Rect::new(100.0, 50.0, 1152.0, 864.0), None, true, now).unwrap();
        assert_eq!(c, Change { scale: 1.0, bounds: Some(BASE) });
        assert_eq!(z.level(), 0);
    }

    #[test]
    fn negative_levels_shrink_and_crossing_zero_restores_the_base() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::Out, BASE, None, true, now).unwrap();
        assert!(near(c.scale, 1.0 / 1.2));
        assert!(rect_near(c.bounds.unwrap(), Rect::new(100.0, 50.0, 666.67, 500.0)), "{:?}", c.bounds);
        let c = z.step(Step::In, c.bounds.unwrap(), None, true, now).unwrap();
        assert_eq!(c.bounds, Some(BASE));
        assert!(near(c.scale, 1.0));
    }

    #[test]
    fn maximized_changes_only_the_text_and_unmaximizing_rebases() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let maxed = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let c = z.step(Step::In, maxed, Some(maxed), false, now).unwrap();
        assert_eq!(c, Change { scale: 1.2, bounds: None });
        assert_eq!(z.level(), 1);
        // Un-maximized: the window manager restores 800×600; at level 1 that means a base
        // of 800/1.2 × 600/1.2, so the next step scales from the restored size.
        assert_eq!(z.observed(Rect::new(100.0, 50.0, 800.0, 600.0), true, now), Observed::Theirs);
        let c = z.step(Step::In, Rect::new(100.0, 50.0, 800.0, 600.0), None, true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(100.0, 50.0, 960.0, 720.0)), "{:?}", c.bounds);
    }

    #[test]
    fn a_user_resize_rederives_the_base() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::In, Rect::new(0.0, 0.0, 1000.0, 750.0), None, true, now).unwrap();
        assert_eq!(z.observed(c.bounds.unwrap(), true, now), Observed::Ours);
        // The user drags the window to 1320×990 while at level 1: the base becomes
        // 1100×825, so the next step is 1584×1188.
        let later = now + Duration::from_secs(5);
        assert_eq!(z.observed(Rect::new(0.0, 0.0, 1320.0, 990.0), true, later), Observed::Theirs);
        let c = z.step(Step::In, Rect::new(0.0, 0.0, 1320.0, 990.0), None, true, later).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(0.0, 0.0, 1584.0, 1188.0)), "{:?}", c.bounds);
        // And out twice lands on the re-derived base.
        z.step(Step::Out, c.bounds.unwrap(), None, true, later).unwrap();
        let c = z.step(Step::Out, Rect::new(0.0, 0.0, 1320.0, 990.0), None, true, later).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(0.0, 0.0, 1100.0, 825.0)), "{:?}", c.bounds);
    }

    #[test]
    fn our_own_resize_is_not_the_users() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::In, BASE, None, true, now).unwrap();
        let want = c.bounds.unwrap();
        // Half a pixel off (HiDPI rounding) is still ours.
        assert_eq!(z.observed(Rect::new(want.x, want.y, want.w + 0.5, want.h - 0.5), true, now), Observed::Ours);
        // A later report of a different size is the user's.
        assert_eq!(z.observed(Rect::new(want.x, want.y, 500.0, 400.0), true, now + Duration::from_secs(2)), Observed::Theirs);
        assert_eq!(z.step(Step::Out, Rect::new(want.x, want.y, 500.0, 400.0), None, true, now).unwrap().bounds,
            Some(Rect::new(want.x, want.y, 500.0 / 1.2, 400.0 / 1.2)));
    }

    #[test]
    fn an_intermediate_configure_of_ours_is_ours() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::In, BASE, None, true, now).unwrap();
        // The window manager reports a size on the way (still within the window).
        assert_eq!(z.observed(Rect::new(100.0, 50.0, 900.0, 650.0), true, now + Duration::from_millis(100)), Observed::Ours);
        assert_eq!(z.observed(c.bounds.unwrap(), true, now + Duration::from_millis(200)), Observed::Ours);
    }

    /// X11 reports our resize and our move as two bounds changes; the second, at a level
    /// clamped to the screen, used to be taken for the user's resize and re-based from the
    /// clamped size (found on Xvfb 2026-11-10: a third Ctrl+plus then grew the window to
    /// the whole screen). An echo of what we applied changes nothing.
    #[test]
    fn an_echo_of_our_own_bounds_at_a_clamped_level_changes_nothing() {
        let screen = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c1 = z.step(Step::In, Rect::new(240.0, 100.0, 800.0, 600.0), Some(screen), true, now).unwrap();
        assert_eq!(z.observed(c1.bounds.unwrap(), true, now), Observed::Ours);
        let c2 = z.step(Step::In, c1.bounds.unwrap(), Some(screen), true, now).unwrap();
        let at2 = c2.bounds.unwrap();
        assert!(rect_near(at2, Rect::new(0.0, 0.0, 1280.0, 800.0)), "{at2:?}");
        assert_eq!(z.observed(at2, true, now), Observed::Ours, "the resize");
        assert_eq!(z.observed(at2, true, now + Duration::from_secs(2)), Observed::Ours, "the move, reported apart and late");
        assert_eq!(z.step(Step::In, at2, Some(screen), true, now), None, "still clamped: nothing");
        let c = z.step(Step::Out, at2, Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), c1.bounds.unwrap()), "{:?}", c.bounds);
    }

    /// X11 reports our resize at the old place first and the move after it: the first
    /// report is not the user's move (it shifted the base's origin on Xvfb, 2026-11-10,
    /// so Ctrl+0 came back 20 px high).
    #[test]
    fn our_resize_reported_before_our_move_is_still_ours() {
        let screen = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let start = Rect::new(240.0, 100.0, 800.0, 600.0);
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::In, start, Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(240.0, 80.0, 960.0, 720.0)), "{:?}", c.bounds);
        assert_eq!(z.observed(Rect::new(240.0, 100.0, 960.0, 720.0), true, now + Duration::from_millis(5)), Observed::Ours, "the resize, still at the old place");
        assert_eq!(z.observed(Rect::new(240.0, 80.0, 960.0, 720.0), true, now + Duration::from_millis(10)), Observed::Ours, "the move");
        let c = z.step(Step::Reset, Rect::new(240.0, 80.0, 960.0, 720.0), Some(screen), true, now).unwrap();
        assert_eq!(c.bounds, Some(start), "back where it started");
    }

    /// Where a window manager (or Wayland) never puts the window where we asked, the step
    /// still ends: after the window has passed, the place the platform reports is taken.
    #[test]
    fn a_window_left_elsewhere_still_settles() {
        let screen = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::In, Rect::new(240.0, 100.0, 800.0, 600.0), Some(screen), true, now).unwrap();
        let want = c.bounds.unwrap();
        let elsewhere = Rect::new(240.0, 100.0, want.w, want.h);
        assert_eq!(z.observed(elsewhere, true, now + Duration::from_millis(5)), Observed::Ours);
        assert_eq!(z.observed(elsewhere, true, now + Duration::from_secs(2)), Observed::Ours, "a late echo settles it");
        // Now the user drags it: that is theirs, and the base follows.
        assert_eq!(z.observed(Rect::new(400.0, 100.0, want.w, want.h), true, now + Duration::from_secs(3)), Observed::Theirs);
        let c = z.step(Step::Out, Rect::new(400.0, 100.0, want.w, want.h), Some(screen), true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(400.0, 100.0, 800.0, 600.0)), "{:?}", c.bounds);
    }

    #[test]
    fn a_user_move_re_anchors_the_base() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let c = z.step(Step::In, BASE, None, true, now).unwrap();
        assert_eq!(z.observed(c.bounds.unwrap(), true, now), Observed::Ours);
        let moved = Rect::new(400.0, 300.0, 960.0, 720.0);
        assert_eq!(z.observed(moved, true, now + Duration::from_secs(3)), Observed::Theirs);
        let c = z.step(Step::Out, moved, None, true, now).unwrap();
        assert!(rect_near(c.bounds.unwrap(), Rect::new(400.0, 300.0, 800.0, 600.0)), "{:?}", c.bounds);
    }

    #[test]
    fn no_screen_means_no_clamp_and_no_move() {
        let mut z = ZoomState::default();
        let now = Instant::now();
        let mut r = Rect::new(500.0, 500.0, 800.0, 600.0);
        for _ in 0..MAX_LEVEL {
            r = z.step(Step::In, r, None, true, now).unwrap().bounds.unwrap();
            assert!(near(r.x, 500.0) && near(r.y, 500.0));
        }
        assert!(near(r.w, 800.0 * scale_at(MAX_LEVEL)), "{}", r.w);
    }

    #[test]
    fn the_action_ids_name_the_steps() {
        assert_eq!(Step::from_action("zoom_in"), Some(Step::In));
        assert_eq!(Step::from_action("zoom_out"), Some(Step::Out));
        assert_eq!(Step::from_action("zoom_reset"), Some(Step::Reset));
        assert_eq!(Step::from_action("zoom"), None);
    }
}
