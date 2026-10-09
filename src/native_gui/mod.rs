//! Native (non-WebView) GUI, behind the `native-gui` feature.
//!
//! Attaches to a running Clay over its WebSocket protocol, as the web and Android
//! clients do. `core` holds everything that doesn't touch GPUI (connection, text
//! styling, wrapping) so it can be unit-tested; `ui` is the GPUI side.

pub mod core;
mod ui;
#[cfg(test)]
mod parity_tests;

/// `clay --native-gui[=host[:port]]`; with `snapshot` set (`--native-gui-snapshot=<png>`)
/// render one frame offscreen to that file and exit; with `verify` set
/// (`--native-gui-verify[=SECONDS]`) run the client headless and check its buffer.
pub fn run(target: Option<String>, snapshot: Option<String>, verify: Option<u64>, ssh: bool) -> std::io::Result<()> {
    let runtime = tokio::runtime::Handle::current();
    let target = if ssh {
        ssh_target(target.as_deref(), &runtime)
    } else {
        match core::net::Target::parse(target.as_deref()) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    };
    // Optional: without it (and without a stored key for this server) the window asks.
    let password = std::env::var("CLAY_PASSWORD").ok().filter(|p| !p.is_empty());
    // An auth key to use instead of the one remembered for this server (scripts, tests).
    let auth_key = std::env::var("CLAY_AUTH_KEY").ok().filter(|k| !k.is_empty());
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
    ui::emoji::init(runtime.clone());
    let (session, events) = core::session::Session::new(&runtime, target, password, auth_key);
    if let Some(secs) = verify {
        std::process::exit(core::verify::run(session, events, secs));
    }
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
    // A cold start shows what was cached and comes back to the world last shown.
    let mut session = session;
    let key = session.target.key();
    session.core.set_cache(core::cache::load(&key));
    session.core.prefer_world(core::store::server(&key).last_world);
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

/// `--ssh`: the server's loopback through an SSH tunnel (core/tunnel.rs), opened before
/// any window (no TUI here, so errors and a passphrase prompt use the terminal).
fn ssh_target(spec: Option<&str>, runtime: &tokio::runtime::Handle) -> core::net::Target {
    let Some(spec) = spec else {
        eprintln!("Error: --ssh requires --native-gui=<[user@]host[:clayport[:sshport]]>.");
        std::process::exit(1);
    };
    #[cfg(feature = "ssh-transport")]
    {
        let t = match crate::ssh::parse_ssh_target(spec) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error: invalid --ssh target: {e}");
                std::process::exit(1);
            }
        };
        let name = format!("ssh:{}@{}:{}", t.user, t.host, t.clay_port);
        match tokio::task::block_in_place(|| runtime.block_on(core::tunnel::start(t))) {
            Ok(port) => core::net::Target::tunnel(port, name),
            Err(e) => {
                eprintln!("clay: {e}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(not(feature = "ssh-transport"))]
    {
        let _ = (spec, runtime);
        eprintln!("clay: --ssh requires the 'ssh-transport' feature.");
        std::process::exit(1);
    }
}
