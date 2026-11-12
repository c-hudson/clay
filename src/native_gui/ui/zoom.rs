//! Ctrl+plus / Ctrl+minus / Ctrl+0: zoom this window - its text and its size together,
//! as xfce4-terminal does (user request 2026-11-10; the rules and their tests are
//! `crate::zoom`). Per window and in memory only: `ClayView::font_size` stays the saved
//! size the slider and `apply_settings` own, the text is drawn at `text_size()` =
//! `font_size × zoom.scale()`, and nothing here reaches settings, the server or another
//! window. The native GUI only, by ruling: the WebView GUI has no zoom and a browser tab
//! keeps the browser's own.
//!
//! GPUI resizes a window (`Window::resize`, the content size) but has no call to move one,
//! so a window grown to its screen's size is put at the screen's origin by asking the
//! platform directly, as `opacity.rs` and `appmenu.rs` do: X11 `ConfigureWindow`, Win32
//! `SetWindowPos`, AppKit `setFrameTopLeftPoint:`; Wayland has no client-side move, so a
//! clamped window may overhang there until the user moves it.
//!
//! Everything is in **content** coordinates: a window is where GPUI reports its content
//! (X11 and Windows report the content's place; AppKit the frame's, so `window_rect`
//! moves down by the title bar there), and the window manager's frame around it
//! (`zoom::Frame`: title bar and borders) is taken off at the two edges - the screen a
//! window may fill is the work area less the frame, and the platform is asked to put the
//! frame at the content's place less the frame's top-left. Without that every step walked
//! the window down and right by the frame, and a window grown to the screen hung its
//! title bar's height off the bottom (user report 2026-11-10, xfwm4). X11 reads
//! `_NET_FRAME_EXTENTS` (nothing without a window manager), Windows DWM's visible frame
//! against GPUI's client origin, macOS the frame's height over the content's. On X11
//! GPUI's "display" is the whole X screen (every monitor at once), so the monitor under
//! the window's centre is read from RandR, cut to `_NET_WORKAREA`; elsewhere
//! `display.visible_bounds()` is the work area (Windows, macOS) or the output (Wayland).

use std::time::Instant;

use gpui::{px, size, Context, Window};

use super::ClayView;
use crate::zoom::{Frame, Rect, Step};

impl ClayView {
    /// The size this window's text is drawn at: the saved font size times the zoom.
    pub(super) fn text_size(&self) -> f32 {
        self.font_size * self.zoom.scale()
    }

    /// A zoom key: step, then resize (and move) the window as the state says. Nothing
    /// happens at all when the state says so (past the last level, or already as large
    /// as the screen).
    pub(super) fn zoom_step(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(step) = Step::from_action(action) else { return };
        let current = window_rect(window);
        let resizable = resizable(window);
        let frame = frame_of(window);
        let screen = work_area(window, cx).map(|area| frame.inset(area));
        let change = self.zoom.step(step, current, screen, resizable, Instant::now());
        crate::debug_log(true, &format!("ZOOM {action}: window {current:?} frame {frame:?} screen {screen:?} resizable={resizable} -> level {} {change:?}", self.zoom.level()));
        let Some(change) = change else { return };
        if let Some(b) = change.bounds {
            window.resize(size(px(b.w), px(b.h)));
            let (x, y) = frame.outer(b.x, b.y);
            move_window(window, x, y);
        }
        // The text changes size under the view: stay at the bottom if that is where it
        // was, as the font slider does (chrome.rs `set_font_size`).
        let follow = self.scroll.borrow().follow;
        if follow {
            self.scroll.borrow_mut().follow_bottom();
        }
        cx.notify();
    }

    /// The platform reported this window's bounds (a resize or a move, ours or the
    /// user's): the zoom learns which, so a dragged size is what later steps scale.
    pub(super) fn zoom_observe_bounds(&mut self, window: &Window) {
        let rect = window_rect(window);
        let whose = self.zoom.observed(rect, resizable(window), Instant::now());
        crate::debug_log(crate::is_debug_enabled(), &format!("ZOOM-BOUNDS {rect:?} -> {whose:?} at level {}", self.zoom.level()));
    }
}

/// A maximized or fullscreen window keeps the size the window manager gave it.
fn resizable(window: &Window) -> bool {
    !(window.is_maximized() || window.is_fullscreen())
}

/// The window now: where its content is, in global logical pixels, and its content size.
/// GPUI's bounds are the content's place on X11 and Windows; AppKit's are the frame's, so
/// the content sits a title bar lower there.
fn window_rect(window: &Window) -> Rect {
    let b = window.bounds();
    let s = window.viewport_size();
    let x = f32::from(b.origin.x);
    #[cfg(target_os = "macos")]
    let y = f32::from(b.origin.y) + title_bar(window);
    #[cfg(not(target_os = "macos"))]
    let y = f32::from(b.origin.y);
    Rect::new(x, y, f32::from(s.width), f32::from(s.height))
}

/// AppKit: the frame's height over the content's.
#[cfg(target_os = "macos")]
fn title_bar(window: &Window) -> f32 {
    (f32::from(window.bounds().size.height) - f32::from(window.viewport_size().height)).max(0.0)
}

/// The window manager's frame around the content, where the platform tells; none where
/// it doesn't (no window manager, Wayland).
fn frame_of(window: &Window) -> Frame {
    #[cfg(target_os = "linux")]
    {
        x11::frame_extents(window).unwrap_or_default()
    }
    #[cfg(target_os = "windows")]
    {
        win32::frame_extents(window).unwrap_or_default()
    }
    #[cfg(target_os = "macos")]
    {
        Frame { top: title_bar(window), ..Frame::default() }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = window;
        Frame::default()
    }
}

/// The usable area of the monitor the window is on (panels and docks excluded), in
/// global logical pixels, when known.
fn work_area(window: &Window, cx: &gpui::App) -> Option<Rect> {
    #[cfg(target_os = "linux")]
    if let Some(r) = x11::monitor_of(window) {
        return Some(r);
    }
    let d = window.display(cx)?;
    let b = d.visible_bounds();
    Some(Rect::new(f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)))
}

/// Put the window's frame's top-left at (`x`, `y`) in global logical pixels, where the
/// platform lets a program do that (see the module doc).
fn move_window(window: &Window, x: f32, y: f32) {
    #[cfg(target_os = "linux")]
    x11::move_to(window, x, y);
    #[cfg(target_os = "windows")]
    win32::move_to(window, x, y);
    #[cfg(target_os = "macos")]
    macos::move_to(window, x, y);
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    let _ = (window, x, y);
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::sync::Mutex;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConfigureWindowAux, ConnectionExt as _, Window as XWindow};
    use x11rb::rust_connection::RustConnection;

    use crate::zoom::{Frame, Rect};

    struct Atoms {
        workarea: Atom,
        frame_extents: Atom,
    }

    /// This module's own connection (opacity.rs keeps its own: its concern is the order
    /// of its property changes, which these requests needn't join), with the root window
    /// and the atoms it asks about.
    static CONN: Mutex<Option<(RustConnection, XWindow, Atoms)>> = Mutex::new(None);

    fn window_id(window: &gpui::Window) -> Option<XWindow> {
        match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
            Ok(RawWindowHandle::Xlib(h)) => Some(h.window as XWindow),
            // Wayland: nothing here applies.
            _ => None,
        }
    }

    fn connect() -> Option<(RustConnection, XWindow, Atoms)> {
        let (conn, screen) = RustConnection::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atom = |name: &[u8]| -> Option<Atom> { Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom) };
        let atoms = Atoms { workarea: atom(b"_NET_WORKAREA")?, frame_extents: atom(b"_NET_FRAME_EXTENTS")? };
        Some((conn, root, atoms))
    }

    /// Run `f` on the connection, made on first use and dropped when a request fails
    /// (the display went away: start again next time). What `f` finds, or doesn't, is
    /// its `Ok`.
    fn with_conn<R>(f: impl FnOnce(&RustConnection, XWindow, &Atoms) -> Result<R, ()>) -> Option<R> {
        let mut guard = CONN.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = connect();
        }
        let (conn, root, atoms) = guard.as_ref()?;
        match f(conn, *root, atoms) {
            Ok(r) => Some(r),
            Err(()) => {
                *guard = None;
                None
            }
        }
    }

    /// `n` CARDINALs of `prop` on `win`: `Ok(None)` when the property isn't there (or is
    /// shorter), `Err` when the request failed.
    fn cardinals(conn: &RustConnection, win: XWindow, prop: Atom, n: usize) -> Result<Option<Vec<u32>>, ()> {
        let reply = conn
            .get_property(false, win, prop, AtomEnum::CARDINAL, 0, n as u32)
            .map_err(|_| ())?
            .reply()
            .map_err(|_| ())?;
        Ok(reply.value32().map(|it| it.collect::<Vec<u32>>()).filter(|v| v.len() >= n))
    }

    /// The frame the window manager put around the window (`_NET_FRAME_EXTENTS`: left,
    /// right, top, bottom), in logical pixels. None without a window manager (Xvfb), off
    /// X11, or before the window is managed.
    pub fn frame_extents(window: &gpui::Window) -> Option<Frame> {
        let id = window_id(window)?;
        let sf = window.scale_factor();
        with_conn(|conn, _, atoms| {
            Ok(cardinals(conn, id, atoms.frame_extents, 4)?.map(|v| Frame {
                left: v[0] as f32 / sf,
                right: v[1] as f32 / sf,
                top: v[2] as f32 / sf,
                bottom: v[3] as f32 / sf,
            }))
        })
        .flatten()
    }

    /// The monitor under the window's centre (else the primary, else the first), cut to
    /// the work area, in global logical pixels. None off X11, or without RandR.
    pub fn monitor_of(window: &gpui::Window) -> Option<Rect> {
        window_id(window)?;
        let sf = window.scale_factor();
        let b = window.bounds();
        // The window's centre in device pixels, which is what RandR speaks.
        let cx = ((f32::from(b.origin.x) + f32::from(b.size.width) / 2.0) * sf) as i32;
        let cy = ((f32::from(b.origin.y) + f32::from(b.size.height) / 2.0) * sf) as i32;
        with_conn(|conn, root, atoms| {
            let Some(reply) = conn.randr_get_monitors(root, true).ok().and_then(|c| c.reply().ok()) else { return Ok(None) };
            let under = |m: &&x11rb::protocol::randr::MonitorInfo| {
                let (x, y, w, h) = (i32::from(m.x), i32::from(m.y), i32::from(m.width), i32::from(m.height));
                (x..x + w).contains(&cx) && (y..y + h).contains(&cy)
            };
            let Some(m) = reply.monitors.iter().find(under).or_else(|| reply.monitors.iter().find(|m| m.primary)).or_else(|| reply.monitors.first()) else {
                return Ok(None);
            };
            let (mut x, mut y, mut w, mut h) = (f32::from(m.x), f32::from(m.y), f32::from(m.width), f32::from(m.height));
            // `_NET_WORKAREA` (x, y, w, h per desktop, the whole X screen's): the first
            // desktop's, cut to this monitor; an empty cut (a work area that doesn't reach
            // this monitor) keeps the monitor whole.
            if let Some(v) = cardinals(conn, root, atoms.workarea, 4)? {
                let (wx, wy, ww, wh) = (v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32);
                let (x1, y1) = (x.max(wx), y.max(wy));
                let (x2, y2) = ((x + w).min(wx + ww), (y + h).min(wy + wh));
                if x2 > x1 && y2 > y1 {
                    (x, y, w, h) = (x1, y1, x2 - x1, y2 - y1);
                }
            }
            Ok(Some(Rect::new(x / sf, y / sf, w / sf, h / sf)))
        })
        .flatten()
    }

    /// Ask the window manager to put the window's frame at (`x`, `y`), global logical
    /// pixels (the default NorthWest gravity: a ConfigureWindow's place is the frame's).
    pub fn move_to(window: &gpui::Window, x: f32, y: f32) {
        let Some(id) = window_id(window) else { return };
        let sf = window.scale_factor();
        let (dx, dy) = ((x * sf).round() as i32, (y * sf).round() as i32);
        with_conn(|conn, _, _| {
            conn.configure_window(id, &ConfigureWindowAux::new().x(dx).y(dy)).map_err(|_| ())?;
            conn.flush().map_err(|_| ())
        });
    }
}

#[cfg(target_os = "windows")]
mod win32 {
    use ::windows::Win32::Foundation::{HWND, RECT};
    use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use ::windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use crate::zoom::Frame;

    fn hwnd_of(window: &gpui::Window) -> Option<HWND> {
        match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => Some(HWND(h.hwnd.get() as *mut core::ffi::c_void)),
            _ => None,
        }
    }

    /// The frame as the eye sees it (DWM's extended frame bounds, without the invisible
    /// resize borders) and the window rectangle `SetWindowPos` places (with them), in
    /// device pixels.
    fn rects(hwnd: HWND) -> Option<(RECT, RECT)> {
        let mut seen = RECT::default();
        let mut placed = RECT::default();
        // SAFETY: a live window of this process; the attribute is read into a RECT of
        // the size given.
        unsafe {
            DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut seen as *mut RECT as *mut core::ffi::c_void, std::mem::size_of::<RECT>() as u32).ok()?;
            GetWindowRect(hwnd, &mut placed).ok()?;
        }
        Some((seen, placed))
    }

    /// The visible frame around GPUI's client area, in logical pixels.
    pub fn frame_extents(window: &gpui::Window) -> Option<Frame> {
        let (seen, _) = rects(hwnd_of(window)?)?;
        let sf = window.scale_factor();
        let b = window.bounds();
        let s = window.viewport_size();
        let (cx, cy) = (f32::from(b.origin.x) * sf, f32::from(b.origin.y) * sf);
        let (cw, ch) = (f32::from(s.width) * sf, f32::from(s.height) * sf);
        let logical = |v: f32| (v / sf).max(0.0);
        Some(Frame {
            left: logical(cx - seen.left as f32),
            top: logical(cy - seen.top as f32),
            right: logical(seen.right as f32 - (cx + cw)),
            bottom: logical(seen.bottom as f32 - (cy + ch)),
        })
    }

    /// Put the visible frame's top-left at (`x`, `y`), global logical pixels, keeping the
    /// size and the place in the Z order (the invisible borders go off the edge, as
    /// Windows itself snaps a window).
    pub fn move_to(window: &gpui::Window, x: f32, y: f32) {
        let Some(hwnd) = hwnd_of(window) else { return };
        let sf = window.scale_factor();
        let (mut dx, mut dy) = ((x * sf).round() as i32, (y * sf).round() as i32);
        if let Some((seen, placed)) = rects(hwnd) {
            dx -= seen.left - placed.left;
            dy -= seen.top - placed.top;
        }
        // SAFETY: a live window of this process; SetWindowPos only moves it.
        unsafe {
            let _ = SetWindowPos(hwnd, None, dx, dy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2_app_kit::{NSScreen, NSView};
    use objc2_foundation::{MainThreadMarker, NSPoint};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// Put the window's frame's top-left at (`x`, `y`), global logical points. AppKit's
    /// global coordinates start at the primary screen's bottom-left; GPUI's at its
    /// top-left.
    pub fn move_to(window: &gpui::Window, x: f32, y: f32) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let view = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::AppKit(h)) => h.ns_view,
            _ => return,
        };
        // SAFETY: GPUI's handle is its live NSView, and this runs on the main thread.
        let view: &NSView = unsafe { view.cast::<NSView>().as_ref() };
        let Some(ns_window) = view.window() else { return };
        let Some(primary) = NSScreen::screens(mtm).firstObject() else { return };
        let top = primary.frame().size.height;
        ns_window.setFrameTopLeftPoint(NSPoint::new(f64::from(x), top - f64::from(y)));
    }
}
