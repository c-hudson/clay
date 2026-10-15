//! The title bar follows the theme on Windows: dark for a dark theme, light for a light
//! one, as the WebView GUI's did (webview_gui.rs `with_theme`, which has tao set
//! `DWMWA_USE_IMMERSIVE_DARK_MODE`). GPUI sets that attribute from the *system's*
//! light/dark setting instead, when the window is made and again whenever the system's
//! setting changes, so a dark Clay theme on a light Windows got white title bars. Every
//! window applies it as it draws, like the Transparency (opacity.rs). macOS and Linux
//! draw their title bars from the system's theme, as the WebView GUI's did there too.

use gpui::{Window, WindowAppearance};

/// What a window's title bar was last set to.
#[derive(Default)]
pub struct TitleBar {
    /// Dark or not, and the system's appearance when it was set: GPUI sets the system's
    /// choice again after a change there, so that change is a reason to set ours again.
    last: Option<(bool, WindowAppearance)>,
}

impl TitleBar {
    pub fn apply(&mut self, window: &Window, dark: bool) {
        let key = (dark, window.appearance());
        if self.last == Some(key) {
            return;
        }
        self.last = Some(key);
        #[cfg(target_os = "windows")]
        win::set_dark(window, dark);
        #[cfg(not(target_os = "windows"))]
        let _ = (window, dark);
    }
}

#[cfg(target_os = "windows")]
mod win {
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
    use ::windows::Win32::UI::WindowsAndMessaging::{SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos};
    use ::windows::core::BOOL;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub fn set_dark(window: &gpui::Window, dark: bool) {
        let Ok(RawWindowHandle::Win32(h)) = HasWindowHandle::window_handle(window).map(|h| h.as_raw()) else { return };
        let hwnd = HWND(h.hwnd.get() as *mut core::ffi::c_void);
        let value = BOOL::from(dark);
        unsafe {
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const BOOL as *const _, std::mem::size_of::<BOOL>() as u32);
            // A title bar already on screen keeps its old colours until something repaints
            // the frame (a theme chosen in Settings would show only once the window lost
            // and regained the focus): ask for that now.
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}
