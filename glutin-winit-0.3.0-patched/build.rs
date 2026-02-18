// XXX keep in sync with glutin's build.rs.

use cfg_aliases::cfg_aliases;

fn main() {
    // Setup alias to reduce `cfg` boilerplate.
    cfg_aliases! {
        // Systems.
        // Patched for Termux/Android X11: when "x11" feature is enabled on Android,
        // treat it as a free_unix platform so X11/GLX backends are available.
        android: { all(target_os = "android", not(feature = "x11")) },
        wasm: { target_arch = "wasm32" },
        macos: { target_os = "macos" },
        ios: { target_os = "ios" },
        apple: { any(target_os = "ios", target_os = "macos") },
        free_unix: { all(unix, not(apple), not(android)) },

        // Native displays.
        x11_platform: { all(feature = "x11", unix, not(apple), not(wasm)) },
        wayland_platform: { all(feature = "wayland", free_unix, not(wasm)) },

        // Backends.
        egl_backend: { all(feature = "egl", any(windows, unix), not(apple), not(wasm)) },
        glx_backend: { all(feature = "glx", feature = "x11", unix, not(apple), not(wasm)) },
        wgl_backend: { all(feature = "wgl", windows, not(wasm)) },
        cgl_backend: { all(macos, not(wasm)) },
    }
}
