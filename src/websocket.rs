use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::net::TcpListener;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use tokio_tungstenite::{accept_async_with_config, tungstenite::Message as WsRawMessage, tungstenite::protocol::WebSocketConfig};

// Import AppEvent and Action from the main crate
use crate::{AppEvent, Action, BanList, debug_log, is_debug_enabled};
use crate::ansi_music::MusicNote;
use crate::http::log_ws_auth;

/// Seconds an unauthenticated WebSocket client has to complete authentication before being dropped.
const WS_AUTH_TIMEOUT_SECS: u64 = 30;
/// Seconds of inactivity after which an authenticated client receives a keepalive Ping.
const WS_KEEPALIVE_INTERVAL_SECS: u64 = 60;
/// Seconds to wait for a Pong response before treating the connection as dead and dropping it.
const WS_PONG_TIMEOUT_SECS: u64 = 20;

/// Bounded capacity of each client's outbound `WsMessage` channel (PROTOCOL-ROADMAP.md
/// Step 3). Previously `mpsc::unbounded_channel`, which let a stalled/slow client (a
/// suspended mobile WebView, a congested SSH tunnel) accumulate server-side memory without
/// bound and with no way to detect it was falling behind.
///
/// Sized against realistic legitimate bursts without becoming a giant hidden buffer that
/// just moves the "client can't keep up" problem further out instead of surfacing it:
///   - `ServerData`: up to one message per socket read per busy world, fanned out to every
///     connected client (`broadcast_to_all`/`broadcast_to_owner`). A fast-scrolling combat
///     or a `look` in a big room can emit several reads in a single tick; a handful of
///     simultaneously-busy worlds could plausibly emit a few dozen `ServerData` messages in
///     under a second.
///   - `PendingCountUpdate`: one per world roughly every 2 seconds — negligible.
///   - `InitialState`/`ScrollbackLines`: large but singular, sent once per connect/resume,
///     not part of a steady-state burst.
///
/// 256 gives several seconds of headroom at typical MUD output rates before a client is
/// considered stuck, while capping worst-case per-client memory to a few hundred queued
/// `WsMessage` clones (each typically well under 1 KiB for `ServerData`) — far short of the
/// 2 MiB `max_message_size` that already bounds a single outbound frame (see `ws_config`
/// below). A client that can't drain 256 messages' worth of backlog is meaningfully behind,
/// not just briefly bursty, and is exactly the case Step 3 wants to detect via
/// `ResyncRequired` rather than let grow forever.
pub(crate) const WS_CLIENT_CHANNEL_CAPACITY: usize = 256;

/// Extract the `world_index` a `WsMessage` pertains to, if any (PROTOCOL-ROADMAP.md Step
/// 3). Used to target a dropped-on-overflow message's `ResyncRequired` at the right world;
/// messages with no single world (control/global messages) return `None` and are simply
/// logged as dropped with no resync target — losing e.g. an `ActivityUpdate` doesn't
/// corrupt any world's `seq` stream the way losing a `ServerData` would.
pub(crate) fn message_world_index(msg: &WsMessage) -> Option<usize> {
    match msg {
        WsMessage::ServerData { world_index, .. }
        | WsMessage::WorldConnected { world_index, .. }
        | WsMessage::WorldDisconnected { world_index, .. }
        | WsMessage::WorldCreated { world_index, .. }
        | WsMessage::WorldRemoved { world_index, .. }
        | WsMessage::PromptUpdate { world_index, .. }
        | WsMessage::PendingLinesUpdate { world_index, .. }
        | WsMessage::PendingReleased { world_index, .. }
        | WsMessage::UnseenCleared { world_index, .. }
        | WsMessage::UnseenUpdate { world_index, .. }
        | WsMessage::NewWatermark { world_index, .. }
        | WsMessage::WorldFlushed { world_index, .. }
        | WsMessage::ServerSpeak { world_index, .. }
        | WsMessage::AnsiMusic { world_index, .. }
        | WsMessage::GmcpData { world_index, .. }
        | WsMessage::MsdpData { world_index, .. }
        | WsMessage::McmpMedia { world_index, .. }
        | WsMessage::GmcpUserToggled { world_index, .. }
        | WsMessage::CertMismatch { world_index, .. }
        | WsMessage::WorldSettingsUpdated { world_index, .. }
        | WsMessage::PendingCountUpdate { world_index, .. }
        | WsMessage::ScrollbackLines { world_index, .. }
        | WsMessage::WorldSwitchResult { world_index, .. }
        | WsMessage::OutputLines { world_index, .. }
        | WsMessage::NoteEditorState { world_index, .. }
        | WsMessage::NotesChanged { world_index, .. }
        | WsMessage::ResyncRequired { world_index, .. } => Some(*world_index),
        _ => None,
    }
}

/// Item type for a client's outbound channel (PROTOCOL-ROADMAP.md Step 8). Before this
/// step, every broadcast fan-out (`broadcast_to_owner`/`broadcast_to_all`/
/// `broadcast_to_world_viewers`) sent a `WsMessage` clone into every recipient's channel,
/// and each client's own `handle_ws_client` receive loop independently ran
/// `serde_json::to_string` on its clone — so one broadcast to N clients did N identical
/// JSON serializations of the same data. `Outbound` lets a broadcast call site serialize
/// once and hand every recipient a cheap `Arc<str>` clone of the result instead.
///
/// - `Shared`: pre-serialized JSON for a message whose content is identical for every
///   recipient (the broadcast case). The receive loop sends it straight through with no
///   further serialization.
/// - `Message`: a `WsMessage` serialized individually by the receiving client's own task,
///   unchanged from before this step. Used for anything genuinely per-client — content
///   that differs per recipient (e.g. `ActivityUpdate`'s per-client-excluded count,
///   `ResyncRequired`'s per-client `from_seq`) or that only ever has one recipient
///   (`InitialState`, auth replies, `ScrollbackLines` replay, `ServerHello`).
///
/// Both variants flow through the same bounded `mpsc::Sender`/`Receiver<Outbound>`, so
/// FIFO ordering between a `Shared` broadcast and a `Message` per-client send to the same
/// client is preserved exactly as it was when the channel only ever carried `WsMessage`.
#[derive(Clone, Debug)]
pub(crate) enum Outbound {
    /// Pre-serialized JSON, shared via `Arc<str>` across every recipient of one broadcast.
    Shared(std::sync::Arc<str>),
    /// A message to be serialized individually in the receiving client's own task.
    Message(Box<WsMessage>),
}

/// Serialize a `WsMessage` once for a broadcast (PROTOCOL-ROADMAP.md Step 8), returning the
/// shared JSON as an `Outbound::Shared`. On a serialization failure, logs
/// `WS-SERIALIZE-FAIL` exactly once (not once per recipient — before this step, every
/// client's receive loop ran the same failing `serde_json::to_string` independently and
/// logged its own copy; broadcasting now serializes once up front, so a failure here means
/// nobody gets the message, which is the same net effect with one log line instead of N)
/// and returns `None` so the caller sends to nobody, matching the old per-client "drop
/// silently past the log" behavior.
pub(crate) fn serialize_for_broadcast(msg: &WsMessage) -> Option<std::sync::Arc<str>> {
    match serde_json::to_string(msg) {
        Ok(json) => Some(std::sync::Arc::from(json.as_str())),
        Err(e) => {
            let debug_str = format!("{:?}", msg);
            let variant = debug_str.split(['{', '(']).next().unwrap_or(&debug_str).trim();
            crate::http::log_remote_event("WS-SERIALIZE-FAIL", "broadcast",
                &format!("variant={}: {}", variant, e));
            debug_log(true, &format!("WS-SERIALIZE-FAIL: broadcast variant={}: {}", variant, e));
            None
        }
    }
}

/// Shared PROTOCOL-ROADMAP.md Step 3 bookkeeping for every send site (the `WebSocketServer`
/// fan-out methods below, and `handle_ws_client`'s own local sends and drain-triggered
/// retry). `full` are client ids whose channel just rejected a `world_index`-bearing message
/// with `TrySendError::Full`; `flush_candidates` are client ids that just had a *successful*
/// send go through while still flagged `needs_resync` for this world, i.e. evidence the
/// channel now has room to retry the `ResyncRequired` that couldn't be delivered earlier.
/// For each id, attempts exactly one `try_send(ResyncRequired)`: on success clears the flag,
/// on failure (re)sets it so a later call (the next overflow, or the next successful send to
/// that client) gets another chance. No-op when `world_index` is `None` — a message with no
/// single world can't be tied to a resync target. `pub(crate)` — also called directly from
/// `main.rs`'s single-user-path App methods (`ws_broadcast`/`ws_send_to_client`/
/// `ws_send_initial_state_and_mark`/`broadcast_activity`), which touch `WsClientInfo.tx`
/// directly rather than going through the `WebSocketServer` fan-out methods below.
pub(crate) fn reconcile_resync(
    clients: &std::sync::RwLock<HashMap<u64, WsClientInfo>>,
    world_index: Option<usize>,
    full: &[u64],
    flush_candidates: &[u64],
) {
    let Some(wi) = world_index else { return };
    if full.is_empty() && flush_candidates.is_empty() {
        return;
    }
    let mut guard = clients.write().unwrap();
    for &id in full.iter().chain(flush_candidates.iter()) {
        if let Some(client) = guard.get_mut(&id) {
            let from_seq = client.acked_seq.get(&wi).copied().unwrap_or(0);
            // ResyncRequired's `from_seq` is per-client (keyed off that client's own
            // acked_seq), so it's always a `Message`, never `Shared` (PROTOCOL-ROADMAP.md
            // Step 8) - there is nothing to share across recipients here.
            match client.tx.try_send(Outbound::Message(Box::new(WsMessage::ResyncRequired { world_index: wi, from_seq }))) {
                Ok(()) => {
                    client.needs_resync.remove(&wi);
                }
                Err(_) => {
                    client.needs_resync.insert(wi);
                }
            }
        }
    }
}

/// Non-blocking send for `handle_ws_client`'s own local response sends (ServerHello,
/// AuthResponse, Pong) — mirrors the `WebSocketServer` fan-out methods' Step 3 handling: a
/// full channel is logged via `WS-CHANNEL-FULL` and, in the unlikely case the message does
/// carry a `world_index`, reconciled through `reconcile_resync` the same way. These
/// particular messages never carry one today, but the helper stays generic rather than
/// silently dropping a future world-scoped message added here without this handling.
fn try_send_local(
    clients: &std::sync::RwLock<HashMap<u64, WsClientInfo>>,
    client_id: u64,
    tx: &mpsc::Sender<Outbound>,
    ip: &str,
    msg: WsMessage,
) {
    let world_index = message_world_index(&msg);
    // These are all genuinely per-client (or one-shot pre-auth) sends - Message, not
    // Shared (PROTOCOL-ROADMAP.md Step 8).
    match tx.try_send(Outbound::Message(Box::new(msg))) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Closed(_)) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            crate::http::log_remote_event("WS-CHANNEL-FULL", ip,
                &format!("client={} world={:?} outbound channel full (capacity {}) - message dropped",
                    client_id, world_index, WS_CLIENT_CHANNEL_CAPACITY));
            reconcile_resync(clients, world_index, &[client_id], &[]);
        }
    }
}

// ============================================================================
// WebSocket Protocol Types
// ============================================================================

/// Default function for serde to return true (for from_server field backwards compatibility)
fn default_true() -> bool { true }
fn is_false(v: &bool) -> bool { !v }
fn is_true(v: &bool) -> bool { *v }

/// Default for the ▶ window's upper bound (`World::viewed_from_seq` in main.rs) when absent
/// from an older peer's message. A bare `#[serde(default)]` on a `u64` yields `0`, which
/// means "exclude every line" - the exact opposite of the intended "no viewing episode in
/// progress, no exclusion" sentinel.
fn default_u64_max() -> u64 { u64::MAX }

/// WebSocket protocol messages for client-server communication
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum WsMessage {
    // Server hello (sent immediately on connection, before auth)
    ServerHello {
        multiuser_mode: bool,  // True if server requires username + password
        #[serde(default)]
        challenge: String,     // Random challenge for challenge-response auth
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
        #[serde(default)]
        challenge_response: bool,  // If true, password_hash is SHA256(SHA256(password) + challenge)
        #[serde(default)]
        resume: Vec<(usize, u64)>,  // Per-world (world_index, last_contiguous_seq) the client already
                                     // has, so the server can replay exactly the gap on reconnect.
                                     // See PROTOCOL-ROADMAP.md Step 2 (single-user) and Step 6a
                                     // (multiuser, owner-scoped).
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
    // Client requests auth key regeneration (from web settings UI)
    RegenerateAuthKey,

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
        #[serde(default)]
        server_version: String,
    },

    // Real-time updates (server -> client)
    /// is_viewed: true if any interface (console/web/GUI) is viewing this world
    /// ts: timestamp in seconds since Unix epoch (when the line was received)
    /// from_server: true if data came from MUD server, false if client-generated
    /// end_seq: the seq of the LAST line in this batch, when the sender actually knows it
    /// (i.e. `data` was built from a contiguous slice of `output_lines`/`pending_lines` that
    /// really did consume seqs `seq..=end_seq`). `Option`, not a trimmed `u64`: seq 0 is a
    /// real value (a world's first line), so "field absent" must stay distinguishable from
    /// "value 0", the same reason `seq`/`ts` are never `skip_serializing_if`-trimmed. Lets a
    /// client derive a batch's true line count/span without re-deriving it by counting
    /// filtered lines locally (see PROTOCOL-ROADMAP.md's seq-drift fix) - the sender always
    /// knows the true count; the receiver may not, once it applies its own line filters
    /// (ANSI-only lines, idler markers, grep mode).
    ServerData { world_index: usize, data: String, is_viewed: bool, #[serde(default)] ts: u64, #[serde(default = "default_true", skip_serializing_if = "is_true")] from_server: bool, #[serde(default)] seq: u64, #[serde(default, skip_serializing_if = "Option::is_none")] end_seq: Option<u64>, #[serde(default, skip_serializing_if = "is_false")] flush: bool, #[serde(default, skip_serializing_if = "is_false")] gagged: bool },
    WorldConnected { world_index: usize, name: String },
    WorldDisconnected { world_index: usize },
    WorldAdded { world: Box<WorldStateMsg> },
    /// Response to CreateWorld - tells the requesting client the index of the new world
    WorldCreated { world_index: usize },
    WorldRemoved { world_index: usize },
    WorldSwitched { new_index: usize },
    PromptUpdate { world_index: usize, prompt: String },
    PendingLinesUpdate { world_index: usize, count: usize },
    /// Broadcast when pending lines are released (by any interface)
    PendingReleased { world_index: usize, count: usize },
    UnseenCleared { world_index: usize },
    UnseenUpdate { world_index: usize, count: usize },
    /// New-text (▶) watermark pair for a world changed: a line is ▶ iff `seq >= new_from_seq
    /// && seq < viewed_from_seq`. Broadcast to ALL clients (not just viewers of that world)
    /// whenever either bound changes - live arrival on a viewed/unviewed world, leaving a
    /// world, Ctrl+L, or MarkWorldSeen - so every instance's copy stays authoritative without
    /// ever storing (and risking staleness on) a per-line flag. See `World::new_from_seq`'s
    /// and `World::viewed_from_seq`'s doc comments in main.rs for the full model.
    NewWatermark {
        world_index: usize,
        new_from_seq: u64,
        /// Upper bound of the ▶ window - see `World::viewed_from_seq` in main.rs. Absent from
        /// an older peer's message, hence the explicit `u64::MAX` ("no exclusion") default.
        #[serde(default = "default_u64_max")]
        viewed_from_seq: u64,
    },
    /// Broadcast server's activity count (number of worlds with activity)
    ActivityUpdate { count: usize },
    /// Sent to a specific client when its server-side pause state changes
    PausedState { paused: bool },
    /// Broadcast when show_tags setting changes (F2 or /tag command)
    ShowTagsChanged { show_tags: bool },
    /// Server is about to reload - clients should auto-reconnect
    ServerReloading,
    /// Clear all output for a world (from /flush command)
    WorldFlushed { world_index: usize },
    /// Tell client to execute a command locally (for action commands like /worlds)
    ExecuteLocalCommand { command: String },
    /// Tell client to open a new window (browser tab), optionally locked to a world
    OpenWindow { world: Option<String> },
    /// Set client's input buffer (server -> client, for API lookup results like /dict, /urban, /translate)
    SetInputBuffer { text: String, #[serde(default)] cursor_start: bool },

    /// Notification for mobile clients (server -> client)
    Notification { title: String, message: String },

    /// Text-to-speech: speak text aloud on client (server -> client)
    /// Console uses espeak/say subprocess; web/Android uses Web Speech API
    ServerSpeak { text: String, world_index: usize },

    /// ANSI Music sequence to play (server -> client)
    AnsiMusic { world_index: usize, notes: Vec<MusicNote> },

    /// GMCP data received from MUD server (server -> client)
    GmcpData { world_index: usize, package: String, data: String },
    /// MSDP variable update from MUD server (server -> client)
    MsdpData { world_index: usize, variable: String, value: String },
    /// MCMP media action (server -> client, specialized for Client.Media.*)
    McmpMedia { world_index: usize, action: String, data: String, default_url: String },
    /// GMCP user toggle state changed (server -> client broadcast)
    GmcpUserToggled { world_index: usize, enabled: bool },

    // Commands (client -> server)
    /// Toggle GMCP user-enabled for a world (client -> server)
    ToggleWorldGmcp { world_index: usize },
    /// Send GMCP message to MUD server (client -> server)
    SendGmcp { world_index: usize, package: String, data: String },
    /// Send MSDP message to MUD server (client -> server)
    SendMsdp { world_index: usize, variable: String, value: String },
    SendCommand { world_index: usize, command: String },
    SwitchWorld { world_index: usize },
    ConnectWorld { world_index: usize },
    /// Server -> client: a MUD world's TLS certificate no longer matches the
    /// trust-on-first-use pin recorded in ~/.clay/known_hosts.dat. The connection
    /// was blocked; the client should show old/new fingerprints and offer a
    /// "Trust new certificate" action that replies with TrustCertificate.
    CertMismatch { world_index: usize, host: String, old_fingerprint: String, new_fingerprint: String },
    /// Client -> server: user explicitly accepted a changed certificate after a
    /// CertMismatch warning. Server re-pins the fingerprint and reconnects.
    TrustCertificate { world_index: usize, host: String, new_fingerprint: String },
    DisconnectWorld { world_index: usize },
    DeleteWorld { world_index: usize },
    CreateWorld { name: String },
    /// Request to release pending lines (count = number to release, 0 = all)
    ReleasePending { world_index: usize, count: usize },
    /// Selective flush: release only highlighted pending lines, discard rest
    SelectiveFlush { world_index: usize },
    /// `previous_world_index` is the world the client is leaving, so its `marked_new`
    /// indicators can be cleared even after a reconnect (new `client_id`) that lost the
    /// server's `ws_client_worlds` tracking of the client's prior world. Optional/defaulted
    /// for wire compat with older clients that don't send it (falls back to the
    /// `ws_client_worlds` lookup).
    MarkWorldSeen { world_index: usize, #[serde(default)] previous_world_index: Option<usize> },
    /// Update client's view state (world index and visible lines for more-mode calculation)
    UpdateViewState { world_index: usize, visible_lines: usize, #[serde(default)] visible_columns: Option<usize> },
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
        #[serde(default)]
        gmcp_packages: String,
        #[serde(default)]
        auto_reconnect_secs: String,
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
        #[serde(default)]
        wrapspace: u8,
        #[serde(default = "default_remote_initial_lines")]
        remote_initial_lines: u16,
        input_height: u16,
        font_name: String,
        font_size: f32,
        web_font_size_phone: f32,
        web_font_size_tablet: f32,
        web_font_size_desktop: f32,
        #[serde(default = "default_web_font_weight")]
        web_font_weight: u16,
        #[serde(default = "default_web_font_line_height")]
        web_font_line_height: f32,
        #[serde(default)]
        web_font_letter_spacing: f32,
        #[serde(default)]
        web_font_word_spacing: f32,
        ws_allow_list: String,
        web_secure: bool,
        http_enabled: bool,
        http_port: u16,
        #[serde(default = "default_web_path")]
        web_path: String,
        #[serde(default)]
        ws_enabled: bool,  // Legacy — ignored, kept for backward compat
        #[serde(default)]
        ws_port: u16,      // Legacy — ignored, kept for backward compat
        ws_cert_file: String,
        ws_key_file: String,
        #[serde(default)]
        ws_password: String,
        tls_proxy_enabled: bool,
        #[serde(default)]
        dictionary_path: String,
        #[serde(default)]
        mouse_enabled: bool,
        #[serde(default)]
        zwj_enabled: bool,
        #[serde(default)]
        new_line_indicator: bool,
        #[serde(default)]
        tts_mode: String,
        #[serde(default)]
        tts_speak_mode: String,
        #[serde(default)]
        scrollback_enabled: bool,
        /// See `Settings::log_input_enabled`'s doc comment in main.rs.
        #[serde(default)]
        log_input_enabled: bool,
        #[serde(default = "default_keyboard_always_visible")]
        keyboard_always_visible: bool,
        #[serde(default)]
        tabs: String,
        #[serde(default)]
        icon_bar: String,
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

    /// Report a duplicate ServerData detected by a remote client (client -> server)
    ReportDuplicate {
        world_index: usize,
        line_seq: u64,
        max_seq: u64,
        line_text: String,
        source: String,  // "web", "gui", "android", "console"
    },

    /// Report that a client recovered an out-of-order ServerData batch that overlapped a
    /// gap it had recorded, instead of treating it as a duplicate (client -> server). See
    /// app.js's insertLinesBySeq/findOverlappingSeqGap (D-Termux-lines investigation).
    ReportOutOfOrder {
        world_index: usize,
        line_seq: u64,
        recovered_count: usize,
        source: String,  // "web", "gui", "android", "console"
    },

    // Remote instance handling (client -> server)
    /// Client declares its type on connection (affects output delivery)
    ClientTypeDeclaration { client_type: RemoteClientType },
    /// Request to cycle to next/previous world (master applies switching rules)
    CycleWorld { direction: String },  // "up" or "down"
    /// Request scrollback lines from master (console clients only)
    /// before_seq: oldest sequence number the client has (server sends lines with seq < before_seq)
    /// after_seq: newest sequence number the client already has (server sends lines with
    /// seq > after_seq) - used for the reconnect gap-fill path, where the client kept its
    /// buffer across the reconnect and only wants what accumulated while it was away.
    /// At most one of before_seq/after_seq should be set; before_seq takes precedence.
    /// request_id: client-chosen correlator, echoed back on the matching `ScrollbackLines`
    /// reply (seq-drift fix, PROTOCOL-ROADMAP.md follow-on). `#[serde(default)]` so an old
    /// client omitting it is unaffected; the server just echoes back whatever it received
    /// (including nothing, via the `Option`'s own default).
    RequestScrollback {
        world_index: usize,
        count: usize,
        #[serde(default)] before_seq: Option<u64>,
        #[serde(default)] after_seq: Option<u64>,
        #[serde(default)] request_id: Option<u64>,
    },

    // Remote instance handling (server -> client)
    /// Batch of output lines for a world (initial or incremental)
    OutputLines {
        world_index: usize,
        lines: Vec<TimestampedLine>,
        is_initial: bool,  // True for initial load or world switch
    },
    /// Periodic pending count update (sent every 2 seconds when pending count changes)
    PendingCountUpdate { world_index: usize, count: usize },
    /// Response to RequestScrollback with historical lines. request_id echoes the
    /// originating `RequestScrollback.request_id` when the reply was solicited by one; the
    /// RESERVED value `Some(0)` marks a server-initiated UNPROMPTED resume replay (the
    /// `AuthRequest.resume` path, `App::handle_ws_auth_initial_state`) - see that path's
    /// callers. `None` means either an old server that predates this field, or (defensively)
    /// a reply the server couldn't correlate to any specific request.
    ScrollbackLines { world_index: usize, lines: Vec<TimestampedLine>, #[serde(default)] backfill_complete: bool, #[serde(default)] request_id: Option<u64> },
    /// World switch result with appropriate initial data
    WorldSwitchResult {
        world_index: usize,
        world_name: String,
        pending_count: usize,
        paused: bool,
    },

    // Settings import (client -> local server): /import host[:port] downloads
    // settings.dat/theme.dat/keybindings.dat from another Clay instance and merges
    // them in (remote wins on conflicts). See plan `i-d-like-to-make-snuggly-rain.md`.
    /// Client -> local server: start an import from `addr`. Collected client-side because
    /// the password/auth-key must never be sent as a bounced /import command line.
    ImportSettings { addr: String, password: Option<String>, auth_key: Option<String>, allow_insecure: bool },
    /// Local server -> client: the target has no TLS and `allow_insecure` was false.
    /// Client should show an explicit "passwords will be sent unencrypted" confirmation
    /// and, if accepted, resend ImportSettings with allow_insecure: true.
    ImportNeedsInsecureConfirm { addr: String },
    /// Local server -> client: final outcome of an import attempt.
    ImportResult { success: bool, summary: String },
    /// Importer -> target, sent over the outbound connection opened for the import: request
    /// the target's settings/theme/keybindings with all secrets decrypted.
    RequestSettingsExport,
    /// Target -> importer: raw file contents (secrets decrypted; importer re-encrypts under
    /// its own local machine key before saving).
    SettingsExport { settings_dat: String, theme_dat: String, keybindings_dat: String },

    // Theme editor (client -> server)
    RequestThemeEditorState,
    UpdateThemeColors { theme_name: String, colors_json: String },
    AddTheme { name: String, copy_from: String },
    DeleteTheme { name: String },
    SaveThemeFile,

    // Theme editor (server -> client)
    ThemeEditorState { themes_json: String, theme_names: Vec<String>, active_theme: String },
    ThemeFileSaved { success: bool, error: Option<String> },
    ThemeCssVarsUpdated { css_vars: String, colors_json: String },

    // Action editor standalone page (client -> server)
    RequestActionEditorState,

    // Action editor standalone page (server -> client)
    ActionEditorState { actions_json: String, world_names_json: String },

    // Keybind editor (client -> server)
    RequestKeybindEditorState,
    UpdateKeybindEditorBindings { bindings_json: String },
    SaveKeybindFile,
    ResetKeybindDefaults,

    // Keybind editor (server -> client)
    KeybindEditorState { bindings_json: String, defaults_json: String, actions_json: String },
    KeybindFileSaved { success: bool, error: Option<String> },

    // Keybindings update (server -> all clients)
    KeybindingsUpdated { bindings_json: String },

    // Note editor — opens as its own native OS window (webview-GUI) or a
    // separate browser tab (web), not a shell-out or an in-page modal. See
    // NOTE_MODE handling in web/app.js and WvEvent::NoteWindow in
    // webview_gui.rs. Backed by the same world.settings.notes the console
    // TUI's split-screen /note editor already uses.
    // (client -> server)
    RequestNoteEditorState { world_index: usize },
    UpdateNote { world_index: usize, notes: String },

    // Note editor (server -> client)
    NoteEditorState { world_index: usize, world_name: String, notes: String },

    // Broadcast whenever a world's notes go from empty to non-empty or back
    // (console /note edits, or a GUI/web note-editor Save) — lets any client
    // viewing that world update its has_notes indicator live. Deliberately a
    // narrow, single-field message rather than reusing WorldSettingsUpdated:
    // WorldSettingsUpdated has no client handler today and always carries an
    // empty password, so writing its first handler risks clobbering the
    // world editor's locally-cached plaintext password.
    NotesChanged { world_index: usize, has_notes: bool },

    // Keepalive
    Ping,
    Pong,

    // Liveness check for /remote command (server -> client -> server)
    PingCheck { nonce: u64 },
    /// PongCheck also piggybacks the client's per-world delivery ack: `acked` is
    /// (world_index, last_contiguous_seq) for each world the client has received
    /// `ServerData` for, i.e. the highest seq such that every seq up to and including
    /// it has been seen with no gap. Lets the server replay exactly the missing range
    /// on reconnect instead of the client guessing. See PROTOCOL-ROADMAP.md (not yet
    /// wired up — Step 1 is schema only).
    PongCheck { nonce: u64, #[serde(default)] acked: Vec<(usize, u64)> },

    /// Server -> client: this world's stream has a gap the server can't (or won't)
    /// silently patch — e.g. the client's outbound queue overflowed and messages were
    /// dropped (see PROTOCOL-ROADMAP.md Step 3). `from_seq` is the seq the client
    /// should request via `RequestScrollback { after_seq: Some(from_seq), .. }` to
    /// resync. Not yet sent by anything — Step 1 is schema only.
    ResyncRequired { world_index: usize, from_seq: u64 },
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
    #[serde(default)]
    pub from_archive: bool, // true if line was loaded from the scrollback.db archive
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
    // Whether GMCP user processing is enabled (F9 toggle)
    #[serde(default)]
    pub gmcp_user_enabled: bool,
    // Total number of output lines on the server (for lazy backfill)
    #[serde(default)]
    pub total_output_lines: usize,
    // Total number of VISIBLE (non-gagged) output lines on the server - the download budget
    // (Remote Lines) is spent only on visible lines, so the client needs this (not
    // total_output_lines, which includes gagged lines it can't see) to know how much visible
    // history genuinely remains to fetch. `Option`, not a bare `usize`: 0 is a real value (a
    // world with no visible lines yet), so "field absent" (an older server that predates this)
    // must stay distinguishable from "genuinely zero" - the same reasoning `end_seq` uses.
    // app.js falls back to total_output_lines when this is `None`.
    #[serde(default)]
    pub total_visible_lines: Option<usize>,
    // Number of pending lines on the server (for More indicator on connect)
    #[serde(default)]
    pub pending_count: usize,
    /// New-text (▶) watermark at connect/reconnect time — see `WsMessage::NewWatermark`'s
    /// doc comment for the model. A client renders ▶ on any `from_server` line with
    /// `seq >= new_from_seq && seq < viewed_from_seq`.
    #[serde(default)]
    pub new_from_seq: u64,
    /// Upper bound of the ▶ window at connect/reconnect time - see `World::viewed_from_seq`
    /// in main.rs. `u64::MAX` means "no viewing episode in progress".
    #[serde(default = "default_u64_max")]
    pub viewed_from_seq: u64,
    /// The server-authoritative "highest seq issued so far this process" counter
    /// (`World::next_seq`) at connect/reconnect time. Lets a reconnecting client detect a
    /// server restart: seq counters reset to 0 on every fresh process start (only the
    /// hot-reload state file persists them across a `/reload`, not a real restart), so if a
    /// client's cached/in-memory buffer for this world claims a max seq >= this value, that
    /// buffer predates the current server session and must not be trusted for dedup/hydration
    /// (see the InitialState handler in app.js). A value of zero is a legitimate real value
    /// for a freshly-created world, so this is unconditionally serialized like `seq`/`ts`
    /// elsewhere in this file, not trimmed via `skip_serializing_if`.
    #[serde(default)]
    pub next_seq: u64,
}

/// World settings for WebSocket protocol
/// Password is sent as plaintext to authenticated clients (stored encrypted in .dat file).
/// has_password mirrors whether the password field is non-empty.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorldSettingsMsg {
    pub hostname: String,
    pub port: String,
    pub user: String,
    #[serde(default)]
    pub password: String,  // Empty from server; encrypted when client sends updates
    pub use_ssl: bool,
    pub log_enabled: bool,
    pub encoding: String,
    pub auto_connect_type: String,
    pub keep_alive_type: String,
    pub keep_alive_cmd: String,
    #[serde(default)]
    pub gmcp_packages: String,
    #[serde(default)]
    pub has_password: bool,  // True if a password is configured (password field is empty)
    #[serde(default)]
    pub auto_reconnect_secs: String,
    /// Mirrors whether world.settings.notes is non-empty, same idea as
    /// has_password — the note text itself is never sent here (see
    /// NoteEditorState, fetched on demand when the note editor opens).
    #[serde(default)]
    pub has_notes: bool,
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
    #[serde(default)]
    pub wrapspace: u8,
    /// Number of visible (non-gagged) lines sent to a remote/web/GUI client per world
    /// on initial connect. See App::build_initial_state.
    #[serde(default = "default_remote_initial_lines")]
    pub remote_initial_lines: u16,
    pub input_height: u16,
    pub font_name: String,
    pub font_size: f32,
    #[serde(default = "default_web_font_size_phone")]
    pub web_font_size_phone: f32,
    #[serde(default = "default_web_font_size_tablet")]
    pub web_font_size_tablet: f32,
    #[serde(default = "default_web_font_size_desktop")]
    pub web_font_size_desktop: f32,
    #[serde(default = "default_web_font_weight")]
    pub web_font_weight: u16,
    #[serde(default = "default_web_font_line_height")]
    pub web_font_line_height: f32,
    #[serde(default)]
    pub web_font_letter_spacing: f32,
    #[serde(default)]
    pub web_font_word_spacing: f32,
    pub ws_allow_list: String,
    pub web_secure: bool,
    pub http_enabled: bool,
    pub http_port: u16,
    /// Stealth path prefix for the web UI (default "clay"; empty = legacy mode at "/")
    #[serde(default = "default_web_path")]
    pub web_path: String,
    #[serde(default)]
    pub ws_enabled: bool,  // Legacy — ignored, kept for backward compat
    #[serde(default)]
    pub ws_port: u16,      // Legacy — ignored, kept for backward compat
    pub ws_cert_file: String,  // Empty from server; populated when client sends updates
    pub ws_key_file: String,   // Empty from server; populated when client sends updates
    #[serde(default)]
    pub tls_configured: bool,  // True if TLS cert+key are configured
    #[serde(default)]
    pub tls_proxy_enabled: bool,
    #[serde(default)]
    pub dictionary_path: String,
    #[serde(default)]
    pub mouse_enabled: bool,
    #[serde(default)]
    pub zwj_enabled: bool,
    #[serde(default)]
    pub new_line_indicator: bool,
    #[serde(default)]
    pub tts_mode: String,
    #[serde(default)]
    pub tts_speak_mode: String,
    #[serde(default)]
    pub scrollback_enabled: bool,
    /// See `Settings::log_input_enabled`'s doc comment in main.rs.
    #[serde(default)]
    pub log_input_enabled: bool,
    /// Force the on-screen keyboard visible on phone/tablet web clients (Android
    /// app included); ignored when a hardware keyboard is attached.
    #[serde(default = "default_keyboard_always_visible")]
    pub keyboard_always_visible: bool,
    /// World-tabs ribbon display mode: "none" (default) / "top" / "bottom".
    /// web/GUI/Android only — see TabsMode.
    #[serde(default)]
    pub tabs: String,
    /// Icon bar visibility mode: "none" / "app_tablet" (default) / "all".
    /// web/GUI/Android only — see IconBarMode.
    #[serde(default)]
    pub icon_bar: String,
    /// Theme colors from ~/.clay/theme.dat (serialized as hex strings)
    #[serde(default)]
    pub theme_colors_json: String,
    /// Keyboard bindings (serialized as JSON object: key -> action)
    #[serde(default)]
    pub keybindings_json: String,
    /// Auth key value for display in web settings (only sent to authenticated clients)
    #[serde(default)]
    pub auth_key: String,
    /// WebSocket password (plaintext, sent to authenticated clients for display in settings)
    #[serde(default)]
    pub ws_password: String,
}

fn default_gui_transparency() -> f32 {
    1.0
}

fn default_web_path() -> String {
    "clay".to_string()
}

fn default_remote_initial_lines() -> u16 {
    100
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

fn default_web_font_weight() -> u16 {
    400
}

fn default_web_font_line_height() -> f32 {
    1.2
}

fn default_keyboard_always_visible() -> bool {
    true
}

/// Type of remote client connected via WebSocket
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum RemoteClientType {
    /// Web browser client - receives full history, scrolls locally
    #[default]
    Web,
    /// GUI client (webview) - receives full history, scrolls locally
    RemoteGUI,
    /// Remote console client (TUI) - receives screenful, requests scrollback from master
    RemoteConsole,
    /// Android app client (app.js running inside the Android WebView) - receives full
    /// history like Web, but declared separately so the source is distinguishable in
    /// diagnostics (e.g. the settings-save audit log).
    Android,
}

impl RemoteClientType {
    /// Short lowercase label used in diagnostics (e.g. the settings-save audit log).
    pub fn label(&self) -> &'static str {
        match self {
            RemoteClientType::Web => "web",
            RemoteClientType::RemoteGUI => "gui",
            RemoteClientType::RemoteConsole => "console",
            RemoteClientType::Android => "android",
        }
    }
}


/// Information about a connected WebSocket client
pub struct WsClientInfo {
    pub authenticated: bool,
    /// Bounded (PROTOCOL-ROADMAP.md Step 3, capacity `WS_CLIENT_CHANNEL_CAPACITY`) —
    /// was `mpsc::UnboundedSender`. All send sites use `try_send` (non-blocking) rather
    /// than `.send().await`, both because the fan-out functions below are sync and
    /// because awaiting a send from inside `handle_ws_client`'s own select loop — the
    /// same task that drains this channel — would deadlock if the channel were ever full.
    /// Item type is `Outbound`, not `WsMessage` (PROTOCOL-ROADMAP.md Step 8) — broadcasts
    /// serialize once and share the JSON (`Outbound::Shared`) across every recipient
    /// instead of each recipient's receive loop re-serializing its own clone.
    pub(crate) tx: mpsc::Sender<Outbound>,
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
    /// Client's IP address
    pub ip_address: String,
    /// When this client connected
    pub connected_at: std::time::Instant,
    /// When the client last sent a message
    pub last_activity: std::time::Instant,
    /// True when this session has been paused via /remote --pause.
    /// Paused sessions don't suppress activity notices for their world.
    pub paused: bool,
    /// Per-world last-contiguous-acked seq (PROTOCOL-ROADMAP.md Step 2): world_index ->
    /// highest seq such that every seq up to and including it has been delivered with no
    /// gap. Updated from `PongCheck.acked` and seeded from `AuthRequest.resume` on
    /// (re)connect, via `WebSocketServer::record_acked_seq`.
    pub acked_seq: std::collections::HashMap<usize, u64>,
    /// Worlds whose outbound stream to this client dropped a message because the bounded
    /// channel was full (PROTOCOL-ROADMAP.md Step 3). Each entry means a `ResyncRequired`
    /// for that world is still owed to the client — set on a failed best-effort
    /// `try_send(ResyncRequired)` in `reconcile_resync`, cleared once one is delivered.
    pub needs_resync: std::collections::HashSet<usize>,
}

/// User credential for multiuser authentication
#[derive(Clone, Debug)]
pub struct UserCredential {
    pub password_hash: String,
}

/// WebSocket server state
pub struct WebSocketServer {
    /// Guarded by `std::sync::RwLock`, not `tokio::sync::RwLock`: every critical section here
    /// is a short map lookup/mutation plus an unbounded-channel `send()`, never held across an
    /// `.await`. A blocking `.read()`/`.write()` (as opposed to `try_read()`/`try_write()` with a
    /// spawn-fallback) guarantees broadcasts to a client are never reordered relative to one
    /// another and are never silently dropped under lock contention (see D-Termux-lines
    /// investigation: reordering + a couple of missed `try_write()`s combined to make output
    /// batches vanish permanently on the Android client).
    pub clients: Arc<std::sync::RwLock<HashMap<u64, WsClientInfo>>>,
    pub next_client_id: Arc<std::sync::Mutex<u64>>,
    pub password_hash: Arc<std::sync::RwLock<String>>,
    /// True when a non-empty password is configured; false for auth-key-only mode.
    /// When false, password-based authentication is rejected even if allow list matches.
    pub password_enabled: Arc<std::sync::RwLock<bool>>,
    pub running: Arc<RwLock<bool>>,
    pub shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    pub port: u16,
    /// Bind address (default "0.0.0.0", can be set to "127.0.0.1" for local-only)
    pub bind_addr: String,
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

/// Parse a CSV allow-list string into trimmed, non-empty entries.
pub fn parse_allow_list_csv(allow_list: &str) -> Vec<String> {
    allow_list
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

impl WebSocketServer {
    pub fn new(password: &str, port: u16, allow_list: &str, whitelisted_host: Option<String>, multiuser_mode: bool, ban_list: BanList) -> Self {
        let password_hash = hash_password(password);
        // Parse allow list: comma-separated, trimmed entries
        let allow_list_vec: Vec<String> = parse_allow_list_csv(allow_list);
        Self {
            clients: Arc::new(std::sync::RwLock::new(HashMap::new())),
            next_client_id: Arc::new(std::sync::Mutex::new(1)),
            password_hash: Arc::new(std::sync::RwLock::new(password_hash)),
            password_enabled: Arc::new(std::sync::RwLock::new(!password.is_empty())),
            running: Arc::new(RwLock::new(false)),
            shutdown_tx: None,
            port,
            bind_addr: "0.0.0.0".to_string(),
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

    /// Extract shared connection state for the unified HTTP+WS server.
    /// The HTTP server uses this to hand off WebSocket upgrade requests.
    pub fn connection_state(&self, event_tx: mpsc::Sender<crate::AppEvent>) -> crate::http::WsConnectionState {
        crate::http::WsConnectionState {
            clients: self.clients.clone(),
            next_client_id: self.next_client_id.clone(),
            password_hash: self.password_hash.clone(),
            password_enabled: self.password_enabled.clone(),
            allow_list: self.allow_list.clone(),
            whitelisted_host: self.whitelisted_host.clone(),
            event_tx,
            multiuser_mode: self.multiuser_mode,
            users: self.users.clone(),
            ban_list: self.ban_list.clone(),
        }
    }

    /// Add a user for multiuser authentication, hashing the given plaintext password.
    pub fn add_user(&self, username: &str, password: &str) {
        let password_hash = hash_password(password);
        let mut users = self.users.write().unwrap();
        users.insert(username.to_string(), UserCredential { password_hash });
    }

    /// Set (or insert) a user's credential directly from an already-computed SHA-256
    /// password hash (hex), without hashing it again. Used by the `ChangePassword`
    /// handler (C1, security remediation): the client only ever sends
    /// `SHA256(new_password)`, never the plaintext, so this is the only way to update
    /// the live credential without breaking the invariant that `UserCredential.password_hash`
    /// is exactly `hash_password(plaintext)` — re-hashing the already-hashed value here
    /// would silently lock the user out.
    pub fn set_user_password_hash(&self, username: &str, password_hash: String) {
        let mut users = self.users.write().unwrap();
        users.insert(username.to_string(), UserCredential { password_hash });
    }

    /// Set the username for a connected client
    pub fn set_client_username(&self, client_id: u64, username: Option<String>) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.username = username;
        }
    }

    /// Get the username of a connected client (multiuser mode)
    pub fn get_client_username(&self, client_id: u64) -> Option<String> {
        let clients = self.clients.read().unwrap();
        clients.get(&client_id).and_then(|c| c.username.clone())
    }

    /// Get the IP address of a connected client
    pub fn get_client_ip(&self, client_id: u64) -> Option<String> {
        let clients = self.clients.read().unwrap();
        clients.get(&client_id).map(|c| c.ip_address.clone())
    }

    /// Clear a client's authentication state (for logout)
    pub fn clear_client_auth(&self, client_id: u64) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.authenticated = false;
            client.username = None;
        }
    }

    /// Broadcast a message to all clients owned by a specific user
    pub fn broadcast_to_owner(&self, msg: WsMessage, owner: Option<&str>) {
        let world_index = message_world_index(&msg);
        // PROTOCOL-ROADMAP.md Step 8: serialize once for the whole broadcast, share the
        // JSON via Arc<str> with every recipient instead of each recipient's own receive
        // loop re-serializing an identical WsMessage clone. A failure here is logged once
        // (inside serialize_for_broadcast) and nobody gets the message - same net effect
        // as the old per-client failure, minus the duplicate log lines.
        let Some(shared_json) = serialize_for_broadcast(&msg) else { return };
        let mut full: Vec<(u64, String)> = Vec::new();
        let mut flush_candidates: Vec<u64> = Vec::new();
        {
            let clients = self.clients.read().unwrap();
            for (&id, client) in clients.iter() {
                // Only broadcast to clients that are authenticated AND have received InitialState
                // This prevents ServerData from reaching clients before InitialState,
                // which causes SEQ MISMATCH errors and duplicate/flickering messages
                if client.authenticated && client.received_initial_state {
                    // In multiuser mode, only send to clients with matching username
                    let eligible = !self.multiuser_mode || client.username.as_deref() == owner;
                    if !eligible {
                        continue;
                    }
                    // Bounded channel (PROTOCOL-ROADMAP.md Step 3) — try_send instead of
                    // the old infallible unbounded send. See `reconcile_resync` for how a
                    // `TrySendError::Full` here is turned into a `ResyncRequired`. The
                    // Arc<str> clone below is cheap (refcount bump, no JSON re-copy).
                    match client.tx.try_send(Outbound::Shared(shared_json.clone())) {
                        Ok(()) => {
                            if let Some(wi) = world_index {
                                if client.needs_resync.contains(&wi) {
                                    flush_candidates.push(id);
                                }
                            }
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => full.push((id, client.ip_address.clone())),
                        Err(mpsc::error::TrySendError::Closed(_)) => {}
                    }
                }
            }
        }
        for (id, ip) in &full {
            crate::http::log_remote_event("WS-CHANNEL-FULL", ip,
                &format!("client={} world={:?} outbound channel full (capacity {}) - message dropped",
                    id, world_index, WS_CLIENT_CHANNEL_CAPACITY));
        }
        let full_ids: Vec<u64> = full.into_iter().map(|(id, _)| id).collect();
        reconcile_resync(&self.clients, world_index, &full_ids, &flush_candidates);
    }

    /// Broadcast a message to all authenticated clients (regardless of owner)
    /// Only sends to clients that have received their InitialState to prevent duplicates
    pub fn broadcast_to_all(&self, msg: WsMessage) {
        let world_index = message_world_index(&msg);
        // PROTOCOL-ROADMAP.md Step 8 — see broadcast_to_owner.
        let Some(shared_json) = serialize_for_broadcast(&msg) else { return };
        let mut full: Vec<(u64, String)> = Vec::new();
        let mut flush_candidates: Vec<u64> = Vec::new();
        {
            let clients = self.clients.read().unwrap();
            for (&id, client) in clients.iter() {
                // Only broadcast to clients that are authenticated AND have received InitialState
                // This prevents duplicate messages when a client connects while data is streaming
                if client.authenticated && client.received_initial_state {
                    // Bounded channel (PROTOCOL-ROADMAP.md Step 3) — see broadcast_to_owner.
                    match client.tx.try_send(Outbound::Shared(shared_json.clone())) {
                        Ok(()) => {
                            if let Some(wi) = world_index {
                                if client.needs_resync.contains(&wi) {
                                    flush_candidates.push(id);
                                }
                            }
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => full.push((id, client.ip_address.clone())),
                        Err(mpsc::error::TrySendError::Closed(_)) => {}
                    }
                }
            }
        }
        for (id, ip) in &full {
            crate::http::log_remote_event("WS-CHANNEL-FULL", ip,
                &format!("client={} world={:?} outbound channel full (capacity {}) - message dropped",
                    id, world_index, WS_CLIENT_CHANNEL_CAPACITY));
        }
        let full_ids: Vec<u64> = full.into_iter().map(|(id, _)| id).collect();
        reconcile_resync(&self.clients, world_index, &full_ids, &flush_candidates);
    }

    /// Mark a client as having received its InitialState
    /// After this, the client will receive broadcasts
    pub fn mark_initial_state_sent(&self, client_id: u64) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.received_initial_state = true;
        }
    }

    /// Send a message to a specific client. Single-recipient by construction, so this
    /// always carries `Outbound::Message` (PROTOCOL-ROADMAP.md Step 8) — there is no
    /// second recipient to share a pre-serialized `Shared` payload with.
    pub fn send_to_client(&self, client_id: u64, msg: WsMessage) {
        let world_index = message_world_index(&msg);
        let outcome = {
            let clients = self.clients.read().unwrap();
            clients.get(&client_id).map(|client| {
                let was_flagged = world_index.map(|wi| client.needs_resync.contains(&wi)).unwrap_or(false);
                (client.tx.try_send(Outbound::Message(Box::new(msg))), client.ip_address.clone(), was_flagged)
            })
        };
        let Some((result, ip, was_flagged)) = outcome else { return };
        match result {
            Ok(()) => {
                if was_flagged {
                    reconcile_resync(&self.clients, world_index, &[], &[client_id]);
                }
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                crate::http::log_remote_event("WS-CHANNEL-FULL", &ip,
                    &format!("client={} world={:?} outbound channel full (capacity {}) - message dropped",
                        client_id, world_index, WS_CLIENT_CHANNEL_CAPACITY));
                reconcile_resync(&self.clients, world_index, &[client_id], &[]);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    /// Send InitialState to a client AND mark received_initial_state = true, in one
    /// lock acquisition — avoids the race where a broadcast lands between sending the
    /// message and setting the flag.
    pub fn send_initial_state_and_mark(&self, client_id: u64, msg: WsMessage) {
        let mut guard = self.clients.write().unwrap();
        if let Some(client) = guard.get_mut(&client_id) {
            // InitialState carries no single world_index to target a resync at (it's the
            // whole-session bootstrap, not a per-world stream), so a `Full` here just gets
            // logged — nothing better to do than let the client hit its auth/keepalive
            // timeout and reconnect, at which point it gets a fresh InitialState anyway.
            // Single-recipient send, so Outbound::Message (PROTOCOL-ROADMAP.md Step 8).
            if let Err(mpsc::error::TrySendError::Full(_)) = client.tx.try_send(Outbound::Message(Box::new(msg))) {
                crate::http::log_remote_event("WS-CHANNEL-FULL", &client.ip_address,
                    &format!("client={} InitialState dropped - outbound channel full (capacity {})",
                        client_id, WS_CLIENT_CHANNEL_CAPACITY));
            }
            client.received_initial_state = true;
        }
    }

    /// Record (or seed) a client's per-world delivery ack (PROTOCOL-ROADMAP.md Step 2).
    /// Called from the `PongCheck.acked` handler on every liveness reply, and once from
    /// the `AuthRequest.resume` handler on (re)connect so a client that resumes and then
    /// reconnects again immediately isn't re-sent lines it already proved it has. Keeps
    /// the max seen per world so a stale/out-of-order ack can never move the tracked
    /// position backwards.
    pub fn record_acked_seq(&self, client_id: u64, acked: &[(usize, u64)]) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            for &(world_index, seq) in acked {
                let entry = client.acked_seq.entry(world_index).or_insert(0);
                if seq > *entry {
                    *entry = seq;
                }
            }
        }
    }

    /// Set the client type for a connected client
    pub fn set_client_type(&self, client_id: u64, client_type: RemoteClientType) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.client_type = client_type;
        }
    }

    /// Set the viewport height for a connected client
    pub fn set_client_viewport(&self, client_id: u64, height: usize) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.viewport_height = height;
        }
    }

    /// Set the current world being viewed by a connected client
    pub fn set_client_world(&self, client_id: u64, world_index: Option<usize>) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.current_world = world_index;
        }
    }

    /// Set the paused state for a connected client. Returns (was_paused, ip_address, current_world).
    pub fn set_client_paused(&self, client_id: u64, paused: bool) -> Option<(bool, String, Option<usize>)> {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            let was_paused = client.paused;
            client.paused = paused;
            return Some((was_paused, client.ip_address.clone(), client.current_world));
        }
        None
    }

    /// Set the authenticated status for a connected client
    pub fn set_client_authenticated(&self, client_id: u64, authenticated: bool) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.authenticated = authenticated;
        }
    }

    /// Get the client type for a connected client
    pub fn get_client_type(&self, client_id: u64) -> Option<RemoteClientType> {
        let clients = self.clients.read().unwrap();
        clients.get(&client_id).map(|c| c.client_type)
    }

    /// Get the minimum viewport height across all clients viewing a specific world
    /// Returns None if no clients are viewing the world
    pub fn min_viewport_for_world(&self, world_index: usize) -> Option<usize> {
        let clients = self.clients.read().unwrap();
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
    }

    /// Get list of client IDs viewing a specific world
    pub fn clients_viewing_world(&self, world_index: usize) -> Vec<u64> {
        let clients = self.clients.read().unwrap();
        clients.iter()
            .filter(|(_, c)| c.authenticated && c.received_initial_state)
            .filter(|(_, c)| c.current_world == Some(world_index))
            .map(|(&id, _)| id)
            .collect()
    }

    /// Broadcast a message to all authenticated clients (they filter by world_index client-side).
    ///
    /// This is deliberate, not merely unimplemented (PROTOCOL-ROADMAP.md Step 9 investigated
    /// filtering this to `client.current_world == world_index` and rejected it): every connected
    /// client — every browser tab, the GUI, Android — maintains a full local buffer for *every*
    /// world simultaneously, not just the one currently focused (see `app.js`'s `ServerData`
    /// handler, which indexes `worlds[msg.world_index]` unconditionally). That's what makes
    /// switching tabs instant and keeps unseen-line badges live for background worlds. Filtering
    /// server-side by "currently viewed" would silently starve every world a client isn't actively
    /// looking at, which is a regression, not a bandwidth optimization. It also avoids race
    /// conditions where a client switches world locally before the server has processed the
    /// corresponding `SwitchWorld`/`UpdateViewState`.
    /// Uses a single blocking lock acquisition (not try_read+spawn-fallback) so broadcasts to the
    /// same world are never reordered relative to one another (see D-Termux-lines investigation).
    pub fn broadcast_to_world_viewers(&self, _world_index: usize, msg: WsMessage) {
        let world_index = message_world_index(&msg);
        // PROTOCOL-ROADMAP.md Step 8 — see broadcast_to_owner.
        let Some(shared_json) = serialize_for_broadcast(&msg) else { return };
        let mut full: Vec<(u64, String)> = Vec::new();
        let mut flush_candidates: Vec<u64> = Vec::new();
        {
            let clients = self.clients.read().unwrap();
            for (&id, client) in clients.iter() {
                if client.authenticated && client.received_initial_state {
                    // Bounded channel (PROTOCOL-ROADMAP.md Step 3) — see broadcast_to_owner.
                    match client.tx.try_send(Outbound::Shared(shared_json.clone())) {
                        Ok(()) => {
                            if let Some(wi) = world_index {
                                if client.needs_resync.contains(&wi) {
                                    flush_candidates.push(id);
                                }
                            }
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => full.push((id, client.ip_address.clone())),
                        Err(mpsc::error::TrySendError::Closed(_)) => {}
                    }
                }
            }
        }
        for (id, ip) in &full {
            crate::http::log_remote_event("WS-CHANNEL-FULL", ip,
                &format!("client={} world={:?} outbound channel full (capacity {}) - message dropped",
                    id, world_index, WS_CLIENT_CHANNEL_CAPACITY));
        }
        let full_ids: Vec<u64> = full.into_iter().map(|(id, _)| id).collect();
        reconcile_resync(&self.clients, world_index, &full_ids, &flush_candidates);
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
        // Clear whitelisted host if it's no longer in the new allow list
        {
            let mut whitelist = self.whitelisted_host.write().unwrap();
            if let Some(ref host) = *whitelist {
                let still_valid = allow_list_vec.iter().any(|entry| entry == "*" || entry == host);
                if !still_valid {
                    *whitelist = None;
                }
            }
        }
        *self.allow_list.write().unwrap() = allow_list_vec;
    }

    /// Update the password hash on the running server (takes effect for new connections)
    pub fn update_password(&self, password: &str) {
        *self.password_hash.write().unwrap() = hash_password(password);
        *self.password_enabled.write().unwrap() = !password.is_empty();
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

/// Compute SHA256(stored_hash + challenge) for challenge-response auth verification
pub fn hash_with_challenge(stored_hash: &str, challenge: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(stored_hash.as_bytes());
    hasher.update(challenge.as_bytes());
    hex::encode(hasher.finalize())
}

/// Returns true if an allow list pattern is a hostname pattern (e.g. "*.rd.shawcable.net")
/// rather than an IP pattern (e.g. "192.168.1.*").
pub fn is_hostname_pattern(pattern: &str) -> bool {
    let p = pattern.trim();
    if p == "*" || p == "localhost" || p == "127.0.0.1" || p == "::1" {
        return false;
    }
    // Hostname wildcard prefix, or any alphabetic character (rules out pure IP patterns)
    p.starts_with("*.") || p.chars().any(|c| c.is_ascii_alphabetic())
}

/// Match a resolved hostname against a hostname pattern (case-insensitive).
/// Supports exact match and `*.suffix` wildcard (matches one or more subdomain labels).
fn matches_hostname_pattern(hostname: &str, pattern: &str) -> bool {
    let hn = hostname.trim().to_ascii_lowercase();
    let pat = pattern.trim().to_ascii_lowercase();
    if let Some(suffix) = pat.strip_prefix("*.") {
        // *.example.com matches foo.example.com but not example.com itself
        hn.ends_with(&format!(".{suffix}"))
    } else {
        hn == pat
    }
}

/// Check if an IP or its resolved hostname is in the allow list.
/// Handles both IP patterns (e.g. `192.168.1.*`) and hostname patterns (e.g. `*.example.com`).
/// Pass `hostname` as the result of a reverse DNS lookup when hostname patterns are present.
pub fn is_in_allow_list(ip: &str, hostname: Option<&str>, allow_list: &[String]) -> bool {
    let normalized_ip = if ip == "127.0.0.1" || ip == "::1" { "localhost" } else { ip };

    for pattern in allow_list {
        let trimmed = pattern.trim();

        // Bare "*" matches everything
        if trimmed == "*" {
            return true;
        }

        let normalized_pattern = if trimmed == "127.0.0.1" || trimmed == "::1" { "localhost" } else { trimmed };

        if is_hostname_pattern(normalized_pattern) {
            // Match against resolved hostname
            if let Some(hn) = hostname {
                if matches_hostname_pattern(hn, normalized_pattern) {
                    return true;
                }
            }
        } else if let Some(prefix) = normalized_pattern.strip_suffix('*') {
            // IP wildcard: reject overly broad patterns (must have at least 4 chars and a dot)
            if prefix.len() < 4 || !prefix.contains('.') {
                continue;
            }
            if normalized_ip.starts_with(prefix) {
                return true;
            }
        } else if normalized_ip == normalized_pattern {
            return true;
        }
    }
    false
}

/// Check if an IP address is in the allow list (IP patterns only; no hostname lookup).
pub fn is_ip_in_allow_list(ip: &str, allow_list: &[String]) -> bool {
    is_in_allow_list(ip, None, allow_list)
}

/// Perform an async reverse DNS lookup on an IP address.
/// Returns the resolved hostname, or None if lookup fails or no PTR record exists.
pub async fn reverse_dns_lookup(ip: &str) -> Option<String> {
    let ip_owned = ip.to_string();
    tokio::task::spawn_blocking(move || reverse_dns_lookup_blocking(&ip_owned))
        .await
        .ok()
        .flatten()
}

#[cfg(unix)]
fn reverse_dns_lookup_blocking(ip: &str) -> Option<String> {
    use std::net::IpAddr;
    use std::str::FromStr;

    let addr = IpAddr::from_str(ip).ok()?;
    let mut host_buf = vec![0u8; 256];

    let ret = match addr {
        IpAddr::V4(v4) => unsafe {
            let mut sa: libc::sockaddr_in = std::mem::zeroed();
            sa.sin_family = libc::AF_INET as libc::sa_family_t;
            sa.sin_addr.s_addr = u32::from_ne_bytes(v4.octets());
            libc::getnameinfo(
                &sa as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                host_buf.as_mut_ptr() as *mut libc::c_char,
                host_buf.len() as _,
                std::ptr::null_mut(), 0, 0,
            )
        },
        IpAddr::V6(v6) => unsafe {
            let mut sa: libc::sockaddr_in6 = std::mem::zeroed();
            sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sa.sin6_addr.s6_addr = v6.octets();
            libc::getnameinfo(
                &sa as *const libc::sockaddr_in6 as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                host_buf.as_mut_ptr() as *mut libc::c_char,
                host_buf.len() as _,
                std::ptr::null_mut(), 0, 0,
            )
        },
    };

    if ret != 0 { return None; }
    let end = host_buf.iter().position(|&b| b == 0).unwrap_or(host_buf.len());
    let hostname = String::from_utf8(host_buf[..end].to_vec()).ok()?;
    // getnameinfo returns the IP string when no PTR record exists — filter it out
    if hostname == ip || hostname.is_empty() { None } else { Some(hostname) }
}

#[cfg(not(unix))]
fn reverse_dns_lookup_blocking(_ip: &str) -> Option<String> {
    None
}

/// Check if any entry in a CSV allow list string contains a bare "*" wildcard.
pub fn allow_list_has_wildcard(allow_list: &str) -> bool {
    allow_list.split(',').any(|s| s.trim() == "*")
}

/// Start the WebSocket server
pub async fn start_websocket_server(
    server: &mut WebSocketServer,
    event_tx: mpsc::Sender<AppEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = format!("{}:{}", server.bind_addr, server.port);
    let listener = TcpListener::bind(&addr).await?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    server.shutdown_tx = Some(shutdown_tx);

    let clients = Arc::clone(&server.clients);
    let next_client_id = Arc::clone(&server.next_client_id);
    let password_hash = server.password_hash.clone();
    let password_enabled = server.password_enabled.clone();
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
                            let password_hash = password_hash.read().unwrap().clone();
                            let allow_list = allow_list.clone();
                            let whitelisted_host = whitelisted_host.clone();
                            let event_tx = event_tx.clone();
                            let multiuser_mode = multiuser_mode;
                            let users = users.clone();
                            let ban_list = ban_list.clone();
                            let password_enabled = *password_enabled.read().unwrap();
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
                                                password_enabled,
                                                allow_list,
                                                whitelisted_host,
                                                client_addr,
                                                event_tx,
                                                multiuser_mode,
                                                users,
                                                ban_list,
                                                false, // standalone WS-only server: no accept-time knock support
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
                                    password_enabled,
                                    allow_list,
                                    whitelisted_host,
                                    client_addr,
                                    event_tx,
                                    multiuser_mode,
                                    users,
                                    ban_list,
                                    false, // standalone WS-only server: no accept-time knock support
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
                                                password_enabled,
                                                allow_list,
                                                whitelisted_host,
                                                client_addr,
                                                event_tx,
                                                multiuser_mode,
                                                users,
                                                ban_list,
                                                false, // standalone WS-only server: no accept-time knock support
                                            ).await {
                                                // Connection error, client disconnected
                                            }
                                        }
                                        Err(e) => {
                                            // TLS handshake failed - log to remote log
                                            crate::http::log_remote_event("WSS-TLS-ERROR",
                                                &client_addr.ip().to_string(), &format!("{}", e));
                                        }
                                    }
                                } else if let Err(_e) = handle_ws_client(
                                    stream,
                                    client_id,
                                    clients,
                                    password_hash,
                                    password_enabled,
                                    allow_list,
                                    whitelisted_host,
                                    client_addr,
                                    event_tx,
                                    multiuser_mode,
                                    users,
                                    ban_list,
                                    false, // standalone WS-only server: no accept-time knock support
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
    clients: Arc<std::sync::RwLock<HashMap<u64, WsClientInfo>>>,
    password_hash: String,
    password_enabled: bool,
    allow_list: Arc<std::sync::RwLock<Vec<String>>>,
    whitelisted_host: Arc<std::sync::RwLock<Option<String>>>,
    client_addr: std::net::SocketAddr,
    event_tx: mpsc::Sender<AppEvent>,
    multiuser_mode: bool,
    users: Arc<std::sync::RwLock<HashMap<String, UserCredential>>>,
    ban_list: BanList,
    // True when this connection already passed the CLAY-KNOCK v1 preamble (D4,
    // SECURITY-ROADMAP.md) at accept time. A knocked device has proven it holds a
    // currently-valid auth key (a revoked key fails the knock) — see the "not in allow
    // list" rejection below for why that matters here.
    knocked: bool,
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

    // Localhost always allowed for password auth (embedded GUI connects via 127.0.0.1)
    let is_localhost = client_ip == "127.0.0.1" || client_ip == "::1";

    // Check allow list for non-auth-key connections.
    // Auth key validation happens later (after WS handshake) and always bypasses allow list.
    // Here we just check if the IP is eligible for password-based auth.
    let in_allow_list = is_localhost || {
        // Determine if any hostname patterns require a reverse DNS lookup
        let has_hostname_patterns = {
            let allow_list_guard = allow_list.read().unwrap();
            allow_list_guard.iter().any(|p| is_hostname_pattern(p.trim()))
        };
        let hostname = if has_hostname_patterns {
            reverse_dns_lookup(&client_ip).await
        } else {
            None
        };
        let allow_list_guard = allow_list.read().unwrap();
        is_in_allow_list(&client_ip, hostname.as_deref(), &allow_list_guard)
    };
    // D5 (SECURITY-ROADMAP.md): an empty allow list means password auth is allowed from
    // anywhere — it does NOT mean "in the list". Used below to skip the allow-list
    // rejection entirely when no list is configured.
    let allow_list_is_empty = !is_localhost && {
        let allow_list_guard = allow_list.read().unwrap();
        allow_list_guard.is_empty()
    };
    // B1 (security remediation): cap message/frame size so an unauthenticated client
    // can't pin large buffers pre-auth (the whole frame is buffered and serde_json-parsed
    // before AuthRequest is checked, below). NOTE: tungstenite enforces this cap
    // symmetrically - it also bounds the daemon's own OUTBOUND sends on this same
    // WebSocketConfig, including InitialState (see build_initial_state / ws_send_
    // initial_state_and_mark). The original 256 KiB was sized only against small
    // legitimate *inbound* client messages (commands, settings pushes) and didn't
    // account for that; a busy multi-world daemon's InitialState can genuinely exceed
    // it, which made ws_sink.send(...) fail and silently kill the connection right
    // after a successful auth (see the WS-SEND-FAIL logging below). 2 MiB/256 KiB still
    // bounds per-connection memory for a pre-auth sender comfortably (and is backed by
    // build_initial_state's own aggregate line budget, which keeps InitialState well
    // under this regardless of world count).
    let ws_config = WebSocketConfig {
        max_message_size: Some(2 * 1024 * 1024),
        max_frame_size: Some(256 * 1024),
        ..Default::default()
    };
    let ws_stream = match accept_async_with_config(stream, Some(ws_config)).await {
        Ok(ws) => ws,
        Err(e) => {
            return Err(e.into());
        }
    };
    let (mut ws_sink, mut ws_source) = ws_stream.split();

    // Create channel for sending messages to this client. Bounded (PROTOCOL-ROADMAP.md
    // Step 3) — see WS_CLIENT_CHANNEL_CAPACITY for sizing rationale.
    let (tx, mut rx) = mpsc::channel::<Outbound>(WS_CLIENT_CHANNEL_CAPACITY);

    // Generate random challenge for challenge-response auth. Fail closed: an all-zero
    // (or otherwise predictable) challenge would let an attacker who knows a user's
    // password hash replay a precomputed response, so refuse the connection outright
    // rather than proceeding with a weak/default challenge (C2, security remediation).
    let challenge = {
        let mut bytes = [0u8; 16];
        if getrandom::getrandom(&mut bytes).is_err() {
            crate::http::log_remote_event("WS-REJECT", &client_ip, "rng failure generating auth challenge");
            return Err("failed to generate secure auth challenge".into());
        }
        hex::encode(bytes)
    };

    // Send ServerHello immediately to tell client about multiuser mode
    try_send_local(&clients, client_id, &tx, &client_ip, WsMessage::ServerHello { multiuser_mode, challenge: challenge.clone() });

    // Add client to clients map (auto-authenticated if whitelisted)
    {
        let mut clients_guard = clients.write().unwrap();
        clients_guard.insert(client_id, WsClientInfo {
            authenticated: is_whitelisted,
            tx: tx.clone(),
            current_world: None,
            username: None,
            received_initial_state: false,
            client_type: RemoteClientType::Web,  // Default, updated by ClientTypeDeclaration
            viewport_height: 24,  // Default, updated by UpdateViewState
            ip_address: client_ip.clone(),
            connected_at: std::time::Instant::now(),
            last_activity: std::time::Instant::now(),
            paused: false,
            acked_seq: std::collections::HashMap::new(),
            needs_resync: std::collections::HashSet::new(),
        });
    }

    // Log connection
    crate::http::log_remote_event("WS-CONNECT", &client_ip,
        &format!("whitelisted={}", is_whitelisted));

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
        try_send_local(&clients, client_id, &tx, &client_ip, response);
        // Create a fake AuthRequest to trigger initial state send
        let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(WsMessage::AuthRequest { username: None, password_hash: String::new(), current_world: None, auth_key: None, request_key: false, challenge_response: false, resume: Vec::new() }))).await;
    }

    // Combined receive/send/keepalive loop.
    // Auth deadline: absolute — unauthenticated clients have WS_AUTH_TIMEOUT_SECS to authenticate.
    // Keepalive: idle authenticated clients receive a protocol-level Ping every
    // WS_KEEPALIVE_INTERVAL_SECS; no Pong within WS_PONG_TIMEOUT_SECS = dead peer, disconnect.
    //
    // Both `keepalive_deadline` and `pong_deadline` are absolute instants, advanced only on
    // a real state transition (Ping just sent / Pong received) — never recomputed as a flat
    // "N seconds from now" on every loop iteration. That used to be the bug here: `sleep_dur`
    // was a fresh `Duration::from_secs(...)` each time, and `tokio::select!` re-enters this
    // loop (recomputing sleep_dur from scratch) whenever ANY arm fires — most commonly
    // `rx.recv()` delivering MUD output. On a world producing output more than once a minute,
    // the keepalive Ping was therefore never sent and a dead-but-locally-still-writable
    // connection was never detected (D-Termux-lines investigation).
    let auth_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(WS_AUTH_TIMEOUT_SECS);
    let mut keepalive_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(WS_KEEPALIVE_INTERVAL_SECS);
    let mut pong_deadline: Option<tokio::time::Instant> = None;

    loop {
        let authed = {
            let clients_guard = clients.read().unwrap();
            clients_guard.get(&client_id).map(|c| c.authenticated).unwrap_or(false)
        };

        let sleep_dur = if !authed {
            let remaining = auth_deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                crate::http::log_remote_event("WS-AUTH-TIMEOUT", &client_ip, "unauthenticated grace period expired");
                break;
            }
            remaining
        } else if let Some(pd) = pong_deadline {
            pd.saturating_duration_since(tokio::time::Instant::now())
        } else {
            keepalive_deadline.saturating_duration_since(tokio::time::Instant::now())
        };

        tokio::select! {
            Some(outbound) = rx.recv() => {
                // PROTOCOL-ROADMAP.md Step 8: `Outbound::Shared` carries JSON that was
                // already successfully serialized once, up front, by whichever broadcast
                // call site produced it (see `serialize_for_broadcast`) - so there is no
                // serialization step (and no WS-SERIALIZE-FAIL path) here for it, unlike
                // the `Message` arm below which still serializes individually exactly as
                // every send did before this step.
                let send_outcome: Result<(String, usize), ()> = match outbound {
                    Outbound::Shared(json) => {
                        let msg_len = json.len();
                        Ok((json.to_string(), msg_len))
                    }
                    Outbound::Message(msg) => {
                        match serde_json::to_string(&msg) {
                            Ok(json) => {
                                let msg_len = json.len();
                                Ok((json, msg_len))
                            }
                            Err(e) => {
                                // PROTOCOL-ROADMAP.md Step 4: was previously silent - a serialization
                                // failure here (e.g. a NaN/Infinity float slipping into a settings
                                // message) vanished with zero trace, right after successfully being
                                // dequeued, as if the message had been delivered. Log which message
                                // *variant* failed (never the full Debug dump - some variants, e.g.
                                // UpdateWorldSettings/AuthRequest, carry a plaintext password field
                                // per CLAUDE.md's password-handling rule, so only the part of the
                                // Debug output before the first field list is kept). The message is
                                // dropped (not retried - a value that can't serialize now won't
                                // serialize later) but the connection stays up.
                                let debug_str = format!("{:?}", msg);
                                let variant = debug_str.split(['{', '(']).next().unwrap_or(&debug_str).trim();
                                crate::http::log_remote_event("WS-SERIALIZE-FAIL", &client_ip,
                                    &format!("variant={}: {}", variant, e));
                                debug_log(true, &format!(
                                    "WS-SERIALIZE-FAIL: client={} variant={}: {}", client_ip, variant, e));
                                Err(())
                            }
                        }
                    }
                };
                if let Ok((json, msg_len)) = send_outcome {
                    if let Err(e) = ws_sink.send(WsRawMessage::Text(json)).await {
                        // Was previously silent - a send failure here (e.g. exceeding
                        // ws_config's max_message_size above) killed the connection right
                        // after a successful auth with zero trace in either log.
                        crate::http::log_remote_event("WS-SEND-FAIL", &client_ip,
                            &format!("{} bytes: {}", msg_len, e));
                        debug_log(is_debug_enabled(), &format!(
                            "WS-SEND-FAIL: client={} {} bytes: {}", client_ip, msg_len, e));
                        break;
                    }
                    // PROTOCOL-ROADMAP.md Step 3: draining `outbound` out of `rx` just freed
                    // a slot in this client's channel. If an earlier broadcast overflowed it
                    // and left a `ResyncRequired` undelivered (`needs_resync`), this is
                    // exactly the moment - right after a successful send, from the same
                    // task that owns and drains this channel - to retry it, without any
                    // risk of the lock-nesting/deadlock that ruled out doing this from
                    // inside the sync fan-out functions themselves.
                    let pending_worlds: Vec<usize> = {
                        let guard = clients.read().unwrap();
                        guard.get(&client_id).map(|c| c.needs_resync.iter().copied().collect()).unwrap_or_default()
                    };
                    for wi in pending_worlds {
                        reconcile_resync(&clients, Some(wi), &[], &[client_id]);
                    }
                }
            }
            msg_result = ws_source.next() => {
            match msg_result {
            Some(Ok(WsRawMessage::Text(text))) => {
                if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                    match &ws_msg {
                        WsMessage::AuthRequest { username, password_hash: client_hash, auth_key, request_key, challenge_response: uses_challenge, .. } => {
                            let has_key = auth_key.as_ref().map(|k| !k.is_empty()).unwrap_or(false);
                            let has_pw = !client_hash.is_empty();
                            crate::http::log_remote_event("WS-AUTH", &client_ip,
                                &format!("has_key={}, has_password={}, challenge={}", has_key, has_pw, uses_challenge));

                            // Try auth_key first (device key authentication)
                            // auth_key validation must happen in the app since keys are stored there
                            // So we forward to app and let it respond (include challenge for verification)
                            if auth_key.is_some() && !auth_key.as_ref().unwrap().is_empty() {
                                // Forward to app for key validation
                                // App will send AuthResponse directly
                                let _ = event_tx.send(AppEvent::WsAuthKeyValidation(client_id, Box::new(ws_msg.clone()), client_ip.clone(), challenge.clone())).await;
                                continue;
                            }

                            // Password-based auth: reject if no password is configured
                            if !password_enabled {
                                crate::http::log_remote_event("WS-REJECT", &client_ip,
                                    "no password configured, auth key required");
                                try_send_local(&clients, client_id, &tx, &client_ip, WsMessage::AuthResponse {
                                    success: false,
                                    error: Some("Password auth not available. Use an auth key.".to_string()),
                                    username: None,
                                    multiuser_mode,
                                });
                                continue;
                            }

                            // Password-based auth: check allow list (D5, SECURITY-ROADMAP.md).
                            // - Empty allow list: password auth allowed from anywhere (no
                            //   rejection here — this is the D5 behavior change).
                            // - Non-empty allow list: IP must be in list or whitelisted.
                            if !allow_list_is_empty && !in_allow_list && !is_whitelisted {
                                crate::http::log_remote_event("WS-REJECT", &client_ip,
                                    "not in allow list");
                                // D6 (SECURITY-ROADMAP.md): with the accept-time gate
                                // active, the only way a non-listed IP reaches this
                                // branch at all is a successful knock — proof it holds
                                // a currently-valid auth key. Don't strike it: a banned
                                // IP can't even knock (ban check runs before accept),
                                // so this would permanently lock the device out of its
                                // own recovery path with no way back in short of a
                                // server restart.
                                if !knocked {
                                    ban_list.record_violation(&client_ip, "WebSocket: not in allow list");
                                }
                                try_send_local(&clients, client_id, &tx, &client_ip, WsMessage::AuthResponse {
                                    success: false,
                                    error: Some("Not authorized from this address".to_string()),
                                    username: None,
                                    multiuser_mode,
                                });
                                continue;
                            }

                            // Password-based authentication
                            // If challenge_response is true, client sent SHA256(SHA256(password) + challenge)
                            // We compare by computing SHA256(stored_hash + challenge) on our side
                            let (auth_success, auth_error, auth_username) = if multiuser_mode {
                                // Multiuser mode: require username and validate against users map
                                match username {
                                    Some(uname) if !uname.is_empty() => {
                                        let users_guard = users.read().unwrap();
                                        if let Some(user_cred) = users_guard.get(uname) {
                                            let matches = if *uses_challenge {
                                                hash_with_challenge(&user_cred.password_hash, &challenge) == *client_hash
                                            } else {
                                                // B3 (security remediation): constant-time compare — this is a
                                                // static stored-secret vs. client-supplied-secret compare (no
                                                // per-connection challenge salt), so a naive `==` could leak
                                                // match-progress via timing.
                                                crate::util::constant_time_eq(user_cred.password_hash.as_bytes(), client_hash.as_bytes())
                                            };
                                            if matches {
                                                (true, None, Some(uname.clone()))
                                            } else {
                                                (false, Some("Authentication failed".to_string()), None)
                                            }
                                        } else {
                                            (false, Some("Authentication failed".to_string()), None)
                                        }
                                    }
                                    _ => (false, Some("Username required".to_string()), None),
                                }
                            } else {
                                // Single-user mode: just validate password
                                let matches = if *uses_challenge {
                                    hash_with_challenge(&password_hash, &challenge) == *client_hash
                                } else {
                                    // B3 (security remediation): constant-time compare (see note above).
                                    crate::util::constant_time_eq(client_hash.as_bytes(), password_hash.as_bytes())
                                };
                                if matches {
                                    (true, None, None)
                                } else {
                                    (false, Some("Authentication failed".to_string()), None)
                                }
                            };

                            if auth_success {
                                // Log successful auth
                                log_ws_auth(&client_ip, true, auth_username.as_deref());
                                // Clear any accumulated violations so transient reconnect
                                // failures don't result in a ban after a successful login
                                ban_list.clear_violations(&client_ip);

                                // Mark as authenticated and set username
                                let mut clients_guard = clients.write().unwrap();
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
                                // Record violation for failed auth attempt. Bans at 5
                                // (not 2) and applies to allow-listed IPs too — see
                                // `BanList::record_auth_failure` (D6, SECURITY-ROADMAP.md).
                                ban_list.record_auth_failure(&client_ip, "WebSocket: failed auth");
                            }
                            // Send auth response
                            let response = WsMessage::AuthResponse {
                                success: auth_success,
                                error: auth_error,
                                username: auth_username,
                                multiuser_mode,
                            };
                            try_send_local(&clients, client_id, &tx, &client_ip, response);

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
                        WsMessage::Ping => {
                            try_send_local(&clients, client_id, &tx, &client_ip, WsMessage::Pong);
                        }
                        _ => {
                            // Check if authenticated before processing other messages
                            let is_authed = {
                                let clients_guard = clients.read().unwrap();
                                clients_guard.get(&client_id).map(|c| c.authenticated).unwrap_or(false)
                            };
                            if is_authed {
                                // Update last activity time
                                {
                                    let mut clients_guard = clients.write().unwrap();
                                    if let Some(client) = clients_guard.get_mut(&client_id) {
                                        client.last_activity = std::time::Instant::now();
                                    }
                                }
                                // Handle RevokeKey and RegenerateAuthKey inside auth check
                                if let WsMessage::RevokeKey { ref auth_key } = ws_msg {
                                    let _ = event_tx.send(AppEvent::WsKeyRevoke(client_id, auth_key.clone())).await;
                                } else if let WsMessage::RegenerateAuthKey = ws_msg {
                                    let _ = event_tx.send(AppEvent::WsKeyRequest(client_id)).await;
                                } else {
                                    let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(ws_msg))).await;
                                }
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
            Some(Ok(WsRawMessage::Pong(_))) => {
                // Response to our keepalive ping — peer is alive. Reset the keepalive
                // deadline from now, not from whenever this loop iteration happens to run.
                pong_deadline = None;
                keepalive_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(WS_KEEPALIVE_INTERVAL_SECS);
            }
            Some(Ok(WsRawMessage::Close(_))) => break,
            Some(Ok(WsRawMessage::Ping(_))) => {
                // Tungstenite auto-handles the protocol-level Pong reply
            }
            Some(Ok(WsRawMessage::Binary(_))) => break,
            Some(Err(_)) | None => break,
            _ => {}
        }
            }
            _ = tokio::time::sleep(sleep_dur) => {
                if !authed {
                    crate::http::log_remote_event("WS-AUTH-TIMEOUT", &client_ip, "unauthenticated grace period expired");
                    break;
                } else if pong_deadline.is_some() {
                    crate::http::log_remote_event("WS-DEAD", &client_ip, "no pong response to keepalive");
                    break;
                } else {
                    if let Err(e) = ws_sink.send(WsRawMessage::Ping(Vec::new())).await {
                        crate::http::log_remote_event("WS-SEND-FAIL", &client_ip, &format!("keepalive ping: {}", e));
                        break;
                    }
                    pong_deadline = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(WS_PONG_TIMEOUT_SECS));
                }
            }
        }
    }

    // Clean up
    {
        let mut clients_guard = clients.write().unwrap();
        clients_guard.remove(&client_id);
    }
    let _ = event_tx.send(AppEvent::WsClientDisconnected(client_id)).await;

    Ok(())
}
