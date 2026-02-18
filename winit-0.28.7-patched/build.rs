use cfg_aliases::cfg_aliases;

#[cfg(all(
    any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
    ),
    feature = "wayland",
))]
mod wayland {
    use std::env;
    use std::path::PathBuf;
    use wayland_scanner::Side;

    pub fn main() {
        let mut path = PathBuf::from(env::var("OUT_DIR").unwrap());
        path.push("fractional_scale_v1.rs");
        wayland_scanner::generate_code(
            "wayland_protocols/fractional-scale-v1.xml",
            &path,
            Side::Client,
        );
    }
}

fn main() {
    // The script doesn't depend on our code
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wayland_protocols");

    // Debug: print cfg state during build
    println!("cargo:warning=winit build.rs: target_os=android? {}", cfg!(target_os = "android"));
    println!("cargo:warning=winit build.rs: unix? {}", cfg!(unix));
    println!("cargo:warning=winit build.rs: feature x11? {}", cfg!(feature = "x11"));

    // Setup cfg aliases
    // Patched for Termux/Android X11 support:
    // When "x11" feature is enabled on Android, use the Linux/X11 backend instead of android-activity.
    // This allows building a GUI on Termux with Termux:X11 installed.
    cfg_aliases! {
        // Systems.
        android_platform: { all(target_os = "android", not(feature = "x11")) },
        wasm_platform: { target_arch = "wasm32" },
        macos_platform: { target_os = "macos" },
        ios_platform: { target_os = "ios" },
        windows_platform: { target_os = "windows" },
        apple: { any(target_os = "ios", target_os = "macos") },
        free_unix: { all(unix, not(apple), not(target_os = "android")) },
        redox: { target_os = "redox" },

        // Native displays.
        // x11_platform includes Android when x11 feature is set (for Termux)
        x11_platform: { all(feature = "x11", unix, not(apple), not(wasm_platform), not(redox)) },
        wayland_platform: { all(feature = "wayland", free_unix, not(wasm_platform), not(redox)) },
        orbital_platform: { redox },
    }

    // XXX aliases are not available for the build script itself.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
        ),
        feature = "wayland",
    ))]
    wayland::main();
}
