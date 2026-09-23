//! Slack and Discord "chat worlds".
//!
//! A chat world is attached to one Discord server (guild) or one Slack workspace. Every
//! channel the bot can read shows up in the world's output tagged `#channel` (DMs as
//! `@user`); typed text goes to the world's current *send target*, switched at runtime
//! with `/chat to ...` (see `commands::execute_chat_command`).
//!
//! Shape of a session:
//! - `start()` spawns the protocol task (`discord::run` / `slack::run`) and returns the
//!   `ChatHandle` the caller stores on the world (or the multiuser connection) at once.
//!   Dropping the handle is the *only* shutdown mechanism: every task of the session
//!   watches its `watch` channel and exits when the sender goes away.
//! - The protocol task reports back through `AppEvent::Chat(owner, conn_id, ChatEvent)`.
//!   `App::handle_chat_event` is the single consumer, shared by every event loop.
//! - The task survives transient network loss itself (Discord RESUME, Slack
//!   reconnect) with backoff; only a fatal error or repeated failure ends it with
//!   `ChatEvent::Closed`, after which the world's own Reconnect setting takes over.
//! - Typed text arrives through the world's ordinary `command_tx` as
//!   `WriteCommand::Text`; target switches ride the same channel as
//!   `WriteCommand::Chat(ChatOp)` so their order relative to text is exact.
//!
//! Never await any of this on an event loop - see `App::spawn_world_connect`.

pub mod directory;
pub mod discord;
pub mod render;
pub mod slack;
pub mod spec;
#[cfg(test)]
pub(crate) mod test_support;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use crate::{AppEvent, WriteCommand};
use directory::{ChatDirectory, ChatTarget};

pub const USER_AGENT: &str = concat!("DiscordBot (https://github.com/c-hudson/clay, ", env!("CARGO_PKG_VERSION"), ")");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    Discord,
    Slack,
}

impl ChatKind {
    pub fn from_world_type(t: &crate::WorldType) -> Option<ChatKind> {
        match t {
            crate::WorldType::Discord => Some(ChatKind::Discord),
            crate::WorldType::Slack => Some(ChatKind::Slack),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ChatKind::Discord => "Discord",
            ChatKind::Slack => "Slack",
        }
    }
}

/// Base URLs. Only tests construct anything but `for_kind` (pointing at a local fake
/// server); there is deliberately no environment override, so a token can never be
/// redirected somewhere else by configuration.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub api_base: String,
    /// Discord: overrides the URL `/gateway/bot` returns (tests). Slack: unused (the
    /// socket URL always comes from `apps.connections.open`).
    pub gateway_url: Option<String>,
}

impl Endpoints {
    pub fn for_kind(kind: ChatKind) -> Endpoints {
        match kind {
            ChatKind::Discord => Endpoints { api_base: discord::DEFAULT_API.to_string(), gateway_url: None },
            ChatKind::Slack => Endpoints { api_base: slack::DEFAULT_API.to_string(), gateway_url: None },
        }
    }
}

/// Who a session belongs to: a single-user world (by name), or one user's connection
/// to a world in `--multiuser` mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatOwner {
    World(String),
    Multiuser { world_index: usize, username: String },
}

/// Discord session resume data, kept across transient drops and hot reload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeState {
    pub session_id: String,
    pub seq: u64,
    pub resume_url: String,
}

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub kind: ChatKind,
    pub owner: ChatOwner,
    pub conn_id: u64,
    pub endpoints: Endpoints,
    /// Discord bot token / Slack bot (`xoxb-`) token.
    pub token: String,
    /// Slack app-level (`xapp-`) token for Socket Mode. Unused for Discord.
    pub app_token: String,
    pub server_spec: String,
    /// Initial send target: the session target to restore (hot reload) if any,
    /// else the world's Send To setting.
    pub target_spec: String,
    pub filter: Vec<spec::Spec>,
    pub resume: Option<ResumeState>,
}

impl ChatConfig {
    /// Build a config from a world's settings. `session_target` (the runtime target a
    /// hot reload saved) wins over the Send To setting.
    pub fn from_settings(
        kind: ChatKind,
        owner: ChatOwner,
        conn_id: u64,
        s: &crate::WorldSettings,
        session_target: &str,
        resume: Option<ResumeState>,
    ) -> ChatConfig {
        let (token, app_token, server_spec, send_to, filter) = match kind {
            ChatKind::Discord => (
                s.discord_token.clone(),
                String::new(),
                s.discord_guild.clone(),
                s.discord_channel.clone(),
                s.discord_channels.clone(),
            ),
            ChatKind::Slack => (
                s.slack_token.clone(),
                s.slack_app_token.clone(),
                s.slack_workspace.clone(),
                s.slack_channel.clone(),
                s.slack_channels.clone(),
            ),
        };
        ChatConfig {
            kind,
            owner,
            conn_id,
            endpoints: Endpoints::for_kind(kind),
            token: token.trim().to_string(),
            app_token: app_token.trim().to_string(),
            server_spec,
            target_spec: if session_target.trim().is_empty() { send_to } else { session_target.to_string() },
            filter: spec::parse_list(kind, &filter),
            resume,
        }
    }
}

/// Changes to where typed text goes, sent in-band on the world's `command_tx`.
#[derive(Debug, Clone)]
pub enum ChatOp {
    SetTarget(ChatTarget),
    /// Send one message to a target without changing the current one (`/chat msg`).
    SendTo(ChatTarget, String),
}

#[derive(Debug, Clone, Default)]
pub struct ReadySummary {
    pub bot_name: String,
    pub server_name: String,
    pub channel_count: usize,
    /// The initial send target, if the configured one resolved.
    pub target: Option<ChatTarget>,
    /// Why the configured target didn't resolve (shown to the user).
    pub target_error: Option<String>,
}

pub enum ChatEvent {
    /// A status/progress/warning line for the world. Never runs triggers.
    Status(String),
    /// The session is up. Sent once per `start()`; later reconnects/resumes are
    /// reported as `Status` lines and the world stays connected throughout.
    Ready { cmd_tx: mpsc::Sender<WriteCommand>, summary: ReadySummary },
    /// Other people's messages, rendered. Run through triggers like MUD text.
    Lines(Vec<String>),
    /// Our own messages echoed back by the server: shown, but never fire triggers
    /// (a trigger answering its own output would loop).
    SelfLines(Vec<String>),
    /// The send target changed (label for the prompt/status).
    TargetChanged(ChatTarget),
    /// The session ended. `fatal`: a configuration problem (bad token, missing
    /// intent) that retrying won't fix - auto-reconnect is suppressed.
    Closed { reason: String, fatal: bool },
}

/// Stored on the world (`World::chat`) or the multiuser connection for the life of a
/// session, from `start()` on. Dropping it (the last clone of it) stops every task of
/// the session.
#[derive(Clone)]
pub struct ChatHandle {
    pub kind: ChatKind,
    pub dir: Arc<RwLock<ChatDirectory>>,
    pub resume: Arc<Mutex<Option<ResumeState>>>,
    _shutdown: Arc<watch::Sender<bool>>,
}

impl ChatHandle {
    pub fn resume_state(&self) -> Option<ResumeState> {
        self.resume.lock().ok().and_then(|g| g.clone())
    }
}

/// How the protocol task reports back.
#[derive(Clone)]
pub struct EventSink {
    tx: mpsc::Sender<AppEvent>,
    owner: ChatOwner,
    conn_id: u64,
}

impl EventSink {
    pub fn new(tx: mpsc::Sender<AppEvent>, owner: ChatOwner, conn_id: u64) -> Self {
        EventSink { tx, owner, conn_id }
    }
    pub async fn send(&self, ev: ChatEvent) {
        let _ = self.tx.send(AppEvent::Chat(self.owner.clone(), self.conn_id, ev)).await;
    }
    pub async fn status(&self, text: impl Into<String>) {
        self.send(ChatEvent::Status(text.into())).await;
    }
}

/// Start a session. Returns immediately; store the handle.
pub fn start(cfg: ChatConfig, event_tx: mpsc::Sender<AppEvent>) -> ChatHandle {
    let (sd_tx, sd_rx) = watch::channel(false);
    let dir = Arc::new(RwLock::new(ChatDirectory::new(cfg.kind)));
    let resume = Arc::new(Mutex::new(cfg.resume.clone()));
    let handle = ChatHandle { kind: cfg.kind, dir: dir.clone(), resume: resume.clone(), _shutdown: Arc::new(sd_tx) };
    let sink = EventSink::new(event_tx, cfg.owner.clone(), cfg.conn_id);
    match cfg.kind {
        ChatKind::Discord => {
            tokio::spawn(discord::run(cfg, sink, dir, resume, sd_rx));
        }
        ChatKind::Slack => {
            tokio::spawn(slack::run(cfg, sink, dir, sd_rx));
        }
    }
    handle
}

/// Resolves once the session's `ChatHandle` has been dropped.
pub(crate) async fn stopped(rx: &mut watch::Receiver<bool>) {
    // `changed()` errors once the sender is gone; no value is ever sent.
    while rx.changed().await.is_ok() {}
}

/// Uniform random fraction in [0, 1) for backoff/heartbeat jitter.
pub(crate) fn jitter() -> f64 {
    let mut b = [0u8; 4];
    if getrandom::getrandom(&mut b).is_err() {
        return 0.5;
    }
    u32::from_le_bytes(b) as f64 / (u32::MAX as f64 + 1.0)
}

/// Reconnect delay for the `n`th consecutive failure (0-based): 1s, 2s, 4s ... capped
/// at 60s, +/-20% jitter.
pub(crate) fn backoff(n: u32) -> Duration {
    let base = 2f64.powi(n.min(6) as i32).min(60.0);
    Duration::from_secs_f64(base * (0.8 + 0.4 * jitter()))
}

/// After this many consecutive failed (re)connects the task gives up and hands over
/// to the world's Reconnect setting.
pub(crate) const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// An HTTP API response: status, the `Retry-After`-style headers, and the JSON body
/// (`Null` when empty or not JSON).
pub(crate) struct ApiResponse {
    pub status: u16,
    pub json: serde_json::Value,
}

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Send a request, honouring HTTP 429 rate limits (up to 3 retries): the wait comes
/// from the `Retry-After` header or a JSON `retry_after` (Discord, fractional seconds),
/// capped at 60s. Discord's `X-RateLimit-Remaining: 0` + `Reset-After` is honoured too,
/// *after* the request, so the next call on the same route doesn't earn a 429 at all.
pub(crate) async fn send_rate_limited<F>(build: F) -> Result<ApiResponse, String>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt = 0;
    loop {
        let resp = build().send().await.map_err(|e| describe_reqwest_error(&e))?;
        let status = resp.status().as_u16();
        let header_f64 = |name: &str| -> Option<f64> {
            resp.headers().get(name).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok())
        };
        let retry_header = header_f64("retry-after");
        let remaining = header_f64("x-ratelimit-remaining");
        let reset_after = header_f64("x-ratelimit-reset-after");
        let text = resp.text().await.unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        if status == 429 && attempt < 3 {
            attempt += 1;
            let wait = json.get("retry_after").and_then(|v| v.as_f64()).or(retry_header).unwrap_or(1.0);
            tokio::time::sleep(Duration::from_secs_f64(wait.clamp(0.05, 60.0))).await;
            continue;
        }
        if remaining == Some(0.0) {
            if let Some(r) = reset_after {
                tokio::time::sleep(Duration::from_secs_f64(r.clamp(0.0, 10.0))).await;
            }
        }
        return Ok(ApiResponse { status, json });
    }
}

fn describe_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "the request timed out".to_string()
    } else if e.is_connect() {
        "couldn't reach the server (network down?)".to_string()
    } else {
        e.to_string()
    }
}

/// Connect a WebSocket with a timeout.
pub(crate) async fn ws_connect(
    url: &str,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
    match tokio::time::timeout(Duration::from_secs(20), tokio_tungstenite::connect_async(url)).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(e)) => Err(format!("couldn't open the connection: {}", e)),
        Err(_) => Err("timed out opening the connection".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        for n in 0..10 {
            let d = backoff(n).as_secs_f64();
            let base = 2f64.powi(n.min(6) as i32).min(60.0);
            assert!(d >= base * 0.8 - 1e-9 && d <= base * 1.2 + 1e-9, "n={} d={}", n, d);
        }
    }

    #[test]
    fn jitter_in_range() {
        for _ in 0..100 {
            let j = jitter();
            assert!((0.0..1.0).contains(&j));
        }
    }
}
