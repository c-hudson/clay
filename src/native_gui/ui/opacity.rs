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

impl Opacity {
    pub fn apply(&mut self, window: &Window, alpha: f32) {
        let v = value(alpha);
        if self.last == Some(v) {
            return;
        }
        self.last = Some(v);
        #[cfg(target_os = "linux")]
        x11::set(window, v);
        #[cfg(not(target_os = "linux"))]
        let _ = window;
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    /// Set (or, fully opaque, remove) the window's `_NET_WM_WINDOW_OPACITY`.
    pub fn set(window: &gpui::Window, value: u32) {
        let id = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xcb(h)) => h.window.get(),
            Ok(RawWindowHandle::Xlib(h)) => h.window as u32,
            // Wayland has no such property.
            _ => return,
        };
        let Ok((conn, _)) = x11rb::connect(None) else { return };
        let Some(atom) = conn.intern_atom(false, b"_NET_WM_WINDOW_OPACITY").ok().and_then(|c| c.reply().ok()).map(|r| r.atom) else { return };
        let _ = if value == u32::MAX {
            conn.delete_property(id, atom).map(|_| ())
        } else {
            conn.change_property32(PropMode::REPLACE, id, atom, AtomEnum::CARDINAL, &[value]).map(|_| ())
        };
        let _ = conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::value;

    #[test]
    fn opacity_is_a_fraction_of_the_whole_range() {
        assert_eq!(value(1.0), u32::MAX);
        assert_eq!(value(0.0), 0);
        assert_eq!(value(0.5), u32::MAX / 2 + 1);
        assert_eq!(value(1.7), u32::MAX, "clamped");
    }
}
