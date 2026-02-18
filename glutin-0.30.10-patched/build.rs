use cfg_aliases::cfg_aliases;

fn main() {
    // Setup alias to reduce `cfg` boilerplate.
    cfg_aliases! {
        // Systems.
        // Patched for Termux/Android X11: when "x11" feature is enabled on Android,
        // treat it as a free_unix platform so X11/GLX backends are available.
        android_platform: { all(target_os = "android", not(feature = "x11")) },
        wasm_platform: { target_family = "wasm" },
        macos_platform: { target_os = "macos" },
        ios_platform: { target_os = "ios" },
        apple: { any(ios_platform, macos_platform) },
        free_unix: { all(unix, not(apple), not(android_platform)) },

        // Native displays.
        x11_platform: { all(feature = "x11", unix, not(apple), not(wasm_platform)) },
        // Wayland not available on Android (no wayland-sys)
        wayland_platform: { all(feature = "wayland", free_unix, not(target_os = "android"), not(wasm_platform)) },

        // Backends.
        egl_backend: { all(feature = "egl", any(windows, unix), not(apple), not(wasm_platform)) },
        // GLX not available on Android (glutin_glx_sys doesn't generate bindings); use EGL+X11 instead
        glx_backend: { all(feature = "glx", feature = "x11", unix, not(apple), not(target_os = "android"), not(wasm_platform)) },
        wgl_backend: { all(feature = "wgl", windows, not(wasm_platform)) },
        cgl_backend: { all(macos_platform, not(wasm_platform)) },
    }
}
