use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::net::TcpListener;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use tokio_tungstenite::{accept_async, tungstenite::Message as WsRawMessage};

// Import AppEvent and Action from the main crate
use crate::{AppEvent, Action, BanList};
use crate::ansi_music::MusicNote;
use crate::http::log_ws_auth;

// ============================================================================
// WebSocket Protocol Types
// ============================================================================

/// Default function for serde to return true (for from_server field backwards compatibility)
fn default_true() -> bool { true }

/// WebSocket protocol messages for client-server communication
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum WsMessage {
    // Server hello (sent immediately on connection, before auth)
    ServerHello {
        multiuser_mode: bool,  // True if server requires username + password
    },

    // Authentication
    AuthRequest {
        #[serde(default)]
        username: Option<String>,  // Required in multiuser mode
        password_hash: String,
        #[serde(default)]
        current_world: Option<usize>,  // Client's current world (for reconnection)
        #[serde(default)]
        auth_key: Option<String>,  // Device auth key (alternative to password)
        #[serde(default)]
        request_key: bool,  // If true, request a new auth key after successful password auth
    },
    AuthResponse {
        success: bool,
        error: Option<String>,
        #[serde(default)]
        username: Option<String>,  // Confirmed username on success (multiuser mode)
        #[serde(default)]
        multiuser_mode: bool,      // True if server is in multiuser mode
    },
    // Server sends auth key to client after successful password auth (if requested)
    KeyGenerated {
        auth_key: String,
    },
    // Client requests to revoke its auth key
    RevokeKey {
        auth_key: String,
    },
    // Server confirms key revocation
    KeyRevoked {
        success: bool,
    },

    // Password change (multiuser mode)
    ChangePassword {
        old_password_hash: String,
        new_password_hash: String,
    },
    PasswordChanged {
        success: bool,
        error: Option<String>,
    },

    // Logout (multiuser mode - client -> server)
    Logout,
    // Logout response (server -> client)
    LoggedOut,

    // Initial state (server -> client after auth)
    InitialState {
        worlds: Vec<WorldStateMsg>,
        settings: GlobalSettingsMsg,
        current_world_index: usize,
        actions: Vec<Action>,
        #[serde(default)]
        splash_lines: Vec<String>,
    },

    // Real-time updates (server -> client)
    /// is_viewed: true if any interface (console/web/GUI) is viewing this world
    /// ts: timestamp in seconds since Unix epoch (when the line was received)
    /// from_server: true if data came from MUD server, false if client-generated
    ServerData { world_index: usize, data: String, is_viewed: bool, #[serde(default)] ts: u64, #[serde(default = "default_true")] from_server: bool },
    WorldConnected { world_index: usize, name: String },
    WorldDisconnected { world_index: usize },
    WorldAdded { world: Box<WorldStateMsg> },
    WorldRemoved { world_index: usize },
    WorldSwitched { new_index: usize },
    PromptUpdate { world_index: usize, prompt: String },
    PendingLinesUpdate { world_index: usize, count: usize },
    /// Broadcast when pending lines are released (by any interface)
    PendingReleased { world_index: usize, count: usize },
    UnseenCleared { world_index: usize },
    UnseenUpdate { world_index: usize, count: usize },
    /// Broadcast server's activity count (number of worlds with activity)
    ActivityUpdate { count: usize },
    /// Broadcast when show_tags setting changes (F2 or /tag command)
    ShowTagsChanged { show_tags: bool },
    /// Clear all output for a world (from /flush command)
    WorldFlushed { world_index: usize },
    /// Tell client to execute a command locally (for action commands like /worlds)
    ExecuteLocalCommand { command: String },

    /// Notification for mobile clients (server -> client)
    Notification { title: String, message: String },

    /// ANSI Music sequence to play (server -> client)
    AnsiMusic { world_index: usize, notes: Vec<MusicNote> },

    // Commands (client -> server)
    SendCommand { world_index: usize, command: String },
    SwitchWorld { world_index: usize },
    ConnectWorld { world_index: usize },
    DisconnectWorld { world_index: usize },
    DeleteWorld { world_index: usize },
    CreateWorld { name: String },
    /// Request to release pending lines (count = number to release, 0 = all)
    ReleasePending { world_index: usize, count: usize },
    MarkWorldSeen { world_index: usize },
    /// Update client's view state (world index and visible lines for more-mode calculation)
    UpdateViewState { world_index: usize, visible_lines: usize },
    /// Update client's output dimensions (for NAWS - report smallest across all instances)
    UpdateDimensions { width: u16, height: u16 },
    RequestState,  // Request full state resync
    /// Request state for a specific world (client -> server, used when switching worlds)
    RequestWorldState { world_index: usize },
    /// Response with current state for a specific world (server -> client)
    WorldStateResponse {
        world_index: usize,
        pending_count: usize,    // Number of pending lines (more-mode)
        prompt: String,          // Current prompt
        scroll_offset: usize,    // Current scroll position
        /// Recent output lines (only lines received since client's last known state)
        recent_lines: Vec<TimestampedLine>,
    },

    // Settings updates (client -> server)
    UpdateWorldSettings {
        world_index: usize,
        name: String,
        hostname: String,
        port: String,
        user: String,
        password: String,
        use_ssl: bool,
        log_enabled: bool,
        encoding: String,
        auto_login: String,
        keep_alive_type: String,
        keep_alive_cmd: String,
    },
    UpdateGlobalSettings {
        more_mode_enabled: bool,
        spell_check_enabled: bool,
        #[serde(default)]
        temp_convert_enabled: bool,
        world_switch_mode: String,
        show_tags: bool,
        #[serde(default)]
        debug_enabled: bool,
        ansi_music_enabled: bool,
        console_theme: String,
        gui_theme: String,
        gui_transparency: f32,
        #[serde(default)]
        color_offset_percent: u8,
        input_height: u16,
        font_name: String,
        font_size: f32,
        web_font_size_phone: f32,
        web_font_size_tablet: f32,
        web_font_size_desktop: f32,
        ws_allow_list: String,
        web_secure: bool,
        http_enabled: bool,
        http_port: u16,
        ws_enabled: bool,
        ws_port: u16,
        ws_cert_file: String,
        ws_key_file: String,
        tls_proxy_enabled: bool,
        #[serde(default)]
        dictionary_path: String,
    },

    // Settings update confirmations (server -> client)
    WorldSettingsUpdated { world_index: usize, settings: WorldSettingsMsg, name: String },
    GlobalSettingsUpdated { settings: GlobalSettingsMsg, input_height: u16 },

    // Actions (triggers)
    ActionsUpdated { actions: Vec<Action> },
    UpdateActions { actions: Vec<Action> },

    // Ban list management
    /// Request current ban list (client -> server)
    BanListRequest,
    /// Current ban list (server -> client)
    /// Each entry is (ip, ban_type, reason) where ban_type is "permanent" or "temporary"
    BanListResponse { bans: Vec<(String, String, String)> },
    /// Request to unban a host (client -> server)
    UnbanRequest { host: String },
    /// Result of unban request (server -> client)
    UnbanResult { success: bool, host: String, error: Option<String> },

    // World switching calculation (client -> server)
    CalculateNextWorld { current_index: usize },
    CalculatePrevWorld { current_index: usize },
    /// Find world with oldest pending output (for Escape+w)
    CalculateOldestPending { current_index: usize },
    // World switching response (server -> client)
    CalculatedWorld { index: Option<usize> },

    /// Request connections list (/l command) - client -> server
    RequestConnectionsList,
    /// Connections list response - server -> client
    /// Lines are pre-formatted for display
    ConnectionsListResponse { lines: Vec<String> },

    /// Report a sequence mismatch detected by a remote client (client -> server)
    ReportSeqMismatch {
        world_index: usize,
        expected_seq_gt: u64,
        actual_seq: u64,
        line_text: String,
        source: String,  // "web", "gui", "console"
    },

    // Remote instance handling (client -> server)
    /// Client declares its type on connection (affects output delivery)
    ClientTypeDeclaration { client_type: RemoteClientType },
    /// Request to cycle to next/previous world (master applies switching rules)
    CycleWorld { direction: String },  // "up" or "down"
    /// Request scrollback lines from master (console clients only)
    /// before_seq: oldest sequence number the client has (server sends lines with seq < before_seq)
    RequestScrollback { world_index: usize, count: usize, #[serde(default)] before_seq: Option<u64> },

    // Remote instance handling (server -> client)
    /// Batch of output lines for a world (initial or incremental)
    OutputLines {
        world_index: usize,
        lines: Vec<TimestampedLine>,
        is_initial: bool,  // True for initial load or world switch
    },
    /// Periodic pending count update (sent every 2 seconds when pending count changes)
    PendingCountUpdate { world_index: usize, count: usize },
    /// Response to RequestScrollback with historical lines
    ScrollbackLines { world_index: usize, lines: Vec<TimestampedLine> },
    /// World switch result with appropriate initial data
    WorldSwitchResult {
        world_index: usize,
        world_name: String,
        pending_count: usize,
        paused: bool,
    },

    // Keepalive
    Ping,
    Pong,
}

/// A line of output with timestamp
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TimestampedLine {
    pub text: String,
    pub ts: u64, // seconds since Unix epoch
    #[serde(default)]
    pub gagged: bool, // true if line was gagged by an action (only shown with F2/show_tags)
    #[serde(default = "default_true")]
    pub from_server: bool, // true if from MUD server, false if client-generated
    #[serde(default)]
    pub seq: u64, // Unique sequential number within the world (for debugging)
    #[serde(default)]
    pub highlight_color: Option<String>, // Optional highlight color from /highlight action command
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
    // Timing info (seconds since event, None if never)
    pub last_send_secs: Option<u64>,
    pub last_recv_secs: Option<u64>,
    pub last_nop_secs: Option<u64>,
    pub keep_alive_type: String,
    // Timestamped versions of output/pending lines (optional for backward compat)
    #[serde(default)]
    pub output_lines_ts: Vec<TimestampedLine>,
    #[serde(default)]
    pub pending_lines_ts: Vec<TimestampedLine>,
    // Whether splash screen is being shown (for centering)
    #[serde(default)]
    pub showing_splash: bool,
    // Whether world has ever connected (for separator bar display)
    #[serde(default)]
    pub was_connected: bool,
    // Whether the connection uses a TLS proxy
    #[serde(default)]
    pub is_proxy: bool,
}

/// World settings for WebSocket protocol
/// Password is always transmitted encrypted (with "ENC:" prefix)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorldSettingsMsg {
    pub hostname: String,
    pub port: String,
    pub user: String,
    #[serde(default)]
    pub password: String,  // Always encrypted when transmitted
    pub use_ssl: bool,
    pub log_enabled: bool,
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
    #[serde(default)]
    pub temp_convert_enabled: bool,
    pub world_switch_mode: String,
    pub debug_enabled: bool,
    pub show_tags: bool,
    pub ansi_music_enabled: bool,
    pub console_theme: String,
    pub gui_theme: String,
    #[serde(default = "default_gui_transparency")]
    pub gui_transparency: f32,
    #[serde(default)]
    pub color_offset_percent: u8,
    pub input_height: u16,
    pub font_name: String,
    pub font_size: f32,
    #[serde(default = "default_web_font_size_phone")]
    pub web_font_size_phone: f32,
    #[serde(default = "default_web_font_size_tablet")]
    pub web_font_size_tablet: f32,
    #[serde(default = "default_web_font_size_desktop")]
    pub web_font_size_desktop: f32,
    pub ws_allow_list: String,
    pub web_secure: bool,
    pub http_enabled: bool,
    pub http_port: u16,
    pub ws_enabled: bool,
    pub ws_port: u16,
    pub ws_cert_file: String,
    pub ws_key_file: String,
    #[serde(default)]
    pub tls_proxy_enabled: bool,
    #[serde(default)]
    pub dictionary_path: String,
}

fn default_gui_transparency() -> f32 {
    1.0
}

fn default_web_font_size_phone() -> f32 {
    10.0
}

fn default_web_font_size_tablet() -> f32 {
    14.0
}

fn default_web_font_size_desktop() -> f32 {
    18.0
}

/// Type of remote client connected via WebSocket
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum RemoteClientType {
    /// Web browser client - receives full history, scrolls locally
    #[default]
    Web,
    /// Remote GUI client (egui) - receives full history, scrolls locally
    RemoteGUI,
    /// Remote console client (TUI) - receives screenful, requests scrollback from master
    RemoteConsole,
}


/// Information about a connected WebSocket client
pub struct WsClientInfo {
    pub authenticated: bool,
    pub tx: mpsc::UnboundedSender<WsMessage>,
    /// Which world this client is currently viewing (for activity indicator)
    pub current_world: Option<usize>,
    /// Username of the authenticated user (multiuser mode only)
    pub username: Option<String>,
    /// Whether the client has received its InitialState message
    /// Clients only receive broadcasts after getting InitialState to prevent duplicates
    pub received_initial_state: bool,
    /// Type of remote client (web, GUI, console) - affects output delivery
    pub client_type: RemoteClientType,
    /// Client's viewport height (for calculating screenful)
    pub viewport_height: usize,
    /// Max seq of pending lines merged into InitialState per world.
    /// Used to avoid sending duplicate lines when ReleasePending broadcasts ServerData.
    /// Cleared when client sends a command (indicating they've synced).
    pub pending_merged_max_seq: std::collections::HashMap<usize, u64>,
}

/// User credential for multiuser authentication
#[derive(Clone, Debug)]
pub struct UserCredential {
    pub password_hash: String,
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
    /// True if server is running in multiuser mode
    pub multiuser_mode: bool,
    /// User credentials for multiuser mode (username -> password_hash)
    pub users: Arc<std::sync::RwLock<HashMap<String, UserCredential>>>,
    /// Ban list for security (shared with HTTP server)
    pub ban_list: BanList,
    #[cfg(feature = "native-tls-backend")]
    pub tls_acceptor: Option<Arc<tokio_native_tls::TlsAcceptor>>,
    #[cfg(feature = "rustls-backend")]
    pub tls_acceptor: Option<Arc<tokio_rustls::TlsAcceptor>>,
}

impl WebSocketServer {
    pub fn new(password: &str, port: u16, allow_list: &str, whitelisted_host: Option<String>, multiuser_mode: bool, ban_list: BanList) -> Self {
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
            multiuser_mode,
            users: Arc::new(std::sync::RwLock::new(HashMap::new())),
            ban_list,
            #[cfg(feature = "native-tls-backend")]
            tls_acceptor: None,
            #[cfg(feature = "rustls-backend")]
            tls_acceptor: None,
        }
    }

    /// Add a user for multiuser authentication
    pub fn add_user(&self, username: &str, password: &str) {
        let password_hash = hash_password(password);
        let mut users = self.users.write().unwrap();
        users.insert(username.to_string(), UserCredential { password_hash });
    }

    /// Set the username for a connected client
    pub fn set_client_username(&self, client_id: u64, username: Option<String>) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.username = username;
            }
        }
    }

    /// Get the username of a connected client (multiuser mode)
    pub fn get_client_username(&self, client_id: u64) -> Option<String> {
        // Use try_read to avoid blocking in async context
        if let Ok(clients) = self.clients.try_read() {
            clients.get(&client_id).and_then(|c| c.username.clone())
        } else {
            None
        }
    }

    /// Clear a client's authentication state (for logout)
    pub fn clear_client_auth(&self, client_id: u64) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.authenticated = false;
                client.username = None;
            }
        }
    }

    /// Broadcast a message to all clients owned by a specific user
    pub fn broadcast_to_owner(&self, msg: WsMessage, owner: Option<&str>) {
        // Use try_read to avoid blocking in async context
        if let Ok(clients) = self.clients.try_read() {
            for client in clients.values() {
                if client.authenticated {
                    // In multiuser mode, only send to clients with matching username
                    if self.multiuser_mode {
                        if client.username.as_deref() == owner {
                            let _ = client.tx.send(msg.clone());
                        }
                    } else {
                        // In single-user mode, broadcast to all authenticated clients
                        let _ = client.tx.send(msg.clone());
                    }
                }
            }
        }
    }

    /// Broadcast a message to all authenticated clients (regardless of owner)
    /// Only sends to clients that have received their InitialState to prevent duplicates
    pub fn broadcast_to_all(&self, msg: WsMessage) {
        // Use try_read to avoid blocking in async context
        if let Ok(clients) = self.clients.try_read() {
            for client in clients.values() {
                // Only broadcast to clients that are authenticated AND have received InitialState
                // This prevents duplicate messages when a client connects while data is streaming
                if client.authenticated && client.received_initial_state {
                    let _ = client.tx.send(msg.clone());
                }
            }
        }
    }

    /// Mark a client as having received its InitialState
    /// After this, the client will receive broadcasts
    pub fn mark_initial_state_sent(&self, client_id: u64) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.received_initial_state = true;
            }
        }
    }

    /// Send a message to a specific client
    pub fn send_to_client(&self, client_id: u64, msg: WsMessage) {
        // Use try_read to avoid blocking in async context
        if let Ok(clients) = self.clients.try_read() {
            if let Some(client) = clients.get(&client_id) {
                let _ = client.tx.send(msg);
            }
        }
    }

    /// Get the current whitelisted host (for saving state)
    pub fn get_whitelisted_host(&self) -> Option<String> {
        self.whitelisted_host.read().unwrap().clone()
    }

    /// Set the client type for a connected client
    pub fn set_client_type(&self, client_id: u64, client_type: RemoteClientType) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.client_type = client_type;
            }
        }
    }

    /// Set the viewport height for a connected client
    pub fn set_client_viewport(&self, client_id: u64, height: usize) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.viewport_height = height;
            }
        }
    }

    /// Set the current world being viewed by a connected client
    pub fn set_client_world(&self, client_id: u64, world_index: Option<usize>) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.current_world = world_index;
            }
        }
    }

    /// Set the authenticated status for a connected client
    pub fn set_client_authenticated(&self, client_id: u64, authenticated: bool) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.authenticated = authenticated;
            }
        }
    }

    /// Get the client type for a connected client
    pub fn get_client_type(&self, client_id: u64) -> Option<RemoteClientType> {
        if let Ok(clients) = self.clients.try_read() {
            clients.get(&client_id).map(|c| c.client_type)
        } else {
            None
        }
    }

    /// Set the max pending seq that was merged into InitialState for a world.
    /// Used to avoid sending duplicate lines when ReleasePending broadcasts.
    pub fn set_pending_merged_seq(&self, client_id: u64, world_index: usize, max_seq: u64) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.pending_merged_max_seq.insert(world_index, max_seq);
            }
        }
    }

    /// Clear pending merged tracking for a client (when they've synced via command).
    pub fn clear_pending_merged(&self, client_id: u64) {
        if let Ok(mut clients) = self.clients.try_write() {
            if let Some(client) = clients.get_mut(&client_id) {
                client.pending_merged_max_seq.clear();
            }
        }
    }

    /// Check if a client should receive released pending lines for a world.
    /// Returns false if the lines (by max_seq) were already sent in InitialState.
    pub fn should_receive_released_lines(&self, client_id: u64, world_index: usize, max_seq: u64) -> bool {
        if let Ok(clients) = self.clients.try_read() {
            if let Some(client) = clients.get(&client_id) {
                if let Some(&merged_seq) = client.pending_merged_max_seq.get(&world_index) {
                    // Client already has lines up to merged_seq from InitialState
                    // Skip if the lines being released are within that range
                    return max_seq > merged_seq;
                }
            }
        }
        true  // No tracking info, should receive
    }

    /// Get the minimum viewport height across all clients viewing a specific world
    /// Returns None if no clients are viewing the world
    pub fn min_viewport_for_world(&self, world_index: usize) -> Option<usize> {
        if let Ok(clients) = self.clients.try_read() {
            let heights: Vec<usize> = clients.values()
                .filter(|c| c.authenticated && c.received_initial_state)
                .filter(|c| c.current_world == Some(world_index))
                .map(|c| c.viewport_height)
                .filter(|&h| h > 0)
                .collect();
            if heights.is_empty() {
                None
            } else {
                Some(*heights.iter().min().unwrap())
            }
        } else {
            None
        }
    }

    /// Get list of client IDs viewing a specific world
    pub fn clients_viewing_world(&self, world_index: usize) -> Vec<u64> {
        if let Ok(clients) = self.clients.try_read() {
            clients.iter()
                .filter(|(_, c)| c.authenticated && c.received_initial_state)
                .filter(|(_, c)| c.current_world == Some(world_index))
                .map(|(&id, _)| id)
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Broadcast a message to all authenticated clients (they filter by world_index client-side)
    /// This avoids race conditions where client switches world but server hasn't processed the update yet
    pub fn broadcast_to_world_viewers(&self, _world_index: usize, msg: WsMessage) {
        if let Ok(clients) = self.clients.try_read() {
            for client in clients.values() {
                if client.authenticated && client.received_initial_state {
                    let _ = client.tx.send(msg.clone());
                }
            }
        }
    }

    /// Broadcast released pending lines to clients viewing a world, but skip clients
    /// that already received those lines in their InitialState.
    pub fn broadcast_released_to_viewers(&self, world_index: usize, max_seq: u64, msg: WsMessage) {
        if let Ok(clients) = self.clients.try_read() {
            for client in clients.values() {
                if client.authenticated && client.received_initial_state {
                    // Check if client already has these lines from InitialState
                    if let Some(&merged_seq) = client.pending_merged_max_seq.get(&world_index) {
                        // Client has lines up to merged_seq, skip if max_seq <= merged_seq
                        if max_seq <= merged_seq {
                            continue;  // Skip - client already has these lines
                        }
                    }
                    let _ = client.tx.send(msg.clone());
                }
            }
        }
    }

    /// Broadcast PendingLinesUpdate, but skip clients that have pending_merged tracking
    /// for this world (they received those lines in InitialState and have correct count already).
    pub fn broadcast_pending_update(&self, world_index: usize, count: usize) {
        let msg = WsMessage::PendingLinesUpdate { world_index, count };
        if let Ok(clients) = self.clients.try_read() {
            for client in clients.values() {
                if client.authenticated && client.received_initial_state {
                    // Skip clients that have pending_merged for this world
                    // They received correct state in InitialState
                    if client.pending_merged_max_seq.contains_key(&world_index) {
                        continue;
                    }
                    let _ = client.tx.send(msg.clone());
                }
            }
        }
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
    let multiuser_mode = server.multiuser_mode;
    let users = server.users.clone();
    let ban_list = server.ban_list.clone();
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
                            // Check if IP is banned
                            let client_ip = client_addr.ip().to_string();
                            if ban_list.is_banned(&client_ip) {
                                // Silently drop connection for banned IPs
                                continue;
                            }

                            // Disable Nagle's algorithm for lower latency
                            let _ = stream.set_nodelay(true);

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
                            let multiuser_mode = multiuser_mode;
                            let users = users.clone();
                            let ban_list = ban_list.clone();
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
                                                multiuser_mode,
                                                users,
                                                ban_list,
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
                                    multiuser_mode,
                                    users,
                                    ban_list,
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
                                                multiuser_mode,
                                                users,
                                                ban_list,
                                            ).await {
                                                // Connection error, client disconnected
                                            }
                                        }
                                        Err(e) => {
                                            // TLS handshake failed - send to output area
                                            let msg = format!("WSS TLS handshake failed from {}: {}", client_addr, e);
                                            let _ = event_tx.send(AppEvent::SystemMessage(msg)).await;
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
                                    multiuser_mode,
                                    users,
                                    ban_list,
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
    multiuser_mode: bool,
    users: Arc<std::sync::RwLock<HashMap<String, UserCredential>>>,
    ban_list: BanList,
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

    // Check allow list - reject if not in allow list and not whitelisted
    let (allow_list_empty, in_allow_list) = {
        let allow_list_guard = allow_list.read().unwrap();
        let empty = allow_list_guard.is_empty();
        let in_list = is_ip_in_allow_list(&client_ip, &allow_list_guard);
        (empty, in_list)
    };

    // Reject connection if:
    // - Allow list is non-empty AND IP is not in list AND not whitelisted
    // (Empty allow list = allow everyone, they still need password to authenticate)
    if !allow_list_empty && !in_allow_list && !is_whitelisted {
        let msg = format!("WS connection rejected from {} (not in allow list)", client_addr);
        let _ = event_tx.send(AppEvent::SystemMessage(msg)).await;
        return Ok(());
    }

    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            return Err(e.into());
        }
    };
    let (mut ws_sink, mut ws_source) = ws_stream.split();

    // Create channel for sending messages to this client
    let (tx, mut rx) = mpsc::unbounded_channel::<WsMessage>();

    // Send ServerHello immediately to tell client about multiuser mode
    let _ = tx.send(WsMessage::ServerHello { multiuser_mode });

    // Add client to clients map (auto-authenticated if whitelisted)
    {
        let mut clients_guard = clients.write().await;
        clients_guard.insert(client_id, WsClientInfo {
            authenticated: is_whitelisted,
            tx: tx.clone(),
            current_world: None,
            username: None,
            received_initial_state: false,
            client_type: RemoteClientType::Web,  // Default, updated by ClientTypeDeclaration
            viewport_height: 24,  // Default, updated by UpdateViewState
            pending_merged_max_seq: std::collections::HashMap::new(),
        });
    }

    // Notify app of new connection
    let _ = event_tx.send(AppEvent::WsClientConnected(client_id)).await;

    // If auto-authenticated via whitelist, send success response and trigger initial state
    if is_whitelisted {
        let response = WsMessage::AuthResponse {
            success: true,
            error: None,
            username: None,
            multiuser_mode,
        };
        let _ = tx.send(response);
        // Create a fake AuthRequest to trigger initial state send
        let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(WsMessage::AuthRequest { username: None, password_hash: String::new(), current_world: None, auth_key: None, request_key: false }))).await;
    }

    // Spawn task to send messages from rx to WebSocket
    let clients_for_sender = Arc::clone(&clients);
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_sink.send(WsRawMessage::Text(json)).await.is_err() {
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
                        WsMessage::AuthRequest { username, password_hash: client_hash, auth_key, request_key, .. } => {
                            // Try auth_key first (device key authentication)
                            // auth_key validation must happen in the app since keys are stored there
                            // So we forward to app and let it respond
                            if auth_key.is_some() && !auth_key.as_ref().unwrap().is_empty() {
                                // Forward to app for key validation
                                // App will send AuthResponse directly
                                let _ = event_tx.send(AppEvent::WsAuthKeyValidation(client_id, Box::new(ws_msg.clone()))).await;
                                continue;
                            }

                            // Fall back to password-based authentication
                            let (auth_success, auth_error, auth_username) = if multiuser_mode {
                                // Multiuser mode: require username and validate against users map
                                match username {
                                    Some(uname) if !uname.is_empty() => {
                                        let users_guard = users.read().unwrap();
                                        if let Some(user_cred) = users_guard.get(uname) {
                                            if user_cred.password_hash == *client_hash {
                                                (true, None, Some(uname.clone()))
                                            } else {
                                                (false, Some("Invalid password".to_string()), None)
                                            }
                                        } else {
                                            (false, Some("Unknown user".to_string()), None)
                                        }
                                    }
                                    _ => (false, Some("Username required".to_string()), None),
                                }
                            } else {
                                // Single-user mode: just validate password
                                if *client_hash == password_hash {
                                    (true, None, None)
                                } else {
                                    (false, Some("Invalid password".to_string()), None)
                                }
                            };

                            if auth_success {
                                // Log successful auth
                                log_ws_auth(&client_ip, true, auth_username.as_deref());

                                // Mark as authenticated and set username
                                let mut clients_guard = clients.write().await;
                                if let Some(client) = clients_guard.get_mut(&client_id) {
                                    client.authenticated = true;
                                    client.username = auth_username.clone();
                                }

                                // If client IP is in allow list, whitelist this host (single-user mode only)
                                // This clears any previously whitelisted host
                                if !multiuser_mode {
                                    let in_allow_list = {
                                        let allow_list_guard = allow_list.read().unwrap();
                                        is_ip_in_allow_list(&client_ip, &allow_list_guard)
                                    };
                                    if in_allow_list {
                                        let mut whitelist = whitelisted_host.write().unwrap();
                                        *whitelist = Some(client_ip.clone());
                                    }
                                }
                            } else {
                                // Log failed auth
                                log_ws_auth(&client_ip, false, None);
                                // Record violation for failed auth attempt
                                ban_list.record_violation(&client_ip, "WebSocket: failed auth");
                            }
                            // Send auth response
                            let response = WsMessage::AuthResponse {
                                success: auth_success,
                                error: auth_error,
                                username: auth_username,
                                multiuser_mode,
                            };
                            let _ = tx.send(response);

                            if auth_success {
                                // Extract request_key before moving ws_msg
                                let wants_key = *request_key;

                                // Forward to app to send initial state (and generate key if requested)
                                let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(ws_msg))).await;

                                // If client requested a key, forward that info to app
                                if wants_key {
                                    let _ = event_tx.send(AppEvent::WsKeyRequest(client_id)).await;
                                }
                            }
                        }
                        WsMessage::RevokeKey { auth_key } => {
                            // Forward key revocation to app
                            let _ = event_tx.send(AppEvent::WsKeyRevoke(client_id, auth_key.clone())).await;
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
                            } else {
                                // Unauthenticated client trying to send non-auth messages - disconnect but don't ban
                                break;
                            }
                        }
                    }
                } else {
                    // Invalid JSON - disconnect but don't ban
                    break;
                }
            }
            Ok(WsRawMessage::Close(_)) => {
                break;
            }
            Ok(WsRawMessage::Ping(data)) => {
                // Pong is handled automatically by tungstenite
                let _ = data;
            }
            Ok(WsRawMessage::Binary(_)) => {
                // Binary messages not supported - disconnect but don't ban
                break;
            }
            Err(_) => {
                // Protocol error - disconnect but don't ban (could be network issues)
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
