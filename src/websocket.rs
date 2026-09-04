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
///
/// **`ScrollbackBatch` is deliberately absent and must stay absent** (PROTOCOL-ROADMAP.md
/// Phase J). It spans every world in one message, so there is no single index to return —
/// but more importantly, a dropped batch must NOT trigger `ResyncRequired`. The push pump
/// already handles `TrySendError::Full` correctly by leaving its cursors untouched and
/// retrying the identical batch on the next tick; adding a resync on top would turn ordinary
/// backpressure into a resync storm on exactly the slow client that provoked it. This looks
/// like an oversight to anyone scanning the list — it isn't.
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
        | WsMessage::ClaimedNew { world_index, .. }
        | WsMessage::ReleasedNew { world_index, .. }
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
        /// Per-world `(world_index, seq_epoch)` for the buffers `resume` was built from -
        /// the client's copy of `World::seq_epoch` as it last saw it.
        ///
        /// Exists so `build_initial_state` can skip re-sending history the client is
        /// about to throw away. On an in-memory reconnect the client keeps its own buffer
        /// (`world.output_lines = dedupBySeq(priorWorld.output_lines)` in app.js's
        /// InitialState handler) and discards `output_lines_ts` entirely - so for a
        /// resumed world those lines are pure waste, and with `remote_initial_lines`
        /// at its 5000 default that is the whole budget being spent to be ignored.
        ///
        /// The epoch, not the index, is what makes the skip safe. It answers "is the world
        /// at this index the same world instance the client is talking about?", which the
        /// index alone cannot: worlds get added and removed, so index N may name a
        /// different world than it did when the client recorded its frontier. It also
        /// covers the case where the client discards its own buffer on arrival - it does
        /// that precisely when the epoch differs, which is exactly when the server keeps
        /// sending the history. Matching epochs means both sides agree the buffer stands.
        ///
        /// Empty from an older client (and always empty in multiuser, where `seq_epoch` is
        /// a hardcoded 0), in which case nothing is skipped and the full history is sent
        /// exactly as before.
        #[serde(default)]
        resume_epochs: Vec<(usize, u64)>,
        /// The client build's version string, logged next to WS-AUTH so a single log pull
        /// answers "what is this peer running?".
        ///
        /// Exists because that question was unanswerable exactly when it mattered: a fix had
        /// shipped, the diagnostics it added produced nothing, and there was no way to tell
        /// "the peer does not have the build yet" from "the build is installed and the thing
        /// being measured did not happen". Those need opposite next steps. An Android APK is
        /// the case that really needs it - it bundles its own assets, so unlike a web client
        /// (served by the server, hence always the same version) it can lag arbitrarily far
        /// behind the server it talks to.
        ///
        /// Empty from a client that predates this field, logged as "-".
        #[serde(default)]
        client_version: String,
        /// Stable, client-generated identity that survives reconnects (localStorage on
        /// web/Android, a per-process value on the Rust console). ▶ ownership
        /// (`OutputLine::display_id`) is keyed on this rather than the per-connection client
        /// id, so a brief transport drop does not lose a client's markers — a reconnect gets
        /// a fresh connection id but the same `client_uid`, and its claims still match.
        /// Empty from an older client, in which case ownership falls back to the connection
        /// id and behaves as before (markers lost on reconnect).
        #[serde(default)]
        client_uid: String,
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
        /// The ▶ ownership id this client's markers are recorded under
        /// (`OutputLine::display_id`). A line renders ▶ iff its `display_id` equals this.
        /// Sent here rather than in `ServerHello` because it is derived from
        /// `AuthRequest.client_uid`, which the server hasn't seen at hello time. Stable
        /// across reconnects for a client that supplies a uid, which is what preserves its
        /// markers through a brief transport drop. `serde(default)` = 0 against an older
        /// server; 0 is the embedded GUI's id, so a remote client simply paints no ▶ rather
        /// than adopting somebody else's markers.
        #[serde(default)]
        your_display_id: u64,
        /// Capability advertisement for the server-push scrollback download
        /// (PROTOCOL-ROADMAP.md Phase J). `true` = this server understands
        /// `ScrollbackSyncRequest`/`ScrollbackContinue` and will push `ScrollbackBatch`.
        /// `serde(default)` = false against an older server, which is exactly the signal a
        /// new client needs to fall back to the legacy `RequestScrollback` bulk fetch —
        /// without it a new client would send a sync request into an old server's `_ => {}`
        /// catch-all and wait forever for a batch that never comes.
        #[serde(default)]
        scrollback_push: bool,
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
    ServerData { world_index: usize, data: String, is_viewed: bool, #[serde(default)] ts: u64, #[serde(default = "default_true", skip_serializing_if = "is_true")] from_server: bool, #[serde(default)] seq: u64, #[serde(default, skip_serializing_if = "Option::is_none")] end_seq: Option<u64>, #[serde(default, skip_serializing_if = "is_false")] flush: bool, #[serde(default, skip_serializing_if = "is_false")] gagged: bool,
        /// Whole batch is `/recall -D` output — archived text delivered as ordinary live
        /// lines. Batch-level rather than per-line because `emit_recall` always emits a
        /// homogeneous block (the archive rows, or a single client notice, never a mix).
        /// Clients draw 🛢️ instead of ✨ for these. See `OutputLine::archive_sourced`.
        #[serde(default, skip_serializing_if = "is_false")] archive_sourced: bool,
        /// Per-line `/hilite` colours, parallel to the newline-separated lines in `data`.
        /// Empty (and omitted from the wire) whenever no line in the batch is highlighted —
        /// the overwhelming common case, so this costs nothing on the hot path.
        ///
        /// `highlight_color` used to be applied to the server-side buffer and never
        /// transmitted on the live path at all, so a highlighted line arrived and rendered
        /// plain on web/Android until some resync re-hydrated it from
        /// `InitialState`/`ScrollbackLines`, both of which do carry it per line.
        #[serde(default, skip_serializing_if = "Vec::is_empty")] highlight_colors: Vec<Option<String>> },
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
    /// This client now owns the ▶ new-text marker for exactly these seqs in `world_index`.
    /// Sent to ONE client (the one that just started displaying the world), never broadcast:
    /// a claim only moves lines from unowned to owned-by-that-client, so no other client's
    /// rendering changes.
    ///
    /// An explicit seq list rather than a range: unviewed lines are NOT a contiguous tail.
    /// `viewed` is decided per line by whether anyone was watching when it arrived, and that
    /// flips back and forth with no display event in between, so a range would wrongly sweep
    /// in already-viewed lines sitting between two unviewed ones. Bounded by the size of the
    /// backlog the user is about to read. See `OutputLine::display_id` in main.rs.
    ClaimedNew { world_index: usize, seqs: Vec<u64> },
    /// This client's ▶ markers in `world_index` are cleared — it switched away, hit Ctrl+L, or
    /// backgrounded. Sent to ONE client, for the same reason as `ClaimedNew`. The lines stay
    /// `viewed`, so no other client picks them up; this is what makes one instance's clear
    /// invisible to another instance, which the old shared watermark could not do.
    ReleasedNew { world_index: usize },
    /// Client → server: this client became visible or went to the background. Backgrounding
    /// is NOT a disconnect (Android keeps its socket open behind `MainActivity.onPause`), so
    /// it has to be signalled explicitly. A non-visible client stops counting as a viewer, so
    /// text arriving meanwhile is unviewed and becomes ▶ when it returns, and it releases the
    /// markers it currently holds.
    ClientVisibility { visible: bool },
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
    /// Client -> server (TF-parity plan Job 22a/P2.7): the client pressed a key that
    /// `GlobalSettingsMsg::tf_bound_keys_json` told it is bound by a `/bind`/`-b`/`-B` or
    /// `key_<name>` macro on the server, so it should run there instead of the client's own
    /// built-in action for that key. `key` is the canonical name (`crate::keynames`
    /// grammar, e.g. `"Esc-x"`, `"^X^R"`); `kbnum` is the client's own pending numeric
    /// prefix, if any, mirrored into the server's TF engine for the duration of the bound
    /// command (tf-help #kbnum) and cleared afterward. The server resolves `key` the exact
    /// same way the console does (`chords::resolve_bound_command`: a `/bind` macro, then a
    /// `key_<name>` macro) and executes the result in this client's own context — output
    /// comes back like any typed command. A `key` with no such binding (stale client-side
    /// cache, race with an `/unbind`) is ignored silently, never an error.
    RunKeyBinding { key: String, kbnum: Option<i64> },
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
        /// Highest seq this world currently owes a caught-up client — `World::
        /// deliverable_high_seq` (PROTOCOL-ROADMAP.md Phase C). Clients send
        /// `RequestWorldState` on every world switch, which makes this the cheapest place to
        /// verify a world the user is about to *look at*: if the client's own contiguous
        /// frontier is below this, it has a hole and asks for a gap-fill on the spot rather
        /// than waiting up to two keepalive cycles for the periodic ack audit to notice.
        /// Switching to a world was previously the one moment nothing checked — `SwitchWorld`
        /// sends no content at all and the client renders purely from its local buffer.
        /// `serde(default)` so an older client that ignores the field is unaffected.
        #[serde(default)]
        deliverable_high_seq: u64,
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

    /// Report a hole in the delivered-seq stream that repeated gap-fill requests failed to
    /// close (client -> server, PROTOCOL-ROADMAP.md Phase F). `[hole_start, hole_end]` is the
    /// run of seqs the client has never received and the server did not return when asked
    /// for them; the client gives up on that run at this point and advances its
    /// `contiguousFrontier` past it, so the pair is a permanent-loss record, not a retry
    /// request.
    ///
    /// This is the one failure mode neither existing audit can see. The server-side ack audit
    /// only knows the client is behind, not which seqs are unfillable, and the server-side
    /// broadcast ledger only proves the server *sent* them — a hole that opens in transit
    /// (a dropped frame, a full outbound channel, a socket blip on Android) is invisible to
    /// both. Logged as `SEQ-HOLE` in `~/.clay/remote.log` so a user's log alone shows how
    /// often output is genuinely being lost and where.
    ReportGap {
        world_index: usize,
        hole_start: u64,
        hole_end: u64,
        attempts: u32,
        source: String,  // "web", "gui", "android", "console"
    },

    /// A client reporting its own Android lifecycle transitions (onCreate / onNewIntent /
    /// onResume / onDestroy), logged server-side as `CLIENT-LIFECYCLE` in `~/.clay/remote.log`.
    ///
    /// Exists because the question "did the app resume, or did it rebuild itself?" is only
    /// answerable from the Android side, and the answer used to require `adb logcat` — which
    /// means it could not be answered at all on a phone the user isn't willing to plug in.
    /// Routing it over the WebSocket the client already holds puts the evidence in the
    /// *desktop's* log instead, where it can just be read.
    ///
    /// `event` is the transition ("onCreate", "onResume", ...); `detail` carries the state
    /// that distinguishes a resume from a restart — the Activity-creation count, and whether
    /// the local server / SSH tunnel were still running at that moment. Buffered client-side
    /// until the socket is up, because the most diagnostic event (onCreate, i.e. the Activity
    /// was rebuilt) necessarily happens before there is any connection to report it on.
    ReportClientLifecycle {
        event: String,
        detail: String,
        source: String,  // "android", "web", "gui"
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
    /// `clamped_by_pending` disambiguates the two very different reasons an `after_seq`
    /// gap-fill can come back short, which `backfill_complete: false` alone conflates:
    ///
    /// - **more history is available right now** — the client should immediately ask again;
    /// - **more is owed but withheld** behind this world's unreleased more-mode backlog
    ///   (`handle_request_scrollback`'s pending clamp) — asking again returns the identical
    ///   answer until the backlog releases.
    ///
    /// Without the distinction a client can only pick one wrong behaviour: re-request in a
    /// tight loop (a livelock the client-side no-progress guard then has to break, which
    /// clears `_gapFillPending` and so loses the `PendingReleased` re-drive the clamp
    /// depends on), or stop and never resume. With it, a clamped reply means "stop asking,
    /// stay armed". `backfill_complete` is deliberately left as-is when clamped (still
    /// `false`) so an older client's behaviour is unchanged by this field's arrival.
    ScrollbackLines { world_index: usize, lines: Vec<TimestampedLine>, #[serde(default)] backfill_complete: bool, #[serde(default)] clamped_by_pending: bool, #[serde(default)] request_id: Option<u64> },
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
    /// Console separator-bar style ("tinyfugue" | "web"). Applied to the running TUI
    /// immediately; persisted to settings.dat by the SaveThemeFile that follows.
    UpdateSeparatorStyle { style: String },

    // Theme editor (server -> client)
    ThemeEditorState {
        themes_json: String,
        theme_names: Vec<String>,
        active_theme: String,
        /// Console separator-bar style ("tinyfugue" | "web"); lives in settings.dat,
        /// not theme.dat, but is edited from the theme editor.
        #[serde(default)]
        separator_style: String,
    },
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

    // ---- Server-push scrollback download (PROTOCOL-ROADMAP.md Phase J) ----
    //
    // Replaces the client-pull backfill pump. The client reports, per world, the highest seq
    // below which it has NO gaps; the server pushes history newest-first until it reaches
    // that seq, spends the Remote Lines budget, or runs off the bottom of its buffer.
    // Completeness becomes the server's job, which is the whole point: eight phases of
    // patching the pull design failed because the client owned it and its bookkeeping kept
    // getting corrupted.
    //
    // NOTE for `message_world_index`: none of these three carry a single `world_index` and
    // none of them must ever appear there. See the comment at that function.
    /// Client -> server. Sent once per connection, AFTER `InitialState` has been applied.
    ///
    /// Deliberately NOT folded into `AuthRequest.resume`, and deliberately keyed by world
    /// NAME rather than array index. `AuthRequest` is sent *before* `InitialState`, so at
    /// that point a cold-started client has an empty `worlds` array and reports nothing at
    /// all (re-downloading history it already holds in its local cache), while a reconnecting
    /// client reports indices taken from the *previous* `InitialState` — which silently land
    /// on the wrong worlds if the world list changed meanwhile.
    ///
    /// `complete` is the explicit done-signal: `true` means "that is the entire list; for any
    /// world absent from it, I hold nothing". A client with many worlds may send several with
    /// `complete: false` and one final `complete: true`; the server accumulates.
    ScrollbackSyncRequest {
        worlds: Vec<ScrollbackClientWorld>,
        complete: bool,
        /// Client viewport height, for the screenful+10 first batch. Carried here rather
        /// than read from `WsClientInfo.viewport_height` because `UpdateViewState` arrives
        /// *after* `InitialState`, so the stored value is still the default 24 at the moment
        /// the first batch is planned.
        #[serde(default)]
        viewport_lines: usize,
        /// Client can decompress a zlib-deflated binary `ScrollbackBatch` frame. Set after
        /// feature-detecting `DecompressionStream`; when false the server sends batches as
        /// ordinary text frames.
        #[serde(default)]
        accepts_deflate: bool,
        /// Protocol generation, 1 today.
        #[serde(default)]
        version: u32,
    },
    /// Client -> server. Acknowledges one delivered cycle and asks for the next. One
    /// continue advances EVERY world, because a batch is a cycle across all worlds rather
    /// than one world's chunk — with 20 worlds, per-world ack-gating would cost 20 round
    /// trips per cycle instead of one.
    ScrollbackContinue { batch_id: u64 },

    /// Server -> client. One cycle's worth of history, covering every world still being
    /// filled. The server waits for the matching `ScrollbackContinue` before sending the
    /// next, and times send->continue on its own clock to decide whether to ramp up or back
    /// off (a client-supplied timestamp would measure clock skew, not transfer duration).
    ScrollbackBatch {
        batch_id: u64,
        worlds: Vec<ScrollbackWorldBatch>,
        /// Worlds that finished during this cycle.
        done: Vec<ScrollbackWorldDone>,
        /// Every world is finished: the client hides its progress badge and sends no further
        /// `ScrollbackContinue`.
        complete: bool,
    },
}

/// One world's entry in a client's `ScrollbackSyncRequest`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScrollbackClientWorld {
    /// World name, not index — see `ScrollbackSyncRequest`'s doc comment.
    pub name: String,
    /// "Don't send me anything at or below this." `None` = the client holds nothing for this
    /// world and wants a full download.
    ///
    /// This is a genuine "everything below here is gapless" claim, which the client can only
    /// make because a completed download collapses its delivered-seq record to a single range
    /// anchored at 0. A frontier derived from an un-collapsed record would be the *top of
    /// whatever contiguous run it happens to hold* — on a cold start, the tail slice from
    /// `InitialState` — and reporting that would make the server stop immediately and
    /// download nothing.
    #[serde(default)]
    pub gapless_seq: Option<u64>,
    /// Bottom/top of the contiguous run the client already holds above `gapless_seq`, so the
    /// server can skip re-sending what `InitialState` (or the client's local cache) just
    /// handed it. Both `None` degenerates to the plain `(world, gapless_seq)` behaviour.
    #[serde(default)]
    pub held_from: Option<u64>,
    #[serde(default)]
    pub held_to: Option<u64>,
}

/// One world's slice of a `ScrollbackBatch`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScrollbackWorldBatch {
    /// Valid at send time; the client resolves by `world_name` and treats this as a hint,
    /// since an index can be retargeted by a world being added or removed mid-download.
    pub world_index: usize,
    pub world_name: String,
    /// ASCENDING by seq *within* a batch, so the client's existing seq-ordered insert works
    /// unchanged. Successive batches walk DOWNWARD: every seq in batch N+1 is strictly below
    /// every seq in batch N for the same world.
    pub lines: Vec<TimestampedLine>,
    /// Progress numerator/denominator, both server-computed. `planned_total` is the exact
    /// number of lines the server intends to send for this world, fixed at plan time — not
    /// `newest_seq - gapless_seq`, which overcounts, because seq ranges legitimately contain
    /// holes (a selective flush discards pending lines whose seqs were already allocated) and
    /// a badge built on it would stall short of 100% and never hide.
    pub delivered: usize,
    pub planned_total: usize,
}

/// Terminal marker for one world's download. The client raises its gapless seq ONLY on
/// receiving this — never on "I stopped receiving lines", which is indistinguishable from a
/// stalled or truncated download and is precisely how the historical frontier-poisoning bugs
/// happened.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScrollbackWorldDone {
    pub world_index: usize,
    pub world_name: String,
    pub reason: ScrollbackDoneReason,
    /// Highest seq delivered during THIS download. The client collapses its delivered-seq
    /// record to `[0, high_seq]`, which is what makes its next `gapless_seq` claim true by
    /// declaration. `None` = nothing was delivered.
    pub high_seq: Option<u64>,
    /// Lowest seq delivered during this download.
    pub low_seq: Option<u64>,
    /// The oldest seq the server still holds in `output_lines` for this world. Without it, a
    /// client whose `gapless_seq` is 500 against a server buffer starting at 3000 can never
    /// advance its frontier past the unreachable 501..2999 hole, and re-requests that same
    /// range on every reconnect forever. With it, the client closes the hole and moves on.
    pub oldest_available_seq: Option<u64>,
    /// The stop point chosen at plan time. If this is BELOW the client's reported
    /// `gapless_seq`, the world's seq space was reset underneath it (a lost `settings.dat`,
    /// a recreated world) and the client must drop its record rather than trust seqs from a
    /// previous epoch.
    pub plan_high_seq: u64,
}

/// Why a world's download stopped.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ScrollbackDoneReason {
    /// Reached the client's `gapless_seq` — it already has everything below.
    ReachedClientSeq,
    /// Spent the "Remote Lines" budget.
    HitLineLimit,
    /// Ran off the bottom of `output_lines`; see `oldest_available_seq`.
    BufferExhausted,
    /// Cancelled: continue timed out twice, the world was flushed or removed, or the client
    /// went away. Nothing is corrupt — the client never raised its gapless seq.
    Aborted,
    /// This server mode can't serve a seq-driven download. Multiuser emits `seq: 0` on every
    /// line, so there is nothing to walk; answering immediately stops the client's progress
    /// badge hanging forever waiting on a batch that will never come.
    Unsupported,
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
    /// Mirrors `OutputLine::archive_sourced` — a `/recall -D` result: archived *text*
    /// delivered as an ordinary live line. Display-only (draws 🛢️ instead of ✨).
    /// Deliberately NOT folded into `from_archive` on the wire: app.js's
    /// `bufferIsCorrupted()` treats any wire `from_archive` as proof of a pre-1.5.23
    /// server and throws the client's whole buffer away.
    #[serde(default)]
    pub archive_sourced: bool,
    /// Mirrors `OutputLine::viewed` — see its doc comment in main.rs. Carried on the wire so a
    /// client hydrating from InitialState/ScrollbackLines knows which lines are still
    /// unclaimed. `serde(default)` = false, matching an older peer that doesn't send it.
    #[serde(default)]
    pub viewed: bool,
    /// Mirrors `OutputLine::display_id` — the client that owns this line's ▶ marker. A client
    /// draws ▶ iff this equals its own id (`ServerHello.client_id`). `serde(default)` = None,
    /// so an older peer simply never shows a marker rather than showing a wrong one.
    #[serde(default)]
    pub display_id: Option<u64>,
}

impl TimestampedLine {
    /// The wire form of a stored line. **Use this rather than building the struct by hand.**
    ///
    /// Every field here has to match what the same line looks like when it arrives by any
    /// other route, because a client dedups and reasons across all of them. Hand-built
    /// copies did not: the five scrollback/world-state sites in `main.rs` passed
    /// `viewed: false, display_id: None` (placeholders from when Phase D added the fields)
    /// while `build_initial_output_lines` passed the real values, and only that one stripped
    /// `\r`.
    ///
    /// The `viewed: false` was user-visible. A world switch backfills, the backfilled lines
    /// claim to be unviewed, so `claimUnviewedLocally()` takes ▶ ownership of all of them and
    /// the whole screenful lights up — then the server's authoritative `ClaimedNew` claims
    /// nothing (it has them viewed) and the markers vanish a round trip later. Only on the
    /// *first* switch to a world, because the optimistic claim sets `viewed = true` on the
    /// client's own copies as it goes.
    ///
    /// `display_id: None` was the other half: a line this client genuinely owns arrives with
    /// its marker stripped, so real ▶ markers go missing depending on how the line was
    /// delivered.
    pub(crate) fn from_output_line(line: &crate::OutputLine) -> Self {
        TimestampedLine {
            // Prefix-free, same as live ServerData broadcasts - the "✨ " client-line marker
            // is added at display time only.
            text: line.text.replace('\r', ""),
            ts: line.timestamp
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            gagged: line.gagged,
            from_server: line.from_server,
            seq: line.seq,
            highlight_color: line.highlight_color.clone(),
            from_archive: line.from_archive,
            archive_sourced: line.archive_sourced,
            viewed: line.viewed,
            display_id: line.display_id,
        }
    }
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
    /// Identifies this world's sequence-number space (`World::seq_epoch`). A client caches
    /// this alongside the buffer and discards that buffer whenever the value it stored does
    /// not match the one here — its seqs then refer to a space that no longer exists.
    ///
    /// This supersedes the `next_seq` comparison above, which is arithmetic and therefore
    /// defeatable: the counter can move back above a stale cached high-water mark (ordinary
    /// output, or an archive load fabricating seqs) and the check falls silent while the
    /// cache is still poisoned. Equality against a random id has no such hole.
    ///
    /// `0` means "this peer does not speak epochs" — an older server, or multiuser, whose
    /// seqs are all hardcoded to 0 anyway. A client must treat 0 as "unknown" and fall back
    /// to the older heuristic rather than comparing it as a real epoch. `serde(default)` gives
    /// exactly that against a server predating this field.
    #[serde(default)]
    pub seq_epoch: u64,
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
    /// Console separator-bar style ("tinyfugue" | "web") — see SeparatorStyle.
    #[serde(default)]
    pub separator_style: String,
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
    /// TF-parity plan Job 22a/P2.7: canonical key names currently answered by a `/bind`/
    /// `-b`/`-B` or `key_<name>` macro (JSON array of strings, e.g. `["Esc-x","^X^R"]`) -
    /// see `App::tf_bound_keys_json`'s doc comment. Tells a web/GUI client which keystrokes
    /// to hand to the server via `WsMessage::RunKeyBinding` instead of running its own
    /// built-in action for them, since TF customizations are engine state that otherwise
    /// never reaches a client at all.
    #[serde(default)]
    pub tf_bound_keys_json: String,
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
    /// Snapshot of `acked_seq` taken at the previous ack audit (PROTOCOL-ROADMAP.md Phase C,
    /// `App::audit_client_acks`): world_index -> the acked seq this client reported one audit
    /// ago. The audit only acts on a world whose ack has NOT advanced since, which is what
    /// distinguishes a client that is genuinely stuck from one that is merely a batch or two
    /// behind on a busy world. Without it, ordinary in-flight lag would fire a resync on
    /// every keepalive.
    pub audit_prev_acked: std::collections::HashMap<usize, u64>,
    /// The acked seq each world's last audit-driven `ResyncRequired` was fired at
    /// (PROTOCOL-ROADMAP.md Phase C). Suppresses re-firing for a world that is stalled at the
    /// same point — a seq the server can genuinely never deliver (trimmed out of the ring,
    /// or consumed by a line that never reached a broadcast) would otherwise produce one
    /// pointless resync per keepalive forever. Cleared for a world once its ack moves.
    pub audit_fired_at: std::collections::HashMap<usize, u64>,
    /// Audits observed since the last `ResyncRequired` for a world still stalled at
    /// `audit_fired_at`'s position. Suppression used to be permanent: one fire per (world,
    /// seq), then silence forever. That is wrong whenever the resync itself failed to arrive
    /// or failed to repair — a dropped `ResyncRequired` (the outbound channel was full, which
    /// is the very condition that produces most stalls), a reply lost to a network blip, or a
    /// client that hadn't finished hydrating. The client then sat permanently behind with the
    /// server having decided it was a lost cause, which is the opposite of what a safety net
    /// should do. Retry on a slow cadence instead: often enough to recover from a lost
    /// message, rare enough that a genuinely unfillable hole costs one message every
    /// `AUDIT_REFIRE_INTERVAL` keepalives rather than one per keepalive.
    pub audit_stall_ticks: std::collections::HashMap<usize, u32>,
    /// In-flight server-push scrollback download for this client, if any
    /// (PROTOCOL-ROADMAP.md Phase J). `None` before the client sends a
    /// `ScrollbackSyncRequest`, and again once every world has finished.
    ///
    /// Lives here rather than in `App::ws_client_worlds`'s `ClientViewState` for three
    /// reasons, in order of weight:
    ///
    /// 1. **Borrow checker.** Every `App` send helper takes `&self` (`ws_send_to_client`,
    ///    `ws_broadcast`). With the state in `App`, driving the pump would need
    ///    `&mut self.ws_client_worlds` while reading `&self.worlds` and calling a `&self`
    ///    sender — a three-way conflict that forces a collect-plan-then-apply dance around
    ///    every mutation. Behind `WebSocketServer::clients` the whole pump runs on `&self`:
    ///    `take_push` under a short lock, drop the guard, plan against `&self.worlds`, send,
    ///    `put_push`.
    /// 2. **Lifetime.** `WsClientInfo` is removed when the socket drops. `ClientViewState`
    ///    deliberately outlives it by `WS_VIEWER_GRACE` so a transport blip doesn't look
    ///    like a world-switch. A download must die with its connection, not linger.
    /// 3. **Precedent.** `acked_seq`, `needs_resync` and the `audit_*` maps are already
    ///    per-client protocol bookkeeping in exactly this struct.
    #[allow(dead_code)]
    pub(crate) push: Option<Box<ScrollbackPush>>,
}

/// Keepalive audits to wait before re-sending a `ResyncRequired` to a client still stalled at
/// the same seq — see `WsClientInfo::audit_stall_ticks`.
pub(crate) const AUDIT_REFIRE_INTERVAL: u32 = 6;

// The `#[allow(dead_code)]` on the download state below and on `WsClientInfo::push` and its
// accessors is temporary scaffolding, not a permanent exemption. Phase J is built bottom-up:
// steps 1-6 land the schema, the per-client state and the pure planner/builder, and nothing
// outside the tests constructs any of it until `drive_scrollback_push` arrives in step 7.
// REMOVE all nine of these then - anything still warning after step 7 is genuinely
// unreachable and should be deleted rather than silenced.

/// Which stage of the download a client is in (PROTOCOL-ROADMAP.md Phase J).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum PushPhase {
    /// First cycle: a screenful + `PUSH_INITIAL_EXTRA` lines per world, to fill the visible
    /// output area before depth-filling starts.
    Initial,
    /// Steady state: `cycle_lines` per world per cycle, ramping.
    Cycling,
}

/// A client's in-flight scrollback download (PROTOCOL-ROADMAP.md Phase J).
///
/// One batch covers every unfinished world and is answered by a single `ScrollbackContinue`,
/// so this is per-client rather than per-(client, world): with 20 worlds, per-world
/// ack-gating would cost 20 round trips per cycle instead of one.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct ScrollbackPush {
    /// Per-world cursors, keyed by world NAME. An index would be retargeted onto a different
    /// world by an add or remove mid-download; the name is re-resolved each cycle.
    pub worlds: Vec<PushWorld>,
    /// Lines per world in the next cycle: starts at `PUSH_CYCLE_START`, ramps by
    /// `PUSH_CYCLE_STEP` up to `PUSH_CYCLE_MAX`.
    pub cycle_lines: usize,
    /// Set once a cycle came back slow, or a batch had to be trimmed to fit the size cap.
    /// The rate never increases again after this — a deliberate asymmetry: a client that has
    /// demonstrated it can't keep up shouldn't be probed repeatedly at a rate it already
    /// failed.
    pub ramp_locked: bool,
    /// `(batch_id, sent_at)` for the cycle awaiting a continue. Timed on the SERVER clock:
    /// a client-supplied timestamp compared against a server one measures clock skew plus
    /// latency, not transfer duration, and on a phone with a drifting clock can come out
    /// negative.
    pub inflight: Option<(u64, std::time::Instant)>,
    pub next_batch_id: u64,
    /// Consecutive overdue continues. At `PUSH_MAX_STALLS` the download aborts.
    pub stalls: u32,
    /// Suspended: the client backgrounded, or a continue is overdue. Cursors are kept, so a
    /// resume picks up exactly where it left off.
    pub parked: bool,
    pub phase: PushPhase,
    /// Viewport height reported in `ScrollbackSyncRequest`, for the first cycle's size.
    pub viewport_lines: usize,
    /// Client can decode a zlib-deflated binary batch frame.
    pub accepts_deflate: bool,
    /// True once a cycle overlapped a park or a visibility change: that cycle's elapsed time
    /// is meaningless as a throughput signal and must not drive the ramp either way.
    /// Without this a single 3-second backgrounding hiccup would pin the rate at
    /// `PUSH_CYCLE_START` for the rest of the download.
    pub timing_invalid: bool,
}

/// One world's cursor within a `ScrollbackPush`.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct PushWorld {
    pub name: String,
    /// The client's reported gapless seq: nothing at or below this is sent. `None` = the
    /// client holds nothing for this world.
    pub floor_seq: Option<u64>,
    /// Inclusive range the client already holds above `floor_seq`, skipped when encountered.
    pub skip: Option<(u64, u64)>,
    /// The next line delivered has `seq < cursor`. Walks downward as the download proceeds.
    pub cursor: u64,
    /// Stop point fixed when the download was planned. Fixing it (rather than chasing the
    /// live tail) is what makes the download terminate on a busy world, and what makes the
    /// push and live streams disjoint so neither needs to dedup against the other.
    pub plan_high_seq: u64,
    /// Oldest seq present in `output_lines` at plan time, reported in the done marker.
    pub oldest_at_plan: Option<u64>,
    /// Remaining "Remote Lines" budget, counted in VISIBLE lines.
    pub budget_left: usize,
    /// Exact number of lines this world's download intends to deliver, for the client's
    /// progress denominator.
    pub planned_total: usize,
    pub delivered: usize,
    /// Highest/lowest seq actually delivered, reported in the done marker. The client raises
    /// its gapless seq to `high_seq` and collapses its record to `[0, high_seq]`.
    pub high_seq: Option<u64>,
    pub low_seq: Option<u64>,
    pub done: bool,
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

    /// Remove a client's in-flight download state so the caller can drive it without holding
    /// the clients lock (PROTOCOL-ROADMAP.md Phase J).
    ///
    /// Deliberately a `take` and not a clone. The lock must not be held across a send — a
    /// send can block on the outbound channel's internals, and every other WS operation
    /// needs this map — so the pump has to work on an owned copy. Taking guarantees that
    /// while one caller is driving a client's pump, no second caller can obtain the same
    /// state and drive it concurrently: the second `take_push` returns `None`. All dispatch
    /// currently happens in a single `select!` per process so there is no real concurrency
    /// today, but this makes the invariant structural rather than incidental.
    ///
    /// Callers MUST pair this with `put_push`, including on early returns, or the client
    /// silently stops downloading.
    #[allow(dead_code)]
    pub(crate) fn take_push(&self, client_id: u64) -> Option<Box<ScrollbackPush>> {
        let mut clients = self.clients.write().unwrap();
        clients.get_mut(&client_id).and_then(|c| c.push.take())
    }

    /// Return a client's download state after driving it. Drops it on the floor if the
    /// client disconnected in the meantime, which is the correct outcome — a download must
    /// not outlive its connection.
    #[allow(dead_code)]
    pub(crate) fn put_push(&self, client_id: u64, state: Box<ScrollbackPush>) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.push = Some(state);
        }
    }

    /// Clear a client's download state without driving it — used when the download aborts.
    #[allow(dead_code)]
    pub(crate) fn clear_push(&self, client_id: u64) {
        let mut clients = self.clients.write().unwrap();
        if let Some(client) = clients.get_mut(&client_id) {
            client.push = None;
        }
    }

    /// `(client_id, when that client next needs attention)` for every client with an active
    /// download. Feeds the shared pacing timer's re-arm so the event loops sleep until there
    /// is genuinely something to do rather than polling.
    ///
    /// A client awaiting a continue is due at its timeout; one that is ready to send is due
    /// immediately; a parked one is not due at all (it is woken by a visibility change, not
    /// by the clock).
    #[allow(dead_code)]
    pub(crate) fn push_deadlines(&self, continue_timeout: std::time::Duration) -> Vec<(u64, std::time::Instant)> {
        let clients = self.clients.read().unwrap();
        clients
            .iter()
            .filter_map(|(&id, c)| {
                let push = c.push.as_ref()?;
                if push.parked {
                    return None;
                }
                match push.inflight {
                    Some((_, sent_at)) => Some((id, sent_at + continue_timeout)),
                    None => Some((id, std::time::Instant::now())),
                }
            })
            .collect()
    }

    /// Free slots in a client's outbound channel. The pump refuses to start a cycle below a
    /// floor, which is the difference between a download that *competes* with live output
    /// and one that *evicts* it: the channel is bounded and a full one discards messages.
    #[allow(dead_code)]
    pub(crate) fn client_channel_capacity(&self, client_id: u64) -> usize {
        let clients = self.clients.read().unwrap();
        clients.get(&client_id).map(|c| c.tx.capacity()).unwrap_or(0)
    }

}

/// Result of auditing one client's ack for one world (PROTOCOL-ROADMAP.md Phase C).
/// See `WebSocketServer::evaluate_ack_audit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckAuditOutcome {
    /// Ack matches what the server owes — nothing to do, and nothing worth logging at
    /// normal verbosity (this is the steady state for every world on every keepalive).
    CaughtUp,
    /// Behind, but the ack moved since the previous audit — ordinary in-flight lag on a
    /// busy world, not a stall. Deliberately does not fire.
    Lagging,
    /// Behind at the same position across two consecutive audits: the caller should send
    /// `ResyncRequired { from_seq: acked }`.
    Fired,
    /// Still stuck at the exact seq we already fired a resync for. Suppressed so a seq the
    /// server genuinely cannot deliver (trimmed out of the ring, or consumed by a line that
    /// never reached a broadcast) costs one message rather than one per keepalive forever.
    StillStalled,
    /// Caught up again after a resync had been fired for this world — the audit worked.
    Recovered,
}

impl AckAuditOutcome {
    /// Whether this outcome is worth an unconditional `remote.log` entry. The two steady
    /// states (`CaughtUp`, `Lagging`) are not: with N worlds and M clients they would write
    /// N*M lines every keepalive and drown the log that exists to diagnose this very class
    /// of bug. `Fired`/`StillStalled`/`Recovered` are transitions and are always logged;
    /// the full live picture is available on demand from `/dump`.
    pub fn is_noteworthy(&self) -> bool {
        matches!(self, AckAuditOutcome::Fired | AckAuditOutcome::StillStalled | AckAuditOutcome::Recovered)
    }

    pub fn label(&self) -> &'static str {
        match self {
            AckAuditOutcome::CaughtUp => "ok",
            AckAuditOutcome::Lagging => "lagging",
            AckAuditOutcome::Fired => "RESYNC-SENT",
            AckAuditOutcome::StillStalled => "still-stalled (suppressed)",
            AckAuditOutcome::Recovered => "recovered",
        }
    }
}

impl WebSocketServer {
    /// Decide, for one client, which worlds have fallen behind and need an audit-driven
    /// resync (PROTOCOL-ROADMAP.md Phase C). `deliverables` is `(world_index,
    /// deliverable_high_seq)` for every world with real seqs, from
    /// `World::deliverable_high_seq`.
    ///
    /// This is the detector the protocol was missing. `ResyncRequired` previously fired
    /// *only* when this client's outbound channel overflowed (`reconcile_resync`), so the
    /// server had no way to notice a client that had quietly stopped keeping up — every
    /// gap detection lived on the client, in a high-water mark that four separate
    /// server-side ordering bugs were each able to poison. Comparing the client's own
    /// reported ack against what the server knows it owes needs no cooperation from the
    /// client's bookkeeping to be correct.
    ///
    /// Returns `(world_index, acked, deliverable, outcome)` per audited world. Every row is
    /// returned, including the uninteresting ones, so the caller decides what's worth
    /// logging — see `AckAuditOutcome` for which are high-signal.
    ///
    /// Bookkeeping (`audit_prev_acked`/`audit_fired_at`) is updated here, so this must be
    /// called exactly once per audit cycle per client.
    pub fn evaluate_ack_audit(&self, client_id: u64, deliverables: &[(usize, u64)]) -> Vec<(usize, u64, u64, AckAuditOutcome)> {
        let mut clients = self.clients.write().unwrap();
        let Some(client) = clients.get_mut(&client_id) else { return Vec::new() };
        if !client.authenticated || !client.received_initial_state {
            return Vec::new();
        }
        let mut out = Vec::new();
        for &(world_index, deliverable) in deliverables {
            // A world this client has never acked at all is deliberately skipped. That's the
            // "InitialState's aggregate line budget ran out before reaching this world" case
            // (see build_initial_state), which the client's own phase-1 backfill already
            // covers; firing a resync from 0 would instead pull the whole in-memory ring for
            // every such world on every connect.
            let Some(&acked) = client.acked_seq.get(&world_index) else { continue };
            if acked == 0 {
                continue;
            }
            let prev = client.audit_prev_acked.insert(world_index, acked);
            if acked >= deliverable {
                // Caught up. Clear any stall record so a future stall at this same seq is
                // still allowed to fire. Whether this is merely "fine" or an actual recovery
                // depends on whether we had previously fired at this world.
                let had_fired = client.audit_fired_at.remove(&world_index).is_some();
                client.audit_stall_ticks.remove(&world_index);
                out.push((world_index, acked, deliverable,
                    if had_fired { AckAuditOutcome::Recovered } else { AckAuditOutcome::CaughtUp }));
                continue;
            }
            // Behind. Only act once it has been behind at the SAME position across two
            // consecutive audits — a client that is still making progress is just lagging,
            // not stuck, and the lines it hasn't acked yet may well still be in flight.
            let stalled = prev == Some(acked);
            let already_fired_here = client.audit_fired_at.get(&world_index) == Some(&acked);
            let outcome = if !stalled {
                AckAuditOutcome::Lagging
            } else if already_fired_here {
                // Still stuck at the seq we already fired at. Count the tick and re-fire
                // every AUDIT_REFIRE_INTERVAL audits rather than staying silent forever —
                // see `audit_stall_ticks`. The counter resets on the re-fire so the cadence
                // stays even.
                let ticks = client.audit_stall_ticks.entry(world_index).or_insert(0);
                *ticks += 1;
                if *ticks >= AUDIT_REFIRE_INTERVAL {
                    *ticks = 0;
                    AckAuditOutcome::Fired
                } else {
                    AckAuditOutcome::StillStalled
                }
            } else {
                client.audit_fired_at.insert(world_index, acked);
                client.audit_stall_ticks.insert(world_index, 0);
                AckAuditOutcome::Fired
            };
            out.push((world_index, acked, deliverable, outcome));
        }
        out
    }

    /// Read-only snapshot of every authenticated client's ack-audit state, for `/dump`
    /// (PROTOCOL-ROADMAP.md Phase C): `(client_id, ip, acked_seq, audit_prev_acked,
    /// audit_fired_at)`. Cloned rather than borrowed so the caller isn't holding the clients
    /// lock while it walks `self.worlds` and writes to a file.
    #[allow(clippy::type_complexity)]
    pub fn ack_audit_snapshot(&self) -> Vec<(u64, String, std::collections::HashMap<usize, u64>, std::collections::HashMap<usize, u64>, std::collections::HashMap<usize, u64>)> {
        let clients = self.clients.read().unwrap();
        let mut out: Vec<_> = clients.iter()
            .filter(|(_, c)| c.authenticated)
            .map(|(&id, c)| (id, c.ip_address.clone(), c.acked_seq.clone(), c.audit_prev_acked.clone(), c.audit_fired_at.clone()))
            .collect();
        out.sort_by_key(|(id, ..)| *id);
        out
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
            audit_prev_acked: std::collections::HashMap::new(),
            audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
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
        let _ = event_tx.send(AppEvent::WsClientMessage(client_id, Box::new(WsMessage::AuthRequest { username: None, password_hash: String::new(), current_world: None, auth_key: None, request_key: false, challenge_response: false, resume: Vec::new(), resume_epochs: Vec::new(), client_version: String::new(), client_uid: String::new() }))).await;
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
                        WsMessage::AuthRequest { username, password_hash: client_hash, auth_key, request_key, challenge_response: uses_challenge, ref client_version, .. } => {
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
                                crate::http::log_remote_event("CLIENT-VERSION", &client_ip,
                                    if client_version.is_empty() { "-" } else { client_version.as_str() });
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
