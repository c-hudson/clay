// WebView GUI client using wry (native WebView window)
//
// Provides two modes:
// --webgui        Master mode: runs App headlessly + opens WebView window (local WS connection)
// --webgui=host:port  Remote mode: opens WebView window connected to remote Clay instance

use std::borrow::Cow;
use std::io;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

/// Custom events sent from IPC handler to event loop
#[derive(Debug)]
enum WvEvent {
    /// Set window opacity (0.0 = fully transparent, 1.0 = fully opaque)
    SetOpacity(f64),
}

use crate::theme::{ThemeColors, ThemeFile};
use crate::websocket::hash_password;

/// Open a URL in the system's default browser (platform-specific).
fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "linux")]
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
struct WebViewParams {
    ws_host: String,
    ws_port: u16,
    ws_protocol: String,
    auto_password: Option<String>,
    theme_css: String,
}

/// Load the user's GUI theme CSS vars for initial HTML rendering.
/// Reads gui_theme name from ~/.clay.dat and theme colors from ~/clay.theme.dat.
fn load_user_theme_css() -> String {
    let home = crate::get_home_dir();
    if home == "." {
        return ThemeColors::dark_default().to_css_vars();
    }

    // Read gui_theme name from ~/.clay.dat (scan for gui_theme= line)
    let settings_path = format!("{}/{}", home, crate::clay_filename("clay.dat"));
    let gui_theme_name = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|content| {
            content.lines()
                .find(|l| l.starts_with("gui_theme="))
                .map(|l| l.trim_start_matches("gui_theme=").to_string())
        })
        .unwrap_or_else(|| "dark".to_string());

    // Load theme colors from ~/clay.theme.dat
    let theme_path = format!("{}/{}", home, crate::clay_filename("clay.theme.dat"));
    let theme_file = ThemeFile::load(std::path::Path::new(&theme_path));
    theme_file.get(&gui_theme_name).to_css_vars()
}

/// Master mode: run App headlessly with a local WebSocket, open WebView window
pub fn run_master_webgui() -> io::Result<()> {

    // Check for display server availability (Linux only)
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let has_display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
        if !has_display {
            return Err(io::Error::other(
                "No display server found. Set DISPLAY (X11) or WAYLAND_DISPLAY environment variable."
            ));
        }
    }

    // Pick a random available port by binding to port 0
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| io::Error::other(format!("Failed to find available port: {}", e)))?;
        listener.local_addr()?.port()
    };
    // listener is dropped here, freeing the port for the WS server

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
    let (_gui_to_app_tx, gui_to_app_rx) = tokio::sync::mpsc::unbounded_channel::<crate::WsMessage>();

    // Spawn the headless App with WS override
    let ws_password = password.clone();
    handle.spawn(async move {
        if let Err(e) = crate::run_app_headless(
            app_to_gui_tx,
            gui_to_app_rx,
            Some((port, ws_password)),
            None, // No GUI repaint callback (webview is event-driven)
        ).await {
            eprintln!("App error: {}", e);
        }
    });

    // Wait for the WS server to be ready (retry connecting for up to 3 seconds)
    // Use std::net (not tokio) to avoid "block_on inside runtime" panic
    let ws_ready = {
        let addr = format!("127.0.0.1:{}", port);
        let mut ready = false;
        for _ in 0..30 {
            if std::net::TcpStream::connect(&addr).is_ok() {
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
    };

    let result = create_webview_window("Clay", &params);

    // Shut down the tokio runtime when the window closes
    runtime.shutdown_background();

    result
}

/// Remote mode: open WebView window connected to a remote Clay instance
pub fn run_remote_webgui(addr: &str) -> io::Result<()> {
    // Check for display server availability (Linux only)
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let has_display = std::env::var("DISPLAY").map(|v| !v.is_empty()).unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
        if !has_display {
            return Err(io::Error::other(
                "No display server found. Set DISPLAY (X11) or WAYLAND_DISPLAY environment variable."
            ));
        }
    }

    // Parse host:port
    let (host, port) = if let Some(colon_pos) = addr.rfind(':') {
        let h = &addr[..colon_pos];
        let p = addr[colon_pos + 1..].parse::<u16>()
            .map_err(|_| io::Error::other(format!("Invalid port in address: {}", addr)))?;
        (h.to_string(), p)
    } else {
        return Err(io::Error::other(
            "Address must be in host:port format (e.g., localhost:9001)"
        ));
    };

    let params = WebViewParams {
        ws_host: host,
        ws_port: port,
        ws_protocol: "ws".to_string(),
        auto_password: None,
        theme_css: load_user_theme_css(),
    };

    create_webview_window("Clay", &params)
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
#output-container { padding: 2px 4px; -webkit-user-select: text; user-select: text; cursor: text; }
#input-container { padding: 2px 4px; }
#output { -webkit-user-select: text; user-select: text; }
#output * { -webkit-user-select: text; user-select: text; }
::-webkit-scrollbar { width: 4px; }
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

/// Create and run the WebView window with custom protocol to serve embedded web content.
/// Uses custom protocol (clay://) instead of with_html() because with_html() loads
/// content with a null origin, which blocks WebSocket connections on WebKit2GTK.
fn create_webview_window(title: &str, params: &WebViewParams) -> io::Result<()> {
    let event_loop = EventLoopBuilder::<WvEvent>::with_user_event().build();
    let proxy: EventLoopProxy<WvEvent> = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title(title)
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
    ))]
    {
        use webkit2gtk::{WebContextExt, WebContext};
        let ctx = WebContext::default().unwrap();
        ctx.set_spell_checking_enabled(true);
        ctx.set_spell_checking_languages(&["en"]);
    }

    // Build HTML with WS params baked into template placeholders
    let html_content = build_html(params);
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
        .with_devtools(cfg!(debug_assertions))
        .with_ipc_handler(move |req| {
            let body = req.body();
            if let Some(rest) = body.strip_prefix("opacity:") {
                if let Ok(opacity) = rest.parse::<f64>() {
                    let _ = proxy.send_event(WvEvent::SetOpacity(opacity));
                }
            }
        })
        // Open external links in the system browser instead of navigating the WebView.
        // Links use target="_blank", which triggers new_window_req_handler.
        .with_new_window_req_handler(|url| {
            open_url_in_browser(&url);
            false // don't open a new webview window
        })
        // Block navigation away from our app (e.g. if a link doesn't use target="_blank")
        .with_navigation_handler(|url| {
            if url.starts_with("clay://") {
                true // allow our custom protocol
            } else {
                open_url_in_browser(&url);
                false // block and open in browser
            }
        });

    // On Linux/GTK, must use build_gtk() to properly embed WebView in tao's GTK container.
    // build(&window) silently produces a non-visible webview on Linux.
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    ))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox()
            .ok_or_else(|| io::Error::other("Failed to get GTK vbox from window"))?;
        builder.build_gtk(vbox)
            .map_err(|e| io::Error::other(format!("Failed to create WebView: {}", e)))?
    };

    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
    )))]
    let _webview = builder.build(&window)
        .map_err(|e| io::Error::other(format!("Failed to create WebView: {}", e)))?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(WvEvent::SetOpacity(opacity)) => {
                // Use GTK's window opacity — sets _NET_WM_WINDOW_OPACITY on X11.
                // This is handled by the compositor (xfwm4, etc.) and is instant/reliable,
                // unlike per-pixel alpha through WebKit2GTK's rendering pipeline.
                #[cfg(any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd",
                ))]
                {
                    use tao::platform::unix::WindowExtUnix;
                    use gtk::prelude::WidgetExt;
                    let gtk_win = window.gtk_window();
                    gtk_win.set_opacity(opacity);
                }
                #[cfg(not(any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd",
                    target_os = "openbsd",
                )))]
                { let _ = opacity; }
            }
            _ => {}
        }
    });
}
