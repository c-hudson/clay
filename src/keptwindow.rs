//! A desktop GUI's main window kept across a restart: its place, size, maximized or
//! fullscreen state and zoom (user request 2026-11-17: "when doing a reload in the gui,
//! the window size is not retained. Please make it so the window size and
//! geometry/location is.").
//!
//! The main window of either GUI records itself (`note`) whenever the platform reports a
//! move or a resize (the native GUI also after a zoom step). Every path that starts Clay
//! again in this process's place - `platform::exec_reload` (the GUI running Clay itself)
//! and `platform::exec_relaunch_with_args` (an attached GUI's `/reload`, `/server`, the
//! restart after `/update`) - hands the last record to its successor in `KEPT_WINDOW_ENV`
//! (`pass_to_successor`). The successor takes the variable out of its environment first
//! thing (`take_from_env`, from `main`, in every mode), so nothing it starts later - a
//! shell world, a `clay` run from one - inherits it, and its GUI opens its main window
//! from it (`take`), only a GUI of the kind that kept it (`gui`). Nothing is saved: an
//! ordinary start opens the window as before.
//!
//! The record is each GUI's own coordinates (logical pixels): the native GUI keeps what
//! GPUI's `WindowBounds` opens a window at (ui/keep.rs), the WebView GUI tao's outer
//! position and inner size. No GPUI or tao types here, so it is tested in every build.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The environment variable a successor finds the record in (JSON).
pub const KEPT_WINDOW_ENV: &str = "CLAY_KEPT_WINDOW";

/// The window's state beyond its place and size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowState {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

/// The native GUI's zoom (`crate::zoom::ZoomState`), kept with the window it sized: a
/// window kept at its zoomed size and opened at 1:1 text would no longer be the window
/// that was there. Rectangles are `[x, y, w, h]` in the zoom's content coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KeptZoom {
    pub level: i8,
    /// The window at level 0 (`None`: zoomed while maximized - only the text grew).
    pub base: Option<[f32; 4]>,
    /// The window as the zoom last saw it.
    pub at: [f32; 4],
    /// The window is maximized because a step would have outgrown the screen (Wayland).
    #[serde(default)]
    pub maxed: bool,
}

/// One GUI's main window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeptWindow {
    /// Which GUI kept it: `NATIVE` or `WEBVIEW`.
    pub gui: String,
    /// Where the window goes, in the coordinates that GUI opens a window at.
    pub x: f32,
    pub y: f32,
    /// Its content size: when maximized or fullscreen, the size it comes back to.
    pub w: f32,
    pub h: f32,
    /// Whether `x`/`y` mean anything (not on Wayland, where a program is never told).
    #[serde(default)]
    pub placed: bool,
    #[serde(default)]
    pub state: WindowState,
    /// The display it was on, by the platform's id (GPUI's `DisplayId`).
    #[serde(default)]
    pub display: Option<u64>,
    #[serde(default)]
    pub zoom: Option<KeptZoom>,
}

pub const NATIVE: &str = "native";
pub const WEBVIEW: &str = "webview";

impl KeptWindow {
    /// A size no window could have is no record (a minimised window can report 0x0).
    fn usable(&self) -> bool {
        [self.x, self.y, self.w, self.h].iter().all(|v| v.is_finite()) && self.w >= 50.0 && self.h >= 50.0
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(json: &str) -> Option<KeptWindow> {
        serde_json::from_str::<KeptWindow>(json).ok().filter(KeptWindow::usable)
    }
}

/// A window's place and content size: x, y, w, h.
pub type Place = (f32, f32, f32, f32);

/// How far behind its new size a maximize's state may be reported (`Windowed`).
pub const STATE_LAG: Duration = Duration::from_secs(1);

/// The window as it was when last neither maximized nor fullscreen: what a maximized one
/// is kept as, the size it comes back to when restored. X11 reports a maximize's new size
/// before the window says it is maximized (the configure comes before `_NET_WM_STATE`, in
/// GPUI and GTK alike), so a maximize seen within `STATE_LAG` of a change takes that
/// change back - to what was there before the burst, since a move and a resize can come
/// apart.
#[derive(Clone, Copy, Debug, Default)]
pub struct Windowed {
    now: Option<Place>,
    /// What `now` was before the latest burst of changes, and when that burst began.
    before: Option<(Place, Instant)>,
}

impl Windowed {
    /// Starting from a kept window's place.
    pub fn from(place: Option<Place>) -> Windowed {
        Windowed { now: place, before: None }
    }

    /// The window was seen in `state` at `here` (its place and content size now): the
    /// place a restart keeps for it.
    pub fn see(&mut self, state: WindowState, here: Place, now: Instant) -> Place {
        if state == WindowState::Windowed {
            if self.now != Some(here) {
                if self.before.is_none_or(|(_, at)| now.duration_since(at) >= STATE_LAG) {
                    self.before = self.now.map(|was| (was, now));
                }
                self.now = Some(here);
            }
        } else if let Some((was, at)) = self.before.take() {
            if now.duration_since(at) < STATE_LAG {
                self.now = Some(was);
            }
        }
        self.now.unwrap_or(here)
    }
}

/// The main window as it is now (`note`), for `pass_to_successor`.
static CURRENT: Mutex<Option<KeptWindow>> = Mutex::new(None);
/// What the predecessor handed over (`take_from_env`), until the GUI opens with it.
static HANDED: Mutex<Option<KeptWindow>> = Mutex::new(None);

fn lock(m: &Mutex<Option<KeptWindow>>) -> std::sync::MutexGuard<'_, Option<KeptWindow>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The main window moved, was resized, maximized or zoomed: this is it now. A record no
/// window could have (`usable`) leaves the last one standing.
pub fn note(window: KeptWindow) {
    if window.usable() {
        *lock(&CURRENT) = Some(window);
    }
}

/// The main window as last noted.
pub fn current() -> Option<KeptWindow> {
    lock(&CURRENT).clone()
}

/// Just before this process is replaced: the successor gets the main window as last
/// noted, and nothing when there is none (a console, `-D`).
pub fn pass_to_successor() {
    match current() {
        Some(window) => std::env::set_var(KEPT_WINDOW_ENV, window.to_json()),
        None => std::env::remove_var(KEPT_WINDOW_ENV),
    }
}

/// The exec failed and this process carries on: what it starts later mustn't inherit it.
pub fn forget_passed() {
    std::env::remove_var(KEPT_WINDOW_ENV);
}

/// At start-up, every mode: the window a predecessor handed over is kept for the GUI to
/// open with, and the variable leaves the environment either way.
pub fn take_from_env() {
    if let Some(json) = std::env::var_os(KEPT_WINDOW_ENV) {
        std::env::remove_var(KEPT_WINDOW_ENV);
        *lock(&HANDED) = json.to_str().and_then(KeptWindow::from_json);
    }
}

/// The handed-over window, once, if `gui` kept it. It becomes the current record too, so
/// a window that is never moved before the next restart is still kept.
pub fn take(gui: &str) -> Option<KeptWindow> {
    let window = lock(&HANDED).take().filter(|w| w.gui == gui)?;
    note(window.clone());
    Some(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> KeptWindow {
        KeptWindow {
            gui: NATIVE.into(),
            x: 120.0,
            y: 64.5,
            w: 1003.0,
            h: 711.0,
            placed: true,
            state: WindowState::Maximized,
            display: Some(7),
            zoom: Some(KeptZoom { level: 2, base: Some([120.0, 64.5, 696.5, 493.75]), at: [120.0, 64.5, 1003.0, 711.0], maxed: false }),
        }
    }

    /// The record survives its trip through the environment exactly, and a record from a
    /// build that wrote fewer fields still reads.
    #[test]
    fn a_kept_window_round_trips() {
        let w = sample();
        assert_eq!(KeptWindow::from_json(&w.to_json()), Some(w));
        let older = r#"{"gui":"webview","x":1,"y":2,"w":640,"h":480}"#;
        let back = KeptWindow::from_json(older).expect("parses");
        assert_eq!((back.state, back.placed, back.display, back.zoom), (WindowState::Windowed, false, None, None));
        assert_eq!(KeptWindow::from_json("not json"), None);
    }

    /// A size no window has is never kept, and never replaces a good record.
    #[test]
    fn an_unusable_size_is_not_a_window() {
        let mut tiny = sample();
        tiny.w = 0.0;
        assert_eq!(KeptWindow::from_json(&tiny.to_json()), None);
        let mut nan = sample();
        nan.x = f32::NAN;
        assert!(!nan.usable());
    }

    /// X11's order: a maximize's size first, its state after. The size seen in between
    /// is taken back, to what was there before the move and the resize came apart; a
    /// change long before the maximize stands, and a restore is the window again.
    #[test]
    fn a_maximize_reported_late_keeps_the_size_before_it() {
        let t = Instant::now();
        let ms = |n: u64| t + Duration::from_millis(n);
        let plain = (300.0, 170.0, 900.0, 560.0);
        let mut w = Windowed::from(Some(plain));
        assert_eq!(w.see(WindowState::Windowed, plain, ms(0)), plain);
        // The maximize: the move, then the size, then the state.
        w.see(WindowState::Windowed, (0.0, 23.0, 900.0, 560.0), ms(10));
        w.see(WindowState::Windowed, (0.0, 23.0, 1280.0, 777.0), ms(20));
        assert_eq!(w.see(WindowState::Maximized, (0.0, 23.0, 1280.0, 777.0), ms(30)), plain);
        // Restored, then moved by the user much later, then maximized in order.
        assert_eq!(w.see(WindowState::Windowed, plain, ms(5000)), plain);
        let moved = (40.0, 50.0, 900.0, 560.0);
        assert_eq!(w.see(WindowState::Windowed, moved, ms(9000)), moved);
        assert_eq!(w.see(WindowState::Maximized, (0.0, 23.0, 1280.0, 777.0), ms(12000)), moved, "a move long before stands");
        // Nothing seen windowed yet: the window as it is.
        let mut fresh = Windowed::default();
        assert_eq!(fresh.see(WindowState::Fullscreen, (0.0, 0.0, 1280.0, 800.0), ms(0)), (0.0, 0.0, 1280.0, 800.0));
    }

    /// The whole hand-over in one process: noted, passed, taken back out of the
    /// environment (which it leaves), opened by the GUI that kept it only, and current
    /// again for the next restart.
    #[test]
    fn the_window_goes_to_the_successor_once() {
        // One test owns the process-wide state and the variable.
        *lock(&CURRENT) = None;
        *lock(&HANDED) = None;
        pass_to_successor();
        assert!(std::env::var_os(KEPT_WINDOW_ENV).is_none(), "nothing noted, nothing passed");

        let w = sample();
        note(w.clone());
        let mut zero = sample();
        zero.h = 0.0;
        note(zero);
        assert_eq!(current(), Some(w.clone()), "a minimised report leaves the last good one");
        pass_to_successor();
        assert!(std::env::var_os(KEPT_WINDOW_ENV).is_some());

        *lock(&CURRENT) = None;
        take_from_env();
        assert!(std::env::var_os(KEPT_WINDOW_ENV).is_none(), "taken out of the environment");
        assert_eq!(take(WEBVIEW), None, "another GUI's window is not this one's");
        // ... and is gone: a mismatched GUI drops it.
        *lock(&HANDED) = Some(w.clone());
        assert_eq!(take(NATIVE), Some(w.clone()));
        assert_eq!(take(NATIVE), None, "once");
        assert_eq!(current(), Some(w), "kept for the next restart without a move");

        forget_passed();
        assert!(std::env::var_os(KEPT_WINDOW_ENV).is_none());
        *lock(&CURRENT) = None;
    }
}
