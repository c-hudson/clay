//! Slack: Socket Mode session, Web API sender, and the editor's Fetch lookup.
//!
//! Slack needs **two** tokens: the app-level token (`xapp-...`, scope
//! `connections:write`) opens the Socket Mode WebSocket via `apps.connections.open`;
//! the bot token (`xoxb-...`) calls the Web API (`auth.test`, `conversations.*`,
//! `users.info`, `chat.postMessage`). Every Socket Mode envelope must be acked by
//! echoing its `envelope_id`, or Slack redelivers it. A `disconnect` envelope means
//! "open a fresh URL and reconnect" (Slack rotates connections every few hours).

use std::collections::HashSet;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

use super::directory::{ChannelInfo, ChannelKind, ChatDirectory, ChatTarget, UserInfo};
use super::render::{self, sanitize_inline};
use super::spec;
use super::{
    backoff, send_rate_limited, stopped, ws_connect, ChatConfig, ChatEvent, ChatKind, ChatOp, EventSink,
    ReadySummary, MAX_CONSECUTIVE_FAILURES,
};
use crate::websocket::{ChatDirectoryMsg, ChatPickItem};
use crate::WriteCommand;

pub const DEFAULT_API: &str = "https://slack.com/api";

/// Slack accepts up to 40k characters but recommends staying under 4000 per message.
pub const MESSAGE_LIMIT: usize = 4000;

const TOKENS_HELP: &str = "Slack worlds need two tokens from your Slack app (https://api.slack.com/apps): \
the Bot Token (xoxb-..., under OAuth & Permissions) and the App Token (xapp-..., under Basic Information -> \
App-Level Tokens, with the connections:write scope; Socket Mode must be enabled).";

/// Scopes the bot token needs (for error messages and docs).
pub const BOT_SCOPES: &str = "channels:read, channels:history, groups:read, groups:history, im:read, im:history, \
im:write, mpim:read, mpim:history, users:read, chat:write";

#[derive(Clone)]
pub(crate) struct SlackApi {
    http: reqwest::Client,
    base: String,
    auth: String,
}

impl SlackApi {
    pub(crate) fn new(base: &str, token: &str) -> SlackApi {
        SlackApi {
            http: super::http_client(),
            base: base.trim_end_matches('/').to_string(),
            auth: format!("Bearer {}", token.trim()),
        }
    }

    /// Call a Web API method. `Ok(json)` only when Slack says `"ok": true`; otherwise
    /// `Err(error_code)` (Slack's `error` string, or a network description).
    pub(crate) async fn call(&self, method: &str, args: Value) -> Result<Value, String> {
        let url = format!("{}/{}", self.base, method);
        let resp = send_rate_limited(|| {
            self.http
                .post(&url)
                .header("Authorization", &self.auth)
                .header("Content-Type", "application/json; charset=utf-8")
                .json(&args)
        })
        .await?;
        if resp.json["ok"].as_bool() == Some(true) {
            Ok(resp.json)
        } else if let Some(e) = resp.json["error"].as_str() {
            Err(e.to_string())
        } else {
            Err(format!("HTTP {}", resp.status))
        }
    }

    /// Read methods (conversations.list, users.info, ...) take form-encoded arguments;
    /// several of them ignore a JSON body.
    pub(crate) async fn get(&self, method: &str, args: Value) -> Result<Value, String> {
        let url = format!("{}/{}", self.base, method);
        let pairs: Vec<(String, String)> = args
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().map(|s| s.to_string()).unwrap_or_else(|| v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let resp = send_rate_limited(|| self.http.post(&url).header("Authorization", &self.auth).form(&pairs)).await?;
        if resp.json["ok"].as_bool() == Some(true) {
            Ok(resp.json)
        } else if let Some(e) = resp.json["error"].as_str() {
            Err(e.to_string())
        } else {
            Err(format!("HTTP {}", resp.status))
        }
    }
}

/// A readable sentence for a Slack error code.
pub fn describe_error(code: &str, context: &str) -> String {
    match code {
        "invalid_auth" | "not_authed" | "token_revoked" | "account_inactive" => format!(
            "Slack rejected the token ({}). Copy it again from your Slack app's settings. {}",
            code, TOKENS_HELP
        ),
        "missing_scope" => format!(
            "The Slack bot token is missing a permission scope ({}). Add these Bot Token Scopes under OAuth & Permissions and reinstall the app: {}.",
            context, BOT_SCOPES
        ),
        "not_in_channel" => format!("The bot isn't in {} - type /invite @yourbot in that channel in Slack.", context),
        "channel_not_found" => format!("{} doesn't exist or the bot can't see it.", context),
        "is_archived" => format!("{} is archived.", context),
        "msg_too_long" => "That message is too long for Slack.".to_string(),
        "restricted_action" | "not_allowed_token_type" => format!("Slack doesn't allow that ({}): {}.", code, context),
        _ => format!("Slack error '{}' ({}).", code, context),
    }
}

fn is_auth_error(code: &str) -> bool {
    matches!(code, "invalid_auth" | "not_authed" | "token_revoked" | "account_inactive" | "not_allowed_token_type")
}

// ============================================================================
// Session
// ============================================================================

pub async fn run(cfg: ChatConfig, sink: EventSink, dir: Arc<RwLock<ChatDirectory>>, mut shutdown: watch::Receiver<bool>) {
    if cfg.token.is_empty() || cfg.app_token.is_empty() {
        close(&sink, TOKENS_HELP.to_string(), true).await;
        return;
    }
    if !cfg.app_token.starts_with("xapp-") {
        close(&sink, format!("The App Token should start with xapp-. {}", TOKENS_HELP), true).await;
        return;
    }
    let web = SlackApi::new(&cfg.endpoints.api_base, &cfg.token);
    let app = SlackApi::new(&cfg.endpoints.api_base, &cfg.app_token);

    // Identify ourselves and load the channel list (retrying network failures).
    let mut failures = 0u32;
    let my_bot_id;
    loop {
        match web.call("auth.test", json!({})).await {
            Ok(v) => {
                let mut d = dir.write().unwrap();
                d.bot_id = v["user_id"].as_str().unwrap_or("").to_string();
                d.bot_name = sanitize_inline(v["user"].as_str().unwrap_or(""));
                d.server = Some((
                    v["team_id"].as_str().unwrap_or("").to_string(),
                    sanitize_inline(v["team"].as_str().unwrap_or("")),
                ));
                let bot = UserInfo { id: d.bot_id.clone(), username: d.bot_name.clone(), display: d.bot_name.clone() };
                d.upsert_user(bot);
                my_bot_id = v["bot_id"].as_str().unwrap_or("").to_string();
                break;
            }
            Err(e) if is_auth_error(&e) => {
                close(&sink, describe_error(&e, "bot token"), true).await;
                return;
            }
            Err(e) => {
                failures += 1;
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    close(&sink, format!("Couldn't reach Slack: {}", e), false).await;
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep(backoff(failures - 1)) => {}
                    _ = stopped(&mut shutdown) => return,
                }
            }
        }
    }
    if let Err(e) = load_channels(&web, &dir).await {
        sink.status(describe_error(&e, "listing channels")).await;
    }

    let (cmd_tx, cmd_rx) = mpsc::channel::<WriteCommand>(256);
    tokio::spawn(writer(web.clone(), dir.clone(), cmd_rx, sink.clone(), shutdown.clone()));

    let mut sess = Session { cfg, web, sink, dir, cmd_tx, my_bot_id, announced: false };
    failures = 0;
    let mut lost_reported = false;
    loop {
        let url = match app.call("apps.connections.open", json!({})).await {
            Ok(v) => v["url"].as_str().unwrap_or("").to_string(),
            Err(e) if is_auth_error(&e) => {
                close(&sess.sink, describe_error(&e, "app token"), true).await;
                return;
            }
            Err(e) => {
                failures += 1;
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    close(&sess.sink, format!("Couldn't open a Slack connection: {}", e), false).await;
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep(backoff(failures - 1)) => continue,
                    _ = stopped(&mut shutdown) => return,
                }
            }
        };
        let (end, got_hello) = sess.connect_once(&url, &mut shutdown).await;
        if got_hello {
            failures = 0;
            lost_reported = false;
        }
        match end {
            End::Shutdown => return,
            End::Fatal(msg) => {
                close(&sess.sink, msg, true).await;
                return;
            }
            End::Reconnect(reason, expected) => {
                if expected {
                    // Slack's routine connection refresh: reconnect at once, quietly.
                    continue;
                }
                failures += if got_hello { 0 } else { 1 };
                if failures >= MAX_CONSECUTIVE_FAILURES {
                    close(&sess.sink, format!("Lost the connection to Slack: {}", reason), false).await;
                    return;
                }
                if sess.announced && !lost_reported {
                    sess.sink.status(format!("Slack connection interrupted ({}); reconnecting...", reason)).await;
                    lost_reported = true;
                }
                tokio::select! {
                    _ = tokio::time::sleep(backoff(failures)) => {}
                    _ = stopped(&mut shutdown) => return,
                }
            }
        }
    }
}

async fn close(sink: &EventSink, reason: String, fatal: bool) {
    sink.send(ChatEvent::Closed { reason, fatal }).await;
}

enum End {
    /// Reconnect; `true` = a routine refresh Slack asked for (no message, no backoff).
    Reconnect(String, bool),
    Fatal(String),
    Shutdown,
}

struct Session {
    cfg: ChatConfig,
    web: SlackApi,
    sink: EventSink,
    dir: Arc<RwLock<ChatDirectory>>,
    cmd_tx: mpsc::Sender<WriteCommand>,
    my_bot_id: String,
    announced: bool,
}

impl Session {
    async fn connect_once(&mut self, url: &str, shutdown: &mut watch::Receiver<bool>) -> (End, bool) {
        let mut ws = match ws_connect(url).await {
            Ok(ws) => ws,
            Err(e) => return (End::Reconnect(e, false), false),
        };
        let mut got_hello = false;
        loop {
            tokio::select! {
                _ = stopped(shutdown) => {
                    let _ = ws.send(Message::Close(None)).await;
                    return (End::Shutdown, got_hello);
                }
                msg = ws.next() => {
                    let text = match msg {
                        None => return (End::Reconnect("the connection dropped".into(), false), got_hello),
                        Some(Err(e)) => return (End::Reconnect(format!("connection error: {}", e), false), got_hello),
                        Some(Ok(Message::Close(_))) => return (End::Reconnect("the connection closed".into(), false), got_hello),
                        Some(Ok(Message::Text(t))) => t,
                        Some(Ok(_)) => continue,
                    };
                    let v: Value = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => continue };
                    // Ack first, always - an un-acked envelope is redelivered.
                    if let Some(env) = v["envelope_id"].as_str() {
                        let _ = ws.send(Message::Text(json!({"envelope_id": env}).to_string())).await;
                    }
                    match v["type"].as_str().unwrap_or("") {
                        "hello" => {
                            got_hello = true;
                            if !self.announced {
                                self.announce().await;
                            } else {
                                // Only a non-routine reconnect was reported as lost.
                            }
                        }
                        "disconnect" => {
                            return match v["reason"].as_str().unwrap_or("") {
                                "link_disabled" => (End::Fatal(format!("Socket Mode was turned off for this Slack app. {}", TOKENS_HELP)), got_hello),
                                _ => (End::Reconnect("Slack refreshed the connection".into(), true), got_hello),
                            };
                        }
                        "events_api" => self.on_event(&v["payload"]["event"]).await,
                        _ => {}
                    }
                }
            }
        }
    }

    async fn announce(&mut self) {
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
    }

    async fn on_event(&mut self, ev: &Value) {
        match ev["type"].as_str().unwrap_or("") {
            "message" => {}
            "channel_created" | "channel_rename" | "group_rename" => {
                let c = &ev["channel"];
                if let (Some(id), Some(name)) = (c["id"].as_str(), c["name"].as_str()) {
                    let mut dir = self.dir.write().unwrap();
                    let kind = if id.starts_with('G') { ChannelKind::Private } else { ChannelKind::Text };
                    let entry = dir.channels.entry(id.to_string()).or_insert(ChannelInfo {
                        id: id.to_string(),
                        name: String::new(),
                        kind,
                        parent_id: None,
                        position: 0,
                        can_send: None,
                    });
                    entry.name = sanitize_inline(name);
                }
                return;
            }
            "channel_deleted" | "group_deleted" => {
                if let Some(id) = ev["channel"].as_str() {
                    self.dir.write().unwrap().channels.remove(id);
                }
                return;
            }
            "member_joined_channel" => {
                if ev["user"].as_str() == Some(self.dir.read().unwrap().bot_id.as_str()) {
                    if let Some(id) = ev["channel"].as_str() {
                        if let Some(c) = self.dir.write().unwrap().channels.get_mut(id) {
                            c.can_send = Some(true);
                        }
                    }
                }
                return;
            }
            _ => return,
        }
        // Resolve anyone we don't know yet before rendering (bounded).
        let mut ids: Vec<String> = Vec::new();
        let msg = if ev["subtype"].as_str() == Some("message_changed") { &ev["message"] } else { ev };
        if let Some(u) = msg["user"].as_str() {
            ids.push(u.to_string());
        }
        for c in user_mention_re().captures_iter(msg["text"].as_str().unwrap_or("")) {
            ids.push(c[1].to_string());
        }
        ensure_users(&self.web, &self.dir, &ids).await;
        let channel_id = ev["channel"].as_str().unwrap_or("").to_string();
        if ev["channel_type"].as_str() == Some("channel") || ev["channel_type"].as_str() == Some("group") {
            let known = self.dir.read().unwrap().channels.contains_key(&channel_id);
            if !known {
                if let Ok(v) = self.web.get("conversations.info", json!({"channel": channel_id})).await {
                    if let Some(c) = channel_from(&v["channel"]) {
                        self.dir.write().unwrap().channels.insert(c.id.clone(), c);
                    }
                }
            }
        }
        let rendered = {
            let mut dir = self.dir.write().unwrap();
            let is_dm = matches!(ev["channel_type"].as_str(), Some("im") | Some("mpim"));
            if !is_dm && !dir.passes_filter(&channel_id, &self.cfg.filter) {
                return;
            }
            render_event(&mut dir, ev, &self.my_bot_id)
        };
        let Some((own, lines)) = rendered else { return };
        if !lines.is_empty() {
            self.sink.send(if own { ChatEvent::SelfLines(lines) } else { ChatEvent::Lines(lines) }).await;
        }
    }
}

fn channel_from(c: &Value) -> Option<ChannelInfo> {
    let id = c["id"].as_str()?.to_string();
    if c["is_im"].as_bool() == Some(true) || c["is_mpim"].as_bool() == Some(true) {
        return None;
    }
    Some(ChannelInfo {
        name: sanitize_inline(c["name"].as_str().unwrap_or(&id)),
        kind: if c["is_private"].as_bool() == Some(true) { ChannelKind::Private } else { ChannelKind::Text },
        parent_id: None,
        position: 0,
        can_send: c["is_member"].as_bool(),
        id,
    })
}

/// Load every channel the bot can list (paginated, bounded).
async fn load_channels(web: &SlackApi, dir: &Arc<RwLock<ChatDirectory>>) -> Result<(), String> {
    let mut cursor = String::new();
    for _ in 0..50 {
        let mut args = json!({"types": "public_channel,private_channel", "exclude_archived": "true", "limit": "200"});
        if !cursor.is_empty() {
            args["cursor"] = Value::String(cursor.clone());
        }
        let v = web.get("conversations.list", args).await?;
        {
            let mut d = dir.write().unwrap();
            for c in v["channels"].as_array().cloned().unwrap_or_default() {
                if let Some(info) = channel_from(&c) {
                    d.channels.insert(info.id.clone(), info);
                }
            }
        }
        cursor = v["response_metadata"]["next_cursor"].as_str().unwrap_or("").to_string();
        if cursor.is_empty() {
            break;
        }
    }
    Ok(())
}

async fn ensure_users(web: &SlackApi, dir: &Arc<RwLock<ChatDirectory>>, ids: &[String]) {
    let unknown: Vec<String> = {
        let d = dir.read().unwrap();
        let mut seen = HashSet::new();
        ids.iter().filter(|id| !d.users.contains_key(*id) && seen.insert((*id).clone())).take(5).cloned().collect()
    };
    for id in unknown {
        if let Ok(Ok(v)) = tokio::time::timeout(Duration::from_secs(4), web.get("users.info", json!({"user": id}))).await {
            let u = &v["user"];
            let username = u["name"].as_str().unwrap_or("").to_string();
            let display = u["profile"]["display_name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| u["profile"]["real_name"].as_str())
                .or_else(|| u["real_name"].as_str())
                .unwrap_or(&username)
                .to_string();
            dir.write().unwrap().upsert_user(UserInfo { id: id.clone(), username: sanitize_inline(&username), display: sanitize_inline(&display) });
        }
    }
}

fn user_mention_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<@([UW][A-Z0-9]+)(?:\|[^>]*)?>").unwrap())
}

fn markup_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<([^<>]+)>").unwrap())
}

/// Convert Slack's message markup to plain text: `<@U123>` -> `@Alice`,
/// `<#C123|general>` -> `#general`, `<!here>` -> `@here`, `<!subteam^S1|@devs>` ->
/// `@devs`, `<https://x|label>` -> `label (https://x)`, then the three HTML entities
/// Slack escapes (`&lt; &gt; &amp;`, in that order so `&amp;lt;` stays literal).
pub fn resolve_markup(dir: &ChatDirectory, text: &str) -> String {
    let replaced = markup_re().replace_all(text, |c: &regex::Captures| {
        let inner = &c[1];
        let (target, label) = match inner.find('|') {
            Some(i) => (&inner[..i], Some(&inner[i + 1..])),
            None => (inner, None),
        };
        if let Some(id) = target.strip_prefix('@') {
            return format!("@{}", dir.user_display(id));
        }
        if let Some(id) = target.strip_prefix('#') {
            return match label {
                Some(l) if !l.is_empty() => format!("#{}", l),
                _ => dir.channel_label(id),
            };
        }
        if let Some(special) = target.strip_prefix('!') {
            if let Some(l) = label {
                return l.to_string();
            }
            return match special.split('^').next().unwrap_or("") {
                "here" => "@here".to_string(),
                "channel" => "@channel".to_string(),
                "everyone" => "@everyone".to_string(),
                other => format!("@{}", other),
            };
        }
        match label {
            Some(l) if !l.is_empty() && l != target && target.strip_prefix("mailto:") != Some(l) => format!("{} ({})", l, target),
            Some(l) if !l.is_empty() => l.to_string(),
            _ => target.to_string(),
        }
    });
    replaced.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Escape outgoing text the way Slack expects (only these three characters).
pub fn escape_outgoing(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Render a `message` event. Returns (is_own, lines) or None for subtypes not shown.
pub fn render_event(dir: &mut ChatDirectory, ev: &Value, my_bot_id: &str) -> Option<(bool, Vec<String>)> {
    let subtype = ev["subtype"].as_str().unwrap_or("");
    let (msg, edited) = match subtype {
        "message_changed" => {
            let m = &ev["message"];
            // Link unfurls also arrive as message_changed with identical text.
            if m["text"] == ev["previous_message"]["text"] {
                return None;
            }
            (m, true)
        }
        "" | "me_message" | "bot_message" | "file_share" | "thread_broadcast" | "channel_join" | "channel_leave"
        | "channel_topic" | "channel_purpose" | "pinned_item" => (ev, false),
        _ => return None,
    };
    let channel_id = ev["channel"].as_str().unwrap_or("").to_string();
    let user_id = msg["user"].as_str().unwrap_or("").to_string();
    let own = (!user_id.is_empty() && user_id == dir.bot_id)
        || (!my_bot_id.is_empty() && msg["bot_id"].as_str() == Some(my_bot_id));
    let is_dm = matches!(ev["channel_type"].as_str(), Some("im"));
    if is_dm && !own && !user_id.is_empty() {
        dir.dm_channels.insert(channel_id.clone(), user_id.clone());
    }
    let tag = if is_dm {
        match dir.dm_channels.get(&channel_id) {
            Some(uid) => format!("@{}", dir.user_handle(uid)),
            None => "@dm".to_string(),
        }
    } else {
        dir.channel_label(&channel_id)
    };
    let display = if user_id.is_empty() {
        sanitize_inline(msg["username"].as_str().or_else(|| msg["bot_profile"]["name"].as_str()).unwrap_or("bot"))
    } else {
        dir.user_display(&user_id)
    };
    if !own {
        let uid = if is_dm { dir.dm_channels.get(&channel_id).cloned() } else { None };
        dir.last_inbound = Some(ChatTarget { channel_id: Some(channel_id.clone()), user_id: uid, label: tag.clone() });
    }
    let body = resolve_markup(dir, msg["text"].as_str().unwrap_or(""));
    match subtype {
        "me_message" => return Some((own, vec![render::action_line(&tag, &format!("{} {}", display, body))])),
        "channel_join" | "channel_leave" | "channel_topic" | "channel_purpose" | "pinned_item" => {
            return Some((own, vec![render::action_line(&tag, &body)]));
        }
        _ => {}
    }
    let mut prefix = String::new();
    if edited {
        prefix.push_str("(edited)");
    }
    let ts = msg["ts"].as_str().unwrap_or("");
    if let Some(thread) = msg["thread_ts"].as_str() {
        if thread != ts {
            if !prefix.is_empty() {
                prefix.push(' ');
            }
            prefix.push_str("(in thread)");
        }
    }
    let mut extras = Vec::new();
    if let Some(files) = msg["files"].as_array() {
        for f in files {
            extras.push(format!(
                "[file: {} {}]",
                f["name"].as_str().unwrap_or("file"),
                f["permalink"].as_str().or_else(|| f["url_private"].as_str()).unwrap_or("")
            ));
        }
    }
    if body.trim().is_empty() && extras.is_empty() {
        return None;
    }
    Some((own, render::message_lines(&tag, &display, &prefix, &body, &extras)))
}

// ============================================================================
// Writer
// ============================================================================

async fn writer(
    web: SlackApi,
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
                Some(t) => send_text(&web, &dir, t, &text, &sink).await,
                None => sink.status("No send target - pick one with /chat to <channel> (see /chat channels).").await,
            },
            WriteCommand::Chat(ChatOp::SetTarget(t)) => {
                target = Some(t.clone());
                sink.send(ChatEvent::TargetChanged(t)).await;
            }
            WriteCommand::Chat(ChatOp::SendTo(mut t, text)) => send_text(&web, &dir, &mut t, &text, &sink).await,
            WriteCommand::Shutdown => return,
            WriteCommand::Raw(_) | WriteCommand::SetEncoding(_) => {}
        }
    }
}

async fn send_text(web: &SlackApi, dir: &Arc<RwLock<ChatDirectory>>, t: &mut ChatTarget, text: &str, sink: &EventSink) {
    if text.trim().is_empty() {
        return;
    }
    let channel = match &t.channel_id {
        Some(c) => c.clone(),
        None => {
            let Some(uid) = t.user_id.clone() else { return };
            match web.call("conversations.open", json!({"users": uid})).await {
                Ok(v) => {
                    let cid = v["channel"]["id"].as_str().unwrap_or("").to_string();
                    dir.write().unwrap().dm_channels.insert(cid.clone(), uid);
                    t.channel_id = Some(cid.clone());
                    cid
                }
                Err(e) => {
                    sink.status(describe_error(&e, &format!("opening a DM with {}", t.label))).await;
                    return;
                }
            }
        }
    };
    for chunk in render::split_message(text, MESSAGE_LIMIT) {
        let args = json!({"channel": channel, "text": escape_outgoing(&chunk)});
        if let Err(e) = web.call("chat.postMessage", args).await {
            sink.status(describe_error(&e, &t.label)).await;
            return;
        }
    }
}

// ============================================================================
// Fetch (world editor picker)
// ============================================================================

pub async fn lookup(api_base: &str, bot_token: &str, app_token: &str) -> ChatDirectoryMsg {
    let mut out = ChatDirectoryMsg::default();
    if bot_token.trim().is_empty() {
        out.error = format!("Enter the Bot Token first. {}", TOKENS_HELP);
        return out;
    }
    let web = SlackApi::new(api_base, bot_token);
    let auth = match web.call("auth.test", json!({})).await {
        Ok(v) => v,
        Err(e) => {
            out.error = describe_error(&e, "bot token");
            return out;
        }
    };
    out.ok = true;
    out.bot_name = sanitize_inline(auth["user"].as_str().unwrap_or(""));
    let team_id = auth["team_id"].as_str().unwrap_or("").to_string();
    let team = sanitize_inline(auth["team"].as_str().unwrap_or(""));
    out.servers.push(ChatPickItem { id: team_id, name: team.clone(), label: team.clone(), value: team.clone(), ..Default::default() });
    out.selected_server = team;
    let app_token = app_token.trim();
    if app_token.is_empty() {
        out.warnings.push(format!("Add the App Token too, or the world can't connect. {}", TOKENS_HELP));
    } else if !app_token.starts_with("xapp-") {
        out.warnings.push(format!("The App Token should start with xapp-. {}", TOKENS_HELP));
    } else if let Err(e) = SlackApi::new(api_base, app_token).call("apps.connections.open", json!({})).await {
        out.warnings.push(describe_error(&e, "app token"));
    }
    let dir = Arc::new(RwLock::new(ChatDirectory::new(ChatKind::Slack)));
    if let Err(e) = load_channels(&web, &dir).await {
        out.warnings.push(describe_error(&e, "listing channels"));
    }
    let d = dir.read().unwrap();
    let mut chans: Vec<&ChannelInfo> = d.channels.values().collect();
    chans.sort_by(|a, b| b.can_send.unwrap_or(false).cmp(&a.can_send.unwrap_or(false)).then(a.name.cmp(&b.name)));
    for c in chans {
        let member = c.can_send.unwrap_or(false);
        out.channels.push(ChatPickItem {
            id: c.id.clone(),
            name: c.name.clone(),
            value: spec::picker_value(&format!("#{}", c.name), &c.id),
            label: if member { format!("#{}", c.name) } else { format!("#{} (bot not invited)", c.name) },
            kind: if c.kind == ChannelKind::Private { "private" } else { "text" }.to_string(),
            can_send: c.can_send,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> ChatDirectory {
        let mut d = ChatDirectory::new(ChatKind::Slack);
        d.bot_id = "UBOT00001".into();
        d.upsert_user(UserInfo { id: "U0ALICE01".into(), username: "alice".into(), display: "Alice".into() });
        d.channels.insert(
            "C0GENERAL".into(),
            ChannelInfo { id: "C0GENERAL".into(), name: "general".into(), kind: ChannelKind::Text, parent_id: None, position: 0, can_send: Some(true) },
        );
        d
    }

    #[test]
    fn markup() {
        let d = dir();
        assert_eq!(
            resolve_markup(&d, "hi <@U0ALICE01>, see <#C0GENERAL> and <#C0OTHER01|random> <!here> <!subteam^S1|@devs>"),
            "hi @Alice, see #general and #random @here @devs"
        );
        assert_eq!(resolve_markup(&d, "<https://x.io|site> <https://y.io> <mailto:a@b.c|a@b.c>"), "site (https://x.io) https://y.io a@b.c");
        assert_eq!(resolve_markup(&d, "a &lt;b&gt; &amp;lt; c"), "a <b> &lt; c");
        assert_eq!(escape_outgoing("a<b>&c"), "a&lt;b&gt;&amp;c");
    }

    #[test]
    fn render_messages() {
        let mut d = dir();
        let ev = json!({"type": "message", "channel": "C0GENERAL", "channel_type": "channel", "user": "U0ALICE01", "text": "hello\nthere", "ts": "1.0"});
        let (own, lines) = render_event(&mut d, &ev, "B0MINE").unwrap();
        assert!(!own);
        assert_eq!(lines, vec!["#general <Alice> hello", "  there"]);
        // DM learns the channel.
        let ev = json!({"type": "message", "channel": "D0DM00001", "channel_type": "im", "user": "U0ALICE01", "text": "psst", "ts": "2.0"});
        assert_eq!(render_event(&mut d, &ev, "B0MINE").unwrap().1, vec!["@alice <Alice> psst"]);
        assert_eq!(d.last_inbound.as_ref().unwrap().user_id.as_deref(), Some("U0ALICE01"));
        // Own message by bot_id.
        let ev = json!({"type": "message", "subtype": "bot_message", "channel": "C0GENERAL", "channel_type": "channel", "bot_id": "B0MINE", "text": "me", "ts": "3.0"});
        assert!(render_event(&mut d, &ev, "B0MINE").unwrap().0);
        // Edit with same text (unfurl) is ignored; a real edit shows.
        let ev = json!({"type": "message", "subtype": "message_changed", "channel": "C0GENERAL", "channel_type": "channel",
            "message": {"user": "U0ALICE01", "text": "same", "ts": "1.0"}, "previous_message": {"text": "same"}});
        assert!(render_event(&mut d, &ev, "").is_none());
        let ev = json!({"type": "message", "subtype": "message_changed", "channel": "C0GENERAL", "channel_type": "channel",
            "message": {"user": "U0ALICE01", "text": "new", "ts": "1.0"}, "previous_message": {"text": "old"}});
        assert_eq!(render_event(&mut d, &ev, "").unwrap().1, vec!["#general <Alice> (edited) new"]);
        // Thread reply + file.
        let ev = json!({"type": "message", "subtype": "file_share", "channel": "C0GENERAL", "channel_type": "channel", "user": "U0ALICE01",
            "text": "", "ts": "5.0", "thread_ts": "1.0", "files": [{"name": "a.txt", "permalink": "https://s/a"}]});
        assert_eq!(render_event(&mut d, &ev, "").unwrap().1, vec!["#general <Alice> (in thread) [file: a.txt https://s/a]"]);
        // Unknown subtypes are skipped.
        let ev = json!({"type": "message", "subtype": "message_deleted", "channel": "C0GENERAL"});
        assert!(render_event(&mut d, &ev, "").is_none());
    }

    // ---- End-to-end against a local fake Slack ----

    use crate::chat::test_support::{next_chat, next_json, send_json, FakeRest, FakeWs};
    use crate::chat::{ChatOwner, Endpoints};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn socket_mode_session_end_to_end() {
        let ws = FakeWs::start().await;
        let ws_url = ws.url.clone();
        let opens = Arc::new(AtomicUsize::new(0));
        let opens2 = opens.clone();
        let rest = FakeRest::start(Arc::new(move |_m, path, body| match path {
            "/auth.test" => (200, json!({"ok": true, "user_id": "UBOT00001", "user": "claybot", "bot_id": "B0MINE", "team_id": "T0TEAM001", "team": "Acme"}).to_string(), vec![]),
            "/conversations.list" => (200, json!({"ok": true, "channels": [
                {"id": "C0GENERAL", "name": "general", "is_member": true},
                {"id": "C0RANDOM1", "name": "random", "is_member": false}
            ], "response_metadata": {"next_cursor": ""}}).to_string(), vec![]),
            "/users.info" => (200, json!({"ok": true, "user": {"name": "alice", "profile": {"display_name": "Alice"}}}).to_string(), vec![]),
            "/apps.connections.open" => {
                opens2.fetch_add(1, Ordering::SeqCst);
                (200, json!({"ok": true, "url": ws_url}).to_string(), vec![])
            }
            "/chat.postMessage" => {
                let b: Value = serde_json::from_str(body).unwrap_or_default();
                if b["channel"] == "C0RANDOM1" {
                    (200, json!({"ok": false, "error": "not_in_channel"}).to_string(), vec![])
                } else {
                    (200, json!({"ok": true}).to_string(), vec![])
                }
            }
            _ => (200, json!({"ok": false, "error": "unknown_method"}).to_string(), vec![]),
        }))
        .await;
        let (tx, mut rx) = mpsc::channel(64);
        let cfg = ChatConfig {
            kind: ChatKind::Slack,
            owner: ChatOwner::World("S".into()),
            conn_id: 1,
            endpoints: Endpoints { api_base: rest.base.clone(), gateway_url: None },
            token: "xoxb-1".into(),
            app_token: "xapp-1".into(),
            server_spec: String::new(),
            target_spec: "#general".into(),
            filter: Vec::new(),
            resume: None,
        };
        let handle = crate::chat::start(cfg, tx);

        let mut sock = ws.accept().await;
        send_json(&mut sock, json!({"type": "hello"})).await;
        let cmd_tx = match next_chat(&mut rx).await {
            ChatEvent::Ready { cmd_tx, summary } => {
                assert_eq!(summary.server_name, "Acme");
                assert_eq!(summary.target.as_ref().map(|t| t.label.as_str()), Some("#general"));
                cmd_tx
            }
            _ => panic!("expected Ready"),
        };
        assert!(matches!(next_chat(&mut rx).await, ChatEvent::TargetChanged(_)));

        // An event envelope is acked, then rendered with the user's name looked up.
        send_json(&mut sock, json!({"envelope_id": "E1", "type": "events_api", "payload": {"event": {
            "type": "message", "channel": "C0GENERAL", "channel_type": "channel", "user": "U0ALICE01",
            "text": "hi &lt;there&gt; <@UBOT00001>", "ts": "1.0"}}})).await;
        let ack = next_json(&mut sock).await;
        assert_eq!(ack["envelope_id"], "E1");
        match next_chat(&mut rx).await {
            ChatEvent::Lines(l) => assert_eq!(l, vec!["#general <Alice> hi <there> @claybot"]),
            _ => panic!("expected Lines"),
        }

        // Sending escapes Slack's three special characters.
        cmd_tx.send(WriteCommand::Text("a < b & c".into())).await.unwrap();
        for _ in 0..100 {
            if rest.requests().iter().any(|r| r.path == "/chat.postMessage") { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let post = rest.requests().into_iter().find(|r| r.path == "/chat.postMessage").unwrap();
        let b: Value = serde_json::from_str(&post.body).unwrap();
        assert_eq!(b["text"], "a &lt; b &amp; c");
        assert_eq!(b["channel"], "C0GENERAL");
        assert_eq!(post.header("authorization"), Some("Bearer xoxb-1"));

        // A channel the bot isn't in gives an actionable message.
        let dir = handle.dir.read().unwrap().clone();
        let t = dir.resolve_target("#random").unwrap();
        cmd_tx.send(WriteCommand::Chat(ChatOp::SendTo(t, "x".into()))).await.unwrap();
        match next_chat(&mut rx).await {
            ChatEvent::Status(m) => assert!(m.contains("/invite"), "{}", m),
            _ => panic!("expected a status line"),
        }

        // Slack's routine refresh: reconnect quietly with a fresh URL.
        send_json(&mut sock, json!({"type": "disconnect", "reason": "refresh_requested"})).await;
        let mut sock2 = ws.accept().await;
        send_json(&mut sock2, json!({"type": "hello"})).await;
        assert!(opens.load(Ordering::SeqCst) >= 2);
        send_json(&mut sock2, json!({"envelope_id": "E2", "type": "events_api", "payload": {"event": {
            "type": "message", "subtype": "bot_message", "channel": "C0GENERAL", "channel_type": "channel",
            "bot_id": "B0MINE", "text": "echo", "ts": "2.0"}}})).await;
        let _ = next_json(&mut sock2).await;
        assert!(matches!(next_chat(&mut rx).await, ChatEvent::SelfLines(_)));
        drop(handle);
    }

    #[tokio::test]
    async fn missing_app_token_is_fatal_and_explains() {
        let (tx, mut rx) = mpsc::channel(8);
        let cfg = ChatConfig {
            kind: ChatKind::Slack,
            owner: ChatOwner::World("S".into()),
            conn_id: 1,
            endpoints: Endpoints { api_base: "http://127.0.0.1:9".into(), gateway_url: None },
            token: "xoxb-1".into(),
            app_token: String::new(),
            server_spec: String::new(),
            target_spec: String::new(),
            filter: Vec::new(),
            resume: None,
        };
        let _h = crate::chat::start(cfg, tx);
        match next_chat(&mut rx).await {
            ChatEvent::Closed { reason, fatal } => {
                assert!(fatal);
                assert!(reason.contains("xapp-"), "{}", reason);
            }
            _ => panic!("expected Closed"),
        }
    }

    #[test]
    fn errors_readable() {
        assert!(describe_error("not_in_channel", "#x").contains("/invite"));
        assert!(describe_error("missing_scope", "listing channels").contains("chat:write"));
        assert!(describe_error("invalid_auth", "bot token").contains("xapp-"));
    }
}
