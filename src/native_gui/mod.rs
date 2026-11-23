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
    // The window's run: what the graphics drivers print goes to debug.log (quiet.rs). With
    // --ssh only once the tunnel is up, since it may ask for a passphrase on the terminal.
    let interactive = snapshot.is_none() && verify.is_none();
    // Its session in debug.log, and an exit line for however it ends (src/crashlog.rs;
    // the panic hook is crashlog's too, installed at the top of main). The snapshot and
    // verify runs are tools, not sessions.
    if interactive {
        let mode = match (&target, ssh) {
            (_, true) => format!("gui over ssh to {}", target.as_deref().unwrap_or("?")),
            (Some(host), false) => format!("gui attached to {host}"),
            (None, false) => "gui".to_string(),
        };
        crate::crashlog::session(&mode);
        // Attached, nothing here handles SIGTERM/SIGHUP (a Clay of its own would).
        if target.is_some() || ssh {
            crate::crashlog::record_ending_signals();
        }
    }
    if interactive && !ssh {
        quiet::capture();
    }
    // Looked at while Clay starts or the target is worked out: it loads the GPU's driver
    // (with --ssh, after the passphrase prompt, which its warnings could land in).
    #[cfg(target_os = "linux")]
    let gpu = (!ssh).then(|| std::thread::spawn(gpu_plan));
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
                crate::crashlog::exit_process(1, &format!("bad address: {e}"));
            }
        }
    };
    if interactive && ssh {
        quiet::capture();
    }
    #[cfg(target_os = "linux")]
    let gpu = gpu.unwrap_or_else(|| std::thread::spawn(gpu_plan));
    // An auth key to use instead of the one remembered for this server (scripts, tests).
    let mut auth_key = std::env::var("CLAY_AUTH_KEY").ok().filter(|k| !k.is_empty());
    // Neither variable given: the login this machine saved for the server, if any
    // (src/clientconf.rs, Settings > Server) - the default server's, or any other's.
    let mut username = None;
    if !local && !ssh && password.is_none() && auth_key.is_none() {
        if let Some(login) = crate::clientconf::ClientConf::load().login_for(&target.key()) {
            username = login.username().map(str::to_string);
            password = login.password().map(str::to_string);
            auth_key = login.auth_key().map(str::to_string);
        }
    }
    // WebKitGTK blends text linearly; GPUI's Linux renderer defaults to gamma 1.8, which
    // measured 15% heavier than the WebView. 1.0 matches it to within 2%.
    #[cfg(target_os = "linux")]
    if std::env::var_os("ZED_FONTS_GAMMA").is_none() {
        std::env::set_var("ZED_FONTS_GAMMA", "1.0");
    }
    #[cfg(target_os = "linux")]
    {
        let plan = gpu.join().unwrap_or_default();
        if plan.hide_vulkan {
            crate::debug_log(true, "native-gui: Vulkan has only software devices here; drawing with OpenGL");
            std::env::set_var("VK_LOADER_DRIVERS_DISABLE", "*");
        }
        if let Some(why) = plan.software_gl {
            crate::debug_log(true, &format!("native-gui: this machine's OpenGL can't draw the window ({why}); drawing with software OpenGL"));
            std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        }
    }
    // WKWebView draws the web client's output with `-webkit-font-smoothing: antialiased`,
    // i.e. without CoreGraphics' stroke thickening; GPUI on macOS thickens unless the
    // app's AppleFontSmoothing preference is 0. Measured 15-25% heavier before. Set for
    // this process only (never synchronized to disk).
    #[cfg(target_os = "macos")]
    disable_font_smoothing();
    ui::emoji::init(runtime.clone());
    let (session, events) = core::session::Session::with_credentials(&runtime, target, core::client::Credentials { username, password, auth_key });
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
    crate::crashlog::exit_process(0, "quit");
}

/// `--gui` alone: Clay itself, in this process, as `--web-gui` runs it
/// (webview_gui::run_master_webgui): the app on the settings' port (or the next free one,
/// when another program has it) with a session
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
    // A port already taken (another Clay, as a rule) is the server's to handle: it takes
    // the next free one and says so (start_web_server_at_startup), and GUI_HTTP_PORT is
    // where this window then connects.
    let ws_override = match crate::gui_session_password(&saved_password) {
        Ok(o) => o,
        Err(e) => fail(format!("Clay failed to start: {e}")),
    };
    let password = ws_override.password().to_string();
    // The app's GUI channels (run_app_headless's API): kept open for the whole run.
    let (to_gui, from_app) = tokio::sync::mpsc::unbounded_channel::<crate::WsMessage>();
    let (to_app, from_gui) = tokio::sync::mpsc::unbounded_channel::<crate::WsMessage>();
    // This window attaches over the WebSocket, so nothing reads `from_app`: dropped, the
    // App skips the clone of every broadcast it used to queue there for good
    // (`ws_broadcast`). `to_app` stays open so the App's own receiver never ends.
    drop(from_app);
    std::mem::forget(to_app);
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
    let port = crate::GUI_HTTP_PORT.load(std::sync::atomic::Ordering::SeqCst);
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
    crate::crashlog::exit_process(1, "failed to start");
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
        crate::crashlog::exit_process(1, "ssh tunnel failed");
    };
    #[cfg(feature = "ssh-transport")]
    {
        let t = match crate::ssh::parse_ssh_target(spec) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error: invalid --ssh target: {e}");
                crate::crashlog::exit_process(1, "ssh tunnel failed");
            }
        };
        let name = format!("ssh:{}@{}:{}", t.user, t.host, t.clay_port);
        match tokio::task::block_in_place(|| runtime.block_on(core::tunnel::start(t))) {
            Ok(port) => core::net::Target::tunnel(port, name),
            Err(e) => {
                eprintln!("clay: {e}");
                crate::crashlog::exit_process(1, "ssh tunnel failed");
            }
        }
    }
    #[cfg(not(feature = "ssh-transport"))]
    {
        let _ = (spec, runtime);
        eprintln!("clay: --ssh requires the 'ssh-transport' feature.");
        crate::crashlog::exit_process(1, "ssh tunnel failed");
    }
}

/// What has to change for GPUI to draw on this Linux machine, worked out before it starts
/// (both are environment variables, read by the Vulkan loader and by Mesa at their first use).
#[cfg(target_os = "linux")]
#[derive(Debug, Default, PartialEq)]
struct GpuPlan {
    /// Hide Vulkan from this process.
    hide_vulkan: bool,
    /// Draw with Mesa's software OpenGL: why the machine's own can't be used.
    software_gl: Option<String>,
}

/// Linux without a Vulkan GPU, the two cases GPUI gets wrong by itself:
/// - Vulkan's only devices are software ones (Mesa's lavapipe, SwiftShader): GPUI prefers
///   them to software OpenGL, and the window stays black though it renders fine offscreen.
///   Then Vulkan is to be hidden from this process, so GPUI draws with OpenGL, which shows.
/// - The OpenGL it is then left with can't give GPUI its device (a VirtualBox guest,
///   2026-11-16: the virtual card's driver has OpenGL ES 3.0, GPUI asks for more). GPUI has
///   no other device to try, so no window opened at all. Then Mesa is to draw in software
///   (llvmpipe), which can.
///
/// Nothing changes with a real Vulkan GPU, nor where the user has already told the Vulkan
/// loader, Mesa or GPUI what to use.
#[cfg(target_os = "linux")]
fn gpu_plan() -> GpuPlan {
    let set = |name: &str| std::env::var_os(name).is_some();
    if set("ZED_DEVICE_ID") {
        return GpuPlan::default();
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let vulkan: Vec<wgpu::DeviceType> = futures::executor::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN)).iter().map(|a| a.get_info().device_type).collect();
    let hide_vulkan = !set("VK_LOADER_DRIVERS_DISABLE") && only_software(&vulkan);
    let asked = set("LIBGL_ALWAYS_SOFTWARE") || set("MESA_LOADER_DRIVER_OVERRIDE");
    let software_gl = if left_with_opengl(&vulkan) && !asked { opengl_problem() } else { None };
    GpuPlan { hide_vulkan, software_gl }
}

/// Every Vulkan device is a CPU one (and there is at least one).
#[cfg(target_os = "linux")]
fn only_software(kinds: &[wgpu::DeviceType]) -> bool {
    !kinds.is_empty() && kinds.iter().all(|k| *k == wgpu::DeviceType::Cpu)
}

/// No Vulkan device is a GPU's (or there is no Vulkan): OpenGL is what GPUI will draw with.
#[cfg(target_os = "linux")]
fn left_with_opengl(vulkan: &[wgpu::DeviceType]) -> bool {
    vulkan.iter().all(|k| *k == wgpu::DeviceType::Cpu)
}

/// Why this machine's own OpenGL can't draw Clay's window, when it can't: there is no
/// device, or it refuses the one request GPUI makes of it (gpui_wgpu's `create_device`:
/// wgpu's `downlevel_defaults`, more than OpenGL ES 3.0 has). `None` when it can - and when
/// it is software already, since there is nothing left to fall back to.
#[cfg(target_os = "linux")]
fn opengl_problem() -> Option<String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::GL, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapters = futures::executor::block_on(instance.enumerate_adapters(wgpu::Backends::GL));
    let mut problem = "it has no usable device".to_string();
    for adapter in &adapters {
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return None;
        }
        let wanted = wgpu::DeviceDescriptor {
            required_features: adapter.features() & wgpu::Features::DUAL_SOURCE_BLENDING,
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()).using_alignment(adapter.limits()),
            ..Default::default()
        };
        match futures::executor::block_on(adapter.request_device(&wanted)) {
            Ok(_) => return None,
            Err(e) => {
                // One line: a driver's name ("SVGA3D; build: RELEASE;  LLVM;") and wgpu's
                // error both come with spacing of their own.
                let tidy = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
                problem = format!("{}: {}", tidy(&info.name).trim_end_matches(';'), tidy(&e.to_string()));
            }
        }
    }
    Some(problem)
}

#[cfg(all(test, target_os = "linux"))]
mod vulkan_tests {
    use super::{left_with_opengl, only_software};
    use wgpu::DeviceType::{Cpu, DiscreteGpu, IntegratedGpu};

    #[test]
    fn opengl_is_looked_at_only_when_no_vulkan_device_is_a_gpu() {
        assert!(left_with_opengl(&[]), "no Vulkan at all");
        assert!(left_with_opengl(&[Cpu]), "lavapipe alone, which is about to be hidden");
        assert!(!left_with_opengl(&[DiscreteGpu, Cpu]));
        assert!(!left_with_opengl(&[IntegratedGpu]));
    }

    #[test]
    fn vulkan_is_hidden_only_when_every_device_is_software() {
        assert!(only_software(&[Cpu]), "lavapipe alone");
        assert!(!only_software(&[DiscreteGpu, Cpu]), "a GPU and lavapipe: the GPU wins anyway");
        assert!(!only_software(&[IntegratedGpu]));
        assert!(!only_software(&[]), "no Vulkan at all: GPUI goes to OpenGL by itself");
    }
}
