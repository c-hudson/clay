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
    let password = match std::env::var("CLAY_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("Error: set CLAY_PASSWORD to the Clay server's password.");
            std::process::exit(1);
        }
    };
    let mut uid = [0u8; 16];
    let _ = getrandom::getrandom(&mut uid);
    // WebKitGTK blends text linearly; GPUI's Linux renderer defaults to gamma 1.8, which
    // measured 15% heavier than the WebView. 1.0 matches it to within 2%.
    #[cfg(target_os = "linux")]
    if std::env::var_os("ZED_FONTS_GAMMA").is_none() {
        std::env::set_var("ZED_FONTS_GAMMA", "1.0");
    }
    let runtime = tokio::runtime::Handle::current();
    let (sender, events) = core::net::connect(&runtime, target, password, hex::encode(uid));
    if let Some(out) = snapshot {
        #[cfg(feature = "native-gui-snapshot")]
        {
            ui::snapshot::run(sender, events, &out, 800.0, 600.0);
            return Ok(());
        }
        #[cfg(not(feature = "native-gui-snapshot"))]
        {
            let _ = out;
            eprintln!("Error: --native-gui-snapshot needs a build with the 'native-gui-snapshot' feature.");
            std::process::exit(1);
        }
    }
    ui::run(sender, events);
    Ok(())
}
