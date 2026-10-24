//! Native (non-WebView) GUI, behind the `native-gui` feature.
//!
//! Talks to Clay over its WebSocket protocol, as the web and Android clients do: to a
//! Clay elsewhere (`--gui=host`), or to the one it runs in this process (`--gui` alone;
//! `--web-gui` runs the WebView GUI instead). `core` holds everything that
//! doesn't touch GPUI (connection, text styling, wrapping) so it can be unit-tested;
//! `ui` is the GPUI side.

pub mod core;
pub mod quiet;
mod ui;
#[cfg(test)]
mod parity_tests;

static LOCAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this GUI runs Clay itself (`--gui` alone): its /reload is then the
/// server's hot reload, which keeps the MUD connections.
pub fn runs_clay() -> bool {
    LOCAL.load(std::sync::atomic::Ordering::Relaxed)
}

/// `clay --gui[=host[:port]]`: with a host, attach to that Clay; alone, run Clay
/// here. With `snapshot` set (`--gui-snapshot=<png>`) render one frame offscreen
/// to that file and exit; with `verify` set (`--gui-verify[=SECONDS]`) run the
/// client headless and check its buffer (both attach, to this machine when no host).
pub fn run(target: Option<String>, snapshot: Option<String>, verify: Option<u64>, ssh: bool) -> std::io::Result<()> {
    log_crashes();
    // The window's run: what the graphics drivers print goes to debug.log (quiet.rs). With
    // --ssh only once the tunnel is up, since it may ask for a passphrase on the terminal.
    let interactive = snapshot.is_none() && verify.is_none();
    if interactive && !ssh {
        quiet::capture();
    }
    // Looked at while Clay starts or the target is worked out: it loads the GPU's driver
    // (with --ssh, after the passphrase prompt, which its warnings could land in).
    #[cfg(target_os = "linux")]
    let vulkan = (!ssh).then(|| std::thread::spawn(vulkan_is_software_only));
    let runtime = tokio::runtime::Handle::current();
    let local = target.is_none() && !ssh && snapshot.is_none() && verify.is_none();
    // Optional: without it (and without a stored key for this server) the window asks.
    let mut password = std::env::var("CLAY_PASSWORD").ok().filter(|p| !p.is_empty());
    let target = if local {
        let (target, session_password) = start_clay(&runtime);
        password = Some(session_password);
        target
    } else if ssh {
        ssh_target(target.as_deref(), &runtime)
    } else {
        match core::net::Target::parse(target.as_deref()) {
            Ok(t) => t,
            Err(e) => {
                quiet::terminal(&format!("Error: {e}"));
                std::process::exit(1);
            }
        }
    };
    if interactive && ssh {
        quiet::capture();
    }
    #[cfg(target_os = "linux")]
    let vulkan = vulkan.unwrap_or_else(|| std::thread::spawn(vulkan_is_software_only));
    // An auth key to use instead of the one remembered for this server (scripts, tests).
    let auth_key = std::env::var("CLAY_AUTH_KEY").ok().filter(|k| !k.is_empty());
    // WebKitGTK blends text linearly; GPUI's Linux renderer defaults to gamma 1.8, which
    // measured 15% heavier than the WebView. 1.0 matches it to within 2%.
    #[cfg(target_os = "linux")]
    if std::env::var_os("ZED_FONTS_GAMMA").is_none() {
        std::env::set_var("ZED_FONTS_GAMMA", "1.0");
    }
    #[cfg(target_os = "linux")]
    if vulkan.join().unwrap_or(false) {
        crate::debug_log(true, "native-gui: Vulkan has only software devices here; drawing with OpenGL");
        std::env::set_var("VK_LOADER_DRIVERS_DISABLE", "*");
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
            eprintln!("Error: --gui-snapshot needs a build with the 'native-gui-snapshot' feature.");
            std::process::exit(1);
        }
    }
    // A cold start shows what was cached and comes back to the world last shown.
    let mut session = session;
    let key = session.target.key();
    session.core.set_cache(core::cache::load(&key));
    session.core.prefer_world(core::store::server(&key).last_world);
    // Started from a desktop: no console window behind it (as `--web-gui` does).
    #[cfg(windows)]
    {
        extern "system" {
            fn FreeConsole() -> i32;
        }
        unsafe { FreeConsole() };
    }
    ui::run(session, events);
    // The windows are closed and the quit's cleanup is done (ui::run's on_app_quit):
    // end here, as the WebView GUI's event loop does, rather than return through
    // main, whose runtime would first wait out any blocking call still running.
    std::process::exit(0);
}

/// `--gui` alone: Clay itself, in this process, as `--web-gui` runs it
/// (webview_gui::run_master_webgui): the app on the settings' port with a session
/// password of its own (the server listens for remote clients too, so it mustn't be
/// guessable), and this window logged in to it over the loopback.
fn start_clay(runtime: &tokio::runtime::Handle) -> (core::net::Target, String) {
    // The port, and the web password the user saved (Settings > Web): the server runs
    // with that one, so phones and browsers can log in with it; without one, a session
    // password of this run's own (crate::gui_session_password).
    let (port, saved_password) = {
        let mut app = crate::App::new();
        let _ = crate::persistence::load_settings(&mut app);
        (app.settings.http_port, app.settings.websocket_password.clone())
    };
    // Another Clay already there? (Not on a reload: the old process may hold the port
    // a moment longer, and the server's bind retries.)
    // Both the loopback this window connects to and every interface, where the server
    // listens: Windows lets a loopback bind succeed beside another program listening on
    // all interfaces, so checking the loopback alone let a Clay already running through,
    // to a server that couldn't bind and a ten-second wait for nothing.
    if !std::env::args().any(|a| a == "--reload") {
        for host in ["127.0.0.1", "0.0.0.0"] {
            if let Err(e) = std::net::TcpListener::bind((host, port)) {
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    fail(crate::platform::port_in_use_message(port, &format!("clay --gui=127.0.0.1:{port}")));
                }
                fail(format!("Clay failed to start: could not bind port {port}: {e}"));
            }
        }
    }
    let ws_override = match crate::gui_session_password(&saved_password) {
        Ok(o) => o,
        Err(e) => fail(format!("Clay failed to start: {e}")),
    };
    let password = ws_override.password().to_string();
    // The app's GUI channels (run_app_headless's API): kept open for the whole run.
    let (to_gui, from_app) = tokio::sync::mpsc::unbounded_channel::<crate::WsMessage>();
    let (to_app, from_gui) = tokio::sync::mpsc::unbounded_channel::<crate::WsMessage>();
    std::mem::forget((from_app, to_app));
    crate::GUI_HTTP_READY.store(false, std::sync::atomic::Ordering::SeqCst);
    *crate::GUI_HTTP_FAILED.lock().unwrap_or_else(|p| p.into_inner()) = None;
    runtime.spawn(async move {
        let _ = crate::run_app_headless(to_gui, from_gui, Some(ws_override), None, None).await;
    });
    // Wait for the server to have its port (it sets GUI_HTTP_READY once bound), or to
    // say why it can't (GUI_HTTP_FAILED: the port taken after all, no certificate).
    let ready = tokio::task::block_in_place(|| {
        for _ in 0..100 {
            if crate::GUI_HTTP_READY.load(std::sync::atomic::Ordering::SeqCst) {
                return true;
            }
            if let Some(why) = crate::GUI_HTTP_FAILED.lock().unwrap_or_else(|p| p.into_inner()).take() {
                fail(format!("Clay failed to start: {why}"));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    });
    if !ready {
        fail(format!("Clay failed to start: its server did not come up on port {port} within 10 seconds."));
    }
    crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: Clay's server is up on port {port}"));
    LOCAL.store(true, std::sync::atomic::Ordering::Relaxed);
    let target = core::net::Target::parse(Some(&format!("127.0.0.1:{port}"))).expect("a loopback address");
    (target, password)
}

/// Starting Clay failed: said on the terminal and in debug.log, since a GUI started from
/// the desktop (or by a /reload on Windows) has no terminal to say it on.
fn fail(message: String) -> ! {
    quiet::terminal(&message);
    crate::debug_log(true, &format!("native-gui: {message}"));
    std::process::exit(1);
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
        eprintln!("Error: --ssh requires --gui=<[user@]host[:clayport[:sshport]]>.");
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

/// A crash leaves what happened and where in debug.log (the console's crash handler is
/// set up after the native GUI has taken over, and a window started from a desktop
/// launcher has no terminal to show the panic on). Background tasks' panics too.
fn log_crashes() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::debug_log(true, &format!("native-gui CRASH: {info}"));
        crate::debug_log(true, &format!("BACKTRACE:\n{}", std::backtrace::Backtrace::force_capture()));
        default_hook(info);
    }));
}

/// Linux without a GPU: when Vulkan's only devices are software ones (Mesa's lavapipe,
/// SwiftShader), GPUI prefers them to software OpenGL, and the window stays black though
/// it renders fine offscreen. Then Vulkan is to be hidden from this process, so GPUI draws
/// with OpenGL (Mesa's llvmpipe), which shows. With a real GPU, or no Vulkan at all, it
/// isn't; nor when the user has already told the Vulkan loader or GPUI which device to use.
#[cfg(target_os = "linux")]
fn vulkan_is_software_only() -> bool {
    if std::env::var_os("VK_LOADER_DRIVERS_DISABLE").is_some() || std::env::var_os("ZED_DEVICE_ID").is_some() {
        return false;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let kinds: Vec<wgpu::DeviceType> = futures::executor::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN)).iter().map(|a| a.get_info().device_type).collect();
    only_software(&kinds)
}

/// Every Vulkan device is a CPU one (and there is at least one).
#[cfg(target_os = "linux")]
fn only_software(kinds: &[wgpu::DeviceType]) -> bool {
    !kinds.is_empty() && kinds.iter().all(|k| *k == wgpu::DeviceType::Cpu)
}

#[cfg(all(test, target_os = "linux"))]
mod vulkan_tests {
    use super::only_software;
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu};

    #[test]
    fn vulkan_is_hidden_only_when_every_device_is_software() {
        assert!(only_software(&[Cpu]), "lavapipe alone");
        assert!(!only_software(&[DiscreteGpu, Cpu]), "a GPU and lavapipe: the GPU wins anyway");
        assert!(!only_software(&[IntegratedGpu]));
        assert!(!only_software(&[]), "no Vulkan at all: GPUI goes to OpenGL by itself");
    }
}
