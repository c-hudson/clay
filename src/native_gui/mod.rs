//! Native (non-WebView) GUI, behind the `native-gui` feature.
//!
//! Attaches to a running Clay over its WebSocket protocol, as the web and Android
//! clients do. `core` holds everything that doesn't touch GPUI (connection, text
//! styling, wrapping) so it can be unit-tested; `ui` is the GPUI side.

pub mod core;
mod ui;

/// `clay --native-gui[=host[:port]]`, or with `snapshot` set (`--native-gui-snapshot=<png>`)
/// render one frame offscreen to that file and exit.
pub fn run(target: Option<String>, snapshot: Option<String>) -> std::io::Result<()> {
    let target = match core::net::Target::parse(target.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };
    // Optional: without it (and without a stored key for this server) the window asks.
    let password = std::env::var("CLAY_PASSWORD").ok().filter(|p| !p.is_empty());
    // WebKitGTK blends text linearly; GPUI's Linux renderer defaults to gamma 1.8, which
    // measured 15% heavier than the WebView. 1.0 matches it to within 2%.
    #[cfg(target_os = "linux")]
    if std::env::var_os("ZED_FONTS_GAMMA").is_none() {
        std::env::set_var("ZED_FONTS_GAMMA", "1.0");
    }
    // WKWebView draws the web client's output with `-webkit-font-smoothing: antialiased`,
    // i.e. without CoreGraphics' stroke thickening; GPUI on macOS thickens unless the
    // app's AppleFontSmoothing preference is 0. Measured 15-25% heavier before. Set for
    // this process only (never synchronized to disk).
    #[cfg(target_os = "macos")]
    disable_font_smoothing();
    let runtime = tokio::runtime::Handle::current();
    ui::emoji::init(runtime.clone());
    let (session, events) = core::session::Session::new(&runtime, target, password);
    if let Some(out) = snapshot {
        #[cfg(feature = "native-gui-snapshot")]
        {
            ui::snapshot::run(session, events, &out, 800.0, 600.0);
            return Ok(());
        }
        #[cfg(not(feature = "native-gui-snapshot"))]
        {
            let _ = (out, session, events);
            eprintln!("Error: --native-gui-snapshot needs a build with the 'native-gui-snapshot' feature.");
            std::process::exit(1);
        }
    }
    ui::run(session, events);
    Ok(())
}

#[cfg(target_os = "macos")]
fn disable_font_smoothing() {
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_foundation_sys::preferences::{CFPreferencesSetAppValue, kCFPreferencesCurrentApplication};
    let key = CFString::new("AppleFontSmoothing");
    let value = CFNumber::from(0i32);
    unsafe {
        CFPreferencesSetAppValue(key.as_concrete_TypeRef(), value.as_CFTypeRef(), kCFPreferencesCurrentApplication);
    }
}
