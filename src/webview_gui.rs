// WebView GUI client using wry (native WebView window)
//
// Provides two modes:
// --gui            Master mode: runs App headlessly + opens WebView window (local WS connection)
// --gui=host:port  Remote mode: opens WebView window connected to remote Clay instance

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{WindowBuilder, WindowId};
use wry::WebViewBuilder;

/// Temporarily suppress stderr (e.g. WebKit/JSC prints harmless warnings on init).
/// Returns a guard that restores stderr when dropped.
#[cfg(unix)]
struct StderrSuppress {
    saved_fd: i32,
}

#[cfg(unix)]
impl StderrSuppress {
    fn new() -> Self {
        unsafe {
            let saved = libc::dup(2);
            let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY);
            if devnull >= 0 {
                libc::dup2(devnull, 2);
                libc::close(devnull);
            }
            Self { saved_fd: saved }
        }
    }
}

#[cfg(unix)]
impl Drop for StderrSuppress {
    fn drop(&mut self) {
        if self.saved_fd >= 0 {
            unsafe {
                libc::dup2(self.saved_fd, 2);
                libc::close(self.saved_fd);
            }
        }
    }
}

/// Custom events sent from IPC handler to event loop
#[derive(Debug)]
enum WvEvent {
    /// Set window opacity (0.0 = fully transparent, 1.0 = fully opaque)
    SetOpacity(f64),
    /// Close the window and exit
    Quit,
    /// Show an update status message in the WebView
    UpdateStatus(String),
    /// Hot reload: exec a new binary (remote GUI only)
    Reload,
    /// Open a new window (optionally locked to a world)
    NewWindow(Option<String>),
    /// Open a grep results window (half height, no status/input, filtered output)
    GrepWindow { pattern: String, world: Option<String>, use_regex: bool },
}

use crate::theme::{ThemeColors, ThemeFile};
use crate::websocket::hash_password;

/// Open a URL in the system's default browser (platform-specific).
fn open_url_in_browser(url: &str) {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    { let _ = std::process::Command::new("xdg-open").arg(url).spawn(); }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(url).spawn(); }
    #[cfg(windows)]
    { let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn(); }
}

const WEB_INDEX_HTML: &str = include_str!("web/index.html");
const WEB_STYLE_CSS: &str = include_str!("web/style.css");
const WEB_APP_JS: &str = include_str!("web/app.js");
const CLAY_LOGO_PNG: &[u8] = include_bytes!("../clay2.png");

/// Parameters for building the WebView HTML content
#[derive(Clone)]
struct WebViewParams {
    ws_host: String,
    ws_port: u16,
    ws_protocol: String,
    auto_password: Option<String>,
    theme_css: String,
    /// Real server address (for /window command — may differ from ws_host when using proxy)
    server_host: Option<String>,
    server_port: Option<u16>,
    server_secure: bool,
}

/// Read the gui_theme name from ~/.clay.dat (defaults to "dark").
fn load_gui_theme_name() -> String {
    let home = crate::get_home_dir();
    if home == "." {
        return "dark".to_string();
    }
    let settings_path = format!("{}/{}", home, crate::clay_filename("clay.dat"));
    std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|content| {
            content.lines()
                .find(|l| l.starts_with("gui_theme="))
                .map(|l| l.trim_start_matches("gui_theme=").to_string())
        })
        .unwrap_or_else(|| "dark".to_string())
}

/// Load the user's GUI theme CSS vars for initial HTML rendering.
/// Reads gui_theme name from ~/.clay.dat and theme colors from ~/.clay.theme.dat.
fn load_user_theme_css() -> String {
    let home = crate::get_home_dir();
    let gui_theme_name = load_gui_theme_name();

    if home == "." {
        return ThemeColors::dark_default().to_css_vars();
    }

    // Load theme colors from ~/.clay.theme.dat
    let theme_path = format!("{}/{}", home, crate::clay_filename("clay.theme.dat"));
    let theme_file = ThemeFile::load(std::path::Path::new(&theme_path));
    theme_file.get(&gui_theme_name).to_css_vars()
}

/// Show a modal error dialog. On Windows this uses MessageBoxW; elsewhere prints to stderr.
fn show_error_dialog(title: &str, message: &str) {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        extern "system" {
            fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, utype: u32) -> i32;
        }
        let title_wide: Vec<u16> = OsStr::new(title).encode_wide().chain(std::iter::once(0)).collect();
        let msg_wide: Vec<u16> = OsStr::new(message).encode_wide().chain(std::iter::once(0)).collect();
        unsafe { MessageBoxW(std::ptr::null_mut(), msg_wide.as_ptr(), title_wide.as_ptr(), 0x10); }
    }
    #[cfg(not(windows))]
    {
        eprintln!("{}: {}", title, message);
    }
}

/// Master mode: run App headlessly with a local WebSocket, open WebView window
pub fn run_master_webgui() -> io::Result<()> {

    // Check for display server availability (Linux only)
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let has_display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
        if !has_display {
            eprintln!("clay: no display server found.");
            eprintln!("clay: start Termux:X11 and run:  DISPLAY=:0 clay --gui");
            std::process::exit(1);
        }
    }

    // Termux:X11 has no DRI3/EGL hardware acceleration; force software rendering
    // so WebKit2GTK doesn't show a blank window. Disable the WebKit sandbox so
    // the web process can reach ws://127.0.0.1 from the clay:// custom scheme.
    #[cfg(target_os = "android")]
    {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        std::env::set_var("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", "1");
    }

    // Read the configured HTTP port from settings (default 9000)
    // The GUI uses the same port as the web interface
    let port = {
        let mut tmp_app = crate::App::new();
        let _ = crate::persistence::load_settings(&mut tmp_app);
        tmp_app.settings.http_port
    };

    // Detect duplicate instances: probe the port before spawning anything.
    // Skip the probe during hot-reload (CLAY_HTTP_LISTENER means we inherited the socket).
    let is_reload = std::env::var("CLAY_HTTP_LISTENER").is_ok();
    if !is_reload {
        if let Err(e) = std::net::TcpListener::bind(format!("127.0.0.1:{}", port)) {
            let msg = if e.kind() == io::ErrorKind::AddrInUse {
                format!(
                    "Clay is already running (port {} is in use).\n\nOnly one instance of Clay can run at a time.",
                    port
                )
            } else {
                format!("Clay failed to start: could not bind port {}.\n\n{}", port, e)
            };
            show_error_dialog("Clay", &msg);
            return Err(e);
        }
        // Drop the test listener immediately — the real bind happens inside run_app_headless.
    }

    // Generate a random password using time + pid as entropy
    let random_bytes: [u8; 32] = {
        let mut buf = [0u8; 32];
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let pid = std::process::id() as u64;
        let mut state = seed ^ (pid << 32) ^ 0x517cc1b727220a95;
        for byte in buf.iter_mut() {
            // xorshift64
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
        buf
    };
    let password = hex::encode(random_bytes);
    let password_hash = hash_password(&password);

    // Build tokio runtime for the App
    let runtime = tokio::runtime::Runtime::new()?;
    let handle = runtime.handle().clone();

    // Create bidirectional channels (required by run_app_headless API)
    let (app_to_gui_tx, _app_to_gui_rx) = tokio::sync::mpsc::unbounded_channel::<crate::WsMessage>();
    let (gui_to_app_tx, gui_to_app_rx) = tokio::sync::mpsc::unbounded_channel::<crate::WsMessage>();

    // Spawn the headless App with WS override
    let ws_password = password.clone();
    handle.spawn(async move {
        if let Err(_e) = crate::run_app_headless(
            app_to_gui_tx,
            gui_to_app_rx,
            Some(ws_password),
            None, // No GUI repaint callback (webview is event-driven)
        ).await {
        }
    });

    // Wait for the HTTP server to signal it has bound the port.
    // Uses an atomic flag set by start_http_server after successful bind,
    // avoiding the race where TCP connect succeeds against a dying old socket.
    crate::GUI_HTTP_READY.store(false, std::sync::atomic::Ordering::SeqCst);
    let ws_ready = {
        let mut ready = false;
        for _ in 0..100 {
            if crate::GUI_HTTP_READY.load(std::sync::atomic::Ordering::SeqCst) {
                // Server has bound — give the accept loop a moment to start
                std::thread::sleep(std::time::Duration::from_millis(200));
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        ready
    };

    if !ws_ready {
        runtime.shutdown_background();
        return Err(io::Error::other("WebSocket server did not start within 3 seconds"));
    }

    let params = WebViewParams {
        ws_host: "127.0.0.1".to_string(),
        ws_port: port,
        ws_protocol: "ws".to_string(),
        auto_password: Some(password_hash),
        theme_css: load_user_theme_css(),
        server_host: None, // Master mode — no separate server URL needed
        server_port: None,
        server_secure: false,
    };

    let result = create_webview_window("Clay", &params, Some(gui_to_app_tx));

    // Shut down the tokio runtime when the window closes
    runtime.shutdown_background();

    result
}

/// Remote mode: open WebView window connected to a remote Clay instance.
///
/// First tries direct ws:// connection (WebKit handles plain WebSocket fine).
/// If the remote server only accepts wss://, falls back to a local WS proxy
/// that handles TLS with self-signed cert support (WebKit rejects self-signed certs).
pub fn run_remote_webgui(addr: &str) -> io::Result<()> {
    // Check for display server availability (Linux only)
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let has_display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
        if !has_display {
            eprintln!("clay: no display server found.");
            eprintln!("clay: start Termux:X11 and run:  DISPLAY=:0 clay --gui");
            std::process::exit(1);
        }
    }

    // Termux:X11 has no DRI3/EGL hardware acceleration; force software rendering
    // so WebKit2GTK doesn't show a blank window. Disable the WebKit sandbox so
    // the web process can reach the remote WebSocket from the clay:// custom scheme.
    #[cfg(target_os = "android")]
    {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        std::env::set_var("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", "1");
    }

    // Strip protocol prefix if provided
    let (addr_stripped, explicit_protocol) = if let Some(rest) = addr.strip_prefix("wss://") {
        (rest, Some("wss"))
    } else if let Some(rest) = addr.strip_prefix("ws://") {
        (rest, Some("ws"))
    } else {
        (addr, None)
    };

    // Parse host:port (default port 9000 if not specified)
    let (host, port) = if let Some(colon_pos) = addr_stripped.rfind(':') {
        let h = &addr_stripped[..colon_pos];
        let p = addr_stripped[colon_pos + 1..].parse::<u16>()
            .map_err(|_| io::Error::other(format!("Invalid port in address: {}", addr)))?;
        (h.to_string(), p)
    } else {
        (addr_stripped.to_string(), 9000)
    };

    // Determine if we can connect directly via ws:// or need a proxy for wss://
    let use_proxy = if explicit_protocol == Some("ws") {
        // User explicitly requested ws://, connect directly
        false
    } else if explicit_protocol == Some("wss") {
        // User explicitly requested wss://, need proxy for self-signed cert support
        true
    } else {
        // Auto-detect: try ws:// first with a quick WebSocket handshake probe
        !probe_ws_connection(&host, port)
    };

    if use_proxy {
        // WSS mode: start local proxy that handles TLS with self-signed cert support
        let runtime = tokio::runtime::Runtime::new()?;
        let _guard = runtime.enter();

        let local_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let local_port = local_listener.local_addr()?.port();
        local_listener.set_nonblocking(true)?;
        let tokio_listener = tokio::net::TcpListener::from_std(local_listener)?;

        let remote_wss = format!("wss://{}:{}", host, port);
        let remote_ws = format!("ws://{}:{}", host, port);

        runtime.handle().spawn(async move {
            loop {
                let Ok((local_stream, _)) = tokio_listener.accept().await else { continue };
                let wss = remote_wss.clone();
                let ws = remote_ws.clone();
                tokio::spawn(async move {
                    let _ = ws_proxy_bridge(local_stream, &wss, &ws).await;
                });
            }
        });

        let params = WebViewParams {
            ws_host: "127.0.0.1".to_string(),
            ws_port: local_port,
            ws_protocol: "ws".to_string(),
            auto_password: None,
            theme_css: load_user_theme_css(),
            server_host: Some(host.clone()),
            server_port: Some(port),
            server_secure: true,
        };

        let result = create_webview_window("Clay", &params, None);
        runtime.shutdown_background();
        result
    } else {
        // Direct ws:// connection — no proxy needed
        let params = WebViewParams {
            ws_host: host.clone(),
            ws_port: port,
            ws_protocol: "ws".to_string(),
            auto_password: None,
            theme_css: load_user_theme_css(),
            server_host: Some(host),
            server_port: Some(port),
            server_secure: false,
        };

        create_webview_window("Clay", &params, None)
    }
}

/// Quick probe to check if a ws:// WebSocket connection can be established.
/// Attempts a TCP connect + HTTP upgrade handshake with a short timeout.
fn probe_ws_connection(host: &str, port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let addr = format!("{}:{}", host, port);
    let stream = match TcpStream::connect_timeout(
        &addr.parse().unwrap_or_else(|_| {
            // Fallback: resolve via ToSocketAddrs
            use std::net::ToSocketAddrs;
            addr.to_socket_addrs()
                .ok()
                .and_then(|mut it| it.next())
                .unwrap_or_else(|| ([127, 0, 0, 1], port).into())
        }),
        Duration::from_secs(3),
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));

    // Send a minimal WebSocket upgrade request
    let request = format!(
        "GET / HTTP/1.1\r\n\
         Host: {}:{}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n",
        host, port
    );

    let mut stream = stream;
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    // Read response — look for "101 Switching Protocols" (ws://) vs TLS alert or error
    let mut buf = [0u8; 256];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => {
            let response = String::from_utf8_lossy(&buf[..n]);
            response.contains("101")
        }
        _ => false,
    }
}

/// Bridge a local WebSocket connection to a remote WebSocket server.
/// Tries WSS first (with self-signed cert support), falls back to WS.
/// Forwards messages bidirectionally until either side disconnects.
async fn ws_proxy_bridge(
    local_stream: tokio::net::TcpStream,
    wss_url: &str,
    ws_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures::{SinkExt, StreamExt};

    // Accept local WebSocket from WebView
    let local_ws = tokio_tungstenite::accept_async(local_stream).await?;
    let (mut local_sink, mut local_source) = local_ws.split();

    // Try WSS first with self-signed cert support, fall back to WS
    let remote_ws = {
        #[cfg(feature = "rustls-backend")]
        {
            let tls_config = rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(std::sync::Arc::new(
                    crate::platform::danger::NoCertificateVerification::new()
                ))
                .with_no_client_auth();
            let connector = tokio_tungstenite::Connector::Rustls(
                std::sync::Arc::new(tls_config)
            );
            match tokio_tungstenite::connect_async_tls_with_config(
                wss_url, None, false, Some(connector),
            ).await {
                Ok((ws, _)) => ws,
                Err(_) => {
                    // WSS failed, try plain WS
                    tokio_tungstenite::connect_async(ws_url).await?.0
                }
            }
        }
        #[cfg(not(feature = "rustls-backend"))]
        {
            // Without rustls, try plain connect (handles both ws:// and wss://)
            match tokio_tungstenite::connect_async(wss_url).await {
                Ok((ws, _)) => ws,
                Err(_) => tokio_tungstenite::connect_async(ws_url).await?.0,
            }
        }
    };
    let (mut remote_sink, mut remote_source) = remote_ws.split();

    // Forward messages in both directions
    let local_to_remote = async {
        while let Some(Ok(msg)) = local_source.next().await {
            if msg.is_close() { break; }
            if remote_sink.send(msg).await.is_err() { break; }
        }
    };

    let remote_to_local = async {
        while let Some(Ok(msg)) = remote_source.next().await {
            if msg.is_close() { break; }
            if local_sink.send(msg).await.is_err() { break; }
        }
    };

    tokio::select! {
        _ = local_to_remote => {},
        _ = remote_to_local => {},
    }

    Ok(())
}

/// Build HTML content with WS params injected into template placeholders,
/// exactly like the HTTP server does. This ensures the inline <script> block
/// in the HTML has the correct values when it executes.
fn build_html(params: &WebViewParams) -> String {
    let mut html = WEB_INDEX_HTML
        .replace("{{WS_HOST}}", &params.ws_host)
        .replace("{{WS_PORT}}", &params.ws_port.to_string())
        .replace("{{WS_PROTOCOL}}", &params.ws_protocol)
        .replace("{{THEME_CSS_VARS}}", &params.theme_css);

    // Inject world lock from env var (set by parent when spawning /window <world>)
    if let Ok(world_name) = std::env::var("CLAY_WINDOW_WORLD") {
        std::env::remove_var("CLAY_WINDOW_WORLD");
        html = html.replace(
            &format!("window.WS_PROTOCOL = '{}';", params.ws_protocol),
            &format!("window.WS_PROTOCOL = '{}';\n        window.LOCK_WORLD = '{}';",
                params.ws_protocol, world_name.replace('\'', "\\'")),
        );
    }

    // Inject real server address for /window command (when using WS proxy)
    if let (Some(ref real_host), Some(real_port)) = (&params.server_host, params.server_port) {
        let proto = if params.server_secure { "https" } else { "http" };
        html = html.replace(
            &format!("window.WS_PROTOCOL = '{}';", params.ws_protocol),
            &format!("window.WS_PROTOCOL = '{}';\n        window.SERVER_URL = '{}://{}:{}';",
                params.ws_protocol, proto, real_host, real_port),
        );
    }

    // Inject AUTO_PASSWORD into the template's existing <script> block
    if let Some(ref pw_hash) = params.auto_password {
        html = html.replace(
            &format!("window.WS_PROTOCOL = '{}';", params.ws_protocol),
            &format!("window.WS_PROTOCOL = '{}';\n        window.AUTO_PASSWORD = '{}';", params.ws_protocol, pw_hash),
        );
    }

    // Inject WebView-specific CSS and JS overrides
    let webview_overrides = r#"<style>
/* WebView overrides - native app, not mobile browser */
#output-container { padding: 2px 4px; -webkit-user-select: text; user-select: text; cursor: text; scrollbar-width: none; }
.clay-scrollbar { position: fixed; right: 1px; width: 5px; background: rgba(255,255,255,0.25); border-radius: 3px; z-index: 50; opacity: 0; transition: opacity 0.3s; pointer-events: none; }
#input-container { padding: 2px 4px; }
#output { -webkit-user-select: text; user-select: text; }
#output * { -webkit-user-select: text; user-select: text; }
#nav-bar { display: none !important; }
.wv-sel-overlay { position: absolute; background: rgba(50,120,220,0.35); pointer-events: none; z-index: 999; }
.wv-ctx-menu {
    position: fixed; z-index: 10000; background: #2a2a2a; border: 1px solid #555;
    border-radius: 4px; padding: 2px 0; min-width: 80px; box-shadow: 0 2px 8px rgba(0,0,0,0.5);
    font-family: system-ui, sans-serif; font-size: 13px;
}
.wv-ctx-menu div {
    padding: 4px 16px; color: #ddd; cursor: default;
}
.wv-ctx-menu div:hover { background: #3a5a8a; }
.wv-ctx-menu div.disabled { color: #666; }
.wv-ctx-menu div.disabled:hover { background: transparent; }
</style>
<script>
window.WEBVIEW_MODE = true;
window.WEBVIEW_DEVICE_OVERRIDE = 'desktop';
document.addEventListener('DOMContentLoaded', function() {
    /* Custom scroll indicator — appended to body (not output-container) to avoid layout interference */
    var scrollThumb = document.createElement('div');
    scrollThumb.className = 'clay-scrollbar';
    var scrollTimer = null;
    function initScrollbar() {
        var oc = document.getElementById('output-container');
        if (!oc) return;
        document.body.appendChild(scrollThumb);
        oc.addEventListener('scroll', function() {
            var sh = oc.scrollHeight - oc.clientHeight;
            if (sh <= 0) { scrollThumb.style.opacity = '0'; return; }
            var ratio = oc.scrollTop / sh;
            var thumbH = Math.max(20, (oc.clientHeight / oc.scrollHeight) * oc.clientHeight);
            var ocRect = oc.getBoundingClientRect();
            var thumbTop = ocRect.top + ratio * (oc.clientHeight - thumbH);
            scrollThumb.style.height = thumbH + 'px';
            scrollThumb.style.top = thumbTop + 'px';
            scrollThumb.style.opacity = '1';
            if (scrollTimer) clearTimeout(scrollTimer);
            scrollTimer = setTimeout(function() { scrollThumb.style.opacity = '0'; }, 1500);
        });
    }
    setTimeout(initScrollbar, 100);

    /* Selection highlight overlays */
    var overlays = [];
    function clearOverlays() {
        for (var i = 0; i < overlays.length; i++) overlays[i].remove();
        overlays = [];
    }
    function addOverlays() {
        var sel = window.getSelection();
        if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return;
        var oc = document.getElementById('output-container');
        if (!oc) return;
        var range = sel.getRangeAt(0);
        var rects = range.getClientRects();
        var cr = oc.getBoundingClientRect();
        for (var i = 0; i < rects.length; i++) {
            var r = rects[i];
            var d = document.createElement('div');
            d.className = 'wv-sel-overlay';
            d.style.left = (r.left - cr.left + oc.scrollLeft) + 'px';
            d.style.top = (r.top - cr.top + oc.scrollTop) + 'px';
            d.style.width = r.width + 'px';
            d.style.height = r.height + 'px';
            oc.appendChild(d);
            overlays.push(d);
        }
    }

    /* Custom context menu (replaces WebKit default) */
    var ctxMenu = null;
    function closeCtxMenu() {
        if (ctxMenu) { ctxMenu.remove(); ctxMenu = null; }
        clearOverlays();
    }
    /* Find data-line-idx values for spans within a selection range */
    function getSelectedLineIndices() {
        var sel = window.getSelection();
        if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return [];
        var range = sel.getRangeAt(0);
        var output = document.getElementById('output');
        if (!output) return [];
        var spans = output.querySelectorAll('span[data-line-idx]');
        var indices = [];
        for (var i = 0; i < spans.length; i++) {
            if (range.intersectsNode(spans[i])) {
                indices.push(parseInt(spans[i].getAttribute('data-line-idx'), 10));
            }
        }
        return indices;
    }

    /* Debug selection popup */
    var debugModal = null;
    function closeDebugModal() {
        if (debugModal) { debugModal.remove(); debugModal = null; }
    }
    function showDebugModal(text) {
        closeDebugModal();
        debugModal = document.createElement('div');
        debugModal.style.cssText = 'position:fixed;top:0;left:0;right:0;bottom:0;background:rgba(0,0,0,0.6);z-index:20000;display:flex;align-items:center;justify-content:center;';
        var box = document.createElement('div');
        box.style.cssText = 'background:#1a1a1a;border:1px solid #555;border-radius:6px;padding:12px;max-width:90%;max-height:80%;display:flex;flex-direction:column;gap:8px;min-width:300px;';
        var title = document.createElement('div');
        title.textContent = 'Debug Selection';
        title.style.cssText = 'color:#ddd;font-family:system-ui,sans-serif;font-size:14px;font-weight:bold;';
        box.appendChild(title);
        var pre = document.createElement('pre');
        pre.textContent = text;
        pre.style.cssText = 'color:#c0c0c0;background:#000;padding:8px;border-radius:4px;overflow:auto;max-height:60vh;margin:0;font-size:13px;white-space:pre-wrap;word-break:break-all;';
        box.appendChild(pre);
        var btnRow = document.createElement('div');
        btnRow.style.cssText = 'display:flex;gap:8px;justify-content:flex-end;';
        var copyBtn = document.createElement('button');
        copyBtn.textContent = 'Copy';
        copyBtn.style.cssText = 'padding:4px 16px;background:#3a5a8a;color:#ddd;border:1px solid #555;border-radius:4px;cursor:pointer;font-size:13px;';
        copyBtn.onclick = function() {
            var ta = document.createElement('textarea');
            ta.value = text; ta.style.position = 'fixed'; ta.style.left = '-9999px';
            document.body.appendChild(ta); ta.select(); document.execCommand('copy');
            ta.remove(); copyBtn.textContent = 'Copied!';
            setTimeout(function() { copyBtn.textContent = 'Copy'; }, 1500);
        };
        btnRow.appendChild(copyBtn);
        var closeBtn = document.createElement('button');
        closeBtn.textContent = 'Close';
        closeBtn.style.cssText = 'padding:4px 16px;background:#333;color:#ddd;border:1px solid #555;border-radius:4px;cursor:pointer;font-size:13px;';
        closeBtn.onclick = closeDebugModal;
        btnRow.appendChild(closeBtn);
        box.appendChild(btnRow);
        debugModal.appendChild(box);
        debugModal.addEventListener('mousedown', function(ev) { if (ev.target === debugModal) closeDebugModal(); });
        document.body.appendChild(debugModal);
    }

    document.addEventListener('contextmenu', function(e) {
        /* Only override context menu in the output area; use browser default elsewhere (input, etc.) */
        if (!e.target.closest('#output-container')) return;
        e.preventDefault();
        closeCtxMenu();
        var sel = window.getSelection();
        var hasSelection = sel && sel.toString().length > 0;
        var lineIndices = hasSelection ? getSelectedLineIndices() : [];
        /* Show selection overlays while menu is open */
        if (hasSelection) addOverlays();
        ctxMenu = document.createElement('div');
        ctxMenu.className = 'wv-ctx-menu';
        var copyItem = document.createElement('div');
        copyItem.textContent = 'Copy';
        if (!hasSelection) copyItem.className = 'disabled';
        copyItem.addEventListener('mousedown', function(ev) {
            ev.stopPropagation();
            if (hasSelection) document.execCommand('copy');
            closeCtxMenu();
        });
        ctxMenu.appendChild(copyItem);
        /* Debug Selection - show raw ANSI codes */
        var debugItem = document.createElement('div');
        debugItem.textContent = 'Debug Selection';
        if (!hasSelection || lineIndices.length === 0) debugItem.className = 'disabled';
        debugItem.addEventListener('mousedown', function(ev) {
            ev.stopPropagation();
            closeCtxMenu();
            if (hasSelection && lineIndices.length > 0 && window.getDebugSelectionText) {
                var rawText = window.getDebugSelectionText(lineIndices);
                showDebugModal(rawText);
            }
        });
        ctxMenu.appendChild(debugItem);
        /* Position near cursor, keep on screen */
        var x = e.clientX, y = e.clientY;
        ctxMenu.style.left = x + 'px';
        ctxMenu.style.top = y + 'px';
        document.body.appendChild(ctxMenu);
        var rect = ctxMenu.getBoundingClientRect();
        if (rect.right > window.innerWidth) ctxMenu.style.left = (window.innerWidth - rect.width - 4) + 'px';
        if (rect.bottom > window.innerHeight) ctxMenu.style.top = (window.innerHeight - rect.height - 4) + 'px';
    });
    document.addEventListener('mousedown', function(e) {
        if (ctxMenu && !ctxMenu.contains(e.target)) closeCtxMenu();
        else if (!ctxMenu) clearOverlays();
    });
    document.addEventListener('keydown', function() { closeCtxMenu(); });
});
</script>"#;
    html = html.replace("</head>", &format!("{}\n</head>", webview_overrides));

    html
}

/// Build a WebView for a given window, returning the WebView.
/// Extracts WebView construction so it can be reused for new windows.
fn build_webview(
    window: &tao::window::Window,
    params: &WebViewParams,
    proxy: &EventLoopProxy<WvEvent>,
    reload_tx: &Option<tokio::sync::mpsc::UnboundedSender<crate::WsMessage>>,
    world_lock: Option<&str>,
    extra_js: Option<&str>,
) -> io::Result<wry::WebView> {
    // Build HTML with WS params baked into template placeholders
    let mut html_content = build_html(params);

    // Inject world lock if provided (for new windows locked to a specific world)
    if let Some(world_name) = world_lock {
        html_content = html_content.replace(
            &format!("window.WS_PROTOCOL = '{}';", params.ws_protocol),
            &format!("window.WS_PROTOCOL = '{}';\n        window.LOCK_WORLD = '{}';",
                params.ws_protocol, world_name.replace('\'', "\\'")),
        );
    }

    // Inject extra JavaScript (e.g. GREP_MODE for grep windows)
    if let Some(js) = extra_js {
        html_content = html_content.replace(
            &format!("window.WS_PROTOCOL = '{}';", params.ws_protocol),
            &format!("window.WS_PROTOCOL = '{}';\n        {}", params.ws_protocol, js),
        );
    }

    let css_content = WEB_STYLE_CSS.to_string();
    let js_content = WEB_APP_JS.to_string();

    let builder = WebViewBuilder::new()
        .with_custom_protocol("clay".into(), move |_id, request| {
            let path = request.uri().path();
            let (content_type, body): (&str, Cow<'static, [u8]>) = match path {
                "/" | "/index.html" => ("text/html", Cow::Owned(html_content.as_bytes().to_vec())),
                "/style.css" => ("text/css", Cow::Owned(css_content.as_bytes().to_vec())),
                "/app.js" => ("application/javascript", Cow::Owned(js_content.as_bytes().to_vec())),
                "/clay2.png" => ("image/png", Cow::Borrowed(CLAY_LOGO_PNG)),
                _ => ("text/plain", Cow::Borrowed(b"Not Found")),
            };
            wry::http::Response::builder()
                .header("Content-Type", content_type)
                .header("Access-Control-Allow-Origin", "*")
                .body(body)
                .unwrap()
        })
        .with_url("clay://localhost/index.html")
        .with_clipboard(true)
        .with_devtools(cfg!(debug_assertions) || cfg!(target_os = "android"))
        .with_ipc_handler({
            let is_master = params.auto_password.is_some();
            let proxy = proxy.clone();
            let reload_tx = reload_tx.clone();
            move |req| {
            let body = req.body();
            if body.starts_with("open-url:") {
                let url = &body[9..];
                open_url_in_browser(url);
            } else if body.starts_with("new-window:") {
                // Open a new WebView window in-process (optionally locked to a world)
                let world_name = body[11..].trim().to_string();
                let world = if world_name.is_empty() { None } else { Some(world_name) };
                let _ = proxy.send_event(WvEvent::NewWindow(world));
            } else if body.starts_with("grep-window:") {
                // Open a grep results window (half height, filtered output)
                let json_str = &body[12..];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    let pattern = v["pattern"].as_str().unwrap_or("").to_string();
                    let world = v["world"].as_str().map(|s| s.to_string());
                    let use_regex = v["regex"].as_bool().unwrap_or(false);
                    if !pattern.is_empty() {
                        let _ = proxy.send_event(WvEvent::GrepWindow { pattern, world, use_regex });
                    }
                }
            } else if body == "quit" {
                let _ = proxy.send_event(WvEvent::Quit);
            } else if body == "update" || body == "update-force" {
                #[cfg(not(target_os = "android"))]
                {
                    let force = body == "update-force";
                    let proxy_clone = proxy.clone();
                    let is_master_clone = is_master;
                    let reload_tx_clone = reload_tx.clone();
                    std::thread::spawn(move || {
                        let rt = match tokio::runtime::Runtime::new() {
                            Ok(rt) => rt,
                            Err(e) => {
                                let _ = proxy_clone.send_event(WvEvent::UpdateStatus(
                                    format!("Failed to start update: {}", e)
                                ));
                                return;
                            }
                        };
                        let result = rt.block_on(crate::platform::check_and_download_update(force));
                        match result {
                            Ok(success) => {
                                let msg = install_update(&success.temp_path, &success.version);
                                let is_success = msg.starts_with("Updated to");
                                let _ = proxy_clone.send_event(WvEvent::UpdateStatus(msg));
                                if is_success {
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                    if is_master_clone {
                                        crate::GUI_RELOAD_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
                                        if let Some(ref tx) = reload_tx_clone {
                                            let _ = tx.send(crate::WsMessage::SendCommand {
                                                world_index: 0,
                                                command: "/reload".to_string(),
                                            });
                                        }
                                    } else {
                                        let _ = proxy_clone.send_event(WvEvent::Reload);
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = proxy_clone.send_event(WvEvent::UpdateStatus(e));
                            }
                        }
                    });
                }
                #[cfg(target_os = "android")]
                {
                    let _ = proxy.send_event(WvEvent::UpdateStatus("Update not available on Android".to_string()));
                }
            } else if body == "reload" {
                // Hot reload: master mode uses both atomic flag AND channel for reliability
                // (can't use SIGUSR1 because WebKit/JSC overrides the signal handler),
                // remote mode sends WvEvent to exec a new binary.
                if is_master {
                    // Set atomic flag (checked by headless event loop on 100ms timer)
                    crate::GUI_RELOAD_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
                    // Also try channel as backup path
                    if let Some(ref tx) = reload_tx {
                        let _ = tx.send(crate::WsMessage::SendCommand {
                            world_index: 0,
                            command: "/reload".to_string(),
                        });
                    }
                } else {
                    // Remote mode: restart the GUI binary
                    let _ = proxy.send_event(WvEvent::Reload);
                }
            } else if let Some(rest) = body.strip_prefix("opacity:") {
                if let Ok(opacity) = rest.parse::<f64>() {
                    let _ = proxy.send_event(WvEvent::SetOpacity(opacity));
                }
            }
        }})
        // Open external links in the system browser instead of navigating the WebView.
        // Links use target="_blank", which triggers new_window_req_handler.
        .with_new_window_req_handler(|url| {
            // On Windows WebView2, custom protocol "clay" is served as http://clay.localhost/
            if url.starts_with("clay://") || url.contains("://clay.localhost") {
                false // our protocol — don't open in browser
            } else {
                open_url_in_browser(&url);
                false
            }
        })
        // Block navigation away from our app (e.g. if a link doesn't use target="_blank")
        .with_navigation_handler(|url| {
            // On Windows WebView2, custom protocol "clay" is served as http://clay.localhost/
            if url.starts_with("clay://") || url.contains("://clay.localhost") {
                true // allow our custom protocol
            } else {
                open_url_in_browser(&url);
                false // block and open in browser
            }
        });

    // On Windows, disable browser accelerator keys (Ctrl+L, Ctrl+N, etc.)
    // so they reach our JS key handler instead of being swallowed by WebView2.
    #[cfg(windows)]
    let builder = {
        use wry::WebViewBuilderExtWindows;
        builder.with_browser_accelerator_keys(false)
    };

    // On Linux/GTK, must use build_gtk() to properly embed WebView in tao's GTK container.
    // build(&window) silently produces a non-visible webview on Linux.
    // On Android/Termux (patched tao), use regular build() — no GTK available.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
        ),
        not(target_os = "android"),
    ))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox()
            .ok_or_else(|| io::Error::other("Failed to get GTK vbox from window"))?;
        builder.build_gtk(vbox)
            .map_err(|e| io::Error::other(format!("Failed to create WebView: {}", e)))?
    };

    // On macOS, Windows, and Android/Termux (patched tao): use regular build()
    #[cfg(any(
        target_os = "android",
        not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
        )),
    ))]
    let webview = builder.build(window)
        .map_err(|e| io::Error::other(format!("Failed to create WebView: {}", e)))?;

    Ok(webview)
}

/// Create and run the WebView window with custom protocol to serve embedded web content.
/// Uses custom protocol (clay://) instead of with_html() because with_html() loads
/// content with a null origin, which blocks WebSocket connections on WebKit2GTK.
/// Supports multiple windows within a single event loop.
fn create_webview_window(
    title: &str,
    params: &WebViewParams,
    reload_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::WsMessage>>,
) -> io::Result<()> {
    let event_loop = EventLoopBuilder::<WvEvent>::with_user_event().build();
    let proxy: EventLoopProxy<WvEvent> = event_loop.create_proxy();

    let window_theme = if load_gui_theme_name() == "light" {
        Some(tao::window::Theme::Light)
    } else {
        Some(tao::window::Theme::Dark)
    };
    let window = WindowBuilder::new()
        .with_title(title)
        .with_theme(window_theme)
        .with_inner_size(tao::dpi::LogicalSize::new(800.0, 600.0))
        .build(&event_loop)
        .map_err(|e| io::Error::other(format!("Failed to create window: {}", e)))?;

    // Enable spell checking on Linux (WebKit2GTK)
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "android",
    ))]
    {
        use webkit2gtk::{WebContextExt, WebContext};
        let ctx = WebContext::default().unwrap();
        ctx.set_spell_checking_enabled(true);
        ctx.set_spell_checking_languages(&["en"]);
    }

    // Read initial world lock from env var (set by CLAY_WINDOW_WORLD for backward compat)
    let initial_world_lock: Option<String> = std::env::var("CLAY_WINDOW_WORLD").ok();

    // Suppress WebKit/JSC stderr noise during WebView init only.
    // Must not wrap EventLoopBuilder or WindowBuilder — GTK/tao panics there would be swallowed.
    #[cfg(unix)]
    let _stderr_guard = StderrSuppress::new();

    // Build the first webview (initial_world_lock is handled inside build_html via env var)
    let webview = build_webview(&window, params, &proxy, &reload_tx, None, None)?;

    #[cfg(unix)]
    drop(_stderr_guard);

    // Store windows and webviews in HashMaps keyed by WindowId for multi-window support
    let mut windows: HashMap<WindowId, tao::window::Window> = HashMap::new();
    let mut webviews: HashMap<WindowId, wry::WebView> = HashMap::new();
    let window_id = window.id();
    windows.insert(window_id, window);
    webviews.insert(window_id, webview);

    // Suppress the unused variable warning — initial_world_lock was read from env
    // and consumed by build_html() via CLAY_WINDOW_WORLD env var
    let _ = initial_world_lock;

    // Clone params for use inside the event loop closure (needed for creating new windows)
    let params = params.clone();

    event_loop.run(move |event, event_loop_target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                // Remove the closed window and its webview
                windows.remove(&window_id);
                webviews.remove(&window_id);
                // Only exit if all windows are closed
                if windows.is_empty() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(WvEvent::Quit) => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(WvEvent::SetOpacity(opacity)) => {
                // Broadcast opacity to all windows
                #[cfg(all(
                    any(
                        target_os = "linux",
                        target_os = "dragonfly",
                        target_os = "freebsd",
                        target_os = "netbsd",
                        target_os = "openbsd",
                    ),
                    not(target_os = "android"),
                ))]
                {
                    use tao::platform::unix::WindowExtUnix;
                    use gtk::prelude::WidgetExt;
                    for win in windows.values() {
                        let gtk_win = win.gtk_window();
                        gtk_win.set_opacity(opacity);
                    }
                }
                #[cfg(any(
                    target_os = "android",
                    not(any(
                        target_os = "linux",
                        target_os = "dragonfly",
                        target_os = "freebsd",
                        target_os = "netbsd",
                        target_os = "openbsd",
                    )),
                ))]
                { let _ = opacity; }
            }
            Event::UserEvent(WvEvent::UpdateStatus(ref msg)) => {
                // Broadcast update status to all webviews
                let escaped = msg.replace('\\', "\\\\").replace('\'', "\\'");
                let script = format!("window.showUpdateStatus('{}')", escaped);
                for wv in webviews.values() {
                    let _ = wv.evaluate_script(&script);
                }
            }
            Event::UserEvent(WvEvent::Reload) => {
                // Remote GUI reload: restart the binary (no state to save — state lives on server)
                // Show error in first available webview if exec fails
                let _first_wv = webviews.values().next();
                #[cfg(all(unix, not(target_os = "android")))]
                {
                    if let Ok((exe_path, _)) = crate::get_executable_path() {
                        let args: Vec<String> = std::env::args().collect();
                        let c_exe = std::ffi::CString::new(exe_path.to_string_lossy().as_bytes()).unwrap();
                        let c_args: Vec<std::ffi::CString> = args.iter()
                            .map(|a| std::ffi::CString::new(a.as_bytes()).unwrap())
                            .collect();
                        let c_arg_ptrs: Vec<*const libc::c_char> = c_args.iter()
                            .map(|a| a.as_ptr())
                            .chain(std::iter::once(std::ptr::null()))
                            .collect();
                        unsafe { libc::execv(c_exe.as_ptr(), c_arg_ptrs.as_ptr()); }
                        // If exec fails, show error in WebView
                        if let Some(wv) = _first_wv {
                            let _ = wv.evaluate_script(
                                "window.showUpdateStatus('Reload failed: exec error')"
                            );
                        }
                    } else if let Some(wv) = _first_wv {
                        let _ = wv.evaluate_script(
                            "window.showUpdateStatus('Reload failed: cannot find binary')"
                        );
                    }
                }
                #[cfg(windows)]
                {
                    if let Ok((exe_path, _)) = crate::get_executable_path() {
                        let args: Vec<String> = std::env::args().collect();
                        match std::process::Command::new(&exe_path).args(&args[1..]).spawn() {
                            Ok(_) => std::process::exit(0),
                            Err(_) => {
                                if let Some(wv) = _first_wv {
                                    let _ = wv.evaluate_script(
                                        "window.showUpdateStatus('Reload failed: spawn error')"
                                    );
                                }
                            }
                        }
                    } else if let Some(wv) = _first_wv {
                        let _ = wv.evaluate_script(
                            "window.showUpdateStatus('Reload failed: cannot find binary')"
                        );
                    }
                }
                #[cfg(not(any(unix, windows)))]
                {
                    if let Some(wv) = _first_wv {
                        let _ = wv.evaluate_script(
                            "window.showUpdateStatus('Reload not supported on this platform')"
                        );
                    }
                }
            }
            Event::UserEvent(WvEvent::NewWindow(ref world)) => {
                // Create a new window in the same process
                let win_title = match world {
                    Some(ref w) => format!("Clay - {}", w),
                    None => "Clay".to_string(),
                };
                let new_window = match WindowBuilder::new()
                    .with_title(&win_title)
                    .with_theme(window_theme)
                    .with_inner_size(tao::dpi::LogicalSize::new(800.0, 600.0))
                    .build(event_loop_target)
                {
                    Ok(w) => w,
                    Err(_) => return,
                };

                let world_lock = world.as_deref();
                match build_webview(&new_window, &params, &proxy, &reload_tx, world_lock, None) {
                    Ok(wv) => {
                        let id = new_window.id();
                        windows.insert(id, new_window);
                        webviews.insert(id, wv);
                    }
                    Err(_) => {
                        // Window will be dropped, which is fine
                    }
                }
            }
            Event::UserEvent(WvEvent::GrepWindow { ref pattern, ref world, use_regex }) => {
                // Create a half-height grep results window
                let win_title = format!("Clay - grep: {}", pattern);
                let new_window = match WindowBuilder::new()
                    .with_title(&win_title)
                    .with_theme(window_theme)
                    .with_inner_size(tao::dpi::LogicalSize::new(800.0, 300.0))
                    .build(event_loop_target)
                {
                    Ok(w) => w,
                    Err(_) => return,
                };

                // Build GREP_MODE JS injection
                let escaped_pattern = pattern.replace('\\', "\\\\").replace('\'', "\\'");
                let grep_js = format!(
                    "window.GREP_MODE = {{ pattern: '{}', regex: {} }};",
                    escaped_pattern, use_regex
                );
                let world_lock = world.as_deref();
                match build_webview(&new_window, &params, &proxy, &reload_tx, world_lock, Some(&grep_js)) {
                    Ok(wv) => {
                        let id = new_window.id();
                        windows.insert(id, new_window);
                        webviews.insert(id, wv);
                    }
                    Err(_) => {}
                }
            }
            _ => {}
        }
    });
}

/// Install a downloaded update binary, returning a status message
#[cfg(not(target_os = "android"))]
fn install_update(temp_path: &std::path::Path, version: &str) -> String {
    // Get current executable path
    let exe_path = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_file(temp_path);
            return format!("Cannot find current binary: {}", e);
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(temp_path, std::fs::Permissions::from_mode(0o755)) {
            let _ = std::fs::remove_file(temp_path);
            return format!("Failed to set permissions: {}", e);
        }
    }

    // Try rename first, fall back to copy (may fail cross-device)
    if let Err(e) = std::fs::rename(temp_path, &exe_path) {
        match std::fs::copy(temp_path, &exe_path) {
            Ok(_) => {
                let _ = std::fs::remove_file(temp_path);
            }
            Err(e2) => {
                // On Windows, can't overwrite running exe — try rename-and-replace
                #[cfg(windows)]
                {
                    let old_path = exe_path.with_extension("exe.old");
                    let _ = std::fs::remove_file(&old_path); // clean up previous .old
                    if std::fs::rename(&exe_path, &old_path).is_ok() {
                        if let Err(e3) = std::fs::rename(temp_path, &exe_path) {
                            // Restore the old binary
                            let _ = std::fs::rename(&old_path, &exe_path);
                            let _ = std::fs::remove_file(temp_path);
                            return format!("Failed to install update: {}", e3);
                        }
                        return format!("Updated to Clay v{} — reloading...", version);
                    }
                }
                let _ = std::fs::remove_file(temp_path);
                return format!("Failed to install update: {} (rename: {})", e2, e);
            }
        }
    }

    format!("Updated to Clay v{} — reloading...", version)
}
