use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::sync::RwLock;

// ============================================================================
// PrefixedStream — replays pre-read bytes before reading from inner stream
// ============================================================================

/// A stream wrapper that replays buffered bytes before reading from the inner stream.
/// Used to pass pre-read HTTP headers to the WebSocket handshake handler.
pub struct PrefixedStream<S> {
    prefix: Vec<u8>,
    prefix_pos: usize,
    inner: S,
}

impl<S> PrefixedStream<S> {
    pub fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self { prefix, prefix_pos: 0, inner }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.prefix_pos < this.prefix.len() {
            let remaining = &this.prefix[this.prefix_pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            this.prefix_pos += to_copy;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

// ============================================================================
// Remote Connection Logging
// ============================================================================

/// Log a remote connection event to ~/.clay/remote.log
/// `cargo test` was appending real BANNED/KNOCK-*/... lines to the user's live
/// ~/.clay/remote.log — tests exercise the same gate/ban/knock code paths that
/// production does, but must never touch a real user's files, so this is a no-op in
/// test builds (the `#[cfg(not(test))]` real implementation below never compiles in).
#[cfg(test)]
pub fn log_remote_event(_event_type: &str, _ip: &str, _details: &str) {}

#[cfg(not(test))]
pub fn log_remote_event(event_type: &str, ip: &str, details: &str) {
    use std::io::Write;
    let timestamp = crate::util::local_time_now();
    let time_str = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        timestamp.year, timestamp.month, timestamp.day,
        timestamp.hour, timestamp.minute, timestamp.second);

    let log_line = format!("[{}] {} {} {}\n", time_str, event_type, ip, details);

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::clay_config_path("remote.log"))
    {
        let _ = file.write_all(log_line.as_bytes());
    }
}

/// Log an HTTP 404 error
pub fn log_http_404(ip: &str, path: &str) {
    log_remote_event("HTTP-404", ip, path);
}

/// Log a WebSocket authentication attempt
pub fn log_ws_auth(ip: &str, success: bool, username: Option<&str>) {
    let details = match (success, username) {
        (true, Some(user)) => format!("AUTH-SUCCESS user={}", user),
        (true, None) => "AUTH-SUCCESS".to_string(),
        (false, _) => "AUTH-FAILURE".to_string(),
    };
    log_remote_event("WEBSOCKET", ip, &details);
}

/// Log when an IP is banned
pub fn log_ban(ip: &str, ban_type: &str, reason: &str) {
    let details = format!("{} reason={}", ban_type, reason);
    log_remote_event("BANNED", ip, &details);
}

// ============================================================================
// Shared HTTP constants and helpers (used by all server implementations)
// ============================================================================

/// Embedded HTML for the web interface
const WEB_INDEX_HTML: &str = include_str!("web/index.html");

/// Embedded CSS for the web interface
const WEB_STYLE_CSS: &str = include_str!("web/style.css");

/// Embedded JavaScript for the web interface
const WEB_APP_JS: &str = include_str!("web/app.js");

/// Embedded theme editor HTML
const WEB_THEME_EDITOR_HTML: &str = include_str!("web/theme-editor.html");

/// Embedded keybind editor HTML
const WEB_KEYBIND_EDITOR_HTML: &str = include_str!("web/keybind-editor.html");

/// Embedded action editor HTML
const WEB_ACTION_EDITOR_HTML: &str = include_str!("web/action-editor.html");

/// Bundled fonts (latin subset, variable weight)
const FONT_JETBRAINS_MONO: &[u8] = include_bytes!("web/fonts/jetbrains-mono-latin-400.woff2");
const FONT_NUNITO: &[u8] = include_bytes!("web/fonts/nunito-latin-400.woff2");

/// Seconds to wait for first bytes of an HTTP request before dropping the connection.
const READ_TIMEOUT_SECS: u64 = 10;
/// Seconds to wait for a TLS handshake to complete before recording a violation and dropping.
const TLS_TIMEOUT_SECS: u64 = 10;
/// Maximum simultaneous HTTP/HTTPS connections allowed per IP address.
const MAX_HTTP_CONNECTIONS_PER_IP: usize = 10;
/// Maximum simultaneous WebSocket connections allowed per IP address.
const MAX_WS_CONNECTIONS_PER_IP: usize = 20;

/// Handle a plain HTTP connection on the HTTPS port. Reuses `decide_route` — the exact
/// same decision `route_connection` makes for every other HTTP request — instead of a
/// hand-rolled reachability check. That duplicate check is exactly what caused the
/// reported bug: it never received `in_allow_list`, so an allow-listed user hitting
/// `http://host:9000/` (instead of `https://`) was treated as unreachable and struck,
/// while the identical request over `https://` correctly got the D2 grace redirect.
/// Since the accept-time gate (D3) already drops every non-listed IP before a
/// connection can reach here at all, the only IPs this function can ever strike are
/// allow-listed ones — so keeping this decision byte-for-byte in sync with
/// `decide_route` (by calling it, not re-deriving it) matters more here than anywhere
/// else in this file.
async fn redirect_http_to_https<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    port: u16,
    is_localhost: bool,
    in_allow_list: bool,
    knocked: bool,
    gate: &SecurityGate,
    client_ip: &str,
) {
    let mut buf = [0u8; 4096];
    let n = match tokio::time::timeout(
        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
        stream.read(&mut buf),
    ).await {
        Ok(Ok(n)) if n > 0 => n,
        _ => return,
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let (method, full_path) = match parse_http_request(&request) {
        Some(v) => v,
        None => {
            // The caller only reaches this function when the peeked first byte isn't
            // 0x16 (TLS ClientHello), so a parse failure here can't be the
            // HSTS/HTTPS-first false positive route_connection has to account for —
            // it's genuine garbage. Mirror route_connection's HTTP-DROP + strike.
            log_remote_event("HTTP-DROP", client_ip, "unparseable-request");
            gate.strike(client_ip, "unparseable-request").await;
            return;
        }
    };
    let path = full_path.split('?').next().unwrap_or(full_path);

    let host = get_host_from_request(&request);
    let host = if host.is_empty() { "localhost".to_string() } else { host };
    // C3 (security remediation): the Host header is client-controlled and gets
    // interpolated into both the Location header and the redirect HTML body below —
    // sanitize it the same way handle_http_routes does for its WS_HOST substitution
    // (:493) before it's used for anything, closing a reflected-XSS/open-redirect hole
    // (e.g. `Host: "><script>...`). CRLF injection isn't possible here since `host` is
    // always a single line from `.lines()`.
    let host: String = host.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == ':')
        .collect();
    let build_location = |p: &str| if port == 443 {
        format!("https://{}{}", host, p)
    } else {
        format!("https://{}:{}{}", host, port, p)
    };

    match decide_route(method, path, &gate.web_path, is_localhost, in_allow_list, knocked) {
        RouteDecision::SilentDrop { violation } => {
            if let Some(reason) = violation {
                gate.strike(client_ip, &reason).await;
            }
            log_remote_event("HTTP-DROP", client_ip, path);
        }
        // decide_route already resolved the reachable target (e.g. the D2 grace
        // redirect from "/" to "/{web_path}/" for an allow-listed IP) — send the
        // client straight there over HTTPS instead of bouncing through a second,
        // same-protocol redirect.
        RouteDecision::Redirect(location) => {
            let response = build_redirect_response(&build_location(&location), false);
            let _ = stream.write_all(&response).await;
        }
        // Serve / legacy fallthrough: the original requested path is what would be
        // served (or 404'd/405'd) on the HTTPS side, so redirect to it unchanged.
        RouteDecision::Serve(_) | RouteDecision::NotFoundLegacy | RouteDecision::MethodNotAllowedLegacy => {
            let response = build_redirect_response(&build_location(path), false);
            let _ = stream.write_all(&response).await;
        }
    }
}

/// Parse an HTTP request line and return the method and path
fn parse_http_request(request: &str) -> Option<(&str, &str)> {
    let first_line = request.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    Some((method, path))
}

/// Extract Host header from HTTP request (without port)
fn get_host_from_request(request: &str) -> String {
    for line in request.lines() {
        if line.to_lowercase().starts_with("host:") {
            let host = line[5..].trim();
            // Remove port if present
            if let Some(colon_pos) = host.rfind(':') {
                return host[..colon_pos].to_string();
            }
            return host.to_string();
        }
    }
    String::new()
}

/// Build an HTTP response with the given status, content type, and body
fn build_binary_http_response(status: u16, status_text: &str, content_type: &str, body: &[u8], is_https: bool) -> Vec<u8> {
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: max-age=31536000, immutable\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n",
        status, status_text, content_type, body.len()
    );
    if is_https {
        headers.push_str("Strict-Transport-Security: max-age=31536000\r\n");
    }
    headers.push_str("\r\n");
    let mut response = headers.into_bytes();
    response.extend_from_slice(body);
    response
}

fn build_http_response(status: u16, status_text: &str, content_type: &str, body: &str, is_https: bool) -> Vec<u8> {
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Frame-Options: DENY\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https://cdn.discordapp.com; connect-src 'self' ws: wss:; frame-ancestors 'none'\r\n\
         Connection: close\r\n",
        status, status_text, content_type, body.len()
    );
    if is_https {
        headers.push_str("Strict-Transport-Security: max-age=31536000\r\n");
    }
    headers.push_str("\r\n");
    headers.push_str(body);
    headers.into_bytes()
}

/// Route result from handle_http_routes - indicates whether a 404 violation occurred
enum RouteResult {
    /// Route matched, response ready
    Ok(Vec<u8>),
    /// 404 - path not found (includes the path for violation tracking)
    NotFound(Vec<u8>, String),
    /// Method not allowed
    MethodNotAllowed(Vec<u8>),
}

/// Check if an HTTP request is a WebSocket upgrade request
fn is_websocket_upgrade(request: &str) -> bool {
    for line in request.lines() {
        let lower = line.to_lowercase();
        if lower.starts_with("upgrade:") && lower.contains("websocket") {
            return true;
        }
    }
    false
}

/// Pure routing decision, decoupled from response-body generation. Implements D2 (see
/// SECURITY-ROADMAP.md): stealth `/{web_path}/...` prefix routing, localhost dual
/// roots, the allow-listed grace redirect from `/`, and silent drops (with a
/// favicon/apple-touch-icon strike exemption) for everything else in stealth mode.
/// `knocked` (D4): `route_connection` only reaches `decide_route` for non-WS-upgrade
/// requests (WS upgrades are handled earlier, at any path, for a knocked connection) —
/// so any request that gets here on an already-knocked connection is a static request
/// the knock does not authorize. Silently drop it, no violation (the key was already
/// proven; don't self-ban a buggy client) — `route_connection` logs `KNOCK-HTTP-DENIED`
/// for this case instead of the usual `HTTP-DROP`.
#[derive(Debug, Clone, PartialEq)]
enum RouteDecision {
    Serve(String),
    Redirect(String),
    SilentDrop { violation: Option<String> },
    NotFoundLegacy,
    MethodNotAllowedLegacy,
}

/// Static asset paths served at the legacy root (`/`) or, in stealth mode, under the
/// `/{web_path}/` prefix. A single list so `decide_route`'s allow-check and
/// `handle_http_routes`'s content dispatch can't drift apart.
const KNOWN_ASSET_PATHS: &[&str] = &[
    "/", "/index.html", "/style.css", "/app.js", "/theme-editor",
    "/keybind-editor", "/action-editor",
    "/fonts/jetbrains-mono-latin-400.woff2", "/fonts/nunito-latin-400.woff2",
    "/favicon.ico",
];

fn is_known_asset(path: &str) -> bool {
    KNOWN_ASSET_PATHS.contains(&path)
}

/// Browsers auto-request these regardless of any stealth path prefix — dropped like any
/// other unrecognized path in stealth mode, but never strike a violation for them.
fn is_favicon_exempt(path: &str) -> bool {
    path == "/favicon.ico" || path.starts_with("/apple-touch-icon")
}

/// If `path` starts with the `/{web_path}` segment, return the remainder re-rooted at
/// `/`: `""` for an exact `/{web_path}` match (caller redirects to the trailing-slash
/// form), `"/"` for `/{web_path}/`, `"/rest"` for `/{web_path}/rest`. Returns `None` if
/// `web_path` is empty or `path` doesn't start with the prefix (as a path segment, so
/// `/clayfoo` does not match prefix `/clay`).
fn strip_web_path_prefix(path: &str, web_path: &str) -> Option<String> {
    if web_path.is_empty() {
        return None;
    }
    let prefix = format!("/{}", web_path);
    if path == prefix {
        return Some(String::new());
    }
    let rest = path.strip_prefix(&prefix)?;
    let after_slash = rest.strip_prefix('/')?;
    Some(if after_slash.is_empty() { "/".to_string() } else { format!("/{}", after_slash) })
}

fn decide_route(
    method: &str,
    path: &str,
    web_path: &str,
    is_localhost: bool,
    in_allow_list: bool,
    knocked: bool,
) -> RouteDecision {
    // D4: a knock is transport gating only — it does not authorize static HTTP
    // content. route_connection dispatches WS upgrades before ever calling
    // decide_route, so reaching here on a knocked connection means a static request;
    // silently drop it (no violation).
    if knocked {
        return RouteDecision::SilentDrop { violation: None };
    }

    let legacy_mode = web_path.is_empty();

    // Localhost: legacy roots AND /{web_path}/... both always work, unconditionally —
    // the GUI WebView (which always connects via 127.0.0.1) must never be gated.
    if is_localhost {
        if method != "GET" {
            return RouteDecision::MethodNotAllowedLegacy;
        }
        if is_known_asset(path) {
            return RouteDecision::Serve(path.to_string());
        }
        if !legacy_mode {
            if let Some(inner) = strip_web_path_prefix(path, web_path) {
                if inner.is_empty() {
                    return RouteDecision::Redirect(format!("/{}/", web_path));
                }
                if is_known_asset(&inner) {
                    return RouteDecision::Serve(inner);
                }
            }
        }
        return RouteDecision::NotFoundLegacy;
    }

    // Non-localhost, legacy mode (web_path == "") — unchanged pre-stealth behavior.
    if legacy_mode {
        if method != "GET" {
            return RouteDecision::MethodNotAllowedLegacy;
        }
        if is_known_asset(path) {
            return RouteDecision::Serve(path.to_string());
        }
        return RouteDecision::NotFoundLegacy;
    }

    // Non-localhost, stealth mode.
    if method != "GET" {
        return RouteDecision::SilentDrop { violation: Some("method-not-allowed".to_string()) };
    }

    // Grace: an IP actually IN the allow list (not merely open mode) gets bounced from
    // the legacy root to /{web_path}/ instead of being dropped.
    if in_allow_list && (path == "/" || path == "/index.html") {
        return RouteDecision::Redirect(format!("/{}/", web_path));
    }

    if let Some(inner) = strip_web_path_prefix(path, web_path) {
        if inner.is_empty() {
            return RouteDecision::Redirect(format!("/{}/", web_path));
        }
        if is_known_asset(&inner) {
            return RouteDecision::Serve(inner);
        }
        return if is_favicon_exempt(&inner) {
            RouteDecision::SilentDrop { violation: None }
        } else {
            RouteDecision::SilentDrop { violation: Some(format!("stealth-404:{}", inner)) }
        };
    }

    // Doesn't start with /{web_path} at all — random probe (or a browser auto-request
    // for /favicon.ico / apple-touch-icon at the true root, which is exempt).
    if is_favicon_exempt(path) {
        RouteDecision::SilentDrop { violation: None }
    } else {
        RouteDecision::SilentDrop { violation: Some(format!("stealth-probe:{}", path)) }
    }
}

/// HTML-escape a string for safe interpolation into an HTML response body (not
/// attribute/URL-safe on its own — callers that also use the value as a URL/attribute,
/// like `build_redirect_response`, sanitize the value separately before it reaches here).
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Build a 301 redirect response with a `Location` header (e.g. `/{web_path}` →
/// `/{web_path}/`, or the allow-listed grace redirect from `/`).
/// `location` is reflected both in the `Location` header (raw — callers are
/// responsible for it being a well-formed URL, e.g. `redirect_http_to_https` sanitizes
/// the Host component before building it, C3 security remediation) and in the HTML
/// body (HTML-escaped here, since the header value alone is not attribute/HTML-safe).
fn build_redirect_response(location: &str, is_https: bool) -> Vec<u8> {
    let escaped = html_escape(location);
    let body = format!("Redirecting to <a href=\"{loc}\">{loc}</a>", loc = escaped);
    let mut headers = format!(
        "HTTP/1.1 301 Moved Permanently\r\n\
         Location: {}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n",
        location, body.len()
    );
    if is_https {
        headers.push_str("Strict-Transport-Security: max-age=31536000\r\n");
    }
    headers.push_str("\r\n");
    headers.push_str(&body);
    headers.into_bytes()
}

/// Handle HTTP route matching and response generation (shared by all server implementations).
/// `path` is the already-resolved inner path (query string stripped, `/{web_path}` prefix
/// already stripped by the caller via `decide_route`/`strip_web_path_prefix`).
/// WS_PORT is injected as 0 (sentinel) — the JS client uses window.location.port.
fn handle_http_routes(
    method: &str,
    path: &str,
    host: &str,
    ws_use_tls: bool,
    theme_css_vars: &str,
    is_https: bool,
    web_path: &str,
) -> Option<RouteResult> {
    if method != "GET" {
        return Some(RouteResult::MethodNotAllowed(
            build_http_response(405, "Method Not Allowed", "text/plain", "Method Not Allowed", is_https)
        ));
    }

    // Only allow hostname-valid characters (alphanumeric, dots, hyphens, colons for port)
    let sanitized_host: String = host.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == ':')
        .collect();
    let response = match path {
        "/" | "/index.html" => {
            let html = WEB_INDEX_HTML
                .replace("{{WEB_PATH}}", web_path)
                .replace("{{WS_HOST}}", &sanitized_host)
                .replace("{{WS_PORT}}", "0")
                .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" })
                .replace("{{WS_LOCAL_HOST}}", &sanitized_host)
                .replace("{{WS_REMOTE_HOST}}", "")
                .replace("{{CONNECTION_MODE}}", "auto")
                .replace("{{SHOW_CONNECTION_WINDOW}}", "false")
                .replace("{{THEME_CSS_VARS}}", theme_css_vars);
            RouteResult::Ok(build_http_response(200, "OK", "text/html", &html, is_https))
        }
        "/style.css" => {
            RouteResult::Ok(build_http_response(200, "OK", "text/css", WEB_STYLE_CSS, is_https))
        }
        "/app.js" => {
            RouteResult::Ok(build_http_response(200, "OK", "application/javascript", WEB_APP_JS, is_https))
        }
        "/theme-editor" => {
            let html = WEB_THEME_EDITOR_HTML
                .replace("{{WEB_PATH}}", web_path)
                .replace("{{WS_HOST}}", &sanitized_host)
                .replace("{{WS_PORT}}", "0")
                .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
            RouteResult::Ok(build_http_response(200, "OK", "text/html", &html, is_https))
        }
        "/keybind-editor" => {
            let html = WEB_KEYBIND_EDITOR_HTML
                .replace("{{WEB_PATH}}", web_path)
                .replace("{{WS_HOST}}", &sanitized_host)
                .replace("{{WS_PORT}}", "0")
                .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
            RouteResult::Ok(build_http_response(200, "OK", "text/html", &html, is_https))
        }
        "/action-editor" => {
            let html = WEB_ACTION_EDITOR_HTML
                .replace("{{WEB_PATH}}", web_path)
                .replace("{{WS_HOST}}", &sanitized_host)
                .replace("{{WS_PORT}}", "0")
                .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
            RouteResult::Ok(build_http_response(200, "OK", "text/html", &html, is_https))
        }
        "/fonts/jetbrains-mono-latin-400.woff2" => {
            RouteResult::Ok(build_binary_http_response(200, "OK", "font/woff2", FONT_JETBRAINS_MONO, is_https))
        }
        "/fonts/nunito-latin-400.woff2" => {
            RouteResult::Ok(build_binary_http_response(200, "OK", "font/woff2", FONT_NUNITO, is_https))
        }
        "/favicon.ico" => {
            RouteResult::Ok(build_http_response(204, "No Content", "image/x-icon", "", is_https))
        }
        _ => {
            RouteResult::NotFound(
                build_http_response(404, "Not Found", "text/plain", "Not Found", is_https),
                path.to_string(),
            )
        }
    };
    Some(response)
}

/// Result of `gate_connection`. `knocked` is `true` when the connection presented a
/// valid CLAY-KNOCK v1 preamble (D4, SECURITY-ROADMAP.md).
#[derive(Debug, Clone, Copy, PartialEq)]
enum GateResult {
    Proceed { knocked: bool, in_allow_list: bool },
    Drop,
}

// ============================================================================
// CLAY-KNOCK v1 (D4, SECURITY-ROADMAP.md) — binary in-band auth-key preamble
// ============================================================================
//
// Wire protocol, all on the same raw TCP socket, before TLS/HTTP:
//   1. C->S HELLO     (6 bytes):  C7 4C 41 59 01 00   (magic + version + reserved)
//   2. S->C CHALLENGE (34 bytes): C7 4B + 32 random bytes
//   3. C->S RESPONSE  (32 bytes): raw SHA256(auth_key_utf8_bytes || challenge_bytes)
//   4. S->C ACK       (2 bytes):  C7 06   (only on success)
// The leading 0xC7 is not `0x16` (TLS ClientHello), not ASCII (no HTTP method
// collision), and an invalid UTF-8 lead byte, so a 1-byte peek disambiguates all three
// protocols cleanly.

/// C->S HELLO magic: `0xC7` + ASCII "LAY".
const KNOCK_MAGIC: [u8; 4] = [0xC7, 0x4C, 0x41, 0x59];
/// C->S HELLO protocol version.
const KNOCK_VERSION: u8 = 0x01;
/// S->C CHALLENGE magic prefix (followed by 32 random bytes).
const KNOCK_CHALLENGE_MAGIC: [u8; 2] = [0xC7, 0x4B];
/// S->C ACK on successful knock.
const KNOCK_ACK: [u8; 2] = [0xC7, 0x06];

/// Validate a 6-byte C->S HELLO: magic + version match, reserved byte is zero.
fn parse_knock_hello(buf: &[u8; 6]) -> bool {
    buf[0..4] == KNOCK_MAGIC && buf[4] == KNOCK_VERSION && buf[5] == 0
}

/// Expected knock response: RAW `SHA256(key_utf8_bytes || challenge_bytes)`.
/// NOTE: this is a raw 32-byte digest, unlike `hash_with_challenge` (src/websocket.rs),
/// which hex-encodes its inputs and output for the WS auth-key challenge-response. The
/// knock protocol is binary throughout (D4) — do not confuse the two conventions.
fn knock_expected_response(key: &str, challenge: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hasher.update(challenge);
    hasher.finalize().into()
}

/// Outcome of `run_knock_handshake`.
enum KnockOutcome {
    Success,
    Fail,
}

/// Run the CLAY-KNOCK v1 handshake (D4) on a connection whose first peeked byte was
/// `0xC7`. Consumes exactly the knock bytes via `read_exact`/`write_all` — on return the
/// stream is positioned exactly at the next protocol byte (TLS ClientHello or HTTP
/// request), whether the knock succeeded or not (failure still only consumes up to the
/// bytes defined by the protocol; there is no further read after a failed digest).
/// Every read/write is bounded by `READ_TIMEOUT_SECS`.
async fn run_knock_handshake(
    stream: &mut tokio::net::TcpStream,
    client_ip: &str,
    gate: &SecurityGate,
) -> KnockOutcome {
    let timeout_dur = std::time::Duration::from_secs(READ_TIMEOUT_SECS);

    let mut hello = [0u8; 6];
    match tokio::time::timeout(timeout_dur, stream.read_exact(&mut hello)).await {
        Ok(Ok(_)) => {}
        _ => {
            log_remote_event("KNOCK-FAIL", client_ip, "hello read timeout/eof");
            gate.strike(client_ip, "knock-hello-timeout").await;
            return KnockOutcome::Fail;
        }
    }
    if !parse_knock_hello(&hello) {
        log_remote_event("KNOCK-BAD-MAGIC", client_ip, "bad hello bytes");
        gate.strike(client_ip, "knock-bad-magic").await;
        return KnockOutcome::Fail;
    }

    let mut challenge = [0u8; 32];
    if getrandom::getrandom(&mut challenge).is_err() {
        log_remote_event("KNOCK-FAIL", client_ip, "rng failure");
        gate.strike(client_ip, "knock-rng-failure").await;
        return KnockOutcome::Fail;
    }
    let mut challenge_msg = [0u8; 34];
    challenge_msg[0..2].copy_from_slice(&KNOCK_CHALLENGE_MAGIC);
    challenge_msg[2..34].copy_from_slice(&challenge);
    if tokio::time::timeout(timeout_dur, stream.write_all(&challenge_msg)).await.is_err() {
        log_remote_event("KNOCK-FAIL", client_ip, "challenge write timeout");
        gate.strike(client_ip, "knock-write-timeout").await;
        return KnockOutcome::Fail;
    }

    let mut response = [0u8; 32];
    match tokio::time::timeout(timeout_dur, stream.read_exact(&mut response)).await {
        Ok(Ok(_)) => {}
        _ => {
            log_remote_event("KNOCK-FAIL", client_ip, "response read timeout/eof");
            gate.strike(client_ip, "knock-response-timeout").await;
            return KnockOutcome::Fail;
        }
    }

    // No stored key (unset, or multiuser mode where the Arc is always None) => every
    // knock fails. Documented limitation (D4): multiuser hosts cannot be reached by a
    // non-allow-listed IP at all.
    let key = gate.auth_key.read().unwrap().clone();
    let key = match key {
        Some(k) if !k.is_empty() => k,
        _ => {
            log_remote_event("KNOCK-FAIL", client_ip, "no auth key configured");
            gate.strike(client_ip, "knock-no-key").await;
            return KnockOutcome::Fail;
        }
    };

    let expected = knock_expected_response(&key, &challenge);
    if !crate::util::constant_time_eq(&response, &expected) {
        log_remote_event("KNOCK-FAIL", client_ip, "digest mismatch");
        gate.strike(client_ip, "knock-bad-digest").await;
        return KnockOutcome::Fail;
    }

    if tokio::time::timeout(timeout_dur, stream.write_all(&KNOCK_ACK)).await.is_err() {
        log_remote_event("KNOCK-FAIL", client_ip, "ack write timeout");
        gate.strike(client_ip, "knock-ack-write-timeout").await;
        return KnockOutcome::Fail;
    }

    log_remote_event("KNOCK-OK", client_ip, "knock succeeded");
    gate.ban_list.clear_violations(client_ip);
    KnockOutcome::Success
}

/// Pure decision for the accept-time IP gate (D3, SECURITY-ROADMAP.md), split out from
/// `gate_connection` purely so the branching logic is unit-testable without async I/O
/// (allow-list lock reads, reverse-DNS). Localhost and an empty allow list both mean
/// "let it through" but do NOT count as `in_allow_list` — `in_allow_list` only reflects
/// actual list/whitelist membership, which matters downstream (e.g. the D2 grace
/// redirect only fires for genuinely allow-listed IPs, not merely open-mode ones).
#[derive(Debug, Clone, Copy, PartialEq)]
enum GateDecision {
    Proceed { in_allow_list: bool },
    Drop,
}

fn decide_gate(is_localhost: bool, allow_list_empty: bool, matched: bool) -> GateDecision {
    if is_localhost || allow_list_empty {
        GateDecision::Proceed { in_allow_list: false }
    } else if matched {
        GateDecision::Proceed { in_allow_list: true }
    } else {
        GateDecision::Drop
    }
}

/// Whether `client_ip` matches the gate's allow list, folding in the runtime
/// `whitelisted_host` (an IP auto-authed from a previous successful WS login).
fn gate_ip_matches(
    client_ip: &str,
    whitelisted_host: Option<&str>,
    allow_list: &[String],
    hostname: Option<&str>,
) -> bool {
    if whitelisted_host == Some(client_ip) {
        return true;
    }
    crate::websocket::is_in_allow_list(client_ip, hostname, allow_list)
}

/// Accept-time security gate (D3 + D4). Peeks the first byte (10s timeout) to detect a
/// CLAY-KNOCK v1 preamble (`0xC7`, D4) BEFORE any allow-list logic; a successful knock
/// admits the connection regardless of allow-list membership (the client can't know its
/// own list status — D4). Otherwise: localhost always proceeds. An empty allow list is
/// open mode: proceed, `in_allow_list=false`. Otherwise proceed only if the IP is listed
/// or matches `whitelisted_host`; non-matching IPs are dropped with zero bytes sent —
/// this MUST run before any TLS peek/handshake or plain-HTTP redirect in every accept
/// loop. Per D3, a plain (non-knock) gate drop logs `GATE-DROP` and records NO ban
/// violation: banning here would also block the IP's one legitimate entry path (a
/// knock). Reverse-DNS is only performed when the allow list contains a hostname
/// pattern.
///
/// Takes a concrete `TcpStream` (not the generic `S: AsyncRead + AsyncWrite` used
/// elsewhere in this module) because the knock preamble detection needs `TcpStream`'s
/// non-consuming `peek()`, which isn't part of the generic `AsyncRead` trait. All three
/// accept loops call this before any TLS layering, so they always hold a raw
/// `TcpStream` at this point — peeking here doesn't disturb the byte the accept loops'
/// own later TLS-vs-plain-HTTP peek (or `route_connection`'s first read) still needs to
/// see, since `peek()` never consumes.
async fn gate_connection(
    stream: &mut tokio::net::TcpStream,
    client_ip: &str,
    gate: &SecurityGate,
) -> GateResult {
    let is_localhost = client_ip
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false);

    // D4: peek 1 byte to detect a knock preamble. Non-consuming, bounded by
    // READ_TIMEOUT_SECS. EOF/timeout => drop with a single GATE-TIMEOUT log line (no
    // violation — plenty of scanners connect-and-hang; not worth a strike).
    let mut peek_buf = [0u8; 1];
    let is_knock_attempt = match tokio::time::timeout(
        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
        stream.peek(&mut peek_buf),
    ).await {
        Ok(Ok(n)) if n > 0 => peek_buf[0] == 0xC7,
        _ => {
            log_remote_event("GATE-TIMEOUT", client_ip, "no bytes before first-byte peek timeout");
            return GateResult::Drop;
        }
    };

    let mut knocked = false;
    if is_knock_attempt {
        match run_knock_handshake(stream, client_ip, gate).await {
            KnockOutcome::Success => knocked = true,
            KnockOutcome::Fail => return GateResult::Drop,
        }
    }

    let allow_list = gate.allow_list.read().unwrap().clone();
    let allow_list_empty = allow_list.is_empty();

    let matched = if is_localhost || allow_list_empty {
        false // unused by decide_gate on this branch
    } else {
        let whitelisted_host = gate.whitelisted_host.read().unwrap().clone();
        let has_hostname_patterns = allow_list.iter().any(|p| crate::websocket::is_hostname_pattern(p));
        let hostname = if has_hostname_patterns {
            crate::websocket::reverse_dns_lookup(client_ip).await
        } else {
            None
        };
        gate_ip_matches(client_ip, whitelisted_host.as_deref(), &allow_list, hostname.as_deref())
    };

    if knocked {
        // A proven knock bypasses the allow-list Drop outcome entirely (D4: "Knocks are
        // accepted from ANY non-banned IP including allow-listed ones") — but
        // `in_allow_list` is still whatever `matched` says, since D2's grace-redirect
        // and WS-path rules key off genuine list membership downstream.
        return GateResult::Proceed { knocked: true, in_allow_list: matched };
    }

    match decide_gate(is_localhost, allow_list_empty, matched) {
        GateDecision::Proceed { in_allow_list } => GateResult::Proceed { knocked: false, in_allow_list },
        GateDecision::Drop => {
            log_remote_event("GATE-DROP", client_ip, "not in allow list");
            GateResult::Drop
        }
    }
}

/// Route an incoming connection: detect WebSocket upgrades vs HTTP requests.
/// If ws_state is provided and the request is a WebSocket upgrade, hands off to the WS handler.
/// Otherwise, serves static HTTP content.
#[allow(clippy::too_many_arguments)]
async fn route_connection<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    mut stream: S,
    ws_state: Option<Arc<WsConnectionState>>,
    ws_use_tls: bool,
    theme_css_vars: &str,
    client_addr: std::net::SocketAddr,
    is_https: bool,
    http_guard: ConnectionGuard,
    ws_counter: &ConnectionCounter,
    gate: &SecurityGate,
    knocked: bool,
    // Computed by `gate_connection` at accept time (D3) and threaded through here —
    // true only for an IP genuinely in the allow list or matching whitelisted_host
    // (never true merely because the allow list is empty/open-mode).
    in_allow_list: bool,
) {
    let client_ip = client_addr.ip().to_string();
    let is_localhost = client_addr.ip().is_loopback();

    let mut buf = [0u8; 4096];
    let n = match tokio::time::timeout(
        std::time::Duration::from_secs(READ_TIMEOUT_SECS),
        stream.read(&mut buf),
    ).await {
        Ok(Ok(n)) if n > 0 => n,
        _ => return,
    };
    // Buffer completely filled — likely oversized/malicious payload
    if n == buf.len() {
        gate.strike(&client_ip, "oversized-request").await;
        return;
    }

    // TLS ClientHello arriving at a plain-HTTP server (D6/decision 3, SECURITY-ROADMAP.md):
    // browsers send these on their own (HSTS cache from an earlier web_secure=true run,
    // HTTPS-First mode) — log it, never strike. Checked on the raw first byte, BEFORE any
    // HTTP-shaped parsing of the bytes below: a real ClientHello's cipher-suite/extension
    // bytes are effectively random and can spuriously contain a space or newline byte,
    // which would make `parse_http_request` "succeed" with garbage method/path instead of
    // returning None — bypassing a check placed only in the parse-failure branch. The
    // first-byte peek is the same disambiguator the TLS accept loops already use (0x16 =
    // TLS, 0xC7 = knock [handled earlier in gate_connection], else = HTTP), so checking it
    // unconditionally here is both correct and reliable regardless of what follows it.
    if buf[0] == 0x16 {
        log_remote_event("TLS-ON-PLAIN", &client_ip, "TLS ClientHello received on a plain HTTP server");
        return;
    }

    let request = String::from_utf8_lossy(&buf[..n]);

    // Check for WebSocket upgrade
    if is_websocket_upgrade(&request) {
        let ws_path = parse_http_request(&request)
            .map(|(_, p)| p.split('?').next().unwrap_or(p))
            .unwrap_or("/");
        // D2: accept at ANY path if knocked, localhost, or allow-listed/whitelisted
        // (keeps old Android APKs working for allow-listed users). Open mode,
        // non-localhost: path must be exactly /{web_path}/ws (any path if web_path
        // empty — legacy mode). Wrong path => silent drop, NO violation (updated
        // Android clients probe /clay/ws then /ws; striking would ban legit devices).
        let path_ok = knocked
            || is_localhost
            || in_allow_list
            || gate.web_path.is_empty()
            || ws_path == format!("/{}/ws", gate.web_path);
        if !path_ok {
            log_remote_event("WS-PATH-DROP", &client_ip, ws_path);
            return;
        }
        if let Some(ws_state) = ws_state {
            // Acquire a WebSocket slot before starting the session
            let ws_guard = match ws_counter.try_acquire(&client_ip, MAX_WS_CONNECTIONS_PER_IP) {
                Some(g) => g,
                None => {
                    log_remote_event("WS-LIMIT", &client_ip, "websocket connection limit reached");
                    let response = build_http_response(503, "Service Unavailable", "text/plain", "Too many connections", is_https);
                    let _ = stream.write_all(&response).await;
                    return;
                }
            };
            // Release the HTTP slot — the WS session is tracked separately
            drop(http_guard);
            let client_id = ws_state.next_client_id();
            let pw_hash = ws_state.password_hash.read().unwrap().clone();
            let pw_enabled = *ws_state.password_enabled.read().unwrap();
            let prefixed = PrefixedStream::new(buf[..n].to_vec(), stream);
            let _ws_guard = ws_guard;
            let _ = crate::websocket::handle_ws_client(
                prefixed,
                client_id,
                ws_state.clients.clone(),
                pw_hash,
                pw_enabled,
                ws_state.allow_list.clone(),
                ws_state.whitelisted_host.clone(),
                client_addr,
                ws_state.event_tx.clone(),
                ws_state.multiuser_mode,
                ws_state.users.clone(),
                ws_state.ban_list.clone(),
                knocked,
            ).await;
        } else {
            // WebSocket not configured (no password set)
            let response = build_http_response(503, "Service Unavailable", "text/plain", "WebSocket not configured", is_https);
            let _ = stream.write_all(&response).await;
        }
        return;
    }

    let (method, full_path) = match parse_http_request(&request) {
        Some(v) => v,
        None => {
            // Closes the unparseable-request hole: a bare `None => return` here was
            // silent and un-logged either way, so genuinely malformed input was free and
            // invisible in remote.log. The TLS-ClientHello-on-plain-server case is
            // already handled above (before parsing), so anything reaching here is
            // neither a knock nor a TLS handshake — genuine garbage.
            log_remote_event("HTTP-DROP", &client_ip, "unparseable-request");
            gate.strike(&client_ip, "unparseable-request").await;
            return;
        }
    };
    let path = full_path.split('?').next().unwrap_or(full_path);
    let host = get_host_from_request(&request);

    let (route_method, route_path) = match decide_route(method, path, &gate.web_path, is_localhost, in_allow_list, knocked) {
        RouteDecision::SilentDrop { violation } => {
            if let Some(reason) = violation {
                gate.strike(&client_ip, &reason).await;
            }
            // D4: a knocked connection hitting a static route (violation is always
            // None for this case — see decide_route) gets its own log event, no
            // violation — the key was already proven at the knock.
            if knocked {
                log_remote_event("KNOCK-HTTP-DENIED", &client_ip, path);
            } else {
                log_remote_event("HTTP-DROP", &client_ip, path);
            }
            return;
        }
        RouteDecision::Redirect(location) => {
            let response = build_redirect_response(&location, is_https);
            let _ = stream.write_all(&response).await;
            return;
        }
        RouteDecision::Serve(inner_path) => (method, inner_path),
        RouteDecision::NotFoundLegacy | RouteDecision::MethodNotAllowedLegacy => (method, path.to_string()),
    };

    // Normal HTTP request
    if let Some(route_result) = handle_http_routes(route_method, &route_path, &host, ws_use_tls, theme_css_vars, is_https, &gate.web_path) {
        let response = match route_result {
            RouteResult::Ok(r) => r,
            RouteResult::MethodNotAllowed(r) => {
                let _ = stream.write_all(&r).await;
                return;
            }
            RouteResult::NotFound(r, path) => {
                log_http_404(&client_ip, &path);
                gate.strike(&client_ip, &path).await;
                r
            }
        };

        if stream.write_all(&response).await.is_ok() {
            let _ = stream.shutdown().await;
        }
    }
}

/// Shared WebSocket connection state passed from WebSocketServer to the unified HTTP+WS server.
pub struct WsConnectionState {
    pub clients: Arc<RwLock<HashMap<u64, crate::websocket::WsClientInfo>>>,
    pub next_client_id: Arc<std::sync::Mutex<u64>>,
    pub password_hash: Arc<std::sync::RwLock<String>>,
    pub password_enabled: Arc<std::sync::RwLock<bool>>,
    pub allow_list: Arc<std::sync::RwLock<Vec<String>>>,
    pub whitelisted_host: Arc<std::sync::RwLock<Option<String>>>,
    pub event_tx: tokio::sync::mpsc::Sender<crate::AppEvent>,
    pub multiuser_mode: bool,
    pub users: Arc<std::sync::RwLock<HashMap<String, crate::websocket::UserCredential>>>,
    pub ban_list: BanList,
}

impl WsConnectionState {
    pub fn next_client_id(&self) -> u64 {
        let mut id = self.next_client_id.lock().unwrap();
        let current = *id;
        *id += 1;
        current
    }
}

/// Shared accept-time security state: allow-list IP gate (D3) + knock protocol (D4,
/// Phase 3). Exists even when no WebSocketServer is running (ws_state is None) so the
/// gate is always available at accept time. `allow_list`/`whitelisted_host` share the
/// SAME Arc as WebSocketServer when one exists (so runtime allow-list edits and
/// whitelisted-host auth are visible immediately); when no WebSocketServer exists yet,
/// callers construct fresh Arcs seeded from settings. `auth_key` mirrors
/// `App.ws_auth_key_shared`. `web_path` is fixed at server start (server restarts on
/// change) — sourced from `Settings.web_path` (sanitized, default `"clay"`; empty =
/// legacy mode serving the UI at `/`).
#[derive(Clone)]
pub struct SecurityGate {
    pub allow_list: Arc<std::sync::RwLock<Vec<String>>>,
    pub whitelisted_host: Arc<std::sync::RwLock<Option<String>>>,
    pub auth_key: Arc<std::sync::RwLock<Option<String>>>,
    pub web_path: String,
    pub ban_list: BanList,
}

/// Pure decision for `SecurityGate::is_ban_exempt`, split out for testability without
/// reverse-DNS I/O — mirrors the `decide_gate`/`gate_ip_matches` split above. D6
/// (SECURITY-ROADMAP.md): allow-listed IPs are never banned for probes, because once an
/// allow list is configured the accept-time gate (D3) already drops every non-listed IP
/// before it can reach any strike site — a scanner can never trigger one, so the only
/// IPs a probe-strike can possibly ban are legitimate, allow-listed ones. A bare `*`
/// allow-list entry is deliberately excluded: an allow list of `*` means "let everyone
/// reach the UI", not "nobody on the internet can ever be banned". `whitelisted_host` is
/// checked directly (it is not itself an allow-list entry).
fn decide_ban_exempt(
    ip: &str,
    whitelisted_host: Option<&str>,
    allow_list: &[String],
    hostname: Option<&str>,
) -> bool {
    if ip == "127.0.0.1" || ip == "::1" || ip == "localhost" {
        return true;
    }
    if whitelisted_host == Some(ip) {
        return true;
    }
    let specific: Vec<String> = allow_list.iter().filter(|p| p.trim() != "*").cloned().collect();
    if specific.is_empty() {
        return false;
    }
    crate::websocket::is_in_allow_list(ip, hostname, &specific)
}

impl SecurityGate {
    /// Record a probe/connection violation, unless the IP is exempt (D6). This is the
    /// one chokepoint every probe-strike site in this module should call instead of
    /// `ban_list.record_violation` directly. Returns true if the violation caused (or
    /// the IP already was) a ban.
    pub async fn strike(&self, ip: &str, reason: &str) -> bool {
        if self.is_ban_exempt(ip).await {
            return false;
        }
        self.ban_list.record_violation(ip, reason)
    }

    /// Localhost, or an IP matching a *specific* allow-list entry (exact IP, IP
    /// wildcard, or hostname pattern) — see `decide_ban_exempt` for the bare-`*`
    /// exclusion rationale. Reverse-DNS is only performed when the allow list actually
    /// contains a hostname pattern, same as `gate_connection`.
    async fn is_ban_exempt(&self, ip: &str) -> bool {
        let whitelisted_host = self.whitelisted_host.read().unwrap().clone();
        let allow_list = self.allow_list.read().unwrap().clone();
        let has_hostname_patterns = allow_list.iter()
            .filter(|p| p.trim() != "*")
            .any(|p| crate::websocket::is_hostname_pattern(p));
        let hostname = if has_hostname_patterns {
            crate::websocket::reverse_dns_lookup(ip).await
        } else {
            None
        };
        decide_ban_exempt(ip, whitelisted_host.as_deref(), &allow_list, hostname.as_deref())
    }
}

// ============================================================================
// HTTPS Web Interface Server
// ============================================================================

/// HTTPS server state for the web interface
#[cfg(feature = "native-tls-backend")]
pub struct HttpsServer {
    pub running: Arc<RwLock<bool>>,
    pub shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub port: u16,
}

#[cfg(feature = "native-tls-backend")]
impl HttpsServer {
    pub fn new(port: u16) -> Self {
        Self {
            running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
            port,
        }
    }
}

/// Start the HTTPS server (unified HTTP+WS)
#[cfg(feature = "native-tls-backend")]
pub async fn start_https_server(
    server: &mut HttpsServer,
    cert_file: &str,
    key_file: &str,
    ws_state: Option<Arc<WsConnectionState>>,
    ban_list: BanList,
    theme_css_vars: String,
    gate: SecurityGate,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::fs::File;
    use std::io::Read;

    // Read certificate and key
    let mut cert_data = Vec::new();
    File::open(cert_file)?.read_to_end(&mut cert_data)?;

    let mut key_data = Vec::new();
    File::open(key_file)?.read_to_end(&mut key_data)?;

    // Create identity from PEM files (same as WebSocket TLS)
    let identity = native_tls::Identity::from_pkcs8(&cert_data, &key_data)?;
    let tls_acceptor = native_tls::TlsAcceptor::new(identity)?;
    let tls_acceptor = tokio_native_tls::TlsAcceptor::from(tls_acceptor);
    let tls_acceptor = Arc::new(tls_acceptor);

    let addr = format!("0.0.0.0:{}", server.port);
    // Retry binding with delays — on reload, the previous process may still be releasing the port
    let listener = {
        let mut last_err = None;
        let mut bound = None;
        for attempt in 0..10 {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => { bound = Some(l); break; }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 9 {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                }
            }
        }
        match bound {
            Some(l) => l,
            None => return Err(format!("Failed to bind HTTPS to port {}: {}", server.port, last_err.unwrap()).into()),
        }
    };

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let server_port = server.port;
    let running = Arc::clone(&server.running);
    *running.write().await = true;

    let theme_css_vars = Arc::new(theme_css_vars);
    let conn_counter = ConnectionCounter::new();
    let ws_counter = ConnectionCounter::new();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((mut stream, addr)) => {
                            let client_ip = addr.ip().to_string();
                            if ban_list.is_banned(&client_ip) {
                                continue;
                            }
                            let guard = match conn_counter.try_acquire(&client_ip, MAX_HTTP_CONNECTIONS_PER_IP) {
                                Some(g) => g,
                                None => {
                                    log_remote_event("CONN-LIMIT", &client_ip, "connection limit reached");
                                    continue;
                                }
                            };
                            let _ = stream.set_nodelay(true);
                            let tls_acceptor = tls_acceptor.clone();
                            let theme_css_vars = theme_css_vars.clone();
                            let ws_state = ws_state.clone();
                            let ws_counter = ws_counter.clone();
                            let gate = gate.clone();
                            tokio::spawn(async move {
                                match gate_connection(&mut stream, &client_ip, &gate).await {
                                    GateResult::Drop => {},
                                    GateResult::Proceed { knocked, in_allow_list } => {
                                        // Peek first byte to detect plain HTTP vs TLS
                                        let mut peek = [0u8; 1];
                                        match tokio::time::timeout(
                                            std::time::Duration::from_secs(READ_TIMEOUT_SECS),
                                            stream.peek(&mut peek),
                                        ).await {
                                            Ok(Ok(1)) if peek[0] != 0x16 => {
                                                // Not a TLS ClientHello — redirect HTTP to HTTPS
                                                // (D2/D6: reuses decide_route via gate/in_allow_list,
                                                // see redirect_http_to_https doc comment)
                                                redirect_http_to_https(stream, server_port, addr.ip().is_loopback(), in_allow_list, knocked, &gate, &client_ip).await;
                                                return;
                                            }
                                            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
                                            _ => {}
                                        }
                                        match tokio::time::timeout(
                                            std::time::Duration::from_secs(TLS_TIMEOUT_SECS),
                                            tls_acceptor.accept(stream),
                                        ).await {
                                            Ok(Ok(tls_stream)) => {
                                                route_connection(tls_stream, ws_state, true, &theme_css_vars, addr, true, guard, &ws_counter, &gate, knocked, in_allow_list).await;
                                            }
                                            Ok(Err(e)) => {
                                                log_remote_event("TLS-ERROR", &client_ip, &format!("{}", e));
                                            }
                                            Err(_) => {
                                                log_remote_event("TLS-TIMEOUT", &client_ip, "handshake timeout");
                                                gate.strike(&client_ip, "tls-timeout").await;
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        Err(_) => break,
                    }
                }
                _ = &mut shutdown_rx => {
                    break;
                }
            }
        }
        *running.write().await = false;
    });

    Ok(())
}

// ============================================================================
// HTTPS Server implementation for rustls-backend
// ============================================================================

/// HTTPS server state for the web interface (rustls version)
#[cfg(feature = "rustls-backend")]
pub struct HttpsServer {
    pub running: Arc<RwLock<bool>>,
    pub shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub port: u16,
}

#[cfg(feature = "rustls-backend")]
impl HttpsServer {
    pub fn new(port: u16) -> Self {
        Self {
            running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
            port,
        }
    }
}

/// Start the HTTPS server (rustls version, unified HTTP+WS)
#[cfg(feature = "rustls-backend")]
pub async fn start_https_server(
    server: &mut HttpsServer,
    cert_file: &str,
    key_file: &str,
    ws_state: Option<Arc<WsConnectionState>>,
    ban_list: BanList,
    theme_css_vars: String,
    gate: SecurityGate,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::fs::File;
    use std::io::BufReader;
    use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    // Read certificate chain
    let cert_file_handle = File::open(cert_file)
        .map_err(|e| format!("Failed to open cert file '{}': {}", cert_file, e))?;
    let mut cert_reader = BufReader::new(cert_file_handle);
    let certs: Vec<CertificateDer<'static>> = certs(&mut cert_reader)
        .map_err(|e| format!("Failed to parse cert file '{}': {}", cert_file, e))?
        .into_iter()
        .map(CertificateDer::from)
        .collect();

    if certs.is_empty() {
        return Err(format!("No certificates found in cert file '{}'", cert_file).into());
    }

    // Read private key - try PKCS8 first, then RSA
    let key_file_handle = File::open(key_file)
        .map_err(|e| format!("Failed to open key file '{}': {}", key_file, e))?;
    let mut key_reader = BufReader::new(key_file_handle);
    let keys = pkcs8_private_keys(&mut key_reader)
        .map_err(|e| format!("Failed to parse key file '{}': {}", key_file, e))?;
    let key: PrivateKeyDer<'static> = if !keys.is_empty() {
        PrivateKeyDer::Pkcs8(keys.into_iter().next().unwrap().into())
    } else {
        // Try RSA format
        let key_file_handle = File::open(key_file)
            .map_err(|e| format!("Failed to open key file '{}': {}", key_file, e))?;
        let mut key_reader = BufReader::new(key_file_handle);
        let keys = rsa_private_keys(&mut key_reader)
            .map_err(|e| format!("Failed to parse key file '{}': {}", key_file, e))?;
        if keys.is_empty() {
            return Err(format!("No private key found in key file '{}'", key_file).into());
        }
        PrivateKeyDer::Pkcs1(keys.into_iter().next().unwrap().into())
    };

    // Build TLS config
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("Failed to build TLS config: {}", e))?;

    let tls_acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

    let addr = format!("0.0.0.0:{}", server.port);
    // Retry binding with delays — on reload, the previous process may still be releasing the port
    let listener = {
        let mut last_err = None;
        let mut bound = None;
        for attempt in 0..10 {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => { bound = Some(l); break; }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 9 {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                }
            }
        }
        match bound {
            Some(l) => l,
            None => return Err(format!("Failed to bind HTTPS to port {}: {}", server.port, last_err.unwrap()).into()),
        }
    };

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let server_port = server.port;
    let running = Arc::clone(&server.running);
    *running.write().await = true;

    let theme_css_vars = Arc::new(theme_css_vars);
    let conn_counter = ConnectionCounter::new();
    let ws_counter = ConnectionCounter::new();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((mut stream, addr)) => {
                            let client_ip = addr.ip().to_string();
                            if ban_list.is_banned(&client_ip) {
                                continue;
                            }
                            let guard = match conn_counter.try_acquire(&client_ip, MAX_HTTP_CONNECTIONS_PER_IP) {
                                Some(g) => g,
                                None => {
                                    log_remote_event("CONN-LIMIT", &client_ip, "connection limit reached");
                                    continue;
                                }
                            };
                            let _ = stream.set_nodelay(true);
                            let tls_acceptor = tls_acceptor.clone();
                            let theme_css_vars = theme_css_vars.clone();
                            let ws_state = ws_state.clone();
                            let ws_counter = ws_counter.clone();
                            let gate = gate.clone();
                            tokio::spawn(async move {
                                match gate_connection(&mut stream, &client_ip, &gate).await {
                                    GateResult::Drop => {},
                                    GateResult::Proceed { knocked, in_allow_list } => {
                                        // Peek first byte to detect plain HTTP vs TLS
                                        let mut peek = [0u8; 1];
                                        match tokio::time::timeout(
                                            std::time::Duration::from_secs(READ_TIMEOUT_SECS),
                                            stream.peek(&mut peek),
                                        ).await {
                                            Ok(Ok(1)) if peek[0] != 0x16 => {
                                                // Not a TLS ClientHello — redirect HTTP to HTTPS
                                                // (D2/D6: reuses decide_route via gate/in_allow_list,
                                                // see redirect_http_to_https doc comment)
                                                redirect_http_to_https(stream, server_port, addr.ip().is_loopback(), in_allow_list, knocked, &gate, &client_ip).await;
                                                return;
                                            }
                                            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
                                            _ => {}
                                        }
                                        match tokio::time::timeout(
                                            std::time::Duration::from_secs(TLS_TIMEOUT_SECS),
                                            tls_acceptor.accept(stream),
                                        ).await {
                                            Ok(Ok(tls_stream)) => {
                                                route_connection(tls_stream, ws_state, true, &theme_css_vars, addr, true, guard, &ws_counter, &gate, knocked, in_allow_list).await;
                                            }
                                            Ok(Err(e)) => {
                                                log_remote_event("TLS-ERROR", &client_ip, &format!("{}", e));
                                            }
                                            Err(_) => {
                                                log_remote_event("TLS-TIMEOUT", &client_ip, "handshake timeout");
                                                gate.strike(&client_ip, "tls-timeout").await;
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        Err(_) => break,
                    }
                }
                _ = &mut shutdown_rx => {
                    break;
                }
            }
        }
        *running.write().await = false;
    });

    Ok(())
}

// ============================================================================
// Ban List for HTTP/WebSocket security
// ============================================================================

/// Tracks violations and bans for IP addresses.
/// Bans are in-memory only and last until server restart.
#[derive(Clone)]
pub struct BanList {
    /// Banned IPs — in-memory only, cleared on restart
    banned: Arc<std::sync::RwLock<HashSet<String>>>,
    /// Violation count per IP (reset when banned)
    violations: Arc<std::sync::RwLock<HashMap<String, u32>>>,
    /// Last URL/reason that triggered the ban
    ban_reasons: Arc<std::sync::RwLock<HashMap<String, String>>>,
}

impl BanList {
    pub fn new() -> Self {
        Self {
            banned: Arc::new(std::sync::RwLock::new(HashSet::new())),
            violations: Arc::new(std::sync::RwLock::new(HashMap::new())),
            ban_reasons: Arc::new(std::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Check if an IP is currently banned
    pub fn is_banned(&self, ip: &str) -> bool {
        self.banned.read().unwrap().contains(ip)
    }

    /// Record a violation for an IP. Bans after 2 violations. Returns true if banned.
    pub fn record_violation(&self, ip: &str, reason: &str) -> bool {
        self.record_with_threshold(ip, reason, 2)
    }

    /// Record a failed WebSocket password-auth attempt. Bans after 5 attempts instead
    /// of 2 (D6, SECURITY-ROADMAP.md) — password auth is the one strike on this path
    /// that actually protects something, so it earns room for a typo, and unlike probe
    /// strikes it applies to allow-listed IPs too (no allow-list exemption; a listed
    /// device brute-forcing the password is still a threat). Localhost stays exempt,
    /// same as `record_violation`. Returns true if the attempt caused (or the IP
    /// already was) a ban.
    pub fn record_auth_failure(&self, ip: &str, reason: &str) -> bool {
        self.record_with_threshold(ip, reason, 5)
    }

    /// Shared bookkeeping for `record_violation`/`record_auth_failure`: never ban
    /// localhost, track a per-IP counter, ban once it reaches `threshold`.
    fn record_with_threshold(&self, ip: &str, reason: &str, threshold: u32) -> bool {
        // Never ban localhost
        if ip == "127.0.0.1" || ip == "::1" || ip == "localhost" {
            return false;
        }
        // Already banned — no need to track further
        if self.banned.read().unwrap().contains(ip) {
            return true;
        }

        self.ban_reasons.write().unwrap().insert(ip.to_string(), reason.to_string());

        let count = {
            let mut violations = self.violations.write().unwrap();
            let entry = violations.entry(ip.to_string()).or_insert(0);
            *entry += 1;
            *entry
        };

        if count >= threshold {
            self.banned.write().unwrap().insert(ip.to_string());
            self.violations.write().unwrap().remove(ip);
            log_ban(ip, "BANNED", reason);
            true
        } else {
            false
        }
    }

    /// Add a ban directly (e.g. from persistence load — no-op, bans are not persisted)
    pub fn add_permanent_ban(&self, _ip: &str) {
        // Bans are in-memory only and not loaded from disk
    }

    /// Get all banned IPs (no-op return — bans are not persisted)
    pub fn get_permanent_bans(&self) -> Vec<String> {
        Vec::new()
    }

    /// Clear violation history for an IP (called on successful auth)
    pub fn clear_violations(&self, ip: &str) {
        self.violations.write().unwrap().remove(ip);
    }

    /// Remove a ban for an IP. Returns true if a ban was removed.
    pub fn remove_ban(&self, ip: &str) -> bool {
        let removed = self.banned.write().unwrap().remove(ip);
        self.violations.write().unwrap().remove(ip);
        self.ban_reasons.write().unwrap().remove(ip);
        removed
    }

    /// Get all current bans with reasons. Returns Vec of (ip, ban_type, reason).
    pub fn get_ban_info(&self) -> Vec<(String, String, String)> {
        let reasons = self.ban_reasons.read().unwrap();
        self.banned.read().unwrap().iter().map(|ip| {
            let reason = reasons.get(ip).cloned().unwrap_or_default();
            (ip.clone(), "banned".to_string(), reason)
        }).collect()
    }
}

impl Default for BanList {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Connection Counter — per-IP connection limiter
// ============================================================================

/// Tracks active connections per IP with a per-IP limit.
/// Localhost connections (127.0.0.1, ::1) are never counted or limited.
#[derive(Clone, Default)]
pub struct ConnectionCounter {
    counts: Arc<std::sync::Mutex<HashMap<String, usize>>>,
}

impl ConnectionCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to acquire a connection slot for the given IP up to `limit`.
    /// Returns `None` if the limit is reached; otherwise returns a guard
    /// that releases the slot when dropped.
    pub fn try_acquire(&self, ip: &str, limit: usize) -> Option<ConnectionGuard> {
        if ip == "127.0.0.1" || ip == "::1" {
            return Some(ConnectionGuard { counter: None, ip: String::new() });
        }
        let mut counts = self.counts.lock().unwrap();
        let count = counts.entry(ip.to_string()).or_insert(0);
        if *count >= limit {
            return None;
        }
        *count += 1;
        Some(ConnectionGuard {
            counter: Some(self.counts.clone()),
            ip: ip.to_string(),
        })
    }
}

/// RAII guard — releases a connection slot on drop.
pub struct ConnectionGuard {
    counter: Option<Arc<std::sync::Mutex<HashMap<String, usize>>>>,
    ip: String,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if let Some(counts) = &self.counter {
            let mut counts = counts.lock().unwrap();
            if let Some(n) = counts.get_mut(&self.ip) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    counts.remove(&self.ip);
                }
            }
        }
    }
}

// ============================================================================
// HTTP Web Interface Server (no TLS)
// ============================================================================

/// HTTP server state for the web interface (no TLS)
pub struct HttpServer {
    pub running: Arc<RwLock<bool>>,
    pub shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub port: u16,
    /// Raw socket handle of the TCP listener (for passing to child process on reload)
    #[cfg(windows)]
    pub listener_handle: Option<u64>,
    #[cfg(not(windows))]
    pub listener_handle: Option<i32>,
}

impl HttpServer {
    pub fn new(port: u16) -> Self {
        Self {
            running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
            listener_handle: None,
            port,
        }
    }
}

/// Start the HTTP server (plain TCP, no TLS, unified HTTP+WS)
pub async fn start_http_server(
    server: &mut HttpServer,
    ws_state: Option<Arc<WsConnectionState>>,
    ban_list: BanList,
    theme_css_vars: String,
    inherited_handle: Option<u64>,
    gate: SecurityGate,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = if let Some(handle) = inherited_handle {
        // Reconstruct listener from inherited socket handle (reload on Windows)
        #[cfg(windows)]
        {
            use std::os::windows::io::FromRawSocket;
            let std_listener = unsafe { std::net::TcpListener::from_raw_socket(handle) };
            std_listener.set_nonblocking(true)?;
            tokio::net::TcpListener::from_std(std_listener)?
        }
        #[cfg(not(windows))]
        {
            let _ = handle;
            return Err("inherited_handle not supported on this platform".into());
        }
    } else {
        let addr = format!("0.0.0.0:{}", server.port);
        // Retry binding with delays — on reload, the previous process may still be releasing the port
        let mut last_err = None;
        let mut bound = None;
        for attempt in 0..10 {
            match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => { bound = Some(l); break; }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 9 {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                }
            }
        }
        match bound {
            Some(l) => l,
            None => return Err(format!("Failed to bind HTTP to port {}: {}", server.port, last_err.unwrap()).into()),
        }
    };

    // Store the listener handle for passing to child process on reload
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        server.listener_handle = Some(listener.as_raw_socket());
    }

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let running = Arc::clone(&server.running);
    *running.write().await = true;

    let theme_css_vars = Arc::new(theme_css_vars);
    let conn_counter = ConnectionCounter::new();
    let ws_counter = ConnectionCounter::new();
    tokio::spawn(async move {
        // Signal ready INSIDE the spawned task — ensures the accept loop is actually running
        crate::GUI_HTTP_READY.store(true, std::sync::atomic::Ordering::SeqCst);
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((mut stream, addr)) => {
                            let client_ip = addr.ip().to_string();
                            if ban_list.is_banned(&client_ip) {
                                drop(stream);
                                continue;
                            }
                            let guard = match conn_counter.try_acquire(&client_ip, MAX_HTTP_CONNECTIONS_PER_IP) {
                                Some(g) => g,
                                None => {
                                    log_remote_event("CONN-LIMIT", &client_ip, "connection limit reached");
                                    continue;
                                }
                            };
                            let _ = stream.set_nodelay(true);
                            let theme_css_vars = theme_css_vars.clone();
                            let ws_state = ws_state.clone();
                            let ws_counter = ws_counter.clone();
                            let gate = gate.clone();
                            tokio::spawn(async move {
                                match gate_connection(&mut stream, &client_ip, &gate).await {
                                    GateResult::Drop => {},
                                    GateResult::Proceed { knocked, in_allow_list } => {
                                        route_connection(stream, ws_state, false, &theme_css_vars, addr, false, guard, &ws_counter, &gate, knocked, in_allow_list).await;
                                    }
                                }
                            });
                        }
                        Err(_) => break,
                    }
                }
                _ = &mut shutdown_rx => {
                    break;
                }
            }
        }
        *running.write().await = false;
    });

    Ok(())
}

// ============================================================================
// decide_route unit tests (D2 stealth routing matrix)
// ============================================================================

#[cfg(test)]
mod decide_route_tests {
    use super::*;

    const P: &str = "clay";

    /// Non-localhost, stealth mode, open mode (not in allow list), not knocked —
    /// the common "random internet scanner" case.
    fn stealth_open(method: &str, path: &str) -> RouteDecision {
        decide_route(method, path, P, false, false, false)
    }

    #[test]
    fn prefix_serve() {
        // /clay/style.css -> Serve("/style.css"); the caller re-feeds this into the
        // existing static-asset match.
        assert_eq!(stealth_open("GET", "/clay/style.css"), RouteDecision::Serve("/style.css".to_string()));
        assert_eq!(stealth_open("GET", "/clay/app.js"), RouteDecision::Serve("/app.js".to_string()));
        assert_eq!(stealth_open("GET", "/clay/"), RouteDecision::Serve("/".to_string()));
        assert_eq!(stealth_open("GET", "/clay/index.html"), RouteDecision::Serve("/index.html".to_string()));
        assert_eq!(stealth_open("GET", "/clay/theme-editor"), RouteDecision::Serve("/theme-editor".to_string()));
        assert_eq!(
            stealth_open("GET", "/clay/fonts/nunito-latin-400.woff2"),
            RouteDecision::Serve("/fonts/nunito-latin-400.woff2".to_string())
        );
    }

    #[test]
    fn clay_redirects_to_clay_slash() {
        assert_eq!(stealth_open("GET", "/clay"), RouteDecision::Redirect("/clay/".to_string()));
    }

    #[test]
    fn prefix_lookalike_does_not_match() {
        // "/clayfoo" must NOT be treated as under the "/clay" prefix.
        match stealth_open("GET", "/clayfoo") {
            RouteDecision::SilentDrop { .. } => {}
            other => panic!("expected SilentDrop, got {:?}", other),
        }
    }

    #[test]
    fn root_drop_with_violation() {
        match stealth_open("GET", "/") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_some()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
    }

    #[test]
    fn random_probe_drop_with_violation() {
        match stealth_open("GET", "/admin.php") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_some()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
        match stealth_open("GET", "/robots.txt") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_some()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
        match stealth_open("GET", "/.well-known/foo") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_some()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
    }

    #[test]
    fn favicon_drop_no_violation() {
        match stealth_open("GET", "/favicon.ico") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_none()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
        match stealth_open("GET", "/apple-touch-icon.png") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_none()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
        match stealth_open("GET", "/apple-touch-icon-precomposed.png") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_none()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
        // Under the prefix, favicon.ico is a known asset and serves normally.
        assert_eq!(stealth_open("GET", "/clay/favicon.ico"), RouteDecision::Serve("/favicon.ico".to_string()));
    }

    #[test]
    fn localhost_dual_roots() {
        // Legacy root works...
        assert_eq!(decide_route("GET", "/", P, true, false, false), RouteDecision::Serve("/".to_string()));
        assert_eq!(decide_route("GET", "/style.css", P, true, false, false), RouteDecision::Serve("/style.css".to_string()));
        // ...AND the prefixed root works, unconditionally, no allow-list/knock needed.
        assert_eq!(decide_route("GET", "/clay/", P, true, false, false), RouteDecision::Serve("/".to_string()));
        assert_eq!(decide_route("GET", "/clay/style.css", P, true, false, false), RouteDecision::Serve("/style.css".to_string()));
        assert_eq!(decide_route("GET", "/clay", P, true, false, false), RouteDecision::Redirect("/clay/".to_string()));
        // Unknown path at localhost -> legacy 404, not a silent drop.
        assert_eq!(decide_route("GET", "/nope", P, true, false, false), RouteDecision::NotFoundLegacy);
    }

    #[test]
    fn legacy_mode_unaffected() {
        // web_path == "" restores exactly the pre-stealth behavior, for both localhost
        // and remote clients.
        assert_eq!(decide_route("GET", "/", "", false, false, false), RouteDecision::Serve("/".to_string()));
        assert_eq!(decide_route("GET", "/style.css", "", false, false, false), RouteDecision::Serve("/style.css".to_string()));
        assert_eq!(decide_route("GET", "/admin.php", "", false, false, false), RouteDecision::NotFoundLegacy);
        assert_eq!(decide_route("GET", "/", "", true, false, false), RouteDecision::Serve("/".to_string()));
    }

    #[test]
    fn non_get_drop_in_stealth_non_localhost() {
        match stealth_open("POST", "/clay/") {
            RouteDecision::SilentDrop { violation } => assert!(violation.is_some()),
            other => panic!("expected SilentDrop, got {:?}", other),
        }
    }

    #[test]
    fn non_get_legacy_405() {
        assert_eq!(decide_route("POST", "/", "", false, false, false), RouteDecision::MethodNotAllowedLegacy);
        assert_eq!(decide_route("POST", "/", P, true, false, false), RouteDecision::MethodNotAllowedLegacy);
    }

    #[test]
    fn allow_listed_grace_redirect() {
        // An IP actually in the allow list gets bounced from "/" to "/clay/"...
        assert_eq!(decide_route("GET", "/", P, false, true, false), RouteDecision::Redirect("/clay/".to_string()));
        assert_eq!(decide_route("GET", "/index.html", P, false, true, false), RouteDecision::Redirect("/clay/".to_string()));
        // ...but an open-mode (not allow-listed) client hitting "/" is silently dropped.
        assert_eq!(
            stealth_open("GET", "/"),
            RouteDecision::SilentDrop { violation: Some("stealth-probe:/".to_string()) }
        );
    }

    #[test]
    fn knocked_short_circuits_static_requests() {
        // D4: a knocked connection reaching decide_route (i.e. a non-WS-upgrade,
        // static request) is always silently dropped with NO violation, regardless of
        // path, method, or localhost/allow-list status — route_connection logs
        // KNOCK-HTTP-DENIED instead of the usual HTTP-DROP for this case.
        assert_eq!(
            decide_route("GET", "/clay/", P, false, false, true),
            RouteDecision::SilentDrop { violation: None }
        );
        assert_eq!(
            decide_route("GET", "/", "", false, false, true),
            RouteDecision::SilentDrop { violation: None }
        );
        assert_eq!(
            decide_route("GET", "/anything", P, true, true, true),
            RouteDecision::SilentDrop { violation: None }
        );
        assert_eq!(
            decide_route("POST", "/admin.php", P, false, false, true),
            RouteDecision::SilentDrop { violation: None }
        );
    }

    #[test]
    fn strip_web_path_prefix_boundary() {
        assert_eq!(strip_web_path_prefix("/clay", "clay"), Some(String::new()));
        assert_eq!(strip_web_path_prefix("/clay/", "clay"), Some("/".to_string()));
        assert_eq!(strip_web_path_prefix("/clay/app.js", "clay"), Some("/app.js".to_string()));
        assert_eq!(strip_web_path_prefix("/clayfoo", "clay"), None);
        assert_eq!(strip_web_path_prefix("/clay", ""), None);
        assert_eq!(strip_web_path_prefix("/other", "clay"), None);
    }
}

// ============================================================================
// gate_connection decision tests (D3 accept-time IP gate)
// ============================================================================

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn localhost_always_proceeds_regardless_of_list() {
        // Localhost bypasses the gate whether the list is empty, non-empty-and-matched,
        // or non-empty-and-unmatched — `matched` is irrelevant on this branch.
        assert_eq!(decide_gate(true, true, false), GateDecision::Proceed { in_allow_list: false });
        assert_eq!(decide_gate(true, false, false), GateDecision::Proceed { in_allow_list: false });
        assert_eq!(decide_gate(true, false, true), GateDecision::Proceed { in_allow_list: false });
    }

    #[test]
    fn empty_allow_list_is_open_mode_not_in_list() {
        // Empty list => proceed (today's open-mode behavior), but in_allow_list stays
        // false — an empty list is not "membership", it's "no gate configured".
        assert_eq!(decide_gate(false, true, false), GateDecision::Proceed { in_allow_list: false });
    }

    #[test]
    fn matched_non_localhost_proceeds_in_allow_list() {
        assert_eq!(decide_gate(false, false, true), GateDecision::Proceed { in_allow_list: true });
    }

    #[test]
    fn unmatched_non_localhost_drops() {
        assert_eq!(decide_gate(false, false, false), GateDecision::Drop);
    }

    #[test]
    fn gate_ip_matches_exact_ip() {
        let list = vec!["203.0.113.5".to_string()];
        assert!(gate_ip_matches("203.0.113.5", None, &list, None));
        assert!(!gate_ip_matches("203.0.113.6", None, &list, None));
    }

    #[test]
    fn gate_ip_matches_ip_wildcard() {
        let list = vec!["192.168.1.*".to_string()];
        assert!(gate_ip_matches("192.168.1.50", None, &list, None));
        assert!(!gate_ip_matches("192.168.2.50", None, &list, None));
    }

    #[test]
    fn gate_ip_matches_whitelisted_host_overrides_list() {
        // A runtime-whitelisted IP matches even though it's absent from the static list.
        let list = vec!["203.0.113.5".to_string()];
        assert!(gate_ip_matches("198.51.100.9", Some("198.51.100.9"), &list, None));
        // Whitelisting a different IP doesn't leak to this one.
        assert!(!gate_ip_matches("198.51.100.9", Some("198.51.100.10"), &list, None));
    }

    #[test]
    fn gate_ip_matches_hostname_pattern_needs_resolved_hostname() {
        let list = vec!["*.example.com".to_string()];
        assert!(gate_ip_matches("203.0.113.5", None, &list, Some("host.example.com")));
        assert!(!gate_ip_matches("203.0.113.5", None, &list, None));
        assert!(!gate_ip_matches("203.0.113.5", None, &list, Some("host.other.com")));
    }

    #[test]
    fn gate_ip_matches_non_listed_ip_fails() {
        let list = vec!["203.0.113.5".to_string(), "192.168.1.*".to_string()];
        assert!(!gate_ip_matches("8.8.8.8", None, &list, None));
    }

    /// End-to-end sanity: compose `decide_gate` with `gate_ip_matches` the same way
    /// `gate_connection` does, for the "IP listed" and "IP not listed" cases.
    #[test]
    fn full_decision_ip_listed_vs_not_listed() {
        let list = vec!["203.0.113.5".to_string()];
        let matched = gate_ip_matches("203.0.113.5", None, &list, None);
        assert_eq!(decide_gate(false, list.is_empty(), matched), GateDecision::Proceed { in_allow_list: true });

        let matched = gate_ip_matches("8.8.8.8", None, &list, None);
        assert_eq!(decide_gate(false, list.is_empty(), matched), GateDecision::Drop);
    }
}

// ============================================================================
// D6 tests (SECURITY-ROADMAP.md): allow-listed IPs are never banned for probes.
// Covers SecurityGate::strike/is_ban_exempt, BanList::record_auth_failure, and the
// redirect_http_to_https/decide_route parity regression for the reported bug.
// ============================================================================

#[cfg(test)]
mod ban_exempt_tests {
    use super::*;

    fn gate_with(allow_list: Vec<String>, whitelisted: Option<&str>) -> SecurityGate {
        SecurityGate {
            allow_list: Arc::new(std::sync::RwLock::new(allow_list)),
            whitelisted_host: Arc::new(std::sync::RwLock::new(whitelisted.map(|s| s.to_string()))),
            auth_key: Arc::new(std::sync::RwLock::new(None)),
            web_path: "clay".to_string(),
            ban_list: BanList::new(),
        }
    }

    #[test]
    fn decide_ban_exempt_localhost_always_exempt() {
        assert!(decide_ban_exempt("127.0.0.1", None, &[], None));
        assert!(decide_ban_exempt("::1", None, &["*".to_string()], None));
    }

    #[test]
    fn decide_ban_exempt_exact_ip_match() {
        let list = vec!["203.0.113.5".to_string()];
        assert!(decide_ban_exempt("203.0.113.5", None, &list, None));
        assert!(!decide_ban_exempt("203.0.113.6", None, &list, None));
    }

    #[test]
    fn decide_ban_exempt_ip_wildcard_match() {
        let list = vec!["192.168.2.*".to_string()];
        assert!(decide_ban_exempt("192.168.2.6", None, &list, None));
        assert!(!decide_ban_exempt("192.168.3.6", None, &list, None));
    }

    #[test]
    fn decide_ban_exempt_hostname_pattern_match() {
        let list = vec!["*.rd.shawcable.net".to_string()];
        assert!(decide_ban_exempt("96.43.12.34", None, &list, Some("abc.rd.shawcable.net")));
        // No resolved hostname available => hostname patterns can't match.
        assert!(!decide_ban_exempt("96.43.12.34", None, &list, None));
        assert!(!decide_ban_exempt("96.43.12.34", None, &list, Some("other.net")));
    }

    /// D6's deliberate carve-out: a bare "*" allow-list entry means "let everyone
    /// reach the UI", not "nobody on the internet can ever be banned" — it must NOT
    /// confer exemption on its own.
    #[test]
    fn decide_ban_exempt_bare_wildcard_not_exempt() {
        let list = vec!["*".to_string()];
        assert!(!decide_ban_exempt("8.8.8.8", None, &list, None));
        assert!(!decide_ban_exempt("1.1.1.1", None, &list, Some("anything")));
    }

    #[test]
    fn decide_ban_exempt_non_matching_ip_not_exempt() {
        let list = vec!["203.0.113.5".to_string(), "192.168.2.*".to_string()];
        assert!(!decide_ban_exempt("8.8.8.8", None, &list, None));
    }

    #[test]
    fn decide_ban_exempt_whitelisted_host_is_exempt() {
        assert!(decide_ban_exempt("198.51.100.9", Some("198.51.100.9"), &[], None));
        assert!(!decide_ban_exempt("198.51.100.9", Some("198.51.100.10"), &[], None));
    }

    /// A mix: bare "*" present alongside a specific entry — the specific entry still
    /// confers exemption for IPs it covers; it's only the bare "*" that never does.
    #[test]
    fn decide_ban_exempt_specific_entry_alongside_bare_wildcard() {
        let list = vec!["*".to_string(), "192.168.2.*".to_string()];
        assert!(decide_ban_exempt("192.168.2.6", None, &list, None));
        assert!(!decide_ban_exempt("8.8.8.8", None, &list, None));
    }

    #[tokio::test]
    async fn strike_never_bans_an_exempt_ip() {
        let gate = gate_with(vec!["192.168.2.*".to_string()], None);
        // Old threshold was 2 — hammer it well past that; an exempt IP must never ban.
        for _ in 0..10 {
            let banned = gate.strike("192.168.2.6", "stealth-probe:/admin.php").await;
            assert!(!banned, "an allow-listed IP must never be banned by strike()");
        }
        assert!(!gate.ban_list.is_banned("192.168.2.6"));
    }

    #[tokio::test]
    async fn strike_still_bans_a_non_exempt_ip() {
        let gate = gate_with(vec!["192.168.2.*".to_string()], None);
        assert!(!gate.strike("8.8.8.8", "stealth-probe:/admin.php").await);
        assert!(gate.strike("8.8.8.8", "stealth-probe:/admin.php").await);
        assert!(gate.ban_list.is_banned("8.8.8.8"));
    }

    /// D6, decision 2: `record_auth_failure` bans at 5, not 2, and applies to
    /// allow-listed IPs too (no allow-list exemption — this is the one strike that
    /// actually protects the password).
    #[test]
    fn record_auth_failure_bans_on_fifth_not_second() {
        let ban_list = BanList::new();
        for _ in 0..4 {
            assert!(!ban_list.record_auth_failure("192.168.2.6", "WebSocket: failed auth"));
        }
        assert!(!ban_list.is_banned("192.168.2.6"));
        assert!(ban_list.record_auth_failure("192.168.2.6", "WebSocket: failed auth"));
        assert!(ban_list.is_banned("192.168.2.6"));
    }

    #[test]
    fn record_auth_failure_exempts_localhost_only() {
        let ban_list = BanList::new();
        for _ in 0..10 {
            assert!(!ban_list.record_auth_failure("127.0.0.1", "WebSocket: failed auth"));
        }
        assert!(!ban_list.is_banned("127.0.0.1"));
    }

    #[test]
    fn record_violation_threshold_unchanged_at_two() {
        let ban_list = BanList::new();
        assert!(!ban_list.record_violation("8.8.8.8", "stealth-probe:/x"));
        assert!(ban_list.record_violation("8.8.8.8", "stealth-probe:/x"));
    }
}

// ============================================================================
// redirect_http_to_https / decide_route parity regression tests — the reported bug:
// an allow-listed IP hitting `http://host:9000/` (instead of `https://`) was struck and
// banned because redirect_http_to_https re-derived reachability with its own
// `path_allowed` check that never saw `in_allow_list`. The fix makes it call
// `decide_route` directly, so these tests drive the real function over an in-memory
// duplex stream rather than re-deriving the expectation by hand.
// ============================================================================

#[cfg(test)]
mod redirect_parity_tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    fn test_gate(allow_list: Vec<String>) -> SecurityGate {
        SecurityGate {
            allow_list: Arc::new(std::sync::RwLock::new(allow_list)),
            whitelisted_host: Arc::new(std::sync::RwLock::new(None)),
            auth_key: Arc::new(std::sync::RwLock::new(None)),
            web_path: "clay".to_string(),
            ban_list: BanList::new(),
        }
    }

    /// Drive `redirect_http_to_https` over an in-memory duplex stream and return
    /// whatever bytes it wrote back before returning (empty = silently dropped).
    async fn run_redirect(
        gate: &SecurityGate,
        request_path: &str,
        is_localhost: bool,
        in_allow_list: bool,
        knocked: bool,
    ) -> Vec<u8> {
        let (mut client, server) = tokio::io::duplex(8192);
        let request = format!("GET {} HTTP/1.1\r\nHost: host.example\r\n\r\n", request_path);
        client.write_all(request.as_bytes()).await.unwrap();
        redirect_http_to_https(server, 9000, is_localhost, in_allow_list, knocked, gate, "192.168.2.6").await;
        let mut buf = vec![0u8; 8192];
        match tokio::time::timeout(std::time::Duration::from_millis(300), client.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => buf[..n].to_vec(),
            _ => Vec::new(),
        }
    }

    /// Like `run_redirect` but lets the caller supply an arbitrary `Host` header value
    /// (for the C3 Host-sanitization regression test below).
    async fn run_redirect_with_host(
        gate: &SecurityGate,
        request_path: &str,
        host_header: &str,
        is_localhost: bool,
        in_allow_list: bool,
        knocked: bool,
    ) -> Vec<u8> {
        let (mut client, server) = tokio::io::duplex(8192);
        let request = format!("GET {} HTTP/1.1\r\nHost: {}\r\n\r\n", request_path, host_header);
        client.write_all(request.as_bytes()).await.unwrap();
        redirect_http_to_https(server, 9000, is_localhost, in_allow_list, knocked, gate, "192.168.2.6").await;
        let mut buf = vec![0u8; 8192];
        match tokio::time::timeout(std::time::Duration::from_millis(300), client.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => buf[..n].to_vec(),
            _ => Vec::new(),
        }
    }

    /// C3 (security remediation): a malicious `Host` header must not survive into the
    /// `Location` header or the HTML body of the plain-HTTP-to-HTTPS redirect. Before
    /// the fix, `Host: "><script>alert(1)</script>` was reflected verbatim into both,
    /// giving reflected XSS (via the HTML body) and a corrupted `Location` value.
    #[tokio::test]
    async fn redirect_sanitizes_malicious_host_header() {
        let gate = test_gate(vec!["192.168.2.*".to_string()]);
        let malicious_host = "\"><script>alert(1)</script>";
        let response = run_redirect_with_host(&gate, "/", malicious_host, false, true, false).await;
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 301"), "expected 301, got: {text:?}");
        // The raw payload must not appear anywhere in the response — neither the
        // Location header (host chars filtered to alphanumeric/./-/:) nor the HTML
        // body (also HTML-escaped as defense in depth).
        assert!(
            !text.contains("<script>"),
            "malicious host leaked an executable <script> tag into the response: {text:?}"
        );
        assert!(
            !text.contains("\"><script>"),
            "malicious host broke out of the href attribute: {text:?}"
        );
        // Location header's host component must only contain the sanitized
        // (alphanumeric/./-/:) characters — the raw `"><script>...` payload must be gone.
        let location_line = text.lines().find(|l| l.starts_with("Location:")).unwrap();
        let url = location_line.trim_start_matches("Location:").trim();
        let host_part = url.trim_start_matches("https://").split('/').next().unwrap();
        assert!(
            host_part.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':'),
            "Location header host component contains unsanitized characters: {host_part:?}"
        );
    }

    /// THE REPORTED BUG, reproduced directly: an allow-listed IP hitting the bare "/"
    /// over plain HTTP (web_secure=true, http/https typo) must get a 301 towards
    /// `/{web_path}/` — the same grace `decide_route` gives the `https://` request —
    /// not a silent drop. Before the fix this path was silently dropped AND struck.
    #[tokio::test]
    async fn allow_listed_root_gets_grace_redirect_not_a_drop() {
        let gate = test_gate(vec!["192.168.2.*".to_string()]);
        let response = run_redirect(&gate, "/", false, true, false).await;
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 301"), "expected 301, got: {text:?}");
        assert!(
            text.contains("Location: https://host.example:9000/clay/"),
            "expected grace redirect straight to /clay/, got: {text:?}"
        );
    }

    /// Two consecutive http/https typos from the same allow-listed IP — the exact
    /// scenario from the bug report — must never produce a ban.
    #[tokio::test]
    async fn allow_listed_root_probed_twice_never_bans() {
        let gate = test_gate(vec!["192.168.2.*".to_string()]);
        for _ in 0..5 {
            let response = run_redirect(&gate, "/", false, true, false).await;
            assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 301"));
        }
        assert!(!gate.ban_list.is_banned("192.168.2.6"));
    }

    /// Open-mode (not allow-listed) root probe must still be silently dropped — the
    /// grace redirect is specifically for genuine allow-list membership.
    #[tokio::test]
    async fn open_mode_root_still_silently_drops() {
        let gate = test_gate(vec![]);
        let response = run_redirect(&gate, "/", false, false, false).await;
        assert!(response.is_empty(), "expected silent drop, got: {:?}", String::from_utf8_lossy(&response));
    }

    /// Parity sweep: for every (path, in_allow_list) pair, whether
    /// redirect_http_to_https responds at all must match whether decide_route's
    /// verdict is reachable (Serve/Redirect/legacy) vs SilentDrop. This is the general
    /// regression guard — the bug was exactly these two functions disagreeing.
    #[tokio::test]
    async fn response_reachability_matches_decide_route_for_matrix() {
        let cases: &[(&str, bool)] = &[
            ("/", false),
            ("/", true),
            ("/index.html", true),
            ("/clay/", false),
            ("/clay/style.css", false),
            ("/clay/nonexistent", false),
            ("/admin.php", false),
            ("/admin.php", true),
            ("/favicon.ico", false),
        ];
        for &(path, in_allow_list) in cases {
            let gate = test_gate(if in_allow_list { vec!["192.168.2.*".to_string()] } else { vec![] });
            let decision = decide_route("GET", path, "clay", false, in_allow_list, false);
            let expect_response = !matches!(decision, RouteDecision::SilentDrop { .. });
            let response = run_redirect(&gate, path, false, in_allow_list, false).await;
            assert_eq!(
                !response.is_empty(), expect_response,
                "path={path:?} in_allow_list={in_allow_list}: decide_route={decision:?} but redirect_http_to_https {}",
                if response.is_empty() { "dropped" } else { "responded" }
            );
        }
    }
}

// ============================================================================
// CLAY-KNOCK v1 tests (D4, SECURITY-ROADMAP.md Phase 3)
// ============================================================================

#[cfg(test)]
mod knock_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Hard-coded vector: raw SHA256("testkey" || [0x01;32]), computed independently
    /// via `python3 -c "import hashlib; print(hashlib.sha256(b'testkey'+bytes([1]*32)).hexdigest())"`
    /// during development, per SECURITY-ROADMAP.md Phase 3 step 4.
    const TESTKEY_CHALLENGE_ALL_ONES_DIGEST: [u8; 32] = [
        0x61, 0xb6, 0x78, 0xd6, 0xc0, 0xed, 0x2d, 0xb5, 0x9e, 0xdb, 0x7e, 0x2d, 0xf9, 0xe8, 0x1d, 0xe3,
        0x04, 0x41, 0xd8, 0x88, 0xff, 0x11, 0x2a, 0xa2, 0x49, 0xfd, 0x1b, 0xfe, 0x01, 0x81, 0x1c, 0x3a,
    ];

    #[test]
    fn parse_knock_hello_accepts_valid() {
        assert!(parse_knock_hello(&[0xC7, 0x4C, 0x41, 0x59, 0x01, 0x00]));
    }

    #[test]
    fn parse_knock_hello_rejects_wrong_magic() {
        assert!(!parse_knock_hello(&[0xC6, 0x4C, 0x41, 0x59, 0x01, 0x00]));
        assert!(!parse_knock_hello(&[0xC7, 0x4C, 0x41, 0x58, 0x01, 0x00]));
    }

    #[test]
    fn parse_knock_hello_rejects_wrong_version() {
        assert!(!parse_knock_hello(&[0xC7, 0x4C, 0x41, 0x59, 0x02, 0x00]));
        assert!(!parse_knock_hello(&[0xC7, 0x4C, 0x41, 0x59, 0x00, 0x00]));
    }

    #[test]
    fn parse_knock_hello_rejects_nonzero_reserved() {
        assert!(!parse_knock_hello(&[0xC7, 0x4C, 0x41, 0x59, 0x01, 0x01]));
        assert!(!parse_knock_hello(&[0xC7, 0x4C, 0x41, 0x59, 0x01, 0xFF]));
    }

    #[test]
    fn knock_expected_response_matches_fixed_vector() {
        let challenge = [0x01u8; 32];
        let digest = knock_expected_response("testkey", &challenge);
        assert_eq!(digest, TESTKEY_CHALLENGE_ALL_ONES_DIGEST);
    }

    #[test]
    fn knock_expected_response_differs_for_different_keys() {
        let challenge = [0x01u8; 32];
        let d1 = knock_expected_response("testkey", &challenge);
        let d2 = knock_expected_response("otherkey", &challenge);
        assert_ne!(d1, d2);
    }

    // constant_time_eq now lives in util.rs (see constant_time_eq_sanity there) — it's
    // shared with websocket.rs/main.rs for the WS/API-key credential compares (B3).

    fn test_gate(auth_key: Option<&str>) -> SecurityGate {
        SecurityGate {
            allow_list: Arc::new(std::sync::RwLock::new(Vec::new())),
            whitelisted_host: Arc::new(std::sync::RwLock::new(None)),
            auth_key: Arc::new(std::sync::RwLock::new(auth_key.map(|s| s.to_string()))),
            web_path: "clay".to_string(),
            ban_list: BanList::new(),
        }
    }

    /// Spin up a real localhost TcpListener, accept exactly one connection, and run
    /// `gate_connection` on it in a background task (tokio::io::duplex won't work here —
    /// gate_connection is specialized to tokio::net::TcpStream for its peek() call).
    /// Returns the connected client TcpStream and a handle to await the server-side
    /// GateResult.
    async fn spawn_gate_server(gate: SecurityGate) -> (tokio::net::TcpStream, tokio::task::JoinHandle<GateResult>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, peer) = listener.accept().await.unwrap();
            let client_ip = peer.ip().to_string();
            gate_connection(&mut stream, &client_ip, &gate).await
        });
        let client = tokio::net::TcpStream::connect(addr).await.unwrap();
        (client, handle)
    }

    #[tokio::test]
    async fn good_knock_proceeds_and_acks() {
        let gate = test_gate(Some("testkey"));
        let (mut client, handle) = spawn_gate_server(gate).await;

        client.write_all(&[0xC7, 0x4C, 0x41, 0x59, 0x01, 0x00]).await.unwrap();
        let mut challenge_msg = [0u8; 34];
        client.read_exact(&mut challenge_msg).await.unwrap();
        assert_eq!(&challenge_msg[0..2], &KNOCK_CHALLENGE_MAGIC);
        let mut challenge = [0u8; 32];
        challenge.copy_from_slice(&challenge_msg[2..34]);

        let response = knock_expected_response("testkey", &challenge);
        client.write_all(&response).await.unwrap();

        let mut ack = [0u8; 2];
        client.read_exact(&mut ack).await.unwrap();
        assert_eq!(ack, KNOCK_ACK);

        let result = handle.await.unwrap();
        assert_eq!(result, GateResult::Proceed { knocked: true, in_allow_list: false });
    }

    #[tokio::test]
    async fn bad_digest_drops_no_ack() {
        let gate = test_gate(Some("testkey"));
        let (mut client, handle) = spawn_gate_server(gate).await;

        client.write_all(&[0xC7, 0x4C, 0x41, 0x59, 0x01, 0x00]).await.unwrap();
        let mut challenge_msg = [0u8; 34];
        client.read_exact(&mut challenge_msg).await.unwrap();

        // Wrong digest — never matches a real SHA256 output.
        client.write_all(&[0u8; 32]).await.unwrap();

        // Server closes without ever sending an ACK.
        let mut ack = [0u8; 2];
        let read_result = client.read_exact(&mut ack).await;
        assert!(read_result.is_err(), "expected connection close, got bytes: {:?}", ack);

        let result = handle.await.unwrap();
        assert_eq!(result, GateResult::Drop);
    }

    #[tokio::test]
    async fn garbage_after_magic_byte_drops() {
        let gate = test_gate(Some("testkey"));
        let (mut client, handle) = spawn_gate_server(gate).await;

        // First byte 0xC7 triggers the knock path, but the rest of the 6-byte hello
        // is garbage — parse_knock_hello must reject it (KNOCK-BAD-MAGIC).
        client.write_all(&[0xC7, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).await.unwrap();
        client.flush().await.unwrap();

        let result = handle.await.unwrap();
        assert_eq!(result, GateResult::Drop);
    }

    #[tokio::test]
    async fn no_stored_key_fails_knock() {
        // Multiuser mode: the auth_key Arc holds None.
        let gate = test_gate(None);
        let (mut client, handle) = spawn_gate_server(gate).await;

        client.write_all(&[0xC7, 0x4C, 0x41, 0x59, 0x01, 0x00]).await.unwrap();
        let mut challenge_msg = [0u8; 34];
        client.read_exact(&mut challenge_msg).await.unwrap();
        client.write_all(&[0u8; 32]).await.unwrap();

        let result = handle.await.unwrap();
        assert_eq!(result, GateResult::Drop);
    }

    #[tokio::test]
    async fn non_knock_byte_falls_through_untouched() {
        // First byte is not 0xC7 — gate_connection must fall straight through to the
        // normal allow-list gate without consuming any bytes (empty allow list here =>
        // open mode => Proceed{knocked:false}).
        let gate = test_gate(Some("testkey"));
        let (mut client, handle) = spawn_gate_server(gate).await;

        client.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();

        let result = handle.await.unwrap();
        assert_eq!(result, GateResult::Proceed { knocked: false, in_allow_list: false });
    }
}
