//! Discord: gateway session (v10, JSON), REST sender, and the editor's Fetch lookup.
//!
//! Gateway lifecycle (https://discord.com/developers/docs/events/gateway):
//! HELLO -> IDENTIFY (or RESUME) -> READY -> GUILD_CREATE per guild -> dispatches.
//! Heartbeats follow HELLO's interval (first one jittered), carry the last sequence
//! number, and a heartbeat that isn't ACKed before the next is due means a zombie
//! connection: we drop it and RESUME. Close codes are mapped by `close_action`; the
//! ones a user can fix (bad token, Message Content intent off) end the session with a
//! message that says exactly what to do.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

use super::directory::{ChannelInfo, ChannelKind, ChatDirectory, ChatTarget, UserInfo};
use super::render::{self, ellipsize, sanitize_inline};
use super::spec;
use super::{
    backoff, jitter, send_rate_limited, stopped, ws_connect, ChatConfig, ChatEvent, ChatKind, ChatOp,
    EventSink, ReadySummary, ResumeState, MAX_CONSECUTIVE_FAILURES,
};
use crate::websocket::{ChatDirectoryMsg, ChatPickItem};
use crate::WriteCommand;

pub const DEFAULT_API: &str = "https://discord.com/api/v10";
pub const DEFAULT_GATEWAY: &str = "wss://gateway.discord.gg";

/// GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT. MESSAGE_CONTENT is a
/// *privileged* intent - it must be switched on in the Developer Portal or the gateway
/// closes with 4014. GUILD_MEMBERS is deliberately not requested (also privileged, and
/// names are learned from messages instead).
pub const INTENTS: u64 = 1 | (1 << 9) | (1 << 12) | (1 << 15);

/// Discord's per-message character limit.
pub const MESSAGE_LIMIT: usize = 2000;

/// View Channel + Send Messages + Read Message History.
pub const INVITE_PERMISSIONS: u64 = (1 << 10) | (1 << 11) | (1 << 16);

const MSG_CONTENT_FIX: &str = "Discord says this bot isn't allowed to read messages. Open the Developer Portal \
(https://discord.com/developers/applications) -> your application -> Bot -> Privileged Gateway Intents, \
turn on MESSAGE CONTENT INTENT, save, then connect again.";
const BAD_TOKEN_FIX: &str = "Discord rejected the bot token. In the Developer Portal -> your application -> Bot, \
press Reset Token, copy the new token into this world's Token setting (paste it as-is, without a \"Bot \" prefix), \
then connect again.";

/// How a gateway connection ended, and what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEnd {
    /// Reconnect and RESUME the same session (missed events are replayed).
    Resume(String),
    /// Reconnect with a fresh IDENTIFY (the session is gone).
    Reidentify(String),
    /// Configuration problem retrying won't fix; the message tells the user what to do.
    Fatal(String),
    /// Our handle was dropped (disconnect/quit).
    Shutdown,
}

/// Map a gateway close code to what to do.
pub fn close_action(code: u16) -> SessionEnd {
    match code {
        4004 => SessionEnd::Fatal(BAD_TOKEN_FIX.to_string()),
        4014 => SessionEnd::Fatal(MSG_CONTENT_FIX.to_string()),
        4013 => SessionEnd::Fatal("Discord rejected the requested intents (invalid intents) - this is a Clay bug, please report it.".to_string()),
        4010..=4012 => SessionEnd::Fatal(format!(
            "Discord closed the connection with code {} (sharding/API version) - this is a Clay bug, please report it.",
            code
        )),
        4003 | 4007 | 4009 => SessionEnd::Reidentify(format!("Discord ended the session (code {})", code)),
        4000 | 4001 | 4002 | 4005 | 4008 => SessionEnd::Resume(format!("Discord closed the connection (code {})", code)),
        _ => SessionEnd::Resume(format!("the connection closed (code {})", code)),
    }
}

// ============================================================================
// REST
// ============================================================================

#[derive(Clone)]
pub(crate) struct Rest {
    http: reqwest::Client,
    base: String,
    auth: String,
}

#[derive(Debug)]
pub(crate) struct RestError {
    /// 0 = no HTTP response (network).
    pub status: u16,
    pub code: i64,
    pub message: String,
}

impl Rest {
    pub(crate) fn new(base: &str, token: &str) -> Rest {
        // Tolerate a pasted "Bot xxx" - we add the prefix ourselves.
        let token = token.trim();
        let token = token.strip_prefix("Bot ").unwrap_or(token).trim();
        Rest { http: super::http_client(), base: base.trim_end_matches('/').to_string(), auth: format!("Bot {}", token) }
    }

    pub(crate) async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, RestError> {
        let url = format!("{}{}", self.base, path);
        let resp = send_rate_limited(|| {
            let mut rb = self.http.request(method.clone(), &url).header("Authorization", &self.auth);
            if let Some(b) = &body {
                rb = rb.json(b);
            }
            rb
        })
        .await
        .map_err(|e| RestError { status: 0, code: 0, message: e })?;
        if (200..300).contains(&resp.status) {
            Ok(resp.json)
        } else {
            Err(RestError {
                status: resp.status,
                code: resp.json.get("code").and_then(|v| v.as_i64()).unwrap_or(0),
                message: resp.json.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
        }
    }

    async fn get(&self, path: &str) -> Result<Value, RestError> {
        self.call(reqwest::Method::GET, path, None).await
    }
}

impl RestError {
    fn is_auth(&self) -> bool {
        self.status == 401
    }
    /// A sentence describing a failed send to `label`.
    fn describe_send(&self, label: &str) -> String {
        match (self.status, self.code) {
            (0, _) => format!("Couldn't send to {}: {}.", label, self.message),
            // Code-specific arms first: 50007 also arrives as HTTP 403.
            (_, 50007) => format!(
                "{} doesn't accept direct messages from the bot (they must share a server with it and allow DMs from server members).",
                label
            ),
            (_, 50013) | (_, 50001) | (403, _) => format!(
                "The bot isn't allowed to post in {} (it needs the View Channel and Send Messages permissions there).",
                label
            ),
            (_, 10003) | (404, _) => format!("{} no longer exists (or the bot can't see it).", label),
            (_, 10013) => format!("Discord doesn't know the user {}.", label),
            (401, _) => BAD_TOKEN_FIX.to_string(),
            _ => format!("Couldn't send to {}: {} (HTTP {}).", label, if self.message.is_empty() { "error" } else { &self.message }, self.status),
        }
    }
}

// ============================================================================
// Session
// ============================================================================

/// The protocol task for one `chat::start` (see mod.rs).
pub async fn run(
    cfg: ChatConfig,
    sink: EventSink,
    dir: Arc<RwLock<ChatDirectory>>,
    resume: Arc<Mutex<Option<ResumeState>>>,
    shutdown: watch::Receiver<bool>,
) {
    let rest = Rest::new(&cfg.endpoints.api_base, &cfg.token);
    let (cmd_tx, cmd_rx) = mpsc::channel::<WriteCommand>(256);
    tokio::spawn(writer(rest.clone(), dir.clone(), cmd_rx, sink.clone(), shutdown.clone()));

    let mut sess = Session {
        cfg,
        rest,
        dir,
        resume,
        sink,
        cmd_tx,
        shutdown,
        announced: false,
        chosen_guild: None,
        seq: None,
    };
    if let Some(r) = sess.resume.lock().ok().and_then(|g| g.clone()) {
        sess.seq = Some(r.seq);
    }

    let mut failures: u32 = 0;
    let mut lost_reported = false;
    loop {
        // A fresh IDENTIFY needs the gateway URL and costs one of the bot's ~1000
        // daily session starts - check the budget first. A RESUME needs neither.
        let resuming = sess.resume.lock().map(|g| g.is_some()).unwrap_or(false);
        let mut url = sess.cfg.endpoints.gateway_url.clone();
        if !resuming {
            match sess.rest.get("/gateway/bot").await {
                Ok(v) => {
                    let limit = &v["session_start_limit"];
                    let remaining = limit["remaining"].as_i64().unwrap_or(1000);
                    if remaining < 5 {
                        let mins = limit["reset_after"].as_i64().unwrap_or(0) / 60000;
                        sess.close(
                            format!(
                                "Discord's daily connection limit for this bot is nearly used up ({} left). It resets in about {} minutes - connect again after that.",
                                remaining, mins
                            ),
                            true,
                        )
                        .await;
                        return;
                    }
                    if url.is_none() {
                        url = v["url"].as_str().map(|s| s.to_string());
                    }
                }
                Err(e) if e.is_auth() => {
                    sess.close(BAD_TOKEN_FIX.to_string(), true).await;
                    return;
                }
                Err(e) => {
                    failures += 1;
                    if failures >= MAX_CONSECUTIVE_FAILURES {
                        sess.close(format!("Couldn't reach Discord: {}", e.message), false).await;
                        return;
                    }
                    if !sess.sleep(backoff(failures - 1)).await {
                        return;
                    }
                    continue;
                }
            }
        } else if let Some(r) = sess.resume.lock().ok().and_then(|g| g.clone()) {
            if url.is_none() && !r.resume_url.is_empty() {
                url = Some(r.resume_url.clone());
            }
        }
        let url = url.unwrap_or_else(|| DEFAULT_GATEWAY.to_string());

        let (end, got_ready) = sess.connect_once(&url).await;
        if got_ready {
            failures = 0;
        }
        match end {
            SessionEnd::Shutdown => return,
            SessionEnd::Fatal(msg) => {
                sess.close(msg, true).await;
                return;
            }
            SessionEnd::Resume(reason) | SessionEnd::Reidentify(reason) => {
                failures += if got_ready { 0 } else { 1 };
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    sess.close(format!("Lost the connection to Discord: {}", reason), false).await;
                    return;
                }
                if sess.announced && !lost_reported {
                    sess.sink.status(format!("Discord connection interrupted ({}); reconnecting...", reason)).await;
                    lost_reported = true;
                }
                if !sess.sleep(backoff(failures)).await {
                    return;
                }
            }
        }
        if got_ready {
            lost_reported = false;
        }
    }
}

struct Session {
    cfg: ChatConfig,
    rest: Rest,
    dir: Arc<RwLock<ChatDirectory>>,
    resume: Arc<Mutex<Option<ResumeState>>>,
    sink: EventSink,
    cmd_tx: mpsc::Sender<WriteCommand>,
    shutdown: watch::Receiver<bool>,
    /// `ChatEvent::Ready` already sent (once per `start`).
    announced: bool,
    chosen_guild: Option<String>,
    seq: Option<u64>,
}

/// Guild-selection progress between READY and the chosen guild's GUILD_CREATE.
struct GuildWait {
    /// Guild ids from READY not yet seen in a GUILD_CREATE.
    pending: HashSet<String>,
    /// (id, name) of every GUILD_CREATE seen, for error messages.
    seen: Vec<(String, String)>,
    deadline: Instant,
    total: usize,
}

impl Session {
    async fn close(&self, reason: String, fatal: bool) {
        self.sink.send(ChatEvent::Closed { reason, fatal }).await;
    }

    /// Sleep unless shut down first; false = shut down.
    async fn sleep(&mut self, d: Duration) -> bool {
        tokio::select! {
            _ = tokio::time::sleep(d) => true,
            _ = stopped(&mut self.shutdown) => false,
        }
    }

    fn set_resume(&self, f: impl FnOnce(&mut Option<ResumeState>)) {
        if let Ok(mut g) = self.resume.lock() {
            f(&mut g);
        }
    }

    /// One gateway connection. Returns how it ended and whether READY/RESUMED was
    /// reached on it (a connection that got that far resets the failure count).
    async fn connect_once(&mut self, url: &str) -> (SessionEnd, bool) {
        let full = format!("{}/?v=10&encoding=json", url.trim_end_matches('/'));
        let mut ws = match ws_connect(&full).await {
            Ok(ws) => ws,
            Err(e) => return (SessionEnd::Resume(e), false),
        };
        let mut interval: Option<Duration> = None;
        let mut next_beat = Instant::now() + Duration::from_secs(3600);
        let mut ack_pending = false;
        let mut got_ready = false;
        let mut wait: Option<GuildWait> = None;

        loop {
            let beat_at = next_beat;
            let wait_deadline = wait.as_ref().map(|w| w.deadline).unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
            tokio::select! {
                _ = stopped(&mut self.shutdown) => {
                    // Close with 1000: Discord ends the session (no ghost presence).
                    let _ = ws.send(Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: "bye".into() }))).await;
                    return (SessionEnd::Shutdown, got_ready);
                }
                _ = tokio::time::sleep_until(beat_at), if interval.is_some() => {
                    if ack_pending {
                        let _ = ws.send(Message::Close(Some(CloseFrame { code: CloseCode::Library(4000), reason: "zombie".into() }))).await;
                        return (SessionEnd::Resume("Discord stopped answering heartbeats".to_string()), got_ready);
                    }
                    let hb = json!({"op": 1, "d": self.seq});
                    if ws.send(Message::Text(hb.to_string())).await.is_err() {
                        return (SessionEnd::Resume("the connection dropped".to_string()), got_ready);
                    }
                    ack_pending = true;
                    next_beat = Instant::now() + interval.unwrap_or(Duration::from_secs(41));
                }
                _ = tokio::time::sleep_until(wait_deadline), if wait.is_some() => {
                    let w = wait.take().unwrap();
                    if let Some(end) = self.finish_guild_wait(Some(w), true).await {
                        return (end, got_ready);
                    }
                }
                msg = ws.next() => {
                    let text = match msg {
                        None => return (SessionEnd::Resume("the connection dropped".to_string()), got_ready),
                        Some(Err(e)) => return (SessionEnd::Resume(format!("connection error: {}", e)), got_ready),
                        Some(Ok(Message::Close(frame))) => {
                            let code = frame.map(|f| u16::from(f.code)).unwrap_or(1000);
                            let end = close_action(code);
                            if let SessionEnd::Reidentify(_) = end {
                                self.set_resume(|r| *r = None);
                                self.seq = None;
                            }
                            return (end, got_ready);
                        }
                        Some(Ok(Message::Text(t))) => t,
                        Some(Ok(_)) => continue,
                    };
                    let v: Value = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                    if let Some(s) = v["s"].as_u64() {
                        self.seq = Some(s);
                        self.set_resume(|r| if let Some(r) = r { r.seq = s; });
                    }
                    match v["op"].as_u64().unwrap_or(99) {
                        10 => {
                            let ms = v["d"]["heartbeat_interval"].as_u64().unwrap_or(41250);
                            let iv = Duration::from_millis(ms);
                            interval = Some(iv);
                            next_beat = Instant::now() + iv.mul_f64(jitter());
                            let resume = self.resume.lock().ok().and_then(|g| g.clone());
                            let payload = match resume {
                                Some(r) => json!({"op": 6, "d": {"token": token_of(&self.cfg.token), "session_id": r.session_id, "seq": r.seq}}),
                                None => json!({"op": 2, "d": {
                                    "token": token_of(&self.cfg.token),
                                    "intents": INTENTS,
                                    "properties": {"os": std::env::consts::OS, "browser": "clay", "device": "clay"}
                                }}),
                            };
                            if ws.send(Message::Text(payload.to_string())).await.is_err() {
                                return (SessionEnd::Resume("the connection dropped".to_string()), got_ready);
                            }
                        }
                        11 => ack_pending = false,
                        1 => {
                            let hb = json!({"op": 1, "d": self.seq});
                            let _ = ws.send(Message::Text(hb.to_string())).await;
                        }
                        7 => return (SessionEnd::Resume("Discord asked us to reconnect".to_string()), got_ready),
                        9 => {
                            if v["d"].as_bool() == Some(true) {
                                return (SessionEnd::Resume("Discord invalidated the session".to_string()), got_ready);
                            }
                            self.set_resume(|r| *r = None);
                            self.seq = None;
                            // Discord asks for a 1-5s pause before re-identifying.
                            if !self.sleep(Duration::from_secs_f64(1.0 + 4.0 * jitter())).await {
                                return (SessionEnd::Shutdown, got_ready);
                            }
                            return (SessionEnd::Reidentify("Discord ended the session".to_string()), got_ready);
                        }
                        0 => {
                            let t = v["t"].as_str().unwrap_or("");
                            let d = &v["d"];
                            match t {
                                "READY" => {
                                    got_ready = true;
                                    self.on_ready(d);
                                    match self.start_guild_wait(d) {
                                        Ok(w) => {
                                            wait = w;
                                            if wait.is_none() {
                                                if let Some(end) = self.finish_guild_wait(None, false).await {
                                                    return (end, got_ready);
                                                }
                                            }
                                        }
                                        Err(end) => return (end, got_ready),
                                    }
                                }
                                "RESUMED" => {
                                    got_ready = true;
                                    if self.announced {
                                        self.sink.status("Reconnected to Discord (session resumed).").await;
                                    }
                                }
                                "GUILD_CREATE" => {
                                    let id = d["id"].as_str().unwrap_or("").to_string();
                                    let name = d["name"].as_str().unwrap_or("").to_string();
                                    if let Some(w) = wait.as_mut() {
                                        w.pending.remove(&id);
                                        w.seen.push((id.clone(), name.clone()));
                                        if self.chosen_guild.is_none() {
                                            if let Some(s) = spec::parse(ChatKind::Discord, &self.cfg.server_spec) {
                                                if s.id.as_deref() == Some(id.as_str())
                                                    || (s.id.is_none() && name.to_lowercase() == s.name)
                                                {
                                                    self.chosen_guild = Some(id.clone());
                                                }
                                            }
                                        }
                                    }
                                    if self.chosen_guild.as_deref() == Some(id.as_str()) {
                                        populate_guild(&mut self.dir.write().unwrap(), d);
                                        if let Some(w) = wait.take() {
                                            if let Some(end) = self.finish_guild_wait(Some(w), false).await {
                                                return (end, got_ready);
                                            }
                                        }
                                    } else if wait.as_ref().map(|w| w.pending.is_empty()).unwrap_or(false) {
                                        let w = wait.take();
                                        if let Some(end) = self.finish_guild_wait(w, true).await {
                                            return (end, got_ready);
                                        }
                                    }
                                }
                                "CHANNEL_CREATE" | "CHANNEL_UPDATE" | "THREAD_CREATE" | "THREAD_UPDATE" => {
                                    self.on_channel(d, false);
                                }
                                "CHANNEL_DELETE" | "THREAD_DELETE" => self.on_channel(d, true),
                                "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
                                    if self.is_ours(d["guild_id"].as_str()) {
                                        let r = &d["role"];
                                        if let (Some(id), Some(name)) = (r["id"].as_str(), r["name"].as_str()) {
                                            self.dir.write().unwrap().roles.insert(id.to_string(), name.to_string());
                                        }
                                    }
                                }
                                "MESSAGE_CREATE" => self.on_message(d, false).await,
                                "MESSAGE_UPDATE" => {
                                    // Embeds unfurling fire an UPDATE with no new content;
                                    // only a real edit carries content + edited_timestamp.
                                    if d["content"].is_string() && d["edited_timestamp"].is_string() && d["author"].is_object() {
                                        self.on_message(d, true).await;
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn is_ours(&self, guild_id: Option<&str>) -> bool {
        matches!((guild_id, self.chosen_guild.as_deref()), (Some(g), Some(c)) if g == c)
    }

    fn on_ready(&mut self, d: &Value) {
        let session_id = d["session_id"].as_str().unwrap_or("").to_string();
        let resume_url = d["resume_gateway_url"].as_str().unwrap_or("").to_string();
        let seq = self.seq.unwrap_or(0);
        self.set_resume(|r| *r = Some(ResumeState { session_id, seq, resume_url }));
        let mut dir = self.dir.write().unwrap();
        dir.bot_id = d["user"]["id"].as_str().unwrap_or("").to_string();
        dir.bot_name = d["user"]["username"].as_str().unwrap_or("").to_string();
        let bot = UserInfo { id: dir.bot_id.clone(), username: dir.bot_name.clone(), display: dir.bot_name.clone() };
        dir.upsert_user(bot);
    }

    /// After READY: decide which guild this world is, or wait for GUILD_CREATEs to
    /// learn names. `Ok(None)` = decided already and nothing to wait for.
    fn start_guild_wait(&mut self, d: &Value) -> Result<Option<GuildWait>, SessionEnd> {
        let ids: Vec<String> = d["guilds"]
            .as_array()
            .map(|a| a.iter().filter_map(|g| g["id"].as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        if ids.is_empty() {
            return Err(SessionEnd::Fatal(
                "The bot isn't in any Discord server yet. Use Fetch in the world editor to get its invite link, add it to your server, then connect again."
                    .to_string(),
            ));
        }
        let wanted = spec::parse(ChatKind::Discord, &self.cfg.server_spec);
        // A re-identify keeps the guild chosen the first time.
        if self.chosen_guild.is_none() {
            match &wanted {
                Some(s) if s.id.is_some() => self.chosen_guild = s.id.clone(),
                None if ids.len() == 1 => self.chosen_guild = Some(ids[0].clone()),
                _ => {}
            }
        }
        Ok(Some(GuildWait {
            pending: ids.iter().cloned().collect(),
            seen: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(8),
            total: ids.len(),
        }))
    }

    /// The chosen guild's data arrived (or the wait timed out/ran out of guilds).
    /// Sends `Ready` on the first success; `Some(end)` = give up.
    async fn finish_guild_wait(&mut self, wait: Option<GuildWait>, exhausted: bool) -> Option<SessionEnd> {
        let populated = self.dir.read().unwrap().server.as_ref().map(|(id, _)| Some(id) == self.chosen_guild.as_ref()).unwrap_or(false);
        if !populated {
            if !exhausted {
                return None;
            }
            let seen = wait.map(|w| (w.seen, w.total)).unwrap_or_default();
            let names: Vec<String> = seen.0.iter().map(|(id, n)| format!("{} ({})", n, id)).collect();
            let msg = match (&self.chosen_guild, self.cfg.server_spec.trim().is_empty()) {
                (Some(id), _) => format!(
                    "The bot isn't in the server {} (or Discord didn't send it). Servers it is in: {}.",
                    id,
                    names.join(", ")
                ),
                (None, true) => format!(
                    "The bot is in {} servers - choose one with the Server setting (use Fetch in the world editor). Servers: {}.",
                    seen.1,
                    names.join(", ")
                ),
                (None, false) => format!(
                    "No server named '{}' - the bot is in: {}.",
                    self.cfg.server_spec.trim(),
                    names.join(", ")
                ),
            };
            return Some(SessionEnd::Fatal(msg));
        }
        if self.announced {
            self.sink.status("Reconnected to Discord.").await;
            return None;
        }
        self.announced = true;
        let (summary, target) = {
            let dir = self.dir.read().unwrap();
            let mut summary = ReadySummary {
                bot_name: dir.bot_name.clone(),
                server_name: dir.server.as_ref().map(|s| s.1.clone()).unwrap_or_default(),
                channel_count: dir.sorted_channels().len(),
                ..Default::default()
            };
            let target = if self.cfg.target_spec.trim().is_empty() {
                None
            } else {
                match dir.resolve_target(&self.cfg.target_spec) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        summary.target_error = Some(e);
                        None
                    }
                }
            };
            summary.target = target.clone();
            (summary, target)
        };
        if let Some(t) = target {
            let _ = self.cmd_tx.send(WriteCommand::Chat(ChatOp::SetTarget(t))).await;
        }
        self.sink.send(ChatEvent::Ready { cmd_tx: self.cmd_tx.clone(), summary }).await;
        None
    }

    fn on_channel(&mut self, d: &Value, deleted: bool) {
        let id = d["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            return;
        }
        let mut dir = self.dir.write().unwrap();
        if d["type"].as_u64() == Some(1) {
            // DM channel created for us (e.g. someone DMed the bot).
            if let Some(u) = d["recipients"].as_array().and_then(|a| a.first()) {
                let uid = u["id"].as_str().unwrap_or("").to_string();
                dir.upsert_user(user_from(u, None));
                dir.dm_channels.insert(id, uid);
            }
            return;
        }
        if !matches!((d["guild_id"].as_str(), self.chosen_guild.as_deref()), (Some(g), Some(c)) if g == c) {
            return;
        }
        if deleted {
            dir.channels.remove(&id);
        } else if let Some(c) = channel_from(d) {
            dir.channels.insert(c.id.clone(), c);
        }
    }

    async fn on_message(&mut self, d: &Value, edited: bool) {
        let guild = d["guild_id"].as_str();
        if guild.is_some() && !self.is_ours(guild) {
            return; // another server the bot is in - not this world
        }
        let channel_id = d["channel_id"].as_str().unwrap_or("").to_string();
        // An unknown channel in our guild is usually a thread created while we were
        // away - look it up once so the tag is a name, not a number.
        let unknown = guild.is_some() && !self.dir.read().unwrap().channels.contains_key(&channel_id);
        if unknown && !channel_id.is_empty() {
            if let Ok(Ok(c)) = tokio::time::timeout(
                Duration::from_secs(4),
                self.rest.get(&format!("/channels/{}", channel_id)),
            )
            .await
            {
                if let Some(info) = channel_from(&c) {
                    self.dir.write().unwrap().channels.insert(info.id.clone(), info);
                }
            }
        }
        let rendered = {
            let mut dir = self.dir.write().unwrap();
            if !dir.passes_filter(&channel_id, &self.cfg.filter) && guild.is_some() {
                return;
            }
            render_message(&mut dir, d, edited)
        };
        let Some((own, lines)) = rendered else { return };
        if lines.is_empty() {
            return;
        }
        self.sink.send(if own { ChatEvent::SelfLines(lines) } else { ChatEvent::Lines(lines) }).await;
    }
}

fn token_of(t: &str) -> String {
    let t = t.trim();
    t.strip_prefix("Bot ").unwrap_or(t).trim().to_string()
}

/// Discord channel type -> our kind; `None` for types we don't list (DMs are handled
/// separately; forum/media parents can't be posted to directly).
fn kind_of(t: u64) -> Option<ChannelKind> {
    match t {
        0 => Some(ChannelKind::Text),
        2 | 13 => Some(ChannelKind::VoiceText),
        4 | 15 | 16 => Some(ChannelKind::Category),
        5 => Some(ChannelKind::News),
        10..=12 => Some(ChannelKind::Thread),
        _ => None,
    }
}

fn channel_from(c: &Value) -> Option<ChannelInfo> {
    let kind = kind_of(c["type"].as_u64()?)?;
    Some(ChannelInfo {
        id: c["id"].as_str()?.to_string(),
        name: sanitize_inline(c["name"].as_str().unwrap_or("?")),
        kind,
        parent_id: c["parent_id"].as_str().map(|s| s.to_string()),
        position: c["position"].as_i64().unwrap_or(0),
        can_send: None,
    })
}

fn user_from(u: &Value, member: Option<&Value>) -> UserInfo {
    let username = u["username"].as_str().unwrap_or("").to_string();
    let display = member
        .and_then(|m| m["nick"].as_str())
        .or_else(|| u["global_name"].as_str())
        .unwrap_or(&username)
        .to_string();
    UserInfo { id: u["id"].as_str().unwrap_or("").to_string(), username: sanitize_inline(&username), display: sanitize_inline(&display) }
}

/// Fill the directory from the chosen guild's GUILD_CREATE (or a REST guild).
pub(crate) fn populate_guild(dir: &mut ChatDirectory, g: &Value) {
    let id = g["id"].as_str().unwrap_or("").to_string();
    let name = sanitize_inline(g["name"].as_str().unwrap_or("?"));
    dir.server = Some((id, name));
    dir.channels.clear();
    for list in ["channels", "threads"] {
        if let Some(a) = g[list].as_array() {
            for c in a {
                if let Some(info) = channel_from(c) {
                    dir.channels.insert(info.id.clone(), info);
                }
            }
        }
    }
    if let Some(a) = g["roles"].as_array() {
        for r in a {
            if let (Some(rid), Some(rn)) = (r["id"].as_str(), r["name"].as_str()) {
                dir.roles.insert(rid.to_string(), sanitize_inline(rn));
            }
        }
    }
    if let Some(a) = g["members"].as_array() {
        for m in a {
            dir.upsert_user(user_from(&m["user"], Some(m)));
        }
    }
}

fn mention_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<(@!?|#|@&)(\d{15,21})>").unwrap())
}

/// Replace user/channel/role mentions with readable names. Custom emoji (`<:x:id>`)
/// and timestamps (`<t:...>`) are left for `encoding::convert_discord_emojis*` /
/// `util::parse_discord_timestamps`, which the display paths already run.
pub fn resolve_mentions(dir: &ChatDirectory, text: &str) -> String {
    mention_re()
        .replace_all(text, |c: &regex::Captures| {
            let id = &c[2];
            match &c[1] {
                "#" => dir.channel_label(id),
                "@&" => format!("@{}", dir.roles.get(id).cloned().unwrap_or_else(|| "role".to_string())),
                _ => format!("@{}", dir.user_display(id)),
            }
        })
        .into_owned()
}

/// Render a MESSAGE_CREATE/UPDATE. Returns (is_own_message, lines), or None for
/// message types that aren't worth showing. Also learns users and DM channels.
pub fn render_message(dir: &mut ChatDirectory, d: &Value, edited: bool) -> Option<(bool, Vec<String>)> {
    let channel_id = d["channel_id"].as_str().unwrap_or("").to_string();
    let author = &d["author"];
    let author_id = author["id"].as_str().unwrap_or("").to_string();
    dir.upsert_user(user_from(author, d.get("member")));
    if let Some(ms) = d["mentions"].as_array() {
        for m in ms {
            dir.upsert_user(user_from(m, m.get("member")));
        }
    }
    let own = !dir.bot_id.is_empty() && author_id == dir.bot_id;
    if d["guild_id"].is_null() && !own && !dir.dm_channels.contains_key(&channel_id) {
        dir.dm_channels.insert(channel_id.clone(), author_id.clone());
    }
    let tag = dir.channel_label(&channel_id);
    let display = dir.user_display(&author_id);
    let ty = d["type"].as_u64().unwrap_or(0);
    let action = match ty {
        0 | 19 | 20 | 23 => None,
        7 => Some(format!("{} joined the server", display)),
        6 => Some(format!("{} pinned a message", display)),
        8 => Some(format!("{} boosted the server", display)),
        9..=11 => Some(format!("{} boosted the server to a new level", display)),
        18 => Some(format!("{} started a thread: {}", display, d["content"].as_str().unwrap_or(""))),
        _ => return None,
    };
    if !own {
        let target = match dir.dm_channels.get(&channel_id).cloned() {
            Some(uid) => ChatTarget { channel_id: Some(channel_id.clone()), user_id: Some(uid), label: tag.clone() },
            None => ChatTarget { channel_id: Some(channel_id.clone()), user_id: None, label: tag.clone() },
        };
        dir.last_inbound = Some(target);
    }
    if let Some(a) = action {
        return Some((own, vec![render::action_line(&tag, &a)]));
    }
    let body = resolve_mentions(dir, d["content"].as_str().unwrap_or(""));
    let mut prefix = String::new();
    if edited {
        prefix.push_str("(edited)");
    }
    let refd = &d["referenced_message"];
    if refd.is_object() {
        let who = dir.user_display(refd["author"]["id"].as_str().unwrap_or(""));
        let quote = ellipsize(&resolve_mentions(dir, refd["content"].as_str().unwrap_or("")), 40);
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        prefix.push_str(&format!("(\u{21aa} {}: {})", who, quote));
    }
    let mut extras = Vec::new();
    if let Some(a) = d["attachments"].as_array() {
        for att in a {
            extras.push(format!(
                "[file: {} {}]",
                att["filename"].as_str().unwrap_or("file"),
                att["url"].as_str().unwrap_or("")
            ));
        }
    }
    if let Some(a) = d["sticker_items"].as_array() {
        for s in a {
            extras.push(format!("[sticker: {}]", s["name"].as_str().unwrap_or("?")));
        }
    }
    if body.trim().is_empty() {
        if let Some(a) = d["embeds"].as_array() {
            for e in a {
                let title = e["title"].as_str().unwrap_or("");
                let url = e["url"].as_str().unwrap_or("");
                if !title.is_empty() || !url.is_empty() {
                    extras.push(format!("[embed: {} {}]", title, url).replace("  ", " "));
                }
            }
        }
    }
    if body.trim().is_empty() && extras.is_empty() && prefix.is_empty() {
        return None;
    }
    Some((own, render::message_lines(&tag, &display, &prefix, &body, &extras)))
}

// ============================================================================
// Writer (outgoing messages)
// ============================================================================

async fn writer(
    rest: Rest,
    dir: Arc<RwLock<ChatDirectory>>,
    mut rx: mpsc::Receiver<WriteCommand>,
    sink: EventSink,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut target: Option<ChatTarget> = None;
    loop {
        let cmd = tokio::select! {
            _ = stopped(&mut shutdown) => return,
            c = rx.recv() => match c { Some(c) => c, None => return },
        };
        match cmd {
            WriteCommand::Text(text) => match target.as_mut() {
                Some(t) => send_text(&rest, &dir, t, &text, &sink).await,
                None => {
                    sink.status("No send target - pick one with /chat to <channel> (see /chat channels).").await;
                }
            },
            WriteCommand::Chat(ChatOp::SetTarget(t)) => {
                target = Some(t.clone());
                sink.send(ChatEvent::TargetChanged(t)).await;
            }
            WriteCommand::Chat(ChatOp::SendTo(mut t, text)) => send_text(&rest, &dir, &mut t, &text, &sink).await,
            WriteCommand::Shutdown => return,
            // Telnet-level (keepalive NOP) or encoding changes: never chat text.
            WriteCommand::Raw(_) | WriteCommand::SetEncoding(_) => {}
        }
    }
}

async fn send_text(rest: &Rest, dir: &Arc<RwLock<ChatDirectory>>, t: &mut ChatTarget, text: &str, sink: &EventSink) {
    if text.trim().is_empty() {
        return;
    }
    let channel = match &t.channel_id {
        Some(c) => c.clone(),
        None => {
            let Some(uid) = t.user_id.clone() else { return };
            match rest.call(reqwest::Method::POST, "/users/@me/channels", Some(json!({"recipient_id": uid}))).await {
                Ok(v) => {
                    let cid = v["id"].as_str().unwrap_or("").to_string();
                    if cid.is_empty() {
                        sink.status(format!("Couldn't open a DM with {}.", t.label)).await;
                        return;
                    }
                    dir.write().unwrap().dm_channels.insert(cid.clone(), uid);
                    t.channel_id = Some(cid.clone());
                    cid
                }
                Err(e) => {
                    sink.status(e.describe_send(&t.label)).await;
                    return;
                }
            }
        }
    };
    for chunk in render::split_message(text, MESSAGE_LIMIT) {
        // allowed_mentions: user pings only - text typed (or produced by a trigger)
        // containing @everyone/@here or a role mention must never mass-ping.
        let body = json!({"content": chunk, "allowed_mentions": {"parse": ["users"]}});
        if let Err(e) = rest.call(reqwest::Method::POST, &format!("/channels/{}/messages", channel), Some(body)).await {
            sink.status(e.describe_send(&t.label)).await;
            return;
        }
    }
}

// ============================================================================
// Fetch (world editor picker)
// ============================================================================

/// Look up what the editor's pickers need: the bot, its servers, and the chosen
/// server's channels. `server_spec` picks the server (empty + exactly one = that one).
pub async fn lookup(api_base: &str, token: &str, server_spec: &str) -> ChatDirectoryMsg {
    let mut out = ChatDirectoryMsg::default();
    if token.trim().is_empty() {
        out.error = "Enter the bot token first.".to_string();
        return out;
    }
    let rest = Rest::new(api_base, token);
    let me = match rest.get("/users/@me").await {
        Ok(v) => v,
        Err(e) if e.is_auth() => {
            out.error = BAD_TOKEN_FIX.to_string();
            return out;
        }
        Err(e) => {
            out.error = format!("Couldn't reach Discord: {}", if e.message.is_empty() { format!("HTTP {}", e.status) } else { e.message });
            return out;
        }
    };
    out.bot_name = sanitize_inline(me["username"].as_str().unwrap_or(""));
    if let Ok(app) = rest.get("/applications/@me").await {
        let flags = app["flags"].as_u64().unwrap_or(0);
        // GATEWAY_MESSAGE_CONTENT (1<<19) or its _LIMITED variant (1<<18).
        if flags & ((1 << 19) | (1 << 18)) == 0 {
            out.warnings.push(MSG_CONTENT_FIX.to_string());
        }
        if let Some(id) = app["id"].as_str() {
            out.invite_url = format!(
                "https://discord.com/oauth2/authorize?client_id={}&scope=bot&permissions={}",
                id, INVITE_PERMISSIONS
            );
        }
    }
    let guilds = match rest.get("/users/@me/guilds").await {
        Ok(v) => v.as_array().cloned().unwrap_or_default(),
        Err(e) => {
            out.error = format!("Couldn't list the bot's servers: {}", e.message);
            return out;
        }
    };
    for g in &guilds {
        let id = g["id"].as_str().unwrap_or("").to_string();
        let name = sanitize_inline(g["name"].as_str().unwrap_or("?"));
        out.servers.push(ChatPickItem { value: spec::picker_value(&name, &id), label: name.clone(), id, name, ..Default::default() });
    }
    if out.servers.is_empty() {
        out.error = "The bot isn't in any server yet - open the invite link to add it to your server, then Fetch again.".to_string();
        out.ok = true; // token is fine; show the invite link
        return out;
    }
    let wanted = spec::parse(ChatKind::Discord, server_spec);
    let chosen = match &wanted {
        Some(s) => out
            .servers
            .iter()
            .find(|g| s.id.as_deref() == Some(g.id.as_str()) || (s.id.is_none() && g.name.to_lowercase() == s.name))
            .cloned(),
        None if out.servers.len() == 1 => out.servers.first().cloned(),
        None => None,
    };
    out.ok = true;
    let Some(chosen) = chosen else {
        if wanted.is_some() {
            out.warnings.push(format!("The bot isn't in a server matching '{}'. Pick one from the list.", server_spec.trim()));
        }
        return out;
    };
    out.selected_server = chosen.value.clone();
    match rest.get(&format!("/guilds/{}/channels", chosen.id)).await {
        Ok(v) => {
            let mut dir = ChatDirectory::new(ChatKind::Discord);
            for c in v.as_array().cloned().unwrap_or_default() {
                if let Some(info) = channel_from(&c) {
                    dir.channels.insert(info.id.clone(), info);
                }
            }
            for c in dir.sorted_channels() {
                let label = match dir.category_name(c) {
                    Some(cat) => format!("{} / #{}", cat, c.name),
                    None => format!("#{}", c.name),
                };
                out.channels.push(ChatPickItem {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    value: spec::picker_value(&format!("#{}", c.name), &c.id),
                    label,
                    kind: match c.kind {
                        ChannelKind::News => "news",
                        ChannelKind::VoiceText => "voice",
                        _ => "text",
                    }
                    .to_string(),
                    can_send: None,
                });
            }
        }
        Err(e) => out.warnings.push(format!("Couldn't list the channels: {}", e.message)),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::directory::tests::sample;

    #[test]
    fn intents_value() {
        assert_eq!(INTENTS, 37377);
        assert_eq!(INVITE_PERMISSIONS, 68608);
    }

    #[test]
    fn close_codes() {
        assert!(matches!(close_action(4004), SessionEnd::Fatal(m) if m.contains("Reset Token")));
        assert!(matches!(close_action(4014), SessionEnd::Fatal(m) if m.contains("MESSAGE CONTENT INTENT")));
        assert!(matches!(close_action(4013), SessionEnd::Fatal(_)));
        assert!(matches!(close_action(4009), SessionEnd::Reidentify(_)));
        assert!(matches!(close_action(4000), SessionEnd::Resume(_)));
        assert!(matches!(close_action(1001), SessionEnd::Resume(_)));
    }

    #[test]
    fn token_prefix_tolerated() {
        assert_eq!(token_of("  Bot abc.def "), "abc.def");
        assert_eq!(token_of("abc"), "abc");
        assert_eq!(Rest::new("x", "Bot t").auth, "Bot t");
    }

    #[test]
    fn mentions() {
        let mut d = sample();
        d.roles.insert("400000000000000001".into(), "Mods".into());
        let s = resolve_mentions(
            &d,
            "hi <@200000000000000001> and <@!200000000000000002> see <#100000000000000003> ping <@&400000000000000001> <:wave:123456789012345678> <t:1700000000:R>",
        );
        assert_eq!(s, "hi @Alice W and @bob see #random ping @Mods <:wave:123456789012345678> <t:1700000000:R>");
    }

    fn msg(v: Value) -> Value {
        v
    }

    #[test]
    fn render_channel_message() {
        let mut d = sample();
        d.bot_id = "999999999999999999".into();
        let m = msg(json!({
            "type": 0, "channel_id": "100000000000000003", "guild_id": "1",
            "author": {"id": "200000000000000003", "username": "carol", "global_name": "Carol"},
            "member": {"nick": "Caz"},
            "content": "hello <@200000000000000001>\nsecond line",
            "attachments": [{"filename": "a.png", "url": "https://cdn/a.png"}]
        }));
        let (own, lines) = render_message(&mut d, &m, false).unwrap();
        assert!(!own);
        assert_eq!(lines, vec!["#random <Caz> hello @Alice W", "  second line", "  [file: a.png https://cdn/a.png]"]);
        assert_eq!(d.user_display("200000000000000003"), "Caz");
        assert_eq!(d.last_inbound.as_ref().unwrap().label, "#random");
    }

    #[test]
    fn render_dm_reply_edit_own() {
        let mut d = sample();
        d.bot_id = "999999999999999999".into();
        // DM from a new user: learns the DM channel, tags @user.
        let m = json!({"type": 0, "channel_id": "300000000000000009",
            "author": {"id": "200000000000000004", "username": "dave"}, "content": "psst"});
        let (_, lines) = render_message(&mut d, &m, false).unwrap();
        assert_eq!(lines, vec!["@dave <dave> psst"]);
        assert_eq!(d.last_inbound.as_ref().unwrap().user_id.as_deref(), Some("200000000000000004"));
        // Reply + edit prefix.
        let m = json!({"type": 19, "channel_id": "100000000000000002", "guild_id": "1",
            "author": {"id": "200000000000000002", "username": "bob"}, "content": "yes",
            "referenced_message": {"author": {"id": "200000000000000001"}, "content": "is this thing on? it's a long question here"}});
        let (_, lines) = render_message(&mut d, &m, true).unwrap();
        assert_eq!(lines, vec!["#general <bob> (edited) (\u{21aa} Alice W: is this thing on? it's a long question\u{2026}) yes"]);
        // Own message.
        let m = json!({"type": 0, "channel_id": "100000000000000002", "guild_id": "1",
            "author": {"id": "999999999999999999", "username": "clay"}, "content": "from me"});
        let (own, _) = render_message(&mut d, &m, false).unwrap();
        assert!(own);
        // System join.
        let m = json!({"type": 7, "channel_id": "100000000000000002", "guild_id": "1",
            "author": {"id": "200000000000000005", "username": "eve"}, "content": ""});
        assert_eq!(render_message(&mut d, &m, false).unwrap().1, vec!["#general * eve joined the server"]);
        // Empty content, nothing else: nothing to show.
        let m = json!({"type": 0, "channel_id": "100000000000000002", "guild_id": "1",
            "author": {"id": "200000000000000005", "username": "eve"}, "content": ""});
        assert!(render_message(&mut d, &m, false).is_none());
    }

    // ---- End-to-end against a local fake Discord (REST + gateway) ----

    use crate::chat::test_support::{next_chat, next_json, send_json, FakeRest, FakeWs};
    use crate::chat::{ChatOwner, Endpoints};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const BOT: &str = "900000000000000001";
    const GUILD: &str = "800000000000000001";
    const GENERAL: &str = "700000000000000001";
    const RANDOM: &str = "700000000000000002";

    fn cfg(api: &str, gateway: Option<String>, target: &str) -> ChatConfig {
        ChatConfig {
            kind: ChatKind::Discord,
            owner: ChatOwner::World("D".into()),
            conn_id: 1,
            endpoints: Endpoints { api_base: api.to_string(), gateway_url: gateway },
            token: "tok".into(),
            app_token: String::new(),
            server_spec: String::new(),
            target_spec: target.into(),
            filter: Vec::new(),
            resume: None,
        }
    }

    fn guild_create() -> Value {
        json!({"op": 0, "t": "GUILD_CREATE", "s": 2, "d": {
            "id": GUILD, "name": "Test Server",
            "channels": [
                {"id": GENERAL, "name": "general", "type": 0, "position": 0},
                {"id": RANDOM, "name": "random", "type": 0, "position": 1}
            ],
            "roles": [], "members": []
        }})
    }

    #[tokio::test]
    async fn gateway_session_end_to_end() {
        let posts = Arc::new(AtomicUsize::new(0));
        let posts2 = posts.clone();
        let ws = FakeWs::start().await;
        let ws_url = ws.url.clone();
        let rest = FakeRest::start(Arc::new(move |method, path, _body| {
            match (method, path) {
                ("GET", "/gateway/bot") => (200, json!({"url": ws_url, "session_start_limit": {"remaining": 900, "reset_after": 0}}).to_string(), vec![]),
                ("POST", p) if p == format!("/channels/{}/messages", GENERAL) => {
                    // First send is rate limited once.
                    if posts2.fetch_add(1, Ordering::SeqCst) == 0 {
                        (429, json!({"retry_after": 0.05, "message": "rate limited"}).to_string(), vec![])
                    } else {
                        (200, json!({"id": "1"}).to_string(), vec![])
                    }
                }
                _ => (404, json!({"message": "not found", "code": 0}).to_string(), vec![]),
            }
        }))
        .await;

        let (tx, mut rx) = mpsc::channel(64);
        let handle = crate::chat::start(cfg(&rest.base, None, "#general"), tx);

        // HELLO -> IDENTIFY with the right intents.
        let mut g = ws.accept().await;
        send_json(&mut g, json!({"op": 10, "d": {"heartbeat_interval": 45000}})).await;
        let identify = next_json(&mut g).await;
        assert_eq!(identify["op"], 2);
        assert_eq!(identify["d"]["intents"], 37377);
        assert_eq!(identify["d"]["token"], "tok");
        // READY + GUILD_CREATE -> Ready with the target resolved by name.
        send_json(&mut g, json!({"op": 0, "t": "READY", "s": 1, "d": {
            "session_id": "S1", "resume_gateway_url": ws.url, "user": {"id": BOT, "username": "claybot"},
            "guilds": [{"id": GUILD, "unavailable": true}]
        }})).await;
        send_json(&mut g, guild_create()).await;
        let cmd_tx = match next_chat(&mut rx).await {
            ChatEvent::Ready { cmd_tx, summary } => {
                assert_eq!(summary.server_name, "Test Server");
                assert_eq!(summary.bot_name, "claybot");
                assert_eq!(summary.target.as_ref().map(|t| t.label.as_str()), Some("#general"));
                cmd_tx
            }
            _ => panic!("expected Ready"),
        };
        assert!(matches!(next_chat(&mut rx).await, ChatEvent::TargetChanged(t) if t.label == "#general"));

        // Someone else's message -> Lines; ours -> SelfLines.
        send_json(&mut g, json!({"op": 0, "t": "MESSAGE_CREATE", "s": 3, "d": {
            "type": 0, "guild_id": GUILD, "channel_id": RANDOM, "content": "hi <@900000000000000001>",
            "author": {"id": "600000000000000001", "username": "alice"}
        }})).await;
        match next_chat(&mut rx).await {
            ChatEvent::Lines(l) => assert_eq!(l, vec!["#random <alice> hi @claybot"]),
            _ => panic!("expected Lines"),
        }
        send_json(&mut g, json!({"op": 0, "t": "MESSAGE_CREATE", "s": 4, "d": {
            "type": 0, "guild_id": GUILD, "channel_id": GENERAL, "content": "mine",
            "author": {"id": BOT, "username": "claybot"}
        }})).await;
        assert!(matches!(next_chat(&mut rx).await, ChatEvent::SelfLines(_)));

        // A message in a guild this world isn't attached to is ignored.
        send_json(&mut g, json!({"op": 0, "t": "MESSAGE_CREATE", "s": 5, "d": {
            "type": 0, "guild_id": "800000000000000099", "channel_id": "1", "content": "elsewhere",
            "author": {"id": "600000000000000001", "username": "alice"}
        }})).await;

        // Typed text: a 429 is retried, @everyone can't ping, long text is split.
        let long = format!("{} tail", "x".repeat(2100));
        cmd_tx.send(WriteCommand::Text(long)).await.unwrap();
        for _ in 0..200 {
            if posts.load(Ordering::SeqCst) >= 3 { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let sent: Vec<_> = rest.requests().into_iter().filter(|r| r.method == "POST").collect();
        assert_eq!(sent.len(), 3, "429 retry + two chunks: {:?}", sent);
        for r in &sent {
            let b: Value = serde_json::from_str(&r.body).unwrap();
            assert_eq!(b["allowed_mentions"]["parse"], json!(["users"]));
            assert!(b["content"].as_str().unwrap().chars().count() <= MESSAGE_LIMIT);
            assert_eq!(r.header("authorization"), Some("Bot tok"));
            assert!(r.header("user-agent").unwrap_or("").starts_with("DiscordBot ("));
        }

        // Server asks to reconnect -> RESUME with the session id and last seq.
        send_json(&mut g, json!({"op": 7, "d": null})).await;
        let mut g2 = ws.accept().await;
        send_json(&mut g2, json!({"op": 10, "d": {"heartbeat_interval": 45000}})).await;
        let resume = next_json(&mut g2).await;
        assert_eq!(resume["op"], 6);
        assert_eq!(resume["d"]["session_id"], "S1");
        assert_eq!(resume["d"]["seq"], 5);
        send_json(&mut g2, json!({"op": 0, "t": "RESUMED", "s": 6, "d": {}})).await;
        send_json(&mut g2, json!({"op": 0, "t": "MESSAGE_CREATE", "s": 7, "d": {
            "type": 0, "guild_id": GUILD, "channel_id": GENERAL, "content": "after resume",
            "author": {"id": "600000000000000001", "username": "alice"}
        }})).await;
        // (the "interrupted"/"resumed" status lines may come first)
        loop {
            match next_chat(&mut rx).await {
                ChatEvent::Lines(l) => {
                    assert_eq!(l, vec!["#general <alice> after resume"]);
                    break;
                }
                ChatEvent::Status(_) => continue,
                _ => panic!("unexpected event"),
            }
        }

        // Dropping the handle ends the session with a normal close.
        drop(handle);
        use futures::StreamExt;
        let closed = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(m) = g2.next().await {
                if let Ok(Message::Close(f)) = m {
                    return f.map(|f| u16::from(f.code));
                }
            }
            None
        })
        .await
        .expect("no close frame after the handle was dropped");
        assert_eq!(closed, Some(1000));
    }

    #[tokio::test]
    async fn unacked_heartbeat_means_zombie_and_resumes() {
        let ws = FakeWs::start().await;
        let rest = FakeRest::start(Arc::new(|_, _, _| (200, json!({"url": "", "session_start_limit": {"remaining": 900}}).to_string(), vec![]))).await;
        let (tx, mut rx) = mpsc::channel(64);
        let _handle = crate::chat::start(cfg(&rest.base, Some(ws.url.clone()), ""), tx);
        let mut g = ws.accept().await;
        send_json(&mut g, json!({"op": 10, "d": {"heartbeat_interval": 500}})).await;
        let identify = next_json(&mut g).await;
        assert_eq!(identify["op"], 2);
        send_json(&mut g, json!({"op": 0, "t": "READY", "s": 1, "d": {
            "session_id": "Z1", "resume_gateway_url": ws.url, "user": {"id": BOT, "username": "claybot"},
            "guilds": [{"id": GUILD}]}})).await;
        send_json(&mut g, guild_create()).await;
        assert!(matches!(next_chat(&mut rx).await, ChatEvent::Ready { .. }));
        // Heartbeats arrive and are never ACKed; the client must give up on this socket.
        use futures::StreamExt;
        let mut beats = 0;
        while let Ok(Some(Ok(m))) = tokio::time::timeout(Duration::from_secs(10), g.next()).await {
            match m {
                Message::Text(t) => {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    assert_eq!(v["op"], 1, "only heartbeats expected, got {v}");
                    beats += 1;
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        assert!(beats >= 1, "at least one heartbeat before the zombie close");
        // ...and RESUME on a new socket.
        let mut g2 = ws.accept().await;
        send_json(&mut g2, json!({"op": 10, "d": {"heartbeat_interval": 45000}})).await;
        let resume = next_json(&mut g2).await;
        assert_eq!(resume["op"], 6);
        assert_eq!(resume["d"]["session_id"], "Z1");
    }

    #[tokio::test]
    async fn disallowed_intent_is_fatal_with_instructions() {
        let ws = FakeWs::start().await;
        let (tx, mut rx) = mpsc::channel(64);
        let rest = FakeRest::start(Arc::new(|_, _, _| (200, json!({"url": "", "session_start_limit": {"remaining": 900}}).to_string(), vec![]))).await;
        let _handle = crate::chat::start(cfg(&rest.base, Some(ws.url.clone()), ""), tx);
        let mut g = ws.accept().await;
        send_json(&mut g, json!({"op": 10, "d": {"heartbeat_interval": 45000}})).await;
        let _identify = next_json(&mut g).await;
        use futures::SinkExt;
        g.send(Message::Close(Some(CloseFrame { code: CloseCode::Library(4014), reason: "Disallowed intent(s).".into() }))).await.unwrap();
        match next_chat(&mut rx).await {
            ChatEvent::Closed { reason, fatal } => {
                assert!(fatal);
                assert!(reason.contains("MESSAGE CONTENT INTENT"), "{}", reason);
            }
            _ => panic!("expected Closed"),
        }
    }

    #[tokio::test]
    async fn bad_token_is_fatal_before_any_gateway() {
        let (tx, mut rx) = mpsc::channel(64);
        let rest = FakeRest::start(Arc::new(|_, _, _| (401, json!({"message": "401: Unauthorized", "code": 0}).to_string(), vec![]))).await;
        let _handle = crate::chat::start(cfg(&rest.base, None, ""), tx);
        match next_chat(&mut rx).await {
            ChatEvent::Closed { reason, fatal } => {
                assert!(fatal);
                assert!(reason.contains("Reset Token"), "{}", reason);
            }
            _ => panic!("expected Closed"),
        }
    }

    #[tokio::test]
    async fn lookup_lists_servers_channels_and_warns_about_intent() {
        let rest = FakeRest::start(Arc::new(|_, path, _| match path {
            "/users/@me" => (200, json!({"id": BOT, "username": "claybot"}).to_string(), vec![]),
            "/applications/@me" => (200, json!({"id": "555555555555555555", "flags": 0}).to_string(), vec![]),
            "/users/@me/guilds" => (200, json!([{"id": GUILD, "name": "Test Server"}]).to_string(), vec![]),
            p if p == format!("/guilds/{}/channels", GUILD) => (200, json!([
                {"id": "1", "name": "Info", "type": 4, "position": 0},
                {"id": GENERAL, "name": "general", "type": 0, "position": 0, "parent_id": "1"},
                {"id": RANDOM, "name": "random", "type": 0, "position": 1}
            ]).to_string(), vec![]),
            _ => (404, "{}".to_string(), vec![]),
        }))
        .await;
        let r = lookup(&rest.base, "tok", "").await;
        assert!(r.ok, "{:?}", r);
        assert_eq!(r.bot_name, "claybot");
        assert_eq!(r.servers.len(), 1);
        assert_eq!(r.selected_server, format!("Test Server ({})", GUILD));
        let labels: Vec<_> = r.channels.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["#random", "Info / #general"]);
        assert_eq!(r.channels[1].value, format!("#general ({})", GENERAL));
        assert!(r.warnings.iter().any(|w| w.contains("MESSAGE CONTENT INTENT")));
        assert!(r.invite_url.contains("client_id=555555555555555555") && r.invite_url.contains("permissions=68608"));
    }

    #[test]
    fn hostile_text_is_sanitized() {
        let mut d = sample();
        let m = json!({"type": 0, "channel_id": "100000000000000002", "guild_id": "1",
            "author": {"id": "200000000000000006", "username": "x\u{1b}[2J", "global_name": "Evil\u{9b}Name"},
            "content": "\u{1b}]8;;http://x\u{7}hi\r\u{85}"});
        let (_, lines) = render_message(&mut d, &m, false).unwrap();
        for l in &lines {
            assert!(!l.chars().any(|c| (c as u32) < 0x20 || ('\u{80}'..='\u{9f}').contains(&c)), "{:?}", l);
        }
        assert_eq!(lines[0], "#general <EvilName> ]8;;http://xhi");
    }
}
