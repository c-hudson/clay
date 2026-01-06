use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::net::TcpListener;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use tokio_tungstenite::{accept_async, tungstenite::Message as WsRawMessage};

// Import AppEvent and Action from the main crate
use crate::{AppEvent, Action};

// ============================================================================
// WebSocket Protocol Types
// ============================================================================

/// WebSocket protocol messages for client-server communication
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum WsMessage {
    // Authentication
    AuthRequest { password_hash: String },
    AuthResponse { success: bool, error: Option<String> },

    // Initial state (server -> client after auth)
    InitialState {
        worlds: Vec<WorldStateMsg>,
        settings: GlobalSettingsMsg,
        current_world_index: usize,
        actions: Vec<Action>,
    },

    // Real-time updates (server -> client)
    ServerData { world_index: usize, data: String },
    WorldConnected { world_index: usize, name: String },
    WorldDisconnected { world_index: usize },
    WorldAdded { world: Box<WorldStateMsg> },
    WorldRemoved { world_index: usize },
    WorldSwitched { new_index: usize },
    PromptUpdate { world_index: usize, prompt: String },
    PendingLinesUpdate { world_index: usize, count: usize },

    // Commands (client -> server)
    SendCommand { world_index: usize, command: String },
    SwitchWorld { world_index: usize },
    ConnectWorld { world_index: usize },
    DisconnectWorld { world_index: usize },
    CreateWorld { name: String },
    ReleasePending { world_index: usize },

    // Settings updates (client -> server)
    UpdateWorldSettings {
        world_index: usize,
        name: String,
        hostname: String,
        port: String,
        user: String,
        use_ssl: bool,
        keep_alive_type: String,
        keep_alive_cmd: String,
    },
    UpdateGlobalSettings {
        console_theme: String,
        gui_theme: String,
        input_height: u16,
        font_name: String,
        font_size: f32,
        ws_allow_list: String,
    },

    // Settings update confirmations (server -> client)
    WorldSettingsUpdated { world_index: usize, settings: WorldSettingsMsg, name: String },
    GlobalSettingsUpdated { settings: GlobalSettingsMsg, input_height: u16 },

    // Actions (triggers)
    ActionsUpdated { actions: Vec<Action> },
    UpdateActions { actions: Vec<Action> },

    // Keepalive
    Ping,
    Pong,
}

/// World state for WebSocket protocol
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorldStateMsg {
    pub index: usize,
    pub name: String,
    pub connected: bool,
    pub output_lines: Vec<String>,
    pub pending_lines: Vec<String>,
    pub scroll_offset: usize,
    pub paused: bool,
    pub prompt: String,
    pub unseen_lines: usize,
    pub settings: WorldSettingsMsg,
}

/// World settings for WebSocket protocol (password intentionally omitted)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorldSettingsMsg {
    pub hostname: String,
    pub port: String,
    pub user: String,
    pub use_ssl: bool,
    pub log_file: Option<String>,
    pub encoding: String,
    pub auto_connect_type: String,
    pub keep_alive_type: String,
    pub keep_alive_cmd: String,
}

/// Global settings for WebSocket protocol
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GlobalSettingsMsg {
    pub more_mode_enabled: bool,
    pub spell_check_enabled: bool,
    pub world_switch_mode: String,
    pub debug_enabled: bool,
    pub show_tags: bool,
    pub console_theme: String,
    pub gui_theme: String,
    pub input_height: u16,
    pub font_name: String,
    pub font_size: f32,
    pub ws_allow_list: String,
}

/// Information about a connected WebSocket client
pub struct WsClientInfo {
    pub authenticated: bool,
    pub tx: mpsc::UnboundedSender<WsMessage>,
}

/// WebSocket server state
pub struct WebSocketServer {
    pub clients: Arc<RwLock<HashMap<u64, WsClientInfo>>>,
    pub next_client_id: Arc<std::sync::Mutex<u64>>,
    pub password_hash: String,
    pub running: Arc<RwLock<bool>>,
    pub shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub port: u16,
    pub allow_list: Arc<std::sync::RwLock<Vec<String>>>,
    /// Single whitelisted host that can connect without password
    /// Set when a user authenticates from an allow-list host
    pub whitelisted_host: Arc<std::sync::RwLock<Option<String>>>,
    #[cfg(feature = "native-tls-backend")]
    pub tls_acceptor: Option<Arc<tokio_native_tls::TlsAcceptor>>,
    #[cfg(feature = "rustls-backend")]
    pub tls_acceptor: Option<Arc<tokio_rustls::TlsAcceptor>>,
}

impl WebSocketServer {
    pub fn new(password: &str, port: u16, allow_list: &str, whitelisted_host: Option<String>) -> Self {
        let password_hash = hash_password(password);
        // Parse allow list: comma-separated, trimmed entries
        let allow_list_vec: Vec<String> = allow_list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            next_client_id: Arc::new(std::sync::Mutex::new(1)),
            password_hash,
            running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
            port,
            allow_list: Arc::new(std::sync::RwLock::new(allow_list_vec)),
            whitelisted_host: Arc::new(std::sync::RwLock::new(whitelisted_host)),
            #[cfg(feature = "native-tls-backend")]
            tls_acceptor: None,
            #[cfg(feature = "rustls-backend")]
            tls_acceptor: None,
        }
    }

    /// Get the current whitelisted host (for saving state)
    pub fn get_whitelisted_host(&self) -> Option<String> {
        self.whitelisted_host.read().unwrap().clone()
    }

    /// Configure TLS for WSS support
    #[cfg(feature = "native-tls-backend")]
    pub fn configure_tls(&mut self, cert_file: &str, key_file: &str) -> Result<(), Box<dyn std::error::Error>> {
        use std::fs::File;
        use std::io::Read;

        // Read certificate file
        let mut cert_data = Vec::new();
        File::open(cert_file)?.read_to_end(&mut cert_data)?;

        // Read key file
        let mut key_data = Vec::new();
        File::open(key_file)?.read_to_end(&mut key_data)?;

        // Create identity from PEM files
        let identity = native_tls::Identity::from_pkcs8(&cert_data, &key_data)?;

        // Create TLS acceptor
        let tls_acceptor = native_tls::TlsAcceptor::new(identity)?;
        let tls_acceptor = tokio_native_tls::TlsAcceptor::from(tls_acceptor);

        self.tls_acceptor = Some(Arc::new(tls_acceptor));
        Ok(())
    }

    /// Configure TLS for WSS support (rustls version)
    #[cfg(feature = "rustls-backend")]
    pub fn configure_tls(&mut self, cert_file: &str, key_file: &str) -> Result<(), Box<dyn std::error::Error>> {
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

        self.tls_acceptor = Some(Arc::new(tokio_rustls::TlsAcceptor::from(Arc::new(config))));
        Ok(())
    }

    pub fn update_allow_list(&self, allow_list: &str) {
        let allow_list_vec: Vec<String> = allow_list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        *self.allow_list.write().unwrap() = allow_list_vec;
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Hash a password using SHA-256
pub fn hash_password(password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

/// Check if an IP address is in the allow list (supports wildcards like 192.168.1.*)
pub fn is_ip_in_allow_list(ip: &str, allow_list: &[String]) -> bool {
    // Normalize localhost
    let normalized_ip = if ip == "127.0.0.1" || ip == "::1" { "localhost" } else { ip };

    for pattern in allow_list {
        // Normalize pattern for localhost comparison
        let normalized_pattern = if pattern == "127.0.0.1" || pattern == "::1" { "localhost" } else { pattern.as_str() };

        if let Some(prefix) = normalized_pattern.strip_suffix('*') {
            // Wildcard match: check if IP starts with pattern prefix
            if normalized_ip.starts_with(prefix) {
                return true;
            }
        } else if normalized_ip == normalized_pattern {
            return true;
        }
    }
    false
}

/// Start the WebSocket server
pub async fn start_websocket_server(
    server: &mut WebSocketServer,
    event_tx: mpsc::Sender<AppEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("0.0.0.0:{}", server.port);
    let listener = TcpListener::bind(&addr).await?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let clients = Arc::clone(&server.clients);
    let next_client_id = Arc::clone(&server.next_client_id);
    let password_hash = server.password_hash.clone();
    let allow_list = server.allow_list.clone();
    let whitelisted_host = server.whitelisted_host.clone();
    let running = Arc::clone(&server.running);
    #[cfg(feature = "native-tls-backend")]
    let tls_acceptor = server.tls_acceptor.clone();
    #[cfg(feature = "rustls-backend")]
    let tls_acceptor = server.tls_acceptor.clone();

    *running.write().await = true;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, client_addr)) => {
                            let client_id = {
                                let mut id = next_client_id.lock().unwrap();
                                let current = *id;
                                *id += 1;
                                current
                            };

                            let clients = Arc::clone(&clients);
                            let password_hash = password_hash.clone();
                            let allow_list = allow_list.clone();
                            let whitelisted_host = whitelisted_host.clone();
                            let event_tx = event_tx.clone();
                            #[cfg(feature = "native-tls-backend")]
                            let tls_acceptor = tls_acceptor.clone();
                            #[cfg(feature = "rustls-backend")]
                            let tls_acceptor = tls_acceptor.clone();

                            tokio::spawn(async move {
                                // If TLS is enabled, wrap the stream (native-tls)
                                #[cfg(feature = "native-tls-backend")]
                                if let Some(acceptor) = tls_acceptor {
                                    match acceptor.accept(stream).await {
                                        Ok(tls_stream) => {
                                            if let Err(_e) = handle_ws_client(
                                                tls_stream,
                                                client_id,
                                                clients,
                                                password_hash,
                                                allow_list,
                                                whitelisted_host,
                                                client_addr,
                                                event_tx,
                                            ).await {
                                                // Connection error, client disconnected
                                            }
                                        }
                                        Err(_e) => {
                                            // TLS handshake failed
                                        }
                                    }
                                } else if let Err(_e) = handle_ws_client(
                                    stream,
                                    client_id,
                                    clients,
                                    password_hash,
                                    allow_list,
                                    whitelisted_host,
                                    client_addr,
                                    event_tx,
                                ).await {
                                    // Connection error, client disconnected
                                }

                                // If TLS is enabled, wrap the stream (rustls)
                                #[cfg(feature = "rustls-backend")]
                                if let Some(acceptor) = tls_acceptor {
                                    match acceptor.accept(stream).await {
                                        Ok(tls_stream) => {
                                            if let Err(_e) = handle_ws_client(
                                                tls_stream,
                                                client_id,
                                                clients,
                                                password_hash,
                                                allow_list,
                                                whitelisted_host,
                                                client_addr,
                                                event_tx,
                                            ).await {
                                                // Connection error, client disconnected
                                            }
                                        }
                                        Err(e) => {
                                            // TLS handshake failed - log for debugging
                                            eprintln!("WSS TLS handshake failed from {}: {}", client_addr, e);
                                        }
                                    }
                                } else if let Err(_e) = handle_ws_client(
                                    stream,
                                    client_id,
                                    clients,
                                    password_hash,
                                    allow_list,
                                    whitelisted_host,
                                    client_addr,
                                    event_tx,
                                ).await {
                                    // Connection error, client disconnected
                                }
                            });
                        }
                        Err(_) => {
                            // Accept error
                            break;
                        }
                    }
                }
                _ = &mut shutdown_rx => {
                    // Shutdown signal received
                    break;
                }
            }
        }
        *running.write().await = false;
    });

    Ok(())
}

/// Handle a single WebSocket client connection
#[allow(clippy::too_many_arguments)]
pub async fn handle_ws_client<S>(
    stream: S,
    client_id: u64,
    clients: Arc<RwLock<HashMap<u64, WsClientInfo>>>,
    password_hash: String,
    allow_list: Arc<std::sync::RwLock<Vec<String>>>,
    whitelisted_host: Arc<std::sync::RwLock<Option<String>>>,
    client_addr: std::net::SocketAddr,
    event_tx: mpsc::Sender<AppEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use futures::{SinkExt, StreamExt};

    // Check if client IP is whitelisted (previously authenticated from an allow-list host)
    let client_ip = client_addr.ip().to_string();
    let is_whitelisted = {
        let whitelist_guard = whitelisted_host.read().unwrap();
        whitelist_guard.as_ref().map(|h| h == &client_ip).unwrap_or(false)
    };

    let ws_stream = accept_async(stream).await?;
    let (mut ws_sink, mut ws_source) = ws_stream.split();

    // Create channel for sending messages to this client
    let (tx, mut rx) = mpsc::unbounded_channel::<WsMessage>();

    // Add client to clients map (auto-authenticated if whitelisted)
    {
        let mut clients_guard = clients.write().await;
        clients_guard.insert(client_id, WsClientInfo {
            authenticated: is_whitelisted,
            tx: tx.clone(),
        });
    }

    // Notify app of new connection
    let _ = event_tx.send(AppEvent::WsClientConnected(client_id)).await;

    // If auto-authenticated via whitelist, send success response and trigger initial state
    if is_whitelisted {
        let response = WsMessage::AuthResponse {
            success: true,
            error: None,
        };
        let _ = tx.send(response);
        // Create a fake AuthRequest to trigger initial state send
        let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(WsMessage::AuthRequest { password_hash: String::new() }))).await;
    }

    // Spawn task to send messages from rx to WebSocket
    let clients_for_sender = Arc::clone(&clients);
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_sink.send(WsRawMessage::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
        let _ = clients_for_sender.write().await.remove(&client_id);
    });

    // Process incoming messages
    while let Some(msg_result) = ws_source.next().await {
        match msg_result {
            Ok(WsRawMessage::Text(text)) => {
                if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                    match &ws_msg {
                        WsMessage::AuthRequest { password_hash: client_hash } => {
                            // Verify password
                            let auth_success = *client_hash == password_hash;
                            if auth_success {
                                // Mark as authenticated
                                let mut clients_guard = clients.write().await;
                                if let Some(client) = clients_guard.get_mut(&client_id) {
                                    client.authenticated = true;
                                }

                                // If client IP is in allow list, whitelist this host
                                // This clears any previously whitelisted host
                                let in_allow_list = {
                                    let allow_list_guard = allow_list.read().unwrap();
                                    is_ip_in_allow_list(&client_ip, &allow_list_guard)
                                };
                                if in_allow_list {
                                    let mut whitelist = whitelisted_host.write().unwrap();
                                    *whitelist = Some(client_ip.clone());
                                }
                            }
                            // Send auth response
                            let response = WsMessage::AuthResponse {
                                success: auth_success,
                                error: if auth_success { None } else { Some("Invalid password".to_string()) },
                            };
                            let _ = tx.send(response);

                            if auth_success {
                                // Forward to app to send initial state
                                let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(ws_msg))).await;
                            }
                        }
                        WsMessage::Ping => {
                            let _ = tx.send(WsMessage::Pong);
                        }
                        _ => {
                            // Check if authenticated before processing other messages
                            let is_authed = {
                                let clients_guard = clients.read().await;
                                clients_guard.get(&client_id).map(|c| c.authenticated).unwrap_or(false)
                            };
                            if is_authed {
                                let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(ws_msg))).await;
                            }
                        }
                    }
                }
            }
            Ok(WsRawMessage::Close(_)) => {
                break;
            }
            Ok(WsRawMessage::Ping(data)) => {
                // Pong is handled automatically by tungstenite
                let _ = data;
            }
            Err(_) => {
                break;
            }
            _ => {}
        }
    }

    // Clean up
    send_task.abort();
    {
        let mut clients_guard = clients.write().await;
        clients_guard.remove(&client_id);
    }
    let _ = event_tx.send(AppEvent::WsClientDisconnected(client_id)).await;

    Ok(())
}
