// Module declarations
pub mod encoding;
pub mod telnet;
pub mod spell;
pub mod input;
pub mod util;
pub mod websocket;

// Re-export commonly used types from modules
pub use encoding::{Encoding, Theme, WorldSwitchMode, convert_discord_emojis, is_visually_empty, strip_non_sgr_sequences};
pub use telnet::{
    WriteCommand, StreamReader, StreamWriter, AutoConnectType, KeepAliveType,
    process_telnet, find_safe_split_point,
    TELNET_IAC, TELNET_NOP, TELNET_GA,
};
pub use spell::{SpellChecker, SpellState};
pub use input::InputArea;
pub use util::{get_binary_name, strip_ansi_codes, visual_line_count, get_current_time_12hr, strip_mud_tag, truncate_str};
pub use websocket::{
    WsMessage, WorldStateMsg, WorldSettingsMsg, GlobalSettingsMsg,
    WsClientInfo, WebSocketServer,
    hash_password, is_ip_in_allow_list, start_websocket_server,
};

use std::io::{self, stdout, BufRead, Write as IoWrite};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::time::Duration;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::BytesMut;
use crossterm::{
    cursor,
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, Clear, ClearType},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use regex::Regex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    signal::unix::{signal, SignalKind},
};
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};

// Rustls danger module for accepting invalid certificates (MUD servers often have self-signed certs)
#[cfg(feature = "rustls-backend")]
mod danger {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub struct NoCertificateVerification;

    impl NoCertificateVerification {
        pub fn new() -> Self {
            Self
        }
    }

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
            ]
        }
    }
}

// ============================================================================
// HTTPS Web Interface Server
// ============================================================================

/// HTTPS server state for the web interface
#[cfg(feature = "native-tls-backend")]
struct HttpsServer {
    running: Arc<RwLock<bool>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    port: u16,
}

#[cfg(feature = "native-tls-backend")]
impl HttpsServer {
    fn new(port: u16) -> Self {
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

/// Handle an HTTPS connection
#[cfg(feature = "native-tls-backend")]
async fn handle_https_client(
    mut stream: tokio_native_tls::TlsStream<TcpStream>,
    ws_port: u16,
    ws_use_tls: bool,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

        let response = match path {
            "/" | "/index.html" => {
                // Inject WebSocket configuration into the HTML
                let html = WEB_INDEX_HTML
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
            _ => {
                build_http_response(404, "Not Found", "text/plain", "Not Found")
            }
        };

        let _ = stream.write_all(&response).await;
    }
}

/// Start the HTTPS server
#[cfg(feature = "native-tls-backend")]
async fn start_https_server(
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
    let listener = TcpListener::bind(&addr).await?;

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
struct HttpsServer {
    running: Arc<RwLock<bool>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    port: u16,
}

#[cfg(feature = "rustls-backend")]
impl HttpsServer {
    fn new(port: u16) -> Self {
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

/// Handle an HTTPS connection (rustls version)
#[cfg(feature = "rustls-backend")]
async fn handle_https_client(
    mut stream: tokio_rustls::server::TlsStream<TcpStream>,
    ws_port: u16,
    ws_use_tls: bool,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

        let response = match path {
            "/" | "/index.html" => {
                // Inject WebSocket configuration into the HTML
                let html = WEB_INDEX_HTML
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
            _ => {
                build_http_response(404, "Not Found", "text/plain", "Not Found")
            }
        };

        let _ = stream.write_all(&response).await;
    }
}

/// Start the HTTPS server (rustls version)
#[cfg(feature = "rustls-backend")]
async fn start_https_server(
    server: &mut HttpsServer,
    cert_file: &str,
    key_file: &str,
    ws_port: u16,
    ws_use_tls: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::fs::File;
    use std::io::BufReader;
    use rustls_pemfile::{certs, private_key};

    // Read certificate chain
    let cert_file_handle = File::open(cert_file)
        .map_err(|e| format!("Failed to open cert file '{}': {}", cert_file, e))?;
    let mut cert_reader = BufReader::new(cert_file_handle);
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> = certs(&mut cert_reader)
        .filter_map(|r| r.ok())
        .collect();

    if certs.is_empty() {
        return Err(format!("No certificates found in cert file '{}'", cert_file).into());
    }

    // Read private key
    let key_file_handle = File::open(key_file)
        .map_err(|e| format!("Failed to open key file '{}': {}", key_file, e))?;
    let mut key_reader = BufReader::new(key_file_handle);
    let key = private_key(&mut key_reader)
        .map_err(|e| format!("Failed to parse key file '{}': {}", key_file, e))?
        .ok_or_else(|| format!("No private key found in key file '{}'", key_file))?;

    // Build TLS config
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("Failed to build TLS config: {}", e))?;

    let tls_acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

    let addr = format!("0.0.0.0:{}", server.port);
    let listener = TcpListener::bind(&addr).await
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
// HTTP Web Interface Server (no TLS)
// ============================================================================

/// HTTP server state for the web interface (no TLS)
struct HttpServer {
    running: Arc<RwLock<bool>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    port: u16,
}

impl HttpServer {
    fn new(port: u16) -> Self {
        Self {
            running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
            port,
        }
    }
}

/// Handle an HTTP connection (plain TCP, no TLS)
async fn handle_http_client(
    mut stream: TcpStream,
    ws_port: u16,
    ws_use_tls: bool,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

        let response = match path {
            "/" | "/index.html" => {
                // Inject WebSocket configuration into the HTML
                let html = HTTP_INDEX_HTML
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
            _ => {
                build_response(404, "Not Found", "text/plain", "Not Found")
            }
        };

        let _ = stream.write_all(&response).await;
    }
}

/// Start the HTTP server (plain TCP, no TLS)
async fn start_http_server(
    server: &mut HttpServer,
    ws_port: u16,
    ws_use_tls: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("0.0.0.0:{}", server.port);
    let listener = TcpListener::bind(&addr).await
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
                        Ok((stream, _addr)) => {
                            // Disable Nagle's algorithm for lower latency
                            let _ = stream.set_nodelay(true);
                            tokio::spawn(async move {
                                handle_http_client(stream, ws_port, ws_use_tls).await;
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


#[derive(Clone, Copy, PartialEq)]
enum SettingsField {
    // World-specific fields
    WorldName,
    Hostname,
    Port,
    User,
    Password,
    UseSsl,
    LogFile,
    Encoding,
    AutoConnect,
    KeepAlive,
    KeepAliveCmd,
    Connect,
    SaveWorld,
    CancelWorld,
    DeleteWorld,
    // Global settings (setup menu)
    MoreMode,
    SpellCheck,
    WorldSwitching,
    Debug,
    ShowTags,
    InputHeight,
    Theme,          // Console theme
    GuiTheme,       // GUI theme
    SaveSetup,
    CancelSetup,
}

impl SettingsField {
    fn is_text_field(&self) -> bool {
        matches!(
            self,
            SettingsField::WorldName
                | SettingsField::Hostname
                | SettingsField::Port
                | SettingsField::User
                | SettingsField::Password
                | SettingsField::LogFile
                | SettingsField::KeepAliveCmd
        )
    }

    fn is_button(&self) -> bool {
        matches!(self, SettingsField::Connect | SettingsField::SaveWorld | SettingsField::CancelWorld | SettingsField::DeleteWorld | SettingsField::SaveSetup | SettingsField::CancelSetup)
    }

    /// Next field for world settings popup (skip_keep_alive_cmd if keep_alive_type is not Custom)
    fn next_world(&self, skip_keep_alive_cmd: bool) -> Self {
        match self {
            SettingsField::WorldName => SettingsField::Hostname,
            SettingsField::Hostname => SettingsField::Port,
            SettingsField::Port => SettingsField::User,
            SettingsField::User => SettingsField::Password,
            SettingsField::Password => SettingsField::UseSsl,
            SettingsField::UseSsl => SettingsField::LogFile,
            SettingsField::LogFile => SettingsField::Encoding,
            SettingsField::Encoding => SettingsField::AutoConnect,
            SettingsField::AutoConnect => SettingsField::KeepAlive,
            SettingsField::KeepAlive => {
                if skip_keep_alive_cmd {
                    SettingsField::SaveWorld
                } else {
                    SettingsField::KeepAliveCmd
                }
            }
            SettingsField::KeepAliveCmd => SettingsField::SaveWorld,
            SettingsField::SaveWorld => SettingsField::CancelWorld,
            SettingsField::CancelWorld => SettingsField::DeleteWorld,
            SettingsField::DeleteWorld => SettingsField::Connect,
            SettingsField::Connect => SettingsField::WorldName,
            // Global fields wrap to world fields
            _ => SettingsField::WorldName,
        }
    }

    /// Previous field for world settings popup (skip_keep_alive_cmd if keep_alive_type is not Custom)
    fn prev_world(&self, skip_keep_alive_cmd: bool) -> Self {
        match self {
            SettingsField::WorldName => SettingsField::Connect,
            SettingsField::Hostname => SettingsField::WorldName,
            SettingsField::Port => SettingsField::Hostname,
            SettingsField::User => SettingsField::Port,
            SettingsField::Password => SettingsField::User,
            SettingsField::UseSsl => SettingsField::Password,
            SettingsField::LogFile => SettingsField::UseSsl,
            SettingsField::Encoding => SettingsField::LogFile,
            SettingsField::AutoConnect => SettingsField::Encoding,
            SettingsField::KeepAlive => SettingsField::AutoConnect,
            SettingsField::KeepAliveCmd => SettingsField::KeepAlive,
            SettingsField::SaveWorld => {
                if skip_keep_alive_cmd {
                    SettingsField::KeepAlive
                } else {
                    SettingsField::KeepAliveCmd
                }
            }
            SettingsField::CancelWorld => SettingsField::SaveWorld,
            SettingsField::DeleteWorld => SettingsField::CancelWorld,
            SettingsField::Connect => SettingsField::DeleteWorld,
            // Global fields wrap to world fields
            _ => SettingsField::Connect,
        }
    }

    /// Next field for setup (global) menu
    fn next_setup(&self) -> Self {
        match self {
            SettingsField::MoreMode => SettingsField::SpellCheck,
            SettingsField::SpellCheck => SettingsField::WorldSwitching,
            SettingsField::WorldSwitching => SettingsField::Debug,
            SettingsField::Debug => SettingsField::ShowTags,
            SettingsField::ShowTags => SettingsField::InputHeight,
            SettingsField::InputHeight => SettingsField::Theme,
            SettingsField::Theme => SettingsField::GuiTheme,
            SettingsField::GuiTheme => SettingsField::SaveSetup,
            SettingsField::SaveSetup => SettingsField::CancelSetup,
            SettingsField::CancelSetup => SettingsField::MoreMode,
            // World fields wrap to global fields
            _ => SettingsField::MoreMode,
        }
    }

    /// Previous field for setup (global) menu
    fn prev_setup(&self) -> Self {
        match self {
            SettingsField::MoreMode => SettingsField::CancelSetup,
            SettingsField::SpellCheck => SettingsField::MoreMode,
            SettingsField::WorldSwitching => SettingsField::SpellCheck,
            SettingsField::Debug => SettingsField::WorldSwitching,
            SettingsField::ShowTags => SettingsField::Debug,
            SettingsField::InputHeight => SettingsField::ShowTags,
            SettingsField::Theme => SettingsField::InputHeight,
            SettingsField::GuiTheme => SettingsField::Theme,
            SettingsField::SaveSetup => SettingsField::GuiTheme,
            SettingsField::CancelSetup => SettingsField::SaveSetup,
            // World fields wrap to global fields
            _ => SettingsField::CancelSetup,
        }
    }
}

struct SettingsPopup {
    visible: bool,
    selected_field: SettingsField,
    editing: bool,
    edit_buffer: String,
    edit_cursor: usize,
    edit_scroll_offset: usize, // Horizontal scroll offset for long text fields
    setup_mode: bool, // True for /setup (global only), false for /world (all settings)
    editing_world_index: Option<usize>, // Which world is being edited (None for setup mode)
    // Temp values for world-specific fields
    temp_world_name: String,
    temp_hostname: String,
    temp_port: String,
    temp_user: String,
    temp_password: String,
    temp_use_ssl: bool,
    temp_log_file: String,
    temp_encoding: Encoding,
    temp_auto_connect_type: AutoConnectType,
    temp_keep_alive_type: KeepAliveType,
    temp_keep_alive_cmd: String,
    // Temp values for global settings
    temp_more_mode: bool,
    temp_spell_check: bool,
    temp_world_switch_mode: WorldSwitchMode,
    temp_debug_enabled: bool,
    temp_show_tags: bool,
    temp_input_height: u16,
    temp_theme: Theme,
    temp_gui_theme: Theme,
}

impl SettingsPopup {
    fn new() -> Self {
        Self {
            visible: false,
            selected_field: SettingsField::WorldName,
            editing: false,
            edit_buffer: String::new(),
            edit_cursor: 0,
            edit_scroll_offset: 0,
            setup_mode: false,
            editing_world_index: None,
            temp_world_name: String::new(),
            temp_hostname: String::new(),
            temp_port: String::new(),
            temp_user: String::new(),
            temp_password: String::new(),
            temp_use_ssl: false,
            temp_log_file: String::new(),
            temp_encoding: Encoding::Utf8,
            temp_auto_connect_type: AutoConnectType::Connect,
            temp_keep_alive_type: KeepAliveType::Nop,
            temp_keep_alive_cmd: String::new(),
            temp_more_mode: true,
            temp_spell_check: true,
            temp_world_switch_mode: WorldSwitchMode::UnseenFirst,
            temp_debug_enabled: false,
            temp_show_tags: false,
            temp_input_height: 3,
            temp_theme: Theme::Dark,
            temp_gui_theme: Theme::Dark,
        }
    }

    fn open(&mut self, settings: &Settings, world: &World, world_index: usize, input_height: u16, show_tags: bool) {
        self.visible = true;
        self.setup_mode = false;
        self.editing_world_index = Some(world_index);
        self.selected_field = SettingsField::WorldName;
        self.editing = false;
        // Load from world settings
        self.temp_world_name = world.name.clone();
        self.temp_hostname = world.settings.hostname.clone();
        self.temp_port = world.settings.port.clone();
        self.temp_user = world.settings.user.clone();
        self.temp_password = world.settings.password.clone();
        self.temp_use_ssl = world.settings.use_ssl;
        self.temp_log_file = world.settings.log_file.clone().unwrap_or_default();
        self.temp_encoding = world.settings.encoding;
        self.temp_auto_connect_type = world.settings.auto_connect_type;
        self.temp_keep_alive_type = world.settings.keep_alive_type;
        self.temp_keep_alive_cmd = world.settings.keep_alive_cmd.clone();
        // Load from global settings
        self.temp_more_mode = settings.more_mode_enabled;
        self.temp_spell_check = settings.spell_check_enabled;
        self.temp_world_switch_mode = settings.world_switch_mode;
        self.temp_show_tags = show_tags;
        self.temp_input_height = input_height;
    }

    fn open_setup(&mut self, settings: &Settings, input_height: u16, show_tags: bool) {
        self.visible = true;
        self.setup_mode = true;
        self.editing_world_index = None;
        self.selected_field = SettingsField::MoreMode;
        self.editing = false;
        // Load from global settings only
        self.temp_more_mode = settings.more_mode_enabled;
        self.temp_spell_check = settings.spell_check_enabled;
        self.temp_world_switch_mode = settings.world_switch_mode;
        self.temp_debug_enabled = settings.debug_enabled;
        self.temp_show_tags = show_tags;
        self.temp_input_height = input_height;
        self.temp_theme = settings.theme;
        self.temp_gui_theme = settings.gui_theme;
    }

    fn close(&mut self) {
        self.visible = false;
        self.editing = false;
    }

    fn next_field(&mut self) {
        if self.setup_mode {
            self.selected_field = self.selected_field.next_setup();
        } else {
            let skip_cmd = self.temp_keep_alive_type != KeepAliveType::Custom;
            self.selected_field = self.selected_field.next_world(skip_cmd);
        }
    }

    fn prev_field(&mut self) {
        if self.setup_mode {
            self.selected_field = self.selected_field.prev_setup();
        } else {
            let skip_cmd = self.temp_keep_alive_type != KeepAliveType::Custom;
            self.selected_field = self.selected_field.prev_world(skip_cmd);
        }
    }

    fn start_edit(&mut self) {
        self.editing = true;
        self.edit_buffer = match self.selected_field {
            SettingsField::WorldName => self.temp_world_name.clone(),
            SettingsField::Hostname => self.temp_hostname.clone(),
            SettingsField::Port => self.temp_port.clone(),
            SettingsField::User => self.temp_user.clone(),
            SettingsField::Password => self.temp_password.clone(),
            SettingsField::LogFile => self.temp_log_file.clone(),
            SettingsField::KeepAliveCmd => self.temp_keep_alive_cmd.clone(),
            _ => String::new(),
        };
        self.edit_cursor = self.edit_buffer.len();
        self.edit_scroll_offset = 0; // Reset scroll when starting edit
    }

    /// Adjust scroll offset to keep cursor visible within given visible width
    fn adjust_scroll(&mut self, visible_width: usize) {
        if visible_width == 0 {
            return;
        }
        // Keep cursor visible with some margin
        let margin = 2.min(visible_width / 4);
        if self.edit_cursor < self.edit_scroll_offset + margin {
            // Cursor is before visible area, scroll left
            self.edit_scroll_offset = self.edit_cursor.saturating_sub(margin);
        } else if self.edit_cursor >= self.edit_scroll_offset + visible_width - margin {
            // Cursor is after visible area, scroll right
            self.edit_scroll_offset = self.edit_cursor.saturating_sub(visible_width - margin - 1);
        }
    }

    fn commit_edit(&mut self) {
        match self.selected_field {
            SettingsField::WorldName => self.temp_world_name = self.edit_buffer.clone(),
            SettingsField::Hostname => self.temp_hostname = self.edit_buffer.clone(),
            SettingsField::Port => self.temp_port = self.edit_buffer.clone(),
            SettingsField::User => self.temp_user = self.edit_buffer.clone(),
            SettingsField::Password => self.temp_password = self.edit_buffer.clone(),
            SettingsField::LogFile => self.temp_log_file = self.edit_buffer.clone(),
            SettingsField::KeepAliveCmd => self.temp_keep_alive_cmd = self.edit_buffer.clone(),
            _ => {}
        }
        self.editing = false;
    }

    fn cancel_edit(&mut self) {
        self.editing = false;
    }

    fn toggle_or_cycle(&mut self) {
        match self.selected_field {
            SettingsField::UseSsl => self.temp_use_ssl = !self.temp_use_ssl,
            SettingsField::MoreMode => self.temp_more_mode = !self.temp_more_mode,
            SettingsField::SpellCheck => self.temp_spell_check = !self.temp_spell_check,
            SettingsField::WorldSwitching => self.temp_world_switch_mode = self.temp_world_switch_mode.next(),
            SettingsField::Debug => self.temp_debug_enabled = !self.temp_debug_enabled,
            SettingsField::ShowTags => self.temp_show_tags = !self.temp_show_tags,
            SettingsField::InputHeight => {
                // Cycle through 1-15
                self.temp_input_height = if self.temp_input_height >= 15 {
                    1
                } else {
                    self.temp_input_height + 1
                };
            }
            SettingsField::Theme => {
                self.temp_theme = self.temp_theme.next();
            }
            SettingsField::GuiTheme => {
                self.temp_gui_theme = self.temp_gui_theme.next();
            }
            SettingsField::Encoding => {
                self.temp_encoding = match self.temp_encoding {
                    Encoding::Utf8 => Encoding::Latin1,
                    Encoding::Latin1 => Encoding::Fansi,
                    Encoding::Fansi => Encoding::Utf8,
                };
            }
            SettingsField::AutoConnect => {
                self.temp_auto_connect_type = self.temp_auto_connect_type.next();
            }
            SettingsField::KeepAlive => {
                self.temp_keep_alive_type = self.temp_keep_alive_type.next();
            }
            _ => {}
        }
    }

    /// Apply settings and return (input_height, show_tags)
    fn apply(&self, settings: &mut Settings, world: &mut World) -> (u16, bool) {
        // Apply global settings
        settings.more_mode_enabled = self.temp_more_mode;
        settings.spell_check_enabled = self.temp_spell_check;
        settings.world_switch_mode = self.temp_world_switch_mode;
        settings.theme = self.temp_theme;
        // Apply world-specific settings (only in world mode)
        if !self.setup_mode {
            world.name = self.temp_world_name.clone();
            world.settings.hostname = self.temp_hostname.clone();
            world.settings.port = self.temp_port.clone();
            world.settings.user = self.temp_user.clone();
            world.settings.password = self.temp_password.clone();
            world.settings.use_ssl = self.temp_use_ssl;
            world.settings.log_file = if self.temp_log_file.is_empty() {
                None
            } else {
                Some(self.temp_log_file.clone())
            };
            world.settings.encoding = self.temp_encoding;
            world.settings.auto_connect_type = self.temp_auto_connect_type;
            world.settings.keep_alive_type = self.temp_keep_alive_type;
            world.settings.keep_alive_cmd = self.temp_keep_alive_cmd.clone();
        }
        (self.temp_input_height, self.temp_show_tags)
    }

    fn apply_global(&self, settings: &mut Settings) -> (u16, bool) {
        // Apply only global settings (for setup mode)
        settings.more_mode_enabled = self.temp_more_mode;
        settings.spell_check_enabled = self.temp_spell_check;
        settings.world_switch_mode = self.temp_world_switch_mode;
        settings.debug_enabled = self.temp_debug_enabled;
        settings.theme = self.temp_theme;
        settings.gui_theme = self.temp_gui_theme;
        (self.temp_input_height, self.temp_show_tags)
    }
}

// ============================================================================
// Web Settings Popup (/web command)
// ============================================================================

#[derive(Clone, Copy, PartialEq, Debug)]
enum WebField {
    // Protocol selection (Secure/Non-Secure)
    Protocol,
    // HTTP/HTTPS server
    HttpEnabled,
    HttpPort,
    // WebSocket server
    WsEnabled,
    WsPort,
    WsPassword,
    WsAllowList,
    // TLS settings (only shown when Protocol is Secure)
    WsCertFile,
    WsKeyFile,
    // Buttons
    SaveWeb,
    CancelWeb,
}

impl WebField {
    fn is_text_field(&self) -> bool {
        matches!(
            self,
            WebField::HttpPort
                | WebField::WsPort
                | WebField::WsPassword
                | WebField::WsAllowList
                | WebField::WsCertFile
                | WebField::WsKeyFile
        )
    }

    fn is_button(&self) -> bool {
        matches!(self, WebField::SaveWeb | WebField::CancelWeb)
    }

    /// Get next field, skipping TLS fields when not secure
    fn next(&self, secure: bool) -> Self {
        match self {
            WebField::Protocol => WebField::HttpEnabled,
            WebField::HttpEnabled => WebField::HttpPort,
            WebField::HttpPort => WebField::WsEnabled,
            WebField::WsEnabled => WebField::WsPort,
            WebField::WsPort => WebField::WsPassword,
            WebField::WsPassword => WebField::WsAllowList,
            WebField::WsAllowList => {
                if secure { WebField::WsCertFile } else { WebField::SaveWeb }
            }
            WebField::WsCertFile => WebField::WsKeyFile,
            WebField::WsKeyFile => WebField::SaveWeb,
            WebField::SaveWeb => WebField::CancelWeb,
            WebField::CancelWeb => WebField::Protocol,
        }
    }

    /// Get previous field, skipping TLS fields when not secure
    fn prev(&self, secure: bool) -> Self {
        match self {
            WebField::Protocol => WebField::CancelWeb,
            WebField::HttpEnabled => WebField::Protocol,
            WebField::HttpPort => WebField::HttpEnabled,
            WebField::WsEnabled => WebField::HttpPort,
            WebField::WsPort => WebField::WsEnabled,
            WebField::WsPassword => WebField::WsPort,
            WebField::WsAllowList => WebField::WsPassword,
            WebField::WsCertFile => WebField::WsAllowList,
            WebField::WsKeyFile => WebField::WsCertFile,
            WebField::SaveWeb => {
                if secure { WebField::WsKeyFile } else { WebField::WsAllowList }
            }
            WebField::CancelWeb => WebField::SaveWeb,
        }
    }
}

struct WebPopup {
    visible: bool,
    selected_field: WebField,
    editing: bool,
    edit_buffer: String,
    edit_cursor: usize,
    edit_scroll_offset: usize,
    // Temp values for web settings (consolidated)
    temp_web_secure: bool,     // Protocol: true=Secure, false=Non-Secure
    temp_http_enabled: bool,
    temp_http_port: String,
    temp_ws_enabled: bool,
    temp_ws_port: String,
    temp_ws_password: String,
    temp_ws_allow_list: String,
    temp_ws_cert_file: String,
    temp_ws_key_file: String,
}

impl WebPopup {
    fn new() -> Self {
        Self {
            visible: false,
            selected_field: WebField::Protocol,
            editing: false,
            edit_buffer: String::new(),
            edit_cursor: 0,
            edit_scroll_offset: 0,
            temp_web_secure: false,
            temp_http_enabled: false,
            temp_http_port: "9000".to_string(),
            temp_ws_enabled: false,
            temp_ws_port: "9001".to_string(),
            temp_ws_password: String::new(),
            temp_ws_allow_list: String::new(),
            temp_ws_cert_file: String::new(),
            temp_ws_key_file: String::new(),
        }
    }

    fn open(&mut self, settings: &Settings) {
        self.visible = true;
        self.selected_field = WebField::Protocol;
        self.editing = false;
        // Load from settings
        self.temp_web_secure = settings.web_secure;
        self.temp_http_enabled = settings.http_enabled;
        self.temp_http_port = settings.http_port.to_string();
        self.temp_ws_enabled = settings.ws_enabled;
        self.temp_ws_port = settings.ws_port.to_string();
        self.temp_ws_password = settings.websocket_password.clone();
        self.temp_ws_allow_list = settings.websocket_allow_list.clone();
        self.temp_ws_cert_file = settings.websocket_cert_file.clone();
        self.temp_ws_key_file = settings.websocket_key_file.clone();
    }

    fn close(&mut self) {
        self.visible = false;
        self.editing = false;
    }

    fn next_field(&mut self) {
        self.selected_field = self.selected_field.next(self.temp_web_secure);
    }

    fn prev_field(&mut self) {
        self.selected_field = self.selected_field.prev(self.temp_web_secure);
    }

    fn start_edit(&mut self) {
        self.editing = true;
        self.edit_buffer = match self.selected_field {
            WebField::HttpPort => self.temp_http_port.clone(),
            WebField::WsPort => self.temp_ws_port.clone(),
            WebField::WsPassword => self.temp_ws_password.clone(),
            WebField::WsAllowList => self.temp_ws_allow_list.clone(),
            WebField::WsCertFile => self.temp_ws_cert_file.clone(),
            WebField::WsKeyFile => self.temp_ws_key_file.clone(),
            _ => String::new(),
        };
        self.edit_cursor = self.edit_buffer.len();
        self.edit_scroll_offset = 0;
    }

    fn commit_edit(&mut self) {
        match self.selected_field {
            WebField::HttpPort => self.temp_http_port = self.edit_buffer.clone(),
            WebField::WsPort => self.temp_ws_port = self.edit_buffer.clone(),
            WebField::WsPassword => self.temp_ws_password = self.edit_buffer.clone(),
            WebField::WsAllowList => self.temp_ws_allow_list = self.edit_buffer.clone(),
            WebField::WsCertFile => self.temp_ws_cert_file = self.edit_buffer.clone(),
            WebField::WsKeyFile => self.temp_ws_key_file = self.edit_buffer.clone(),
            _ => {}
        }
        self.editing = false;
    }

    fn cancel_edit(&mut self) {
        self.editing = false;
    }

    fn toggle_option(&mut self) {
        match self.selected_field {
            WebField::Protocol => self.temp_web_secure = !self.temp_web_secure,
            WebField::HttpEnabled => self.temp_http_enabled = !self.temp_http_enabled,
            WebField::WsEnabled => self.temp_ws_enabled = !self.temp_ws_enabled,
            _ => {}
        }
    }

    /// Adjust scroll offset to keep cursor visible within given visible width
    fn adjust_scroll(&mut self, visible_width: usize) {
        if visible_width == 0 {
            return;
        }
        let margin = 2.min(visible_width / 4);
        if self.edit_cursor < self.edit_scroll_offset + margin {
            self.edit_scroll_offset = self.edit_cursor.saturating_sub(margin);
        } else if self.edit_cursor >= self.edit_scroll_offset + visible_width - margin {
            self.edit_scroll_offset = self.edit_cursor.saturating_sub(visible_width - margin - 1);
        }
    }

    fn apply(&self, settings: &mut Settings) {
        settings.web_secure = self.temp_web_secure;
        settings.http_enabled = self.temp_http_enabled;
        if let Ok(port) = self.temp_http_port.parse::<u16>() {
            settings.http_port = port;
        }
        settings.ws_enabled = self.temp_ws_enabled;
        if let Ok(port) = self.temp_ws_port.parse::<u16>() {
            settings.ws_port = port;
        }
        settings.websocket_password = self.temp_ws_password.clone();
        settings.websocket_allow_list = self.temp_ws_allow_list.clone();
        settings.websocket_cert_file = self.temp_ws_cert_file.clone();
        settings.websocket_key_file = self.temp_ws_key_file.clone();
    }
}

#[derive(Clone, Copy, PartialEq)]
enum WorldSelectorFocus {
    List,
    AddButton,
    EditButton,
    ConnectButton,
    CancelButton,
}

struct WorldSelectorPopup {
    visible: bool,
    selected_index: usize,
    filter: String,
    filter_cursor: usize,
    editing_filter: bool,
    focus: WorldSelectorFocus,
}

impl WorldSelectorPopup {
    fn new() -> Self {
        Self {
            visible: false,
            selected_index: 0,
            filter: String::new(),
            filter_cursor: 0,
            editing_filter: false,
            focus: WorldSelectorFocus::List,
        }
    }

    fn open(&mut self, current_world_index: usize) {
        self.visible = true;
        self.selected_index = current_world_index;
        self.filter.clear();
        self.filter_cursor = 0;
        self.editing_filter = false;
        self.focus = WorldSelectorFocus::List;
    }

    fn close(&mut self) {
        self.visible = false;
        self.editing_filter = false;
    }

    fn next_focus(&mut self) {
        self.focus = match self.focus {
            WorldSelectorFocus::List => WorldSelectorFocus::AddButton,
            WorldSelectorFocus::AddButton => WorldSelectorFocus::EditButton,
            WorldSelectorFocus::EditButton => WorldSelectorFocus::ConnectButton,
            WorldSelectorFocus::ConnectButton => WorldSelectorFocus::CancelButton,
            WorldSelectorFocus::CancelButton => WorldSelectorFocus::List,
        };
    }

    fn prev_focus(&mut self) {
        self.focus = match self.focus {
            WorldSelectorFocus::List => WorldSelectorFocus::CancelButton,
            WorldSelectorFocus::AddButton => WorldSelectorFocus::List,
            WorldSelectorFocus::EditButton => WorldSelectorFocus::AddButton,
            WorldSelectorFocus::ConnectButton => WorldSelectorFocus::EditButton,
            WorldSelectorFocus::CancelButton => WorldSelectorFocus::ConnectButton,
        };
    }

    /// Get indices of worlds matching the filter
    fn filtered_indices(&self, worlds: &[World]) -> Vec<usize> {
        if self.filter.is_empty() {
            (0..worlds.len()).collect()
        } else {
            let filter_lower = self.filter.to_lowercase();
            worlds
                .iter()
                .enumerate()
                .filter(|(_, w)| {
                    w.name.to_lowercase().contains(&filter_lower)
                        || w.settings.hostname.to_lowercase().contains(&filter_lower)
                        || w.settings.user.to_lowercase().contains(&filter_lower)
                })
                .map(|(i, _)| i)
                .collect()
        }
    }

    /// Move up in the list. Returns true if at top (should go to buttons).
    fn move_up(&mut self, worlds: &[World]) -> bool {
        let indices = self.filtered_indices(worlds);
        if indices.is_empty() {
            return true;
        }
        // Find current position in filtered list
        if let Some(pos) = indices.iter().position(|&i| i == self.selected_index) {
            if pos > 0 {
                self.selected_index = indices[pos - 1];
                false
            } else {
                true // At top, signal to go to buttons
            }
        } else if !indices.is_empty() {
            self.selected_index = indices[0];
            false
        } else {
            true
        }
    }

    /// Move down in the list. Returns true if at bottom (should go to buttons).
    fn move_down(&mut self, worlds: &[World]) -> bool {
        let indices = self.filtered_indices(worlds);
        if indices.is_empty() {
            return true;
        }
        // Find current position in filtered list
        if let Some(pos) = indices.iter().position(|&i| i == self.selected_index) {
            if pos < indices.len() - 1 {
                self.selected_index = indices[pos + 1];
                false
            } else {
                true // At bottom, signal to go to buttons
            }
        } else if !indices.is_empty() {
            self.selected_index = indices[0];
            false
        } else {
            true
        }
    }

    /// Move to the last item in the filtered list
    fn move_to_last(&mut self, worlds: &[World]) {
        let indices = self.filtered_indices(worlds);
        if !indices.is_empty() {
            self.selected_index = indices[indices.len() - 1];
        }
    }

    /// Move to the first item in the filtered list
    fn move_to_first(&mut self, worlds: &[World]) {
        let indices = self.filtered_indices(worlds);
        if !indices.is_empty() {
            self.selected_index = indices[0];
        }
    }

    fn start_filter_edit(&mut self) {
        self.editing_filter = true;
        self.filter_cursor = self.filter.len();
    }

    fn stop_filter_edit(&mut self) {
        self.editing_filter = false;
    }
}

struct ConfirmDialog {
    visible: bool,
    message: String,
    yes_selected: bool,
    action: ConfirmAction,
}

#[derive(Clone, Copy, PartialEq)]
enum ConfirmAction {
    None,
    DeleteWorld(usize), // world index to delete
}

impl ConfirmDialog {
    fn new() -> Self {
        Self {
            visible: false,
            message: String::new(),
            yes_selected: false,
            action: ConfirmAction::None,
        }
    }

    fn show_delete_world(&mut self, world_name: &str, world_index: usize) {
        self.visible = true;
        self.message = format!("Delete world '{}'?", world_name);
        self.yes_selected = false; // Default to No for safety
        self.action = ConfirmAction::DeleteWorld(world_index);
    }

    fn close(&mut self) {
        self.visible = false;
        self.action = ConfirmAction::None;
    }
}

/// Popup to display connected worlds status (from /worlds command)
struct WorldsPopup {
    visible: bool,
    lines: Vec<String>,
}

impl WorldsPopup {
    fn new() -> Self {
        Self {
            visible: false,
            lines: Vec::new(),
        }
    }

    fn show(&mut self, worlds: &[World], current_world_index: usize, screen_width: u16) {
        self.visible = true;
        self.lines.clear();

        // Helper to format elapsed time
        fn format_elapsed(instant: Option<std::time::Instant>) -> String {
            match instant {
                None => "-".to_string(),
                Some(t) => {
                    let secs = t.elapsed().as_secs();
                    if secs < 60 {
                        format!("{}s", secs)
                    } else if secs < 3600 {
                        format!("{}m", secs / 60)
                    } else if secs < 86400 {
                        format!("{}h", secs / 3600)
                    } else {
                        format!("{}d", secs / 86400)
                    }
                }
            }
        }

        // Helper to format time until next NOP
        fn format_next_nop(last_send_time: Option<std::time::Instant>, last_receive_time: Option<std::time::Instant>) -> String {
            const KEEPALIVE_SECS: u64 = 5 * 60;
            let last_activity = match (last_send_time, last_receive_time) {
                (Some(s), Some(r)) => Some(s.max(r)),
                (Some(s), None) => Some(s),
                (None, Some(r)) => Some(r),
                (None, None) => None,
            };
            let elapsed = last_activity.map(|t| t.elapsed().as_secs()).unwrap_or(KEEPALIVE_SECS);
            let remaining = KEEPALIVE_SECS.saturating_sub(elapsed);
            if remaining < 60 {
                format!("{}s", remaining)
            } else {
                format!("{}m", remaining / 60)
            }
        }

        // Collect connected world info
        let connected_info: Vec<_> = worlds.iter().enumerate()
            .filter(|(_, w)| w.connected)
            .map(|(i, w)| {
                (
                    if i == current_world_index { "*" } else { " " },
                    w.name.clone(),
                    if w.unseen_lines > 0 { w.unseen_lines.to_string() } else { String::new() },
                    format_elapsed(w.last_user_command_time),
                    format_elapsed(w.last_receive_time),
                    w.settings.keep_alive_type.name().to_string(),
                    format_elapsed(w.last_nop_time),
                    format_next_nop(w.last_send_time, w.last_receive_time),
                )
            })
            .collect();

        if connected_info.is_empty() {
            self.lines.push("No worlds connected.".to_string());
        } else {
            // Calculate column widths
            let name_width = connected_info.iter().map(|(_, n, _, _, _, _, _, _)| n.len()).max().unwrap_or(5).max(5);
            let unseen_width = connected_info.iter().map(|(_, _, u, _, _, _, _, _)| u.len()).max().unwrap_or(6).max(6);
            let send_width = connected_info.iter().map(|(_, _, _, s, _, _, _, _)| s.len()).max().unwrap_or(8).max(8);
            let recv_width = connected_info.iter().map(|(_, _, _, _, r, _, _, _)| r.len()).max().unwrap_or(8).max(8);
            let ka_width = connected_info.iter().map(|(_, _, _, _, _, k, _, _)| k.len()).max().unwrap_or(9).max(9);
            let nop_width = connected_info.iter().map(|(_, _, _, _, _, _, n, _)| n.len()).max().unwrap_or(6).max(6);
            let next_width = connected_info.iter().map(|(_, _, _, _, _, _, _, p)| p.len()).max().unwrap_or(6).max(6);

            // Calculate combined column widths
            let send_recv_combined_width = connected_info.iter()
                .map(|(_, _, _, s, r, _, _, _)| format!("{}/{}", s, r).len())
                .max().unwrap_or(9).max(9);
            let ka_next_combined_width = connected_info.iter()
                .map(|(_, _, _, _, _, _, lk, nk)| format!("{}/{}", lk, nk).len())
                .max().unwrap_or(7).max(7);

            // Calculate total widths for different layouts
            // Layout 1: All columns separate
            let width_full = 2 + name_width + 2 + unseen_width + 2 + send_width + 2 + recv_width + 2 + ka_width + 2 + nop_width + 2 + next_width;
            // Layout 2: Combine Send/Recv AND LastKA/NextKA
            let width_combined_both = 2 + name_width + 2 + unseen_width + 2 + send_recv_combined_width + 2 + ka_width + 2 + ka_next_combined_width;
            // Layout 3: Remove KeepAlive column
            let width_no_ka_type = 2 + name_width + 2 + unseen_width + 2 + send_recv_combined_width + 2 + ka_next_combined_width;
            // Layout 4: Remove LastKA/NextKA columns entirely
            let width_minimal = 2 + name_width + 2 + unseen_width + 2 + send_recv_combined_width;

            let available = screen_width as usize;

            // Determine which layout to use
            let layout = if available >= width_full {
                1  // Full layout
            } else if available >= width_combined_both {
                2  // Combined Send/Recv and LastKA/NextKA
            } else if available >= width_no_ka_type {
                3  // Remove KeepAlive type column
            } else if available >= width_minimal {
                4  // Remove KA columns entirely
            } else {
                4  // Fallback to minimal
            };

            // Generate header and data based on layout
            match layout {
                1 => {
                    // Full layout: all columns separate
                    self.lines.push(format!(
                        "  {:name_width$}  {:>unseen_width$}  {:>send_width$}  {:>recv_width$}  {:ka_width$}  {:>nop_width$}  {:>next_width$}",
                        "World", "Unseen", "LastSend", "LastRecv", "KeepAlive", "LastKA", "NextKA",
                    ));
                    for (current, name, unseen, send, recv, ka_type, last_ka, next_ka) in &connected_info {
                        self.lines.push(format!(
                            "{} {:name_width$}  {:>unseen_width$}  {:>send_width$}  {:>recv_width$}  {:ka_width$}  {:>nop_width$}  {:>next_width$}",
                            current, name, unseen, send, recv, ka_type, last_ka, next_ka,
                        ));
                    }
                }
                2 => {
                    // Combined Send/Recv and LastKA/NextKA
                    self.lines.push(format!(
                        "  {:name_width$}  {:>unseen_width$}  {:>send_recv_combined_width$}  {:ka_width$}  {:>ka_next_combined_width$}",
                        "World", "Unseen", "Send/Recv", "KeepAlive", "KA/Next",
                    ));
                    for (current, name, unseen, send, recv, ka_type, last_ka, next_ka) in &connected_info {
                        let sr = format!("{}/{}", send, recv);
                        let kn = format!("{}/{}", last_ka, next_ka);
                        self.lines.push(format!(
                            "{} {:name_width$}  {:>unseen_width$}  {:>send_recv_combined_width$}  {:ka_width$}  {:>ka_next_combined_width$}",
                            current, name, unseen, sr, ka_type, kn,
                        ));
                    }
                }
                3 => {
                    // Remove KeepAlive type column
                    self.lines.push(format!(
                        "  {:name_width$}  {:>unseen_width$}  {:>send_recv_combined_width$}  {:>ka_next_combined_width$}",
                        "World", "Unseen", "Send/Recv", "KA/Next",
                    ));
                    for (current, name, unseen, send, recv, _ka_type, last_ka, next_ka) in &connected_info {
                        let sr = format!("{}/{}", send, recv);
                        let kn = format!("{}/{}", last_ka, next_ka);
                        self.lines.push(format!(
                            "{} {:name_width$}  {:>unseen_width$}  {:>send_recv_combined_width$}  {:>ka_next_combined_width$}",
                            current, name, unseen, sr, kn,
                        ));
                    }
                }
                _ => {
                    // Minimal: Remove KA columns entirely
                    self.lines.push(format!(
                        "  {:name_width$}  {:>unseen_width$}  {:>send_recv_combined_width$}",
                        "World", "Unseen", "Send/Recv",
                    ));
                    for (current, name, unseen, send, recv, _ka_type, _last_ka, _next_ka) in &connected_info {
                        let sr = format!("{}/{}", send, recv);
                        self.lines.push(format!(
                            "{} {:name_width$}  {:>unseen_width$}  {:>send_recv_combined_width$}",
                            current, name, unseen, sr,
                        ));
                    }
                }
            }
        }
    }

    fn close(&mut self) {
        self.visible = false;
        self.lines.clear();
    }
}

struct FilterPopup {
    visible: bool,
    filter_text: String,
    cursor: usize,
    filtered_indices: Vec<usize>,  // Indices of matching lines in output_lines
    scroll_offset: usize,          // Scroll position within filtered results
}

impl FilterPopup {
    fn new() -> Self {
        Self {
            visible: false,
            filter_text: String::new(),
            cursor: 0,
            filtered_indices: Vec::new(),
            scroll_offset: 0,
        }
    }

    fn open(&mut self) {
        self.visible = true;
        self.filter_text.clear();
        self.cursor = 0;
        self.filtered_indices.clear();
        self.scroll_offset = 0;
    }

    fn close(&mut self) {
        self.visible = false;
        self.filter_text.clear();
        self.filtered_indices.clear();
        self.scroll_offset = 0;
    }

    fn update_filter(&mut self, output_lines: &[String]) {
        if self.filter_text.is_empty() {
            self.filtered_indices = (0..output_lines.len()).collect();
        } else {
            let filter_lower = self.filter_text.to_lowercase();
            self.filtered_indices = output_lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    // Strip ANSI codes for matching
                    let plain = strip_ansi_codes(line);
                    plain.to_lowercase().contains(&filter_lower)
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Reset scroll to end (most recent matches)
        self.scroll_offset = self.filtered_indices.len().saturating_sub(1);
    }
}

struct HelpPopup {
    visible: bool,
    scroll_offset: usize,
    lines: Vec<&'static str>,
}

impl HelpPopup {
    fn new() -> Self {
        Self {
            visible: false,
            scroll_offset: 0,
            lines: vec![
                "Commands:",
                "  /help                      Show this help",
                "  /connect [host port [ssl]] Connect to server",
                "  /disconnect (or /dc)       Disconnect from server",
                "  /send [-W] [-w<world>] [-n] <text>",
                "                             Send text to world(s)",
                "  /world                     Open world selector",
                "  /world <name>              Connect to or create world",
                "  /world -e [name]           Edit world settings",
                "  /world -l <name>           Connect without auto-login",
                "  /worlds (or /l)            List connected worlds",
                "  /keepalive                 Show keepalive settings for all worlds",
                "  /actions                   Open actions/triggers editor",
                "  /setup                     Open global settings",
                "  /reload                    Hot reload binary",
                "  /quit                      Exit client",
                "",
                "World Switching:",
                "  Up/Down                    Switch worlds",
                "",
                "Input:",
                "  Left/Right, Ctrl+B/F       Move cursor",
                "  Ctrl+Up/Down               Resize input area",
                "  Ctrl+U                     Clear input",
                "  Ctrl+W                     Delete word",
                "  Ctrl+P/N                   Command history",
                "  Ctrl+Q                     Spell suggestions",
                "  Home/End                   Jump to start/end",
                "",
                "Output:",
                "  PageUp/PageDown            Scroll output",
                "  Tab                        Release one screenful",
                "  Alt+j                      Jump to end",
                "  Alt+w                      Switch to oldest pending world",
                "  F4                         Filter output",
                "",
                "General:",
                "  F1                         Show this help",
                "  F2                         Toggle MUD tag display",
                "  Ctrl+L                     Redraw screen",
                "  Ctrl+R                     Hot reload",
                "  Ctrl+C (x2)                Quit",
            ],
        }
    }

    fn open(&mut self) {
        self.visible = true;
        self.scroll_offset = 0;
    }

    fn close(&mut self) {
        self.visible = false;
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    fn scroll_down(&mut self, visible_height: usize) {
        let max_offset = self.lines.len().saturating_sub(visible_height);
        if self.scroll_offset < max_offset {
            self.scroll_offset += 1;
        }
    }
}

/// Which view the actions popup is showing
#[derive(Clone, Copy, PartialEq)]
enum ActionsView {
    List,           // List of actions with Add/Edit/Delete/Cancel
    Editor,         // Editing a single action
    ConfirmDelete,  // Confirming deletion
}

/// Which field in the action list view is focused
#[derive(Clone, Copy, PartialEq)]
enum ActionListField {
    List,           // Action list (selecting which action)
    AddButton,
    EditButton,
    DeleteButton,
    CancelButton,
}

/// Which field in the action editor is focused
#[derive(Clone, Copy, PartialEq)]
enum ActionEditorField {
    Name,
    World,
    Pattern,
    Command,
    SaveButton,
    CancelButton,
}

/// Popup for editing actions/triggers
struct ActionsPopup {
    visible: bool,
    view: ActionsView,              // Current view mode
    actions: Vec<Action>,           // Working copy of actions
    selected_index: usize,          // Currently selected action in list
    editing_index: Option<usize>,   // Index being edited (None = new action)
    list_field: ActionListField,    // Current field in list view
    editor_field: ActionEditorField, // Current field in editor view
    confirm_selected: bool,         // Yes (true) or No (false) in confirm dialog
    scroll_offset: usize,           // Scroll offset for action list
    // Editing state for current action
    edit_name: String,
    edit_world: String,
    edit_pattern: String,
    edit_command: String,
    cursor_pos: usize,              // Cursor position in current text field
    error_message: Option<String>,  // Validation error message
    command_expanded: bool,         // Whether command field is expanded to show all lines
}

impl ActionsPopup {
    fn new() -> Self {
        Self {
            visible: false,
            view: ActionsView::List,
            actions: Vec::new(),
            selected_index: 0,
            editing_index: None,
            list_field: ActionListField::List,
            editor_field: ActionEditorField::Name,
            confirm_selected: false,
            scroll_offset: 0,
            edit_name: String::new(),
            edit_world: String::new(),
            edit_pattern: String::new(),
            edit_command: String::new(),
            cursor_pos: 0,
            error_message: None,
            command_expanded: false,
        }
    }

    fn open(&mut self, actions: &[Action]) {
        self.visible = true;
        self.view = ActionsView::List;
        self.actions = actions.to_vec();
        self.selected_index = if actions.is_empty() { 0 } else { 0 };
        self.editing_index = None;
        self.list_field = ActionListField::List;
        self.scroll_offset = 0;
        self.error_message = None;
        self.command_expanded = false;
    }

    fn close(&mut self) {
        self.visible = false;
        self.view = ActionsView::List;
        self.error_message = None;
    }

    fn open_editor(&mut self, index: Option<usize>) {
        self.view = ActionsView::Editor;
        self.editing_index = index;
        self.editor_field = ActionEditorField::Name;
        self.error_message = None;
        self.cursor_pos = 0;
        self.command_expanded = false;

        if let Some(idx) = index {
            if let Some(action) = self.actions.get(idx) {
                self.edit_name = action.name.clone();
                self.edit_world = action.world.clone();
                self.edit_pattern = action.pattern.clone();
                self.edit_command = action.command.clone();
            }
        } else {
            // New action
            self.edit_name.clear();
            self.edit_world.clear();
            self.edit_pattern.clear();
            self.edit_command.clear();
        }
    }

    fn close_editor(&mut self) {
        self.view = ActionsView::List;
        self.error_message = None;
    }

    fn open_confirm_delete(&mut self) {
        if self.selected_index < self.actions.len() {
            self.view = ActionsView::ConfirmDelete;
            self.confirm_selected = false; // Default to No
        }
    }

    fn close_confirm_delete(&mut self) {
        self.view = ActionsView::List;
    }

    fn save_current_action(&mut self) -> bool {
        // Validate name
        let name = self.edit_name.trim();
        if name.is_empty() {
            self.error_message = Some("Name is required".to_string());
            return false;
        }
        if is_internal_command(name) {
            self.error_message = Some(format!("'{}' is a reserved command name", name));
            return false;
        }
        // Check for duplicate names (excluding current action if editing)
        let editing_idx = self.editing_index;
        for (i, action) in self.actions.iter().enumerate() {
            if Some(i) != editing_idx && action.name.eq_ignore_ascii_case(name) {
                self.error_message = Some(format!("Action '{}' already exists", name));
                return false;
            }
        }

        // Update or create action
        let action = Action {
            name: name.to_string(),
            world: self.edit_world.trim().to_string(),
            pattern: self.edit_pattern.clone(),
            command: self.edit_command.clone(),
        };

        if let Some(idx) = self.editing_index {
            // Update existing
            if idx < self.actions.len() {
                self.actions[idx] = action;
            }
        } else {
            // New action
            self.actions.push(action);
            self.selected_index = self.actions.len() - 1;
        }

        self.error_message = None;
        true
    }

    fn delete_selected_action(&mut self) {
        if self.selected_index < self.actions.len() && !self.actions.is_empty() {
            self.actions.remove(self.selected_index);
            if self.selected_index >= self.actions.len() && !self.actions.is_empty() {
                self.selected_index = self.actions.len() - 1;
            }
        }
    }

    fn current_field_text(&self) -> &str {
        match self.editor_field {
            ActionEditorField::Name => &self.edit_name,
            ActionEditorField::World => &self.edit_world,
            ActionEditorField::Pattern => &self.edit_pattern,
            ActionEditorField::Command => &self.edit_command,
            _ => "",
        }
    }

    fn insert_char(&mut self, c: char) {
        let cursor = self.cursor_pos;
        let text = match self.editor_field {
            ActionEditorField::Name => &mut self.edit_name,
            ActionEditorField::World => &mut self.edit_world,
            ActionEditorField::Pattern => &mut self.edit_pattern,
            ActionEditorField::Command => &mut self.edit_command,
            _ => return,
        };
        if cursor <= text.len() {
            text.insert(cursor, c);
            self.cursor_pos += c.len_utf8();
        }
    }

    fn delete_char(&mut self) {
        let cursor = self.cursor_pos;
        if cursor == 0 {
            return;
        }
        let text = match self.editor_field {
            ActionEditorField::Name => &mut self.edit_name,
            ActionEditorField::World => &mut self.edit_world,
            ActionEditorField::Pattern => &mut self.edit_pattern,
            ActionEditorField::Command => &mut self.edit_command,
            _ => return,
        };
        // Find the character boundary before cursor
        let mut new_pos = cursor - 1;
        while new_pos > 0 && !text.is_char_boundary(new_pos) {
            new_pos -= 1;
        }
        text.remove(new_pos);
        self.cursor_pos = new_pos;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            let text = self.current_field_text();
            let mut new_pos = self.cursor_pos - 1;
            while new_pos > 0 && !text.is_char_boundary(new_pos) {
                new_pos -= 1;
            }
            self.cursor_pos = new_pos;
        }
    }

    fn move_cursor_right(&mut self) {
        let text = self.current_field_text();
        if self.cursor_pos < text.len() {
            let mut new_pos = self.cursor_pos + 1;
            while new_pos < text.len() && !text.is_char_boundary(new_pos) {
                new_pos += 1;
            }
            self.cursor_pos = new_pos;
        }
    }

    fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    fn move_cursor_end(&mut self) {
        self.cursor_pos = self.current_field_text().len();
    }

    // List view field navigation
    fn next_list_field(&mut self) {
        self.list_field = match self.list_field {
            ActionListField::List => ActionListField::AddButton,
            ActionListField::AddButton => ActionListField::EditButton,
            ActionListField::EditButton => ActionListField::DeleteButton,
            ActionListField::DeleteButton => ActionListField::CancelButton,
            ActionListField::CancelButton => ActionListField::List,
        };
    }

    fn prev_list_field(&mut self) {
        self.list_field = match self.list_field {
            ActionListField::List => ActionListField::CancelButton,
            ActionListField::AddButton => ActionListField::List,
            ActionListField::EditButton => ActionListField::AddButton,
            ActionListField::DeleteButton => ActionListField::EditButton,
            ActionListField::CancelButton => ActionListField::DeleteButton,
        };
    }

    // Editor view field navigation
    fn next_editor_field(&mut self) {
        self.editor_field = match self.editor_field {
            ActionEditorField::Name => ActionEditorField::World,
            ActionEditorField::World => ActionEditorField::Pattern,
            ActionEditorField::Pattern => ActionEditorField::Command,
            ActionEditorField::Command => ActionEditorField::SaveButton,
            ActionEditorField::SaveButton => ActionEditorField::CancelButton,
            ActionEditorField::CancelButton => ActionEditorField::Name,
        };
        self.cursor_pos = self.current_field_text().len();
    }

    fn prev_editor_field(&mut self) {
        self.editor_field = match self.editor_field {
            ActionEditorField::Name => ActionEditorField::CancelButton,
            ActionEditorField::World => ActionEditorField::Name,
            ActionEditorField::Pattern => ActionEditorField::World,
            ActionEditorField::Command => ActionEditorField::Pattern,
            ActionEditorField::SaveButton => ActionEditorField::Command,
            ActionEditorField::CancelButton => ActionEditorField::SaveButton,
        };
        self.cursor_pos = self.current_field_text().len();
    }

    fn select_prev_action(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
        // Update scroll offset to keep selection visible
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }
    }

    fn select_next_action(&mut self) {
        if self.selected_index + 1 < self.actions.len() {
            self.selected_index += 1;
        }
        // Update scroll offset to keep selection visible (assuming 5 visible items)
        let visible_items = 5;
        if self.selected_index >= self.scroll_offset + visible_items {
            self.scroll_offset = self.selected_index - visible_items + 1;
        }
    }
}

#[derive(Clone)]
struct Settings {
    more_mode_enabled: bool,
    spell_check_enabled: bool,
    world_switch_mode: WorldSwitchMode,
    debug_enabled: bool,    // Debug logging to clay.debug.log
    theme: Theme,           // Console theme
    gui_theme: Theme,       // GUI theme (separate from console)
    // Remote GUI font settings
    font_name: String,
    font_size: f32,
    // Web server settings (consolidated)
    web_secure: bool,              // Protocol: true=Secure (https/wss), false=Non-Secure (http/ws)
    http_enabled: bool,            // Enable HTTP/HTTPS web server (name depends on web_secure)
    http_port: u16,                // Port for HTTP/HTTPS web interface
    ws_enabled: bool,              // Enable WS/WSS server (name depends on web_secure)
    ws_port: u16,                  // Port for WS/WSS server
    websocket_password: String,
    websocket_allow_list: String,  // CSV list of hosts that can be whitelisted
    websocket_whitelisted_host: Option<String>,  // Currently whitelisted host (authenticated from allow list)
    websocket_cert_file: String,   // Path to TLS certificate file (PEM) - only used when web_secure=true
    websocket_key_file: String,    // Path to TLS private key file (PEM) - only used when web_secure=true
    // User-defined actions/triggers
    actions: Vec<Action>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            more_mode_enabled: true,
            spell_check_enabled: true,
            world_switch_mode: WorldSwitchMode::UnseenFirst,
            debug_enabled: false,
            theme: Theme::Dark,
            gui_theme: Theme::Dark,
            font_name: String::new(),  // Empty means use system default
            font_size: 14.0,
            web_secure: false,         // Default to non-secure
            http_enabled: false,
            http_port: 9000,
            ws_enabled: false,
            ws_port: 9001,
            websocket_password: String::new(),
            websocket_allow_list: String::new(),
            websocket_whitelisted_host: None,
            websocket_cert_file: String::new(),
            websocket_key_file: String::new(),
            actions: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct WorldSettings {
    hostname: String,
    port: String,
    user: String,
    password: String,
    use_ssl: bool,
    log_file: Option<String>,
    encoding: Encoding,
    auto_connect_type: AutoConnectType,
    keep_alive_type: KeepAliveType,
    keep_alive_cmd: String,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            hostname: String::new(),
            port: String::new(),
            user: String::new(),
            password: String::new(),
            use_ssl: false,
            log_file: None,
            encoding: Encoding::Utf8,
            auto_connect_type: AutoConnectType::Connect,
            keep_alive_type: KeepAliveType::Nop,
            keep_alive_cmd: String::new(),
        }
    }
}

/// User-defined action/trigger
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub name: String,           // Unique name (also used as /name command if no pattern)
    pub world: String,          // World name to match (empty = all worlds)
    pub pattern: String,        // Regex pattern to match output (empty = manual /name only)
    pub command: String,        // Command(s) to execute, semicolon-separated
}

impl Action {
    fn new() -> Self {
        Self {
            name: String::new(),
            world: String::new(),
            pattern: String::new(),
            command: String::new(),
        }
    }
}

/// List of internal commands that cannot be overridden by actions
const INTERNAL_COMMANDS: &[&str] = &[
    "help", "connect", "disconnect", "dc", "setup", "world", "worlds", "l",
    "keepalive", "reload", "quit", "actions", "gag", "web", "send",
];

fn is_internal_command(name: &str) -> bool {
    INTERNAL_COMMANDS.contains(&name.to_lowercase().as_str())
}

// ============================================================================
// Shared Command Parsing
// ============================================================================

/// Parsed command representation - shared across console, GUI, and web interfaces
#[derive(Debug, Clone, PartialEq)]
enum Command {
    /// /help - show help popup
    Help,
    /// /quit - exit application
    Quit,
    /// /reload - hot reload binary
    Reload,
    /// /setup - show global settings popup
    Setup,
    /// /web - show web settings popup
    Web,
    /// /actions - show actions popup
    Actions,
    /// /worlds or /l - show connected worlds list
    WorldsList,
    /// /world (no args) - show world selector
    WorldSelector,
    /// /world -e [name] - edit world settings
    WorldEdit { name: Option<String> },
    /// /world -l <name> - connect without auto-login
    WorldConnectNoLogin { name: String },
    /// /world <name> - switch to or connect to named world
    WorldSwitch { name: String },
    /// /connect [host port [ssl]] - connect to server
    Connect { host: Option<String>, port: Option<String>, ssl: bool },
    /// /disconnect or /dc - disconnect current world
    Disconnect,
    /// /send [-W] [-w<world>] [-n] <text> - send text
    Send { text: String, all_worlds: bool, target_world: Option<String>, no_newline: bool },
    /// /keepalive - show keepalive settings
    Keepalive,
    /// /gag <pattern> - gag lines matching pattern
    Gag { pattern: String },
    /// /<action_name> [args] - execute action
    ActionCommand { name: String, args: String },
    /// Not a command (regular text to send to MUD)
    NotACommand { text: String },
    /// Unknown/invalid command
    Unknown { cmd: String },
}

/// Parse a command string into a Command enum
fn parse_command(input: &str) -> Command {
    let trimmed = input.trim();

    // Not a command if doesn't start with /
    if !trimmed.starts_with('/') {
        return Command::NotACommand { text: trimmed.to_string() };
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return Command::NotACommand { text: trimmed.to_string() };
    }

    let cmd = parts[0].to_lowercase();
    let args = &parts[1..];

    match cmd.as_str() {
        "/help" => Command::Help,
        "/quit" => Command::Quit,
        "/reload" => Command::Reload,
        "/setup" => Command::Setup,
        "/web" => Command::Web,
        "/actions" => Command::Actions,
        "/worlds" | "/l" => Command::WorldsList,
        "/world" => parse_world_command(args),
        "/connect" => parse_connect_command(args),
        "/disconnect" | "/dc" => Command::Disconnect,
        "/send" => parse_send_command(args, trimmed),
        "/keepalive" => Command::Keepalive,
        "/gag" => {
            if args.is_empty() {
                Command::Unknown { cmd: trimmed.to_string() }
            } else {
                Command::Gag { pattern: args.join(" ") }
            }
        }
        _ => {
            // Check if it's an action command (starts with / but not a known command)
            let action_name = cmd.trim_start_matches('/');
            if !action_name.is_empty() {
                Command::ActionCommand {
                    name: action_name.to_string(),
                    args: args.join(" "),
                }
            } else {
                Command::Unknown { cmd: trimmed.to_string() }
            }
        }
    }
}

/// Parse /world command with its various forms
fn parse_world_command(args: &[&str]) -> Command {
    if args.is_empty() {
        return Command::WorldSelector;
    }

    match args[0] {
        "-e" => {
            // /world -e [name] - edit world
            let name = if args.len() > 1 {
                Some(args[1..].join(" "))
            } else {
                None
            };
            Command::WorldEdit { name }
        }
        "-l" => {
            // /world -l <name> - connect without auto-login
            if args.len() > 1 {
                Command::WorldConnectNoLogin { name: args[1..].join(" ") }
            } else {
                Command::Unknown { cmd: "/world -l".to_string() }
            }
        }
        _ => {
            // /world <name> - switch to or connect to named world
            Command::WorldSwitch { name: args.join(" ") }
        }
    }
}

/// Parse /connect command
fn parse_connect_command(args: &[&str]) -> Command {
    if args.is_empty() {
        return Command::Connect { host: None, port: None, ssl: false };
    }

    let host = Some(args[0].to_string());
    let port = args.get(1).map(|s| s.to_string());
    let ssl = args.get(2).map(|s| s.to_lowercase() == "ssl").unwrap_or(false);

    Command::Connect { host, port, ssl }
}

/// Parse /send command with flags
fn parse_send_command(args: &[&str], full_cmd: &str) -> Command {
    let mut all_worlds = false;
    let mut target_world: Option<String> = None;
    let mut no_newline = false;
    let mut text_start = 0;

    for (i, arg) in args.iter().enumerate() {
        if *arg == "-W" {
            all_worlds = true;
            text_start = i + 1;
        } else if arg.starts_with("-w") {
            target_world = Some(arg[2..].to_string());
            text_start = i + 1;
        } else if *arg == "-n" {
            no_newline = true;
            text_start = i + 1;
        } else {
            break;
        }
    }

    // Get the text after flags - use original string to preserve spacing
    let text = if text_start < args.len() {
        // Find position of first non-flag argument in original string
        let mut pos = 0;
        let mut found_flags = 0;
        for c in full_cmd.chars() {
            if found_flags >= text_start + 1 { // +1 for /send itself
                break;
            }
            if c.is_whitespace() && pos > 0 {
                // Check if we just finished a word
                let prev_nonws = full_cmd[..pos].chars().rev().find(|c| !c.is_whitespace());
                if prev_nonws.is_some() {
                    found_flags += 1;
                }
            }
            pos += c.len_utf8();
        }
        // Skip whitespace after last flag
        while pos < full_cmd.len() && full_cmd[pos..].starts_with(char::is_whitespace) {
            pos += 1;
        }
        full_cmd[pos..].to_string()
    } else {
        String::new()
    };

    Command::Send { text, all_worlds, target_world, no_newline }
}

/// Split action command string by semicolons, handling escaped semicolons (\;)
fn split_action_commands(command: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&';') {
            // Escaped semicolon - add literal semicolon
            chars.next(); // consume the semicolon
            current.push(';');
        } else if c == ';' {
            // Command separator
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                result.push(trimmed);
            }
            current.clear();
        } else {
            current.push(c);
        }
    }

    // Don't forget the last command
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }

    result
}

/// Result of checking action triggers on a line
struct ActionTriggerResult {
    should_gag: bool,           // If true, suppress the line from output
    commands: Vec<String>,      // Commands to execute
}

/// Check if a line matches any action triggers
/// Returns None if no match, Some(result) if matched
fn check_action_triggers(
    line: &str,
    world_name: &str,
    actions: &[Action],
) -> Option<ActionTriggerResult> {
    // Strip ANSI codes for pattern matching
    let plain_line = strip_ansi_codes(line);

    for action in actions {
        // Skip actions without patterns (those are manual /name only)
        if action.pattern.is_empty() {
            continue;
        }

        // Check if world matches (empty = all worlds)
        if !action.world.is_empty() && action.world != world_name {
            continue;
        }

        // Try to compile and match the regex
        if let Ok(regex) = Regex::new(&action.pattern) {
            if regex.is_match(&plain_line) {
                let commands = split_action_commands(&action.command);
                let should_gag = commands.iter().any(|cmd|
                    cmd.eq_ignore_ascii_case("/gag") || cmd.to_lowercase().starts_with("/gag ")
                );

                // Filter out /gag from the commands to execute
                let filtered_commands: Vec<String> = commands.into_iter()
                    .filter(|cmd| !cmd.eq_ignore_ascii_case("/gag") && !cmd.to_lowercase().starts_with("/gag "))
                    .collect();

                return Some(ActionTriggerResult {
                    should_gag,
                    commands: filtered_commands,
                });
            }
        }
    }

    None
}

const RELOAD_FDS_ENV: &str = "CLAY_RELOAD_FDS";
const CRASH_COUNT_ENV: &str = "CLAY_CRASH_COUNT";
const MAX_CRASH_RESTARTS: u32 = 2;

// Static pointer to App for crash recovery - set when app is running
static APP_PTR: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());
// Track current crash count to avoid re-reading env var
static CRASH_COUNT: AtomicU32 = AtomicU32::new(0);

fn get_reload_state_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".clay.reload")
}

/// Get the current crash count from environment variable
fn get_crash_count() -> u32 {
    std::env::var(CRASH_COUNT_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Clear the crash count (called after successful operation)
fn clear_crash_count() {
    std::env::remove_var(CRASH_COUNT_ENV);
    CRASH_COUNT.store(0, Ordering::SeqCst);
}

/// Set the global app pointer for crash recovery
fn set_app_ptr(app: *mut App) {
    APP_PTR.store(app, Ordering::SeqCst);
}

/// Get the global app pointer
fn get_app_ptr() -> *mut App {
    APP_PTR.load(Ordering::SeqCst)
}

/// Attempt to restart after a crash
fn crash_restart() {
    let crash_count = CRASH_COUNT.load(Ordering::SeqCst);
    if crash_count >= MAX_CRASH_RESTARTS {
        // Already crashed too many times, don't restart
        eprintln!("Maximum crash restarts ({}) reached, not restarting.", MAX_CRASH_RESTARTS);
        return;
    }

    // Try to save state from the app pointer
    let app_ptr = get_app_ptr();
    if !app_ptr.is_null() {
        // SAFETY: We set this pointer in run_app and it remains valid until run_app returns
        let app = unsafe { &*app_ptr };

        // Try to save state
        if let Err(e) = save_reload_state(app) {
            eprintln!("Failed to save state during crash: {}", e);
        }

        // Clear CLOEXEC on socket fds so they survive exec
        for world in &app.worlds {
            if let Some(fd) = world.socket_fd {
                let _ = clear_cloexec(fd);
            }
        }

        // Pass fd list via environment
        let fds_str: String = app.worlds
            .iter()
            .filter_map(|w| w.socket_fd)
            .map(|fd| fd.to_string())
            .collect::<Vec<_>>()
            .join(",");
        std::env::set_var(RELOAD_FDS_ENV, &fds_str);
    }

    // Increment crash count in env
    let new_count = crash_count + 1;
    std::env::set_var(CRASH_COUNT_ENV, new_count.to_string());

    // Try to exec the binary
    if let Ok((exe, _)) = get_executable_path() {
        use std::os::unix::process::CommandExt;
        let mut args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| a != "--reload" && a != "--crash")
            .collect();
        args.push("--crash".to_string());

        // This replaces the current process if successful
        let _ = std::process::Command::new(&exe).args(&args).exec();
    }
}

/// Set up the crash handler (panic hook)
fn setup_crash_handler() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal first to ensure output is visible
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);

        // Print the panic info using the default handler
        eprintln!("\n\nClay crashed! Attempting to restart...\n");
        default_hook(panic_info);

        // Attempt to restart
        crash_restart();

        // If we get here, restart failed - exit normally
    }));
}

struct World {
    name: String,
    output_lines: Vec<String>,
    scroll_offset: usize,
    connected: bool,
    command_tx: Option<mpsc::Sender<WriteCommand>>,
    unseen_lines: usize,
    paused: bool,
    pending_lines: Vec<String>,
    lines_since_pause: usize,
    settings: WorldSettings,
    log_handle: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
    socket_fd: Option<RawFd>,    // Store fd for hot reload (plain TCP only)
    is_tls: bool,                // Track if using TLS
    telnet_mode: bool,           // True if telnet negotiation detected
    prompt: String,              // Current prompt detected via telnet GA
    prompt_count: usize,         // Number of prompts received since connect (for auto-login)
    last_send_time: Option<std::time::Instant>, // For keepalive timing
    last_receive_time: Option<std::time::Instant>, // Last time server data was received
    last_nop_time: Option<std::time::Instant>,     // Last time NOP keepalive was sent
    last_user_command_time: Option<std::time::Instant>, // Last time user sent a command
    partial_line: String,        // Buffer for incomplete lines (no trailing newline)
    partial_in_pending: bool,    // True if partial_line is in pending_lines (vs output_lines)
    is_initial_world: bool,      // True for the auto-created world before first connection
    was_connected: bool,         // True if world has ever been connected (for world cycling)
    skip_auto_login: bool,       // True to skip auto-login on next connect (for /world -l)
    showing_splash: bool,        // True when showing startup splash (for centering)
    needs_redraw: bool,          // True when terminal needs full redraw (after splash clear)
    pending_since: Option<std::time::Instant>, // When pending output first appeared (for Alt-w)
}

impl World {
    fn new(name: &str) -> Self {
        Self::new_with_splash(name, false)
    }

    fn new_with_splash(name: &str, show_splash: bool) -> Self {
        let output_lines = if show_splash {
            Self::generate_splash_lines()
        } else {
            Vec::new()
        };
        let scroll_offset = output_lines.len().saturating_sub(1);
        Self {
            name: name.to_string(),
            output_lines,
            scroll_offset,
            connected: false,
            command_tx: None,
            unseen_lines: 0,
            paused: false,
            pending_lines: Vec::new(),
            lines_since_pause: 0,
            settings: WorldSettings::default(),
            log_handle: None,
            socket_fd: None,
            is_tls: false,
            telnet_mode: false,
            prompt: String::new(),
            prompt_count: 0,
            last_send_time: None,
            last_receive_time: None,
            last_nop_time: None,
            last_user_command_time: None,
            partial_line: String::new(),
            partial_in_pending: false,
            is_initial_world: false,
            was_connected: false,
            skip_auto_login: false,
            showing_splash: show_splash,
            needs_redraw: false,
            pending_since: None,
        }
    }

    fn add_output(
        &mut self,
        text: &str,
        is_current: bool,
        settings: &Settings,
        output_height: u16,
        output_width: u16,
        clear_splash: bool,
    ) {
        // Convert Discord custom emojis to Unicode or :name: fallback
        let text = convert_discord_emojis(text);
        let text = text.as_str();

        // Clear splash mode when MUD data is received (not for client messages)
        if clear_splash && self.showing_splash {
            self.showing_splash = false;
            self.needs_redraw = true; // Signal terminal needs full redraw
            // Clear splash content from output_lines so MUD data starts fresh
            self.output_lines.clear();
            self.scroll_offset = 0;
        }
        let max_lines = (output_height as usize).saturating_sub(2);

        // If we have a partial line from before, combine it with new text
        let had_partial = !self.partial_line.is_empty();
        let partial_was_in_pending = self.partial_in_pending;
        let combined = if had_partial {
            let mut s = std::mem::take(&mut self.partial_line);
            s.push_str(text);
            self.partial_in_pending = false;
            s
        } else {
            text.to_string()
        };

        // Check if text ends with newline (all lines complete) or not (last line is partial)
        let ends_with_newline = combined.ends_with('\n');

        // Collect lines
        let lines: Vec<&str> = combined.lines().collect();
        if lines.is_empty() {
            return;
        }

        // If we had a partial line, update it in the correct list
        let start_idx = if had_partial {
            if partial_was_in_pending {
                // Update the last pending line
                if let Some(last) = self.pending_lines.last_mut() {
                    *last = lines[0].to_string();
                }
            } else {
                // Update the last output line
                if let Some(last) = self.output_lines.last_mut() {
                    *last = lines[0].to_string();
                }
            }
            1 // Skip first line since we updated it
        } else {
            0
        };

        // Process remaining lines
        let line_count = lines.len();
        for (i, line) in lines.iter().enumerate().skip(start_idx) {
            let is_last = i == line_count - 1;
            let is_partial = is_last && !ends_with_newline;

            // Filter out keep-alive idler message lines (don't show or log them)
            if line.contains("###_idler_message_") && line.contains("_###") {
                continue;
            }

            // Skip visually empty lines (only ANSI codes/whitespace) unless partial
            // This filters out blank lines generated by cursor control sequences
            if !is_partial && is_visually_empty(line) {
                continue;
            }

            // Write to log file if configured (only for complete lines)
            if !is_partial {
                if let Some(ref handle) = self.log_handle {
                    if let Ok(mut file) = handle.lock() {
                        let _ = writeln!(file, "{}", line);
                    }
                }
            }

            // Track if this line goes to pending (for partial tracking)
            let goes_to_pending = self.paused && settings.more_mode_enabled;
            let triggers_pause = !goes_to_pending
                && settings.more_mode_enabled
                && self.lines_since_pause >= max_lines
                && self.output_lines.len() >= max_lines;

            if goes_to_pending {
                // Track when pending output first appeared
                if self.pending_lines.is_empty() {
                    self.pending_since = Some(std::time::Instant::now());
                }
                self.pending_lines.push(line.to_string());
                if is_partial {
                    self.partial_line = line.to_string();
                    self.partial_in_pending = true;
                }
            } else if triggers_pause {
                // Scroll to show lines added before pause, then pause
                self.scroll_to_bottom();
                self.paused = true;
                // Track when pending output first appeared
                if self.pending_lines.is_empty() {
                    self.pending_since = Some(std::time::Instant::now());
                }
                self.pending_lines.push(line.to_string());
                if is_partial {
                    self.partial_line = line.to_string();
                    self.partial_in_pending = true;
                }
            } else {
                self.output_lines.push(line.to_string());
                // Count visual lines (accounting for word wrap) instead of logical lines
                let visual_lines = visual_line_count(line, output_width as usize);
                self.lines_since_pause += visual_lines;
                if !is_current {
                    self.unseen_lines += 1;
                }
                if is_partial {
                    self.partial_line = line.to_string();
                    self.partial_in_pending = false;
                }
            }
        }
        // If more mode is off, always unpause, release pending, and scroll to bottom
        if !settings.more_mode_enabled {
            self.paused = false;
            // Release any pending lines immediately
            if !self.pending_lines.is_empty() {
                self.output_lines.append(&mut self.pending_lines);
            }
        }
        // Always scroll to bottom unless paused (and more mode is on)
        if !self.paused {
            self.scroll_to_bottom();
        }
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.output_lines.len().saturating_sub(1);
    }

    fn mark_seen(&mut self) {
        self.unseen_lines = 0;
    }

    fn release_pending(&mut self, count: usize) {
        let to_release: Vec<String> = self
            .pending_lines
            .drain(..count.min(self.pending_lines.len()))
            .collect();
        for line in to_release {
            self.output_lines.push(line);
        }
        if self.pending_lines.is_empty() {
            self.paused = false;
            self.lines_since_pause = 0;
            // If partial was in pending, it's now in output
            if self.partial_in_pending {
                self.partial_in_pending = false;
            }
        } else {
            // Reset counter for next batch
            self.lines_since_pause = 0;
        }
        self.scroll_to_bottom();
    }

    fn release_all_pending(&mut self) {
        self.output_lines.append(&mut self.pending_lines);
        self.paused = false;
        self.lines_since_pause = 0;
        self.pending_since = None; // Clear pending timestamp
        // If partial was in pending, it's now in output
        if self.partial_in_pending {
            self.partial_in_pending = false;
        }
        self.scroll_to_bottom();
    }

    fn is_at_bottom(&self) -> bool {
        self.scroll_offset >= self.output_lines.len().saturating_sub(1)
    }

    fn lines_from_bottom(&self) -> usize {
        self.output_lines
            .len()
            .saturating_sub(1)
            .saturating_sub(self.scroll_offset)
    }

    fn generate_splash_lines() -> Vec<String> {
        // Splash content without centering - will be centered at render time
        // Dog art:
        //           (\/\__o
        //   __      `-/ `_/
        //  `--\______/  |
        //     /        /
        //  -`/_------'\_.
        vec![
            "".to_string(),
            "\x1b[38;5;180m          (\\/\\__o     \x1b[38;5;209m ██████╗██╗      █████╗ ██╗   ██╗\x1b[0m".to_string(),
            "\x1b[38;5;180m  __      `-/ `_/     \x1b[38;5;208m██╔════╝██║     ██╔══██╗╚██╗ ██╔╝\x1b[0m".to_string(),
            "\x1b[38;5;180m `--\\______/  |       \x1b[38;5;215m██║     ██║     ███████║ ╚████╔╝ \x1b[0m".to_string(),
            "\x1b[38;5;180m    /        /        \x1b[38;5;216m██║     ██║     ██╔══██║  ╚██╔╝  \x1b[0m".to_string(),
            "\x1b[38;5;180m -`/_------'\\_.       \x1b[38;5;217m╚██████╗███████╗██║  ██║   ██║   \x1b[0m".to_string(),
            "\x1b[38;5;218m                       ╚═════╝╚══════╝╚═╝  ╚═╝   ╚═╝   \x1b[0m".to_string(),
            "".to_string(),
            "\x1b[38;5;213m✨ A 90dies mud client written today ✨\x1b[0m".to_string(),
            "".to_string(),
            "\x1b[38;5;244m/help for how to use clay\x1b[0m".to_string(),
            "".to_string(),
        ]
    }
}

struct App {
    worlds: Vec<World>,
    current_world_index: usize,
    input: InputArea,
    input_height: u16,
    output_height: u16,
    output_width: u16,
    spell_checker: SpellChecker,
    spell_state: SpellState,
    suggestion_message: Option<String>,
    settings: Settings,
    settings_popup: SettingsPopup,
    world_selector: WorldSelectorPopup,
    confirm_dialog: ConfirmDialog,
    worlds_popup: WorldsPopup,
    filter_popup: FilterPopup,
    help_popup: HelpPopup,
    actions_popup: ActionsPopup,
    web_popup: WebPopup,
    last_ctrl_c: Option<std::time::Instant>,
    last_escape: Option<std::time::Instant>, // For Escape+key sequences (Alt emulation)
    show_tags: bool, // F2 toggles - false = hide tags (default), true = show tags
    // WebSocket server (ws:// or wss:// depending on web_secure setting)
    ws_server: Option<WebSocketServer>,
    // HTTP web interface server (no TLS)
    http_server: Option<HttpServer>,
    // HTTPS web interface server
    #[cfg(feature = "native-tls-backend")]
    https_server: Option<HttpsServer>,
    #[cfg(feature = "rustls-backend")]
    https_server: Option<HttpsServer>,
    // Track if popup was visible last frame (for terminal clear on transition)
    popup_was_visible: bool,
}

impl App {
    fn new() -> Self {
        Self {
            worlds: Vec::new(),
            current_world_index: 0,
            input: InputArea::new(3),
            input_height: 3,
            output_height: 20, // Will be updated by ui()
            output_width: 80,  // Will be updated by ui()
            spell_checker: SpellChecker::new(),
            spell_state: SpellState::new(),
            suggestion_message: None,
            settings: Settings::default(),
            settings_popup: SettingsPopup::new(),
            world_selector: WorldSelectorPopup::new(),
            confirm_dialog: ConfirmDialog::new(),
            worlds_popup: WorldsPopup::new(),
            filter_popup: FilterPopup::new(),
            help_popup: HelpPopup::new(),
            actions_popup: ActionsPopup::new(),
            web_popup: WebPopup::new(),
            last_ctrl_c: None,
            last_escape: None,
            show_tags: false, // Default: hide tags
            ws_server: None,
            http_server: None,
            #[cfg(feature = "native-tls-backend")]
            https_server: None,
            #[cfg(feature = "rustls-backend")]
            https_server: None,
            popup_was_visible: false,
        }
        // Note: No initial world created here - it will be created after load_settings()
        // if no worlds are configured
    }

    /// Ensure there's at least one world (creates initial world if needed)
    /// Also adds splash screen to current world if it has no output
    fn ensure_has_world(&mut self) {
        if self.worlds.is_empty() {
            let mut initial_world = World::new_with_splash(&get_binary_name(), true);
            initial_world.is_initial_world = true;
            self.worlds.push(initial_world);
        } else {
            // Add splash to current world if it has no output yet
            let current = &mut self.worlds[self.current_world_index];
            if current.output_lines.is_empty() && !current.connected {
                current.output_lines = World::generate_splash_lines();
                current.showing_splash = true;
                current.scroll_offset = current.output_lines.len().saturating_sub(1);
            }
        }
    }

    fn current_world(&self) -> &World {
        // Safety: clamp index to valid range to prevent panic
        let idx = if self.worlds.is_empty() {
            0  // Will panic below, but ensure_has_world() should prevent this
        } else {
            self.current_world_index.min(self.worlds.len() - 1)
        };
        &self.worlds[idx]
    }

    fn current_world_mut(&mut self) -> &mut World {
        // Safety: clamp index to valid range to prevent panic
        let idx = if self.worlds.is_empty() {
            0  // Will panic below, but ensure_has_world() should prevent this
        } else {
            self.current_world_index.min(self.worlds.len() - 1)
        };
        &mut self.worlds[idx]
    }

    fn switch_world(&mut self, index: usize) {
        if index < self.worlds.len() {
            self.current_world_index = index;
            self.current_world_mut().mark_seen();
        }
    }

    /// Switch to the world with the oldest pending output (Alt-w)
    /// Returns true if switched, false if no world has pending output
    fn switch_to_oldest_pending(&mut self) -> bool {
        let mut oldest_idx: Option<usize> = None;
        let mut oldest_time: Option<std::time::Instant> = None;

        for (idx, world) in self.worlds.iter().enumerate() {
            // Skip current world and worlds without pending output
            if idx == self.current_world_index || world.pending_lines.is_empty() {
                continue;
            }
            if let Some(pending_time) = world.pending_since {
                if oldest_time.is_none() || pending_time < oldest_time.unwrap() {
                    oldest_time = Some(pending_time);
                    oldest_idx = Some(idx);
                }
            }
        }

        if let Some(idx) = oldest_idx {
            self.switch_world(idx);
            true
        } else {
            false
        }
    }

    /// Get sorted list of world indices for cycling (alphabetically by name, case-insensitive)
    fn get_sorted_world_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.worlds.len()).collect();
        indices.sort_by(|&a, &b| {
            self.worlds[a]
                .name
                .to_lowercase()
                .cmp(&self.worlds[b].name.to_lowercase())
        });
        indices
    }

    fn next_world(&mut self) {
        // Build world info for shared function
        let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
            .map(|w| crate::util::WorldSwitchInfo {
                name: w.name.clone(),
                connected: w.connected,
                unseen_lines: w.unseen_lines,
            })
            .collect();

        if let Some(next_idx) = crate::util::calculate_next_world(
            &world_info,
            self.current_world_index,
            self.settings.world_switch_mode,
        ) {
            self.switch_world(next_idx);
        }
    }

    fn prev_world(&mut self) {
        // Build world info for shared function
        let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
            .map(|w| crate::util::WorldSwitchInfo {
                name: w.name.clone(),
                connected: w.connected,
                unseen_lines: w.unseen_lines,
            })
            .collect();

        if let Some(prev_idx) = crate::util::calculate_prev_world(
            &world_info,
            self.current_world_index,
            self.settings.world_switch_mode,
        ) {
            self.switch_world(prev_idx);
        }
    }

    fn next_world_all(&mut self) {
        // Cycle through all worlds that have ever been connected (alphabetically)
        let sorted = self.get_sorted_world_indices();
        if sorted.is_empty() {
            return;
        }
        let current_pos = sorted.iter().position(|&i| i == self.current_world_index).unwrap_or(0);
        let len = sorted.len();

        for i in 1..=len {
            let sorted_idx = (current_pos + i) % len;
            let world_idx = sorted[sorted_idx];
            if self.worlds[world_idx].was_connected {
                self.switch_world(world_idx);
                return;
            }
        }
        // No worlds that were connected, stay on current
    }

    fn prev_world_all(&mut self) {
        // Cycle through all worlds that have ever been connected (alphabetically)
        let sorted = self.get_sorted_world_indices();
        if sorted.is_empty() {
            return;
        }
        let current_pos = sorted.iter().position(|&i| i == self.current_world_index).unwrap_or(0);
        let len = sorted.len();

        for i in 1..=len {
            let sorted_idx = if current_pos >= i {
                current_pos - i
            } else {
                len - (i - current_pos)
            };
            let world_idx = sorted[sorted_idx];
            if self.worlds[world_idx].was_connected {
                self.switch_world(world_idx);
                return;
            }
        }
        // No worlds that were connected, stay on current
    }

    fn find_world(&self, name: &str) -> Option<usize> {
        self.worlds.iter().position(|w| w.name == name)
    }

    fn find_or_create_world(&mut self, name: &str) -> usize {
        if let Some(idx) = self.find_world(name) {
            idx
        } else {
            self.worlds.push(World::new(name));
            self.worlds.len() - 1
        }
    }

    fn activity_count(&self) -> usize {
        self.worlds
            .iter()
            .enumerate()
            .filter(|(i, w)| *i != self.current_world_index && w.unseen_lines > 0)
            .count()
    }

    /// Discard the initial "fake" world if it exists and is not connected.
    /// Called after first successful connection to a real world.
    fn discard_initial_world(&mut self) {
        // Find index of initial world that's not connected
        if let Some(idx) = self.worlds.iter().position(|w| w.is_initial_world && !w.connected) {
            // Don't discard if it's the only world or if it's the current world
            if self.worlds.len() > 1 && idx != self.current_world_index {
                self.worlds.remove(idx);
                // Adjust current_world_index if needed
                if self.current_world_index > idx {
                    self.current_world_index -= 1;
                }
            }
        }
    }

    fn add_output(&mut self, text: &str) {
        let is_current = true;
        let settings = self.settings.clone();
        let output_height = self.output_height;
        let output_width = self.output_width;
        // Ensure client-generated messages are complete lines (end with newline)
        let text_with_newline = if text.ends_with('\n') || text.is_empty() {
            text.to_string()
        } else {
            format!("{}\n", text)
        };
        self.current_world_mut()
            .add_output(&text_with_newline, is_current, &settings, output_height, output_width, false);
    }

    /// Broadcast a message to all authenticated WebSocket clients
    fn ws_broadcast(&self, msg: WsMessage) {
        // Broadcast to secure WebSocket server (wss://)
        if let Some(ref server) = self.ws_server {
            let clients = server.clients.clone();
            let msg_clone = msg.clone();
            tokio::spawn(async move {
                let clients_guard = clients.read().await;
                if let Ok(json) = serde_json::to_string(&msg_clone) {
                    for client in clients_guard.values() {
                        if client.authenticated {
                            let _ = client.tx.send(msg_clone.clone());
                        }
                    }
                    drop(json); // Used to validate serialization works
                }
            });
        }
    }

    /// Send a message to a specific WebSocket client
    fn ws_send_to_client(&self, client_id: u64, msg: WsMessage) {
        if let Some(ref server) = self.ws_server {
            let clients = server.clients.clone();
            tokio::spawn(async move {
                let clients_guard = clients.read().await;
                if let Some(client) = clients_guard.get(&client_id) {
                    let _ = client.tx.send(msg);
                }
            });
        }
    }

    /// Build initial state message for a newly authenticated client
    fn build_initial_state(&self) -> WsMessage {
        let worlds: Vec<WorldStateMsg> = self.worlds.iter().enumerate().map(|(idx, world)| {
            // Strip carriage returns from output/pending lines for web clients
            let clean_output: Vec<String> = world.output_lines.iter()
                .map(|s| s.replace('\r', ""))
                .collect();
            let clean_pending: Vec<String> = world.pending_lines.iter()
                .map(|s| s.replace('\r', ""))
                .collect();
            WorldStateMsg {
                index: idx,
                name: world.name.clone(),
                connected: world.connected,
                output_lines: clean_output,
                pending_lines: clean_pending,
                prompt: world.prompt.replace('\r', ""),
                scroll_offset: world.scroll_offset,
                paused: world.paused,
                unseen_lines: world.unseen_lines,
                settings: WorldSettingsMsg {
                    hostname: world.settings.hostname.clone(),
                    port: world.settings.port.clone(),
                    user: world.settings.user.clone(),
                    use_ssl: world.settings.use_ssl,
                    log_file: world.settings.log_file.clone(),
                    encoding: world.settings.encoding.name().to_string(),
                    auto_connect_type: world.settings.auto_connect_type.name().to_string(),
                    keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                    keep_alive_cmd: world.settings.keep_alive_cmd.clone(),
                },
                last_send_secs: world.last_send_time.map(|t| t.elapsed().as_secs()),
                last_recv_secs: world.last_receive_time.map(|t| t.elapsed().as_secs()),
                last_nop_secs: world.last_nop_time.map(|t| t.elapsed().as_secs()),
                keep_alive_type: world.settings.keep_alive_type.name().to_string(),
            }
        }).collect();

        let settings = GlobalSettingsMsg {
            more_mode_enabled: self.settings.more_mode_enabled,
            spell_check_enabled: self.settings.spell_check_enabled,
            world_switch_mode: self.settings.world_switch_mode.name().to_string(),
            debug_enabled: self.settings.debug_enabled,
            show_tags: self.show_tags,
            console_theme: self.settings.theme.name().to_string(),
            gui_theme: self.settings.gui_theme.name().to_string(),
            input_height: self.input_height,
            font_name: self.settings.font_name.clone(),
            font_size: self.settings.font_size,
            ws_allow_list: self.settings.websocket_allow_list.clone(),
            web_secure: self.settings.web_secure,
            http_enabled: self.settings.http_enabled,
            http_port: self.settings.http_port,
            ws_enabled: self.settings.ws_enabled,
            ws_port: self.settings.ws_port,
        };

        WsMessage::InitialState {
            worlds,
            settings,
            current_world_index: self.current_world_index,
            actions: self.settings.actions.clone(),
        }
    }

    fn increase_input_height(&mut self) {
        if self.input_height < 15 {
            self.input_height += 1;
            self.input.visible_height = self.input_height;
        }
    }

    fn decrease_input_height(&mut self) {
        if self.input_height > 1 {
            self.input_height -= 1;
            self.input.visible_height = self.input_height;
            self.input.adjust_viewport();
        }
    }

    fn handle_spell_check(&mut self) {
        if !self.spell_state.showing_suggestions {
            if let Some((start, end, word)) = self.input.current_word() {
                if !self.spell_checker.is_valid(&word) {
                    let mut suggestions = self.spell_checker.suggestions(&word, 6);
                    if !suggestions.is_empty() {
                        // Store original word and add it to the end for cycling
                        self.spell_state.original_word = word.clone();
                        suggestions.push(word);  // Add original word at the end

                        // Output suggestions to the output area (excluding the original word)
                        let display_suggestions: Vec<_> = suggestions[..suggestions.len()-1].to_vec();
                        self.add_output(&format!(
                            "Suggestions for '{}': {}",
                            self.spell_state.original_word,
                            display_suggestions.join(", ")
                        ));

                        self.spell_state.suggestions = suggestions;
                        self.spell_state.suggestion_index = 0;
                        self.spell_state.word_start = start;
                        self.spell_state.word_end = end;
                        self.spell_state.showing_suggestions = true;
                        self.suggestion_message = Some(format!(
                            "Press Ctrl+Q to cycle: {}",
                            self.spell_state.suggestions[0]
                        ));
                    }
                    // If no suggestions found or word is correctly spelled, do nothing
                }
                // If word is spelled correctly, do nothing
            }
            // If no word at cursor, do nothing
        } else if !self.spell_state.suggestions.is_empty() {
            // Cycle through suggestions (including original word at the end)
            let replacement = self.spell_state.suggestions[self.spell_state.suggestion_index].clone();
            self.input.replace_word(
                self.spell_state.word_start,
                self.spell_state.word_end,
                &replacement,
            );
            self.spell_state.word_end = self.spell_state.word_start + replacement.chars().count();
            self.spell_state.suggestion_index =
                (self.spell_state.suggestion_index + 1) % self.spell_state.suggestions.len();

            let next_word = &self.spell_state.suggestions[self.spell_state.suggestion_index];
            if next_word == &self.spell_state.original_word {
                self.suggestion_message = Some(format!(
                    "Applied '{}'. Next: '{}' (original)",
                    replacement, next_word
                ));
            } else {
                self.suggestion_message = Some(format!(
                    "Applied '{}'. Next: '{}'",
                    replacement, next_word
                ));
            }
        }
    }

    fn check_word_ended(&mut self) {
        if self.spell_state.showing_suggestions {
            // Convert byte cursor to character position for comparison
            let cursor_char_pos = self.input.buffer[..self.input.cursor_position].chars().count();
            if cursor_char_pos < self.spell_state.word_start || cursor_char_pos > self.spell_state.word_end {
                self.spell_state.reset();
                self.suggestion_message = None;
            }
        }
    }

    fn find_misspelled_words(&self) -> Vec<(usize, usize)> {
        let mut misspelled = Vec::new();
        let chars: Vec<char> = self.input.buffer.chars().collect();
        let mut i = 0;

        // Convert byte cursor to character position
        let cursor_char_pos = self.input.buffer[..self.input.cursor_position].chars().count();

        while i < chars.len() {
            while i < chars.len() && !chars[i].is_alphabetic() {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            let start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            let end = i;

            let word: String = chars[start..end].iter().collect();
            let cursor_in_word = cursor_char_pos >= start && cursor_char_pos <= end;

            if !cursor_in_word && !self.spell_checker.is_valid(&word) {
                misspelled.push((start, end));
            }
        }

        misspelled
    }

    fn scroll_output_up(&mut self) {
        let more_mode = self.settings.more_mode_enabled;
        let target_visual_lines = (self.output_height as usize).saturating_sub(2).max(1);
        let visible_height = (self.output_height as usize).max(1);
        let width = (self.output_width as usize).max(1);
        let world = self.current_world_mut();

        // Calculate the minimum scroll_offset where line 0 is at the top
        // This is where all content from line 0 to scroll_offset fits in visible_height
        let mut min_offset = 0usize;
        let mut visual_lines = 0usize;
        for (idx, line) in world.output_lines.iter().enumerate() {
            visual_lines += visual_line_count(line, width);
            if visual_lines >= visible_height {
                min_offset = idx;
                break;
            }
            min_offset = idx;
        }

        // If already at or past the minimum, don't scroll further
        if world.scroll_offset <= min_offset {
            // Still enable pause mode if more_mode is on
            if more_mode && !world.paused {
                world.paused = true;
            }
            return;
        }

        // Count lines being scrolled off (from scroll_offset going backwards)
        // These are the lines that will disappear from the bottom
        let mut visual_lines_moved = 0;
        let mut new_offset = world.scroll_offset;

        while visual_lines_moved < target_visual_lines {
            visual_lines_moved += visual_line_count(&world.output_lines[new_offset], width);
            if new_offset == 0 {
                break;
            }
            new_offset -= 1;
        }

        // Clamp to minimum offset
        world.scroll_offset = new_offset.max(min_offset);
        if more_mode && !world.paused {
            world.paused = true;
        }
    }

    fn scroll_output_down(&mut self) {
        let target_visual_lines = (self.output_height as usize).saturating_sub(2).max(1);
        let width = (self.output_width as usize).max(1);
        let world = self.current_world_mut();
        let max_scroll = world.output_lines.len().saturating_sub(1);

        if world.scroll_offset >= max_scroll {
            return; // Already at bottom
        }

        // Count lines being scrolled in (from scroll_offset+1 going forwards)
        // These are the lines that will appear at the bottom
        let mut visual_lines_moved = 0;
        let mut new_offset = world.scroll_offset + 1;

        while new_offset <= max_scroll && visual_lines_moved < target_visual_lines {
            visual_lines_moved += visual_line_count(&world.output_lines[new_offset], width);
            new_offset += 1;
        }

        // new_offset is one past the last line counted, so subtract 1
        world.scroll_offset = (new_offset - 1).min(max_scroll);

        // If we've scrolled to bottom, unpause
        if world.is_at_bottom() {
            world.paused = false;
        }
    }
}

pub enum AppEvent {
    ServerData(usize, Vec<u8>),  // world_index, raw bytes
    Disconnected(usize),         // world_index
    TelnetDetected(usize),       // world_index - telnet negotiation detected
    Prompt(usize, Vec<u8>),      // world_index, prompt bytes (from telnet GA)
    SystemMessage(String),       // message to display in current world's output
    // WebSocket events
    WsClientConnected(u64),                    // client_id
    WsClientDisconnected(u64),                 // client_id
    WsClientMessage(u64, Box<WsMessage>),      // client_id, message
}

fn get_settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".clay.dat")
}

fn get_debug_log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("clay.debug.log")
}

/// Write a debug message to clay.debug.log if debug is enabled
fn debug_log(debug_enabled: bool, message: &str) {
    if !debug_enabled {
        return;
    }
    use std::io::Write;
    let path = get_debug_log_path();
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            // Get local time using libc
            let timestamp = unsafe {
                let mut now: libc::time_t = 0;
                libc::time(&mut now);
                let tm = libc::localtime(&now);
                if tm.is_null() {
                    "????-??-?? ??:??:??".to_string()
                } else {
                    format!(
                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                        (*tm).tm_year + 1900,
                        (*tm).tm_mon + 1,
                        (*tm).tm_mday,
                        (*tm).tm_hour,
                        (*tm).tm_min,
                        (*tm).tm_sec
                    )
                }
            };
            let _ = writeln!(file, "[{}] {}", timestamp, message);
        }
        Err(e) => {
            eprintln!("Failed to open debug log {:?}: {}", path, e);
        }
    }
}

/// Log keepalive being sent
fn debug_log_keepalive(debug_enabled: bool, world_name: &str, keepalive_type: &str, data_sent: &str) {
    // Log the command and its bytes for debugging
    let bytes: Vec<u8> = data_sent.bytes().collect();
    debug_log(debug_enabled, &format!("KEEPALIVE world='{}' type={} sent='{}' bytes={:?}", world_name, keepalive_type, data_sent, bytes));
}

/// Encryption key for password storage (padded to 32 bytes for AES-256)
const PASSWORD_ENCRYPTION_KEY: &[u8; 32] = b"nonsupersecretpassword#\0\0\0\0\0\0\0\0\0";

/// Encrypt a password using AES-256-GCM and return base64-encoded result
fn encrypt_password(password: &str) -> String {
    if password.is_empty() {
        return String::new();
    }

    let cipher = Aes256Gcm::new(PASSWORD_ENCRYPTION_KEY.into());

    // Use a fixed nonce derived from the password length (not cryptographically ideal,
    // but acceptable for this obfuscation use case with a known key)
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[0] = (password.len() & 0xFF) as u8;
    nonce_bytes[1] = ((password.len() >> 8) & 0xFF) as u8;
    // Add some variation based on first few chars
    for (i, b) in password.bytes().take(10).enumerate() {
        nonce_bytes[2 + i] = b;
    }
    let nonce = Nonce::from_slice(&nonce_bytes);

    match cipher.encrypt(nonce, password.as_bytes()) {
        Ok(ciphertext) => {
            // Prepend nonce to ciphertext and base64 encode
            let mut combined = nonce_bytes.to_vec();
            combined.extend(ciphertext);
            format!("ENC:{}", BASE64.encode(&combined))
        }
        Err(_) => {
            // Fallback to plain (shouldn't happen)
            password.to_string()
        }
    }
}

/// Decrypt a password. Returns the original string if it's not encrypted or decryption fails.
fn decrypt_password(stored: &str) -> String {
    if stored.is_empty() {
        return String::new();
    }

    // Check if it's an encrypted password
    if !stored.starts_with("ENC:") {
        // Not encrypted, return as-is (legacy plain password)
        return stored.to_string();
    }

    let encoded = &stored[4..]; // Skip "ENC:" prefix

    let combined = match BASE64.decode(encoded) {
        Ok(data) => data,
        Err(_) => return stored.to_string(), // Invalid base64, treat as plain
    };

    if combined.len() < 12 {
        // Too short to contain nonce, treat as plain
        return stored.to_string();
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new(PASSWORD_ENCRYPTION_KEY.into());

    match cipher.decrypt(nonce, ciphertext) {
        Ok(plaintext) => String::from_utf8_lossy(&plaintext).to_string(),
        Err(_) => {
            // Decryption failed - might be a plain password that happens to start with "ENC:"
            // This is unlikely but we handle it gracefully
            stored.to_string()
        }
    }
}

fn save_settings(app: &App) -> io::Result<()> {
    let path = get_settings_path();
    let mut file = std::fs::File::create(&path)?;

    // Save global settings
    writeln!(file, "[global]")?;
    writeln!(file, "more_mode={}", app.settings.more_mode_enabled)?;
    writeln!(file, "spell_check={}", app.settings.spell_check_enabled)?;
    writeln!(file, "world_switch_mode={}", app.settings.world_switch_mode.name())?;
    writeln!(file, "debug_enabled={}", app.settings.debug_enabled)?;
    writeln!(file, "input_height={}", app.input_height)?;
    writeln!(file, "theme={}", app.settings.theme.name())?;
    writeln!(file, "gui_theme={}", app.settings.gui_theme.name())?;
    writeln!(file, "font_name={}", app.settings.font_name)?;
    writeln!(file, "font_size={}", app.settings.font_size)?;
    writeln!(file, "web_secure={}", app.settings.web_secure)?;
    writeln!(file, "http_enabled={}", app.settings.http_enabled)?;
    writeln!(file, "http_port={}", app.settings.http_port)?;
    writeln!(file, "ws_enabled={}", app.settings.ws_enabled)?;
    writeln!(file, "ws_port={}", app.settings.ws_port)?;
    if !app.settings.websocket_password.is_empty() {
        writeln!(file, "websocket_password={}", encrypt_password(&app.settings.websocket_password))?;
    }
    if !app.settings.websocket_allow_list.is_empty() {
        writeln!(file, "websocket_allow_list={}", app.settings.websocket_allow_list)?;
    }
    if !app.settings.websocket_cert_file.is_empty() {
        writeln!(file, "websocket_cert_file={}", app.settings.websocket_cert_file)?;
    }
    if !app.settings.websocket_key_file.is_empty() {
        writeln!(file, "websocket_key_file={}", app.settings.websocket_key_file)?;
    }

    // Save each world's settings
    for world in &app.worlds {
        writeln!(file)?;
        writeln!(file, "[world:{}]", world.name)?;
        writeln!(file, "hostname={}", world.settings.hostname)?;
        writeln!(file, "port={}", world.settings.port)?;
        writeln!(file, "user={}", world.settings.user)?;
        writeln!(file, "password={}", encrypt_password(&world.settings.password))?;
        writeln!(file, "use_ssl={}", world.settings.use_ssl)?;
        writeln!(file, "encoding={}", world.settings.encoding.name())?;
        writeln!(file, "auto_connect_type={}", world.settings.auto_connect_type.name())?;
        writeln!(file, "keep_alive_type={}", world.settings.keep_alive_type.name())?;
        if !world.settings.keep_alive_cmd.is_empty() {
            writeln!(file, "keep_alive_cmd={}", world.settings.keep_alive_cmd)?;
        }
        if let Some(ref log) = world.settings.log_file {
            writeln!(file, "log_file={}", log)?;
        }
    }

    // Save actions
    for (idx, action) in app.settings.actions.iter().enumerate() {
        writeln!(file)?;
        writeln!(file, "[action:{}]", idx)?;
        writeln!(file, "name={}", action.name)?;
        if !action.world.is_empty() {
            writeln!(file, "world={}", action.world)?;
        }
        if !action.pattern.is_empty() {
            // Escape newlines and equals signs in pattern
            writeln!(file, "pattern={}", action.pattern.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
        }
        if !action.command.is_empty() {
            // Escape newlines and equals signs in command
            writeln!(file, "command={}", action.command.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
        }
    }

    Ok(())
}

fn load_settings(app: &mut App) -> io::Result<()> {
    let path = get_settings_path();
    if !path.exists() {
        return Ok(());
    }

    let file = std::fs::File::open(&path)?;
    let reader = std::io::BufReader::new(file);

    let mut current_world: Option<String> = None;
    let mut current_action: Option<usize> = None;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with("[global]") {
            current_world = None;
            current_action = None;
            continue;
        }

        if line.starts_with("[world:") && line.ends_with(']') {
            let name = &line[7..line.len() - 1];
            // Find or create world
            let idx = app.find_or_create_world(name);
            current_world = Some(app.worlds[idx].name.clone());
            current_action = None;
            continue;
        }

        if line.starts_with("[action:") && line.ends_with(']') {
            // Start a new action
            current_world = None;
            app.settings.actions.push(Action::new());
            current_action = Some(app.settings.actions.len() - 1);
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];

            if current_world.is_none() {
                // Global settings
                match key {
                    "more_mode" => {
                        app.settings.more_mode_enabled = value == "true";
                    }
                    "spell_check" => {
                        app.settings.spell_check_enabled = value == "true";
                    }
                    "pending_first" => {
                        // Backward compatibility: pending_first=true -> UnseenFirst
                        app.settings.world_switch_mode = if value == "true" {
                            WorldSwitchMode::UnseenFirst
                        } else {
                            WorldSwitchMode::Alphabetical
                        };
                    }
                    "world_switch_mode" => {
                        app.settings.world_switch_mode = WorldSwitchMode::from_name(value);
                    }
                    "debug_enabled" => {
                        app.settings.debug_enabled = value == "true";
                    }
                    "input_height" => {
                        if let Ok(h) = value.parse::<u16>() {
                            app.input_height = h.clamp(1, 15);
                            app.input.visible_height = app.input_height;
                        }
                    }
                    "theme" => {
                        app.settings.theme = Theme::from_name(value);
                    }
                    "gui_theme" => {
                        app.settings.gui_theme = Theme::from_name(value);
                    }
                    "font_name" => {
                        app.settings.font_name = value.to_string();
                    }
                    "font_size" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.font_size = s.clamp(8.0, 48.0);
                        }
                    }
                    "web_secure" => {
                        app.settings.web_secure = value == "true";
                    }
                    "ws_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    "ws_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_enabled maps to ws_enabled
                    "websocket_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    // Legacy: websocket_port maps to ws_port
                    "websocket_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_use_tls maps to web_secure
                    "websocket_use_tls" => {
                        app.settings.web_secure = value == "true";
                    }
                    "websocket_password" => {
                        app.settings.websocket_password = decrypt_password(value);
                    }
                    "websocket_allow_list" => {
                        app.settings.websocket_allow_list = value.to_string();
                    }
                    "websocket_cert_file" => {
                        app.settings.websocket_cert_file = value.to_string();
                    }
                    "websocket_key_file" => {
                        app.settings.websocket_key_file = value.to_string();
                    }
                    "http_enabled" => {
                        app.settings.http_enabled = value == "true";
                    }
                    "http_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.http_port = p;
                        }
                    }
                    // Legacy fields - map https to http when web_secure, ws_nonsecure to ws when !web_secure
                    "https_enabled" => {
                        // If https was enabled in old config, set http_enabled and web_secure
                        if value == "true" {
                            app.settings.http_enabled = true;
                            app.settings.web_secure = true;
                        }
                    }
                    "https_port" => {
                        // Legacy: https_port was separate, now http_port is used for both
                        if let Ok(p) = value.parse::<u16>() {
                            // Only use https_port if web_secure is set
                            if app.settings.web_secure {
                                app.settings.http_port = p;
                            }
                        }
                    }
                    "ws_nonsecure_enabled" => {
                        // Legacy: ws_nonsecure maps to ws_enabled when not secure
                        if value == "true" && !app.settings.web_secure {
                            app.settings.ws_enabled = true;
                        }
                    }
                    "ws_nonsecure_port" => {
                        // Legacy: ws_nonsecure_port was separate, now ws_port is used for both
                        if let Ok(p) = value.parse::<u16>() {
                            if !app.settings.web_secure {
                                app.settings.ws_port = p;
                            }
                        }
                    }
                    // Legacy: ignore global encoding, it's now per-world
                    "encoding" => {}
                    _ => {}
                }
            } else if let Some(ref world_name) = current_world {
                // Find the world and update its settings
                if let Some(world) = app.worlds.iter_mut().find(|w| &w.name == world_name) {
                    match key {
                        "hostname" => world.settings.hostname = value.to_string(),
                        "port" => world.settings.port = value.to_string(),
                        "user" => world.settings.user = value.to_string(),
                        "password" => world.settings.password = decrypt_password(value),
                        "use_ssl" => world.settings.use_ssl = value == "true",
                        "log_file" => world.settings.log_file = Some(value.to_string()),
                        "encoding" => {
                            world.settings.encoding = match value {
                                "latin1" => Encoding::Latin1,
                                "fansi" => Encoding::Fansi,
                                _ => Encoding::Utf8,
                            };
                        }
                        "auto_connect_type" => {
                            world.settings.auto_connect_type = AutoConnectType::from_name(value);
                        }
                        "keep_alive_type" => {
                            world.settings.keep_alive_type = KeepAliveType::from_name(value);
                        }
                        "keep_alive_cmd" => {
                            world.settings.keep_alive_cmd = value.to_string();
                        }
                        _ => {}
                    }
                }
            } else if let Some(action_idx) = current_action {
                // Action settings
                if let Some(action) = app.settings.actions.get_mut(action_idx) {
                    // Helper to unescape saved strings
                    fn unescape_action_value(s: &str) -> String {
                        s.replace("\\n", "\n").replace("\\e", "=").replace("\\\\", "\\")
                    }
                    match key {
                        "name" => action.name = value.to_string(),
                        "world" => action.world = value.to_string(),
                        "pattern" => action.pattern = unescape_action_value(value),
                        "command" => action.command = unescape_action_value(value),
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

fn save_reload_state(app: &App) -> io::Result<()> {
    let path = get_reload_state_path();
    let mut file = std::fs::File::create(&path)?;

    // Save global state
    writeln!(file, "[reload]")?;
    writeln!(file, "current_world_index={}", app.current_world_index)?;
    writeln!(file, "input_height={}", app.input_height)?;
    writeln!(file, "more_mode={}", app.settings.more_mode_enabled)?;
    writeln!(file, "spell_check={}", app.settings.spell_check_enabled)?;
    writeln!(file, "world_switch_mode={}", app.settings.world_switch_mode.name())?;
    writeln!(file, "debug_enabled={}", app.settings.debug_enabled)?;
    writeln!(file, "show_tags={}", app.show_tags)?;
    writeln!(file, "theme={}", app.settings.theme.name())?;
    writeln!(file, "font_name={}", app.settings.font_name)?;
    writeln!(file, "font_size={}", app.settings.font_size)?;
    writeln!(file, "web_secure={}", app.settings.web_secure)?;
    writeln!(file, "http_enabled={}", app.settings.http_enabled)?;
    writeln!(file, "http_port={}", app.settings.http_port)?;
    writeln!(file, "ws_enabled={}", app.settings.ws_enabled)?;
    writeln!(file, "ws_port={}", app.settings.ws_port)?;
    if !app.settings.websocket_password.is_empty() {
        writeln!(file, "websocket_password={}", encrypt_password(&app.settings.websocket_password))?;
    }
    if !app.settings.websocket_allow_list.is_empty() {
        writeln!(file, "websocket_allow_list={}", app.settings.websocket_allow_list)?;
    }
    // Get whitelisted_host from running server, or from settings
    let whitelisted_host = if let Some(ref server) = app.ws_server {
        server.get_whitelisted_host()
    } else {
        app.settings.websocket_whitelisted_host.clone()
    };
    if let Some(ref host) = whitelisted_host {
        writeln!(file, "websocket_whitelisted_host={}", host)?;
    }
    if !app.settings.websocket_cert_file.is_empty() {
        writeln!(file, "websocket_cert_file={}", app.settings.websocket_cert_file)?;
    }
    if !app.settings.websocket_key_file.is_empty() {
        writeln!(file, "websocket_key_file={}", app.settings.websocket_key_file)?;
    }

    // Save input history (base64 encode each line to handle special chars)
    writeln!(file, "history_count={}", app.input.history.len())?;
    for (i, hist) in app.input.history.iter().enumerate() {
        // Simple escape: replace newlines and = with escape sequences
        let escaped = hist.replace('\\', "\\\\").replace('\n', "\\n").replace('=', "\\e");
        writeln!(file, "history_{}={}", i, escaped)?;
    }

    // Save each world's state
    writeln!(file, "world_count={}", app.worlds.len())?;
    for (idx, world) in app.worlds.iter().enumerate() {
        writeln!(file)?;
        writeln!(file, "[world_state:{}]", idx)?;
        writeln!(file, "name={}", world.name.replace('=', "\\e"))?;
        writeln!(file, "scroll_offset={}", world.scroll_offset)?;
        writeln!(file, "connected={}", world.connected)?;
        writeln!(file, "unseen_lines={}", world.unseen_lines)?;
        writeln!(file, "paused={}", world.paused)?;
        writeln!(file, "lines_since_pause={}", world.lines_since_pause)?;
        writeln!(file, "is_tls={}", world.is_tls)?;
        writeln!(file, "was_connected={}", world.was_connected)?;

        // Socket fd if connected (will be passed via env var separately)
        if let Some(fd) = world.socket_fd {
            writeln!(file, "socket_fd={}", fd)?;
        }

        // World settings
        writeln!(file, "hostname={}", world.settings.hostname)?;
        writeln!(file, "port={}", world.settings.port)?;
        writeln!(file, "user={}", world.settings.user.replace('=', "\\e"))?;
        writeln!(file, "password={}", world.settings.password.replace('=', "\\e"))?;
        writeln!(file, "use_ssl={}", world.settings.use_ssl)?;
        writeln!(file, "encoding={}", world.settings.encoding.name())?;
        writeln!(file, "auto_connect_type={}", world.settings.auto_connect_type.name())?;
        writeln!(file, "keep_alive_type={}", world.settings.keep_alive_type.name())?;
        if !world.settings.keep_alive_cmd.is_empty() {
            writeln!(file, "keep_alive_cmd={}", world.settings.keep_alive_cmd.replace('=', "\\e"))?;
        }
        if let Some(ref log) = world.settings.log_file {
            writeln!(file, "log_file={}", log)?;
        }

        // Output lines count (we'll save the actual lines separately due to size)
        writeln!(file, "output_count={}", world.output_lines.len())?;
        writeln!(file, "pending_count={}", world.pending_lines.len())?;
    }

    // Save output lines in a separate section (can be large)
    for (idx, world) in app.worlds.iter().enumerate() {
        writeln!(file)?;
        writeln!(file, "[output:{}]", idx)?;
        for line in &world.output_lines {
            let escaped = line.replace('\\', "\\\\").replace('\n', "\\n");
            writeln!(file, "{}", escaped)?;
        }
        writeln!(file, "[pending:{}]", idx)?;
        for line in &world.pending_lines {
            let escaped = line.replace('\\', "\\\\").replace('\n', "\\n");
            writeln!(file, "{}", escaped)?;
        }
    }

    Ok(())
}

fn unescape_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('e') => result.push('='),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn load_reload_state(app: &mut App) -> io::Result<bool> {
    let path = get_reload_state_path();
    if !path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();

    // Parse the reload state
    let mut current_section = String::new();
    let mut current_world_idx: Option<usize> = None;
    let mut output_world_idx: Option<usize> = None;
    let mut pending_world_idx: Option<usize> = None;

    // Temporary storage for world data
    struct TempWorld {
        name: String,
        output_lines: Vec<String>,
        scroll_offset: usize,
        connected: bool,
        socket_fd: Option<RawFd>,
        unseen_lines: usize,
        paused: bool,
        pending_lines: Vec<String>,
        lines_since_pause: usize,
        is_tls: bool,
        was_connected: bool,
        settings: WorldSettings,
    }

    let mut temp_worlds: Vec<TempWorld> = Vec::new();

    for line in lines {
        // Check for section headers FIRST (before output/pending line handling)
        // This prevents section headers from being added as output lines
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = &trimmed[1..trimmed.len() - 1];
            if section == "reload" {
                current_section = "reload".to_string();
            } else if let Some(suffix) = section.strip_prefix("world_state:") {
                let idx: usize = suffix.parse().unwrap_or(0);
                current_section = "world_state".to_string();
                current_world_idx = Some(idx);
                // Ensure we have enough temp worlds
                while temp_worlds.len() <= idx {
                    temp_worlds.push(TempWorld {
                        name: String::new(),
                        output_lines: Vec::new(),
                        scroll_offset: 0,
                        connected: false,
                        socket_fd: None,
                        unseen_lines: 0,
                        paused: false,
                        pending_lines: Vec::new(),
                        lines_since_pause: 0,
                        is_tls: false,
                        was_connected: false,
                        settings: WorldSettings::default(),
                    });
                }
            } else if let Some(suffix) = section.strip_prefix("output:") {
                let idx: usize = suffix.parse().unwrap_or(0);
                current_section = "output".to_string();
                output_world_idx = Some(idx);
                pending_world_idx = None;
            } else if let Some(suffix) = section.strip_prefix("pending:") {
                let idx: usize = suffix.parse().unwrap_or(0);
                current_section = "pending".to_string();
                pending_world_idx = Some(idx);
                output_world_idx = None;
            }
            continue;
        }

        // Handle output/pending lines without trimming to preserve leading spaces
        if current_section == "output" {
            if let Some(idx) = output_world_idx {
                if idx < temp_worlds.len() {
                    temp_worlds[idx].output_lines.push(unescape_string(line));
                }
            }
            continue;
        }
        if current_section == "pending" {
            if let Some(idx) = pending_world_idx {
                if idx < temp_worlds.len() {
                    temp_worlds[idx].pending_lines.push(unescape_string(line));
                }
            }
            continue;
        }

        // For non-output sections, trim whitespace
        let line = trimmed;
        if line.is_empty() {
            continue;
        }

        // Parse key=value
        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];

            if current_section == "reload" {
                match key {
                    "current_world_index" => {
                        app.current_world_index = value.parse().unwrap_or(0);
                    }
                    "input_height" => {
                        app.input_height = value.parse().unwrap_or(3);
                        app.input.visible_height = app.input_height;
                    }
                    "more_mode" => {
                        app.settings.more_mode_enabled = value == "true";
                    }
                    "spell_check" => {
                        app.settings.spell_check_enabled = value == "true";
                    }
                    "pending_first" => {
                        // Backward compatibility: pending_first=true -> UnseenFirst
                        app.settings.world_switch_mode = if value == "true" {
                            WorldSwitchMode::UnseenFirst
                        } else {
                            WorldSwitchMode::Alphabetical
                        };
                    }
                    "world_switch_mode" => {
                        app.settings.world_switch_mode = WorldSwitchMode::from_name(value);
                    }
                    "debug_enabled" => {
                        app.settings.debug_enabled = value == "true";
                    }
                    "show_tags" => {
                        app.show_tags = value == "true";
                    }
                    "theme" => {
                        app.settings.theme = Theme::from_name(value);
                    }
                    "gui_theme" => {
                        app.settings.gui_theme = Theme::from_name(value);
                    }
                    "font_name" => {
                        app.settings.font_name = value.to_string();
                    }
                    "font_size" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.font_size = s.clamp(8.0, 48.0);
                        }
                    }
                    "web_secure" => {
                        app.settings.web_secure = value == "true";
                    }
                    "ws_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    "ws_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_enabled maps to ws_enabled
                    "websocket_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    // Legacy: websocket_port maps to ws_port
                    "websocket_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_use_tls maps to web_secure
                    "websocket_use_tls" => {
                        app.settings.web_secure = value == "true";
                    }
                    "websocket_password" => {
                        app.settings.websocket_password = decrypt_password(value);
                    }
                    "websocket_allow_list" => {
                        app.settings.websocket_allow_list = value.to_string();
                    }
                    "websocket_whitelisted_host" => {
                        app.settings.websocket_whitelisted_host = Some(value.to_string());
                    }
                    "websocket_cert_file" => {
                        app.settings.websocket_cert_file = value.to_string();
                    }
                    "websocket_key_file" => {
                        app.settings.websocket_key_file = value.to_string();
                    }
                    "http_enabled" => {
                        app.settings.http_enabled = value == "true";
                    }
                    "http_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.http_port = p;
                        }
                    }
                    // Legacy fields
                    "https_enabled" => {
                        if value == "true" {
                            app.settings.http_enabled = true;
                            app.settings.web_secure = true;
                        }
                    }
                    "https_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            if app.settings.web_secure {
                                app.settings.http_port = p;
                            }
                        }
                    }
                    "ws_nonsecure_enabled" => {
                        if value == "true" && !app.settings.web_secure {
                            app.settings.ws_enabled = true;
                        }
                    }
                    "ws_nonsecure_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            if !app.settings.web_secure {
                                app.settings.ws_port = p;
                            }
                        }
                    }
                    // Legacy: ignore global encoding, it's now per-world
                    "encoding" => {}
                    "history_count" | "world_count" => {
                        // These are informational, not needed for parsing
                    }
                    k if k.starts_with("history_") => {
                        app.input.history.push(unescape_string(value));
                    }
                    _ => {}
                }
            } else if current_section == "world_state" {
                if let Some(idx) = current_world_idx {
                    if idx < temp_worlds.len() {
                        let tw = &mut temp_worlds[idx];
                        match key {
                            "name" => tw.name = unescape_string(value),
                            "scroll_offset" => tw.scroll_offset = value.parse().unwrap_or(0),
                            "connected" => tw.connected = value == "true",
                            "unseen_lines" => tw.unseen_lines = value.parse().unwrap_or(0),
                            "paused" => tw.paused = value == "true",
                            "lines_since_pause" => tw.lines_since_pause = value.parse().unwrap_or(0),
                            "is_tls" => tw.is_tls = value == "true",
                            "was_connected" => tw.was_connected = value == "true",
                            "socket_fd" => tw.socket_fd = value.parse().ok(),
                            "hostname" => tw.settings.hostname = value.to_string(),
                            "port" => tw.settings.port = value.to_string(),
                            "user" => tw.settings.user = unescape_string(value),
                            "password" => tw.settings.password = unescape_string(value),
                            "use_ssl" => tw.settings.use_ssl = value == "true",
                            "log_file" => tw.settings.log_file = Some(value.to_string()),
                            "encoding" => {
                                tw.settings.encoding = match value {
                                    "latin1" => Encoding::Latin1,
                                    "fansi" => Encoding::Fansi,
                                    _ => Encoding::Utf8,
                                };
                            }
                            "auto_connect_type" => {
                                tw.settings.auto_connect_type = AutoConnectType::from_name(value);
                            }
                            "keep_alive_type" => {
                                tw.settings.keep_alive_type = KeepAliveType::from_name(value);
                            }
                            "keep_alive_cmd" => {
                                tw.settings.keep_alive_cmd = value.replace("\\e", "=");
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Convert temp worlds to real worlds
    app.worlds.clear();
    for tw in temp_worlds {
        let mut world = World::new(&tw.name);
        world.output_lines = tw.output_lines;
        world.scroll_offset = tw.scroll_offset;
        world.connected = tw.connected;
        world.unseen_lines = tw.unseen_lines;
        world.paused = tw.paused;
        world.pending_lines = tw.pending_lines;
        world.lines_since_pause = tw.lines_since_pause;
        world.is_tls = tw.is_tls;
        world.was_connected = tw.was_connected;
        world.socket_fd = tw.socket_fd;
        world.settings = tw.settings;
        // Leave timing fields as None for connected worlds after reload
        // This triggers immediate keepalive since we don't know how long connection was idle
        app.worlds.push(world);
    }

    // Note: Don't create initial world here - let ensure_has_world() handle it after
    // settings are fully loaded, to avoid creating unnecessary "clay" world

    // Validate current_world_index
    if app.current_world_index >= app.worlds.len() {
        app.current_world_index = 0;
    }

    // Clean up the reload state file
    let _ = std::fs::remove_file(&path);

    Ok(true)
}

fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    // Clear the FD_CLOEXEC flag so the fd survives exec
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        let new_flags = flags & !libc::FD_CLOEXEC;
        if libc::fcntl(fd, libc::F_SETFD, new_flags) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Strip " (deleted)" suffix from a path string if present.
fn strip_deleted_suffix(path_str: &str) -> String {
    // Try common variations of the deleted marker
    for suffix in [" (deleted)", "(deleted)"] {
        if let Some(stripped) = path_str.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    path_str.to_string()
}

/// Get the executable path, handling the case where binary was updated.
/// On Linux, if the binary was replaced, /proc/self/exe shows " (deleted)".
/// We strip that suffix to get the path to the new binary.
/// Returns (path, debug_info) for better error messages.
fn get_executable_path() -> io::Result<(PathBuf, String)> {
    let proc_exe = PathBuf::from("/proc/self/exe");
    let link_target = std::fs::read_link(&proc_exe)?;
    let target_str = link_target.to_string_lossy().to_string();
    let clean_path = strip_deleted_suffix(&target_str);
    let debug_info = format!(
        "raw='{}', cleaned='{}', exists={}",
        target_str,
        clean_path,
        PathBuf::from(&clean_path).exists()
    );
    Ok((PathBuf::from(clean_path), debug_info))
}

fn exec_reload(app: &App) -> io::Result<()> {
    // Save the current state
    save_reload_state(app)?;

    // Collect socket fds that need to survive exec
    let mut fds_to_keep: Vec<RawFd> = Vec::new();
    for world in &app.worlds {
        if let Some(fd) = world.socket_fd {
            clear_cloexec(fd)?;
            fds_to_keep.push(fd);
        }
    }

    // Get the executable path with debug info
    let (exe, debug_info) = get_executable_path()?;

    // Verify the executable exists
    if !exe.exists() {
        return Err(io::Error::other(format!(
            "Executable not found. Debug: {}",
            debug_info
        )));
    }

    // Pass fd list via environment (fds must survive exec)
    let fds_str: String = fds_to_keep
        .iter()
        .map(|fd| fd.to_string())
        .collect::<Vec<_>>()
        .join(",");
    std::env::set_var(RELOAD_FDS_ENV, &fds_str);

    // Execute the new binary with --reload argument
    use std::os::unix::process::CommandExt;
    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--reload").collect();
    args.push("--reload".to_string());
    let err = std::process::Command::new(&exe)
        .args(&args)
        .exec();

    // If we get here, exec failed
    Err(io::Error::other(format!("exec failed: {} (path: {})", err, exe.display())))
}

// ============================================================================
// Remote GUI Client (feature = "remote-gui")
// ============================================================================

#[cfg(feature = "remote-gui")]
mod remote_gui {
    use super::*;
    use egui::{Color32, ScrollArea, TextEdit};
    use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};

    /// World settings for remote GUI
    #[derive(Clone, Default)]
    pub struct RemoteWorldSettings {
        pub hostname: String,
        pub port: String,
        pub user: String,
        pub use_ssl: bool,
        pub keep_alive_type: String,
        pub keep_alive_cmd: String,
    }

    /// State for a remote world
    pub struct RemoteWorld {
        pub name: String,
        pub connected: bool,
        pub output_lines: Vec<String>,
        pub prompt: String,
        pub settings: RemoteWorldSettings,
        pub unseen_lines: usize,
        pub pending_count: usize,  // Count of pending lines (for pending_first sorting)
        // Timing info (seconds since event, None if never)
        pub last_send_secs: Option<u64>,
        pub last_recv_secs: Option<u64>,
        pub last_nop_secs: Option<u64>,
    }

    /// Which popup is currently open
    #[derive(PartialEq, Clone)]
    enum PopupState {
        None,
        WorldList,
        ConnectedWorlds,  // /worlds or /l - shows connected worlds with stats
        WorldEditor(usize),  // world index being edited
        Setup,
        Web,  // /web - web settings (HTTP/HTTPS/WS)
        Font,
        Help,
        ActionsList,           // Actions list (first window)
        ActionEditor(usize),   // Action editor (second window) - index of action being edited
        ActionConfirmDelete,   // Delete confirmation dialog
    }

    /// Remote GUI client application state
    /// GUI Theme - mirrors the TUI Theme but with egui colors
    #[derive(Clone, Copy, PartialEq)]
    enum GuiTheme {
        Dark,
        Light,
    }

    #[allow(dead_code)]
    impl GuiTheme {
        fn from_name(name: &str) -> Self {
            match name {
                "light" => GuiTheme::Light,
                _ => GuiTheme::Dark,
            }
        }

        fn bg(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::BLACK,  // Pure black like a terminal
                GuiTheme::Light => Color32::from_rgb(250, 250, 250),
            }
        }

        fn fg(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(192, 192, 192),  // Light gray like terminal default
                GuiTheme::Light => Color32::BLACK,
            }
        }

        fn fg_dim(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(128, 128, 128),  // Medium gray
                GuiTheme::Light => Color32::DARK_GRAY,
            }
        }

        fn accent(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(0, 255, 255),  // Cyan like terminal
                GuiTheme::Light => Color32::from_rgb(0, 100, 180),
            }
        }

        fn highlight(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::YELLOW,
                GuiTheme::Light => Color32::from_rgb(180, 100, 0),
            }
        }

        fn success(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(0, 255, 0),  // Bright green like terminal
                GuiTheme::Light => Color32::from_rgb(0, 128, 0),
            }
        }

        fn error(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(255, 0, 0),  // Bright red like terminal
                GuiTheme::Light => Color32::from_rgb(180, 0, 0),
            }
        }

        fn panel_bg(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(16, 16, 16),  // Very dark gray for panels
                GuiTheme::Light => Color32::from_rgb(235, 235, 235),
            }
        }

        fn button_bg(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(32, 32, 32),  // Dark gray for buttons
                GuiTheme::Light => Color32::from_rgb(220, 220, 220),
            }
        }

        fn selection_bg(&self) -> Color32 {
            match self {
                GuiTheme::Dark => Color32::from_rgb(0, 64, 128),  // Dark blue selection
                GuiTheme::Light => Color32::from_rgb(180, 200, 230),
            }
        }

        fn prompt(&self) -> Color32 {
            // Cyan color for prompts (matches TUI style)
            match self {
                GuiTheme::Dark => Color32::from_rgb(0, 255, 255),
                GuiTheme::Light => Color32::from_rgb(0, 139, 139),
            }
        }

        fn link(&self) -> Color32 {
            // Blue color for clickable links
            match self {
                GuiTheme::Dark => Color32::from_rgb(100, 149, 237), // Cornflower blue
                GuiTheme::Light => Color32::from_rgb(0, 0, 238),    // Standard link blue
            }
        }
    }

    pub struct RemoteGuiApp {
        /// WebSocket URL
        ws_url: String,
        /// Password for authentication
        password: String,
        /// Whether we're connected to the server
        connected: bool,
        /// Whether we're authenticated
        authenticated: bool,
        /// Error message to display
        error_message: Option<String>,
        /// Worlds received from server
        worlds: Vec<RemoteWorld>,
        /// Currently selected world index
        current_world: usize,
        /// Input buffer for commands
        input_buffer: String,
        /// Channel for sending messages to WebSocket
        ws_tx: Option<mpsc::UnboundedSender<WsMessage>>,
        /// Channel for receiving messages from WebSocket
        ws_rx: Option<mpsc::UnboundedReceiver<WsMessage>>,
        /// Runtime handle for async operations
        runtime: tokio::runtime::Handle,
        /// Flag indicating password was submitted
        password_submitted: bool,
        /// Flag indicating we've attempted auto-connect for allow list
        auto_connect_attempted: bool,
        /// Time when connection was established (for allow list timeout)
        connect_time: Option<std::time::Instant>,
        /// Current popup state
        popup_state: PopupState,
        /// Selected world in world list popup
        world_list_selected: usize,
        /// Temp fields for world editor
        edit_name: String,
        edit_hostname: String,
        edit_port: String,
        edit_user: String,
        edit_ssl: bool,
        edit_keep_alive_type: KeepAliveType,
        edit_keep_alive_cmd: String,
        /// Input area height in lines
        input_height: u16,
        /// Console theme (for TUI on server)
        console_theme: GuiTheme,
        /// GUI theme (local)
        theme: GuiTheme,
        /// Font name (empty for system default)
        font_name: String,
        /// Font size in points
        font_size: f32,
        /// Temp field for font editor
        edit_font_name: String,
        /// Temp field for font size editor
        edit_font_size: String,
        /// Last loaded font name (to avoid reloading)
        loaded_font_name: String,
        /// Command history
        command_history: Vec<String>,
        /// Current position in command history (0 = current input, 1+ = history)
        history_index: usize,
        /// Saved input when browsing history
        saved_input: String,
        /// Manual scroll offset for output (None = auto-scroll to bottom)
        scroll_offset: Option<f32>,
        /// Maximum scroll offset (content height - viewport height)
        scroll_max_offset: f32,
        /// Show MUD tags
        show_tags: bool,
        /// More mode enabled (pause on overflow)
        more_mode: bool,
        /// Spell check enabled
        spell_check_enabled: bool,
        /// Filter text for output
        filter_text: String,
        /// Whether filter popup is open
        filter_active: bool,
        /// WebSocket allow list (CSV of hosts that can connect without password)
        ws_allow_list: String,
        /// Web secure protocol (true = https/wss, false = http/ws)
        web_secure: bool,
        /// HTTP/HTTPS server enabled
        http_enabled: bool,
        /// HTTP/HTTPS server port
        http_port: u16,
        /// WS/WSS server enabled
        ws_enabled: bool,
        /// WS/WSS server port
        ws_port: u16,
        /// World switching mode (Unseen First or Alphabetical)
        world_switch_mode: WorldSwitchMode,
        /// Debug logging enabled (synced from server, not used locally)
        debug_enabled: bool,
        /// Spell checker for input validation
        spell_checker: SpellChecker,
        /// Spell check state (suggestions, current word, etc.)
        spell_state: SpellState,
        /// Message about spell suggestions
        suggestion_message: Option<String>,
        /// Actions synced from server
        actions: Vec<Action>,
        /// Selected action index in actions list
        actions_selected: usize,
        /// Action editor temp fields
        edit_action_name: String,
        edit_action_world: String,
        edit_action_pattern: String,
        edit_action_command: String,
        /// Action error message
        action_error: Option<String>,
    }

    impl RemoteGuiApp {
        pub fn new(ws_url: String, runtime: tokio::runtime::Handle) -> Self {
            Self {
                ws_url,
                password: String::new(),
                connected: false,
                authenticated: false,
                error_message: None,
                worlds: Vec::new(),
                current_world: 0,
                input_buffer: String::new(),
                ws_tx: None,
                ws_rx: None,
                runtime,
                password_submitted: false,
                auto_connect_attempted: false,
                connect_time: None,
                popup_state: PopupState::None,
                world_list_selected: 0,
                edit_name: String::new(),
                edit_hostname: String::new(),
                edit_port: String::new(),
                edit_user: String::new(),
                edit_ssl: false,
                edit_keep_alive_type: KeepAliveType::Nop,
                edit_keep_alive_cmd: String::new(),
                input_height: 3,
                console_theme: GuiTheme::Dark,
                theme: GuiTheme::Dark,
                font_name: String::new(),
                font_size: 14.0,
                edit_font_name: String::new(),
                edit_font_size: String::from("14.0"),
                loaded_font_name: String::from("__uninitialized__"),
                command_history: Vec::new(),
                history_index: 0,
                saved_input: String::new(),
                scroll_offset: None,
                scroll_max_offset: 0.0,
                show_tags: false,
                more_mode: true,
                spell_check_enabled: true,
                filter_text: String::new(),
                filter_active: false,
                ws_allow_list: String::new(),
                web_secure: false,
                http_enabled: false,
                http_port: 9000,
                ws_enabled: false,
                ws_port: 9001,
                world_switch_mode: WorldSwitchMode::UnseenFirst,
                debug_enabled: false,
                spell_checker: SpellChecker::new(),
                spell_state: SpellState::new(),
                suggestion_message: None,
                actions: Vec::new(),
                actions_selected: 0,
                edit_action_name: String::new(),
                edit_action_world: String::new(),
                edit_action_pattern: String::new(),
                edit_action_command: String::new(),
                action_error: None,
            }
        }

        /// Try to find a system font file by name
        fn find_system_font(font_name: &str) -> Option<Vec<u8>> {
            // Common font directories on Linux
            let font_dirs = [
                "/usr/share/fonts",
                "/usr/local/share/fonts",
                "~/.fonts",
                "~/.local/share/fonts",
            ];

            // Map font names to common file names
            let file_patterns: &[&str] = match font_name {
                "Monospace" => &["DejaVuSansMono.ttf", "LiberationMono-Regular.ttf", "UbuntuMono-R.ttf"],
                "DejaVu Sans Mono" => &["DejaVuSansMono.ttf"],
                "Liberation Mono" => &["LiberationMono-Regular.ttf"],
                "Ubuntu Mono" => &["UbuntuMono-R.ttf", "UbuntuMono-Regular.ttf"],
                "Fira Code" => &["FiraCode-Regular.ttf", "FiraCode-Retina.ttf"],
                "Source Code Pro" => &["SourceCodePro-Regular.ttf", "SourceCodePro-Regular.otf"],
                "JetBrains Mono" => &["JetBrainsMono-Regular.ttf", "JetBrainsMono[wght].ttf"],
                "Hack" => &["Hack-Regular.ttf"],
                "Inconsolata" => &["Inconsolata-Regular.ttf", "Inconsolata.ttf"],
                "Courier New" => &["cour.ttf", "CourierNew.ttf"],
                "Consolas" => &["consola.ttf", "Consolas.ttf"],
                _ => &[],
            };

            // Search for font files
            for dir in &font_dirs {
                let dir_path = if dir.starts_with('~') {
                    if let Some(home) = std::env::var_os("HOME") {
                        std::path::PathBuf::from(home).join(&dir[2..])
                    } else {
                        continue;
                    }
                } else {
                    std::path::PathBuf::from(dir)
                };

                if !dir_path.exists() {
                    continue;
                }

                // Recursively search for font files
                fn search_dir(dir: &std::path::Path, patterns: &[&str]) -> Option<Vec<u8>> {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() {
                                if let Some(data) = search_dir(&path, patterns) {
                                    return Some(data);
                                }
                            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                for pattern in patterns {
                                    if name.eq_ignore_ascii_case(pattern) {
                                        if let Ok(data) = std::fs::read(&path) {
                                            return Some(data);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    None
                }

                if let Some(data) = search_dir(&dir_path, file_patterns) {
                    return Some(data);
                }
            }

            None
        }

        fn connect_websocket(&mut self) {
            // Determine URL scheme - if ws_url starts with wss:// or ws://, use as-is
            // Otherwise, try wss:// (secure) by default
            let url = if self.ws_url.starts_with("ws://") || self.ws_url.starts_with("wss://") {
                self.ws_url.clone()
            } else {
                // Default to wss:// for security
                format!("wss://{}", self.ws_url)
            };

            let (tx, rx) = mpsc::unbounded_channel::<WsMessage>();
            let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WsMessage>();

            self.ws_tx = Some(out_tx);
            self.ws_rx = Some(rx);

            let password_hash = hash_password(&self.password);
            let password_submitted = self.password_submitted;
            let ws_url_for_fallback = self.ws_url.clone();

            self.runtime.spawn(async move {
                // Try to connect - for wss:// we need to configure TLS
                #[cfg(feature = "native-tls-backend")]
                let connect_result = if url.starts_with("wss://") {
                    // Create a TLS connector that accepts self-signed certificates
                    let tls_connector = native_tls::TlsConnector::builder()
                        .danger_accept_invalid_certs(true)
                        .danger_accept_invalid_hostnames(true)
                        .build()
                        .map_err(|e| tokio_tungstenite::tungstenite::Error::Tls(
                            tokio_tungstenite::tungstenite::error::TlsError::Native(e)
                        ));

                    match tls_connector {
                        Ok(connector) => {
                            let connector = tokio_tungstenite::Connector::NativeTls(connector);
                            tokio_tungstenite::connect_async_tls_with_config(
                                &url,
                                None,
                                false,
                                Some(connector),
                            ).await.map(|(ws, resp)| (ws, resp))
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    connect_async(&url).await
                };

                // Without native-tls, only ws:// is supported
                #[cfg(not(feature = "native-tls-backend"))]
                let connect_result = {
                    // For rustls backend, fall back to ws://
                    let ws_url = if url.starts_with("wss://") {
                        format!("ws://{}", ws_url_for_fallback)
                    } else {
                        url.clone()
                    };
                    connect_async(&ws_url).await
                };

                // If wss:// failed and we defaulted to it, try ws:// as fallback
                #[cfg(feature = "native-tls-backend")]
                let connect_result = match connect_result {
                    Ok(result) => Ok(result),
                    Err(_e) if url.starts_with("wss://") && !ws_url_for_fallback.starts_with("wss://") => {
                        // Try ws:// fallback
                        let fallback_url = format!("ws://{}", ws_url_for_fallback);
                        connect_async(&fallback_url).await
                    }
                    Err(e) => Err(e),
                };

                match connect_result {
                    Ok((ws_stream, _)) => {
                        use futures::SinkExt;
                        let (mut ws_sink, mut ws_source) = ws_stream.split();

                        // Send auth request if password was submitted
                        if password_submitted {
                            let auth_msg = WsMessage::AuthRequest { password_hash };
                            if let Ok(json) = serde_json::to_string(&auth_msg) {
                                let _ = ws_sink.send(WsRawMessage::Text(json.into())).await;
                            }
                        }

                        // Spawn sender task
                        let mut ws_sink = ws_sink;
                        tokio::spawn(async move {
                            while let Some(msg) = out_rx.recv().await {
                                if let Ok(json) = serde_json::to_string(&msg) {
                                    if ws_sink.send(WsRawMessage::Text(json.into())).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        });

                        // Receive messages
                        while let Some(msg_result) = ws_source.next().await {
                            match msg_result {
                                Ok(WsRawMessage::Text(text)) => {
                                    if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                                        if tx.send(ws_msg).is_err() {
                                            break;
                                        }
                                    }
                                }
                                Ok(WsRawMessage::Close(_)) => break,
                                Err(_) => break,
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(WsMessage::AuthResponse {
                            success: false,
                            error: Some(format!("Connection failed: {}", e)),
                        });
                    }
                }
            });

            self.connected = true;
            self.connect_time = Some(std::time::Instant::now());
        }

        /// Send authentication request with current password
        fn send_auth(&mut self) {
            if let Some(ref tx) = self.ws_tx {
                let password_hash = hash_password(&self.password);
                let _ = tx.send(WsMessage::AuthRequest { password_hash });
            }
        }

        fn process_messages(&mut self) {
            if let Some(ref mut rx) = self.ws_rx {
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        WsMessage::AuthResponse { success, error } => {
                            if success {
                                self.authenticated = true;
                                self.error_message = None;
                            } else {
                                self.error_message = error;
                                self.authenticated = false;
                            }
                        }
                        WsMessage::InitialState { worlds, current_world_index, settings, actions } => {
                            self.worlds = worlds.into_iter().map(|w| RemoteWorld {
                                name: w.name,
                                connected: w.connected,
                                output_lines: w.output_lines,
                                prompt: w.prompt,
                                settings: RemoteWorldSettings {
                                    hostname: w.settings.hostname,
                                    port: w.settings.port,
                                    user: w.settings.user,
                                    use_ssl: w.settings.use_ssl,
                                    keep_alive_type: w.keep_alive_type.clone(),
                                    keep_alive_cmd: w.settings.keep_alive_cmd.clone(),
                                },
                                unseen_lines: w.unseen_lines,
                                pending_count: w.pending_lines.len(),
                                last_send_secs: w.last_send_secs,
                                last_recv_secs: w.last_recv_secs,
                                last_nop_secs: w.last_nop_secs,
                            }).collect();
                            self.current_world = current_world_index;
                            self.console_theme = GuiTheme::from_name(&settings.console_theme);
                            self.theme = GuiTheme::from_name(&settings.gui_theme);
                            self.font_name = settings.font_name;
                            self.font_size = settings.font_size;
                            self.ws_allow_list = settings.ws_allow_list;
                            self.web_secure = settings.web_secure;
                            self.http_enabled = settings.http_enabled;
                            self.http_port = settings.http_port;
                            self.ws_enabled = settings.ws_enabled;
                            self.ws_port = settings.ws_port;
                            self.world_switch_mode = WorldSwitchMode::from_name(&settings.world_switch_mode);
                            self.debug_enabled = settings.debug_enabled;
                            self.more_mode = settings.more_mode_enabled;
                            self.spell_check_enabled = settings.spell_check_enabled;
                            self.show_tags = settings.show_tags;
                            self.actions = actions;
                        }
                        WsMessage::ServerData { world_index, data } => {
                            if world_index < self.worlds.len() {
                                // Parse and add new output lines, filtering out visually empty lines
                                let mut lines_added = 0;
                                for line in data.lines() {
                                    // Skip visually empty lines (only ANSI codes/whitespace)
                                    if !is_visually_empty(line) {
                                        self.worlds[world_index].output_lines.push(line.to_string());
                                        lines_added += 1;
                                    }
                                }
                                // Track unseen lines for non-current worlds
                                if world_index != self.current_world {
                                    self.worlds[world_index].unseen_lines += lines_added;
                                }
                            }
                        }
                        WsMessage::WorldConnected { world_index, name } => {
                            if world_index < self.worlds.len() {
                                self.worlds[world_index].connected = true;
                                self.worlds[world_index].name = name;
                            }
                        }
                        WsMessage::WorldDisconnected { world_index } => {
                            if world_index < self.worlds.len() {
                                self.worlds[world_index].connected = false;
                            }
                        }
                        WsMessage::WorldSwitched { new_index } => {
                            self.current_world = new_index;
                            // Mark seen when switching to a world
                            if new_index < self.worlds.len() {
                                self.worlds[new_index].unseen_lines = 0;
                            }
                        }
                        WsMessage::PromptUpdate { world_index, prompt } => {
                            if world_index < self.worlds.len() {
                                self.worlds[world_index].prompt = prompt;
                            }
                        }
                        WsMessage::WorldSettingsUpdated { world_index, settings, name } => {
                            // Update local world settings from server confirmation
                            if world_index < self.worlds.len() {
                                self.worlds[world_index].name = name;
                                self.worlds[world_index].settings.hostname = settings.hostname;
                                self.worlds[world_index].settings.port = settings.port;
                                self.worlds[world_index].settings.user = settings.user;
                                self.worlds[world_index].settings.use_ssl = settings.use_ssl;
                                self.worlds[world_index].settings.keep_alive_type = settings.keep_alive_type;
                                self.worlds[world_index].settings.keep_alive_cmd = settings.keep_alive_cmd;
                            }
                        }
                        WsMessage::GlobalSettingsUpdated { settings, input_height } => {
                            // Update local global settings from server confirmation
                            self.console_theme = GuiTheme::from_name(&settings.console_theme);
                            self.theme = GuiTheme::from_name(&settings.gui_theme);
                            self.input_height = input_height;
                            self.font_name = settings.font_name;
                            self.font_size = settings.font_size;
                            self.ws_allow_list = settings.ws_allow_list;
                            self.web_secure = settings.web_secure;
                            self.http_enabled = settings.http_enabled;
                            self.http_port = settings.http_port;
                            self.ws_enabled = settings.ws_enabled;
                            self.ws_port = settings.ws_port;
                            self.world_switch_mode = WorldSwitchMode::from_name(&settings.world_switch_mode);
                            self.debug_enabled = settings.debug_enabled;
                            self.more_mode = settings.more_mode_enabled;
                            self.spell_check_enabled = settings.spell_check_enabled;
                            self.show_tags = settings.show_tags;
                        }
                        WsMessage::PendingLinesUpdate { world_index, count } => {
                            // Update pending count for world
                            if world_index < self.worlds.len() {
                                self.worlds[world_index].pending_count = count;
                            }
                        }
                        WsMessage::ActionsUpdated { actions } => {
                            // Update local actions from server
                            self.actions = actions;
                        }
                        WsMessage::UnseenCleared { world_index } => {
                            // Another client (console or web) has viewed this world
                            if world_index < self.worlds.len() {
                                self.worlds[world_index].unseen_lines = 0;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        fn send_command(&mut self, world_index: usize, command: String) {
            // Store in command history (avoid duplicates of last command)
            if !command.is_empty()
                && self.command_history.last().map(|s| s.as_str()) != Some(&command)
            {
                self.command_history.push(command.clone());
            }
            // Reset history navigation
            self.history_index = 0;
            self.saved_input.clear();

            if let Some(ref tx) = self.ws_tx {
                let _ = tx.send(WsMessage::SendCommand { world_index, command });
            }
        }

        fn switch_world(&mut self, world_index: usize) {
            if let Some(ref tx) = self.ws_tx {
                let _ = tx.send(WsMessage::SwitchWorld { world_index });
            }
        }

        fn connect_world(&mut self, world_index: usize) {
            if let Some(ref tx) = self.ws_tx {
                let _ = tx.send(WsMessage::ConnectWorld { world_index });
            }
        }

        fn disconnect_world(&mut self, world_index: usize) {
            if let Some(ref tx) = self.ws_tx {
                let _ = tx.send(WsMessage::DisconnectWorld { world_index });
            }
        }

        /// Find all misspelled words in the input buffer (excluding word at cursor)
        fn find_misspelled_words(&self) -> Vec<(usize, usize)> {
            let mut misspelled = Vec::new();
            let chars: Vec<char> = self.input_buffer.chars().collect();
            let mut i = 0;

            // Simple cursor position estimate (egui doesn't expose cursor position easily)
            // We'll just check all words since we can't know where the cursor is during input_mut
            let cursor_char_pos = chars.len(); // Assume cursor at end for now

            while i < chars.len() {
                // Skip non-alphabetic characters
                while i < chars.len() && !chars[i].is_alphabetic() {
                    i += 1;
                }
                if i >= chars.len() {
                    break;
                }

                let start = i;
                while i < chars.len() && chars[i].is_alphabetic() {
                    i += 1;
                }
                let end = i;

                let word: String = chars[start..end].iter().collect();
                let cursor_in_word = cursor_char_pos >= start && cursor_char_pos <= end;

                // Don't mark the word currently being typed
                if !cursor_in_word && !self.spell_checker.is_valid(&word) {
                    misspelled.push((start, end));
                }
            }

            misspelled
        }

        /// Get the word at or before the cursor position
        fn current_word(&self) -> Option<(usize, usize, String)> {
            let chars: Vec<char> = self.input_buffer.chars().collect();
            if chars.is_empty() {
                return None;
            }

            // Assume cursor is at end of input (egui limitation)
            let cursor_pos = chars.len();
            if cursor_pos == 0 {
                return None;
            }

            // Find word boundaries around cursor
            let mut start = cursor_pos.saturating_sub(1);
            while start > 0 && chars[start - 1].is_alphabetic() {
                start -= 1;
            }
            if !chars[start].is_alphabetic() {
                return None;
            }

            let mut end = cursor_pos;
            while end < chars.len() && chars[end].is_alphabetic() {
                end += 1;
            }

            let word: String = chars[start..end].iter().collect();
            if word.is_empty() {
                return None;
            }

            Some((start, end, word))
        }

        /// Handle Ctrl+Q spell check
        fn handle_spell_check(&mut self) -> Option<String> {
            if !self.spell_state.showing_suggestions {
                // First press - find misspelled word and show suggestions
                if let Some((start, end, word)) = self.current_word() {
                    if !self.spell_checker.is_valid(&word) {
                        let mut suggestions = self.spell_checker.suggestions(&word, 6);
                        if !suggestions.is_empty() {
                            self.spell_state.original_word = word.clone();
                            suggestions.push(word); // Add original at end for cycling back

                            let display_suggestions: Vec<_> = suggestions[..suggestions.len()-1].to_vec();
                            let message = format!(
                                "Suggestions for '{}': {}",
                                self.spell_state.original_word,
                                display_suggestions.join(", ")
                            );

                            self.spell_state.suggestions = suggestions;
                            self.spell_state.suggestion_index = 0;
                            self.spell_state.word_start = start;
                            self.spell_state.word_end = end;
                            self.spell_state.showing_suggestions = true;
                            self.suggestion_message = Some(format!(
                                "Press Ctrl+Q to cycle: {}",
                                self.spell_state.suggestions[0]
                            ));
                            return Some(message);
                        }
                    }
                }
            } else if !self.spell_state.suggestions.is_empty() {
                // Subsequent press - cycle and apply suggestions
                let replacement = self.spell_state.suggestions[self.spell_state.suggestion_index].clone();

                // Replace word in input buffer
                let chars: Vec<char> = self.input_buffer.chars().collect();
                let before: String = chars[..self.spell_state.word_start].iter().collect();
                let after: String = if self.spell_state.word_end < chars.len() {
                    chars[self.spell_state.word_end..].iter().collect()
                } else {
                    String::new()
                };
                self.input_buffer = format!("{}{}{}", before, replacement, after);

                // Update word end position
                self.spell_state.word_end = self.spell_state.word_start + replacement.chars().count();
                self.spell_state.suggestion_index =
                    (self.spell_state.suggestion_index + 1) % self.spell_state.suggestions.len();

                let next_word = &self.spell_state.suggestions[self.spell_state.suggestion_index];
                if next_word == &self.spell_state.original_word {
                    self.suggestion_message = Some(format!(
                        "Applied '{}'. Next: '{}' (original)",
                        replacement, next_word
                    ));
                } else {
                    self.suggestion_message = Some(format!(
                        "Applied '{}'. Next: '{}'",
                        replacement, next_word
                    ));
                }
            }
            None
        }

        /// Reset spell state when cursor moves away from word
        fn reset_spell_state(&mut self) {
            self.spell_state.reset();
            self.suggestion_message = None;
        }

        fn update_world_settings(&mut self, world_index: usize) {
            if let Some(ref tx) = self.ws_tx {
                let _ = tx.send(WsMessage::UpdateWorldSettings {
                    world_index,
                    name: self.edit_name.clone(),
                    hostname: self.edit_hostname.clone(),
                    port: self.edit_port.clone(),
                    user: self.edit_user.clone(),
                    use_ssl: self.edit_ssl,
                    keep_alive_type: self.edit_keep_alive_type.name().to_string(),
                    keep_alive_cmd: self.edit_keep_alive_cmd.clone(),
                });
            }
        }

        fn update_global_settings(&mut self) {
            if let Some(ref tx) = self.ws_tx {
                let _ = tx.send(WsMessage::UpdateGlobalSettings {
                    console_theme: if self.console_theme == GuiTheme::Dark { "dark".to_string() } else { "light".to_string() },
                    gui_theme: if self.theme == GuiTheme::Dark { "dark".to_string() } else { "light".to_string() },
                    input_height: self.input_height,
                    font_name: self.font_name.clone(),
                    font_size: self.font_size,
                    ws_allow_list: self.ws_allow_list.clone(),
                    web_secure: self.web_secure,
                    http_enabled: self.http_enabled,
                    http_port: self.http_port,
                    ws_enabled: self.ws_enabled,
                    ws_port: self.ws_port,
                });
            }
        }

        fn update_actions(&mut self) {
            if let Some(ref tx) = self.ws_tx {
                let _ = tx.send(WsMessage::UpdateActions {
                    actions: self.actions.clone(),
                });
            }
        }

        fn open_world_editor(&mut self, world_index: usize) {
            if let Some(world) = self.worlds.get(world_index) {
                self.edit_name = world.name.clone();
                self.edit_hostname = world.settings.hostname.clone();
                self.edit_port = world.settings.port.clone();
                self.edit_user = world.settings.user.clone();
                self.edit_ssl = world.settings.use_ssl;
                self.edit_keep_alive_type = KeepAliveType::from_name(&world.settings.keep_alive_type);
                self.edit_keep_alive_cmd = world.settings.keep_alive_cmd.clone();
                self.popup_state = PopupState::WorldEditor(world_index);
            }
        }

        /// Strip ANSI escape codes for clipboard copy
        fn strip_ansi_for_copy(text: &str) -> String {
            let mut result = String::new();
            let mut chars = text.chars().peekable();

            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    // Skip escape sequence
                    if chars.peek() == Some(&'[') {
                        chars.next(); // consume '['
                        // Skip until we hit a letter
                        while let Some(&sc) = chars.peek() {
                            chars.next();
                            if sc.is_ascii_alphabetic() {
                                break;
                            }
                        }
                    }
                } else {
                    result.push(c);
                }
            }

            result
        }

        /// Strip MUD tags like [channel:] or [channel(player)] from start of line
        fn strip_mud_tags(text: &str) -> String {
            let trimmed = text.trim_start();
            if trimmed.starts_with('[') {
                // Find the closing bracket
                if let Some(end) = trimmed.find(']') {
                    let tag = &trimmed[1..end];
                    // Check if it looks like a MUD tag (contains : or parentheses)
                    if tag.contains(':') || tag.contains('(') {
                        // Return the rest of the line, preserving original leading whitespace
                        let leading_ws = text.len() - trimmed.len();
                        let after_tag = &trimmed[end + 1..];
                        // Trim one space after tag if present
                        let after_tag = after_tag.strip_prefix(' ').unwrap_or(after_tag);
                        return format!("{}{}", &text[..leading_ws], after_tag);
                    }
                }
            }
            text.to_string()
        }

        /// Strip MUD tags from ANSI text while preserving color codes
        fn strip_mud_tags_ansi(text: &str) -> String {
            // First, find any leading whitespace
            let trimmed = text.trim_start();
            let leading_ws_len = text.len() - trimmed.len();
            let leading_ws = &text[..leading_ws_len];

            // Check if line starts with [ (possibly after ANSI codes)
            // Need to skip ANSI codes to find the actual start
            let mut chars = trimmed.chars().peekable();
            let mut ansi_prefix = String::new();
            let mut in_ansi = false;

            while let Some(c) = chars.next() {
                if c == '\x1b' && chars.peek() == Some(&'[') {
                    ansi_prefix.push(c);
                    in_ansi = true;
                } else if in_ansi {
                    ansi_prefix.push(c);
                    if c.is_ascii_alphabetic() {
                        in_ansi = false;
                    }
                } else if c == '[' {
                    // Found the start of a potential tag
                    // Look for closing bracket
                    let rest: String = chars.collect();
                    if let Some(end) = rest.find(']') {
                        let tag = &rest[..end];
                        if tag.contains(':') || tag.contains('(') {
                            // It's a MUD tag, skip it
                            let after_tag = &rest[end + 1..];
                            let after_tag = after_tag.strip_prefix(' ').unwrap_or(after_tag);
                            return format!("{}{}{}", leading_ws, ansi_prefix, after_tag);
                        } else {
                            // Not a MUD tag, return original
                            return text.to_string();
                        }
                    } else {
                        return text.to_string();
                    }
                } else {
                    // Not a tag start, return original
                    return text.to_string();
                }
            }
            text.to_string()
        }

        /// Convert 256-color palette index to RGB
        fn color256_to_rgb(n: u8, is_light_theme: bool) -> (u8, u8, u8) {
            match n {
                // Standard colors (0-7)
                0 => (0, 0, 0),
                1 => (205, 49, 49),
                2 => (13, 188, 121),
                3 => if is_light_theme { (160, 140, 0) } else { (229, 229, 16) },
                4 => (36, 114, 200),
                5 => (188, 63, 188),
                6 => (17, 168, 205),
                7 => if is_light_theme { (80, 80, 80) } else { (229, 229, 229) },
                // High-intensity colors (8-15)
                8 => (102, 102, 102),
                9 => (241, 76, 76),
                10 => (35, 209, 139),
                11 => if is_light_theme { (180, 160, 0) } else { (245, 245, 67) },
                12 => (59, 142, 234),
                13 => (214, 112, 214),
                14 => (41, 184, 219),
                15 => if is_light_theme { (40, 40, 40) } else { (255, 255, 255) },
                // 216 colors (16-231): 6x6x6 color cube
                16..=231 => {
                    let n = n - 16;
                    let r = (n / 36) % 6;
                    let g = (n / 6) % 6;
                    let b = n % 6;
                    let to_255 = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                    (to_255(r), to_255(g), to_255(b))
                }
                // Grayscale (232-255): 24 shades
                232..=255 => {
                    let gray = 8 + (n - 232) * 10;
                    (gray, gray, gray)
                }
            }
        }

        /// Append ANSI-colored text to an existing LayoutJob
        fn append_ansi_to_job(text: &str, default_color: egui::Color32, font_id: egui::FontId, job: &mut egui::text::LayoutJob, is_light_theme: bool) {
            let mut current_color = default_color;
            let mut bold = false;
            let mut chars = text.chars().peekable();
            let mut segment = String::new();

            while let Some(c) = chars.next() {
                if c == '\x1b' && chars.peek() == Some(&'[') {
                    // Flush current segment
                    if !segment.is_empty() {
                        let color = if bold {
                            egui::Color32::from_rgb(
                                (current_color.r() as u16 * 4 / 3).min(255) as u8,
                                (current_color.g() as u16 * 4 / 3).min(255) as u8,
                                (current_color.b() as u16 * 4 / 3).min(255) as u8,
                            )
                        } else {
                            current_color
                        };
                        job.append(&segment, 0.0, egui::TextFormat {
                            font_id: font_id.clone(),
                            color,
                            ..Default::default()
                        });
                        segment.clear();
                    }

                    // Parse escape sequence
                    chars.next(); // consume '['
                    let mut code = String::new();
                    while let Some(&sc) = chars.peek() {
                        if sc.is_ascii_alphabetic() {
                            chars.next();
                            break;
                        }
                        chars.next();
                        code.push(sc);
                    }

                    // Parse SGR codes (semicolon-separated)
                    let parts: Vec<&str> = code.split(';').collect();
                    let mut i = 0;
                    while i < parts.len() {
                        match parts[i].parse::<u8>().unwrap_or(0) {
                            0 => { current_color = default_color; bold = false; }
                            1 => bold = true,
                            22 => bold = false,
                            // Standard foreground colors (30-37)
                            30 => current_color = egui::Color32::from_rgb(0, 0, 0),
                            31 => current_color = egui::Color32::from_rgb(205, 49, 49),
                            32 => current_color = egui::Color32::from_rgb(13, 188, 121),
                            33 => current_color = if is_light_theme {
                                egui::Color32::from_rgb(160, 140, 0)  // Darker gold for light theme
                            } else {
                                egui::Color32::from_rgb(229, 229, 16)
                            },
                            34 => current_color = egui::Color32::from_rgb(36, 114, 200),
                            35 => current_color = egui::Color32::from_rgb(188, 63, 188),
                            36 => current_color = egui::Color32::from_rgb(17, 168, 205),
                            37 => current_color = if is_light_theme {
                                egui::Color32::from_rgb(80, 80, 80)  // Dark gray for light theme
                            } else {
                                egui::Color32::from_rgb(229, 229, 229)
                            },
                            39 => current_color = default_color,
                            // Bright/high-intensity foreground colors (90-97)
                            90 => current_color = egui::Color32::from_rgb(102, 102, 102),
                            91 => current_color = egui::Color32::from_rgb(241, 76, 76),
                            92 => current_color = egui::Color32::from_rgb(35, 209, 139),
                            93 => current_color = if is_light_theme {
                                egui::Color32::from_rgb(180, 160, 0)  // Darker gold for light theme
                            } else {
                                egui::Color32::from_rgb(245, 245, 67)
                            },
                            94 => current_color = egui::Color32::from_rgb(59, 142, 234),
                            95 => current_color = egui::Color32::from_rgb(214, 112, 214),
                            96 => current_color = egui::Color32::from_rgb(41, 184, 219),
                            97 => current_color = if is_light_theme {
                                egui::Color32::from_rgb(40, 40, 40)  // Near black for light theme
                            } else {
                                egui::Color32::from_rgb(255, 255, 255)
                            },
                            // Extended color modes
                            38 => {
                                // 38;5;n = 256-color, 38;2;r;g;b = 24-bit RGB
                                if i + 1 < parts.len() {
                                    match parts[i + 1].parse::<u8>().unwrap_or(0) {
                                        5 => {
                                            // 256-color mode: 38;5;n
                                            if i + 2 < parts.len() {
                                                if let Ok(n) = parts[i + 2].parse::<u8>() {
                                                    let (r, g, b) = Self::color256_to_rgb(n, is_light_theme);
                                                    current_color = egui::Color32::from_rgb(r, g, b);
                                                }
                                                i += 2;
                                            }
                                        }
                                        2 => {
                                            // 24-bit RGB mode: 38;2;r;g;b
                                            if i + 4 < parts.len() {
                                                let r = parts[i + 2].parse::<u8>().unwrap_or(0);
                                                let g = parts[i + 3].parse::<u8>().unwrap_or(0);
                                                let b = parts[i + 4].parse::<u8>().unwrap_or(0);
                                                current_color = egui::Color32::from_rgb(r, g, b);
                                                i += 4;
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            // Background colors (40-47, 100-107) - ignored for now as egui TextFormat
                            // doesn't easily support per-character backgrounds
                            _ => {}
                        }
                        i += 1;
                    }
                } else {
                    segment.push(c);
                }
            }

            // Flush remaining segment
            if !segment.is_empty() {
                let color = if bold {
                    egui::Color32::from_rgb(
                        (current_color.r() as u16 * 4 / 3).min(255) as u8,
                        (current_color.g() as u16 * 4 / 3).min(255) as u8,
                        (current_color.b() as u16 * 4 / 3).min(255) as u8,
                    )
                } else {
                    current_color
                };
                job.append(&segment, 0.0, egui::TextFormat {
                    font_id: font_id.clone(),
                    color,
                    ..Default::default()
                });
            }
        }

        /// Find URLs in text and return their positions
        fn find_urls(text: &str) -> Vec<(usize, usize, String)> {
            let mut urls = Vec::new();
            // Simple URL detection: look for http:// or https://
            let mut search_start = 0;
            while search_start < text.len() {
                let remaining = &text[search_start..];
                // Find the earliest http:// or https://
                let http_pos = remaining.find("http://");
                let https_pos = remaining.find("https://");

                let rel_pos = match (http_pos, https_pos) {
                    (Some(h), Some(hs)) => Some(h.min(hs)),
                    (Some(h), None) => Some(h),
                    (None, Some(hs)) => Some(hs),
                    (None, None) => None,
                };

                if let Some(rel_pos) = rel_pos {
                    let start = search_start + rel_pos;
                    // Find end of URL (space, newline, or end of string)
                    let end = text[start..].find(|c: char| c.is_whitespace() || c == '>' || c == '"' || c == '\'' || c == ')' || c == ']')
                        .map(|e| start + e)
                        .unwrap_or(text.len());
                    if end > start {
                        urls.push((start, end, text[start..end].to_string()));
                    }
                    search_start = end;
                } else {
                    break;
                }
            }
            urls
        }

        /// Open a URL in the default browser
        fn open_url(url: &str) {
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("xdg-open")
                    .arg(url)
                    .spawn();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open")
                    .arg(url)
                    .spawn();
            }
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", url])
                    .spawn();
            }
        }
    }

    impl eframe::App for RemoteGuiApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            // Process incoming WebSocket messages
            self.process_messages();

            // Apply theme to egui visuals
            let theme = self.theme;
            let mut visuals = if theme == GuiTheme::Dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };

            // Customize based on our theme
            visuals.override_text_color = Some(theme.fg());
            visuals.panel_fill = theme.panel_bg();
            visuals.window_fill = theme.panel_bg();
            visuals.widgets.noninteractive.bg_fill = theme.button_bg();
            visuals.widgets.inactive.bg_fill = theme.button_bg();
            visuals.widgets.hovered.bg_fill = theme.selection_bg();
            visuals.widgets.active.bg_fill = theme.selection_bg();
            visuals.selection.bg_fill = theme.selection_bg();
            ctx.set_visuals(visuals);

            // Load custom font if font name changed
            if self.loaded_font_name != self.font_name {
                self.loaded_font_name = self.font_name.clone();

                let mut fonts = egui::FontDefinitions::default();

                if !self.font_name.is_empty() {
                    // Try to load the system font
                    if let Some(font_data) = Self::find_system_font(&self.font_name) {
                        // Add the custom font
                        fonts.font_data.insert(
                            "custom_mono".to_owned(),
                            std::sync::Arc::new(egui::FontData::from_owned(font_data)),
                        );

                        // Make it the first priority for monospace
                        fonts.families
                            .entry(egui::FontFamily::Monospace)
                            .or_default()
                            .insert(0, "custom_mono".to_owned());

                        // Also use for proportional text
                        fonts.families
                            .entry(egui::FontFamily::Proportional)
                            .or_default()
                            .insert(0, "custom_mono".to_owned());
                    }
                }
                // If font_name is empty or font not found, use defaults (already in fonts)

                ctx.set_fonts(fonts);
            }

            // Apply font size to monospace text style
            let mut style = (*ctx.style()).clone();
            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Monospace) {
                font_id.size = self.font_size;
            }
            // Also apply to body text
            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Body) {
                font_id.size = self.font_size;
            }
            ctx.set_style(style);

            // Request repaint to keep polling messages
            ctx.request_repaint();

            if !self.connected || !self.authenticated {
                // Show login dialog with dog and Clay branding
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);

                        // Dog ASCII art and Clay title - matching console splash colors
                        // ANSI 256-color to RGB: 180=#d7af87, 209=#ff875f, 208=#ff8700, 215=#ffaf5f
                        // 216=#ffaf87, 217=#ffafaf, 218=#ffafd7, 213=#ff87ff, 244=#808080
                        let dog_color = egui::Color32::from_rgb(0xd7, 0xaf, 0x87);  // tan/gold (180)
                        let clay_colors = [
                            egui::Color32::from_rgb(0xff, 0x87, 0x5f),  // 209
                            egui::Color32::from_rgb(0xff, 0x87, 0x00),  // 208
                            egui::Color32::from_rgb(0xff, 0xaf, 0x5f),  // 215
                            egui::Color32::from_rgb(0xff, 0xaf, 0x87),  // 216
                            egui::Color32::from_rgb(0xff, 0xaf, 0xaf),  // 217
                            egui::Color32::from_rgb(0xff, 0xaf, 0xd7),  // 218
                        ];
                        let tagline_color = egui::Color32::from_rgb(0xff, 0x87, 0xff);  // 213
                        let help_color = egui::Color32::from_rgb(0x80, 0x80, 0x80);  // 244

                        // Splash art: dog on left, CLAY block letters on right
                        let dog_lines = [
                            "          (\\/\\__o     ",
                            "  __      `-/ `_/     ",
                            " `--\\______/  |       ",
                            "    /        /        ",
                            " -`/_------'\\_.       ",
                            "                       ",
                        ];
                        let clay_lines = [
                            " ██████╗██╗      █████╗ ██╗   ██╗",
                            "██╔════╝██║     ██╔══██╗╚██╗ ██╔╝",
                            "██║     ██║     ███████║ ╚████╔╝ ",
                            "██║     ██║     ██╔══██║  ╚██╔╝  ",
                            "╚██████╗███████╗██║  ██║   ██║   ",
                            "╚═════╝╚══════╝╚═╝  ╚═╝   ╚═╝   ",
                        ];

                        ui.vertical(|ui| {
                            for (i, (dog, clay)) in dog_lines.iter().zip(clay_lines.iter()).enumerate() {
                                ui.horizontal(|ui| {
                                    ui.add_space(ui.available_width() / 2.0 - 250.0);
                                    ui.label(egui::RichText::new(*dog).color(dog_color).monospace());
                                    ui.label(egui::RichText::new(*clay).color(clay_colors[i]).monospace());
                                });
                            }
                        });

                        ui.add_space(10.0);
                        ui.label(egui::RichText::new("✨ A 90dies mud client written today ✨").color(tagline_color).italics());
                        ui.add_space(5.0);
                        ui.label(egui::RichText::new("/help for how to use clay").color(help_color));
                        ui.add_space(15.0);

                        ui.label(format!("Server: {}", self.ws_url));
                        ui.add_space(10.0);

                        // Auto-connect on first frame to check if allow list grants access
                        if !self.auto_connect_attempted && !self.connected {
                            self.auto_connect_attempted = true;
                            self.connect_websocket();
                        }

                        // Check if we're still waiting for allow list response (500ms timeout)
                        let allow_list_timeout = std::time::Duration::from_millis(500);
                        let still_checking_allow_list = self.connected
                            && !self.authenticated
                            && !self.password_submitted
                            && self.connect_time.map_or(false, |t| t.elapsed() < allow_list_timeout);

                        // Show connection status or password prompt
                        if still_checking_allow_list {
                            ui.label("Checking allow list...");
                            ui.add_space(10.0);
                            // Request repaint to update when timeout expires
                            ctx.request_repaint();
                        }

                        ui.label("Password:");
                        let password_edit = TextEdit::singleline(&mut self.password)
                            .password(true)
                            .desired_width(200.0);
                        let response = ui.add(password_edit);

                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.password_submitted = true;
                            // If already connected, just send auth; otherwise reconnect
                            if self.connected {
                                self.send_auth();
                            } else {
                                self.connect_websocket();
                            }
                        }

                        ui.add_space(10.0);
                        if ui.button("Connect").clicked() {
                            self.password_submitted = true;
                            if self.connected {
                                self.send_auth();
                            } else {
                                self.connect_websocket();
                            }
                        }

                        if let Some(ref err) = self.error_message {
                            ui.add_space(10.0);
                            ui.colored_label(theme.error(), err);
                        }
                    });
                });
            } else {
                // Show main interface with menu bar
                let mut action: Option<&str> = None;

                // Handle keyboard shortcuts (only when no popup is open)
                if self.popup_state == PopupState::None && !self.filter_active {
                    let mut switch_world: Option<usize> = None;
                    let mut history_action: Option<i32> = None; // -1 = prev, 1 = next
                    let mut scroll_action: Option<i32> = None; // -1 = up, 1 = down
                    let mut clear_input = false;
                    let mut delete_word = false;
                    let mut resize_input: i32 = 0;

                    // Use input_mut to consume events before widgets get them
                    ctx.input_mut(|i| {
                        // Ctrl+key shortcuts
                        if i.modifiers.ctrl {
                            if i.consume_key(egui::Modifiers::CTRL, egui::Key::L) {
                                action = Some("world_list");
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::E) {
                                action = Some("edit_current");
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::S) {
                                action = Some("setup");
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::U) {
                                // Ctrl+U - clear input (like console)
                                clear_input = true;
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::O) {
                                action = Some("connect");
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::D) {
                                action = Some("disconnect");
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::P) {
                                // Ctrl+P - previous command history
                                history_action = Some(-1);
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::N) {
                                // Ctrl+N - next command history
                                history_action = Some(1);
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::ArrowUp) {
                                // Ctrl+Up - resize input smaller
                                resize_input = -1;
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::ArrowDown) {
                                // Ctrl+Down - resize input larger
                                resize_input = 1;
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::W) {
                                // Ctrl+W - delete word before cursor
                                delete_word = true;
                            } else if i.consume_key(egui::Modifiers::CTRL, egui::Key::Q) {
                                // Ctrl+Q - spell check
                                action = Some("spell_check");
                            }
                        } else if i.modifiers.shift {
                            // Shift+Up/Down - cycle through all worlds
                            if i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowUp) {
                                if !self.worlds.is_empty() {
                                    let prev = if self.current_world == 0 {
                                        self.worlds.len() - 1
                                    } else {
                                        self.current_world - 1
                                    };
                                    switch_world = Some(prev);
                                }
                            } else if i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowDown) {
                                if !self.worlds.is_empty() {
                                    let next = (self.current_world + 1) % self.worlds.len();
                                    switch_world = Some(next);
                                }
                            }
                        } else {
                            // Non-modified keys - consume to prevent widgets from handling
                            if i.consume_key(egui::Modifiers::NONE, egui::Key::PageUp) {
                                scroll_action = Some(-1);
                            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::PageDown) {
                                scroll_action = Some(1);
                            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F2) {
                                // F2 - toggle MUD tag display
                                self.show_tags = !self.show_tags;
                            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F4) {
                                // F4 - toggle filter popup
                                self.filter_active = true;
                                self.filter_text.clear();
                            } else if self.input_buffer.trim().is_empty() {
                                // Up/Down arrow with empty input - cycle active worlds
                                // Use shared world switching logic
                                let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
                                    .map(|w| crate::util::WorldSwitchInfo {
                                        name: w.name.clone(),
                                        connected: w.connected,
                                        unseen_lines: w.unseen_lines,
                                    })
                                    .collect();

                                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                                    if let Some(prev_idx) = crate::util::calculate_prev_world(
                                        &world_info,
                                        self.current_world,
                                        self.world_switch_mode,
                                    ) {
                                        switch_world = Some(prev_idx);
                                    }
                                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                                    if let Some(next_idx) = crate::util::calculate_next_world(
                                        &world_info,
                                        self.current_world,
                                        self.world_switch_mode,
                                    ) {
                                        switch_world = Some(next_idx);
                                    }
                                }
                            }
                        }
                    });

                    // Apply clear input
                    if clear_input {
                        self.input_buffer.clear();
                        self.history_index = 0;
                    }

                    // Apply delete word
                    if delete_word && !self.input_buffer.is_empty() {
                        // Delete one word before cursor (end of buffer)
                        // First, skip any trailing whitespace
                        while self.input_buffer.ends_with(|c: char| c.is_whitespace()) {
                            self.input_buffer.pop();
                        }
                        // Then delete the word (non-whitespace characters)
                        while !self.input_buffer.is_empty()
                            && !self.input_buffer.ends_with(|c: char| c.is_whitespace())
                        {
                            self.input_buffer.pop();
                        }
                    }

                    // Apply input resize
                    if resize_input != 0 {
                        let new_height = (self.input_height as i32 + resize_input).clamp(1, 15) as u16;
                        self.input_height = new_height;
                    }

                    // Apply history navigation
                    if let Some(dir) = history_action {
                        if dir < 0 {
                            // Previous (older)
                            if self.history_index == 0 && !self.command_history.is_empty() {
                                self.saved_input = self.input_buffer.clone();
                                self.history_index = 1;
                                self.input_buffer = self.command_history[self.command_history.len() - 1].clone();
                            } else if self.history_index > 0 && self.history_index < self.command_history.len() {
                                self.history_index += 1;
                                let idx = self.command_history.len() - self.history_index;
                                self.input_buffer = self.command_history[idx].clone();
                            }
                        } else {
                            // Next (newer)
                            if self.history_index > 1 {
                                self.history_index -= 1;
                                let idx = self.command_history.len() - self.history_index;
                                self.input_buffer = self.command_history[idx].clone();
                            } else if self.history_index == 1 {
                                self.history_index = 0;
                                self.input_buffer = std::mem::take(&mut self.saved_input);
                            }
                        }
                    }

                    // Apply world switch (GUI-local only, doesn't affect console)
                    if let Some(new_world) = switch_world {
                        if new_world != self.current_world && new_world < self.worlds.len() {
                            self.current_world = new_world;
                            self.worlds[new_world].unseen_lines = 0;
                            self.scroll_offset = None; // Reset scroll
                            // Notify server so other clients sync their unseen counts
                            if let Some(ref tx) = self.ws_tx {
                                let _ = tx.send(WsMessage::MarkWorldSeen { world_index: new_world });
                            }
                        }
                    }

                    // Handle scroll action (will be used by scroll area)
                    // In egui, scroll_offset is from the TOP, so:
                    // - PageUp = decrease offset (scroll towards top/older content)
                    // - PageDown = increase offset (scroll towards bottom/newer content)
                    if let Some(dir) = scroll_action {
                        if dir < 0 {
                            // Scroll up (PageUp) - decrease offset to show older content
                            if let Some(offset) = self.scroll_offset {
                                let new_offset = (offset - 300.0).max(0.0);
                                self.scroll_offset = Some(new_offset);
                            } else {
                                // Currently at bottom, start scrolling up from max offset
                                let new_offset = (self.scroll_max_offset - 300.0).max(0.0);
                                self.scroll_offset = Some(new_offset);
                            }
                        } else {
                            // Scroll down (PageDown) - increase offset to show newer content
                            if let Some(offset) = self.scroll_offset {
                                let new_offset = offset + 300.0;
                                // If we're within one page of the bottom, snap to bottom
                                if new_offset >= self.scroll_max_offset - 10.0 {
                                    self.scroll_offset = None;
                                } else {
                                    self.scroll_offset = Some(new_offset);
                                }
                            }
                            // If scroll_offset is None, we're already at bottom, nothing to do
                        }
                    }
                }

                // Handle filter popup escape
                if self.filter_active {
                    ctx.input(|i| {
                        if i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::F4) {
                            self.filter_active = false;
                            self.filter_text.clear();
                        }
                    });
                }

                egui::TopBottomPanel::top("menu_bar")
                    .frame(egui::Frame::none()
                        .fill(theme.bg())
                        .inner_margin(egui::Margin::symmetric(4.0, 5.0))  // 3px extra padding top/bottom
                        .stroke(egui::Stroke::NONE))
                    .show(ctx, |ui| {
                    ui.horizontal_centered(|ui| {
                        // Hamburger menu (☰) - 44% larger than base
                        let hamburger_size = 23.0;  // 16 * 1.2 * 1.2 = 23.04
                        ui.menu_button(egui::RichText::new("☰").size(hamburger_size), |ui| {
                            if ui.button("Worlds List").clicked() {
                                action = Some("connected_worlds");
                                ui.close_menu();
                            }
                            if ui.button("World Selector").clicked() {
                                action = Some("world_list");
                                ui.close_menu();
                            }
                            if ui.button("Actions").clicked() {
                                action = Some("actions");
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Toggle Tags").clicked() {
                                action = Some("toggle_tags");
                                ui.close_menu();
                            }
                        });

                        // S/M/L font size buttons (centered vertically via horizontal_centered)
                        let btn_size = 16.0;
                        let small_text = if self.font_size <= 12.0 {
                            egui::RichText::new("S").size(btn_size).strong()
                        } else {
                            egui::RichText::new("S").size(btn_size)
                        };
                        if ui.button(small_text).on_hover_text("Small font").clicked() {
                            self.font_size = 12.0;
                            action = Some("font_changed");
                        }

                        let medium_text = if self.font_size > 12.0 && self.font_size <= 16.0 {
                            egui::RichText::new("M").size(btn_size).strong()
                        } else {
                            egui::RichText::new("M").size(btn_size)
                        };
                        if ui.button(medium_text).on_hover_text("Medium font").clicked() {
                            self.font_size = 14.0;
                            action = Some("font_changed");
                        }

                        let large_text = if self.font_size > 16.0 {
                            egui::RichText::new("L").size(btn_size).strong()
                        } else {
                            egui::RichText::new("L").size(btn_size)
                        };
                        if ui.button(large_text).on_hover_text("Large font").clicked() {
                            self.font_size = 18.0;
                            action = Some("font_changed");
                        }
                    });
                });

                // Handle menu actions
                match action {
                    Some("world_list") => {
                        self.popup_state = PopupState::WorldList;
                        self.world_list_selected = self.current_world;
                    }
                    Some("connected_worlds") => {
                        self.popup_state = PopupState::ConnectedWorlds;
                        self.world_list_selected = self.current_world;
                    }
                    Some("actions") => {
                        self.popup_state = PopupState::ActionsList;
                        self.actions_selected = 0;
                        self.action_error = None;
                    }
                    Some("edit_current") => self.open_world_editor(self.current_world),
                    Some("setup") => self.popup_state = PopupState::Setup,
                    Some("font") => {
                        self.edit_font_name = self.font_name.clone();
                        self.edit_font_size = self.font_size.to_string();
                        self.popup_state = PopupState::Font;
                    }
                    Some("font_changed") => {
                        // Font size was changed via S/M/L buttons - update server settings
                        self.update_global_settings();
                    }
                    Some("connect") => self.connect_world(self.current_world),
                    Some("disconnect") => self.disconnect_world(self.current_world),
                    Some("toggle_tags") => self.show_tags = !self.show_tags,
                    Some("spell_check") => {
                        if let Some(message) = self.handle_spell_check() {
                            // Add suggestion message to current world's output
                            if let Some(world) = self.worlds.get_mut(self.current_world) {
                                world.output_lines.push(message);
                            }
                        }
                    }
                    Some("help") => self.popup_state = PopupState::Help,
                    _ => {}
                }

                // Input area at bottom (full width)
                let input_height = self.input_height as f32 * 16.0 + 8.0;
                let prompt_text = self.worlds.get(self.current_world)
                    .map(|w| Self::strip_ansi_for_copy(&w.prompt))
                    .unwrap_or_default();

                egui::TopBottomPanel::bottom("input_panel")
                    .exact_height(input_height)
                    .frame(egui::Frame::none()
                        .fill(theme.bg())
                        .inner_margin(egui::Margin::same(2.0))
                        .stroke(egui::Stroke::NONE))
                    .show(ctx, |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0; // Remove horizontal spacing
                        ui.horizontal(|ui| {
                            // Show prompt if present (cyan colored like TUI)
                            if !prompt_text.is_empty() {
                                ui.label(egui::RichText::new(&prompt_text)
                                    .monospace()
                                    .color(theme.prompt()));
                            }

                            // Text input takes remaining width (no border)
                            // Build layout job with spell check coloring (misspelled words in red)
                            let input_id = egui::Id::new("main_input");
                            let misspelled = self.find_misspelled_words();
                            let font_id = egui::FontId::monospace(self.font_size);
                            let default_color = theme.fg();

                            // Build layouter using actual text parameter (not pre-computed job)
                            // This ensures cursor positioning works correctly when text changes
                            let misspelled_ranges = misspelled;
                            let layouter_font_id = font_id.clone();
                            let layouter_default_color = default_color;

                            let response = ui.add_sized(
                                ui.available_size(),
                                TextEdit::multiline(&mut self.input_buffer)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_rows(self.input_height as usize)
                                    .margin(egui::Margin::ZERO)
                                    .frame(false)
                                    .id(input_id)
                                    .layouter(&mut |_ui, text, wrap_width| {
                                        // Build layout job from actual text parameter
                                        let mut job = egui::text::LayoutJob::default();
                                        let chars: Vec<char> = text.chars().collect();
                                        let mut pos = 0;

                                        // Use misspelled ranges (may be slightly stale for one frame)
                                        let mut ranges = misspelled_ranges.clone();
                                        ranges.sort_by_key(|(start, _)| *start);

                                        for (start, end) in ranges {
                                            if start >= chars.len() || end > chars.len() {
                                                continue; // Skip stale ranges
                                            }
                                            // Add normal text before misspelled word
                                            if pos < start {
                                                let normal_text: String = chars[pos..start].iter().collect();
                                                job.append(&normal_text, 0.0, egui::TextFormat {
                                                    font_id: layouter_font_id.clone(),
                                                    color: layouter_default_color,
                                                    ..Default::default()
                                                });
                                            }
                                            // Add misspelled word in red
                                            let misspelled_text: String = chars[start..end].iter().collect();
                                            job.append(&misspelled_text, 0.0, egui::TextFormat {
                                                font_id: layouter_font_id.clone(),
                                                color: egui::Color32::RED,
                                                ..Default::default()
                                            });
                                            pos = end;
                                        }
                                        // Add remaining text
                                        if pos < chars.len() {
                                            let remaining: String = chars[pos..].iter().collect();
                                            job.append(&remaining, 0.0, egui::TextFormat {
                                                font_id: layouter_font_id.clone(),
                                                color: layouter_default_color,
                                                ..Default::default()
                                            });
                                        }
                                        // Handle empty text
                                        if chars.is_empty() {
                                            job.append("", 0.0, egui::TextFormat {
                                                font_id: layouter_font_id.clone(),
                                                color: layouter_default_color,
                                                ..Default::default()
                                            });
                                        }

                                        job.wrap = egui::text::TextWrapping {
                                            max_width: wrap_width,
                                            ..Default::default()
                                        };
                                        _ui.fonts(|f| f.layout_job(job))
                                    })
                            );

                            // If input doesn't have focus and a printable key is pressed,
                            // refocus the input and capture the typed text
                            if !response.has_focus() && self.popup_state == PopupState::None && !self.filter_active {
                                let typed_text: Option<String> = ctx.input(|i| {
                                    // Find any text that was typed
                                    for e in &i.events {
                                        if let egui::Event::Text(text) = e {
                                            return Some(text.clone());
                                        }
                                    }
                                    None
                                });
                                if let Some(text) = typed_text {
                                    // Add the typed text to input buffer and refocus
                                    self.input_buffer.push_str(&text);
                                    response.request_focus();
                                    // Set cursor position to end of buffer (create state if needed)
                                    let mut state = egui::TextEdit::load_state(ctx, input_id)
                                        .unwrap_or_default();
                                    let ccursor = egui::text::CCursor::new(self.input_buffer.len());
                                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                                    state.store(ctx, input_id);
                                }
                            }

                            // Send on Enter (without Shift)
                            if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
                                if self.input_buffer.ends_with('\n') {
                                    self.input_buffer.pop();
                                }
                                let cmd = std::mem::take(&mut self.input_buffer);
                                // Reset spell state when sending command
                                self.reset_spell_state();
                                if !cmd.is_empty() {
                                    // Use shared command parsing
                                    let parsed = super::parse_command(&cmd);

                                    // Handle local GUI popup commands
                                    match parsed {
                                        super::Command::Setup => {
                                            self.popup_state = PopupState::Setup;
                                        }
                                        super::Command::Web => {
                                            self.popup_state = PopupState::Web;
                                        }
                                        super::Command::WorldSelector => {
                                            self.popup_state = PopupState::WorldList;
                                            self.world_list_selected = self.current_world;
                                        }
                                        super::Command::WorldsList => {
                                            self.popup_state = PopupState::ConnectedWorlds;
                                            self.world_list_selected = self.current_world;
                                        }
                                        super::Command::Help => {
                                            self.popup_state = PopupState::Help;
                                        }
                                        super::Command::Actions => {
                                            self.actions_selected = 0;
                                            self.popup_state = PopupState::ActionsList;
                                        }
                                        super::Command::WorldEdit { name } => {
                                            // Open world editor
                                            let idx = if let Some(ref world_name) = name {
                                                self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(world_name))
                                                    .unwrap_or(self.current_world)
                                            } else {
                                                self.current_world
                                            };
                                            self.open_world_editor(idx);
                                        }
                                        _ => {
                                            // Check for /font which is GUI-specific
                                            if cmd.trim().eq_ignore_ascii_case("/font") {
                                                self.edit_font_name = self.font_name.clone();
                                                self.edit_font_size = format!("{:.1}", self.font_size);
                                                self.popup_state = PopupState::Font;
                                            } else {
                                                // Send other commands to server
                                                self.send_command(self.current_world, cmd);
                                            }
                                        }
                                    }
                                }
                            }

                            // Show suggestion message if present
                            if let Some(ref msg) = self.suggestion_message {
                                ui.label(egui::RichText::new(msg).color(theme.prompt()).monospace());
                            }
                        });
                    });

                // Separator bar (matches TUI style)
                let separator_bg = match theme {
                    GuiTheme::Dark => egui::Color32::from_rgb(40, 40, 40),
                    GuiTheme::Light => egui::Color32::from_rgb(200, 200, 200),  // Darker for light theme
                };
                egui::TopBottomPanel::bottom("separator_bar")
                    .exact_height(20.0)
                    .frame(egui::Frame::none()
                        .fill(separator_bg)
                        .inner_margin(egui::Margin::same(0.0))
                        .stroke(egui::Stroke::NONE))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            // Get current world info
                            let world_name = self.worlds.get(self.current_world)
                                .map(|w| w.name.as_str())
                                .unwrap_or("---");
                            let connected = self.worlds.get(self.current_world)
                                .map(|w| w.connected)
                                .unwrap_or(false);

                            // Collect worlds with activity (unseen output)
                            let worlds_with_activity: Vec<&str> = self.worlds.iter()
                                .enumerate()
                                .filter(|(i, w)| *i != self.current_world && w.unseen_lines > 0)
                                .map(|(_, w)| w.name.as_str())
                                .collect();
                            let activity_count = worlds_with_activity.len();

                            // Status indicator (More/Hist or underscores)
                            // Status area - spaces instead of underscores when no More/Hist indicator
                            let status_text = "           ";
                            ui.label(egui::RichText::new(status_text).monospace());

                            // Underscore padding
                            ui.label(egui::RichText::new(" ").monospace().color(theme.fg_dim()));

                            // World name (bold)
                            let name_color = if connected { theme.success() } else { theme.fg() };
                            ui.label(egui::RichText::new(world_name).monospace().strong().color(name_color));

                            // Tag indicator (only shown when F2 toggled to show tags)
                            if self.show_tags {
                                ui.label(egui::RichText::new(" [tag]").monospace().color(theme.prompt()));
                            }

                            // Activity indicator with hover tooltip
                            if activity_count > 0 {
                                ui.label(egui::RichText::new(" ").monospace().color(theme.fg_dim()));
                                let activity_label = ui.label(egui::RichText::new(format!("(Activity: {})", activity_count))
                                    .monospace().color(theme.highlight()));
                                // Show hover popup with list of worlds that have activity
                                activity_label.on_hover_ui(|ui| {
                                    ui.label(egui::RichText::new("Worlds with unseen output:").strong());
                                    for world_name in &worlds_with_activity {
                                        ui.label(*world_name);
                                    }
                                });
                            }

                            // Spacer with underscore-style fill
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // Current time (HH:MM)
                                let now = std::time::SystemTime::now();
                                let datetime = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                                let secs = datetime.as_secs();
                                let hours = (secs / 3600) % 24;
                                let mins = (secs / 60) % 60;
                                // Adjust for local timezone (rough estimate)
                                let local_hours = (hours + 24 - 5) % 24; // UTC-5 estimate
                                ui.label(egui::RichText::new(format!("{:02}:{:02}", local_hours, mins))
                                    .monospace().color(theme.accent()));
                            });
                        });
                    });

                // Filter popup (F4)
                if self.filter_active {
                    egui::Window::new("Filter")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::RIGHT_TOP, [-10.0, 40.0])
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Filter:");
                                let response = ui.text_edit_singleline(&mut self.filter_text);
                                response.request_focus();
                            });
                            ui.label("(Esc or F4 to close)");
                        });
                }

                // Main output area with scrollbar (no frame/border/margin)
                egui::CentralPanel::default()
                    .frame(egui::Frame::none()
                        .fill(theme.bg())
                        .inner_margin(egui::Margin::same(0.0))
                        .stroke(egui::Stroke::NONE))
                    .show(ctx, |ui| {
                    if let Some(world) = self.worlds.get(self.current_world) {
                        // Keep original lines with ANSI for coloring
                        let colored_lines: Vec<&String> = world.output_lines.iter()
                            .filter(|line| {
                                // Apply filter if active (filter on stripped text)
                                if self.filter_active && !self.filter_text.is_empty() {
                                    let stripped = Self::strip_ansi_for_copy(line);
                                    stripped.to_lowercase().contains(&self.filter_text.to_lowercase())
                                } else {
                                    true
                                }
                            })
                            .collect();

                        // Build plain text version for selection (strip ANSI codes and empty lines)
                        let lines: Vec<String> = colored_lines.iter()
                            .map(|line| {
                                let stripped = Self::strip_ansi_for_copy(line);
                                if self.show_tags {
                                    stripped
                                } else {
                                    // Strip MUD tags like [channel:] or [channel(player)]
                                    Self::strip_mud_tags(&stripped)
                                }
                            })
                            .filter(|line| !line.is_empty())  // Filter empty lines
                            .collect();
                        let plain_text: String = lines.join("\n");

                        // Build combined LayoutJob with ANSI colors
                        let default_color = theme.fg();
                        let font_id = egui::FontId::monospace(self.font_size);
                        let mut combined_job = egui::text::LayoutJob::default();
                        combined_job.wrap = egui::text::TextWrapping {
                            max_width: ui.available_width(),
                            ..Default::default()
                        };

                        // Filter out empty lines to match console behavior
                        let non_empty_lines: Vec<_> = colored_lines.iter()
                            .filter(|line| !Self::strip_ansi_for_copy(line).is_empty())
                            .collect();

                        for (i, line) in non_empty_lines.iter().enumerate() {
                            // Apply tag stripping if needed (on the ANSI version)
                            let display_line = if self.show_tags {
                                (**line).clone()
                            } else {
                                Self::strip_mud_tags_ansi(line)
                            };

                            // Parse ANSI and append to combined job
                            let is_light_theme = matches!(theme, GuiTheme::Light);
                            Self::append_ansi_to_job(&display_line, default_color, font_id.clone(), &mut combined_job, is_light_theme);

                            // Add newline between lines (except after last)
                            if i < non_empty_lines.len() - 1 {
                                combined_job.append("\n", 0.0, egui::TextFormat {
                                    font_id: font_id.clone(),
                                    color: default_color,
                                    ..Default::default()
                                });
                            }
                        }

                        // Use a unique ID per world to ensure scroll state is preserved per-world
                        let scroll_id = egui::Id::new(format!("output_scroll_{}", self.current_world));
                        let stick_to_bottom = self.scroll_offset.is_none() && !self.filter_active;

                        // Apply scroll offset if set (from PageUp/PageDown)
                        let scroll_delta = if let Some(offset) = self.scroll_offset.take() {
                            // Convert our offset to a delta we want to apply
                            Some(offset)
                        } else {
                            None
                        };

                        let mut scroll_area = ScrollArea::vertical()
                            .id_salt(scroll_id)
                            .auto_shrink([false; 2])
                            .stick_to_bottom(stick_to_bottom)
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);

                        // If we have a scroll delta, apply it
                        if let Some(delta) = scroll_delta {
                            scroll_area = scroll_area.vertical_scroll_offset(delta);
                        }

                        // Clone the job for the layouter closure
                        let layout_job = combined_job.clone();

                        let scroll_output = scroll_area.show(ui, |ui| {
                                ui.set_width(ui.available_width());

                                // Use TextEdit with custom layouter for colored text
                                let mut text_copy = plain_text.clone();
                                let response = TextEdit::multiline(&mut text_copy)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY)
                                    .interactive(true)
                                    .frame(false)
                                    .layouter(&mut |_ui, _string, wrap_width| {
                                        let mut job = layout_job.clone();
                                        job.wrap.max_width = wrap_width;
                                        _ui.fonts(|f| f.layout_job(job))
                                    })
                                    .show(ui);

                                // Store selection in egui memory on every frame when there is one
                                // This ensures we have it captured before any click clears it
                                let selection_id = egui::Id::new("output_selection");
                                let selection_range_id = egui::Id::new("output_selection_range");
                                if let Some(cursor_range) = response.cursor_range {
                                    if cursor_range.primary != cursor_range.secondary {
                                        let start = cursor_range.primary.ccursor.index.min(cursor_range.secondary.ccursor.index);
                                        let end = cursor_range.primary.ccursor.index.max(cursor_range.secondary.ccursor.index);
                                        let selected = response.galley.text()[start..end].to_string();
                                        // Always store selection text and range when we have one
                                        ui.ctx().data_mut(|d| {
                                            d.insert_temp(selection_id, selected);
                                            d.insert_temp(selection_range_id, (start, end));
                                        });
                                    }
                                }
                                // Handle clicks - check for URL clicks and clear selection
                                if response.response.clicked() {
                                    if let Some(cursor_range) = response.cursor_range {
                                        if cursor_range.primary == cursor_range.secondary {
                                            let click_pos = cursor_range.primary.ccursor.index;

                                            // Check if clicking on a URL
                                            let urls = Self::find_urls(&plain_text);
                                            let mut url_clicked = false;
                                            for (start, end, url) in urls {
                                                if click_pos >= start && click_pos <= end {
                                                    Self::open_url(&url);
                                                    url_clicked = true;
                                                    break;
                                                }
                                            }

                                            // Clear selection if not clicking a URL
                                            if !url_clicked {
                                                ui.ctx().data_mut(|d| {
                                                    d.remove::<String>(selection_id);
                                                    d.remove::<(usize, usize)>(selection_range_id);
                                                });
                                            }
                                        }
                                    } else {
                                        ui.ctx().data_mut(|d| {
                                            d.remove::<String>(selection_id);
                                            d.remove::<(usize, usize)>(selection_range_id);
                                        });
                                    }
                                }

                                // Always draw custom selection highlight when we have a stored selection
                                // This ensures no flicker when context menu opens/closes
                                {
                                    if let Some((start, end)) = ui.ctx().data(|d| d.get_temp::<(usize, usize)>(selection_range_id)) {
                                        let galley = &response.galley;
                                        let text_pos = response.galley_pos;

                                        // Get cursor positions for start and end
                                        let start_cursor = galley.from_ccursor(egui::text::CCursor::new(start));
                                        let end_cursor = galley.from_ccursor(egui::text::CCursor::new(end));

                                        // Draw selection rectangles for each row in the selection
                                        let selection_color = egui::Color32::from_rgba_unmultiplied(100, 100, 200, 100);
                                        let painter = ui.painter();

                                        for row_idx in start_cursor.rcursor.row..=end_cursor.rcursor.row {
                                            if let Some(row) = galley.rows.get(row_idx) {
                                                let row_start = if row_idx == start_cursor.rcursor.row {
                                                    galley.pos_from_cursor(&start_cursor).min.x
                                                } else {
                                                    row.rect.left()
                                                };
                                                let row_end = if row_idx == end_cursor.rcursor.row {
                                                    galley.pos_from_cursor(&end_cursor).min.x
                                                } else {
                                                    row.rect.right()
                                                };

                                                let rect = egui::Rect::from_min_max(
                                                    egui::pos2(text_pos.x + row_start, text_pos.y + row.rect.top()),
                                                    egui::pos2(text_pos.x + row_end, text_pos.y + row.rect.bottom()),
                                                );
                                                painter.rect_filled(rect, 0.0, selection_color);
                                            }
                                        }
                                    }
                                }

                                // Draw URL underlines and handle hover cursor
                                {
                                    let urls = Self::find_urls(&plain_text);
                                    if !urls.is_empty() {
                                        let galley = &response.galley;
                                        let text_pos = response.galley_pos;
                                        let painter = ui.painter();
                                        let link_color = theme.link();
                                        let hover_pos = ui.input(|i| i.pointer.hover_pos());

                                        for (start, end, _url) in urls {
                                            let start_cursor = galley.from_ccursor(egui::text::CCursor::new(start));
                                            let end_cursor = galley.from_ccursor(egui::text::CCursor::new(end));

                                            // Draw underline for each row the URL spans
                                            for row_idx in start_cursor.rcursor.row..=end_cursor.rcursor.row {
                                                if let Some(row) = galley.rows.get(row_idx) {
                                                    let row_start = if row_idx == start_cursor.rcursor.row {
                                                        galley.pos_from_cursor(&start_cursor).min.x
                                                    } else {
                                                        row.rect.left()
                                                    };
                                                    let row_end = if row_idx == end_cursor.rcursor.row {
                                                        galley.pos_from_cursor(&end_cursor).min.x
                                                    } else {
                                                        row.rect.right()
                                                    };

                                                    // Create rect for this URL segment
                                                    let url_rect = egui::Rect::from_min_max(
                                                        egui::pos2(text_pos.x + row_start, text_pos.y + row.rect.top()),
                                                        egui::pos2(text_pos.x + row_end, text_pos.y + row.rect.bottom()),
                                                    );

                                                    // Check if mouse is hovering over this URL segment
                                                    if let Some(pos) = hover_pos {
                                                        if url_rect.contains(pos) {
                                                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                                        }
                                                    }

                                                    // Draw underline at bottom of text
                                                    let y = text_pos.y + row.rect.bottom() - 1.0;
                                                    painter.line_segment(
                                                        [egui::pos2(text_pos.x + row_start, y), egui::pos2(text_pos.x + row_end, y)],
                                                        egui::Stroke::new(1.0, link_color),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }

                                // Right-click context menu
                                let plain_text_for_menu = plain_text.clone();
                                response.response.context_menu(|ui| {
                                    // Get stored selection from egui memory
                                    let stored_selection: Option<String> = ui.ctx().data(|d| d.get_temp(selection_id));

                                    // Show Copy button if there's stored selected text
                                    if let Some(selected) = stored_selection {
                                        if ui.button("Copy").clicked() {
                                            ui.ctx().copy_text(selected);
                                            ui.close_menu();
                                        }
                                    }
                                    if ui.button("Copy All").clicked() {
                                        ui.ctx().copy_text(plain_text_for_menu.clone());
                                        ui.close_menu();
                                    }
                                });
                            });

                        // Track actual scroll position from scroll area state
                        let content_size = scroll_output.content_size.y;
                        let viewport_height = scroll_output.inner_rect.height();
                        let max_offset = (content_size - viewport_height).max(0.0);
                        let current_offset = scroll_output.state.offset.y;

                        // Save max offset for PageUp/PageDown calculations
                        self.scroll_max_offset = max_offset;

                        // Update our tracked offset based on actual scroll position
                        if current_offset >= max_offset - 1.0 {
                            // At or near bottom
                            self.scroll_offset = None;
                        } else if self.scroll_offset.is_some() {
                            // Clamp our tracked offset to valid range
                            self.scroll_offset = Some(current_offset.clamp(0.0, max_offset));
                        }
                    }
                });

                // Popup windows
                let mut close_popup = false;
                let mut popup_action: Option<(&str, usize)> = None;

                // World List popup
                if self.popup_state == PopupState::WorldList {
                    egui::Window::new("World List")
                        .collapsible(false)
                        .resizable(true)
                        .default_size([400.0, 300.0])
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }
                            ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                for (idx, world) in self.worlds.iter().enumerate() {
                                    let status = if world.connected { "●" } else { "○" };
                                    let label = format!("{} {} - {}:{}",
                                        status, world.name,
                                        world.settings.hostname, world.settings.port);
                                    if ui.selectable_label(idx == self.world_list_selected, &label).clicked() {
                                        self.world_list_selected = idx;
                                    }
                                }
                            });
                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button("Connect").clicked() {
                                    popup_action = Some(("connect", self.world_list_selected));
                                    close_popup = true;
                                }
                                if ui.button("Edit").clicked() {
                                    popup_action = Some(("edit", self.world_list_selected));
                                }
                                if ui.button("Switch To").clicked() {
                                    popup_action = Some(("switch", self.world_list_selected));
                                    close_popup = true;
                                }
                                if ui.button("Close").clicked() {
                                    close_popup = true;
                                }
                            });
                        });
                }

                // Connected Worlds popup (/worlds or /l)
                if self.popup_state == PopupState::ConnectedWorlds {
                    // Helper to format elapsed seconds
                    fn format_elapsed_secs(secs: Option<u64>) -> String {
                        match secs {
                            None => "-".to_string(),
                            Some(s) => {
                                if s < 60 {
                                    format!("{}s", s)
                                } else if s < 3600 {
                                    format!("{}m", s / 60)
                                } else if s < 86400 {
                                    format!("{}h", s / 3600)
                                } else {
                                    format!("{}d", s / 86400)
                                }
                            }
                        }
                    }

                    // Helper to calculate time until next NOP (based on 5 min keepalive)
                    fn format_next_nop(last_send: Option<u64>, last_recv: Option<u64>) -> String {
                        const KEEPALIVE_SECS: u64 = 5 * 60;
                        let elapsed = match (last_send, last_recv) {
                            (Some(s), Some(r)) => s.min(r),  // Use more recent (smaller elapsed)
                            (Some(s), None) => s,
                            (None, Some(r)) => r,
                            (None, None) => KEEPALIVE_SECS,
                        };
                        let remaining = KEEPALIVE_SECS.saturating_sub(elapsed);
                        if remaining < 60 {
                            format!("{}s", remaining)
                        } else {
                            format!("{}m", remaining / 60)
                        }
                    }

                    egui::Window::new("Connected Worlds")
                        .collapsible(false)
                        .resizable(true)
                        .default_size([620.0, 250.0])
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }
                            // Table header - matches console columns exactly
                            egui::Grid::new("worlds_header")
                                .num_columns(7)
                                .min_col_width(55.0)
                                .spacing([10.0, 4.0])
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("World").strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(egui::RichText::new("Unseen").strong());
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(egui::RichText::new("LastSend").strong());
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(egui::RichText::new("LastRecv").strong());
                                    });
                                    ui.label(egui::RichText::new("KeepAlive").strong());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(egui::RichText::new("LastKA").strong());
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(egui::RichText::new("NextKA").strong());
                                    });
                                    ui.end_row();
                                });
                            ui.separator();

                            // Check if any worlds are connected
                            let has_connected = self.worlds.iter().any(|w| w.connected);
                            if !has_connected {
                                ui.label("No worlds connected.");
                            } else {
                                ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                                    egui::Grid::new("worlds_grid")
                                        .num_columns(7)
                                        .min_col_width(55.0)
                                        .spacing([10.0, 4.0])
                                        .show(ui, |ui| {
                                            for (idx, world) in self.worlds.iter().enumerate() {
                                                // Only show connected worlds
                                                if !world.connected {
                                                    continue;
                                                }
                                                let is_current = idx == self.current_world;
                                                let name_text = if is_current {
                                                    format!("* {}", world.name)
                                                } else {
                                                    format!("  {}", world.name)
                                                };
                                                let name_label = if is_current {
                                                    egui::RichText::new(&name_text).strong().color(egui::Color32::WHITE)
                                                } else {
                                                    egui::RichText::new(&name_text)
                                                };
                                                if ui.selectable_label(idx == self.world_list_selected, name_label).clicked() {
                                                    self.world_list_selected = idx;
                                                }
                                                // Unseen column (right-aligned)
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if world.unseen_lines > 0 {
                                                        ui.label(egui::RichText::new(format!("{}", world.unseen_lines))
                                                            .color(egui::Color32::YELLOW));
                                                    } else {
                                                        ui.label("");
                                                    }
                                                });
                                                // Send column (right-aligned)
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.label(format_elapsed_secs(world.last_send_secs));
                                                });
                                                // Recv column (right-aligned)
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.label(format_elapsed_secs(world.last_recv_secs));
                                                });
                                                // KeepAlive type
                                                ui.label(&world.settings.keep_alive_type);
                                                // Last column (right-aligned)
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.label(format_elapsed_secs(world.last_nop_secs));
                                                });
                                                // Next column (right-aligned)
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.label(format_next_nop(world.last_send_secs, world.last_recv_secs));
                                                });
                                                ui.end_row();
                                            }
                                        });
                                });
                            }
                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button("Switch To").clicked() {
                                    popup_action = Some(("switch", self.world_list_selected));
                                    close_popup = true;
                                }
                                if ui.button("Close").clicked() {
                                    close_popup = true;
                                }
                            });
                        });
                }

                // World Editor popup
                if let PopupState::WorldEditor(world_idx) = self.popup_state {
                    egui::Window::new("World Editor")
                        .collapsible(false)
                        .resizable(false)
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }
                            egui::Grid::new("world_editor_grid")
                                .num_columns(2)
                                .spacing([10.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label("Name:");
                                    ui.add(TextEdit::singleline(&mut self.edit_name).desired_width(200.0));
                                    ui.end_row();

                                    ui.label("Hostname:");
                                    ui.add(TextEdit::singleline(&mut self.edit_hostname).desired_width(200.0));
                                    ui.end_row();

                                    ui.label("Port:");
                                    ui.add(TextEdit::singleline(&mut self.edit_port).desired_width(200.0));
                                    ui.end_row();

                                    ui.label("User:");
                                    ui.add(TextEdit::singleline(&mut self.edit_user).desired_width(200.0));
                                    ui.end_row();

                                    ui.label("Use SSL:");
                                    ui.checkbox(&mut self.edit_ssl, "");
                                    ui.end_row();

                                    ui.label("Keep-Alive:");
                                    if ui.button(self.edit_keep_alive_type.name()).clicked() {
                                        self.edit_keep_alive_type = self.edit_keep_alive_type.next();
                                    }
                                    ui.end_row();

                                    // Only show Keep-Alive CMD when Custom is selected
                                    if self.edit_keep_alive_type == KeepAliveType::Custom {
                                        ui.label("Keep-Alive CMD:");
                                        ui.add(TextEdit::singleline(&mut self.edit_keep_alive_cmd)
                                            .hint_text("use ##rand## for idler tag")
                                            .desired_width(200.0));
                                        ui.end_row();
                                    }
                                });
                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button("Save").clicked() {
                                    // Update local world settings and send to server
                                    if let Some(world) = self.worlds.get_mut(world_idx) {
                                        world.name = self.edit_name.clone();
                                        world.settings.hostname = self.edit_hostname.clone();
                                        world.settings.port = self.edit_port.clone();
                                        world.settings.user = self.edit_user.clone();
                                        world.settings.use_ssl = self.edit_ssl;
                                        world.settings.keep_alive_type = self.edit_keep_alive_type.name().to_string();
                                        world.settings.keep_alive_cmd = self.edit_keep_alive_cmd.clone();
                                    }
                                    // Send update to server
                                    self.update_world_settings(world_idx);
                                    close_popup = true;
                                }
                                if ui.button("Cancel").clicked() {
                                    close_popup = true;
                                }
                                if ui.button("Connect").clicked() {
                                    popup_action = Some(("connect", world_idx));
                                    close_popup = true;
                                }
                            });
                        });
                }

                // Setup popup (matches console /setup)
                if self.popup_state == PopupState::Setup {
                    egui::Window::new("Setup")
                        .collapsible(false)
                        .resizable(false)
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }
                            ui.label("Global settings");
                            ui.separator();
                            egui::Grid::new("setup_grid")
                                .num_columns(2)
                                .spacing([10.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label("More mode:");
                                    let more_text = if self.more_mode { "on" } else { "off" };
                                    if ui.button(more_text).clicked() {
                                        self.more_mode = !self.more_mode;
                                    }
                                    ui.end_row();

                                    ui.label("Spell check:");
                                    let spell_text = if self.spell_check_enabled { "on" } else { "off" };
                                    if ui.button(spell_text).clicked() {
                                        self.spell_check_enabled = !self.spell_check_enabled;
                                    }
                                    ui.end_row();

                                    ui.label("World Switching:");
                                    if ui.button(self.world_switch_mode.name()).clicked() {
                                        self.world_switch_mode = self.world_switch_mode.next();
                                    }
                                    ui.end_row();

                                    ui.label("Debug:");
                                    let debug_text = if self.debug_enabled { "on" } else { "off" };
                                    if ui.button(debug_text).clicked() {
                                        self.debug_enabled = !self.debug_enabled;
                                    }
                                    ui.end_row();

                                    ui.label("Show tags:");
                                    let tags_text = if self.show_tags { "on" } else { "off" };
                                    if ui.button(tags_text).clicked() {
                                        self.show_tags = !self.show_tags;
                                    }
                                    ui.end_row();

                                    ui.label("Input height:");
                                    ui.horizontal(|ui| {
                                        if ui.button("-").clicked() && self.input_height > 1 {
                                            self.input_height -= 1;
                                        }
                                        ui.label(format!("{}", self.input_height));
                                        if ui.button("+").clicked() && self.input_height < 15 {
                                            self.input_height += 1;
                                        }
                                    });
                                    ui.end_row();

                                    ui.label("Console Theme:");
                                    let console_theme_text = if self.console_theme == GuiTheme::Dark { "Dark" } else { "Light" };
                                    if ui.button(console_theme_text).clicked() {
                                        self.console_theme = if self.console_theme == GuiTheme::Dark { GuiTheme::Light } else { GuiTheme::Dark };
                                    }
                                    ui.end_row();

                                    ui.label("GUI Theme:");
                                    let gui_theme_text = if self.theme == GuiTheme::Dark { "Dark" } else { "Light" };
                                    if ui.button(gui_theme_text).clicked() {
                                        self.theme = if self.theme == GuiTheme::Dark { GuiTheme::Light } else { GuiTheme::Dark };
                                    }
                                    ui.end_row();
                                });
                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button("Save").clicked() {
                                    // Send updated settings to server
                                    self.update_global_settings();
                                    close_popup = true;
                                }
                                if ui.button("Cancel").clicked() {
                                    close_popup = true;
                                }
                            });
                        });
                }

                // Web popup (matches console /web)
                if self.popup_state == PopupState::Web {
                    egui::Window::new("Web Settings")
                        .collapsible(false)
                        .resizable(false)
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }
                            ui.label("Web server settings");
                            ui.separator();
                            egui::Grid::new("web_grid")
                                .num_columns(2)
                                .spacing([10.0, 8.0])
                                .show(ui, |ui| {
                                    // Protocol selection
                                    ui.label("Protocol:");
                                    let proto_text = if self.web_secure { "Secure" } else { "Non-Secure" };
                                    if ui.button(proto_text).clicked() {
                                        self.web_secure = !self.web_secure;
                                    }
                                    ui.end_row();

                                    // HTTP/HTTPS enabled (name changes based on protocol)
                                    let http_label = if self.web_secure { "HTTPS enabled:" } else { "HTTP enabled:" };
                                    ui.label(http_label);
                                    let http_text = if self.http_enabled { "on" } else { "off" };
                                    if ui.button(http_text).clicked() {
                                        self.http_enabled = !self.http_enabled;
                                    }
                                    ui.end_row();

                                    // HTTP/HTTPS port (name changes based on protocol)
                                    let http_port_label = if self.web_secure { "HTTPS port:" } else { "HTTP port:" };
                                    ui.label(http_port_label);
                                    let mut http_port_str = self.http_port.to_string();
                                    if ui.add(egui::TextEdit::singleline(&mut http_port_str).desired_width(80.0)).changed() {
                                        if let Ok(port) = http_port_str.parse::<u16>() {
                                            self.http_port = port;
                                        }
                                    }
                                    ui.end_row();

                                    // WS/WSS enabled (name changes based on protocol)
                                    let ws_label = if self.web_secure { "WSS enabled:" } else { "WS enabled:" };
                                    ui.label(ws_label);
                                    let ws_text = if self.ws_enabled { "on" } else { "off" };
                                    if ui.button(ws_text).clicked() {
                                        self.ws_enabled = !self.ws_enabled;
                                    }
                                    ui.end_row();

                                    // WS/WSS port (name changes based on protocol)
                                    let ws_port_label = if self.web_secure { "WSS port:" } else { "WS port:" };
                                    ui.label(ws_port_label);
                                    let mut ws_port_str = self.ws_port.to_string();
                                    if ui.add(egui::TextEdit::singleline(&mut ws_port_str).desired_width(80.0)).changed() {
                                        if let Ok(port) = ws_port_str.parse::<u16>() {
                                            self.ws_port = port;
                                        }
                                    }
                                    ui.end_row();

                                    // Allow list
                                    ui.label("Allow List:");
                                    ui.add(egui::TextEdit::singleline(&mut self.ws_allow_list)
                                        .hint_text("localhost, 192.168.*")
                                        .desired_width(200.0));
                                    ui.end_row();
                                });
                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button("Save").clicked() {
                                    // Send updated settings to server
                                    self.update_global_settings();
                                    close_popup = true;
                                }
                                if ui.button("Cancel").clicked() {
                                    close_popup = true;
                                }
                            });
                        });
                }

                // Font popup
                if self.popup_state == PopupState::Font {
                    // Common monospace font families
                    const FONT_FAMILIES: &[(&str, &str)] = &[
                        ("", "System Default"),
                        ("Monospace", "Monospace"),
                        ("DejaVu Sans Mono", "DejaVu Sans Mono"),
                        ("Liberation Mono", "Liberation Mono"),
                        ("Ubuntu Mono", "Ubuntu Mono"),
                        ("Fira Code", "Fira Code"),
                        ("Source Code Pro", "Source Code Pro"),
                        ("JetBrains Mono", "JetBrains Mono"),
                        ("Hack", "Hack"),
                        ("Inconsolata", "Inconsolata"),
                        ("Courier New", "Courier New"),
                        ("Consolas", "Consolas"),
                    ];

                    egui::Window::new("Font Settings")
                        .collapsible(false)
                        .resizable(false)
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }
                            egui::Grid::new("font_grid")
                                .num_columns(2)
                                .spacing([10.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label("Font family:");
                                    let current_label = FONT_FAMILIES.iter()
                                        .find(|(value, _)| *value == self.edit_font_name)
                                        .map(|(_, label)| *label)
                                        .unwrap_or_else(|| {
                                            if self.edit_font_name.is_empty() { "System Default" } else { &self.edit_font_name }
                                        });
                                    egui::ComboBox::from_id_salt("font_family")
                                        .selected_text(current_label)
                                        .width(180.0)
                                        .show_ui(ui, |ui| {
                                            for (value, label) in FONT_FAMILIES {
                                                if ui.selectable_value(&mut self.edit_font_name, value.to_string(), *label).clicked() {
                                                    // Selection handled by selectable_value
                                                }
                                            }
                                        });
                                    ui.end_row();

                                    ui.label("Font size:");
                                    ui.horizontal(|ui| {
                                        if ui.button("-").clicked() {
                                            if let Ok(size) = self.edit_font_size.parse::<f32>() {
                                                let new_size = (size - 1.0).max(8.0);
                                                self.edit_font_size = format!("{:.1}", new_size);
                                            }
                                        }
                                        ui.add(egui::TextEdit::singleline(&mut self.edit_font_size)
                                            .desired_width(50.0));
                                        if ui.button("+").clicked() {
                                            if let Ok(size) = self.edit_font_size.parse::<f32>() {
                                                let new_size = (size + 1.0).min(48.0);
                                                self.edit_font_size = format!("{:.1}", new_size);
                                            }
                                        }
                                    });
                                    ui.end_row();
                                });
                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button("OK").clicked() {
                                    // Parse and apply font settings
                                    self.font_name = self.edit_font_name.clone();
                                    if let Ok(size) = self.edit_font_size.parse::<f32>() {
                                        self.font_size = size.clamp(8.0, 48.0);
                                    }
                                    // Send updated settings to server
                                    self.update_global_settings();
                                    close_popup = true;
                                }
                                if ui.button("Cancel").clicked() {
                                    close_popup = true;
                                }
                            });
                        });
                }

                // Help popup
                if self.popup_state == PopupState::Help {
                    egui::Window::new("Help")
                        .collapsible(false)
                        .resizable(true)
                        .default_size([450.0, 400.0])
                        .show(ctx, |ui| {
                            // Check for Escape key to close
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }

                            egui::ScrollArea::vertical()
                                .max_height(350.0)
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new("Clay - A MUD Client").strong().size(16.0));
                                    ui.add_space(8.0);

                                    ui.label(egui::RichText::new("World Switching").strong());
                                    ui.label("  Up/Down      - Cycle through active worlds");
                                    ui.label("  Shift+Up/Down - Cycle through all worlds");
                                    ui.add_space(4.0);

                                    ui.label(egui::RichText::new("Output Navigation").strong());
                                    ui.label("  PageUp/Down  - Scroll through output history");
                                    ui.label("  Tab          - Release one screenful (when paused)");
                                    ui.label("  Alt+J        - Jump to end, release all pending");
                                    ui.add_space(4.0);

                                    ui.label(egui::RichText::new("Input").strong());
                                    ui.label("  Enter        - Send command");
                                    ui.label("  Ctrl+P/N     - Previous/Next command history");
                                    ui.label("  Ctrl+U       - Clear input line");
                                    ui.label("  Ctrl+W       - Delete word before cursor");
                                    ui.label("  Ctrl+Q       - Spell check suggestions");
                                    ui.add_space(4.0);

                                    ui.label(egui::RichText::new("Display").strong());
                                    ui.label("  F2           - Toggle MUD tag display");
                                    ui.label("  F4           - Open filter popup");
                                    ui.add_space(4.0);

                                    ui.label(egui::RichText::new("Options Menu").strong());
                                    ui.label("  World List   - View and select worlds");
                                    ui.label("  World Editor - Edit world connection settings");
                                    ui.label("  Setup        - Global settings");
                                    ui.label("  Font         - Change font family and size");
                                    ui.label("  Connect      - Connect to current world");
                                    ui.label("  Disconnect   - Disconnect from current world");
                                });

                            ui.separator();
                            ui.horizontal(|ui| {
                                if ui.button("OK").clicked() {
                                    close_popup = true;
                                }
                            });
                        });
                }

                // Actions List popup (first window)
                if self.popup_state == PopupState::ActionsList {
                    egui::Window::new("Actions")
                        .collapsible(false)
                        .resizable(true)
                        .default_size([450.0, 300.0])
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                close_popup = true;
                            }

                            // Actions list with columns: Name, World, Pattern
                            ui.label(egui::RichText::new("Saved Actions").strong());
                            ui.separator();

                            if self.actions.is_empty() {
                                ui.label("No actions defined.");
                            } else {
                                egui::ScrollArea::vertical()
                                    .max_height(180.0)
                                    .show(ui, |ui| {
                                        egui::Grid::new("actions_list_grid")
                                            .num_columns(3)
                                            .min_col_width(80.0)
                                            .spacing([10.0, 4.0])
                                            .striped(true)
                                            .show(ui, |ui| {
                                                // Header
                                                ui.label(egui::RichText::new("Name").strong());
                                                ui.label(egui::RichText::new("World").strong());
                                                ui.label(egui::RichText::new("Pattern").strong());
                                                ui.end_row();

                                                // Actions
                                                for (idx, action) in self.actions.iter().enumerate() {
                                                    let is_selected = idx == self.actions_selected;
                                                    let name_text = if is_selected {
                                                        egui::RichText::new(&action.name).strong().color(egui::Color32::WHITE)
                                                    } else {
                                                        egui::RichText::new(&action.name)
                                                    };
                                                    if ui.selectable_label(is_selected, name_text).clicked() {
                                                        self.actions_selected = idx;
                                                    }
                                                    let world_display = if action.world.is_empty() { "(all)" } else { &action.world };
                                                    ui.label(world_display);
                                                    let pattern_display = if action.pattern.is_empty() { "(manual)" } else { &action.pattern };
                                                    ui.label(pattern_display);
                                                    ui.end_row();
                                                }
                                            });
                                    });
                            }

                            ui.separator();

                            // Buttons: Add, Edit, Delete, Cancel
                            ui.horizontal(|ui| {
                                if ui.button("Add").clicked() {
                                    // Create new action and open editor
                                    self.edit_action_name = String::new();
                                    self.edit_action_world = String::new();
                                    self.edit_action_pattern = String::new();
                                    self.edit_action_command = String::new();
                                    self.action_error = None;
                                    self.popup_state = PopupState::ActionEditor(usize::MAX); // MAX = new action
                                }
                                if ui.button("Edit").clicked() && !self.actions.is_empty() {
                                    // Load selected action into editor
                                    if let Some(action) = self.actions.get(self.actions_selected) {
                                        self.edit_action_name = action.name.clone();
                                        self.edit_action_world = action.world.clone();
                                        self.edit_action_pattern = action.pattern.clone();
                                        self.edit_action_command = action.command.clone();
                                        self.action_error = None;
                                        self.popup_state = PopupState::ActionEditor(self.actions_selected);
                                    }
                                }
                                if ui.button("Delete").clicked() && !self.actions.is_empty() {
                                    self.popup_state = PopupState::ActionConfirmDelete;
                                }
                                if ui.button("Cancel").clicked() {
                                    close_popup = true;
                                }
                            });
                        });
                }

                // Actions Editor popup (second window)
                if let PopupState::ActionEditor(edit_idx) = self.popup_state.clone() {
                    let title = if edit_idx == usize::MAX { "New Action" } else { "Edit Action" };
                    egui::Window::new(title)
                        .collapsible(false)
                        .resizable(true)
                        .default_size([400.0, 350.0])
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                // Return to actions list
                                self.popup_state = PopupState::ActionsList;
                            }

                            egui::Grid::new("action_editor_grid")
                                .num_columns(2)
                                .spacing([10.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label("Name:");
                                    ui.add(egui::TextEdit::singleline(&mut self.edit_action_name)
                                        .desired_width(250.0));
                                    ui.end_row();

                                    ui.label("World:");
                                    ui.add(egui::TextEdit::singleline(&mut self.edit_action_world)
                                        .hint_text("(empty = all worlds)")
                                        .desired_width(250.0));
                                    ui.end_row();

                                    ui.label("Pattern:");
                                    ui.add(egui::TextEdit::singleline(&mut self.edit_action_pattern)
                                        .hint_text("(regex, empty = manual only)")
                                        .desired_width(250.0));
                                    ui.end_row();

                                    ui.label("Command:");
                                    ui.end_row();
                                });

                            // Larger command area
                            ui.add(egui::TextEdit::multiline(&mut self.edit_action_command)
                                .hint_text("Commands (semicolon-separated)")
                                .desired_width(f32::INFINITY)
                                .desired_rows(5));

                            // Error message
                            if let Some(ref err) = self.action_error {
                                ui.colored_label(egui::Color32::RED, err);
                            }

                            ui.separator();

                            // Buttons: Save, Cancel
                            ui.horizontal(|ui| {
                                if ui.button("Save").clicked() {
                                    // Validate
                                    let name = self.edit_action_name.trim();
                                    if name.is_empty() {
                                        self.action_error = Some("Name is required".to_string());
                                    } else {
                                        // Check for duplicates (excluding current if editing)
                                        let mut duplicate = false;
                                        for (i, a) in self.actions.iter().enumerate() {
                                            if (edit_idx == usize::MAX || i != edit_idx) &&
                                               a.name.eq_ignore_ascii_case(name) {
                                                self.action_error = Some(format!("Action '{}' already exists", name));
                                                duplicate = true;
                                                break;
                                            }
                                        }
                                        if !duplicate {
                                            let new_action = Action {
                                                name: name.to_string(),
                                                world: self.edit_action_world.trim().to_string(),
                                                pattern: self.edit_action_pattern.clone(),
                                                command: self.edit_action_command.clone(),
                                            };
                                            if edit_idx == usize::MAX {
                                                // New action
                                                self.actions.push(new_action);
                                                self.actions_selected = self.actions.len() - 1;
                                            } else {
                                                // Update existing
                                                self.actions[edit_idx] = new_action;
                                            }
                                            // Send updated actions to server
                                            self.update_actions();
                                            self.popup_state = PopupState::ActionsList;
                                        }
                                    }
                                }
                                if ui.button("Cancel").clicked() {
                                    self.popup_state = PopupState::ActionsList;
                                }
                            });
                        });
                }

                // Action delete confirmation popup
                if self.popup_state == PopupState::ActionConfirmDelete {
                    egui::Window::new("Confirm Delete")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                        .show(ctx, |ui| {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                self.popup_state = PopupState::ActionsList;
                            }

                            let action_name = self.actions.get(self.actions_selected)
                                .map(|a| a.name.as_str())
                                .unwrap_or("(unknown)");
                            ui.label(format!("Delete action '{}'?", action_name));
                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                if ui.button("Yes").clicked() {
                                    if self.actions_selected < self.actions.len() {
                                        self.actions.remove(self.actions_selected);
                                        if self.actions_selected >= self.actions.len() && !self.actions.is_empty() {
                                            self.actions_selected = self.actions.len() - 1;
                                        }
                                        // Send updated actions to server
                                        self.update_actions();
                                    }
                                    self.popup_state = PopupState::ActionsList;
                                }
                                if ui.button("No").clicked() {
                                    self.popup_state = PopupState::ActionsList;
                                }
                            });
                        });
                }

                // Handle popup actions
                if let Some((action, idx)) = popup_action {
                    match action {
                        "connect" => self.connect_world(idx),
                        "edit" => self.open_world_editor(idx),
                        "switch" => {
                            self.current_world = idx;
                            self.switch_world(idx);
                        }
                        _ => {}
                    }
                }

                if close_popup {
                    self.popup_state = PopupState::None;
                }
            }
        }
    }

    /// Run the remote GUI client
    pub fn run(addr: &str, runtime: tokio::runtime::Handle) -> io::Result<()> {
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([800.0, 600.0])
                .with_title("Clay Mud Client"),
            ..Default::default()
        };

        let addr_string = addr.to_string();

        eframe::run_native(
            "Clay Mud Client",
            options,
            Box::new(move |_cc| {
                Ok(Box::new(RemoteGuiApp::new(addr_string, runtime)) as Box<dyn eframe::App>)
            }),
        ).map_err(|e| io::Error::other(format!("eframe error: {}", e)))
    }
}

#[cfg(feature = "remote-gui")]
fn run_remote_gui(addr: &str) -> io::Result<()> {
    let runtime = tokio::runtime::Handle::current();
    remote_gui::run(addr, runtime)
}

#[tokio::main]
async fn main() -> io::Result<()> {
    // Check for --remote=host:port argument for GUI client mode
    if let Some(remote_arg) = std::env::args().find(|a| a.starts_with("--remote=")) {
        #[cfg(feature = "remote-gui")]
        {
            let addr = remote_arg.strip_prefix("--remote=").unwrap();
            return run_remote_gui(addr);
        }
        #[cfg(not(feature = "remote-gui"))]
        {
            eprintln!("Error: --remote requires the 'remote-gui' feature.");
            eprintln!("Rebuild with: cargo build --features remote-gui");
            eprintln!("Argument provided: {}", remote_arg);
            return Ok(());
        }
    }

    // Set up SIGFPE handler to print debug info before crashing
    unsafe {
        extern "C" fn sigfpe_handler(_: libc::c_int) {
            // Restore terminal before printing
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            eprintln!("\n\n=== SIGFPE (Floating Point Exception) detected! ===");
            eprintln!("This is typically caused by division by zero.");
            eprintln!("Please report this bug with the steps to reproduce.");

            // Try to print a backtrace
            eprintln!("\nBacktrace:");
            let bt = std::backtrace::Backtrace::force_capture();
            eprintln!("{}", bt);

            std::process::exit(136);  // 128 + 8 (SIGFPE)
        }
        libc::signal(libc::SIGFPE, sigfpe_handler as libc::sighandler_t);
    }

    // Set up crash handler for automatic recovery
    setup_crash_handler();

    enable_raw_mode()?;
    let mut stdout = stdout();
    // Use explicit cursor positioning and clearing for Windows 11 compatibility
    execute!(
        stdout,
        EnterAlternateScreen,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_app(&mut terminal).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        eprintln!("Error: {err}");
    }

    Ok(())
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut app = App::new();

    // Check if we're in reload mode (via --reload command line argument)
    let is_reload = std::env::args().any(|a| a == "--reload");
    // Check if we're recovering from a crash (via --crash command line argument)
    let is_crash = std::env::args().any(|a| a == "--crash");

    // Initialize crash count from environment variable
    let crash_count = get_crash_count();
    CRASH_COUNT.store(crash_count, Ordering::SeqCst);

    // Collect any startup messages to display after ensuring we have a world
    let mut startup_messages: Vec<String> = Vec::new();

    // Crash recovery also loads state like reload
    let should_load_state = is_reload || is_crash;

    if should_load_state {
        // Load the reload state
        match load_reload_state(&mut app) {
            Ok(true) => {
                if is_crash {
                    startup_messages.push(format!(
                        "Crash recovery successful (attempt {}/{})",
                        crash_count, MAX_CRASH_RESTARTS
                    ));
                } else {
                    startup_messages.push("Hot reload successful!".to_string());
                }
            }
            Ok(false) => {
                startup_messages.push("Warning: No reload state found, starting fresh.".to_string());
                if let Err(e) = load_settings(&mut app) {
                    startup_messages.push(format!("Warning: Could not load settings: {}", e));
                }
            }
            Err(e) => {
                startup_messages.push(format!("Warning: Failed to load reload state: {}", e));
                if let Err(e) = load_settings(&mut app) {
                    startup_messages.push(format!("Warning: Could not load settings: {}", e));
                }
            }
        }
    } else {
        // Normal startup - load settings from file
        if let Err(e) = load_settings(&mut app) {
            startup_messages.push(format!("Warning: Could not load settings: {}", e));
        }
    }

    // Ensure we have at least one world (creates initial world only if no worlds loaded)
    app.ensure_has_world();

    // Now display any startup messages
    for msg in startup_messages {
        app.add_output(&msg);
    }

    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(100);

    // If in reload or crash recovery mode, reconstruct connections from saved fds
    if should_load_state {
        // First pass: identify TLS worlds that need to be disconnected
        let mut tls_disconnect_worlds: Vec<usize> = Vec::new();
        for (world_idx, world) in app.worlds.iter().enumerate() {
            if world.connected && world.is_tls {
                tls_disconnect_worlds.push(world_idx);
            }
        }

        // Disconnect TLS worlds
        let tls_msg = if is_crash {
            "TLS connection was closed during crash recovery. Use /connect to reconnect."
        } else {
            "TLS connection was closed during reload. Use /connect to reconnect."
        };
        for world_idx in tls_disconnect_worlds {
            app.worlds[world_idx].connected = false;
            app.worlds[world_idx].command_tx = None;
            app.worlds[world_idx].socket_fd = None;
            app.worlds[world_idx].output_lines.push(tls_msg.to_string());
        }

        // Second pass: reconstruct plain TCP connections
        for world_idx in 0..app.worlds.len() {
            let world = &app.worlds[world_idx];
            if world.connected && world.socket_fd.is_some() && !world.is_tls {
                let fd = world.socket_fd.unwrap();

                // Reconstruct TcpStream from the raw fd
                let tcp_stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
                tcp_stream.set_nonblocking(true)?;
                let tcp_stream = TcpStream::from_std(tcp_stream)?;

                let (r, w) = tcp_stream.into_split();
                let mut read_half = StreamReader::Plain(r);
                let mut write_half = StreamWriter::Plain(w);

                let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
                // Skip auto-login for restored connections (only fresh connects should auto-login)
                app.worlds[world_idx].skip_auto_login = true;

                // Re-open log file if configured
                if let Some(ref log_path) = app.worlds[world_idx].settings.log_file.clone() {
                    if let Ok(file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(log_path)
                    {
                        app.worlds[world_idx].log_handle =
                            Some(std::sync::Arc::new(std::sync::Mutex::new(file)));
                    }
                }

                // Clone tx for use in reader (for telnet responses)
                let telnet_tx = cmd_tx;

                // Spawn reader task
                let event_tx_read = event_tx.clone();
                tokio::spawn(async move {
                    let mut buffer = BytesMut::with_capacity(4096);
                    buffer.resize(4096, 0);
                    let mut line_buffer: Vec<u8> = Vec::new();

                    loop {
                        match read_half.read(&mut buffer).await {
                            Ok(0) => {
                                // Send any remaining buffered data
                                if !line_buffer.is_empty() {
                                    let (cleaned, responses, detected, prompt) = process_telnet(&line_buffer);
                                    if !responses.is_empty() {
                                        let _ = telnet_tx.send(WriteCommand::Raw(responses)).await;
                                    }
                                    if detected {
                                        let _ = event_tx_read.send(AppEvent::TelnetDetected(world_idx)).await;
                                    }
                                    // Send prompt FIRST for immediate auto-login response
                                    if let Some(prompt_bytes) = prompt {
                                        let _ = event_tx_read.send(AppEvent::Prompt(world_idx, prompt_bytes)).await;
                                    }
                                    // Send remaining data
                                    if !cleaned.is_empty() {
                                        let _ = event_tx_read.send(AppEvent::ServerData(world_idx, cleaned)).await;
                                    }
                                }
                                let _ = event_tx_read
                                    .send(AppEvent::ServerData(
                                        world_idx,
                                        "Connection closed by server.".as_bytes().to_vec(),
                                    ))
                                    .await;
                                let _ = event_tx_read.send(AppEvent::Disconnected(world_idx)).await;
                                break;
                            }
                            Ok(n) => {
                                // Append new data to line buffer
                                line_buffer.extend_from_slice(&buffer[..n]);

                                // Find safe split point (complete lines with complete ANSI sequences)
                                let split_at = find_safe_split_point(&line_buffer);

                                // Send data immediately - either up to split point, or all if no incomplete sequences
                                let to_send = if split_at > 0 {
                                    line_buffer.drain(..split_at).collect()
                                } else if !line_buffer.is_empty() {
                                    // No safe split point but we have data - send it anyway
                                    std::mem::take(&mut line_buffer)
                                } else {
                                    Vec::new()
                                };

                                if !to_send.is_empty() {
                                    // Process telnet sequences
                                    let (cleaned, responses, detected, prompt) = process_telnet(&to_send);

                                    // Send telnet responses if any
                                    if !responses.is_empty() {
                                        let _ = telnet_tx.send(WriteCommand::Raw(responses)).await;
                                    }

                                    // Notify if telnet detected
                                    if detected {
                                        let _ = event_tx_read
                                            .send(AppEvent::TelnetDetected(world_idx))
                                            .await;
                                    }

                                    // Send prompt FIRST if detected via telnet GA
                                    if let Some(prompt_bytes) = prompt {
                                        let _ = event_tx_read
                                            .send(AppEvent::Prompt(world_idx, prompt_bytes))
                                            .await;
                                    }

                                    // Send cleaned data to main loop
                                    if !cleaned.is_empty()
                                        && event_tx_read
                                            .send(AppEvent::ServerData(world_idx, cleaned))
                                            .await
                                            .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                let msg = format!("Read error: {}", e);
                                let _ = event_tx_read
                                    .send(AppEvent::ServerData(world_idx, msg.into_bytes()))
                                    .await;
                                let _ = event_tx_read.send(AppEvent::Disconnected(world_idx)).await;
                                break;
                            }
                        }
                    }
                });

                // Spawn writer task
                tokio::spawn(async move {
                    while let Some(cmd) = cmd_rx.recv().await {
                        let bytes = match cmd {
                            WriteCommand::Text(text) => format!("{}\r\n", text).into_bytes(),
                            WriteCommand::Raw(raw) => raw,
                        };
                        if write_half.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                });
            }
        }

        // Final cleanup pass: mark any world as disconnected if it claims to be connected
        // but has no command channel (meaning the connection wasn't successfully reconstructed)
        for world in &mut app.worlds {
            if world.connected && world.command_tx.is_none() {
                world.connected = false;
                world.socket_fd = None;
                world.output_lines
                    .push("Connection was not restored during reload. Use /connect to reconnect.".to_string());
            }
            // Clear pending_lines for disconnected worlds - they're only meaningful for active connections
            if !world.connected {
                world.pending_lines.clear();
                world.pending_since = None;
                world.paused = false;
            }
        }

        // Send immediate keepalive for all reconnected worlds since we don't know how long they were idle
        for world in &mut app.worlds {
            if world.connected {
                if let Some(tx) = &world.command_tx {
                    let now = std::time::Instant::now();

                    debug_log(app.settings.debug_enabled, &format!(
                        "KEEPALIVE_RELOAD world='{}' type={:?} cmd='{}'",
                        world.name, world.settings.keep_alive_type, world.settings.keep_alive_cmd
                    ));

                    match world.settings.keep_alive_type {
                        KeepAliveType::None => {
                            // Keepalive disabled - do nothing, don't update times
                            debug_log(app.settings.debug_enabled, &format!(
                                "KEEPALIVE_RELOAD_SKIP world='{}' - keepalive disabled", world.name
                            ));
                        }
                        KeepAliveType::Nop => {
                            let nop = vec![TELNET_IAC, TELNET_NOP];
                            let _ = tx.try_send(WriteCommand::Raw(nop));
                            debug_log_keepalive(app.settings.debug_enabled, &world.name, "NOP", "NOP");
                            world.last_send_time = Some(now);
                            world.last_nop_time = Some(now);
                        }
                        KeepAliveType::Custom => {
                            let rand_num = (std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos() % 1000 + 1) as u32;
                            let idler_tag = format!("###_idler_message_{}_###", rand_num);
                            let cmd = world.settings.keep_alive_cmd
                                .replace("##rand##", &idler_tag);
                            let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                            debug_log_keepalive(app.settings.debug_enabled, &world.name, "Custom", &cmd);
                            world.last_send_time = Some(now);
                            world.last_nop_time = Some(now);
                        }
                        KeepAliveType::Generic => {
                            let rand_num = (std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos() % 1000 + 1) as u32;
                            let cmd = format!("help commands ###_idler_message_{}_###", rand_num);
                            let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                            debug_log_keepalive(app.settings.debug_enabled, &world.name, "Generic", &cmd);
                            world.last_send_time = Some(now);
                            world.last_nop_time = Some(now);
                        }
                    }
                }
            }
        }
    }

    // Start WebSocket server if enabled (ws:// or wss:// based on web_secure setting)
    if app.settings.ws_enabled && !app.settings.websocket_password.is_empty() {
        let mut server = WebSocketServer::new(
            &app.settings.websocket_password,
            app.settings.ws_port,
            &app.settings.websocket_allow_list,
            app.settings.websocket_whitelisted_host.clone(),
        );

        // Configure TLS if secure mode and cert/key files are specified
        #[cfg(feature = "native-tls-backend")]
        let tls_configured = if app.settings.web_secure
            && !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty()
        {
            match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
                Ok(()) => true,
                Err(e) => {
                    app.add_output(&format!("Warning: Failed to configure WSS TLS: {}", e));
                    false
                }
            }
        } else {
            false
        };
        #[cfg(feature = "rustls-backend")]
        let tls_configured = if app.settings.web_secure
            && !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty()
        {
            match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
                Ok(()) => true,
                Err(e) => {
                    app.add_output(&format!("Warning: Failed to configure WSS TLS: {}", e));
                    false
                }
            }
        } else {
            false
        };

        if let Err(e) = start_websocket_server(&mut server, event_tx.clone()).await {
            // Don't show error if port is in use (likely another clay instance)
            let err_str = e.to_string();
            if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                app.add_output(&format!("Warning: Failed to start WebSocket server: {}", e));
            }
        } else {
            let protocol = if tls_configured { "wss" } else { "ws" };
            app.add_output(&format!("WebSocket server started on port {} ({})", app.settings.ws_port, protocol));
            app.ws_server = Some(server);
        }
    }

    // Start HTTP/HTTPS web interface server if enabled
    if app.settings.http_enabled {
        if app.settings.web_secure
            && !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty()
        {
            // Start HTTPS server (secure mode)
            #[cfg(feature = "native-tls-backend")]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
                    app.settings.ws_port,
                    true, // HTTPS uses secure WebSocket (wss://)
                ).await {
                    Ok(()) => {
                        app.add_output(&format!("HTTPS web interface started on port {} (wss://localhost:{})", app.settings.http_port, app.settings.ws_port));
                        app.https_server = Some(https_server);
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                            app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                        }
                    }
                }
            }
            #[cfg(feature = "rustls-backend")]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
                    app.settings.ws_port,
                    true, // HTTPS uses secure WebSocket (wss://)
                ).await {
                    Ok(()) => {
                        app.add_output(&format!("HTTPS web interface started on port {} (wss://localhost:{})", app.settings.http_port, app.settings.ws_port));
                        app.https_server = Some(https_server);
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                            app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                        }
                    }
                }
            }
        } else {
            // Start HTTP server (non-secure mode)
            let mut http_server = HttpServer::new(app.settings.http_port);
            match start_http_server(
                &mut http_server,
                app.settings.ws_port,
                false, // HTTP uses non-secure WebSocket (ws://)
            ).await {
                Ok(()) => {
                    app.add_output(&format!("HTTP web interface started on port {} (ws://localhost:{})", app.settings.http_port, app.settings.ws_port));
                    app.http_server = Some(http_server);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                        app.add_output(&format!("Warning: Failed to start HTTP server: {}", e));
                    }
                }
            }
        }
    }

    // Keepalive: send NOP every 5 minutes if telnet mode and idle
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5 * 60);

    // Use async event stream instead of polling to reduce CPU usage
    let mut event_stream = EventStream::new();

    // Set up SIGUSR1 handler for hot reload
    let mut sigusr1 = signal(SignalKind::user_defined1())
        .expect("Failed to set up SIGUSR1 handler");

    // Initial draw
    terminal.clear()?;
    terminal.draw(|f| ui(f, &mut app))?;
    // Render output with crossterm (needed after reload when ratatui early-returns)
    render_output_crossterm(&app);

    // Create a persistent interval for periodic tasks (clock updates, keepalive checks)
    let mut keepalive_interval = tokio::time::interval(Duration::from_secs(60));
    // Skip the first tick which fires immediately
    keepalive_interval.tick().await;

    // Set the app pointer for crash recovery
    // SAFETY: app lives for the duration of this function and the pointer is only used
    // in the panic hook which only runs while this function is on the stack
    set_app_ptr(&mut app as *mut App);

    // Track if we've cleared the crash count after successful user input
    let mut crash_count_cleared = false;

    loop {
        // Use tokio::select! to efficiently wait for events without busy-polling
        tokio::select! {
            // SIGUSR1 signal - trigger hot reload
            _ = sigusr1.recv() => {
                app.add_output("Received SIGUSR1, performing hot reload...");
                if handle_command("/reload", &mut app, event_tx.clone()).await {
                    return Ok(());
                }
            }
            // Terminal events (keyboard input)
            maybe_event = event_stream.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_event {
                    match handle_key_event(key, &mut app) {
                        KeyAction::Quit => return Ok(()),
                        KeyAction::Redraw => {
                            terminal.clear()?;
                        }
                        KeyAction::Connect => {
                            if !app.current_world().connected
                                && handle_command("/connect", &mut app, event_tx.clone()).await
                            {
                                return Ok(());
                            }
                        }
                        KeyAction::Reload => {
                            if handle_command("/reload", &mut app, event_tx.clone()).await {
                                return Ok(());
                            }
                        }
                        KeyAction::SendCommand(cmd) => {
                            // Clear crash count after first successful user input
                            // This indicates the client is stable and running normally
                            if !crash_count_cleared {
                                clear_crash_count();
                                crash_count_cleared = true;
                            }

                            app.spell_state.reset();
                            app.suggestion_message = None;

                            if cmd.starts_with('/') {
                                if handle_command(&cmd, &mut app, event_tx.clone()).await {
                                    return Ok(());
                                }
                            } else if app.current_world().connected {
                                if let Some(tx) = &app.current_world().command_tx {
                                    if tx.send(WriteCommand::Text(cmd)).await.is_err() {
                                        app.add_output("Failed to send command");
                                    } else {
                                        let now = std::time::Instant::now();
                                        app.current_world_mut().last_send_time = Some(now);
                                        app.current_world_mut().last_user_command_time = Some(now);
                                        app.current_world_mut().prompt.clear();
                                    }
                                }
                            } else {
                                app.add_output("Not connected. Use /connect <host> <port>");
                            }
                        }
                        KeyAction::UpdateWebSocket => {
                            // Check if we need to start or stop the WebSocket server
                            let ws_enabled = app.settings.ws_enabled;
                            let has_password = !app.settings.websocket_password.is_empty();
                            let is_running = app.ws_server.is_some();

                            if ws_enabled && has_password && !is_running {
                                // Start the server
                                let mut server = WebSocketServer::new(
                                    &app.settings.websocket_password,
                                    app.settings.ws_port,
                                    &app.settings.websocket_allow_list,
                                    app.settings.websocket_whitelisted_host.clone(),
                                );

                                // Configure TLS if secure mode enabled
                                #[cfg(feature = "native-tls-backend")]
                                let tls_configured = if app.settings.web_secure
                                    && !app.settings.websocket_cert_file.is_empty()
                                    && !app.settings.websocket_key_file.is_empty()
                                {
                                    match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
                                        Ok(()) => true,
                                        Err(e) => {
                                            app.add_output(&format!("Warning: Failed to configure TLS: {}", e));
                                            false
                                        }
                                    }
                                } else {
                                    false
                                };
                                #[cfg(feature = "rustls-backend")]
                                let tls_configured = if app.settings.web_secure
                                    && !app.settings.websocket_cert_file.is_empty()
                                    && !app.settings.websocket_key_file.is_empty()
                                {
                                    match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
                                        Ok(()) => true,
                                        Err(e) => {
                                            app.add_output(&format!("Warning: Failed to configure TLS: {}", e));
                                            false
                                        }
                                    }
                                } else {
                                    false
                                };

                                if let Err(e) = start_websocket_server(&mut server, event_tx.clone()).await {
                                    // Don't show error if port is in use (likely another clay instance)
                                    let err_str = e.to_string();
                                    if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                                        app.add_output(&format!("Warning: Failed to start WebSocket server: {}", e));
                                    }
                                } else {
                                    let protocol = if tls_configured { "wss" } else { "ws" };
                                    app.add_output(&format!("WebSocket server started on port {} ({})", app.settings.ws_port, protocol));
                                    app.ws_server = Some(server);
                                }
                            } else if (!ws_enabled || !has_password) && is_running {
                                // Stop the server
                                if let Some(ref mut server) = app.ws_server {
                                    server.stop();
                                }
                                app.ws_server = None;
                                app.add_output("WebSocket server stopped.");
                            }

                            // Check if we need to start or stop the HTTP/HTTPS server
                            {
                                let http_enabled = app.settings.http_enabled;
                                let http_running = app.http_server.is_some();
                                let https_running = app.https_server.is_some();
                                let has_cert = !app.settings.websocket_cert_file.is_empty()
                                    && !app.settings.websocket_key_file.is_empty();
                                let web_secure = app.settings.web_secure;

                                if http_enabled && web_secure && has_cert && !https_running {
                                    // Stop HTTP if running, start HTTPS
                                    if http_running {
                                        app.http_server = None;
                                    }
                                    #[cfg(feature = "native-tls-backend")]
                                    {
                                        let mut https_server = HttpsServer::new(app.settings.http_port);
                                        match start_https_server(
                                            &mut https_server,
                                            &app.settings.websocket_cert_file,
                                            &app.settings.websocket_key_file,
                                            app.settings.ws_port,
                                            true,
                                        ).await {
                                            Ok(()) => {
                                                app.add_output(&format!("HTTPS web interface started on port {} (wss://localhost:{})", app.settings.http_port, app.settings.ws_port));
                                                app.https_server = Some(https_server);
                                            }
                                            Err(e) => {
                                                app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                                            }
                                        }
                                    }
                                    #[cfg(feature = "rustls-backend")]
                                    {
                                        let mut https_server = HttpsServer::new(app.settings.http_port);
                                        match start_https_server(
                                            &mut https_server,
                                            &app.settings.websocket_cert_file,
                                            &app.settings.websocket_key_file,
                                            app.settings.ws_port,
                                            true,
                                        ).await {
                                            Ok(()) => {
                                                app.add_output(&format!("HTTPS web interface started on port {} (wss://localhost:{})", app.settings.http_port, app.settings.ws_port));
                                                app.https_server = Some(https_server);
                                            }
                                            Err(e) => {
                                                app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                                            }
                                        }
                                    }
                                } else if http_enabled && !web_secure && !http_running {
                                    // Stop HTTPS if running, start HTTP
                                    if https_running {
                                        app.https_server = None;
                                    }
                                    let mut http_server = HttpServer::new(app.settings.http_port);
                                    match start_http_server(
                                        &mut http_server,
                                        app.settings.ws_port,
                                        false,
                                    ).await {
                                        Ok(()) => {
                                            app.add_output(&format!("HTTP web interface started on port {} (ws://localhost:{})", app.settings.http_port, app.settings.ws_port));
                                            app.http_server = Some(http_server);
                                        }
                                        Err(e) => {
                                            app.add_output(&format!("Warning: Failed to start HTTP server: {}", e));
                                        }
                                    }
                                } else if !http_enabled && (http_running || https_running) {
                                    // Stop both servers
                                    if http_running {
                                        app.http_server = None;
                                        app.add_output("HTTP web interface stopped.");
                                    }
                                    if https_running {
                                        app.https_server = None;
                                        app.add_output("HTTPS web interface stopped.");
                                    }
                                }
                            }
                        }
                        KeyAction::SwitchedWorld(world_index) => {
                            // Console switched to a new world, broadcast to remote clients
                            app.ws_broadcast(WsMessage::UnseenCleared { world_index });
                        }
                        KeyAction::None => {}
                    }
                    app.check_word_ended();
                }
            }

            // Server events (data from MUD connections)
            Some(event) = event_rx.recv() => {
                match event {
                    AppEvent::ServerData(world_idx, bytes) => {
                        if world_idx < app.worlds.len() {
                            app.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());
                            let is_current = world_idx == app.current_world_index;
                            let data = app.worlds[world_idx].settings.encoding.decode(&bytes);
                            let world_name = app.worlds[world_idx].name.clone();
                            let actions = app.settings.actions.clone();

                            // Process action triggers on complete lines
                            let mut filtered_lines: Vec<&str> = Vec::new();
                            let mut commands_to_execute: Vec<String> = Vec::new();
                            let ends_with_newline = data.ends_with('\n');
                            let lines: Vec<&str> = data.lines().collect();
                            let line_count = lines.len();

                            for (i, line) in lines.iter().enumerate() {
                                let is_last = i == line_count - 1;
                                let is_partial = is_last && !ends_with_newline;

                                // Filter out keep-alive idler message lines
                                if line.contains("###_idler_message_") && line.contains("_###") {
                                    continue;
                                }

                                // Only check triggers on complete lines
                                if !is_partial {
                                    if let Some(result) = check_action_triggers(line, &world_name, &actions) {
                                        // Collect commands to execute
                                        commands_to_execute.extend(result.commands);
                                        // If gagged, skip this line
                                        if result.should_gag {
                                            continue;
                                        }
                                    }
                                }

                                // Add line to filtered data
                                filtered_lines.push(line);
                            }

                            // Rebuild the data with proper newlines
                            let filtered_data = if filtered_lines.is_empty() {
                                String::new()
                            } else {
                                let mut result = filtered_lines.join("\n");
                                if ends_with_newline {
                                    result.push('\n');
                                }
                                result
                            };

                            // Add output to world (if any non-gagged content)
                            if !filtered_data.is_empty() {
                                let settings = app.settings.clone();
                                let output_height = app.output_height;
                                let output_width = app.output_width;
                                app.worlds[world_idx].add_output(&filtered_data, is_current, &settings, output_height, output_width, true);
                                // Check if terminal needs full redraw (after splash clear)
                                if app.worlds[world_idx].needs_redraw {
                                    app.worlds[world_idx].needs_redraw = false;
                                    terminal.clear()?;
                                }
                                // Broadcast filtered data to WebSocket clients
                                // Strip carriage returns - some MUDs send \r\n or bare \r
                                let ws_data = filtered_data.replace('\r', "");
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: ws_data,
                                });
                            }

                            // Execute any triggered commands
                            if let Some(tx) = &app.worlds[world_idx].command_tx {
                                for cmd in commands_to_execute {
                                    // Send command to the MUD (not as a local command)
                                    let _ = tx.try_send(WriteCommand::Text(cmd));
                                }
                            }
                        }
                    }
                    AppEvent::Disconnected(world_idx) => {
                        if world_idx < app.worlds.len() {
                            app.worlds[world_idx].connected = false;
                            app.worlds[world_idx].command_tx = None;
                            app.worlds[world_idx].telnet_mode = false;
                            app.worlds[world_idx].socket_fd = None;
                            app.worlds[world_idx].prompt.clear();
                            // Broadcast to WebSocket clients
                            app.ws_broadcast(WsMessage::WorldDisconnected { world_index: world_idx });
                        }
                    }
                    AppEvent::TelnetDetected(world_idx) => {
                        if world_idx < app.worlds.len() && !app.worlds[world_idx].telnet_mode {
                            app.worlds[world_idx].telnet_mode = true;
                        }
                    }
                    AppEvent::Prompt(world_idx, prompt_bytes) => {
                        if world_idx < app.worlds.len() {
                            app.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());
                            let encoding = app.worlds[world_idx].settings.encoding;
                            let prompt_text = encoding.decode(&prompt_bytes);
                            // Normalize trailing spaces: strip all, then add exactly one
                            let prompt_normalized = format!("{} ", prompt_text.trim_end());
                            app.worlds[world_idx].prompt = prompt_normalized.clone();

                            // Broadcast prompt to WebSocket clients
                            app.ws_broadcast(WsMessage::PromptUpdate {
                                world_index: world_idx,
                                prompt: prompt_normalized,
                            });

                            let world = &mut app.worlds[world_idx];
                            world.prompt_count += 1;

                            // Skip auto-login if flag is set (from /world -l)
                            if world.skip_auto_login {
                                continue;
                            }

                            let auto_type = world.settings.auto_connect_type;
                            let user = world.settings.user.clone();
                            let password = world.settings.password.clone();
                            let prompt_num = world.prompt_count;

                            if !user.is_empty() || !password.is_empty() {
                                let cmd_to_send = match auto_type {
                                    AutoConnectType::Prompt => {
                                        match prompt_num {
                                            1 if !user.is_empty() => Some(user),
                                            2 if !password.is_empty() => Some(password),
                                            _ => None,
                                        }
                                    }
                                    AutoConnectType::MooPrompt => {
                                        match prompt_num {
                                            1 if !user.is_empty() => Some(user.clone()),
                                            2 if !password.is_empty() => Some(password),
                                            3 if !user.is_empty() => Some(user),
                                            _ => None,
                                        }
                                    }
                                    AutoConnectType::Connect => None,
                                };

                                if let Some(cmd) = cmd_to_send {
                                    if let Some(tx) = &world.command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(cmd));
                                        world.last_send_time = Some(std::time::Instant::now());
                                        // Clear prompt since we auto-answered it
                                        world.prompt.clear();
                                    }
                                }
                            }
                        }
                    }
                    AppEvent::SystemMessage(message) => {
                        // Display system message in current world's output
                        app.add_output(&message);
                    }
                    // WebSocket events
                    AppEvent::WsClientConnected(_client_id) => {
                        // Client connected but not yet authenticated - nothing to do
                    }
                    AppEvent::WsClientDisconnected(_client_id) => {
                        // Client disconnected - cleanup handled in handle_ws_client
                    }
                    AppEvent::WsClientMessage(client_id, msg) => {
                        match *msg {
                            WsMessage::AuthRequest { .. } => {
                                // Client just authenticated - send initial state
                                let initial_state = app.build_initial_state();
                                app.ws_send_to_client(client_id, initial_state);
                            }
                            WsMessage::SendCommand { world_index, command } => {
                                // Use shared command parsing
                                let parsed = parse_command(&command);

                                match parsed {
                                    // Commands handled locally on server
                                    Command::ActionCommand { name, args: _ } => {
                                        // Execute action if it exists
                                        if let Some(action) = app.settings.actions.iter().find(|a| a.name.eq_ignore_ascii_case(&name)) {
                                            let commands = split_action_commands(&action.command);
                                            if world_index < app.worlds.len() {
                                                if let Some(tx) = &app.worlds[world_index].command_tx {
                                                    for cmd in commands {
                                                        if cmd.eq_ignore_ascii_case("/gag") || cmd.to_lowercase().starts_with("/gag ") {
                                                            continue;
                                                        }
                                                        let _ = tx.try_send(WriteCommand::Text(cmd));
                                                    }
                                                    app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                                }
                                            }
                                        } else {
                                            app.ws_broadcast(WsMessage::ServerData {
                                                world_index,
                                                data: format!("Unknown action: /{}", name),
                                            });
                                        }
                                    }
                                    Command::NotACommand { text } => {
                                        // Regular text - send to MUD
                                        if world_index < app.worlds.len() {
                                            if let Some(tx) = &app.worlds[world_index].command_tx {
                                                if tx.try_send(WriteCommand::Text(text)).is_ok() {
                                                    app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                                    app.worlds[world_index].prompt.clear();
                                                }
                                            }
                                        }
                                    }
                                    Command::Unknown { cmd } => {
                                        app.ws_broadcast(WsMessage::ServerData {
                                            world_index,
                                            data: format!("Unknown command: {}", cmd),
                                        });
                                    }
                                    _ => {
                                        // All other commands - send to MUD as-is
                                        if world_index < app.worlds.len() {
                                            if let Some(tx) = &app.worlds[world_index].command_tx {
                                                if tx.try_send(WriteCommand::Text(command)).is_ok() {
                                                    app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                                    app.worlds[world_index].prompt.clear();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            WsMessage::SwitchWorld { world_index } => {
                                // Switch current world and broadcast
                                if world_index < app.worlds.len() {
                                    app.switch_world(world_index);
                                    app.ws_broadcast(WsMessage::WorldSwitched { new_index: world_index });
                                }
                            }
                            WsMessage::ConnectWorld { world_index } => {
                                // Trigger connection for specified world
                                if world_index < app.worlds.len() && !app.worlds[world_index].connected {
                                    // Save current world index, switch to target, connect, then restore
                                    let prev_index = app.current_world_index;
                                    app.current_world_index = world_index;
                                    if handle_command("/connect", &mut app, event_tx.clone()).await {
                                        // Quit was triggered (shouldn't happen from /connect)
                                        return Ok(());
                                    }
                                    // Broadcast world connected
                                    let name = app.worlds[world_index].name.clone();
                                    app.ws_broadcast(WsMessage::WorldConnected { world_index, name });
                                    // Restore previous world if it wasn't the target
                                    if prev_index != world_index {
                                        app.current_world_index = prev_index;
                                        // Mark the restored world as seen (data may have arrived during connect)
                                        app.current_world_mut().mark_seen();
                                    }
                                }
                            }
                            WsMessage::DisconnectWorld { world_index } => {
                                // Disconnect specified world
                                if world_index < app.worlds.len() && app.worlds[world_index].connected {
                                    let prev_index = app.current_world_index;
                                    app.current_world_index = world_index;
                                    if handle_command("/disconnect", &mut app, event_tx.clone()).await {
                                        return Ok(());
                                    }
                                    app.current_world_index = prev_index;
                                    // Mark the restored world as seen (data may have arrived during disconnect)
                                    app.current_world_mut().mark_seen();
                                    // WorldDisconnected broadcast happens via AppEvent::Disconnected
                                }
                            }
                            WsMessage::MarkWorldSeen { world_index } => {
                                // A remote client has viewed this world - clear unseen count
                                if world_index < app.worlds.len() {
                                    app.worlds[world_index].mark_seen();
                                    // Broadcast to all clients so they update their UI
                                    app.ws_broadcast(WsMessage::UnseenCleared { world_index });
                                }
                            }
                            WsMessage::UpdateWorldSettings { world_index, name, hostname, port, user, use_ssl, keep_alive_type, keep_alive_cmd } => {
                                // Update world settings from remote client
                                if world_index < app.worlds.len() {
                                    app.worlds[world_index].name = name.clone();
                                    app.worlds[world_index].settings.hostname = hostname.clone();
                                    app.worlds[world_index].settings.port = port.clone();
                                    app.worlds[world_index].settings.user = user.clone();
                                    app.worlds[world_index].settings.use_ssl = use_ssl;
                                    app.worlds[world_index].settings.keep_alive_type = KeepAliveType::from_name(&keep_alive_type);
                                    app.worlds[world_index].settings.keep_alive_cmd = keep_alive_cmd;
                                    // Save settings to persist changes
                                    let _ = save_settings(&app);
                                    // Build settings message for broadcast
                                    let settings_msg = WorldSettingsMsg {
                                        hostname,
                                        port,
                                        user,
                                        use_ssl,
                                        log_file: app.worlds[world_index].settings.log_file.clone(),
                                        encoding: app.worlds[world_index].settings.encoding.name().to_string(),
                                        auto_connect_type: app.worlds[world_index].settings.auto_connect_type.name().to_string(),
                                        keep_alive_type: app.worlds[world_index].settings.keep_alive_type.name().to_string(),
                                        keep_alive_cmd: app.worlds[world_index].settings.keep_alive_cmd.clone(),
                                    };
                                    // Broadcast update to all clients
                                    app.ws_broadcast(WsMessage::WorldSettingsUpdated {
                                        world_index,
                                        settings: settings_msg,
                                        name,
                                    });
                                }
                            }
                            WsMessage::UpdateGlobalSettings { console_theme, gui_theme, input_height, font_name, font_size, ws_allow_list, web_secure, http_enabled, http_port, ws_enabled, ws_port } => {
                                // Update global settings from remote client
                                // Console theme affects the TUI on the server
                                app.settings.theme = Theme::from_name(&console_theme);
                                // GUI theme is stored for sending back to GUI clients
                                app.settings.gui_theme = Theme::from_name(&gui_theme);
                                app.input_height = input_height.clamp(1, 15);
                                app.input.visible_height = app.input_height;
                                app.settings.font_name = font_name;
                                app.settings.font_size = font_size.clamp(8.0, 48.0);
                                app.settings.websocket_allow_list = ws_allow_list.clone();
                                // Update the running WebSocket server's allow list
                                if let Some(ref server) = app.ws_server {
                                    server.update_allow_list(&ws_allow_list);
                                }
                                // Update web settings
                                app.settings.web_secure = web_secure;
                                app.settings.http_enabled = http_enabled;
                                app.settings.http_port = http_port;
                                app.settings.ws_enabled = ws_enabled;
                                app.settings.ws_port = ws_port;
                                // Save settings to persist changes
                                let _ = save_settings(&app);
                                // Build settings message for broadcast
                                let settings_msg = GlobalSettingsMsg {
                                    more_mode_enabled: app.settings.more_mode_enabled,
                                    spell_check_enabled: app.settings.spell_check_enabled,
                                    world_switch_mode: app.settings.world_switch_mode.name().to_string(),
                                    debug_enabled: app.settings.debug_enabled,
                                    show_tags: app.show_tags,
                                    console_theme: app.settings.theme.name().to_string(),
                                    gui_theme: app.settings.gui_theme.name().to_string(),
                                    input_height: app.input_height,
                                    font_name: app.settings.font_name.clone(),
                                    font_size: app.settings.font_size,
                                    ws_allow_list: app.settings.websocket_allow_list.clone(),
                                    web_secure: app.settings.web_secure,
                                    http_enabled: app.settings.http_enabled,
                                    http_port: app.settings.http_port,
                                    ws_enabled: app.settings.ws_enabled,
                                    ws_port: app.settings.ws_port,
                                };
                                // Broadcast update to all clients
                                app.ws_broadcast(WsMessage::GlobalSettingsUpdated {
                                    settings: settings_msg,
                                    input_height: app.input_height,
                                });
                            }
                            WsMessage::UpdateActions { actions } => {
                                // Update actions from remote client
                                app.settings.actions = actions.clone();
                                // Save settings to persist changes
                                let _ = save_settings(&app);
                                // Broadcast update to all clients
                                app.ws_broadcast(WsMessage::ActionsUpdated {
                                    actions,
                                });
                            }
                            WsMessage::CalculateNextWorld { current_index } => {
                                // Calculate next world using shared logic
                                let world_info: Vec<crate::util::WorldSwitchInfo> = app.worlds.iter()
                                    .map(|w| crate::util::WorldSwitchInfo {
                                        name: w.name.clone(),
                                        connected: w.connected,
                                        unseen_lines: w.unseen_lines,
                                    })
                                    .collect();
                                let next_idx = crate::util::calculate_next_world(
                                    &world_info,
                                    current_index,
                                    app.settings.world_switch_mode,
                                );
                                app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: next_idx });
                            }
                            WsMessage::CalculatePrevWorld { current_index } => {
                                // Calculate prev world using shared logic
                                let world_info: Vec<crate::util::WorldSwitchInfo> = app.worlds.iter()
                                    .map(|w| crate::util::WorldSwitchInfo {
                                        name: w.name.clone(),
                                        connected: w.connected,
                                        unseen_lines: w.unseen_lines,
                                    })
                                    .collect();
                                let prev_idx = crate::util::calculate_prev_world(
                                    &world_info,
                                    current_index,
                                    app.settings.world_switch_mode,
                                );
                                app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: prev_idx });
                            }
                            _ => {
                                // Other message types handled elsewhere or ignored
                            }
                        }
                    }
                }
            }

            // Periodic timer for clock updates and keepalive checks (once per minute)
            _ = keepalive_interval.tick() => {
                // Check keepalive for all connected worlds (send NOP if no activity in 5 min)
                for world in &mut app.worlds {
                    if world.connected {
                        // Check last activity time (either send or receive)
                        let last_activity = match (world.last_send_time, world.last_receive_time) {
                            (Some(s), Some(r)) => Some(s.max(r)),
                            (Some(s), None) => Some(s),
                            (None, Some(r)) => Some(r),
                            (None, None) => None,
                        };
                        let should_send = match last_activity {
                            Some(t) => t.elapsed() >= KEEPALIVE_INTERVAL,
                            None => true,
                        };
                        if should_send {
                            if let Some(tx) = &world.command_tx {
                                let now = std::time::Instant::now();

                                // Log the keepalive type being used
                                debug_log(app.settings.debug_enabled, &format!(
                                    "KEEPALIVE_CHECK world='{}' type={:?} cmd='{}'",
                                    world.name, world.settings.keep_alive_type, world.settings.keep_alive_cmd
                                ));

                                // Send keepalive based on type
                                match world.settings.keep_alive_type {
                                    KeepAliveType::None => {
                                        debug_log(app.settings.debug_enabled, &format!(
                                            "KEEPALIVE_SKIP world='{}' - keepalive disabled", world.name
                                        ));
                                        // Don't update times - nothing was sent
                                    }
                                    KeepAliveType::Nop => {
                                        let nop = vec![TELNET_IAC, TELNET_NOP];
                                        let _ = tx.try_send(WriteCommand::Raw(nop));
                                        debug_log_keepalive(app.settings.debug_enabled, &world.name, "NOP", "NOP");
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Custom => {
                                        // Generate random number 1-1000
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let idler_tag = format!("###_idler_message_{}_###", rand_num);
                                        let cmd = world.settings.keep_alive_cmd
                                            .replace("##rand##", &idler_tag);
                                        let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                                        debug_log_keepalive(app.settings.debug_enabled, &world.name, "Custom", &cmd);
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Generic => {
                                        // Generate random number 1-1000
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let cmd = format!("help commands ###_idler_message_{}_###", rand_num);
                                        let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                                        debug_log_keepalive(app.settings.debug_enabled, &world.name, "Generic", &cmd);
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                }
                            }
                        }
                    }
                }
                // Redraw to update the clock display in separator bar
            }
        }

        // Check if any popup is now visible
        let any_popup_visible = app.settings_popup.visible
            || app.world_selector.visible
            || app.confirm_dialog.visible
            || app.worlds_popup.visible
            || app.filter_popup.visible
            || app.help_popup.visible
            || app.actions_popup.visible
            || app.web_popup.visible;

        // If transitioning from no popup to popup, clear terminal to sync ratatui with terminal state
        if any_popup_visible && !app.popup_was_visible {
            terminal.clear()?;
        }
        app.popup_was_visible = any_popup_visible;

        // Use ratatui for everything, but render output area with raw crossterm
        // after the ratatui draw (ratatui's Paragraph has rendering bugs)
        terminal.draw(|f| ui(f, &mut app))?;

        // Render output area with crossterm (ratatui early-returns when no popup visible)
        render_output_crossterm(&app);

        // Process any additional queued server events before next select
        // Track if we processed any events to know if we need to redraw
        let mut processed_events = false;
        while let Ok(event) = event_rx.try_recv() {
            processed_events = true;
            match event {
                AppEvent::ServerData(world_idx, bytes) => {
                    if world_idx < app.worlds.len() {
                        app.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());
                        let is_current = world_idx == app.current_world_index;
                        let data = app.worlds[world_idx].settings.encoding.decode(&bytes);
                        let world_name = app.worlds[world_idx].name.clone();
                        let actions = app.settings.actions.clone();

                        // Process action triggers on complete lines
                        let mut filtered_lines: Vec<&str> = Vec::new();
                        let mut commands_to_execute: Vec<String> = Vec::new();
                        let ends_with_newline = data.ends_with('\n');
                        let lines: Vec<&str> = data.lines().collect();
                        let line_count = lines.len();

                        for (i, line) in lines.iter().enumerate() {
                            let is_last = i == line_count - 1;
                            let is_partial = is_last && !ends_with_newline;

                            // Filter out keep-alive idler message lines
                            if line.contains("###_idler_message_") && line.contains("_###") {
                                continue;
                            }

                            if !is_partial {
                                if let Some(result) = check_action_triggers(line, &world_name, &actions) {
                                    commands_to_execute.extend(result.commands);
                                    if result.should_gag {
                                        continue;
                                    }
                                }
                            }
                            filtered_lines.push(line);
                        }

                        let filtered_data = if filtered_lines.is_empty() {
                            String::new()
                        } else {
                            let mut result = filtered_lines.join("\n");
                            if ends_with_newline {
                                result.push('\n');
                            }
                            result
                        };

                        if !filtered_data.is_empty() {
                            let settings = app.settings.clone();
                            let output_height = app.output_height;
                            let output_width = app.output_width;
                            app.worlds[world_idx].add_output(&filtered_data, is_current, &settings, output_height, output_width, true);
                            if app.worlds[world_idx].needs_redraw {
                                app.worlds[world_idx].needs_redraw = false;
                                let _ = terminal.clear();
                            }
                            // Strip carriage returns for WebSocket - some MUDs send \r\n or bare \r
                            let ws_data = filtered_data.replace('\r', "");
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index: world_idx,
                                data: ws_data,
                            });
                        }

                        if let Some(tx) = &app.worlds[world_idx].command_tx {
                            for cmd in commands_to_execute {
                                let _ = tx.try_send(WriteCommand::Text(cmd));
                            }
                        }
                    }
                }
                AppEvent::Disconnected(world_idx) => {
                    if world_idx < app.worlds.len() {
                        app.worlds[world_idx].connected = false;
                        app.worlds[world_idx].command_tx = None;
                        app.worlds[world_idx].telnet_mode = false;
                        app.worlds[world_idx].socket_fd = None;
                        app.worlds[world_idx].prompt.clear();
                        app.ws_broadcast(WsMessage::WorldDisconnected { world_index: world_idx });
                    }
                }
                AppEvent::TelnetDetected(world_idx) => {
                    if world_idx < app.worlds.len() && !app.worlds[world_idx].telnet_mode {
                        app.worlds[world_idx].telnet_mode = true;
                    }
                }
                AppEvent::Prompt(world_idx, prompt_bytes) => {
                    if world_idx < app.worlds.len() {
                        app.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());
                        let encoding = app.worlds[world_idx].settings.encoding;
                        let prompt_text = encoding.decode(&prompt_bytes);
                        // Normalize trailing spaces: strip all, then add exactly one
                        let prompt_normalized = format!("{} ", prompt_text.trim_end());
                        app.worlds[world_idx].prompt = prompt_normalized.clone();
                        app.ws_broadcast(WsMessage::PromptUpdate {
                            world_index: world_idx,
                            prompt: prompt_normalized,
                        });

                        let world = &mut app.worlds[world_idx];
                        world.prompt_count += 1;

                        // Skip auto-login if flag is set (from /world -l)
                        if world.skip_auto_login {
                            continue;
                        }

                        let auto_type = world.settings.auto_connect_type;
                        let user = world.settings.user.clone();
                        let password = world.settings.password.clone();
                        let prompt_num = world.prompt_count;

                        if !user.is_empty() || !password.is_empty() {
                            let cmd_to_send = match auto_type {
                                AutoConnectType::Prompt => {
                                    match prompt_num {
                                        1 if !user.is_empty() => Some(user),
                                        2 if !password.is_empty() => Some(password),
                                        _ => None,
                                    }
                                }
                                AutoConnectType::MooPrompt => {
                                    match prompt_num {
                                        1 if !user.is_empty() => Some(user.clone()),
                                        2 if !password.is_empty() => Some(password),
                                        3 if !user.is_empty() => Some(user),
                                        _ => None,
                                    }
                                }
                                AutoConnectType::Connect => None,
                            };

                            if let Some(cmd) = cmd_to_send {
                                if let Some(tx) = &world.command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(cmd));
                                    world.last_send_time = Some(std::time::Instant::now());
                                    // Clear prompt since we auto-answered it
                                    world.prompt.clear();
                                }
                            }
                        }
                    }
                }
                AppEvent::SystemMessage(message) => {
                    // Display system message in current world's output
                    app.add_output(&message);
                }
                // WebSocket events (drain loop - complex handlers use primary loop)
                AppEvent::WsClientConnected(_) => {}
                AppEvent::WsClientDisconnected(_) => {}
                AppEvent::WsClientMessage(client_id, msg) => {
                    // Handle simple messages in drain loop
                    match *msg {
                        WsMessage::AuthRequest { .. } => {
                            let initial_state = app.build_initial_state();
                            app.ws_send_to_client(client_id, initial_state);
                        }
                        WsMessage::SendCommand { world_index, command } => {
                            // Use shared command parsing
                            let parsed = parse_command(&command);

                            match parsed {
                                // Commands handled locally on server
                                Command::ActionCommand { name, args: _ } => {
                                    // Execute action if it exists
                                    if let Some(action) = app.settings.actions.iter().find(|a| a.name.eq_ignore_ascii_case(&name)) {
                                        let commands = split_action_commands(&action.command);
                                        if world_index < app.worlds.len() {
                                            if let Some(tx) = &app.worlds[world_index].command_tx {
                                                for cmd in commands {
                                                    if cmd.eq_ignore_ascii_case("/gag") || cmd.to_lowercase().starts_with("/gag ") {
                                                        continue;
                                                    }
                                                    let _ = tx.try_send(WriteCommand::Text(cmd));
                                                }
                                                app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                            }
                                        }
                                    } else {
                                        app.ws_broadcast(WsMessage::ServerData {
                                            world_index,
                                            data: format!("Unknown action: /{}", name),
                                        });
                                    }
                                }
                                Command::NotACommand { text } => {
                                    // Regular text - send to MUD
                                    if world_index < app.worlds.len() {
                                        if let Some(tx) = &app.worlds[world_index].command_tx {
                                            if tx.try_send(WriteCommand::Text(text)).is_ok() {
                                                app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                                app.worlds[world_index].prompt.clear();
                                            }
                                        }
                                    }
                                }
                                Command::Unknown { cmd } => {
                                    app.ws_broadcast(WsMessage::ServerData {
                                        world_index,
                                        data: format!("Unknown command: {}", cmd),
                                    });
                                }
                                _ => {
                                    // All other commands - send to MUD as-is
                                    // These are either handled by the GUI/web interface locally
                                    // or should be passed through to the MUD
                                    if world_index < app.worlds.len() {
                                        if let Some(tx) = &app.worlds[world_index].command_tx {
                                            if tx.try_send(WriteCommand::Text(command)).is_ok() {
                                                app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                                app.worlds[world_index].prompt.clear();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        WsMessage::SwitchWorld { world_index } => {
                            if world_index < app.worlds.len() {
                                app.switch_world(world_index);
                                app.ws_broadcast(WsMessage::WorldSwitched { new_index: world_index });
                            }
                        }
                        WsMessage::UpdateWorldSettings { world_index, name, hostname, port, user, use_ssl, keep_alive_type, keep_alive_cmd } => {
                            if world_index < app.worlds.len() {
                                app.worlds[world_index].name = name.clone();
                                app.worlds[world_index].settings.hostname = hostname.clone();
                                app.worlds[world_index].settings.port = port.clone();
                                app.worlds[world_index].settings.user = user.clone();
                                app.worlds[world_index].settings.use_ssl = use_ssl;
                                app.worlds[world_index].settings.keep_alive_type = KeepAliveType::from_name(&keep_alive_type);
                                app.worlds[world_index].settings.keep_alive_cmd = keep_alive_cmd;
                                let _ = save_settings(&app);
                                let settings_msg = WorldSettingsMsg {
                                    hostname, port, user, use_ssl,
                                    log_file: app.worlds[world_index].settings.log_file.clone(),
                                    encoding: app.worlds[world_index].settings.encoding.name().to_string(),
                                    auto_connect_type: app.worlds[world_index].settings.auto_connect_type.name().to_string(),
                                    keep_alive_type: app.worlds[world_index].settings.keep_alive_type.name().to_string(),
                                    keep_alive_cmd: app.worlds[world_index].settings.keep_alive_cmd.clone(),
                                };
                                app.ws_broadcast(WsMessage::WorldSettingsUpdated { world_index, settings: settings_msg, name });
                            }
                        }
                        WsMessage::UpdateGlobalSettings { console_theme, gui_theme, input_height, font_name, font_size, ws_allow_list, web_secure, http_enabled, http_port, ws_enabled, ws_port } => {
                            app.settings.theme = Theme::from_name(&console_theme);
                            app.settings.gui_theme = Theme::from_name(&gui_theme);
                            app.input_height = input_height.clamp(1, 15);
                            app.input.visible_height = app.input_height;
                            app.settings.font_name = font_name;
                            app.settings.font_size = font_size.clamp(8.0, 48.0);
                            app.settings.websocket_allow_list = ws_allow_list.clone();
                            if let Some(ref server) = app.ws_server {
                                server.update_allow_list(&ws_allow_list);
                            }
                            app.settings.web_secure = web_secure;
                            app.settings.http_enabled = http_enabled;
                            app.settings.http_port = http_port;
                            app.settings.ws_enabled = ws_enabled;
                            app.settings.ws_port = ws_port;
                            let _ = save_settings(&app);
                            let settings_msg = GlobalSettingsMsg {
                                more_mode_enabled: app.settings.more_mode_enabled,
                                spell_check_enabled: app.settings.spell_check_enabled,
                                world_switch_mode: app.settings.world_switch_mode.name().to_string(),
                                debug_enabled: app.settings.debug_enabled,
                                show_tags: app.show_tags,
                                console_theme: app.settings.theme.name().to_string(),
                                gui_theme: app.settings.gui_theme.name().to_string(),
                                input_height: app.input_height,
                                font_name: app.settings.font_name.clone(),
                                font_size: app.settings.font_size,
                                ws_allow_list: app.settings.websocket_allow_list.clone(),
                                web_secure: app.settings.web_secure,
                                http_enabled: app.settings.http_enabled,
                                http_port: app.settings.http_port,
                                ws_enabled: app.settings.ws_enabled,
                                ws_port: app.settings.ws_port,
                            };
                            app.ws_broadcast(WsMessage::GlobalSettingsUpdated { settings: settings_msg, input_height: app.input_height });
                        }
                        WsMessage::UpdateActions { actions } => {
                            app.settings.actions = actions.clone();
                            let _ = save_settings(&app);
                            app.ws_broadcast(WsMessage::ActionsUpdated { actions });
                        }
                        WsMessage::CalculateNextWorld { current_index } => {
                            let world_info: Vec<crate::util::WorldSwitchInfo> = app.worlds.iter()
                                .map(|w| crate::util::WorldSwitchInfo {
                                    name: w.name.clone(),
                                    connected: w.connected,
                                    unseen_lines: w.unseen_lines,
                                })
                                .collect();
                            let next_idx = crate::util::calculate_next_world(
                                &world_info,
                                current_index,
                                app.settings.world_switch_mode,
                            );
                            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: next_idx });
                        }
                        WsMessage::CalculatePrevWorld { current_index } => {
                            let world_info: Vec<crate::util::WorldSwitchInfo> = app.worlds.iter()
                                .map(|w| crate::util::WorldSwitchInfo {
                                    name: w.name.clone(),
                                    connected: w.connected,
                                    unseen_lines: w.unseen_lines,
                                })
                                .collect();
                            let prev_idx = crate::util::calculate_prev_world(
                                &world_info,
                                current_index,
                                app.settings.world_switch_mode,
                            );
                            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: prev_idx });
                        }
                        _ => {}
                    }
                }
            }
        }

        // If we processed events in try_recv, redraw before waiting in select
        if processed_events {
            terminal.draw(|f| ui(f, &mut app))?;
            render_output_crossterm(&app);
        }
    }
}

enum KeyAction {
    Quit,
    SendCommand(String),
    Connect, // Trigger connection from settings popup
    Redraw,  // Force screen redraw
    Reload,  // Trigger /reload
    UpdateWebSocket, // Check and update WebSocket server state
    SwitchedWorld(usize), // Console switched to this world, broadcast unseen clear
    None,
}

fn handle_key_event(key: KeyEvent, app: &mut App) -> KeyAction {
    // Handle confirm dialog first (highest priority)
    if app.confirm_dialog.visible {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                // Toggle between Yes and No
                app.confirm_dialog.yes_selected = !app.confirm_dialog.yes_selected;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.confirm_dialog.yes_selected = true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.confirm_dialog.yes_selected = false;
            }
            KeyCode::Enter => {
                if app.confirm_dialog.yes_selected {
                    // Execute the action
                    match app.confirm_dialog.action {
                        ConfirmAction::DeleteWorld(world_index) => {
                            // Delete the world
                            if app.worlds.len() > 1 {
                                // Save the world name before deletion
                                let world_name = app.worlds[world_index].name.clone();
                                // Disconnect if connected
                                app.worlds[world_index].connected = false;
                                app.worlds[world_index].command_tx = None;
                                app.worlds.remove(world_index);
                                // Adjust current world index
                                if app.current_world_index >= app.worlds.len() {
                                    app.current_world_index = app.worlds.len() - 1;
                                }
                                app.add_output("");
                                app.add_output(&format!("World '{}' deleted.", world_name));
                                app.add_output("");
                                // Save settings to persist deletion
                                let _ = save_settings(app);
                            } else {
                                app.add_output("");
                                app.add_output("Cannot delete the last world.");
                                app.add_output("");
                            }
                        }
                        ConfirmAction::None => {}
                    }
                }
                app.confirm_dialog.close();
                app.settings_popup.close();
            }
            KeyCode::Esc => {
                // Cancel - just close the dialog
                app.confirm_dialog.close();
            }
            _ => {}
        }
        return KeyAction::None;
    }

    // Handle worlds popup input (simple OK dialog)
    if app.worlds_popup.visible {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                app.worlds_popup.close();
            }
            _ => {}
        }
        return KeyAction::None;
    }

    // Handle help popup input
    if app.help_popup.visible {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.help_popup.close();
            }
            KeyCode::Up => {
                app.help_popup.scroll_up();
            }
            KeyCode::Down => {
                // Calculate visible height (popup height - borders - blank line - button line)
                let visible_height = 16usize.saturating_sub(4);
                app.help_popup.scroll_down(visible_height);
            }
            KeyCode::PageUp => {
                for _ in 0..5 {
                    app.help_popup.scroll_up();
                }
            }
            KeyCode::PageDown => {
                let visible_height = 16usize.saturating_sub(4);
                for _ in 0..5 {
                    app.help_popup.scroll_down(visible_height);
                }
            }
            _ => {}
        }
        return KeyAction::None;
    }

    // Handle actions popup input (split into List, Editor, and ConfirmDelete views)
    if app.actions_popup.visible {
        match app.actions_popup.view {
            ActionsView::ConfirmDelete => {
                // Confirm delete dialog
                match key.code {
                    KeyCode::Esc => {
                        app.actions_popup.close_confirm_delete();
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        app.actions_popup.confirm_selected = !app.actions_popup.confirm_selected;
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        // Delete and return to list
                        app.actions_popup.delete_selected_action();
                        app.settings.actions = app.actions_popup.actions.clone();
                        let _ = save_settings(app);
                        app.ws_broadcast(WsMessage::ActionsUpdated {
                            actions: app.settings.actions.clone(),
                        });
                        app.actions_popup.close_confirm_delete();
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        app.actions_popup.close_confirm_delete();
                    }
                    KeyCode::Enter => {
                        if app.actions_popup.confirm_selected {
                            // Yes - delete
                            app.actions_popup.delete_selected_action();
                            app.settings.actions = app.actions_popup.actions.clone();
                            let _ = save_settings(app);
                            app.ws_broadcast(WsMessage::ActionsUpdated {
                                actions: app.settings.actions.clone(),
                            });
                        }
                        app.actions_popup.close_confirm_delete();
                    }
                    _ => {}
                }
            }
            ActionsView::Editor => {
                // Editor view
                match key.code {
                    KeyCode::Esc => {
                        app.actions_popup.close_editor();
                    }
                    KeyCode::Tab => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            app.actions_popup.prev_editor_field();
                        } else {
                            app.actions_popup.next_editor_field();
                        }
                    }
                    KeyCode::Up => {
                        app.actions_popup.prev_editor_field();
                    }
                    KeyCode::Down => {
                        app.actions_popup.next_editor_field();
                    }
                    KeyCode::Left => {
                        match app.actions_popup.editor_field {
                            ActionEditorField::Name | ActionEditorField::World |
                            ActionEditorField::Pattern | ActionEditorField::Command => {
                                app.actions_popup.move_cursor_left();
                            }
                            ActionEditorField::CancelButton => {
                                app.actions_popup.editor_field = ActionEditorField::SaveButton;
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Right => {
                        match app.actions_popup.editor_field {
                            ActionEditorField::Name | ActionEditorField::World |
                            ActionEditorField::Pattern | ActionEditorField::Command => {
                                app.actions_popup.move_cursor_right();
                            }
                            ActionEditorField::SaveButton => {
                                app.actions_popup.editor_field = ActionEditorField::CancelButton;
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Home => {
                        app.actions_popup.move_cursor_home();
                    }
                    KeyCode::End => {
                        app.actions_popup.move_cursor_end();
                    }
                    KeyCode::Backspace => {
                        app.actions_popup.delete_char();
                    }
                    KeyCode::Enter => {
                        match app.actions_popup.editor_field {
                            ActionEditorField::SaveButton => {
                                if app.actions_popup.save_current_action() {
                                    app.settings.actions = app.actions_popup.actions.clone();
                                    let _ = save_settings(app);
                                    app.ws_broadcast(WsMessage::ActionsUpdated {
                                        actions: app.settings.actions.clone(),
                                    });
                                    app.actions_popup.close_editor();
                                }
                            }
                            ActionEditorField::CancelButton => {
                                app.actions_popup.close_editor();
                            }
                            _ => {
                                app.actions_popup.next_editor_field();
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        match app.actions_popup.editor_field {
                            ActionEditorField::Name | ActionEditorField::World |
                            ActionEditorField::Pattern | ActionEditorField::Command => {
                                app.actions_popup.insert_char(c);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            ActionsView::List => {
                // List view
                match key.code {
                    KeyCode::Esc => {
                        app.actions_popup.close();
                    }
                    KeyCode::Tab => {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            app.actions_popup.prev_list_field();
                        } else {
                            app.actions_popup.next_list_field();
                        }
                    }
                    KeyCode::Up => {
                        if app.actions_popup.list_field == ActionListField::List {
                            app.actions_popup.select_prev_action();
                        } else {
                            app.actions_popup.list_field = ActionListField::List;
                        }
                    }
                    KeyCode::Down => {
                        if app.actions_popup.list_field == ActionListField::List {
                            app.actions_popup.select_next_action();
                        } else {
                            app.actions_popup.list_field = ActionListField::List;
                        }
                    }
                    KeyCode::Left => {
                        match app.actions_popup.list_field {
                            ActionListField::EditButton => {
                                app.actions_popup.list_field = ActionListField::AddButton;
                            }
                            ActionListField::DeleteButton => {
                                app.actions_popup.list_field = ActionListField::EditButton;
                            }
                            ActionListField::CancelButton => {
                                app.actions_popup.list_field = ActionListField::DeleteButton;
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Right => {
                        match app.actions_popup.list_field {
                            ActionListField::AddButton => {
                                app.actions_popup.list_field = ActionListField::EditButton;
                            }
                            ActionListField::EditButton => {
                                app.actions_popup.list_field = ActionListField::DeleteButton;
                            }
                            ActionListField::DeleteButton => {
                                app.actions_popup.list_field = ActionListField::CancelButton;
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Enter => {
                        match app.actions_popup.list_field {
                            ActionListField::List => {
                                // Edit selected action
                                if !app.actions_popup.actions.is_empty() {
                                    let idx = app.actions_popup.selected_index;
                                    app.actions_popup.open_editor(Some(idx));
                                }
                            }
                            ActionListField::AddButton => {
                                app.actions_popup.open_editor(None);
                            }
                            ActionListField::EditButton => {
                                if !app.actions_popup.actions.is_empty() {
                                    let idx = app.actions_popup.selected_index;
                                    app.actions_popup.open_editor(Some(idx));
                                }
                            }
                            ActionListField::DeleteButton => {
                                app.actions_popup.open_confirm_delete();
                            }
                            ActionListField::CancelButton => {
                                app.actions_popup.close();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        return KeyAction::None;
    }

    // Handle filter popup input
    if app.filter_popup.visible {
        match key.code {
            KeyCode::Esc => {
                app.filter_popup.close();
            }
            KeyCode::F(4) => {
                // F4 again closes the filter
                app.filter_popup.close();
            }
            KeyCode::Backspace => {
                if app.filter_popup.cursor > 0 {
                    app.filter_popup.cursor -= 1;
                    app.filter_popup.filter_text.remove(app.filter_popup.cursor);
                    let output_lines = app.current_world().output_lines.clone();
                    app.filter_popup.update_filter(&output_lines);
                }
            }
            KeyCode::Delete => {
                if app.filter_popup.cursor < app.filter_popup.filter_text.len() {
                    app.filter_popup.filter_text.remove(app.filter_popup.cursor);
                    let output_lines = app.current_world().output_lines.clone();
                    app.filter_popup.update_filter(&output_lines);
                }
            }
            KeyCode::Left | KeyCode::Char('b') if key.code == KeyCode::Left || key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Left or Ctrl+B = cursor left
                if app.filter_popup.cursor > 0 {
                    app.filter_popup.cursor -= 1;
                }
            }
            KeyCode::Right | KeyCode::Char('f') if key.code == KeyCode::Right || key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Right or Ctrl+F = cursor right
                if app.filter_popup.cursor < app.filter_popup.filter_text.len() {
                    app.filter_popup.cursor += 1;
                }
            }
            KeyCode::Home => {
                app.filter_popup.cursor = 0;
            }
            KeyCode::End => {
                app.filter_popup.cursor = app.filter_popup.filter_text.len();
            }
            KeyCode::PageUp => {
                // Scroll up in filtered results
                let visible_height = app.output_height as usize;
                app.filter_popup.scroll_offset = app.filter_popup.scroll_offset
                    .saturating_sub(visible_height.saturating_sub(2));
            }
            KeyCode::PageDown => {
                // Scroll down in filtered results
                let visible_height = app.output_height as usize;
                let max_offset = app.filter_popup.filtered_indices.len().saturating_sub(1);
                app.filter_popup.scroll_offset = (app.filter_popup.scroll_offset + visible_height.saturating_sub(2))
                    .min(max_offset);
            }
            KeyCode::Char(c) => {
                app.filter_popup.filter_text.insert(app.filter_popup.cursor, c);
                app.filter_popup.cursor += 1;
                let output_lines = app.current_world().output_lines.clone();
                app.filter_popup.update_filter(&output_lines);
            }
            _ => {}
        }
        return KeyAction::None;
    }

    // Handle settings popup input
    if app.settings_popup.visible {
        if app.settings_popup.editing {
            // Text editing mode - inline editing
            match key.code {
                KeyCode::Esc => {
                    // Cancel and close popup
                    app.settings_popup.cancel_edit();
                    app.settings_popup.close();
                }
                KeyCode::Backspace => {
                    if app.settings_popup.edit_cursor > 0 {
                        app.settings_popup.edit_cursor -= 1;
                        app.settings_popup.edit_buffer.remove(app.settings_popup.edit_cursor);
                        app.settings_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Delete => {
                    if app.settings_popup.edit_cursor < app.settings_popup.edit_buffer.len() {
                        app.settings_popup.edit_buffer.remove(app.settings_popup.edit_cursor);
                    }
                }
                KeyCode::Left => {
                    if app.settings_popup.edit_cursor > 0 {
                        app.settings_popup.edit_cursor -= 1;
                        app.settings_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Right => {
                    if app.settings_popup.edit_cursor < app.settings_popup.edit_buffer.len() {
                        app.settings_popup.edit_cursor += 1;
                        app.settings_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Ctrl+B = Left (backward)
                    if app.settings_popup.edit_cursor > 0 {
                        app.settings_popup.edit_cursor -= 1;
                        app.settings_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Ctrl+F = Right (forward)
                    if app.settings_popup.edit_cursor < app.settings_popup.edit_buffer.len() {
                        app.settings_popup.edit_cursor += 1;
                        app.settings_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Home => {
                    app.settings_popup.edit_cursor = 0;
                    app.settings_popup.adjust_scroll(33);
                }
                KeyCode::End => {
                    app.settings_popup.edit_cursor = app.settings_popup.edit_buffer.len();
                    app.settings_popup.adjust_scroll(33);
                }
                KeyCode::Up => {
                    // Commit and move to previous field
                    app.settings_popup.commit_edit();
                    app.settings_popup.prev_field();
                    // Auto-start editing if new field is text
                    if app.settings_popup.selected_field.is_text_field() {
                        app.settings_popup.start_edit();
                    }
                }
                KeyCode::Down | KeyCode::Enter | KeyCode::Tab => {
                    // Commit and move to next field
                    app.settings_popup.commit_edit();
                    app.settings_popup.next_field();
                    // Auto-start editing if new field is text
                    if app.settings_popup.selected_field.is_text_field() {
                        app.settings_popup.start_edit();
                    }
                }
                KeyCode::Char(c) => {
                    app.settings_popup.edit_buffer.insert(app.settings_popup.edit_cursor, c);
                    app.settings_popup.edit_cursor += 1;
                    app.settings_popup.adjust_scroll(33);
                }
                _ => {}
            }
        } else {
            // Navigation mode
            match key.code {
                KeyCode::Esc => {
                    app.settings_popup.close();
                }
                KeyCode::Enter => {
                    if app.settings_popup.selected_field.is_text_field() {
                        // Start editing text field
                        app.settings_popup.start_edit();
                    } else if app.settings_popup.selected_field.is_button() {
                        // Handle buttons
                        match app.settings_popup.selected_field {
                            SettingsField::Connect => {
                                // Save settings first (to the world being edited)
                                let idx = app.settings_popup.editing_world_index.unwrap_or(app.current_world_index);
                                let (new_input_height, new_show_tags) = app.settings_popup
                                    .apply(&mut app.settings, &mut app.worlds[idx]);
                                app.input_height = new_input_height;
                                app.input.visible_height = new_input_height;
                                app.show_tags = new_show_tags;
                                if let Err(e) = save_settings(app) {
                                    app.add_output(&format!("Warning: Could not save settings: {}", e));
                                }
                                app.settings_popup.close();
                                // Return Connect action if not already connected
                                if !app.current_world().connected {
                                    return KeyAction::Connect;
                                } else {
                                    app.add_output("Already connected.");
                                }
                            }
                            SettingsField::SaveSetup => {
                                // Save global settings
                                let (new_input_height, new_show_tags) = app.settings_popup.apply_global(&mut app.settings);
                                app.input_height = new_input_height;
                                app.input.visible_height = new_input_height;
                                app.show_tags = new_show_tags;
                                // Log that debug was enabled/disabled
                                if app.settings.debug_enabled {
                                    debug_log(true, "Debug logging enabled");
                                    app.add_output("Debug logging enabled - writing to ~/clay.debug.log");
                                }
                                if let Err(e) = save_settings(app) {
                                    app.add_output(&format!("Warning: Could not save settings: {}", e));
                                }
                                app.settings_popup.close();
                                // Update WebSocket server state based on new settings
                                return KeyAction::UpdateWebSocket;
                            }
                            SettingsField::CancelSetup => {
                                // Just close without saving
                                app.settings_popup.close();
                            }
                            SettingsField::SaveWorld => {
                                // Save world settings (to the world being edited)
                                let idx = app.settings_popup.editing_world_index.unwrap_or(app.current_world_index);
                                let (new_input_height, new_show_tags) = app.settings_popup
                                    .apply(&mut app.settings, &mut app.worlds[idx]);
                                app.input_height = new_input_height;
                                app.input.visible_height = new_input_height;
                                app.show_tags = new_show_tags;
                                if let Err(e) = save_settings(app) {
                                    app.add_output(&format!("Warning: Could not save settings: {}", e));
                                }
                                app.settings_popup.close();
                            }
                            SettingsField::CancelWorld => {
                                // Just close without saving
                                app.settings_popup.close();
                            }
                            SettingsField::DeleteWorld => {
                                // Show confirmation dialog
                                let world_name = app.current_world().name.clone();
                                let world_index = app.current_world_index;
                                app.confirm_dialog.show_delete_world(&world_name, world_index);
                            }
                            _ => {}
                        }
                    } else {
                        // Toggle/cycle for other fields
                        app.settings_popup.toggle_or_cycle();
                    }
                }
                KeyCode::Up => {
                    if app.settings_popup.selected_field.is_button() {
                        // When on a button, cycle between buttons (same as Left)
                        app.settings_popup.selected_field = match app.settings_popup.selected_field {
                            SettingsField::SaveWorld => SettingsField::Connect,
                            SettingsField::CancelWorld => SettingsField::SaveWorld,
                            SettingsField::DeleteWorld => SettingsField::CancelWorld,
                            SettingsField::Connect => SettingsField::DeleteWorld,
                            SettingsField::SaveSetup => SettingsField::CancelSetup,
                            SettingsField::CancelSetup => SettingsField::SaveSetup,
                            other => other,
                        };
                    } else {
                        app.settings_popup.prev_field();
                        // Auto-start editing if text field
                        if app.settings_popup.selected_field.is_text_field() {
                            app.settings_popup.start_edit();
                        }
                    }
                }
                KeyCode::Down => {
                    if app.settings_popup.selected_field.is_button() {
                        // When on a button, cycle between buttons (same as Right)
                        app.settings_popup.selected_field = match app.settings_popup.selected_field {
                            SettingsField::SaveWorld => SettingsField::CancelWorld,
                            SettingsField::CancelWorld => SettingsField::DeleteWorld,
                            SettingsField::DeleteWorld => SettingsField::Connect,
                            SettingsField::Connect => SettingsField::SaveWorld,
                            SettingsField::SaveSetup => SettingsField::CancelSetup,
                            SettingsField::CancelSetup => SettingsField::SaveSetup,
                            other => other,
                        };
                    } else {
                        app.settings_popup.next_field();
                        // Auto-start editing if text field
                        if app.settings_popup.selected_field.is_text_field() {
                            app.settings_popup.start_edit();
                        }
                    }
                }
                KeyCode::Tab => {
                    app.settings_popup.next_field();
                    // Auto-start editing if text field
                    if app.settings_popup.selected_field.is_text_field() {
                        app.settings_popup.start_edit();
                    }
                }
                KeyCode::Left => {
                    // Decrement for InputHeight, cycle backwards for Encoding/AutoConnect, or move between buttons
                    if app.settings_popup.selected_field == SettingsField::InputHeight {
                        if app.settings_popup.temp_input_height > 1 {
                            app.settings_popup.temp_input_height -= 1;
                        }
                    } else if app.settings_popup.selected_field == SettingsField::Encoding {
                        app.settings_popup.temp_encoding = app.settings_popup.temp_encoding.prev();
                    } else if app.settings_popup.selected_field == SettingsField::AutoConnect {
                        app.settings_popup.temp_auto_connect_type = app.settings_popup.temp_auto_connect_type.prev();
                    } else if app.settings_popup.selected_field == SettingsField::KeepAlive {
                        app.settings_popup.temp_keep_alive_type = app.settings_popup.temp_keep_alive_type.prev();
                    } else if app.settings_popup.selected_field.is_button() {
                        // Move to previous button (world settings buttons)
                        app.settings_popup.selected_field = match app.settings_popup.selected_field {
                            SettingsField::SaveWorld => SettingsField::Connect,
                            SettingsField::CancelWorld => SettingsField::SaveWorld,
                            SettingsField::DeleteWorld => SettingsField::CancelWorld,
                            SettingsField::Connect => SettingsField::DeleteWorld,
                            // Setup mode buttons
                            SettingsField::SaveSetup => SettingsField::CancelSetup,
                            SettingsField::CancelSetup => SettingsField::SaveSetup,
                            other => other,
                        };
                    }
                }
                KeyCode::Right => {
                    // Increment for InputHeight, cycle forwards for Encoding/AutoConnect, or move between buttons
                    if app.settings_popup.selected_field == SettingsField::InputHeight {
                        if app.settings_popup.temp_input_height < 15 {
                            app.settings_popup.temp_input_height += 1;
                        }
                    } else if app.settings_popup.selected_field == SettingsField::Encoding {
                        app.settings_popup.temp_encoding = app.settings_popup.temp_encoding.next();
                    } else if app.settings_popup.selected_field == SettingsField::AutoConnect {
                        app.settings_popup.temp_auto_connect_type = app.settings_popup.temp_auto_connect_type.next();
                    } else if app.settings_popup.selected_field == SettingsField::KeepAlive {
                        app.settings_popup.temp_keep_alive_type = app.settings_popup.temp_keep_alive_type.next();
                    } else if app.settings_popup.selected_field.is_button() {
                        // Move to next button (world settings buttons)
                        app.settings_popup.selected_field = match app.settings_popup.selected_field {
                            SettingsField::SaveWorld => SettingsField::CancelWorld,
                            SettingsField::CancelWorld => SettingsField::DeleteWorld,
                            SettingsField::DeleteWorld => SettingsField::Connect,
                            SettingsField::Connect => SettingsField::SaveWorld,
                            // Setup mode buttons
                            SettingsField::SaveSetup => SettingsField::CancelSetup,
                            SettingsField::CancelSetup => SettingsField::SaveSetup,
                            other => other,
                        };
                    }
                }
                KeyCode::Char(' ') => {
                    if app.settings_popup.selected_field.is_text_field() {
                        app.settings_popup.start_edit();
                        // Insert the space
                        app.settings_popup.edit_buffer.push(' ');
                        app.settings_popup.edit_cursor += 1;
                        app.settings_popup.adjust_scroll(33);
                    } else if app.settings_popup.selected_field.is_button() {
                        // Handle Connect button with Space too
                        if app.settings_popup.selected_field == SettingsField::Connect {
                            let idx = app.settings_popup.editing_world_index.unwrap_or(app.current_world_index);
                            let (new_input_height, new_show_tags) = app.settings_popup
                                .apply(&mut app.settings, &mut app.worlds[idx]);
                            app.input_height = new_input_height;
                            app.input.visible_height = new_input_height;
                            app.show_tags = new_show_tags;
                            if let Err(e) = save_settings(app) {
                                app.add_output(&format!("Warning: Could not save settings: {}", e));
                            }
                            app.settings_popup.close();
                            if !app.current_world().connected {
                                return KeyAction::Connect;
                            } else {
                                app.add_output("Already connected.");
                            }
                        }
                    } else {
                        app.settings_popup.toggle_or_cycle();
                    }
                }
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Ctrl+S to save (to the world being edited)
                    let idx = app.settings_popup.editing_world_index.unwrap_or(app.current_world_index);
                    let (new_input_height, new_show_tags) = app.settings_popup
                        .apply(&mut app.settings, &mut app.worlds[idx]);
                    app.input_height = new_input_height;
                    app.input.visible_height = new_input_height;
                    app.show_tags = new_show_tags;
                    if !app.settings.more_mode_enabled {
                        app.worlds[idx].release_all_pending();
                    }
                    app.settings_popup.close();
                    if let Err(e) = save_settings(app) {
                        app.add_output(&format!("Warning: Could not save settings: {}", e));
                    } else {
                        app.add_output("Settings saved");
                    }
                }
                KeyCode::Char(c) => {
                    // Start editing on any character for text fields
                    if app.settings_popup.selected_field.is_text_field() {
                        app.settings_popup.start_edit();
                        app.settings_popup.edit_buffer.push(c);
                        app.settings_popup.edit_cursor += 1;
                        app.settings_popup.adjust_scroll(33);
                    }
                }
                _ => {}
            }
        }
        return KeyAction::None;
    }

    // Handle web popup input
    if app.web_popup.visible {
        if app.web_popup.editing {
            // Text editing mode
            match key.code {
                KeyCode::Esc => {
                    app.web_popup.cancel_edit();
                    app.web_popup.close();
                }
                KeyCode::Backspace => {
                    if app.web_popup.edit_cursor > 0 {
                        app.web_popup.edit_cursor -= 1;
                        app.web_popup.edit_buffer.remove(app.web_popup.edit_cursor);
                        app.web_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Delete => {
                    if app.web_popup.edit_cursor < app.web_popup.edit_buffer.len() {
                        app.web_popup.edit_buffer.remove(app.web_popup.edit_cursor);
                    }
                }
                KeyCode::Left => {
                    if app.web_popup.edit_cursor > 0 {
                        app.web_popup.edit_cursor -= 1;
                        app.web_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Right => {
                    if app.web_popup.edit_cursor < app.web_popup.edit_buffer.len() {
                        app.web_popup.edit_cursor += 1;
                        app.web_popup.adjust_scroll(33);
                    }
                }
                KeyCode::Home => {
                    app.web_popup.edit_cursor = 0;
                    app.web_popup.adjust_scroll(33);
                }
                KeyCode::End => {
                    app.web_popup.edit_cursor = app.web_popup.edit_buffer.len();
                    app.web_popup.adjust_scroll(33);
                }
                KeyCode::Up => {
                    app.web_popup.commit_edit();
                    app.web_popup.prev_field();
                    if app.web_popup.selected_field.is_text_field() {
                        app.web_popup.start_edit();
                    }
                }
                KeyCode::Down | KeyCode::Enter | KeyCode::Tab => {
                    app.web_popup.commit_edit();
                    app.web_popup.next_field();
                    if app.web_popup.selected_field.is_text_field() {
                        app.web_popup.start_edit();
                    }
                }
                KeyCode::Char(c) => {
                    app.web_popup.edit_buffer.insert(app.web_popup.edit_cursor, c);
                    app.web_popup.edit_cursor += 1;
                    app.web_popup.adjust_scroll(33);
                }
                _ => {}
            }
        } else {
            // Navigation mode
            match key.code {
                KeyCode::Esc => {
                    app.web_popup.close();
                }
                KeyCode::Enter => {
                    if app.web_popup.selected_field.is_text_field() {
                        app.web_popup.start_edit();
                    } else if app.web_popup.selected_field.is_button() {
                        match app.web_popup.selected_field {
                            WebField::SaveWeb => {
                                app.web_popup.apply(&mut app.settings);
                                if let Err(e) = save_settings(app) {
                                    app.add_output(&format!("Warning: Could not save settings: {}", e));
                                }
                                app.web_popup.close();
                                return KeyAction::UpdateWebSocket;
                            }
                            WebField::CancelWeb => {
                                app.web_popup.close();
                            }
                            _ => {}
                        }
                    } else {
                        app.web_popup.toggle_option();
                    }
                }
                KeyCode::Up => {
                    if app.web_popup.selected_field.is_button() {
                        app.web_popup.selected_field = match app.web_popup.selected_field {
                            WebField::SaveWeb => WebField::CancelWeb,
                            WebField::CancelWeb => WebField::SaveWeb,
                            other => other,
                        };
                    } else {
                        app.web_popup.prev_field();
                        if app.web_popup.selected_field.is_text_field() {
                            app.web_popup.start_edit();
                        }
                    }
                }
                KeyCode::Down => {
                    if app.web_popup.selected_field.is_button() {
                        app.web_popup.selected_field = match app.web_popup.selected_field {
                            WebField::SaveWeb => WebField::CancelWeb,
                            WebField::CancelWeb => WebField::SaveWeb,
                            other => other,
                        };
                    } else {
                        app.web_popup.next_field();
                        if app.web_popup.selected_field.is_text_field() {
                            app.web_popup.start_edit();
                        }
                    }
                }
                KeyCode::Tab => {
                    app.web_popup.next_field();
                    if app.web_popup.selected_field.is_text_field() {
                        app.web_popup.start_edit();
                    }
                }
                KeyCode::Left | KeyCode::Right => {
                    if app.web_popup.selected_field.is_button() {
                        app.web_popup.selected_field = match app.web_popup.selected_field {
                            WebField::SaveWeb => WebField::CancelWeb,
                            WebField::CancelWeb => WebField::SaveWeb,
                            other => other,
                        };
                    }
                }
                KeyCode::Char(' ') => {
                    if app.web_popup.selected_field.is_text_field() {
                        app.web_popup.start_edit();
                        app.web_popup.edit_buffer.push(' ');
                        app.web_popup.edit_cursor += 1;
                        app.web_popup.adjust_scroll(33);
                    } else if !app.web_popup.selected_field.is_button() {
                        app.web_popup.toggle_option();
                    }
                }
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.web_popup.apply(&mut app.settings);
                    app.web_popup.close();
                    if let Err(e) = save_settings(app) {
                        app.add_output(&format!("Warning: Could not save settings: {}", e));
                    } else {
                        app.add_output("Web settings saved");
                    }
                    return KeyAction::UpdateWebSocket;
                }
                KeyCode::Char(c) => {
                    if app.web_popup.selected_field.is_text_field() {
                        app.web_popup.start_edit();
                        app.web_popup.edit_buffer.push(c);
                        app.web_popup.edit_cursor += 1;
                        app.web_popup.adjust_scroll(33);
                    }
                }
                _ => {}
            }
        }
        return KeyAction::None;
    }

    // Handle world selector popup input
    if app.world_selector.visible {
        if app.world_selector.editing_filter {
            // Filter text editing mode
            match key.code {
                KeyCode::Esc => {
                    // Stop editing filter but keep popup open
                    app.world_selector.stop_filter_edit();
                }
                KeyCode::Enter => {
                    // Stop editing and connect to selected world
                    app.world_selector.stop_filter_edit();
                    let idx = app.world_selector.selected_index;
                    app.world_selector.close();
                    app.switch_world(idx);
                    if !app.current_world().connected {
                        let has_settings = !app.current_world().settings.hostname.is_empty()
                            && !app.current_world().settings.port.is_empty();
                        if has_settings {
                            return KeyAction::Connect;
                        }
                    }
                }
                KeyCode::Backspace => {
                    if app.world_selector.filter_cursor > 0 {
                        app.world_selector.filter_cursor -= 1;
                        app.world_selector.filter.remove(app.world_selector.filter_cursor);
                        // Reset selection to first filtered item
                        let indices = app.world_selector.filtered_indices(&app.worlds);
                        if !indices.is_empty() {
                            app.world_selector.selected_index = indices[0];
                        }
                    }
                }
                KeyCode::Delete => {
                    if app.world_selector.filter_cursor < app.world_selector.filter.len() {
                        app.world_selector.filter.remove(app.world_selector.filter_cursor);
                        let indices = app.world_selector.filtered_indices(&app.worlds);
                        if !indices.is_empty() {
                            app.world_selector.selected_index = indices[0];
                        }
                    }
                }
                KeyCode::Left => {
                    if app.world_selector.filter_cursor > 0 {
                        app.world_selector.filter_cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if app.world_selector.filter_cursor < app.world_selector.filter.len() {
                        app.world_selector.filter_cursor += 1;
                    }
                }
                KeyCode::Home => {
                    app.world_selector.filter_cursor = 0;
                }
                KeyCode::End => {
                    app.world_selector.filter_cursor = app.world_selector.filter.len();
                }
                KeyCode::Up => {
                    // In filter mode, just move through list (ignore if at edge)
                    let _ = app.world_selector.move_up(&app.worlds);
                }
                KeyCode::Down => {
                    // In filter mode, just move through list (ignore if at edge)
                    let _ = app.world_selector.move_down(&app.worlds);
                }
                KeyCode::Char(c) => {
                    app.world_selector.filter.insert(app.world_selector.filter_cursor, c);
                    app.world_selector.filter_cursor += 1;
                    // Reset selection to first filtered item
                    let indices = app.world_selector.filtered_indices(&app.worlds);
                    if !indices.is_empty() {
                        app.world_selector.selected_index = indices[0];
                    }
                }
                _ => {}
            }
        } else {
            // Navigation mode
            match key.code {
                KeyCode::Esc => {
                    app.world_selector.close();
                }
                KeyCode::Tab => {
                    // Cycle focus: List -> Edit -> Connect -> Cancel -> List
                    app.world_selector.next_focus();
                }
                KeyCode::BackTab => {
                    // Reverse cycle
                    app.world_selector.prev_focus();
                }
                KeyCode::Enter => {
                    // Action depends on current focus
                    match app.world_selector.focus {
                        WorldSelectorFocus::List | WorldSelectorFocus::ConnectButton => {
                            // Connect to selected world
                            let idx = app.world_selector.selected_index;
                            app.world_selector.close();
                            app.switch_world(idx);
                            if !app.current_world().connected {
                                let has_settings = !app.current_world().settings.hostname.is_empty()
                                    && !app.current_world().settings.port.is_empty();
                                if has_settings {
                                    return KeyAction::Connect;
                                }
                            }
                        }
                        WorldSelectorFocus::AddButton => {
                            // Create new world and open editor
                            let new_idx = app.worlds.len();
                            let new_name = format!("world{}", new_idx + 1);
                            app.worlds.push(World::new(&new_name));
                            app.switch_world(new_idx);  // Switch to the new world
                            app.world_selector.close();
                            let input_height = app.input_height;
                            let show_tags = app.show_tags;
                            app.settings_popup.open(&app.settings, &app.worlds[new_idx], new_idx, input_height, show_tags);
                        }
                        WorldSelectorFocus::EditButton => {
                            // Edit selected world (don't switch current world)
                            let idx = app.world_selector.selected_index;
                            app.world_selector.close();
                            let input_height = app.input_height;
                            let show_tags = app.show_tags;
                            app.settings_popup.open(&app.settings, &app.worlds[idx], idx, input_height, show_tags);
                        }
                        WorldSelectorFocus::CancelButton => {
                            // Close without action
                            app.world_selector.close();
                        }
                    }
                }
                KeyCode::Up => {
                    if app.world_selector.focus == WorldSelectorFocus::List {
                        // Try to move up in list, if at top go to last button
                        if app.world_selector.move_up(&app.worlds) {
                            app.world_selector.focus = WorldSelectorFocus::CancelButton;
                        }
                    } else if app.world_selector.focus == WorldSelectorFocus::AddButton {
                        // At first button, go back to last item in list
                        app.world_selector.focus = WorldSelectorFocus::List;
                        app.world_selector.move_to_last(&app.worlds);
                    } else {
                        // Move to previous button
                        app.world_selector.prev_focus();
                    }
                }
                KeyCode::Down => {
                    if app.world_selector.focus == WorldSelectorFocus::List {
                        // Try to move down in list, if at bottom go to first button
                        if app.world_selector.move_down(&app.worlds) {
                            app.world_selector.focus = WorldSelectorFocus::AddButton;
                        }
                    } else if app.world_selector.focus == WorldSelectorFocus::CancelButton {
                        // At last button, go back to first item in list
                        app.world_selector.focus = WorldSelectorFocus::List;
                        app.world_selector.move_to_first(&app.worlds);
                    } else {
                        // Move to next button
                        app.world_selector.next_focus();
                    }
                }
                KeyCode::Left => {
                    // Move between buttons
                    if app.world_selector.focus != WorldSelectorFocus::List {
                        app.world_selector.prev_focus();
                        // Skip List when using Left/Right
                        if app.world_selector.focus == WorldSelectorFocus::List {
                            app.world_selector.focus = WorldSelectorFocus::CancelButton;
                        }
                    }
                }
                KeyCode::Right => {
                    // Move between buttons
                    if app.world_selector.focus != WorldSelectorFocus::List {
                        app.world_selector.next_focus();
                        // Skip List when using Left/Right
                        if app.world_selector.focus == WorldSelectorFocus::List {
                            app.world_selector.focus = WorldSelectorFocus::AddButton;
                        }
                    }
                }
                KeyCode::Char('/') => {
                    // Start filter editing
                    app.world_selector.focus = WorldSelectorFocus::List;
                    app.world_selector.start_filter_edit();
                }
                KeyCode::Char(c) => {
                    // Start filter editing with this character (only from List focus)
                    if app.world_selector.focus == WorldSelectorFocus::List {
                        app.world_selector.start_filter_edit();
                        app.world_selector.filter.push(c);
                        app.world_selector.filter_cursor = app.world_selector.filter.len();
                        // Reset selection to first filtered item
                        let indices = app.world_selector.filtered_indices(&app.worlds);
                        if !indices.is_empty() {
                            app.world_selector.selected_index = indices[0];
                        }
                    }
                }
                _ => {}
            }
        }
        return KeyAction::None;
    }

    // Handle Tab when paused - release one screenful of lines
    if app.current_world().paused && key.code == KeyCode::Tab && key.modifiers.is_empty() {
        let batch_size = (app.output_height as usize).saturating_sub(2);
        let world_idx = app.current_world_index;
        app.current_world_mut().release_pending(batch_size);
        // Broadcast updated pending count to GUI clients
        let pending_count = app.worlds[world_idx].pending_lines.len();
        app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: pending_count });
        return KeyAction::None;
    }

    // Helper to check if escape was pressed recently (for Escape+key sequences)
    let recent_escape = app.last_escape
        .map(|t| t.elapsed() < Duration::from_millis(500))
        .unwrap_or(false);

    // Track bare Escape key presses for Escape+key sequences
    if key.code == KeyCode::Esc && key.modifiers.is_empty() {
        app.last_escape = Some(std::time::Instant::now());
        return KeyAction::None;
    }

    // Handle Escape+j (Alt+j) to jump to end - release all pending
    if key.code == KeyCode::Char('j') && (key.modifiers.contains(KeyModifiers::ALT) || recent_escape) {
        app.last_escape = None; // Clear escape state
        if app.current_world().paused {
            let world_idx = app.current_world_index;
            app.current_world_mut().release_all_pending();
            // Broadcast updated pending count to GUI clients
            app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: 0 });
        }
        return KeyAction::None;
    }

    // Handle Escape+w (Alt+w) to switch to world with oldest pending output
    if key.code == KeyCode::Char('w') && (key.modifiers.contains(KeyModifiers::ALT) || recent_escape) {
        app.last_escape = None; // Clear escape state
        app.switch_to_oldest_pending();
        return KeyAction::None;
    }

    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            // Check if we pressed Ctrl+C within the last 15 seconds
            if let Some(last_time) = app.last_ctrl_c {
                if last_time.elapsed() < Duration::from_secs(15) {
                    return KeyAction::Quit;
                }
            }
            // First Ctrl+C or timeout - show message and record time
            app.last_ctrl_c = Some(std::time::Instant::now());
            app.add_output("Press Ctrl+C again within 15 seconds to exit, or use /quit");
            KeyAction::None
        }

        // Ctrl+L to redraw screen
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => KeyAction::Redraw,

        // Ctrl+R to reload
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => KeyAction::Reload,

        // F1 to open help popup
        (_, KeyCode::F(1)) => {
            app.help_popup.open();
            KeyAction::None
        }

        // F2 to toggle tag display
        (_, KeyCode::F(2)) => {
            app.show_tags = !app.show_tags;
            KeyAction::Redraw // Force full screen redraw to apply change
        }

        // F4 to open filter popup
        (_, KeyCode::F(4)) => {
            app.filter_popup.open();
            let output_lines = app.current_world().output_lines.clone();
            app.filter_popup.update_filter(&output_lines);
            KeyAction::None
        }

        // Switch worlds (Up/Down without modifiers)
        (KeyModifiers::NONE, KeyCode::Up) => {
            app.prev_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            app.next_world();
            KeyAction::SwitchedWorld(app.current_world_index)
        }

        // Resize input area
        (KeyModifiers::CONTROL, KeyCode::Up) => {
            app.increase_input_height();
            KeyAction::None
        }
        (KeyModifiers::CONTROL, KeyCode::Down) => {
            app.decrease_input_height();
            KeyAction::None
        }

        // Switch worlds (all that have been connected)
        (KeyModifiers::SHIFT, KeyCode::Up) => {
            app.prev_world_all();
            KeyAction::SwitchedWorld(app.current_world_index)
        }
        (KeyModifiers::SHIFT, KeyCode::Down) => {
            app.next_world_all();
            KeyAction::SwitchedWorld(app.current_world_index)
        }

        // Clear input
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            app.input.clear();
            app.spell_state.reset();
            app.suggestion_message = None;
            KeyAction::None
        }

        // Delete word before cursor
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            app.input.delete_word_before_cursor();
            app.spell_state.reset();
            app.suggestion_message = None;
            KeyAction::None
        }

        // History navigation
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.input.history_prev();
            app.spell_state.reset();
            KeyAction::None
        }
        (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
            app.input.history_next();
            app.spell_state.reset();
            KeyAction::None
        }

        // Spell check
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
            app.handle_spell_check();
            KeyAction::None
        }

        // Submit
        (_, KeyCode::Enter) => {
            // If paused, release all pending and submit
            if app.current_world().paused {
                let world_idx = app.current_world_index;
                app.current_world_mut().release_all_pending();
                // Broadcast updated pending count to GUI clients
                app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: 0 });
            }
            let input = app.input.take_input();
            // Allow empty input to be sent if connected (some MUDs use empty lines)
            if !input.is_empty() || app.current_world().connected {
                // Reset more mode counter now, before any queued output is processed
                app.current_world_mut().lines_since_pause = 0;
                KeyAction::SendCommand(input)
            } else {
                KeyAction::None
            }
        }

        // Editing
        (_, KeyCode::Backspace) => {
            app.input.delete_char();
            KeyAction::None
        }
        (_, KeyCode::Delete) => {
            app.input.delete_char_forward();
            KeyAction::None
        }

        // Cursor movement
        (_, KeyCode::Left) | (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
            app.input.move_cursor_left();
            KeyAction::None
        }
        (_, KeyCode::Right) | (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            app.input.move_cursor_right();
            KeyAction::None
        }
        (_, KeyCode::Home) => {
            app.input.home();
            KeyAction::None
        }
        (_, KeyCode::End) => {
            app.input.end();
            KeyAction::None
        }

        // Output scrolling (history)
        (_, KeyCode::PageUp) => {
            app.scroll_output_up();
            KeyAction::None
        }
        (_, KeyCode::PageDown) => {
            app.scroll_output_down();
            KeyAction::None
        }

        // Character input
        (_, KeyCode::Char(c)) => {
            if !c.is_alphabetic() && app.spell_state.showing_suggestions {
                app.spell_state.reset();
            }
            app.input.insert_char(c);
            KeyAction::None
        }

        _ => KeyAction::None,
    }
}

async fn handle_command(cmd: &str, app: &mut App, event_tx: mpsc::Sender<AppEvent>) -> bool {
    let parsed = parse_command(cmd);

    match parsed {
        Command::Help => {
            app.help_popup.open();
        }
        Command::Quit => {
            return true; // Signal to quit
        }
        Command::Setup => {
            // Open settings popup for global settings only
            app.settings_popup.open_setup(&app.settings, app.input_height, app.show_tags);
        }
        Command::Web => {
            // Open web settings popup
            app.web_popup.open(&app.settings);
        }
        Command::WorldSelector => {
            // /world (no args) - show world selector popup
            app.world_selector.open(app.current_world_index);
        }
        Command::WorldEdit { name } => {
            // /world -e or /world -e <name>
            let idx = if let Some(ref world_name) = name {
                // /world -e <name> - find or create the world, then edit
                app.find_or_create_world(world_name)
            } else {
                // /world -e - edit current world
                app.current_world_index
            };
            let input_height = app.input_height;
            let show_tags = app.show_tags;
            app.settings_popup.open(&app.settings, &app.worlds[idx], idx, input_height, show_tags);
        }
        Command::WorldConnectNoLogin { name } => {
            // /world -l <name> - connect without auto-login
            if let Some(idx) = app.find_world(&name) {
                app.switch_world(idx);
                if !app.current_world().connected {
                    let has_settings = !app.current_world().settings.hostname.is_empty()
                        && !app.current_world().settings.port.is_empty();
                    if has_settings {
                        // Set flag to skip auto-login
                        app.current_world_mut().skip_auto_login = true;
                        return Box::pin(handle_command("/connect", app, event_tx)).await;
                    } else {
                        app.add_output(&format!("World '{}' has no connection settings.", name));
                    }
                }
            } else {
                app.add_output(&format!("World '{}' not found.", name));
            }
        }
        Command::WorldSwitch { name } => {
            // /world <name> - connect to world if exists, else show editor for new world
            if let Some(idx) = app.find_world(&name) {
                // World exists - switch to it and connect if has settings
                app.switch_world(idx);
                if !app.current_world().connected {
                    let has_settings = !app.current_world().settings.hostname.is_empty()
                        && !app.current_world().settings.port.is_empty();
                    if has_settings {
                        return Box::pin(handle_command("/connect", app, event_tx)).await;
                    } else {
                        // No settings configured - open editor
                        let input_height = app.input_height;
                        let show_tags = app.show_tags;
                        app.settings_popup.open(&app.settings, &app.worlds[idx], idx, input_height, show_tags);
                    }
                }
            } else {
                // World doesn't exist - create it and show editor
                let idx = app.find_or_create_world(&name);
                app.switch_world(idx);
                let input_height = app.input_height;
                let show_tags = app.show_tags;
                app.settings_popup.open(&app.settings, &app.worlds[idx], idx, input_height, show_tags);
            }
        }
        Command::WorldsList => {
            let current_idx = app.current_world_index;
            let screen_width = app.output_width;
            app.worlds_popup.show(&app.worlds, current_idx, screen_width);
        }
        Command::Keepalive => {
            // Show keepalive settings for all worlds
            let current_idx = app.current_world_index;
            let lines: Vec<String> = app.worlds.iter().enumerate().map(|(idx, world)| {
                let current = if idx == current_idx { "*" } else { " " };
                let connected = if world.connected { "✓" } else { " " };
                let type_str = world.settings.keep_alive_type.name();
                let cmd_str = if world.settings.keep_alive_cmd.is_empty() {
                    "(none)".to_string()
                } else {
                    world.settings.keep_alive_cmd.clone()
                };
                let last_nop = match world.last_nop_time {
                    Some(t) => format!("{:.0}s ago", t.elapsed().as_secs_f64()),
                    None => "never".to_string(),
                };
                format!(
                    "{}{} {:15} type={:8} cmd={:30} last={}",
                    current, connected, world.name, type_str, cmd_str, last_nop
                )
            }).collect();

            app.add_output("");
            app.add_output("Keepalive Settings for All Worlds:");
            app.add_output("─".repeat(70).as_str());
            for line in lines {
                app.add_output(&line);
            }
            app.add_output("─".repeat(70).as_str());
            app.add_output("(*=current, ✓=connected)");
        }
        Command::Actions => {
            app.actions_popup.open(&app.settings.actions);
        }
        Command::Connect { host: arg_host, port: arg_port, ssl: arg_ssl } => {
            if app.current_world().connected {
                app.add_output("Already connected. Use /disconnect first.");
                return false;
            }

            // Determine host/port/ssl: use args if provided, else use stored settings
            let world_settings = &app.current_world().settings;
            let (host, port, use_ssl) = if let (Some(h), Some(p)) = (arg_host, arg_port) {
                (h, p, arg_ssl)
            } else if !world_settings.hostname.is_empty() && !world_settings.port.is_empty() {
                (
                    world_settings.hostname.clone(),
                    world_settings.port.clone(),
                    world_settings.use_ssl,
                )
            } else {
                app.add_output("Usage: /connect [<host> <port> [ssl]]");
                app.add_output("Or configure host/port in world settings (/world)");
                return false;
            };

            let ssl_msg = if use_ssl { " with SSL" } else { "" };
            app.add_output("");
            app.add_output(&format!("Connecting to {}:{}{}...", host, port, ssl_msg));
            app.add_output("");

            match TcpStream::connect(format!("{}:{}", host, port)).await {
                Ok(tcp_stream) => {
                    // Store the socket fd for hot reload (before splitting)
                    let socket_fd = tcp_stream.as_raw_fd();

                    // Handle SSL if needed
                    let (mut read_half, mut write_half): (StreamReader, StreamWriter) = if use_ssl {
                        #[cfg(feature = "native-tls-backend")]
                        {
                            // Accept invalid/expired certificates (common for MUD servers)
                            let connector = match native_tls::TlsConnector::builder()
                                .danger_accept_invalid_certs(true)
                                .build()
                            {
                                Ok(c) => c,
                                Err(e) => {
                                    app.add_output(&format!("TLS error: {}", e));
                                    return false;
                                }
                            };
                            let connector = tokio_native_tls::TlsConnector::from(connector);

                            match connector.connect(&host, tcp_stream).await {
                                Ok(tls_stream) => {
                                    app.add_output("SSL handshake successful!");
                                    // For TLS, we can't preserve the connection across reload
                                    app.current_world_mut().socket_fd = None;
                                    app.current_world_mut().is_tls = true;
                                    let (r, w) = tokio::io::split(tls_stream);
                                    (StreamReader::Tls(r), StreamWriter::Tls(w))
                                }
                                Err(e) => {
                                    app.add_output(&format!("SSL handshake failed: {}", e));
                                    return false;
                                }
                            }
                        }

                        #[cfg(feature = "rustls-backend")]
                        {
                            use std::sync::Arc;
                            use rustls::RootCertStore;
                            use tokio_rustls::TlsConnector;
                            use rustls::pki_types::ServerName;

                            // Create a config that accepts invalid certs (common for MUD servers)
                            let mut root_store = RootCertStore::empty();
                            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

                            let config = rustls::ClientConfig::builder()
                                .dangerous()
                                .with_custom_certificate_verifier(Arc::new(danger::NoCertificateVerification::new()))
                                .with_no_client_auth();

                            let connector = TlsConnector::from(Arc::new(config));
                            let server_name = match ServerName::try_from(host.clone()) {
                                Ok(sn) => sn,
                                Err(e) => {
                                    app.add_output(&format!("Invalid server name: {}", e));
                                    return false;
                                }
                            };

                            match connector.connect(server_name, tcp_stream).await {
                                Ok(tls_stream) => {
                                    app.add_output("SSL handshake successful!");
                                    app.current_world_mut().socket_fd = None;
                                    app.current_world_mut().is_tls = true;
                                    let (r, w) = tokio::io::split(tls_stream);
                                    (StreamReader::Tls(r), StreamWriter::Tls(w))
                                }
                                Err(e) => {
                                    app.add_output(&format!("SSL handshake failed: {}", e));
                                    return false;
                                }
                            }
                        }

                        #[cfg(not(any(feature = "native-tls-backend", feature = "rustls-backend")))]
                        {
                            app.add_output("No TLS backend available. Compile with native-tls-backend or rustls-backend feature.");
                            return false;
                        }
                    } else {
                        // Store fd for plain TCP connections (can be preserved across reload)
                        app.current_world_mut().socket_fd = Some(socket_fd);
                        app.current_world_mut().is_tls = false;
                        let (r, w) = tcp_stream.into_split();
                        (StreamReader::Plain(r), StreamWriter::Plain(w))
                    };

                    app.current_world_mut().connected = true;
                    app.current_world_mut().was_connected = true;
                    app.current_world_mut().prompt_count = 0; // Reset for auto-login
                    // Initialize timing for NOP tracking
                    let now = std::time::Instant::now();
                    app.current_world_mut().last_send_time = Some(now);
                    app.current_world_mut().last_receive_time = Some(now);

                    // Mark this world as no longer initial (if it was)
                    app.current_world_mut().is_initial_world = false;

                    // Discard any unused initial world now that we have a real connection
                    app.discard_initial_world();

                    // Re-capture world_idx after potential discard (indices may have shifted)
                    let world_idx = app.current_world_index;

                    // Open log file if configured
                    let log_path = app.current_world().settings.log_file.clone();
                    if let Some(log_path) = log_path {
                        match std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&log_path)
                        {
                            Ok(file) => {
                                app.current_world_mut().log_handle =
                                    Some(std::sync::Arc::new(std::sync::Mutex::new(file)));
                                app.add_output(&format!("Logging to: {}", log_path));
                            }
                            Err(e) => {
                                app.add_output(&format!("Warning: Could not open log file: {}", e));
                            }
                        }
                    }

                    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                    app.current_world_mut().command_tx = Some(cmd_tx.clone());

                    // Send "connect <user> <password>" if configured and auto_connect_type is Connect
                    // Skip if skip_auto_login flag is set (from /world -l)
                    let skip_login = app.current_world().skip_auto_login;
                    // Reset flag so future reconnects will try auto-login again
                    app.current_world_mut().skip_auto_login = false;
                    let user = app.current_world().settings.user.clone();
                    let password = app.current_world().settings.password.clone();
                    let auto_connect_type = app.current_world().settings.auto_connect_type;
                    if !skip_login && !user.is_empty() && auto_connect_type == AutoConnectType::Connect {
                        let tx = cmd_tx.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            let connect_cmd = format!("connect {} {}", user, password);
                            let _ = tx.send(WriteCommand::Text(connect_cmd)).await;
                        });
                    }

                    // Clone tx for use in reader (for telnet responses)
                    let telnet_tx = cmd_tx.clone();
                    let event_tx_read = event_tx.clone();
                    tokio::spawn(async move {
                        let mut buffer = BytesMut::with_capacity(4096);
                        buffer.resize(4096, 0);
                        let mut line_buffer: Vec<u8> = Vec::new();

                        loop {
                            match read_half.read(&mut buffer).await {
                                Ok(0) => {
                                    // Send any remaining buffered data
                                    if !line_buffer.is_empty() {
                                        let (cleaned, responses, detected, prompt) = process_telnet(&line_buffer);
                                        if !responses.is_empty() {
                                            let _ = telnet_tx.send(WriteCommand::Raw(responses)).await;
                                        }
                                        if detected {
                                            let _ = event_tx_read.send(AppEvent::TelnetDetected(world_idx)).await;
                                        }
                                        // Send prompt FIRST for immediate auto-login response
                                        if let Some(prompt_bytes) = prompt {
                                            let _ = event_tx_read.send(AppEvent::Prompt(world_idx, prompt_bytes)).await;
                                        }
                                        // Send remaining data
                                        if !cleaned.is_empty() {
                                            let _ = event_tx_read.send(AppEvent::ServerData(world_idx, cleaned)).await;
                                        }
                                    }
                                    let _ = event_tx_read
                                        .send(AppEvent::ServerData(
                                            world_idx,
                                            "Connection closed by server.".as_bytes().to_vec(),
                                        ))
                                        .await;
                                    let _ =
                                        event_tx_read.send(AppEvent::Disconnected(world_idx)).await;
                                    break;
                                }
                                Ok(n) => {
                                    // Append new data to line buffer
                                    line_buffer.extend_from_slice(&buffer[..n]);

                                    // Find safe split point (complete lines with complete ANSI sequences)
                                    let split_at = find_safe_split_point(&line_buffer);

                                    // Send data immediately - either up to split point, or all if no incomplete sequences
                                    let to_send = if split_at > 0 {
                                        line_buffer.drain(..split_at).collect()
                                    } else if !line_buffer.is_empty() {
                                        // No safe split point but we have data - send it anyway
                                        std::mem::take(&mut line_buffer)
                                    } else {
                                        Vec::new()
                                    };

                                    if !to_send.is_empty() {
                                        // Process telnet sequences
                                        let (cleaned, responses, detected, prompt) = process_telnet(&to_send);

                                        // Send telnet responses if any
                                        if !responses.is_empty() {
                                            let _ = telnet_tx.send(WriteCommand::Raw(responses)).await;
                                        }

                                        // Notify if telnet detected
                                        if detected {
                                            let _ = event_tx_read
                                                .send(AppEvent::TelnetDetected(world_idx))
                                                .await;
                                        }

                                        // Send prompt FIRST if detected via telnet GA
                                        if let Some(prompt_bytes) = prompt {
                                            let _ = event_tx_read
                                                .send(AppEvent::Prompt(world_idx, prompt_bytes))
                                                .await;
                                        }

                                        // Send cleaned data to main loop
                                        if !cleaned.is_empty()
                                            && event_tx_read
                                                .send(AppEvent::ServerData(world_idx, cleaned))
                                                .await
                                                .is_err()
                                        {
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let msg = format!("Read error: {}", e);
                                    let _ = event_tx_read
                                        .send(AppEvent::ServerData(world_idx, msg.into_bytes()))
                                        .await;
                                    let _ =
                                        event_tx_read.send(AppEvent::Disconnected(world_idx)).await;
                                    break;
                                }
                            }
                        }
                    });

                    tokio::spawn(async move {
                        while let Some(cmd) = cmd_rx.recv().await {
                            let bytes = match cmd {
                                WriteCommand::Text(text) => format!("{}\r\n", text).into_bytes(),
                                WriteCommand::Raw(raw) => raw,
                            };
                            if write_half.write_all(&bytes).await.is_err() {
                                break;
                            }
                        }
                    });
                }
                Err(e) => {
                    app.add_output(&format!("Connection failed: {}", e));
                }
            }
        }
        Command::Disconnect => {
            if app.current_world().connected {
                app.current_world_mut().command_tx = None;
                app.current_world_mut().connected = false;
                app.current_world_mut().socket_fd = None;
                app.current_world_mut().log_handle = None;
                app.current_world_mut().prompt.clear();
                app.add_output("Disconnected.");
            } else {
                app.add_output("Not connected.");
            }
        }
        Command::Send { text, all_worlds, target_world, no_newline } => {
            // Send the text
            let send_to_world = |world: &mut World, text: &str, no_newline: bool| -> bool {
                if !world.connected {
                    return false;
                }
                if let Some(tx) = &world.command_tx {
                    let result = if no_newline {
                        tx.try_send(WriteCommand::Raw(text.as_bytes().to_vec()))
                    } else {
                        tx.try_send(WriteCommand::Text(text.to_string()))
                    };
                    if result.is_ok() {
                        world.last_send_time = Some(std::time::Instant::now());
                        return true;
                    }
                }
                false
            };

            if all_worlds {
                // Send to all connected worlds
                let mut sent_count = 0;
                for world in &mut app.worlds {
                    if send_to_world(world, &text, no_newline) {
                        sent_count += 1;
                    }
                }
                if sent_count == 0 {
                    app.add_output("No connected worlds to send to.");
                }
            } else if let Some(world_name) = target_world {
                // Send to specific world
                if let Some(idx) = app.find_world(&world_name) {
                    if !send_to_world(&mut app.worlds[idx], &text, no_newline) {
                        app.add_output(&format!("World '{}' is not connected.", world_name));
                    }
                } else {
                    app.add_output(&format!("World '{}' not found.", world_name));
                }
            } else {
                // Send to current world
                let world = app.current_world_mut();
                if !world.connected {
                    app.add_output("Not connected. Use /connect first.");
                } else if let Some(tx) = &world.command_tx {
                    let result = if no_newline {
                        tx.try_send(WriteCommand::Raw(text.as_bytes().to_vec()))
                    } else {
                        tx.try_send(WriteCommand::Text(text.to_string()))
                    };
                    if result.is_ok() {
                        world.last_send_time = Some(std::time::Instant::now());
                    } else {
                        app.add_output("Failed to send command.");
                    }
                }
            }
        }
        Command::Reload => {
            // First check if we can find the executable (handling " (deleted)" suffix)
            let exe_path = match get_executable_path() {
                Ok((p, _)) => p,
                Err(e) => {
                    app.add_output(&format!("Cannot find executable: {}", e));
                    return false;
                }
            };

            if !exe_path.exists() {
                app.add_output(&format!("Executable not found: {}", exe_path.display()));
                app.add_output("Hot reload requires the binary to exist on disk.");
                app.add_output("If running via 'cargo run', try running the compiled binary directly.");
                return false;
            }

            // Check if there are any TLS connections that will be lost
            let tls_worlds: Vec<_> = app
                .worlds
                .iter()
                .filter(|w| w.connected && w.is_tls)
                .map(|w| w.name.clone())
                .collect();

            if !tls_worlds.is_empty() {
                app.add_output(&format!(
                    "Warning: TLS connections will be closed: {}",
                    tls_worlds.join(", ")
                ));
                app.add_output("These connections will need to be re-established after reload.");
            }

            app.add_output(&format!("Performing hot reload from: {}", exe_path.display()));

            // Disable raw mode before exec (the new process will re-enable it)
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::terminal::LeaveAlternateScreen
            );

            match exec_reload(app) {
                Ok(()) => {
                    // This should never be reached - exec replaces the process
                    unreachable!();
                }
                Err(e) => {
                    // Restore terminal state if exec failed
                    let _ = crossterm::terminal::enable_raw_mode();
                    let _ = crossterm::execute!(
                        std::io::stdout(),
                        crossterm::terminal::EnterAlternateScreen,
                        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                        crossterm::cursor::MoveTo(0, 0)
                    );
                    app.add_output(&format!("Hot reload failed: {}", e));
                    app.add_output(&format!("Executable path: {}", exe_path.display()));
                }
            }
        }
        Command::Gag { pattern } => {
            // TODO: Implement gag patterns
            app.add_output(&format!("Gag pattern set: {}", pattern));
        }
        Command::ActionCommand { name, args: _ } => {
            // Check if this is an action command (/name)
            let action_found = app.settings.actions.iter()
                .find(|a| a.name.eq_ignore_ascii_case(&name))
                .cloned();

            if let Some(action) = action_found {
                // Execute the action's commands
                let commands = split_action_commands(&action.command);
                if let Some(tx) = &app.current_world().command_tx {
                    for cmd_str in commands {
                        // Skip /gag commands when invoked manually
                        if cmd_str.eq_ignore_ascii_case("/gag") || cmd_str.to_lowercase().starts_with("/gag ") {
                            continue;
                        }
                        let _ = tx.try_send(WriteCommand::Text(cmd_str));
                    }
                    app.current_world_mut().last_send_time = Some(std::time::Instant::now());
                } else {
                    app.add_output("Not connected. Cannot execute action.");
                }
            } else {
                app.add_output(&format!("Unknown action: /{}", name));
            }
        }
        Command::NotACommand { text } => {
            // Not a command - send to MUD as regular input
            if let Some(tx) = &app.current_world().command_tx {
                let _ = tx.try_send(WriteCommand::Text(text));
                app.current_world_mut().last_send_time = Some(std::time::Instant::now());
            }
        }
        Command::Unknown { cmd } => {
            app.add_output(&format!("Unknown command: {}", cmd));
        }
    }
    false
}

fn ui(f: &mut Frame, app: &mut App) {
    let total_height = f.area().height.max(3);  // Minimum 3 lines for output + separator + input

    // Layout: output area, separator bar (1 line), input area
    let separator_height = 1;
    let input_total_height = app.input_height;
    let output_height = total_height.saturating_sub(separator_height + input_total_height);

    // Store output dimensions for scrolling and more-mode calculations
    // Use max(1) to prevent any division by zero elsewhere
    app.output_height = output_height.max(1);
    app.output_width = f.area().width.max(1);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(output_height),
            Constraint::Length(separator_height),
            Constraint::Length(input_total_height),
        ])
        .split(f.area());

    let output_area = chunks[0];
    let separator_area = chunks[1];
    let input_area = chunks[2];

    // Update input dimensions
    app.input.set_dimensions(input_area.width, app.input_height);

    // Render output area
    render_output_area(f, app, output_area);

    // Render separator bar
    render_separator_bar(f, app, separator_area);

    // Render input area
    render_input_area(f, app, input_area);

    // Render popups if visible (confirm dialog last so it's on top)
    render_settings_popup(f, app);
    render_web_popup(f, app);
    render_world_selector_popup(f, app);
    render_confirm_dialog(f, app);
    render_worlds_popup(f, app);
    render_filter_popup(f, app);
    render_help_popup(f, app);
    render_actions_popup(f, app);
}

/// Render output area using raw crossterm (bypasses ratatui's buggy rendering)
/// Returns early if splash screen or popup is visible (let ratatui handle those)
fn render_output_crossterm(app: &App) {
    use std::io::Write;
    use crossterm::{cursor, style::Print, QueueableCommand};

    // Skip if showing splash screen or any popup is visible
    let any_popup_visible = app.settings_popup.visible
        || app.world_selector.visible
        || app.confirm_dialog.visible
        || app.worlds_popup.visible
        || app.filter_popup.visible
        || app.help_popup.visible
        || app.actions_popup.visible
        || app.web_popup.visible;
    if app.current_world().showing_splash || any_popup_visible {
        return;
    }

    let mut stdout = std::io::stdout();
    let world = app.current_world();
    let visible_height = (app.output_height as usize).max(1);
    let term_width = (app.output_width as usize).max(1);

    // Calculate visible width of a string (excluding ANSI escape sequences)
    fn visible_width(s: &str) -> usize {
        let mut width = 0;
        let mut in_escape = false;
        for c in s.chars() {
            if c == '\x1b' {
                in_escape = true;
            } else if in_escape {
                if c.is_alphabetic() || c == '~' {
                    in_escape = false;
                }
            } else {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            }
        }
        width
    }

    // Wrap a line with ANSI codes by visible width
    fn wrap_ansi_line(line: &str, max_width: usize) -> Vec<String> {
        if max_width == 0 {
            return vec![line.to_string()];
        }

        let mut result = Vec::new();
        let mut current_line = String::new();
        let mut current_width = 0;
        let mut in_escape = false;
        let mut escape_seq = String::new();
        let mut active_codes: Vec<String> = Vec::new();

        for c in line.chars() {
            if c == '\x1b' {
                in_escape = true;
                escape_seq.push(c);
            } else if in_escape {
                escape_seq.push(c);
                if c.is_alphabetic() || c == '~' {
                    in_escape = false;
                    current_line.push_str(&escape_seq);
                    if c == 'm' {
                        if escape_seq == "\x1b[0m" || escape_seq == "\x1b[m" {
                            active_codes.clear();
                        } else {
                            active_codes.push(escape_seq.clone());
                        }
                    }
                    escape_seq.clear();
                }
            } else {
                let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                if current_width + char_width > max_width && current_width > 0 {
                    current_line.push_str("\x1b[0m");
                    result.push(current_line);
                    current_line = active_codes.join("");
                    current_width = 0;
                }
                current_line.push(c);
                current_width += char_width;
            }
        }

        if !current_line.is_empty() || result.is_empty() {
            result.push(current_line);
        }

        result
    }

    // Collect wrapped lines centered around scroll_offset to fill the screen
    let mut visual_lines: Vec<String> = Vec::new();
    let mut first_line_idx: usize = 0;

    if !world.output_lines.is_empty() {
        let end_line = world.scroll_offset.min(world.output_lines.len().saturating_sub(1));
        let show_tags = app.show_tags;

        let expand_and_wrap = |line: &str| -> Vec<String> {
            // Skip visually empty lines (only ANSI codes/whitespace)
            if is_visually_empty(line) {
                return Vec::new();
            }
            let processed = if show_tags {
                line.to_string()
            } else {
                strip_mud_tag(line)
            };
            let expanded = processed.replace('\t', "        ");
            wrap_ansi_line(&expanded, term_width)
        };

        first_line_idx = end_line;
        for line_idx in (0..=end_line).rev() {
            first_line_idx = line_idx;
            let line = &world.output_lines[line_idx];
            let wrapped = expand_and_wrap(line);

            for w in wrapped.into_iter().rev() {
                visual_lines.insert(0, w);
            }

            if visual_lines.len() >= visible_height {
                break;
            }
        }

        if visual_lines.len() < visible_height && first_line_idx == 0 {
            for line_idx in (end_line + 1)..world.output_lines.len() {
                let line = &world.output_lines[line_idx];
                let wrapped = expand_and_wrap(line);

                for w in wrapped {
                    visual_lines.push(w);
                }

                if visual_lines.len() >= visible_height {
                    break;
                }
            }
        }
    }

    let lines_to_show: &[String] = if first_line_idx == 0 && visual_lines.len() <= visible_height {
        &visual_lines[..visual_lines.len().min(visible_height)]
    } else {
        let display_start = visual_lines.len().saturating_sub(visible_height);
        &visual_lines[display_start..]
    };

    for (row_idx, wrapped) in lines_to_show.iter().enumerate() {
        let _ = stdout.queue(cursor::MoveTo(0, row_idx as u16));
        let _ = stdout.queue(Print(wrapped));

        let line_visible_width = visible_width(wrapped);
        if line_visible_width < term_width {
            let padding = " ".repeat(term_width - line_visible_width);
            let _ = stdout.queue(Print(padding));
        }

        let _ = stdout.queue(Print("\x1b[0m"));
    }

    for row_idx in lines_to_show.len()..visible_height {
        let _ = stdout.queue(cursor::MoveTo(0, row_idx as u16));
        let spaces = " ".repeat(term_width);
        let _ = stdout.queue(Print(spaces));
    }

    // Restore cursor position for the input area
    let separator_height = 1u16;
    let input_area_y = app.output_height + separator_height;
    let prompt = &app.current_world().prompt;
    let prompt_visible_len = visible_width(prompt);
    // Use max(1) to prevent division by zero
    let input_width = (app.output_width as usize).max(1);
    let chars_before_cursor = app.input.buffer[..app.input.cursor_position].chars().count();

    let effective_chars = if app.input.viewport_start_line == 0 {
        chars_before_cursor + prompt_visible_len
    } else {
        chars_before_cursor
    };

    let cursor_line = effective_chars / input_width;
    let cursor_col = effective_chars % input_width;
    let cursor_y = input_area_y + (cursor_line as u16).saturating_sub(app.input.viewport_start_line as u16);
    let cursor_x = cursor_col as u16;

    let _ = stdout.queue(cursor::MoveTo(cursor_x, cursor_y));
    let _ = stdout.flush();
}

fn render_output_area(f: &mut Frame, app: &App, area: Rect) {
    let world = app.current_world();
    let visible_height = area.height as usize;
    let area_width = area.width as usize;

    // Clear the output area first
    f.render_widget(ratatui::widgets::Clear, area);

    // Check if showing splash screen - ratatui handles splash rendering
    if world.showing_splash {
        let output_text = render_splash_centered(world, visible_height, area_width);
        let output_paragraph = Paragraph::new(output_text);
        f.render_widget(output_paragraph, area);
        return;
    }

    // Check if any popup is visible - ratatui handles output when popups are shown
    let any_popup_visible = app.settings_popup.visible
        || app.world_selector.visible
        || app.confirm_dialog.visible
        || app.worlds_popup.visible
        || app.filter_popup.visible
        || app.help_popup.visible
        || app.actions_popup.visible
        || app.web_popup.visible;

    // If no popup is visible, raw crossterm will handle output rendering
    // (it provides better ANSI color handling)
    // Just clear the area and return - crossterm will fill it in
    if !any_popup_visible {
        return;
    }

    // Popup is visible - render output with ratatui (crossterm is skipped when popups are shown)
    // First, fill the entire output area with background to cover any crossterm remnants
    let theme = app.settings.theme;
    let background = ratatui::widgets::Block::default().style(Style::default().bg(theme.bg()));
    f.render_widget(background, area);

    // Get lines to display
    let end = world.scroll_offset.saturating_add(1).min(world.output_lines.len());
    let start = end.saturating_sub(visible_height);

    // Build Text with proper ANSI color parsing
    let mut lines: Vec<Line<'_>> = Vec::with_capacity(end - start);

    for line in world.output_lines.iter().skip(start).take(end - start) {
        // For visually empty lines, render as blank line (don't skip - that leaves gaps)
        if is_visually_empty(line) {
            lines.push(Line::from(""));
            continue;
        }

        // Strip MUD tags if show_tags is disabled
        let display_line = if app.show_tags {
            line.clone()
        } else {
            strip_mud_tag(line)
        };

        // Parse ANSI codes and convert to ratatui spans
        match ansi_to_tui::IntoText::into_text(&display_line) {
            Ok(text) => {
                for l in text.lines {
                    lines.push(l);
                }
            }
            Err(_) => {
                lines.push(Line::raw(display_line));
            }
        }
    }

    let output_text = Text::from(lines);
    let output_paragraph = Paragraph::new(output_text).style(Style::default().bg(theme.bg()));
    f.render_widget(output_paragraph, area);
}

fn render_splash_centered<'a>(world: &World, visible_height: usize, area_width: usize) -> Text<'a> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Calculate visible width of a string (excluding ANSI escape sequences)
    fn visible_width(s: &str) -> usize {
        let mut width = 0;
        let mut in_escape = false;
        for c in s.chars() {
            if c == '\x1b' {
                in_escape = true;
            } else if in_escape {
                if c == 'm' {
                    in_escape = false;
                }
            } else {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            }
        }
        width
    }

    // Calculate vertical centering - how many blank lines to add at top
    let content_height = world.output_lines.len();
    let vertical_padding = if visible_height > content_height {
        (visible_height - content_height) / 2
    } else {
        0
    };

    // Add vertical padding
    for _ in 0..vertical_padding {
        lines.push(Line::from(""));
    }

    // Process and center each line
    for line in &world.output_lines {
        let line_width = visible_width(line);
        let padding = if area_width > line_width {
            (area_width - line_width) / 2
        } else {
            0
        };

        // Create padded line
        let padded = format!("{:width$}{}", "", line, width = padding);

        // Parse ANSI codes and convert to ratatui spans
        match ansi_to_tui::IntoText::into_text(&padded) {
            Ok(text) => {
                for l in text.lines {
                    lines.push(l);
                }
            }
            Err(_) => {
                lines.push(Line::raw(padded));
            }
        }
    }

    Text::from(lines)
}

fn format_more_count(count: usize) -> String {
    if count <= 9999 {
        format!("{:>4}", count)
    } else if count < 100_000 {
        // 10K, 20K, etc.
        format!("{:>3}K", count / 1000)
    } else if count < 1_000_000 {
        // 100K, 200K, etc. - but only 4 chars, so use 99K+ for anything above
        format!("{:>3}K", (count / 1000).min(999))
    } else {
        "Alot".to_string()
    }
}

fn render_separator_bar(f: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let world = app.current_world();
    let theme = app.settings.theme;

    // Build bar components
    let time_str = get_current_time_12hr();

    // Status indicator - always reserve space for "More: XXXX" or "Hist: XXXX" (11 chars)
    // Priority: More (when paused) > Hist (when scrolled back) > underscores
    const STATUS_INDICATOR_LEN: usize = 11;
    let (status_str, status_active) = if world.paused && !world.pending_lines.is_empty() {
        // Show More indicator when paused with pending lines
        (format!("More: {}", format_more_count(world.pending_lines.len())), true)
    } else if !world.is_at_bottom() {
        // Show History indicator when scrolled back
        let lines_back = world.lines_from_bottom();
        (format!("Hist: {}", format_more_count(lines_back)), true)
    } else {
        // Fill with underscores when nothing to show
        ("_".repeat(STATUS_INDICATOR_LEN), false)
    };

    // World name
    let world_display = world.name.clone();

    // Tag indicator (only shown when F2 toggled to show tags)
    let tag_indicator = if app.show_tags { " [tag]" } else { "" };

    // Activity indicator - positioned at column 24
    const ACTIVITY_POSITION: usize = 24;
    let activity_count = app.activity_count();

    // Determine activity string based on available space
    // Full format: "(Activity: X)", Short format: "(Act X)"
    let activity_str = if activity_count > 0 {
        let full_format = format!("(Activity: {})", activity_count);
        let short_format = format!("(Act {})", activity_count);
        // Use short format if screen is narrow (less than 60 chars)
        if width < 60 {
            short_format
        } else {
            full_format
        }
    } else {
        String::new()
    };

    // Time on the right (no space before it, underscores fill to it)
    let time_display = time_str.clone();

    // Create styled spans
    let mut spans = Vec::new();

    // Status indicator on the left (black on red if active, dim underscores if not)
    spans.push(Span::styled(
        status_str.clone(),
        if status_active {
            Style::default().fg(Color::Black).bg(theme.fg_error())
        } else {
            Style::default().fg(theme.fg_dim())
        },
    ));

    // World name
    spans.push(Span::styled(
        world_display.clone(),
        Style::default().fg(theme.fg()),
    ));

    // Tag indicator (cyan, like prompt)
    if !tag_indicator.is_empty() {
        spans.push(Span::styled(
            tag_indicator.to_string(),
            Style::default().fg(theme.fg_accent()),
        ));
    }

    // Calculate current position after status, world name, and tag indicator
    let current_pos = status_str.len() + world_display.len() + tag_indicator.len();

    // Add underscores to reach position 24 (or as close as possible)
    if !activity_str.is_empty() && current_pos < ACTIVITY_POSITION {
        let padding = ACTIVITY_POSITION - current_pos;
        spans.push(Span::styled(
            "_".repeat(padding),
            Style::default().fg(theme.fg_dim()),
        ));
    }

    // Activity indicator (highlight color)
    if !activity_str.is_empty() {
        spans.push(Span::styled(
            activity_str.clone(),
            Style::default()
                .fg(theme.fg_highlight())
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Calculate underscore padding - fill between content and time
    let used_len = if activity_str.is_empty() {
        status_str.len() + world_display.len() + tag_indicator.len()
    } else {
        ACTIVITY_POSITION.max(current_pos) + activity_str.len()
    };
    let underscore_count = width.saturating_sub(used_len + time_display.len());

    spans.push(Span::styled(
        "_".repeat(underscore_count),
        Style::default().fg(theme.fg_dim()),
    ));

    // Time on the right (no spaces around it)
    spans.push(Span::styled(time_display, Style::default().fg(theme.fg())));

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(theme.bg()));

    f.render_widget(paragraph, area);
}

fn render_input_area(f: &mut Frame, app: &App, area: Rect) {
    // Get prompt for current world only
    let prompt = &app.current_world().prompt;
    // Use visible length (without ANSI codes) for cursor positioning
    let prompt_len = strip_ansi_codes(prompt).chars().count();

    let input_text = render_input(app, area.width as usize, prompt);

    let input_paragraph = Paragraph::new(input_text);
    f.render_widget(input_paragraph, area);

    // Set cursor position (offset by prompt length on first line)
    let cursor_line = app.input.cursor_line();
    let viewport_line = cursor_line.saturating_sub(app.input.viewport_start_line);

    if viewport_line < app.input_height as usize {
        let inner_width = area.width.max(1) as usize;
        // Use character count for cursor column, not byte index
        let chars_before_cursor = app.input.buffer[..app.input.cursor_position].chars().count();
        // Add prompt length offset if on first display line and viewport starts at 0
        let effective_chars = if app.input.viewport_start_line == 0 {
            chars_before_cursor + prompt_len
        } else {
            chars_before_cursor
        };
        let cursor_col = effective_chars % inner_width;
        let cursor_x = area.x + cursor_col as u16;
        // Account for potential line wrap due to prompt
        let extra_lines = if app.input.viewport_start_line == 0 {
            (chars_before_cursor + prompt_len) / inner_width
        } else {
            chars_before_cursor / inner_width
        };
        let cursor_y = area.y + (viewport_line + extra_lines - cursor_line) as u16;
        f.set_cursor_position((cursor_x, cursor_y.min(area.y + area.height - 1)));
    }
}

fn render_input(app: &App, width: usize, prompt: &str) -> Text<'static> {
    let misspelled = app.find_misspelled_words();
    let chars: Vec<char> = app.input.buffer.chars().collect();

    // Calculate visible prompt length (without ANSI codes)
    let prompt_visible_len = strip_ansi_codes(prompt).chars().count();

    if width == 0 {
        return Text::default();
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    // If we're at the start, render the prompt first
    if app.input.viewport_start_line == 0 && !prompt.is_empty() {
        // Check if prompt has ANSI codes
        let has_ansi = prompt.contains("\x1b[");

        if has_ansi {
            // Parse ANSI codes and render with proper styling
            match ansi_to_tui::IntoText::into_text(&prompt) {
                Ok(text) => {
                    // Get all spans from the parsed text
                    for line in text.lines {
                        for span in line.spans {
                            // Add prompt spans - will be combined with first line
                            lines.push(Line::from(vec![span]));
                        }
                    }
                }
                Err(_) => {
                    // Fallback to cyan if parsing fails
                    lines.push(Line::from(Span::styled(
                        prompt.to_string(),
                        Style::default().fg(Color::Cyan),
                    )));
                }
            }
        } else {
            // No ANSI codes, use cyan
            lines.push(Line::from(Span::styled(
                prompt.to_string(),
                Style::default().fg(Color::Cyan),
            )));
        }
    }

    // Build a combined first line if prompt doesn't fill the width
    if app.input.viewport_start_line == 0 && !prompt.is_empty() && !lines.is_empty() {
        let prompt_line_chars = prompt_visible_len % width;
        let remaining_width = if prompt_line_chars == 0 && prompt_visible_len > 0 {
            0
        } else {
            width - prompt_line_chars
        };

        if remaining_width > 0 && !chars.is_empty() {
            // Add user input to the same line as prompt
            let input_chars_on_first_line = remaining_width.min(chars.len());
            let first_input: String = chars[..input_chars_on_first_line].iter().collect();

            // Get the last line (which has the prompt) and append user input
            if let Some(last_line) = lines.last_mut() {
                let mut new_spans = last_line.spans.clone();
                // Check for misspellings in this portion
                let misspelled_in_range: Vec<_> = misspelled
                    .iter()
                    .filter(|(s, e)| *s < input_chars_on_first_line || *e <= input_chars_on_first_line)
                    .cloned()
                    .collect();

                if misspelled_in_range.is_empty() {
                    new_spans.push(Span::raw(first_input));
                } else {
                    // Handle misspellings
                    let mut pos = 0;
                    for (start, end) in &misspelled_in_range {
                        let s = (*start).min(input_chars_on_first_line);
                        let e = (*end).min(input_chars_on_first_line);
                        if pos < s {
                            let text: String = chars[pos..s].iter().collect();
                            new_spans.push(Span::raw(text));
                        }
                        if s < e {
                            let text: String = chars[s..e].iter().collect();
                            new_spans.push(Span::styled(text, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
                        }
                        pos = e;
                    }
                    if pos < input_chars_on_first_line {
                        let text: String = chars[pos..input_chars_on_first_line].iter().collect();
                        new_spans.push(Span::raw(text));
                    }
                }
                *last_line = Line::from(new_spans);
            }

            // Now handle remaining input on subsequent lines
            let mut char_pos = input_chars_on_first_line;
            while char_pos < chars.len() && lines.len() < app.input_height as usize {
                let line_end = (char_pos + width).min(chars.len());
                let mut spans: Vec<Span<'static>> = Vec::new();
                let mut current_pos = char_pos;

                while current_pos < line_end {
                    let in_misspelled = misspelled
                        .iter()
                        .find(|(s, e)| current_pos >= *s && current_pos < *e);

                    if let Some(&(word_start, word_end)) = in_misspelled {
                        if current_pos > char_pos && spans.is_empty() {
                            let before_end = word_start.min(line_end);
                            if before_end > char_pos {
                                let text: String = chars[char_pos..before_end].iter().collect();
                                spans.push(Span::raw(text));
                            }
                        }
                        let mis_start = word_start.max(char_pos);
                        let mis_end = word_end.min(line_end);
                        let text: String = chars[mis_start..mis_end].iter().collect();
                        spans.push(Span::styled(text, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
                        current_pos = mis_end;
                    } else {
                        let next_mis = misspelled
                            .iter()
                            .filter(|(s, _)| *s > current_pos && *s < line_end)
                            .map(|(s, _)| *s)
                            .min();
                        let chunk_end = next_mis.unwrap_or(line_end);
                        let text: String = chars[current_pos..chunk_end].iter().collect();
                        spans.push(Span::raw(text));
                        current_pos = chunk_end;
                    }
                }

                if spans.is_empty() {
                    let text: String = chars[char_pos..line_end].iter().collect();
                    lines.push(Line::from(text));
                } else {
                    lines.push(Line::from(spans));
                }
                char_pos = line_end;
            }
        } else if remaining_width == 0 {
            // Prompt fills the line exactly, user input starts on next line
            let mut char_pos = 0;
            while char_pos < chars.len() && lines.len() < app.input_height as usize {
                let line_end = (char_pos + width).min(chars.len());
                let text: String = chars[char_pos..line_end].iter().collect();
                lines.push(Line::from(text));
                char_pos = line_end;
            }
        }
    } else if app.input.viewport_start_line == 0 && prompt.is_empty() {
        // No prompt, just render user input
        let mut char_pos = 0;
        while char_pos < chars.len() && lines.len() < app.input_height as usize {
            let line_end = (char_pos + width).min(chars.len());
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut current_pos = char_pos;

            while current_pos < line_end {
                let in_misspelled = misspelled
                    .iter()
                    .find(|(s, e)| current_pos >= *s && current_pos < *e);

                if let Some(&(word_start, word_end)) = in_misspelled {
                    if current_pos > char_pos && spans.is_empty() {
                        let before_end = word_start.min(line_end);
                        if before_end > char_pos {
                            let text: String = chars[char_pos..before_end].iter().collect();
                            spans.push(Span::raw(text));
                        }
                    }
                    let mis_start = word_start.max(char_pos);
                    let mis_end = word_end.min(line_end);
                    let text: String = chars[mis_start..mis_end].iter().collect();
                    spans.push(Span::styled(text, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
                    current_pos = mis_end;
                } else {
                    let next_mis = misspelled
                        .iter()
                        .filter(|(s, _)| *s > current_pos && *s < line_end)
                        .map(|(s, _)| *s)
                        .min();
                    let chunk_end = next_mis.unwrap_or(line_end);
                    let text: String = chars[current_pos..chunk_end].iter().collect();
                    spans.push(Span::raw(text));
                    current_pos = chunk_end;
                }
            }

            if spans.is_empty() {
                let text: String = chars[char_pos..line_end].iter().collect();
                lines.push(Line::from(text));
            } else {
                lines.push(Line::from(spans));
            }
            char_pos = line_end;
        }
    } else {
        // Scrolled down, don't show prompt
        let start_char = app.input.viewport_start_line * width;
        let mut char_pos = start_char;
        while char_pos < chars.len() && lines.len() < app.input_height as usize {
            let line_end = (char_pos + width).min(chars.len());
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut current_pos = char_pos;

            while current_pos < line_end {
                let in_misspelled = misspelled
                    .iter()
                    .find(|(s, e)| current_pos >= *s && current_pos < *e);

                if let Some(&(word_start, word_end)) = in_misspelled {
                    if current_pos > char_pos && spans.is_empty() {
                        let before_end = word_start.min(line_end);
                        if before_end > char_pos {
                            let text: String = chars[char_pos..before_end].iter().collect();
                            spans.push(Span::raw(text));
                        }
                    }
                    let mis_start = word_start.max(char_pos);
                    let mis_end = word_end.min(line_end);
                    let text: String = chars[mis_start..mis_end].iter().collect();
                    spans.push(Span::styled(text, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));
                    current_pos = mis_end;
                } else {
                    let next_mis = misspelled
                        .iter()
                        .filter(|(s, _)| *s > current_pos && *s < line_end)
                        .map(|(s, _)| *s)
                        .min();
                    let chunk_end = next_mis.unwrap_or(line_end);
                    let text: String = chars[current_pos..chunk_end].iter().collect();
                    spans.push(Span::raw(text));
                    current_pos = chunk_end;
                }
            }

            if spans.is_empty() {
                let text: String = chars[char_pos..line_end].iter().collect();
                lines.push(Line::from(text));
            } else {
                lines.push(Line::from(spans));
            }
            char_pos = line_end;
        }
    }

    // Pad remaining lines
    while lines.len() < app.input_height as usize {
        lines.push(Line::from(""));
    }

    Text::from(lines)
}

fn render_settings_popup(f: &mut Frame, app: &App) {
    if !app.settings_popup.visible {
        return;
    }

    let area = f.area();
    let popup = &app.settings_popup;
    let theme = app.settings.theme;

    // Helper to get field style
    let field_style = |field: SettingsField| -> Style {
        if field == popup.selected_field {
            if popup.editing {
                Style::default()
                    .fg(theme.fg_success())
                    .add_modifier(Modifier::BOLD)
            } else if field.is_button() {
                // Highlight buttons with background when selected
                Style::default()
                    .fg(theme.button_selected_fg())
                    .bg(theme.button_selected_bg())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.fg_highlight())
                    .add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(theme.fg())
        }
    };

    // Label widths for alignment (longest label + 1 space)
    // Global: "World Switching:" = 16, so use 17
    // World: "Auto login:" = 11, so use 12
    let global_label_width = 17;
    let world_label_width = 12;

    // Maximum visible width for text field values (for scrolling)
    let max_field_display_width = 35usize;

    // Helper to render a text field with horizontal scrolling support
    let render_text_field = |label: &str, value: &str, field: SettingsField, width: usize| -> Line<'static> {
        let style = field_style(field);
        let display_value = if field == popup.selected_field && popup.editing {
            // Show edit buffer with cursor, applying scroll offset
            let buf = &popup.edit_buffer;
            let cursor = popup.edit_cursor;
            let scroll = popup.edit_scroll_offset;

            // Calculate visible portion
            let visible_width = max_field_display_width.saturating_sub(2); // Leave room for scroll indicators
            let buf_chars: Vec<char> = buf.chars().collect();
            let buf_len = buf_chars.len();

            // Build visible text with cursor
            let start = scroll.min(buf_len);
            let end = (scroll + visible_width).min(buf_len);

            let mut display = String::new();

            // Left scroll indicator
            if scroll > 0 {
                display.push('<');
            } else {
                display.push(' ');
            }

            // Visible text with cursor
            for (i, &c) in buf_chars.iter().enumerate() {
                if i == cursor {
                    display.push('|');
                }
                if i >= start && i < end {
                    display.push(c);
                }
            }
            // Cursor at end
            if cursor >= buf_len && cursor >= start {
                display.push('|');
            }

            // Right scroll indicator
            if end < buf_len {
                display.push('>');
            }

            display
        } else {
            // Not editing - show truncated value if needed
            let val_chars: Vec<char> = value.chars().collect();
            if val_chars.len() > max_field_display_width {
                format!("{}...", val_chars[..max_field_display_width - 3].iter().collect::<String>())
            } else {
                value.to_string()
            }
        };
        Line::from(vec![
            Span::styled(format!("  {:<width$}", label), style),
            Span::styled(format!("[{}]", display_value), style),
        ])
    };

    // Helper to render a toggle field
    let render_toggle_field = |label: &str, value: bool, field: SettingsField, width: usize| -> Line<'static> {
        let style = field_style(field);
        let value_str = if value { "on" } else { "off" };
        Line::from(vec![
            Span::styled(format!("  {:<width$}", label), style),
            Span::styled(format!("[{}]", value_str), style),
        ])
    };

    // Helper to render input height field
    let render_height_field = |label: &str, value: u16, field: SettingsField, width: usize| -> Line<'static> {
        let style = field_style(field);
        Line::from(vec![
            Span::styled(format!("  {:<width$}", label), style),
            Span::styled(format!("[{}]", value), style),
        ])
    };

    // Helper to render a button
    let render_button = |label: &str, field: SettingsField| -> Span<'static> {
        let style = field_style(field);
        Span::styled(format!("[ {} ]", label), style)
    };

    let (lines, title) = if popup.setup_mode {
        // Setup mode: only global settings
        let w = global_label_width;
        let lines = vec![
            Line::from(""),
            render_toggle_field("More mode:", popup.temp_more_mode, SettingsField::MoreMode, w),
            render_toggle_field("Spell check:", popup.temp_spell_check, SettingsField::SpellCheck, w),
            Line::from(vec![
                Span::styled(
                    format!("  {:<w$}", "World Switching:"),
                    field_style(SettingsField::WorldSwitching),
                ),
                Span::styled(
                    format!("[{}]", popup.temp_world_switch_mode.name()),
                    field_style(SettingsField::WorldSwitching),
                ),
            ]),
            render_toggle_field("Debug:", popup.temp_debug_enabled, SettingsField::Debug, w),
            render_toggle_field("Show tags:", popup.temp_show_tags, SettingsField::ShowTags, w),
            render_height_field("Input height:", popup.temp_input_height, SettingsField::InputHeight, w),
            Line::from(vec![
                Span::styled(
                    format!("  {:<w$}", "Console Theme:"),
                    field_style(SettingsField::Theme),
                ),
                Span::styled(
                    format!("[{}]", popup.temp_theme.name()),
                    field_style(SettingsField::Theme),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("  {:<w$}", "GUI Theme:"),
                    field_style(SettingsField::GuiTheme),
                ),
                Span::styled(
                    format!("[{}]", popup.temp_gui_theme.name()),
                    field_style(SettingsField::GuiTheme),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                render_button("Save", SettingsField::SaveSetup),
                Span::raw("  "),
                render_button("Cancel", SettingsField::CancelSetup),
                Span::raw(" "),
            ]).alignment(Alignment::Right),
        ];
        (lines, " Global Settings ")
    } else {
        // World mode: all settings
        let w = world_label_width;
        let connect_text = "Connect";

        let mut lines = vec![
            Line::from(""),
            render_text_field("World:", &popup.temp_world_name, SettingsField::WorldName, w),
            render_text_field("Hostname:", &popup.temp_hostname, SettingsField::Hostname, w),
            render_text_field("Port:", &popup.temp_port, SettingsField::Port, w),
            render_text_field("User:", &popup.temp_user, SettingsField::User, w),
            render_text_field("Password:", &popup.temp_password, SettingsField::Password, w),
            render_toggle_field("Use SSL:", popup.temp_use_ssl, SettingsField::UseSsl, w),
            render_text_field("Log file:", &popup.temp_log_file, SettingsField::LogFile, w),
            Line::from(vec![
                Span::styled(
                    format!("  {:<w$}", "Encoding:"),
                    field_style(SettingsField::Encoding),
                ),
                Span::styled(
                    format!("[{}]", popup.temp_encoding.name()),
                    field_style(SettingsField::Encoding),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("  {:<w$}", "Auto login:"),
                    field_style(SettingsField::AutoConnect),
                ),
                Span::styled(
                    format!("[{}]", popup.temp_auto_connect_type.name()),
                    field_style(SettingsField::AutoConnect),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("  {:<w$}", "Keep-Alive:"),
                    field_style(SettingsField::KeepAlive),
                ),
                Span::styled(
                    format!("[{}]", popup.temp_keep_alive_type.name()),
                    field_style(SettingsField::KeepAlive),
                ),
            ]),
        ];
        // Conditionally add Keep-Alive CMD field only when Custom is selected
        if popup.temp_keep_alive_type == KeepAliveType::Custom {
            lines.push(render_text_field("Keep-Alive CMD:", &popup.temp_keep_alive_cmd, SettingsField::KeepAliveCmd, w));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            render_button("Save", SettingsField::SaveWorld),
            Span::raw(" "),
            render_button("Cancel", SettingsField::CancelWorld),
            Span::raw(" "),
            render_button("Delete", SettingsField::DeleteWorld),
            Span::raw(" "),
            render_button(connect_text, SettingsField::Connect),
            Span::raw(" "),
        ]).alignment(Alignment::Right));
        (lines, " World Settings ")
    };

    // Calculate dynamic width based on content
    // Find the maximum line width by summing span widths
    let max_content_width = lines.iter().map(|line| {
        line.spans.iter().map(|span| span.content.chars().count()).sum::<usize>()
    }).max().unwrap_or(20);

    // Add borders (2) and some padding (2)
    let popup_width = ((max_content_width + 4) as u16).min(area.width.saturating_sub(2));

    // Calculate required height for all content + borders
    let required_height = (lines.len() + 2) as u16; // +2 for borders
    // Use full terminal height minus 1 for margin if content doesn't fit
    let max_available_height = area.height.saturating_sub(1);
    let popup_height = required_height.min(max_available_height);

    let x = area.width.saturating_sub(popup_width) / 2;
    let y = if popup_height >= area.height {
        0 // Start at top if popup fills screen
    } else {
        area.height.saturating_sub(popup_height) / 2
    };

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let popup_block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

fn render_web_popup(f: &mut Frame, app: &App) {
    if !app.web_popup.visible {
        return;
    }

    let area = f.area();
    let popup = &app.web_popup;
    let theme = app.settings.theme;

    // Helper to get field style
    let field_style = |field: WebField| -> Style {
        if field == popup.selected_field {
            if popup.editing {
                Style::default()
                    .fg(theme.fg_success())
                    .add_modifier(Modifier::BOLD)
            } else if field.is_button() {
                Style::default()
                    .fg(theme.button_selected_fg())
                    .bg(theme.button_selected_bg())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.fg_highlight())
                    .add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(theme.fg())
        }
    };

    let label_width = 17; // "WS Nonsecure:" = 13, add padding
    let max_field_display_width = 35usize;

    // Helper to render a text field with horizontal scrolling support
    let render_text_field = |label: &str, value: &str, field: WebField, width: usize| -> Line<'static> {
        let style = field_style(field);
        let display_value = if field == popup.selected_field && popup.editing {
            let buf = &popup.edit_buffer;
            let cursor = popup.edit_cursor;
            let scroll = popup.edit_scroll_offset;
            let visible_width = max_field_display_width.saturating_sub(2);
            let buf_chars: Vec<char> = buf.chars().collect();
            let buf_len = buf_chars.len();
            let start = scroll.min(buf_len);
            let end = (scroll + visible_width).min(buf_len);
            let mut display = String::new();
            if scroll > 0 { display.push('<'); } else { display.push(' '); }
            for (i, &c) in buf_chars.iter().enumerate() {
                if i == cursor { display.push('|'); }
                if i >= start && i < end { display.push(c); }
            }
            if cursor >= buf_len && cursor >= start { display.push('|'); }
            if end < buf_len { display.push('>'); }
            display
        } else {
            let val_chars: Vec<char> = value.chars().collect();
            if val_chars.len() > max_field_display_width {
                format!("{}...", val_chars[..max_field_display_width - 3].iter().collect::<String>())
            } else {
                value.to_string()
            }
        };
        Line::from(vec![
            Span::styled(format!("  {:<width$}", label), style),
            Span::styled(format!("[{}]", display_value), style),
        ])
    };

    // Helper to render a toggle field
    let render_toggle_field = |label: &str, value: bool, field: WebField, width: usize| -> Line<'static> {
        let style = field_style(field);
        let value_str = if value { "on" } else { "off" };
        Line::from(vec![
            Span::styled(format!("  {:<width$}", label), style),
            Span::styled(format!("[{}]", value_str), style),
        ])
    };

    // Helper to render a button
    let render_button = |label: &str, field: WebField| -> Span<'static> {
        let style = field_style(field);
        Span::styled(format!("[ {} ]", label), style)
    };

    let w = label_width;

    // Build dynamic field labels based on protocol
    let http_label = if popup.temp_web_secure { "HTTPS enabled:" } else { "HTTP enabled:" };
    let http_port_label = if popup.temp_web_secure { "HTTPS port:" } else { "HTTP port:" };
    let ws_label = if popup.temp_web_secure { "WSS enabled:" } else { "WS enabled:" };
    let ws_port_label = if popup.temp_web_secure { "WSS port:" } else { "WS port:" };
    let protocol_value = if popup.temp_web_secure { "Secure" } else { "Non-Secure" };

    // Helper to render protocol toggle (shows as option with current value)
    let render_protocol_field = |label: &str, value: &str, field: WebField, width: usize| -> Line<'static> {
        let style = field_style(field);
        Line::from(vec![
            Span::styled(format!("  {:<width$}", label), style),
            Span::styled(format!("[{}]", value), style),
        ])
    };

    let mut lines = vec![
        Line::from(""),
        render_protocol_field("Protocol:", protocol_value, WebField::Protocol, w),
        Line::from(""),
        Line::from(Span::styled("  -- Web Interface --", Style::default().fg(theme.fg_accent()))),
        render_toggle_field(http_label, popup.temp_http_enabled, WebField::HttpEnabled, w),
        render_text_field(http_port_label, &popup.temp_http_port, WebField::HttpPort, w),
        Line::from(""),
        Line::from(Span::styled("  -- WebSocket Server --", Style::default().fg(theme.fg_accent()))),
        render_toggle_field(ws_label, popup.temp_ws_enabled, WebField::WsEnabled, w),
        render_text_field(ws_port_label, &popup.temp_ws_port, WebField::WsPort, w),
        render_text_field("Password:", &popup.temp_ws_password, WebField::WsPassword, w),
        render_text_field("Allow List:", &popup.temp_ws_allow_list, WebField::WsAllowList, w),
    ];

    // Only show TLS cert/key fields when secure protocol is selected
    if popup.temp_web_secure {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  -- TLS Settings --", Style::default().fg(theme.fg_accent()))));
        lines.push(render_text_field("TLS cert file:", &popup.temp_ws_cert_file, WebField::WsCertFile, w));
        lines.push(render_text_field("TLS key file:", &popup.temp_ws_key_file, WebField::WsKeyFile, w));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        render_button("Save", WebField::SaveWeb),
        Span::raw("  "),
        render_button("Cancel", WebField::CancelWeb),
        Span::raw(" "),
    ]).alignment(Alignment::Right));

    // Calculate dynamic size
    let max_content_width = lines.iter().map(|line| {
        line.spans.iter().map(|span| span.content.chars().count()).sum::<usize>()
    }).max().unwrap_or(20);

    let popup_width = ((max_content_width + 4) as u16).min(area.width.saturating_sub(2));
    let required_height = (lines.len() + 2) as u16;
    let max_available_height = area.height.saturating_sub(1);
    let popup_height = required_height.min(max_available_height);

    let x = area.width.saturating_sub(popup_width) / 2;
    let y = if popup_height >= area.height {
        0
    } else {
        area.height.saturating_sub(popup_height) / 2
    };

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(ratatui::widgets::Clear, popup_area);

    let popup_block = Block::default()
        .title(" Web Settings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

fn render_confirm_dialog(f: &mut Frame, app: &App) {
    if !app.confirm_dialog.visible {
        return;
    }

    let area = f.area();
    let dialog = &app.confirm_dialog;
    let theme = app.settings.theme;

    // Build button styles with background highlight
    let yes_style = if dialog.yes_selected {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };
    let no_style = if !dialog.yes_selected {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(dialog.message.clone(), Style::default().fg(theme.fg()))).alignment(Alignment::Center),
        Line::from(""),
        Line::from(vec![
            Span::styled("[ Yes ]", yes_style),
            Span::raw("  "),
            Span::styled("[ No ]", no_style),
        ]).alignment(Alignment::Center),
    ];

    // Calculate dynamic size based on content
    let message_width = dialog.message.chars().count();
    let buttons_width = 17; // "[ Yes ]  [ No ]"
    let content_width = message_width.max(buttons_width);
    let popup_width = ((content_width + 6) as u16).min(area.width.saturating_sub(4)); // +6 for borders and padding
    let popup_height = ((lines.len() + 2) as u16).min(area.height.saturating_sub(2)); // +2 for borders

    let x = area.width.saturating_sub(popup_width) / 2;
    let y = area.height.saturating_sub(popup_height) / 2;

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let popup_block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.fg_error()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

fn render_worlds_popup(f: &mut Frame, app: &App) {
    if !app.worlds_popup.visible {
        return;
    }

    let area = f.area();
    let popup = &app.worlds_popup;
    let theme = app.settings.theme;

    // Calculate popup size based on content
    let content_width = popup.lines.iter().map(|l| l.len()).max().unwrap_or(20).max(20);
    let popup_width = (content_width + 4).min(area.width as usize - 4) as u16;
    let popup_height = (popup.lines.len() + 4).min(area.height as usize - 2) as u16; // +4 for borders and button

    let x = area.width.saturating_sub(popup_width) / 2;
    let y = area.height.saturating_sub(popup_height) / 2;

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    // Build content lines
    let mut lines: Vec<Line> = popup.lines.iter()
        .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(theme.fg()))))
        .collect();

    // Add blank line and OK button
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "[ OK ]",
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    )).alignment(Alignment::Center));

    let popup_block = Block::default()
        .title(" Connected Worlds ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.fg()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

fn render_filter_popup(f: &mut Frame, app: &App) {
    if !app.filter_popup.visible {
        return;
    }

    let area = f.area();
    let filter = &app.filter_popup;
    let theme = app.settings.theme;

    // Small popup in upper right corner
    let popup_width = 40u16.min(area.width);
    let popup_height = 3u16;

    let x = area.width.saturating_sub(popup_width); // Right edge
    let y = 0; // Top edge, no gap

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    // Show filter text with cursor
    let mut display_text = filter.filter_text.clone();
    display_text.insert(filter.cursor, '|');

    let lines = vec![
        Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(theme.fg_accent())),
            Span::styled(display_text, Style::default().fg(theme.fg())),
        ]),
    ];

    let popup_block = Block::default()
        .title(" Find [Esc to close] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

fn render_help_popup(f: &mut Frame, app: &App) {
    if !app.help_popup.visible {
        return;
    }

    let area = f.area();
    let help = &app.help_popup;
    let theme = app.settings.theme;

    // Centered popup - wider to fit longest help lines
    let popup_width = 60u16.min(area.width.saturating_sub(4));
    let popup_height = 20u16.min(area.height.saturating_sub(4)).max(10);

    let x = area.width.saturating_sub(popup_width) / 2;
    let y = area.height.saturating_sub(popup_height) / 2;

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    // Calculate visible height for content (popup height - 2 borders - 1 blank line - 1 button line)
    let visible_height = (popup_height as usize).saturating_sub(4);

    // Build lines from help content with scrolling
    let mut lines: Vec<Line<'static>> = Vec::new();

    for line in help.lines.iter().skip(help.scroll_offset).take(visible_height) {
        lines.push(Line::from(Span::styled(*line, Style::default().fg(theme.fg()))));
    }

    // Pad if needed
    while lines.len() < visible_height {
        lines.push(Line::from(""));
    }

    // Blank line before button
    lines.push(Line::from(""));

    // OK button (always highlighted since it's the only option)
    let ok_style = Style::default()
        .fg(theme.button_selected_fg())
        .bg(theme.button_selected_bg())
        .add_modifier(Modifier::BOLD);
    lines.push(Line::from(vec![
        Span::styled("[ OK ]", ok_style),
    ]).alignment(Alignment::Center));

    let popup_block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

fn render_world_selector_popup(f: &mut Frame, app: &App) {
    if !app.world_selector.visible {
        return;
    }

    let area = f.area();
    let selector = &app.world_selector;
    let theme = app.settings.theme;

    // Get filtered world indices
    let filtered_indices = selector.filtered_indices(&app.worlds);

    // Calculate dynamic column widths based on actual content
    let name_width = app.worlds.iter()
        .map(|w| w.name.chars().count())
        .max()
        .unwrap_or(5)
        .clamp(5, 20); // Min "World", max 20
    let host_width = app.worlds.iter()
        .map(|w| w.settings.hostname.chars().count())
        .max()
        .unwrap_or(8)
        .clamp(8, 25); // Min "Hostname", max 25
    let port_width = 6; // Fixed for "Port" and typical values
    let user_width = app.worlds.iter()
        .map(|w| w.settings.user.chars().count())
        .max()
        .unwrap_or(4)
        .clamp(4, 15); // Min "User", max 15

    // Calculate total content width: marker(2) + columns + spacing(6) + some padding(4)
    let content_width = 2 + name_width + host_width + port_width + user_width + 6 + 4;
    let buttons_width = 47; // "[ Add ]  [ Edit ]  [ Connect ]  [ Cancel ] "
    let min_content_width = content_width.max(buttons_width);

    // Add borders (2) and apply screen limits
    let popup_width = ((min_content_width + 2) as u16).min(area.width.saturating_sub(4));
    let popup_height = ((filtered_indices.len() + 8) as u16).min(area.height.saturating_sub(4)).clamp(10, 20);

    let x = area.width.saturating_sub(popup_width) / 2;
    let y = area.height.saturating_sub(popup_height) / 2;

    let popup_area = Rect::new(x, y, popup_width, popup_height);

    // Clear the background
    f.render_widget(ratatui::widgets::Clear, popup_area);

    // Recalculate content_width based on actual popup_width
    let content_width = popup_width.saturating_sub(2) as usize;

    // Build lines
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Filter line at top
    let filter_label = "Filter: ";
    let filter_display = if selector.editing_filter {
        let mut buf = selector.filter.clone();
        buf.insert(selector.filter_cursor, '|');
        buf
    } else if selector.filter.is_empty() {
        "(type to filter)".to_string()
    } else {
        selector.filter.clone()
    };
    let filter_style = if selector.editing_filter {
        Style::default().fg(theme.fg_success())
    } else {
        Style::default().fg(theme.fg_dim())
    };
    lines.push(Line::from(vec![
        Span::styled(filter_label, Style::default().fg(theme.fg_accent())),
        Span::styled(filter_display, filter_style),
    ]));
    lines.push(Line::from(""));

    // Recalculate host_width to fill remaining space
    let host_width = content_width.saturating_sub(name_width + port_width + user_width + 8); // 8 for marker and spacing

    lines.push(Line::from(vec![
        Span::styled(
            format!("  {:<name_width$} ", "World"),
            Style::default().fg(theme.fg_accent()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<host_width$} ", "Hostname"),
            Style::default().fg(theme.fg_accent()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<port_width$} ", "Port"),
            Style::default().fg(theme.fg_accent()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<user_width$}", "User"),
            Style::default().fg(theme.fg_accent()).add_modifier(Modifier::BOLD),
        ),
    ]));

    // World list (scrollable area)
    // 7 = borders(2) + filter(1) + blank(1) + header(1) + blank-before-buttons(1) + buttons(1)
    let list_height = popup_height.saturating_sub(7) as usize;

    // Find where selected item is in the filtered list
    let selected_pos = filtered_indices
        .iter()
        .position(|&i| i == selector.selected_index)
        .unwrap_or(0);

    // Calculate scroll offset to keep selected visible
    let scroll_offset = if selected_pos >= list_height {
        selected_pos - list_height + 1
    } else {
        0
    };

    for &world_idx in filtered_indices.iter().skip(scroll_offset).take(list_height) {
        let world = &app.worlds[world_idx];
        let is_selected = world_idx == selector.selected_index;
        let is_current = world_idx == app.current_world_index;

        // Truncate hostname if needed
        let hostname = if world.settings.hostname.len() > host_width {
            format!("{}...", &world.settings.hostname[..host_width.saturating_sub(3)])
        } else {
            world.settings.hostname.clone()
        };

        let name_display = if world.name.len() > name_width {
            format!("{}...", &world.name[..name_width.saturating_sub(3)])
        } else {
            world.name.clone()
        };

        let user_display = if world.settings.user.len() > user_width {
            format!("{}...", &world.settings.user[..user_width.saturating_sub(3)])
        } else {
            world.settings.user.clone()
        };

        let marker = if is_current { "*" } else { " " };

        // Calculate highlighted content width (content_width - 2 for marker - 2 for right margin)
        let highlight_width = content_width.saturating_sub(4);

        // Build the content portion (everything after marker)
        let content = format!(
            "{:<name_width$} {:<host_width$} {:<port_width$} {:<user_width$}",
            name_display, hostname, world.settings.port, user_display
        );

        // Pad or truncate to exact highlight width
        let padded_content = if content.len() < highlight_width {
            format!("{:<width$}", content, width = highlight_width)
        } else {
            content[..highlight_width].to_string()
        };

        if is_selected {
            let marker_style = Style::default().fg(theme.fg_highlight()).add_modifier(Modifier::BOLD);
            let highlight_style = Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD);
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", marker), marker_style),
                Span::styled(padded_content, highlight_style),
            ]));
        } else {
            let style = if world.connected {
                Style::default().fg(theme.fg_success())
            } else {
                Style::default().fg(theme.fg())
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", marker), style),
                Span::styled(padded_content, style),
            ]));
        }
    }

    // Pad remaining lines if list is short (subtract 5 for filter, blank, header, blank-before-buttons, buttons)
    while lines.len() < (popup_height as usize).saturating_sub(5) {
        lines.push(Line::from(""));
    }

    // Blank line before buttons
    lines.push(Line::from(""));

    // Button styles based on focus with background highlight
    let add_style = if selector.focus == WorldSelectorFocus::AddButton {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };
    let edit_style = if selector.focus == WorldSelectorFocus::EditButton {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };
    let connect_style = if selector.focus == WorldSelectorFocus::ConnectButton {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };
    let cancel_style = if selector.focus == WorldSelectorFocus::CancelButton {
        Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg()).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg())
    };

    // Buttons at bottom
    lines.push(Line::from(vec![
        Span::styled("[ Add ]", add_style),
        Span::raw("  "),
        Span::styled("[ Edit ]", edit_style),
        Span::raw("  "),
        Span::styled("[ Connect ]", connect_style),
        Span::raw("  "),
        Span::styled("[ Cancel ]", cancel_style),
        Span::raw(" "),
    ]).alignment(Alignment::Right));

    let popup_block = Block::default()
        .title(" Select World ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.popup_border()))
        .style(Style::default().bg(theme.popup_bg()));

    let popup_text = Paragraph::new(lines).block(popup_block);

    f.render_widget(popup_text, popup_area);
}

fn render_actions_popup(f: &mut Frame, app: &App) {
    if !app.actions_popup.visible {
        return;
    }

    let area = f.area();
    let popup = &app.actions_popup;
    let theme = app.settings.theme;

    // Common styles
    let label_style = Style::default().fg(theme.fg());
    let value_style = Style::default().fg(theme.fg());
    let selected_style = Style::default().fg(theme.button_selected_fg()).bg(theme.button_selected_bg());
    let button_style = Style::default().fg(theme.fg());
    let button_selected_style = Style::default()
        .fg(theme.button_selected_fg())
        .bg(theme.button_selected_bg())
        .add_modifier(Modifier::BOLD);
    let error_style = Style::default().fg(Color::Red);

    match popup.view {
        ActionsView::List => {
            // List view - show actions with Add/Edit/Delete/Cancel buttons
            let max_action_display = popup.actions.iter().map(|a| {
                let display_len = if a.pattern.is_empty() {
                    if a.world.is_empty() {
                        a.name.chars().count() + 3
                    } else {
                        a.name.chars().count() + a.world.chars().count() + 6
                    }
                } else if a.world.is_empty() {
                    a.name.chars().count() + 25
                } else {
                    a.name.chars().count() + a.world.chars().count() + 25
                };
                display_len
            }).max().unwrap_or(30);

            let buttons_width = 48; // "[ Add ]  [ Edit ]  [ Delete ]  [ Cancel ]"
            let content_width = max_action_display.max(buttons_width).max(40);
            let popup_width = ((content_width + 4) as u16).min(area.width.saturating_sub(4));
            let list_height = 8usize;
            let popup_height = ((list_height + 5) as u16).min(area.height.saturating_sub(2));

            let x = area.width.saturating_sub(popup_width) / 2;
            let y = area.height.saturating_sub(popup_height) / 2;
            let popup_area = Rect::new(x, y, popup_width, popup_height);

            f.render_widget(ratatui::widgets::Clear, popup_area);

            let mut lines: Vec<Line<'static>> = Vec::new();
            let inner_width = popup_width.saturating_sub(4) as usize;

            lines.push(Line::from(""));

            let actions_len = popup.actions.len();
            let scroll = popup.scroll_offset;

            for i in 0..list_height {
                let action_idx = scroll + i;
                if action_idx < actions_len {
                    let action = &popup.actions[action_idx];
                    let is_selected = action_idx == popup.selected_index;
                    let marker = if is_selected { ">" } else { " " };
                    let style = if popup.list_field == ActionListField::List && is_selected {
                        selected_style
                    } else if is_selected {
                        Style::default().fg(theme.fg_accent())
                    } else {
                        value_style
                    };

                    let display = if action.pattern.is_empty() {
                        if action.world.is_empty() {
                            format!("{} /{}", marker, action.name)
                        } else {
                            format!("{} /{} ({})", marker, action.name, action.world)
                        }
                    } else if action.world.is_empty() {
                        format!("{} {} [{}]", marker, action.name, truncate_str(&action.pattern, 20))
                    } else {
                        format!("{} {} ({}) [{}]", marker, action.name, action.world, truncate_str(&action.pattern, 15))
                    };

                    lines.push(Line::from(Span::styled(
                        truncate_str(&display, inner_width),
                        style,
                    )));
                } else {
                    lines.push(Line::from(""));
                }
            }

            lines.push(Line::from(""));

            // Buttons row
            let add_style = if popup.list_field == ActionListField::AddButton { button_selected_style } else { button_style };
            let edit_style = if popup.list_field == ActionListField::EditButton { button_selected_style } else { button_style };
            let del_style = if popup.list_field == ActionListField::DeleteButton { button_selected_style } else { button_style };
            let cancel_style = if popup.list_field == ActionListField::CancelButton { button_selected_style } else { button_style };

            lines.push(Line::from(vec![
                Span::styled("[ Add ]", add_style),
                Span::raw("  "),
                Span::styled("[ Edit ]", edit_style),
                Span::raw("  "),
                Span::styled("[ Delete ]", del_style),
                Span::raw("  "),
                Span::styled("[ Cancel ]", cancel_style),
            ]));

            let popup_block = Block::default()
                .title(" Actions ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.popup_border()))
                .style(Style::default().bg(theme.popup_bg()));

            let popup_text = Paragraph::new(lines).block(popup_block);
            f.render_widget(popup_text, popup_area);
        }

        ActionsView::Editor => {
            // Editor view - show name/world/pattern/command with Save/Cancel
            let popup_width = 60u16.min(area.width.saturating_sub(4));
            let popup_height = 14u16.min(area.height.saturating_sub(2));

            let x = area.width.saturating_sub(popup_width) / 2;
            let y = area.height.saturating_sub(popup_height) / 2;
            let popup_area = Rect::new(x, y, popup_width, popup_height);

            f.render_widget(ratatui::widgets::Clear, popup_area);

            let mut lines: Vec<Line<'static>> = Vec::new();
            let inner_width = popup_width.saturating_sub(4) as usize;
            let field_width = inner_width.saturating_sub(12);

            fn format_field_with_cursor(value: &str, cursor: usize, is_current: bool, max_len: usize) -> String {
                if is_current {
                    let before = &value[..cursor.min(value.len())];
                    let after = &value[cursor.min(value.len())..];
                    let cursor_char = if cursor < value.len() { "" } else { "_" };
                    let display = format!("{}{}{}", before, cursor_char, after);
                    truncate_str(&display, max_len).to_string()
                } else {
                    truncate_str(value, max_len).to_string()
                }
            }

            let name_style = if popup.editor_field == ActionEditorField::Name { selected_style } else { value_style };
            let world_style = if popup.editor_field == ActionEditorField::World { selected_style } else { value_style };
            let pattern_style = if popup.editor_field == ActionEditorField::Pattern { selected_style } else { value_style };
            let command_style = if popup.editor_field == ActionEditorField::Command { selected_style } else { value_style };

            lines.push(Line::from(""));

            lines.push(Line::from(vec![
                Span::styled("Name:    ", label_style),
                Span::styled(
                    format_field_with_cursor(&popup.edit_name, popup.cursor_pos, popup.editor_field == ActionEditorField::Name, field_width),
                    name_style,
                ),
            ]));

            lines.push(Line::from(vec![
                Span::styled("World:   ", label_style),
                Span::styled(
                    format_field_with_cursor(&popup.edit_world, popup.cursor_pos, popup.editor_field == ActionEditorField::World, field_width),
                    world_style,
                ),
            ]));

            lines.push(Line::from(vec![
                Span::styled("Pattern: ", label_style),
                Span::styled(
                    format_field_with_cursor(&popup.edit_pattern, popup.cursor_pos, popup.editor_field == ActionEditorField::Pattern, field_width),
                    pattern_style,
                ),
            ]));

            lines.push(Line::from(Span::styled("Command:", label_style)));
            let cmd_display = format_field_with_cursor(&popup.edit_command, popup.cursor_pos, popup.editor_field == ActionEditorField::Command, inner_width.saturating_sub(2));
            lines.push(Line::from(vec![
                Span::styled("  ", label_style),
                Span::styled(cmd_display, command_style),
            ]));

            // Show additional command lines if expanded
            if popup.command_expanded || popup.edit_command.contains(';') {
                let commands: Vec<&str> = popup.edit_command.split(';').collect();
                if commands.len() > 1 {
                    for cmd in commands.iter().skip(1).take(3) {
                        lines.push(Line::from(vec![
                            Span::styled("  ", label_style),
                            Span::styled(format!(";{}", truncate_str(cmd.trim(), inner_width.saturating_sub(3))), value_style),
                        ]));
                    }
                }
            }

            lines.push(Line::from(""));

            // Error message if any
            if let Some(ref error) = popup.error_message {
                lines.push(Line::from(Span::styled(error.clone(), error_style)));
            } else {
                lines.push(Line::from(""));
            }

            // Buttons row
            let save_style = if popup.editor_field == ActionEditorField::SaveButton { button_selected_style } else { button_style };
            let cancel_style = if popup.editor_field == ActionEditorField::CancelButton { button_selected_style } else { button_style };

            lines.push(Line::from(vec![
                Span::styled("[ Save ]", save_style),
                Span::raw("  "),
                Span::styled("[ Cancel ]", cancel_style),
            ]));

            let title = if popup.editing_index.is_some() { " Edit Action " } else { " New Action " };
            let popup_block = Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.popup_border()))
                .style(Style::default().bg(theme.popup_bg()));

            let popup_text = Paragraph::new(lines).block(popup_block);
            f.render_widget(popup_text, popup_area);
        }

        ActionsView::ConfirmDelete => {
            // Confirm delete dialog
            let popup_width = 40u16.min(area.width.saturating_sub(4));
            let popup_height = 6u16.min(area.height.saturating_sub(2));

            let x = area.width.saturating_sub(popup_width) / 2;
            let y = area.height.saturating_sub(popup_height) / 2;
            let popup_area = Rect::new(x, y, popup_width, popup_height);

            f.render_widget(ratatui::widgets::Clear, popup_area);

            let action_name = popup.actions.get(popup.selected_index)
                .map(|a| a.name.as_str())
                .unwrap_or("this action");

            let yes_style = if popup.confirm_selected { button_selected_style } else { button_style };
            let no_style = if !popup.confirm_selected { button_selected_style } else { button_style };

            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("Delete '{}'?", truncate_str(action_name, 20)),
                    label_style,
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[ Yes ]", yes_style),
                    Span::raw("    "),
                    Span::styled("[ No ]", no_style),
                ]),
            ];

            let popup_block = Block::default()
                .title(" Confirm Delete ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.popup_border()))
                .style(Style::default().bg(theme.popup_bg()));

            let popup_text = Paragraph::new(lines).block(popup_block);
            f.render_widget(popup_text, popup_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_char_ascii() {
        let mut input = InputArea::new(3);
        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        assert_eq!(input.buffer, "abc");
        assert_eq!(input.cursor_position, 3);
    }

    #[test]
    fn test_insert_char_emoji() {
        let mut input = InputArea::new(3);
        input.insert_char('😀');
        assert_eq!(input.buffer, "😀");
        assert_eq!(input.cursor_position, 4); // emoji is 4 bytes

        input.insert_char('a');
        assert_eq!(input.buffer, "😀a");
        assert_eq!(input.cursor_position, 5);
    }

    #[test]
    fn test_insert_char_mixed() {
        let mut input = InputArea::new(3);
        input.insert_char('H');
        input.insert_char('i');
        input.insert_char('😀');
        input.insert_char('!');
        assert_eq!(input.buffer, "Hi😀!");
        assert_eq!(input.cursor_position, 7); // 2 + 4 + 1 bytes
    }

    #[test]
    fn test_move_cursor_left_ascii() {
        let mut input = InputArea::new(3);
        input.buffer = "abc".to_string();
        input.cursor_position = 3;

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 2);

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 1);

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 0);

        // Should not go below 0
        input.move_cursor_left();
        assert_eq!(input.cursor_position, 0);
    }

    #[test]
    fn test_move_cursor_left_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 6; // end of string (1 + 4 + 1 bytes)

        input.move_cursor_left(); // move before 'b'
        assert_eq!(input.cursor_position, 5);

        input.move_cursor_left(); // move before emoji (skips all 4 bytes)
        assert_eq!(input.cursor_position, 1);

        input.move_cursor_left(); // move before 'a'
        assert_eq!(input.cursor_position, 0);
    }

    #[test]
    fn test_move_cursor_right_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 0;

        input.move_cursor_right(); // move after 'a'
        assert_eq!(input.cursor_position, 1);

        input.move_cursor_right(); // move after emoji (skips all 4 bytes)
        assert_eq!(input.cursor_position, 5);

        input.move_cursor_right(); // move after 'b'
        assert_eq!(input.cursor_position, 6);

        // Should not go beyond end
        input.move_cursor_right();
        assert_eq!(input.cursor_position, 6);
    }

    #[test]
    fn test_delete_char_ascii() {
        let mut input = InputArea::new(3);
        input.buffer = "abc".to_string();
        input.cursor_position = 3;

        input.delete_char();
        assert_eq!(input.buffer, "ab");
        assert_eq!(input.cursor_position, 2);
    }

    #[test]
    fn test_delete_char_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 5; // after emoji

        input.delete_char(); // delete emoji
        assert_eq!(input.buffer, "ab");
        assert_eq!(input.cursor_position, 1);
    }

    #[test]
    fn test_delete_char_forward_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 1; // before emoji

        input.delete_char_forward(); // delete emoji
        assert_eq!(input.buffer, "ab");
        assert_eq!(input.cursor_position, 1);
    }

    #[test]
    fn test_cursor_line_with_emoji() {
        let mut input = InputArea::new(3);
        input.width = 5;
        // 5 emojis = 5 characters, should wrap to 2 lines
        input.buffer = "😀😀😀😀😀".to_string();
        input.cursor_position = input.buffer.len(); // end

        // 5 chars at width 5 = cursor on line 1 (0-indexed)
        assert_eq!(input.cursor_line(), 1);
    }

    #[test]
    fn test_delete_word_before_cursor_with_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "hello 😀😀 world".to_string();
        input.cursor_position = input.buffer.len();

        input.delete_word_before_cursor(); // delete "world"
        assert_eq!(input.buffer, "hello 😀😀 ");

        // delete_word skips whitespace first, then deletes non-whitespace
        // so this deletes " 😀😀" (space + emojis)
        input.delete_word_before_cursor();
        assert_eq!(input.buffer, "hello ");

        input.delete_word_before_cursor(); // delete "hello"
        assert_eq!(input.buffer, "");
    }

    #[test]
    fn test_home_and_end() {
        let mut input = InputArea::new(3);
        input.buffer = "a😀b".to_string();
        input.cursor_position = 5;

        input.home();
        assert_eq!(input.cursor_position, 0);

        input.end();
        assert_eq!(input.cursor_position, 6);
    }

    #[test]
    fn test_insert_at_middle_with_emoji() {
        let mut input = InputArea::new(3);
        input.buffer = "ab".to_string();
        input.cursor_position = 1; // between a and b

        input.insert_char('😀');
        assert_eq!(input.buffer, "a😀b");
        assert_eq!(input.cursor_position, 5); // 1 + 4 bytes
    }

    #[test]
    fn test_multiple_emojis() {
        let mut input = InputArea::new(3);
        input.insert_char('🎉');
        input.insert_char('🎊');
        input.insert_char('🎈');

        assert_eq!(input.buffer, "🎉🎊🎈");
        assert_eq!(input.cursor_position, 12); // 3 emojis * 4 bytes each

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 8);

        input.delete_char();
        assert_eq!(input.buffer, "🎉🎈");
        assert_eq!(input.cursor_position, 4);
    }

    #[test]
    fn test_unicode_characters() {
        let mut input = InputArea::new(3);
        // Test various unicode: Chinese, emoji, accented
        input.insert_char('中');  // 3 bytes
        input.insert_char('😀');  // 4 bytes
        input.insert_char('é');   // 2 bytes

        assert_eq!(input.buffer, "中😀é");
        assert_eq!(input.cursor_position, 9); // 3 + 4 + 2

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 7); // before é

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 3); // before 😀

        input.move_cursor_left();
        assert_eq!(input.cursor_position, 0); // before 中
    }

    #[test]
    fn test_password_encrypt_decrypt() {
        // Test basic encryption/decryption
        let password = "mysecretpassword";
        let encrypted = encrypt_password(password);
        assert!(encrypted.starts_with("ENC:"));
        let decrypted = decrypt_password(&encrypted);
        assert_eq!(decrypted, password);
    }

    #[test]
    fn test_password_empty() {
        // Empty password should stay empty
        let encrypted = encrypt_password("");
        assert_eq!(encrypted, "");
        let decrypted = decrypt_password("");
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_password_plain_fallback() {
        // Plain passwords (not starting with ENC:) should be returned as-is
        let plain = "plainpassword";
        let decrypted = decrypt_password(plain);
        assert_eq!(decrypted, plain);
    }

    #[test]
    fn test_password_special_chars() {
        // Test password with special characters
        let password = "p@$$w0rd!#$%^&*()";
        let encrypted = encrypt_password(password);
        let decrypted = decrypt_password(&encrypted);
        assert_eq!(decrypted, password);
    }

    #[test]
    fn test_password_unicode() {
        // Test password with unicode
        let password = "密码🔐пароль";
        let encrypted = encrypt_password(password);
        let decrypted = decrypt_password(&encrypted);
        assert_eq!(decrypted, password);
    }

    #[test]
    fn test_hash_password() {
        let hash = hash_password("test");
        assert_eq!(hash, "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08");
    }

    #[tokio::test]
    async fn test_websocket_auth() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::{connect_async, tungstenite::Message as WsRawMessage};
        use futures::{SinkExt, StreamExt};

        // Start a minimal WebSocket server on a random port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        // Expected password hash for "test"
        let server_password = "test";
        let expected_hash = hash_password(server_password);
        println!("Server expects hash: {}", expected_hash);

        // Spawn server task
        let server_hash = expected_hash.clone();
        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut ws_sink, mut ws_source) = ws_stream.split();

            while let Some(msg_result) = ws_source.next().await {
                if let Ok(WsRawMessage::Text(text)) = msg_result {
                    println!("Server received: {}", text);
                    if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                        if let WsMessage::AuthRequest { password_hash: client_hash } = ws_msg {
                            println!("Client hash: {}", client_hash);
                            println!("Server hash: {}", server_hash);
                            let auth_success = client_hash == server_hash;
                            println!("Auth success: {}", auth_success);
                            let response = WsMessage::AuthResponse {
                                success: auth_success,
                                error: if auth_success { None } else { Some("Invalid password".to_string()) },
                            };
                            let json = serde_json::to_string(&response).unwrap();
                            ws_sink.send(WsRawMessage::Text(json.into())).await.unwrap();
                            break;
                        }
                    }
                }
            }
        });

        // Give server time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Connect client
        let url = format!("ws://127.0.0.1:{}", port);
        let (ws_stream, _) = connect_async(&url).await.unwrap();
        let (mut ws_sink, mut ws_source) = ws_stream.split();

        // Send auth request with correct password hash
        let client_password = "test";
        let client_hash = hash_password(client_password);
        println!("Client sending hash: {}", client_hash);
        let auth_msg = WsMessage::AuthRequest { password_hash: client_hash };
        let json = serde_json::to_string(&auth_msg).unwrap();
        ws_sink.send(WsRawMessage::Text(json.into())).await.unwrap();

        // Wait for response
        if let Some(Ok(WsRawMessage::Text(text))) = ws_source.next().await {
            println!("Client received: {}", text);
            let response: WsMessage = serde_json::from_str(&text).unwrap();
            if let WsMessage::AuthResponse { success, error } = response {
                assert!(success, "Auth should succeed but got error: {:?}", error);
            } else {
                panic!("Expected AuthResponse");
            }
        } else {
            panic!("No response received");
        }

        server_task.abort();
    }

    #[test]
    fn test_world_cycling_all_connected() {
        // Test cycling through multiple connected worlds
        let mut app = App::new();
        app.worlds.clear(); // Remove any default world

        // Create 3 connected worlds with different names
        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("zeta");
        world_zeta.connected = true;
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on alpha

        // Verify initial state
        assert_eq!(app.worlds[app.current_world_index].name, "alpha");

        // Cycle forward: alpha -> cave
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "After first next_world from alpha, should be on cave");

        // Cycle forward: cave -> zeta
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "zeta",
            "After second next_world from cave, should be on zeta");

        // Cycle forward: zeta -> alpha (wrap)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "alpha",
            "After third next_world from zeta, should wrap to alpha");

        // Cycle backward: alpha -> zeta
        app.prev_world();
        assert_eq!(app.worlds[app.current_world_index].name, "zeta",
            "After prev_world from alpha, should be on zeta");

        // Cycle backward: zeta -> cave
        app.prev_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "After prev_world from zeta, should be on cave");
    }

    #[test]
    fn test_world_cycling_with_disconnected() {
        // Test that disconnected worlds without unseen output are skipped
        let mut app = App::new();
        app.worlds.clear();

        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        app.worlds.push(world_alpha);

        let mut world_beta = World::new("beta");
        world_beta.connected = false; // Disconnected, no unseen output
        app.worlds.push(world_beta);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        app.worlds.push(world_cave);

        app.current_world_index = 0; // Start on alpha

        // Cycle forward: alpha -> cave (skipping disconnected beta)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "Should skip disconnected beta and go to cave");

        // Cycle forward: cave -> alpha (skipping disconnected beta)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "alpha",
            "Should skip disconnected beta and wrap to alpha");
    }

    #[test]
    fn test_world_cycling_case_insensitive_sort() {
        // Test that world names are sorted case-insensitively
        let mut app = App::new();
        app.worlds.clear();

        let mut world_alpha = World::new("Alpha"); // Capital A
        world_alpha.connected = true;
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave"); // lowercase c
        world_cave.connected = true;
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("Zeta"); // Capital Z
        world_zeta.connected = true;
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on Alpha

        // Should cycle: Alpha -> cave -> Zeta (case-insensitive alphabetical)
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "Case-insensitive sort: Alpha -> cave");

        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "Zeta",
            "Case-insensitive sort: cave -> Zeta");

        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "Alpha",
            "Case-insensitive sort: Zeta -> Alpha (wrap)");
    }

    #[test]
    fn test_world_cycling_unseen_first_no_unseen() {
        // Test world_switch_mode=UnseenFirst when no worlds have unseen output
        let mut app = App::new();
        app.worlds.clear();
        app.settings.world_switch_mode = WorldSwitchMode::UnseenFirst;

        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        world_alpha.unseen_lines = 0; // No unseen
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        world_cave.unseen_lines = 0; // No unseen
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("zeta");
        world_zeta.connected = true;
        world_zeta.unseen_lines = 0; // No unseen
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on alpha

        // With UnseenFirst ON but no unseen, should cycle alphabetically
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "With UnseenFirst but no unseen, should go to cave");

        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "zeta",
            "With UnseenFirst but no unseen, should go to zeta");
    }

    #[test]
    fn test_world_cycling_unseen_first_with_unseen() {
        // Test world_switch_mode=UnseenFirst prioritizes worlds with unseen output
        let mut app = App::new();
        app.worlds.clear();
        app.settings.world_switch_mode = WorldSwitchMode::UnseenFirst;

        let mut world_alpha = World::new("alpha");
        world_alpha.connected = true;
        world_alpha.unseen_lines = 0; // No unseen
        app.worlds.push(world_alpha);

        let mut world_cave = World::new("cave");
        world_cave.connected = true;
        world_cave.unseen_lines = 5; // Has unseen!
        app.worlds.push(world_cave);

        let mut world_zeta = World::new("zeta");
        world_zeta.connected = true;
        world_zeta.unseen_lines = 0; // No unseen
        app.worlds.push(world_zeta);

        app.current_world_index = 0; // Start on alpha

        // With UnseenFirst ON and cave has unseen, should go to cave first
        app.next_world();
        assert_eq!(app.worlds[app.current_world_index].name, "cave",
            "With UnseenFirst, should prioritize cave with unseen output");
    }

    #[test]
    fn test_decode_strips_control_chars() {
        // Test that carriage return is stripped
        let input = b"hello\rworld";
        let result = Encoding::Utf8.decode(input);
        assert!(!result.contains('\r'), "Carriage return should be stripped");
        assert_eq!(result, "helloworld", "CR should be removed, text concatenated");

        // Test that other control characters are stripped but tab/newline kept
        let input = b"a\x01b\tc\nd\x7Fe";
        let result = Encoding::Utf8.decode(input);
        assert_eq!(result, "ab\tc\nde", "Control chars stripped except tab/newline");

        // Test that BEL is stripped in final output
        let input = b"hello\x07world";
        let result = Encoding::Utf8.decode(input);
        assert!(!result.contains('\x07'), "BEL should be stripped in final output");
    }

    #[test]
    fn test_strip_non_sgr_sequences() {
        // Test that SGR (color/style) sequences are kept
        let input = "\x1b[31mred text\x1b[0m";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "\x1b[31mred text\x1b[0m", "SGR sequences should be preserved");

        // Test that cursor position (H) inserts newline
        let input = "first\x1b[10;5Hsecond";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "first\nsecond", "Cursor positioning (H) should insert newline");

        // Test that cursor column (G) inserts space
        let input = "before\x1b[10Gafter";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "before after", "Cursor column (G) should insert space");

        // Test that erase sequences are stripped without separator
        let input = "hello\x1b[2Jworld";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "helloworld", "Erase (J) should be stripped");

        // Test that erase line (K) is stripped
        let input = "hello\x1b[Kworld";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "helloworld", "Erase line (K) should be stripped");

        // Test OSC (window title) sequences are stripped
        let input = "before\x1b]0;Window Title\x07after";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "beforeafter", "OSC sequences should be stripped");

        // Test cursor up/down inserts newline
        let input = "line1\x1b[Aline2";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "line1\nline2", "Cursor up (A) should insert newline");

        // Test @ character (insert character)
        let input = "before\x1b[5@after";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "beforeafter", "Insert character (@) should be stripped");

        // Test ~ character (function key)
        let input = "text\x1b[6~more";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "textmore", "Function key sequences (~) should be stripped");

        // Test that consecutive positioning doesn't add multiple separators
        let input = "text\x1b[H\x1b[Hmore";
        let result = strip_non_sgr_sequences(input);
        assert_eq!(result, "text\nmore", "Consecutive H should only add one newline");
    }

    #[test]
    fn test_keep_alive_type_cycling() {
        // Test next() cycling
        assert_eq!(KeepAliveType::None.next(), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::Nop.next(), KeepAliveType::Custom);
        assert_eq!(KeepAliveType::Custom.next(), KeepAliveType::Generic);
        assert_eq!(KeepAliveType::Generic.next(), KeepAliveType::None);

        // Test prev() cycling
        assert_eq!(KeepAliveType::None.prev(), KeepAliveType::Generic);
        assert_eq!(KeepAliveType::Nop.prev(), KeepAliveType::None);
        assert_eq!(KeepAliveType::Custom.prev(), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::Generic.prev(), KeepAliveType::Custom);
    }

    #[test]
    fn test_keep_alive_type_name() {
        assert_eq!(KeepAliveType::None.name(), "None");
        assert_eq!(KeepAliveType::Nop.name(), "NOP");
        assert_eq!(KeepAliveType::Custom.name(), "Custom");
        assert_eq!(KeepAliveType::Generic.name(), "Generic");
    }

    #[test]
    fn test_keep_alive_type_from_name() {
        assert_eq!(KeepAliveType::from_name("None"), KeepAliveType::None);
        assert_eq!(KeepAliveType::from_name("none"), KeepAliveType::None);
        assert_eq!(KeepAliveType::from_name("NOP"), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::from_name("nop"), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::from_name("Custom"), KeepAliveType::Custom);
        assert_eq!(KeepAliveType::from_name("custom"), KeepAliveType::Custom);
        assert_eq!(KeepAliveType::from_name("Generic"), KeepAliveType::Generic);
        assert_eq!(KeepAliveType::from_name("generic"), KeepAliveType::Generic);
        // Unknown should default to Nop
        assert_eq!(KeepAliveType::from_name("unknown"), KeepAliveType::Nop);
        assert_eq!(KeepAliveType::from_name(""), KeepAliveType::Nop);
    }

    #[test]
    fn test_idler_message_filter() {
        // Test that lines containing idler message pattern are detected
        let idler_line = "You don't know how to help commands ###_idler_message_123_###.";
        assert!(idler_line.contains("###_idler_message_") && idler_line.contains("_###"));

        let normal_line = "You say, \"Hello world!\"";
        assert!(!(normal_line.contains("###_idler_message_") && normal_line.contains("_###")));

        // Test partial matches don't trigger
        let partial1 = "###_idler_message_ incomplete";
        assert!(!(partial1.contains("###_idler_message_") && partial1.contains("_###")));

        let partial2 = "incomplete _### suffix only";
        assert!(!(partial2.contains("###_idler_message_") && partial2.contains("_###")));
    }

    #[test]
    fn test_idler_message_replacement() {
        // Test that ##rand## is replaced correctly in custom commands
        let custom_cmd = "look ##rand##";
        let rand_num = 42u32;
        let idler_tag = format!("###_idler_message_{}_###", rand_num);
        let result = custom_cmd.replace("##rand##", &idler_tag);
        assert_eq!(result, "look ###_idler_message_42_###");

        // Test generic command format
        let generic_cmd = format!("help commands ###_idler_message_{}_###", rand_num);
        assert_eq!(generic_cmd, "help commands ###_idler_message_42_###");
    }

    #[test]
    fn test_is_visually_empty() {
        use super::is_visually_empty;

        // Empty string is visually empty
        assert!(is_visually_empty(""));

        // Whitespace-only is visually empty
        assert!(is_visually_empty("   "));
        assert!(is_visually_empty("\t"));
        assert!(is_visually_empty("  \t  "));

        // ANSI codes only are visually empty
        assert!(is_visually_empty("\x1b[0m"));
        assert!(is_visually_empty("\x1b[31m\x1b[0m"));
        assert!(is_visually_empty("\x1b[1;32m"));

        // ANSI codes with whitespace are visually empty
        assert!(is_visually_empty("\x1b[0m   \x1b[31m"));
        assert!(is_visually_empty("  \x1b[0m  "));

        // Visible text is NOT visually empty
        assert!(!is_visually_empty("hello"));
        assert!(!is_visually_empty("  hello  "));
        assert!(!is_visually_empty("\x1b[31mhello\x1b[0m"));
        assert!(!is_visually_empty("a"));
        assert!(!is_visually_empty("\x1b[0m.\x1b[0m"));
    }

    #[test]
    fn test_strip_mud_tag() {
        use super::strip_mud_tag;

        // Basic tag stripping
        assert_eq!(strip_mud_tag("[channel:] hello"), "hello");
        assert_eq!(strip_mud_tag("[chat:] message"), "message");
        assert_eq!(strip_mud_tag("[ooc(player)] text"), "text");

        // With leading whitespace
        assert_eq!(strip_mud_tag("  [channel:] hello"), "  hello");

        // With ANSI color prefix
        assert_eq!(strip_mud_tag("\x1b[31m[channel:] hello"), "\x1b[31mhello");
        assert_eq!(strip_mud_tag("\x1b[1;32m[chat:] text"), "\x1b[1;32mtext");

        // Non-tag brackets should NOT be stripped
        assert_eq!(strip_mud_tag("[hello] world"), "[hello] world");
        assert_eq!(strip_mud_tag("[nochannel] text"), "[nochannel] text");

        // No brackets at start
        assert_eq!(strip_mud_tag("hello world"), "hello world");
        assert_eq!(strip_mud_tag("text [tag:] later"), "text [tag:] later");

        // Empty or only tag
        assert_eq!(strip_mud_tag("[channel:]"), "");
        assert_eq!(strip_mud_tag("[channel:] "), "");
    }
}
