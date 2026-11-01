//! The native GUI's half of src/desktop.rs: what the taskbar or Dock shows for Clay.
//!
//! - Taskbar Alert (Settings, off by default): the MUD sent text and no Clay window has
//!   focus - flash the main window's taskbar button (X11's urgency hint, Windows'
//!   `FlashWindowEx`), bounce the Dock icon (macOS). Once per stretch without focus;
//!   GPUI does nothing on Wayland. The window with the connection decides (follow.rs
//!   `handle`), since every `/window` sees the same text.
//! - The activity count (the ACT badge's) on the taskbar button or Dock icon, kept by the
//!   window with the connection.

use gpui::{AnyWindowHandle, App, Context, Window};

/// The window that owns the connection asks for attention: `handle` is its own window.
pub(super) fn alert(handle: AnyWindowHandle, cx: &mut App) {
    if cx.active_window().is_some() || !crate::desktop::alert_once() {
        return;
    }
    // After this update: the window may be the one being updated now.
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, _| {
            window.request_attention();
            #[cfg(target_os = "linux")]
            x11::flagged(window);
        });
    });
}

/// A Clay window (the main one, a `/window`, Settings...) has focus: the next stretch
/// without it alerts again, and the flash stops.
pub(super) fn watch<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    cx.observe_window_activation(window, |_, window, _| {
        if window.is_window_active() {
            crate::desktop::alert_reset();
            #[cfg(target_os = "linux")]
            x11::clear();
        }
    })
    .detach();
}

/// The activity count on the taskbar or the Dock, as it was last set.
#[derive(Default)]
pub struct Badge {
    last: Option<usize>,
    hwnd: Option<isize>,
}

impl Badge {
    /// Each frame of the window with the connection: `count` worlds with activity.
    pub(super) fn apply(&mut self, window: &Window, count: usize) {
        if self.last == Some(count) {
            return;
        }
        self.last = Some(count);
        #[cfg(windows)]
        if self.hwnd.is_none() {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(RawWindowHandle::Win32(h)) = HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
                self.hwnd = Some(h.hwnd.get());
            }
        }
        let _ = window;
        crate::desktop::set_badge(count, self.hwnd);
    }
}

impl Drop for Badge {
    fn drop(&mut self) {
        if self.last.is_some_and(|n| n > 0) {
            crate::desktop::set_badge(0, self.hwnd);
        }
    }
}

/// GPUI clears its X11 urgency hint only when the window manager says the window has
/// `_NET_WM_STATE_FOCUSED`, which not every one does, so Clay clears it too, on the
/// window it flagged, when any Clay window gets the focus.
#[cfg(target_os = "linux")]
mod x11 {
    use std::sync::Mutex;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// ICCCM's `UrgencyHint`, in `WM_HINTS`' flags.
    const URGENCY: u32 = 1 << 8;

    /// The window flagged urgent, and a connection to clear it with.
    static FLAGGED: Mutex<(Option<u32>, Option<RustConnection>)> = Mutex::new((None, None));

    pub(super) fn flagged(window: &gpui::Window) {
        let xid = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xcb(h)) => h.window.get(),
            Ok(RawWindowHandle::Xlib(h)) => h.window as u32,
            _ => return,
        };
        FLAGGED.lock().unwrap_or_else(|e| e.into_inner()).0 = Some(xid);
    }

    pub(super) fn clear() {
        let mut guard = FLAGGED.lock().unwrap_or_else(|e| e.into_inner());
        let Some(xid) = guard.0.take() else { return };
        if guard.1.is_none() {
            guard.1 = RustConnection::connect(None).ok().map(|(c, _)| c);
        }
        let Some(conn) = guard.1.as_ref() else { return };
        let cleared = (|| {
            let hints = conn.get_property(false, xid, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, 0, 9).ok()?.reply().ok()?;
            let mut words: Vec<u32> = hints.value32()?.collect();
            let flags = words.first_mut()?;
            if *flags & URGENCY != 0 {
                *flags &= !URGENCY;
                conn.change_property32(PropMode::REPLACE, xid, AtomEnum::WM_HINTS, AtomEnum::WM_HINTS, &words).ok()?;
                conn.flush().ok()?;
            }
            Some(())
        })();
        if cleared.is_none() {
            // The display or the window went away: connect again next time.
            guard.1 = None;
        }
    }
}
