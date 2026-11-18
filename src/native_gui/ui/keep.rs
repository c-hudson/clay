//! The main window kept across a restart: a `/reload` (or `/server`, or the restart after
//! `/update`) opens the new window where the old one was, at its size, maximized or
//! fullscreen if it was, zoomed if it was (user request 2026-11-17; the hand-over is
//! `crate::keptwindow`). Only the main window: a `/window` or popup is not reopened by a
//! restart at all.
//!
//! What is kept is what GPUI opens a window at - `WindowBounds` - so each platform gets
//! back what it reported: the place is `Window::bounds()`'s origin (the content's on X11
//! and Windows, the frame's top-left on macOS, which is also what GPUI's open takes there)
//! and the size is the content's (`viewport_size`, what every platform's open takes). A
//! maximized or fullscreen window keeps the size it had before (`Keep::windowed`), which
//! is what it comes back to when the user restores it.
//!
//! Windows and macOS open the window there themselves. X11 window managers place a new
//! window by their own rules (GPUI sets no position hint and maps the window before Clay
//! sees it), so there the window opens as a plain window and is then moved, as the zoom
//! moves one (ui/zoom.rs, the frame taken off), once the window manager has framed it -
//! and maximized or made fullscreen only after that, so a restore brings it back to the
//! kept place. Wayland never lets a program place a window: the size and state are kept,
//! the place is the compositor's.

use std::time::{Duration, Instant};

use gpui::{point, px, size, Bounds, DisplayId, Window, WindowBounds};

use super::ClayView;
use crate::keptwindow::{KeptWindow, KeptZoom, WindowState, Windowed, NATIVE};
use crate::zoom::{Rect, SIZE_TOLERANCE};

/// The main window's record keeping (`ClayView::keep`; no other window has one).
#[derive(Default)]
pub(super) struct Keep {
    /// The window when last seen neither maximized nor fullscreen.
    windowed: Windowed,
    /// X11: the kept place and state, still to be taken.
    placing: Option<Placing>,
    /// The last record noted, so an unchanged window notes nothing.
    last: Option<KeptWindow>,
}

struct Placing {
    /// Where the window goes (`None`: kept where no place is known).
    to: Option<(f32, f32)>,
    /// Its content size, which it must have too before it counts as placed.
    size: (f32, f32),
    /// What it becomes once there.
    then: WindowState,
    since: Instant,
    moves: u8,
    /// The last move asked: the next waits for its result.
    moved_at: Option<Instant>,
    /// A maximize the window manager made by itself has been undone.
    unmaxed: bool,
}

/// How long to wait for the window manager's frame (none comes without one: Xvfb).
const FRAME_WAIT: Duration = Duration::from_secs(1);
/// After this the window stays wherever it is (a window manager that won't move it).
const PLACE_FOR: Duration = Duration::from_secs(3);
/// Moves asked before giving up (a window manager that put it back, the user's drag).
const MOVES: u8 = 2;
/// How long a move is given to show in a bounds report before it counts as not taken.
const MOVE_WAIT: Duration = Duration::from_millis(500);

/// The bounds and display the main window opens with: the kept window's. On Linux it
/// opens as a plain window, made maximized or fullscreen by `Keep::opened`.
pub(super) fn opening(kept: &KeptWindow, cx: &gpui::App) -> (WindowBounds, Option<DisplayId>) {
    let display = kept.display.map(DisplayId::from).filter(|id| cx.find_display(*id).is_some());
    let content = size(px(kept.w), px(kept.h));
    let bounds = if kept.placed { Bounds::new(point(px(kept.x), px(kept.y)), content) } else { Bounds::centered(display, content, cx) };
    let state = if cfg!(target_os = "linux") { WindowState::Windowed } else { kept.state };
    let bounds = match state {
        WindowState::Windowed => WindowBounds::Windowed(bounds),
        WindowState::Maximized => WindowBounds::Maximized(bounds),
        WindowState::Fullscreen => WindowBounds::Fullscreen(bounds),
    };
    (bounds, display)
}

impl Keep {
    /// The main window has just opened, from `kept` when a restart handed one over.
    pub(super) fn opened(kept: Option<&KeptWindow>, window: &mut Window) -> Keep {
        let Some(kept) = kept else { return Keep::default() };
        let mut keep = Keep { windowed: Windowed::from(Some((kept.x, kept.y, kept.w, kept.h))), ..Keep::default() };
        if cfg!(target_os = "linux") {
            if super::zoom::unplaced(window) {
                // Wayland: there is no place to take, only the state.
                apply_state(kept.state, window);
            } else if kept.placed || kept.state != WindowState::Windowed {
                keep.placing = Some(Placing { to: kept.placed.then_some((kept.x, kept.y)), size: (kept.w, kept.h), then: kept.state, since: Instant::now(), moves: 0, moved_at: None, unmaxed: false });
            }
        }
        crate::debug_log(true, &format!("KEPT WINDOW: opened from {kept:?}"));
        keep
    }
}

/// The kept zoom, as the zoom resumes it.
pub(super) fn resumed_zoom(kept: &KeptWindow) -> Option<crate::zoom::ZoomState> {
    let z = kept.zoom?;
    let rect = |r: [f32; 4]| Rect::new(r[0], r[1], r[2], r[3]);
    Some(crate::zoom::ZoomState::resumed(z.level, z.base.map(rect), rect(z.at), z.maxed))
}

fn apply_state(state: WindowState, window: &mut Window) {
    match state {
        WindowState::Maximized if !window.is_maximized() => window.zoom_window(),
        WindowState::Fullscreen if !window.is_fullscreen() => window.toggle_fullscreen(),
        _ => {}
    }
}

impl ClayView {
    /// A kept place is still being taken (X11): the window manager's reports meanwhile
    /// (its own placing, a maximize of its own and the restore) are not the user's, so
    /// the zoom doesn't re-base on them and no size popup shows.
    pub(super) fn keep_placing(&self) -> bool {
        self.keep.as_ref().is_some_and(|k| k.placing.is_some())
    }

    /// The window as it is now is what a restart keeps (src/keptwindow.rs): after every
    /// bounds report and zoom step, and at every tick, since a change of state alone
    /// (maximized, fullscreen) reports no bounds. Not while a kept place is still being
    /// taken - the record is the kept one until then.
    pub(super) fn keep_note(&mut self, window: &Window, cx: &gpui::App) {
        let Some(keep) = self.keep.as_mut() else { return };
        if keep.placing.is_some() {
            return;
        }
        let state = if window.is_fullscreen() {
            WindowState::Fullscreen
        } else if window.is_maximized() {
            WindowState::Maximized
        } else {
            WindowState::Windowed
        };
        let b = window.bounds();
        let s = window.viewport_size();
        let here = (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(s.width), f32::from(s.height));
        let (x, y, w, h) = keep.windowed.see(state, here, Instant::now());
        let at = super::zoom::window_rect(window);
        let zoom = self.zoom.kept().map(|(level, base, maxed)| KeptZoom {
            level,
            base: base.map(|r| [r.x, r.y, r.w, r.h]),
            at: [at.x, at.y, at.w, at.h],
            maxed,
        });
        let record = KeptWindow {
            gui: NATIVE.into(),
            x,
            y,
            w,
            h,
            placed: !super::zoom::unplaced(window),
            state,
            display: window.display(cx).map(|d| u64::from(d.id())),
            zoom,
        };
        if keep.last.as_ref() != Some(&record) {
            keep.last = Some(record.clone());
            crate::keptwindow::note(record);
        }
    }

    /// The tick's share: take the kept place (X11), and note the window.
    pub(super) fn keep_tick(&mut self, window: &mut Window, cx: &gpui::App) {
        if self.keep.is_none() {
            return;
        }
        self.keep_place(window);
        self.keep_note(window, cx);
    }

    /// X11: once the window manager has framed the window, move it to the kept place
    /// (the frame's top-left at the content's place less the frame), then maximize it or
    /// make it fullscreen if it was. From every bounds report and every tick until done.
    pub(super) fn keep_place(&mut self, window: &mut Window) {
        let Some(keep) = self.keep.as_mut() else { return };
        let Some(p) = keep.placing.as_mut() else { return };
        let waited = p.since.elapsed();
        // xfwm4 maximizes a new window that fills the screen (one zoomed to the screen's
        // size does): not the window that was kept, which comes back as it was.
        if p.then != WindowState::Maximized && window.is_maximized() && waited < PLACE_FOR {
            if !p.unmaxed {
                crate::debug_log(true, "KEPT WINDOW: maximized by the window manager, restored");
                window.zoom_window();
                p.unmaxed = true;
                p.moved_at = Some(Instant::now());
            }
            return;
        }
        let done = match p.to {
            None => true,
            // Maximized already (the window manager's own doing, for a window that fills
            // the screen): what it was to become.
            Some(_) if p.then == WindowState::Maximized && window.is_maximized() => true,
            Some((x, y)) => {
                let b = window.bounds();
                let s = window.viewport_size();
                let near = |a: f32, b: f32| (a - b).abs() <= SIZE_TOLERANCE;
                let there = near(f32::from(b.origin.x), x)
                    && near(f32::from(b.origin.y), y)
                    && near(f32::from(s.width), p.size.0)
                    && near(f32::from(s.height), p.size.1);
                if there || waited >= PLACE_FOR {
                    true
                } else if p.moved_at.is_some_and(|at| at.elapsed() < MOVE_WAIT) {
                    // The last move hasn't been reported yet.
                    return;
                } else if p.moves >= MOVES {
                    true
                } else {
                    let frame = super::zoom::frame_known(window);
                    if frame.is_none() && waited < FRAME_WAIT {
                        // Not framed yet: a move now would be the frame's place.
                        return;
                    }
                    let (fx, fy) = frame.unwrap_or_default().outer(x, y);
                    crate::debug_log(true, &format!("KEPT WINDOW: at {:?}, moved to {x},{y} (frame {frame:?})", b.origin));
                    super::zoom::move_window(window, fx, fy);
                    p.moves += 1;
                    p.moved_at = Some(Instant::now());
                    false
                }
            }
        };
        if done {
            let then = p.then;
            keep.placing = None;
            crate::debug_log(true, &format!("KEPT WINDOW: placed at {:?} after {waited:?}, now {then:?}", window.bounds()));
            apply_state(then, window);
        }
    }
}
