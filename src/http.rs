use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
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

/// Log a remote connection event to clay.remote.log
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
        .open("clay.remote.log")
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

/// Handle a plain HTTP connection on the HTTPS port by sending a redirect to HTTPS.
/// Reads the HTTP request, extracts Host and path, and responds with 301.
async fn redirect_http_to_https<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, port: u16) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = parse_http_request(&request)
        .map(|(_, p)| p)
        .unwrap_or("/");
    let host = get_host_from_request(&request);
    let host = if host.is_empty() { "localhost".to_string() } else { host };
    let location = if port == 443 {
        format!("https://{}{}", host, path)
    } else {
        format!("https://{}:{}{}", host, port, path)
    };
    let body = format!("Redirecting to <a href=\"{loc}\">{loc}</a>", loc=location);
    let response = format!(
        "HTTP/1.1 301 Moved Permanently\r\n\
         Location: {}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        location, body.len(), body
    );
    let _ = stream.write_all(response.as_bytes()).await;
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

/// Handle HTTP route matching and response generation (shared by all server implementations)
/// WS_PORT is injected as 0 (sentinel) — the JS client uses window.location.port
fn handle_http_routes(
    request: &str,
    ws_use_tls: bool,
    theme_css_vars: &str,
    is_https: bool,
) -> Option<RouteResult> {
    let (method, full_path) = parse_http_request(request)?;
    // Strip query string for route matching (e.g. "/?world=Name" → "/")
    let path = full_path.split('?').next().unwrap_or(full_path);

    if method != "GET" {
        return Some(RouteResult::MethodNotAllowed(
            build_http_response(405, "Method Not Allowed", "text/plain", "Method Not Allowed", is_https)
        ));
    }

    let host = get_host_from_request(request);
    // Only allow hostname-valid characters (alphanumeric, dots, hyphens, colons for port)
    let sanitized_host: String = host.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-' || *c == ':')
        .collect();
    let response = match path {
        "/" | "/index.html" => {
            let html = WEB_INDEX_HTML
                .replace("{{WS_HOST}}", &sanitized_host)
                .replace("{{WS_PORT}}", "0")
                .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" })
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
                .replace("{{WS_HOST}}", &sanitized_host)
                .replace("{{WS_PORT}}", "0")
                .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
            RouteResult::Ok(build_http_response(200, "OK", "text/html", &html, is_https))
        }
        "/keybind-editor" => {
            let html = WEB_KEYBIND_EDITOR_HTML
                .replace("{{WS_HOST}}", &sanitized_host)
                .replace("{{WS_PORT}}", "0")
                .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
            RouteResult::Ok(build_http_response(200, "OK", "text/html", &html, is_https))
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

/// Route an incoming connection: detect WebSocket upgrades vs HTTP requests.
/// If ws_state is provided and the request is a WebSocket upgrade, hands off to the WS handler.
/// Otherwise, serves static HTTP content.
async fn route_connection<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    mut stream: S,
    ws_state: Option<Arc<WsConnectionState>>,
    ws_use_tls: bool,
    theme_css_vars: &str,
    client_addr: std::net::SocketAddr,
    ban_list: &BanList,
    is_https: bool,
) {
    let client_ip = client_addr.ip().to_string();

    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);

    // Check for WebSocket upgrade
    if is_websocket_upgrade(&request) {
        if let Some(ws_state) = ws_state {
            let client_id = ws_state.next_client_id();
            let pw_hash = ws_state.password_hash.read().unwrap().clone();
            let prefixed = PrefixedStream::new(buf[..n].to_vec(), stream);
            let _ = crate::websocket::handle_ws_client(
                prefixed,
                client_id,
                ws_state.clients.clone(),
                pw_hash,
                ws_state.allow_list.clone(),
                ws_state.whitelisted_host.clone(),
                client_addr,
                ws_state.event_tx.clone(),
                ws_state.multiuser_mode,
                ws_state.users.clone(),
                ws_state.ban_list.clone(),
            ).await;
        } else {
            // WebSocket not configured (no password set)
            let response = build_http_response(503, "Service Unavailable", "text/plain", "WebSocket not configured", is_https);
            let _ = stream.write_all(&response).await;
        }
        return;
    }

    // Normal HTTP request
    if let Some(route_result) = handle_http_routes(&request, ws_use_tls, theme_css_vars, is_https) {
        let response = match route_result {
            RouteResult::Ok(r) => r,
            RouteResult::MethodNotAllowed(r) => {
                let _ = stream.write_all(&r).await;
                return;
            }
            RouteResult::NotFound(r, path) => {
                log_http_404(&client_ip, &path);
                ban_list.record_violation(&client_ip, &path);
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
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            let client_ip = addr.ip().to_string();
                            if ban_list.is_banned(&client_ip) {
                                continue;
                            }
                            let _ = stream.set_nodelay(true);
                            let tls_acceptor = tls_acceptor.clone();
                            let theme_css_vars = theme_css_vars.clone();
                            let ws_state = ws_state.clone();
                            let ban_list = ban_list.clone();
                            tokio::spawn(async move {
                                // Peek first byte to detect plain HTTP vs TLS
                                let mut peek = [0u8; 1];
                                match stream.peek(&mut peek).await {
                                    Ok(1) if peek[0] != 0x16 => {
                                        // Not a TLS ClientHello — redirect HTTP to HTTPS
                                        redirect_http_to_https(stream, server_port).await;
                                        return;
                                    }
                                    Ok(0) | Err(_) => return,
                                    _ => {}
                                }
                                match tls_acceptor.accept(stream).await {
                                    Ok(tls_stream) => {
                                        route_connection(tls_stream, ws_state, true, &theme_css_vars, addr, &ban_list, true).await;
                                    }
                                    Err(e) => {
                                        log_remote_event("TLS-ERROR", &client_ip, &format!("{}", e));
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
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, addr)) => {
                            let client_ip = addr.ip().to_string();
                            if ban_list.is_banned(&client_ip) {
                                continue;
                            }
                            let _ = stream.set_nodelay(true);
                            let tls_acceptor = tls_acceptor.clone();
                            let theme_css_vars = theme_css_vars.clone();
                            let ws_state = ws_state.clone();
                            let ban_list = ban_list.clone();
                            tokio::spawn(async move {
                                // Peek first byte to detect plain HTTP vs TLS
                                let mut peek = [0u8; 1];
                                match stream.peek(&mut peek).await {
                                    Ok(1) if peek[0] != 0x16 => {
                                        // Not a TLS ClientHello — redirect HTTP to HTTPS
                                        redirect_http_to_https(stream, server_port).await;
                                        return;
                                    }
                                    Ok(0) | Err(_) => return,
                                    _ => {}
                                }
                                match tls_acceptor.accept(stream).await {
                                    Ok(tls_stream) => {
                                        route_connection(tls_stream, ws_state, true, &theme_css_vars, addr, &ban_list, true).await;
                                    }
                                    Err(e) => {
                                        log_remote_event("TLS-ERROR", &client_ip, &format!("{}", e));
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

/// Tracks violations and bans for IP addresses
#[derive(Clone)]
pub struct BanList {
    /// Permanently banned IPs (saved to .dat file)
    permanent_bans: Arc<std::sync::RwLock<HashSet<String>>>,
    /// Temporary bans: IP -> expiration time (Unix timestamp)
    temp_bans: Arc<std::sync::RwLock<HashMap<String, u64>>>,
    /// Violation tracking: IP -> list of violation timestamps
    violations: Arc<std::sync::RwLock<HashMap<String, Vec<u64>>>>,
    /// Ban reasons: IP -> last URL/reason that caused the ban
    ban_reasons: Arc<std::sync::RwLock<HashMap<String, String>>>,
}

impl BanList {
    pub fn new() -> Self {
        Self {
            permanent_bans: Arc::new(std::sync::RwLock::new(HashSet::new())),
            temp_bans: Arc::new(std::sync::RwLock::new(HashMap::new())),
            violations: Arc::new(std::sync::RwLock::new(HashMap::new())),
            ban_reasons: Arc::new(std::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Check if an IP is currently banned (permanent or temporary)
    pub fn is_banned(&self, ip: &str) -> bool {
        // Check permanent bans
        if self.permanent_bans.read().unwrap().contains(ip) {
            return true;
        }
        // Check temporary bans
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Some(&expiry) = self.temp_bans.read().unwrap().get(ip) {
            if now < expiry {
                return true;
            }
        }
        false
    }

    /// Record a violation for an IP address with a reason (URL or description)
    /// Returns true if the IP should be banned (5+ violations in 1 hour = permanent)
    pub fn record_violation(&self, ip: &str, reason: &str) -> bool {
        // Never ban localhost
        if ip == "127.0.0.1" || ip == "::1" || ip == "localhost" {
            return false;
        }

        // Store the reason for this violation
        self.ban_reasons.write().unwrap().insert(ip.to_string(), reason.to_string());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let one_hour_ago = now.saturating_sub(3600);

        let mut violations = self.violations.write().unwrap();
        let ip_violations = violations.entry(ip.to_string()).or_default();

        // Remove old violations (older than 1 hour)
        ip_violations.retain(|&ts| ts > one_hour_ago);

        // Add new violation
        ip_violations.push(now);

        let violation_count = ip_violations.len();

        if violation_count >= 5 {
            // Permanent ban
            self.permanent_bans.write().unwrap().insert(ip.to_string());
            log_ban(ip, "PERMANENT", reason);
            violations.remove(ip); // Clear violations since permanently banned
            true
        } else if violation_count >= 3 {
            // Temporary ban (5 minutes) after 3+ violations
            self.temp_bans.write().unwrap().insert(ip.to_string(), now + 300);
            log_ban(ip, "TEMPORARY", reason);
            true
        } else {
            false
        }
    }

    /// Add a permanent ban directly
    pub fn add_permanent_ban(&self, ip: &str) {
        self.permanent_bans.write().unwrap().insert(ip.to_string());
    }

    /// Get all permanent bans (for saving to .dat file)
    pub fn get_permanent_bans(&self) -> Vec<String> {
        self.permanent_bans.read().unwrap().iter().cloned().collect()
    }

    /// Clean up expired temporary bans
    pub fn cleanup_expired(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.temp_bans.write().unwrap().retain(|_, &mut expiry| expiry > now);
    }

    /// Remove a ban (both permanent and temporary) for an IP
    /// Returns true if a ban was removed
    pub fn remove_ban(&self, ip: &str) -> bool {
        let removed_perm = self.permanent_bans.write().unwrap().remove(ip);
        let removed_temp = self.temp_bans.write().unwrap().remove(ip).is_some();
        // Also clear violations and reason
        self.violations.write().unwrap().remove(ip);
        self.ban_reasons.write().unwrap().remove(ip);
        removed_perm || removed_temp
    }

    /// Get all current bans with their reasons
    /// Returns Vec of (ip, ban_type, reason) where ban_type is "permanent" or "temporary"
    pub fn get_ban_info(&self) -> Vec<(String, String, String)> {
        let mut result = Vec::new();
        let reasons = self.ban_reasons.read().unwrap();

        // Get permanent bans
        for ip in self.permanent_bans.read().unwrap().iter() {
            let reason = reasons.get(ip).cloned().unwrap_or_default();
            result.push((ip.clone(), "permanent".to_string(), reason));
        }

        // Get active temporary bans
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for (ip, expiry) in self.temp_bans.read().unwrap().iter() {
            if *expiry > now {
                let reason = reasons.get(ip).cloned().unwrap_or_default();
                result.push((ip.clone(), "temporary".to_string(), reason));
            }
        }

        result
    }
}

impl Default for BanList {
    fn default() -> Self {
        Self::new()
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
                                let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 7\r\nConnection: close\r\n\r\nBanned\n").await;
                                continue;
                            }
                            let _ = stream.set_nodelay(true);
                            let ban_list_clone = ban_list.clone();
                            let theme_css_vars = theme_css_vars.clone();
                            let ws_state = ws_state.clone();
                            tokio::spawn(async move {
                                route_connection(stream, ws_state, false, &theme_css_vars, addr, &ban_list_clone, false).await;
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
