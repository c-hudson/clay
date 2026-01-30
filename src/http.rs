use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

// ============================================================================
// Remote Connection Logging
// ============================================================================

/// Log a remote connection event to clay.remote.log
fn log_remote_event(event_type: &str, ip: &str, details: &str) {
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

/// Embedded HTML for the web interface
#[cfg(feature = "native-tls-backend")]
const WEB_INDEX_HTML: &str = include_str!("web/index.html");

/// Embedded CSS for the web interface
#[cfg(feature = "native-tls-backend")]
const WEB_STYLE_CSS: &str = include_str!("web/style.css");

/// Embedded JavaScript for the web interface
#[cfg(feature = "native-tls-backend")]
const WEB_APP_JS: &str = include_str!("web/app.js");

/// Parse an HTTP request line and return the method and path
#[cfg(feature = "native-tls-backend")]
fn parse_http_request(request: &str) -> Option<(&str, &str)> {
    let first_line = request.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    Some((method, path))
}

/// Extract Host header from HTTP request (without port)
#[cfg(feature = "native-tls-backend")]
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
#[cfg(feature = "native-tls-backend")]
fn build_http_response(status: u16, status_text: &str, content_type: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        status, status_text, content_type, body.len(), body
    ).into_bytes()
}

/// Build an HTTP response with binary body (for images)
#[cfg(feature = "native-tls-backend")]
fn build_http_response_binary(status: u16, status_text: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        status, status_text, content_type, body.len()
    );
    let mut response = header.into_bytes();
    response.extend_from_slice(body);
    response
}

/// Handle an HTTPS connection
#[cfg(feature = "native-tls-backend")]
async fn handle_https_client(
    mut stream: tokio_native_tls::TlsStream<TcpStream>,
    ws_port: u16,
    ws_use_tls: bool,
) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);

    if let Some((method, path)) = parse_http_request(&request) {
        if method != "GET" {
            let response = build_http_response(405, "Method Not Allowed", "text/plain", "Method Not Allowed");
            let _ = stream.write_all(&response).await;
            return;
        }

        let host = get_host_from_request(&request);
        let response = match path {
            "/" | "/index.html" => {
                // Inject WebSocket configuration into the HTML
                let html = WEB_INDEX_HTML
                    .replace("{{WS_HOST}}", &host)
                    .replace("{{WS_PORT}}", &ws_port.to_string())
                    .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
                build_http_response(200, "OK", "text/html", &html)
            }
            "/style.css" => {
                build_http_response(200, "OK", "text/css", WEB_STYLE_CSS)
            }
            "/app.js" => {
                build_http_response(200, "OK", "application/javascript", WEB_APP_JS)
            }
            "/favicon.ico" => {
                build_http_response(204, "No Content", "image/x-icon", "")
            }
            _ => {
                build_http_response(404, "Not Found", "text/plain", "Not Found")
            }
        };

        let _ = stream.write_all(&response).await;
    }
}

/// Start the HTTPS server
#[cfg(feature = "native-tls-backend")]
pub async fn start_https_server(
    server: &mut HttpsServer,
    cert_file: &str,
    key_file: &str,
    ws_port: u16,
    ws_use_tls: bool,
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
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let running = Arc::clone(&server.running);
    *running.write().await = true;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            // Disable Nagle's algorithm for lower latency
                            let _ = stream.set_nodelay(true);
                            let tls_acceptor = tls_acceptor.clone();
                            tokio::spawn(async move {
                                if let Ok(tls_stream) = tls_acceptor.accept(stream).await {
                                    handle_https_client(tls_stream, ws_port, ws_use_tls).await;
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

/// Embedded HTML for the web interface (rustls version)
#[cfg(feature = "rustls-backend")]
const WEB_INDEX_HTML: &str = include_str!("web/index.html");

/// Embedded CSS for the web interface (rustls version)
#[cfg(feature = "rustls-backend")]
const WEB_STYLE_CSS: &str = include_str!("web/style.css");

/// Embedded JavaScript for the web interface (rustls version)
#[cfg(feature = "rustls-backend")]
const WEB_APP_JS: &str = include_str!("web/app.js");

/// Parse an HTTP request line and return the method and path (rustls version)
#[cfg(feature = "rustls-backend")]
fn parse_http_request(request: &str) -> Option<(&str, &str)> {
    let first_line = request.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    Some((method, path))
}

/// Extract Host header from HTTP request (without port) - rustls version
#[cfg(feature = "rustls-backend")]
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

/// Build an HTTP response with the given status, content type, and body (rustls version)
#[cfg(feature = "rustls-backend")]
fn build_http_response(status: u16, status_text: &str, content_type: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        status, status_text, content_type, body.len(), body
    ).into_bytes()
}

/// Build an HTTP response with binary body (for images) (rustls version)
#[cfg(feature = "rustls-backend")]
fn build_http_response_binary(status: u16, status_text: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        status, status_text, content_type, body.len()
    );
    let mut response = header.into_bytes();
    response.extend_from_slice(body);
    response
}

/// Handle an HTTPS connection (rustls version)
#[cfg(feature = "rustls-backend")]
async fn handle_https_client(
    mut stream: tokio_rustls::server::TlsStream<TcpStream>,
    ws_port: u16,
    ws_use_tls: bool,
) {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);

    if let Some((method, path)) = parse_http_request(&request) {
        if method != "GET" {
            let response = build_http_response(405, "Method Not Allowed", "text/plain", "Method Not Allowed");
            let _ = stream.write_all(&response).await;
            return;
        }

        let host = get_host_from_request(&request);
        let response = match path {
            "/" | "/index.html" => {
                // Inject WebSocket configuration into the HTML
                let html = WEB_INDEX_HTML
                    .replace("{{WS_HOST}}", &host)
                    .replace("{{WS_PORT}}", &ws_port.to_string())
                    .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
                build_http_response(200, "OK", "text/html", &html)
            }
            "/style.css" => {
                build_http_response(200, "OK", "text/css", WEB_STYLE_CSS)
            }
            "/app.js" => {
                build_http_response(200, "OK", "application/javascript", WEB_APP_JS)
            }
            "/favicon.ico" => {
                build_http_response(204, "No Content", "image/x-icon", "")
            }
            _ => {
                build_http_response(404, "Not Found", "text/plain", "Not Found")
            }
        };

        let _ = stream.write_all(&response).await;
    }
}

/// Start the HTTPS server (rustls version)
#[cfg(feature = "rustls-backend")]
pub async fn start_https_server(
    server: &mut HttpsServer,
    cert_file: &str,
    key_file: &str,
    ws_port: u16,
    ws_use_tls: bool,
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
    let listener = tokio::net::TcpListener::bind(&addr).await
        .map_err(|e| format!("Failed to bind to port {}: {}", server.port, e))?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let running = Arc::clone(&server.running);
    *running.write().await = true;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            // Disable Nagle's algorithm for lower latency
                            let _ = stream.set_nodelay(true);
                            let tls_acceptor = tls_acceptor.clone();
                            tokio::spawn(async move {
                                if let Ok(tls_stream) = tls_acceptor.accept(stream).await {
                                    handle_https_client(tls_stream, ws_port, ws_use_tls).await;
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
}

impl HttpServer {
    pub fn new(port: u16) -> Self {
        Self {
            running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
            port,
        }
    }
}

/// Handle an HTTP connection (plain TCP, no TLS)
/// Returns true if a violation occurred (404 access)
async fn handle_http_client(
    mut stream: TcpStream,
    ws_port: u16,
    ws_use_tls: bool,
    ban_list: BanList,
    client_ip: String,
) {
    // Check if IP is banned
    if ban_list.is_banned(&client_ip) {
        // Send minimal response and close
        let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 7\r\nConnection: close\r\n\r\nBanned\n").await;
        return;
    }

    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);

    // Reuse the parse function from either TLS backend
    fn parse_request(request: &str) -> Option<(&str, &str)> {
        let first_line = request.lines().next()?;
        let mut parts = first_line.split_whitespace();
        let method = parts.next()?;
        let path = parts.next()?;
        Some((method, path))
    }

    fn get_host(request: &str) -> String {
        for line in request.lines() {
            if line.to_lowercase().starts_with("host:") {
                let host = line[5..].trim();
                if let Some(colon_pos) = host.rfind(':') {
                    return host[..colon_pos].to_string();
                }
                return host.to_string();
            }
        }
        String::new()
    }

    fn build_response(status: u16, status_text: &str, content_type: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: {}; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            status, status_text, content_type, body.len(), body
        ).into_bytes()
    }

    fn build_response_binary(status: u16, status_text: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
        let header = format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: {}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n",
            status, status_text, content_type, body.len()
        );
        let mut response = header.into_bytes();
        response.extend_from_slice(body);
        response
    }

    if let Some((method, path)) = parse_request(&request) {
        if method != "GET" {
            let response = build_response(405, "Method Not Allowed", "text/plain", "Method Not Allowed");
            let _ = stream.write_all(&response).await;
            return;
        }

        // Read embedded files at compile time
        const HTTP_INDEX_HTML: &str = include_str!("web/index.html");
        const HTTP_STYLE_CSS: &str = include_str!("web/style.css");
        const HTTP_APP_JS: &str = include_str!("web/app.js");

        let host = get_host(&request);
        let response = match path {
            "/" | "/index.html" => {
                // Inject WebSocket configuration into the HTML
                let html = HTTP_INDEX_HTML
                    .replace("{{WS_HOST}}", &host)
                    .replace("{{WS_PORT}}", &ws_port.to_string())
                    .replace("{{WS_PROTOCOL}}", if ws_use_tls { "wss" } else { "ws" });
                build_response(200, "OK", "text/html", &html)
            }
            "/style.css" => {
                build_response(200, "OK", "text/css", HTTP_STYLE_CSS)
            }
            "/app.js" => {
                build_response(200, "OK", "application/javascript", HTTP_APP_JS)
            }
            "/favicon.ico" => {
                // Return 204 No Content for favicon requests (browsers/WebViews request this)
                build_response(204, "No Content", "image/x-icon", "")
            }
            _ => {
                // Log the 404 before recording violation
                log_http_404(&client_ip, path);
                // Record violation for accessing non-existent page
                ban_list.record_violation(&client_ip, path);
                build_response(404, "Not Found", "text/plain", "Not Found")
            }
        };

        let _ = stream.write_all(&response).await;
    }
}

/// Start the HTTP server (plain TCP, no TLS)
pub async fn start_http_server(
    server: &mut HttpServer,
    ws_port: u16,
    ws_use_tls: bool,
    ban_list: BanList,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("0.0.0.0:{}", server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await
        .map_err(|e| format!("Failed to bind HTTP to port {}: {}", server.port, e))?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let running = Arc::clone(&server.running);
    *running.write().await = true;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((mut stream, addr)) => {
                            let client_ip = addr.ip().to_string();
                            // Check if banned before processing
                            if ban_list.is_banned(&client_ip) {
                                // Send minimal response and close
                                let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 7\r\nConnection: close\r\n\r\nBanned\n").await;
                                continue;
                            }
                            // Disable Nagle's algorithm for lower latency
                            let _ = stream.set_nodelay(true);
                            let ban_list_clone = ban_list.clone();
                            tokio::spawn(async move {
                                handle_http_client(stream, ws_port, ws_use_tls, ban_list_clone, client_ip).await;
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
