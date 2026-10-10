//! The settings' Transparency, as the WebView GUI applies it (webview_gui.rs `SetOpacity`:
//! GTK's `set_opacity` on every window): the whole window, text and all, at that
//! opacity. On X11 that is the `_NET_WM_WINDOW_OPACITY` property a compositing window
//! manager reads. Elsewhere (Wayland, Windows, macOS) it's kept but not applied, as in
//! the WebView GUI.

use gpui::Window;

/// A window's opacity, applied when the setting changes.
#[derive(Default)]
pub struct Opacity {
    /// What was last applied, as the property's value.
    last: Option<u32>,
}

/// The property's value for `alpha` (0 to 1): a fraction of `u32::MAX`.
fn value(alpha: f32) -> u32 {
    (f64::from(alpha.clamp(0.0, 1.0)) * f64::from(u32::MAX)).round() as u32
}

/// What to send for `v` after `last`: nothing, or the property's new value (`u32::MAX`
/// removes it). A window starts without the property, so fully opaque before anything
/// else needs nothing.
fn change(last: Option<u32>, v: u32) -> Option<u32> {
    if last.unwrap_or(u32::MAX) == v { None } else { Some(v) }
}

impl Opacity {
    pub fn apply(&mut self, window: &Window, alpha: f32) {
        let v = value(alpha);
        let send = change(self.last, v);
        self.last = Some(v);
        let Some(v) = send else { return };
        #[cfg(target_os = "linux")]
        x11::set(window, v);
        #[cfg(not(target_os = "linux"))]
        let _ = (window, v);
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::Mutex;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// One connection for every change, every window's: the X server keeps one client's
    /// requests in order but not two clients', so a connection per change let an earlier
    /// change (the removal while the settings weren't in yet) land after the later one and
    /// leave the window opaque.
    static CONN: Mutex<Option<(RustConnection, Atom)>> = Mutex::new(None);

    /// Set (or, fully opaque, remove) the window's `_NET_WM_WINDOW_OPACITY`.
    pub fn set(window: &gpui::Window, value: u32) {
        let id = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xcb(h)) => h.window.get(),
            Ok(RawWindowHandle::Xlib(h)) => h.window as u32,
            // Wayland has no such property.
            _ => return,
        };
        let mut guard = CONN.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = connect();
        }
        let Some((conn, atom)) = guard.as_ref() else { return };
        let sent = if value == u32::MAX {
            conn.delete_property(id, *atom).map(|_| ())
        } else {
            conn.change_property32(PropMode::REPLACE, id, *atom, AtomEnum::CARDINAL, &[value]).map(|_| ())
        };
        if sent.and_then(|_| conn.flush()).is_err() {
            // The display went away: start again next time.
            *guard = None;
        }
    }

    fn connect() -> Option<(RustConnection, Atom)> {
        let (conn, _) = RustConnection::connect(None).ok()?;
        let atom = conn.intern_atom(false, b"_NET_WM_WINDOW_OPACITY").ok()?.reply().ok()?.atom;
        Some((conn, atom))
    }
}

#[cfg(test)]
mod tests {
    use super::{change, value};

    #[test]
    fn opacity_is_a_fraction_of_the_whole_range() {
        assert_eq!(value(1.0), u32::MAX);
        assert_eq!(value(0.0), 0);
        assert_eq!(value(0.5), u32::MAX / 2 + 1);
        assert_eq!(value(1.7), u32::MAX, "clamped");
    }

    #[test]
    fn only_a_change_is_sent_and_a_new_window_starts_opaque() {
        let half = value(0.5);
        // Opaque on the first frame (the settings not in yet): nothing to remove, so no
        // removal to land after the real value.
        assert_eq!(change(None, u32::MAX), None);
        assert_eq!(change(None, half), Some(half));
        assert_eq!(change(Some(u32::MAX), half), Some(half));
        assert_eq!(change(Some(half), half), None);
        assert_eq!(change(Some(half), u32::MAX), Some(u32::MAX), "back to opaque removes it");
    }
}
