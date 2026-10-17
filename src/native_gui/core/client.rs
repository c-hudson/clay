//! The native GUI's client: everything the web client (`app.js`) does to stay in step
//! with a Clay server, with no I/O of its own.
//!
//! Connection events, server messages and clock ticks go in; [`Effect`]s come out (send
//! this message, open a connection, close it, store this key, redraw). The transport
//! (`net`) and the GUI carry the effects out, and the headless verifier
//! (`--gui-verify`) drives the very same core, so what it proves is what the GUI
//! does.
//!
//! The rules are app.js's, which is the protocol's reference client
//! (PROTOCOL-ROADMAP.md, CLAUDE.md "Key Design Patterns"):
//!
//! - Output is deduped by exact seq membership ([`SeenRanges`]), and a batch's whole seq
//!   span is recorded even when some of its lines are filtered out.
//! - The client acks its contiguous frontier per world every keepalive (`PongCheck`) and
//!   sends it as `resume` when it logs in again, so the server replays exactly the gap.
//! - A hole is filled with `RequestScrollback { after_seq }`; after two replies that bring
//!   nothing, the client gives the hole up and reports it (`ReportGap`).
//! - A buffer kept across a reconnect is thrown away when the world's `seq_epoch` says
//!   its seqs belong to another numbering.
//! - The ▶ markers a line carries belong to one client (`display_id`), claimed and
//!   released by the server; the client guesses its claim before the server confirms it.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::cache::{CachedLine, CachedWorld};
use super::seq::SeenRanges;
use crate::actions::Action;
use crate::stats::StatEntry;
use crate::tf::status::StatusView;
use crate::websocket::{
    GlobalSettingsMsg, RemoteClientType, TimestampedLine, WorldSettingsMsg, WorldStateMsg, WsMessage, hash_password,
    hash_with_challenge,
};

/// What this client calls itself in the diagnostics it sends (`ReportGap` and friends).
pub const SOURCE: &str = "native";

const KEEPALIVE: Duration = Duration::from_secs(30);
/// No message at all for this long on a logged-in connection (the server answers every
/// keepalive Ping with a Pong) and the connection is dead without having said so.
const LIVENESS: Duration = Duration::from_secs(75);
/// The server must say hello this soon after the socket opens.
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
/// A scrollback request with no reply in this long is given up (app.js's watchdog).
const SCROLLBACK_WATCHDOG: Duration = Duration::from_secs(15);
/// How long a resumed world waits for the server's unprompted replay before asking itself.
const REPLAY_WATCHDOG: Duration = Duration::from_secs(15);
/// A locally guessed ▶ claim older than this is not trusted when the server's arrives.
const CLAIM_TTL: Duration = Duration::from_secs(5);
/// Gap-fill replies in a row that bring nothing before the hole is given up.
const GAPFILL_MAX_NO_PROGRESS: u32 = 2;
const GAPFILL_COUNT: usize = 500;
const BACKFILL_DELAY: Duration = Duration::from_millis(500);
/// A tick arriving this much later than the last means the machine slept: check the
/// connection is still alive before trusting it (app.js's wake check).
const SLEEP_GAP: Duration = Duration::from_secs(10);
const WAKE_TIMEOUT: Duration = Duration::from_secs(3);
/// After a `ServerReloading`, how many quick reconnects to try.
const RELOAD_RETRIES: u32 = 5;
/// Seconds between reconnect attempts, by attempt (the last repeats).
const BACKOFF: [u64; 6] = [2, 2, 5, 10, 20, 30];
/// A line's timestamp jumping back this far marks a corrupted buffer (app.js).
const CORRUPT_TS_REGRESSION: u64 = 3600;

/// What to log in with. A stored auth key is tried first (it also works from an address
/// the server's allow list doesn't name); the password is the fallback.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: Option<String>,
    pub auth_key: Option<String>,
}

/// A pinned certificate that no longer matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertMismatch {
    pub host: String,
    pub old_fingerprint: String,
    pub new_fingerprint: String,
}

/// Where the connection stands, for the GUI's status line and screens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Connecting { attempt: u32 },
    LoggingIn,
    Online,
    /// The server wants credentials this client doesn't have (or refused the ones it had).
    NeedLogin { multiuser: bool, error: Option<String> },
    /// The server's certificate changed: nothing more happens until the user decides.
    CertMismatch(CertMismatch),
    Reconnecting { at: Instant, attempt: u32, reason: String },
    /// Won't reconnect on its own.
    Stopped(String),
}

/// What the core asks its owner to do.
#[derive(Clone, Debug)]
pub enum Effect {
    /// Open a connection (none is open).
    Connect,
    /// Close the open connection; a `Closed` input follows.
    Disconnect,
    Send(Box<WsMessage>),
    /// Trust this certificate from now on (`persistence::replace_pin`).
    TrustCert { host: String, fingerprint: String },
    /// The server handed this client its auth key: keep it for the next login.
    StoreAuthKey(String),
    /// Something visible changed.
    Changed,
    /// A message the core doesn't handle itself, for the GUI.
    Event(Box<WsMessage>),
    /// `/update`, its own half (src/selfupdate.rs): the server's is over - update this
    /// program's binary now and restart attached again.
    SelfUpdate { force: bool },
}

/// Which world to ask the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldRequest {
    Next,
    Prev,
    OldestPending,
}

/// What the transport reports.
#[derive(Clone, Debug)]
pub enum Input {
    /// A connection is open (TLS and WebSocket handshakes done).
    Opened,
    /// The connection closed, or the attempt failed.
    Closed(String),
    /// The attempt failed on a changed certificate.
    CertMismatch(CertMismatch),
    Message(Box<WsMessage>),
}

/// One line of a world's output, raw (escape codes and all).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Unique within this client, for caching what's drawn for it.
    pub id: u64,
    pub text: String,
    pub ts: u64,
    /// The server's number for it; `None` for a line that came without one (a notice the
    /// server never numbered).
    pub seq: Option<u64>,
    pub from_server: bool,
    pub gagged: bool,
    pub highlight: Option<String>,
    pub viewed: bool,
    /// Whose ▶ this line carries.
    pub display_id: Option<u64>,
    pub archive_sourced: bool,
    pub from_archive: bool,
    /// Came in a scrollback reply (a gap fill, the server's resume replay, a backfill):
    /// history this client is catching up on, never a live arrival for more-mode. A cold
    /// start's catch-up after the disk cache is appended at the tail like live output,
    /// so the position alone can't tell them apart.
    pub scrollback: bool,
}

#[derive(Clone, Debug, Default)]
pub struct World {
    pub name: String,
    pub connected: bool,
    pub was_connected: bool,
    pub showing_splash: bool,
    pub lines: Vec<Line>,
    pub prompt: String,
    pub pending_count: usize,
    pub unseen: usize,
    /// The world's settings as the server last described them (notes flag, reconnect
    /// interval, encoding, ...). `None` until the first InitialState.
    pub settings: Option<WorldSettingsMsg>,
    pub gmcp_user_enabled: bool,
    pub echo_masked: bool,
    /// The MUD status display's rows (GMCP/MSDP), empty when it has none.
    pub stats: Vec<StatEntry>,
    /// TF's status area for this world, once a script has made it its own.
    pub tf_status: Option<StatusView>,
    /// When a line last went to or came from the MUD (Recent Worlds): seeded from
    /// InitialState's last_recv_secs/last_send_secs, then moved on by this client.
    pub last_activity: Option<Instant>,
    /// Bumped whenever `lines` change.
    pub revision: u64,
    pub seen: SeenRanges,
    /// A shell world's screen (see `crate::shell`): `Some` for a shell world, kept current
    /// by `ShellScreen` frames. `None` for every other world type.
    pub shell: Option<crate::shell::ShellScreen>,
    epoch: u64,
    /// Text after the last newline, held until the rest of its line arrives.
    partial: String,
    total_visible: usize,
    oldest_seq: Option<u64>,
    backfill_exhausted: bool,
    gap_fill_pending: bool,
    gap_no_progress: u32,
    /// Lines kept from before a reconnect rather than taken from `InitialState`.
    hydrated: bool,
    /// Listed in this connection's `resume`: the server replays its gap unprompted.
    resumed: bool,
    replay_deadline: Option<Instant>,
    claim: Option<(HashSet<u64>, Instant)>,
}

impl World {
    /// The last line held, if it has a real seq (for the order check).
    fn last_real_seq(&self) -> Option<u64> {
        self.lines.iter().rev().find_map(|l| l.seq)
    }

    fn touch(&mut self) {
        self.revision += 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestKind {
    GapFill,
    Backfill,
    InitialFill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting,
    AwaitHello,
    AwaitAuth { with_key: bool },
    NeedLogin,
    Online,
    Closed,
}

/// A scrollback request in flight: what it was for, its world, when to give up on it.
#[derive(Clone, Copy, Debug)]
pub struct PendingRequest(RequestKind, usize, Instant);

#[derive(Clone)]
pub struct ClientCore {
    /// How many `InitialState`s have come (a new one means the worlds were sent afresh).
    pub initial_states: u64,
    /// Bumped whenever the settings may have changed (Settings windows redraw on it).
    pub settings_rev: u64,
    creds: Credentials,
    client_uid: String,
    client_version: String,
    phase: Phase,
    status: Status,
    /// Never reconnect on our own until the user acts (bad login, changed certificate).
    held: bool,
    challenge: String,
    multiuser: bool,
    key_failed: bool,
    failures: u32,
    reconnect_at: Option<Instant>,
    reload_retries: Option<u32>,
    /// Where a `/update` stands while the server's half runs (src/selfupdate.rs).
    update_flow: Option<crate::selfupdate::ClientFlow>,
    /// The user asked to start over: reconnect the moment the old connection is gone.
    restarting: bool,
    hello_deadline: Option<Instant>,
    last_rx: Instant,
    last_tick: Option<Instant>,
    next_keepalive: Option<Instant>,
    wake_deadline: Option<Instant>,

    pub display_id: u64,
    pub worlds: Vec<World>,
    pub current: usize,
    pub settings: Option<GlobalSettingsMsg>,
    /// The server's actions (F8 highlighting, shortcut tiles).
    pub actions: Vec<Action>,
    /// Worlds with activity, as the server counts them (`ActivityUpdate`).
    pub activity: usize,
    /// An operator paused this client (`/remote pause`).
    pub paused: bool,
    pub visible: bool,
    /// The ASCII splash shown before there's any world (multiuser, before connecting).
    pub splash_lines: Vec<String>,
    /// Worlds from the disk cache, offered once, at the first InitialState.
    cached: HashMap<String, CachedWorld>,
    /// The world to show when there's none on screen yet (the one last shown, from the
    /// previous run): it wins over the server's current world.
    preferred_world: Option<String>,
    view: Option<(usize, usize)>,
    /// World name -> index, for the worlds listed in this connection's `resume`.
    resume_sent: HashMap<String, usize>,
    /// The worlds this connection's other windows show (`set_other_views`).
    other_views: Vec<usize>,
    /// The server takes `OtherViews` (its `InitialState` says so); an older one would
    /// close the connection on it.
    server_takes_other_views: bool,
    /// The server takes `ExpandLine` (`InitialState.expand_line`).
    pub server_expands_line: bool,
    /// The server takes `PictureUpload` (`InitialState.picture_upload`): Import Picture.
    pub server_picture_upload: bool,
    requests: HashMap<u64, (RequestKind, usize, Instant)>,
    next_request_id: u64,
    backfill_at: Option<Instant>,
    backfill_queue: Vec<usize>,
    backfill_phase: u8,
    backfill_inflight: bool,
    next_line_id: u64,
    /// Why the last connections were given up (`ReportClientLifecycle`), sent once there's
    /// a connection again: the server's remote.log is where a remote client's trouble
    /// can be read afterwards (the Android app reports the same events).
    reports: Vec<(&'static str, String)>,
    out: Vec<Effect>,
}

impl ClientCore {
    pub fn new(creds: Credentials, client_uid: String, now: Instant) -> Self {
        ClientCore {
            initial_states: 0,
            settings_rev: 0,
            creds,
            client_uid,
            client_version: crate::VERSION.to_string(),
            phase: Phase::Idle,
            status: Status::Idle,
            held: false,
            challenge: String::new(),
            multiuser: false,
            key_failed: false,
            failures: 0,
            reconnect_at: None,
            reload_retries: None,
            update_flow: None,
            restarting: false,
            hello_deadline: None,
            last_rx: now,
            last_tick: None,
            next_keepalive: None,
            wake_deadline: None,
            display_id: 0,
            worlds: Vec::new(),
            current: 0,
            settings: None,
            actions: Vec::new(),
            activity: 0,
            paused: false,
            splash_lines: Vec::new(),
            cached: HashMap::new(),
            preferred_world: None,
            visible: true,
            view: None,
            resume_sent: HashMap::new(),
            other_views: Vec::new(),
            server_takes_other_views: false,
            server_expands_line: false,
            server_picture_upload: false,
            requests: HashMap::new(),
            next_request_id: 1,
            backfill_at: None,
            backfill_queue: Vec::new(),
            backfill_phase: 0,
            backfill_inflight: false,
            next_line_id: 1,
            reports: Vec::new(),
            out: Vec::new(),
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn is_online(&self) -> bool {
        self.phase == Phase::Online
    }

    pub fn is_multiuser(&self) -> bool {
        self.multiuser
    }

    /// Worlds from the disk cache, to stand in for what a fresh connect would download.
    pub fn set_cache(&mut self, worlds: Vec<CachedWorld>) {
        self.cached = worlds.into_iter().filter(|w| w.epoch != 0 && !w.lines.is_empty()).map(|w| (w.name.clone(), w)).collect();
    }

    /// The world to come back to (the user's last, or one just switched to).
    pub fn prefer_world(&mut self, name: Option<String>) {
        self.preferred_world = name;
    }

    /// Each world's newest lines for the disk cache, capped at `remote_initial_lines`
    /// (at least 10). Worlds without an epoch aren't cached.
    pub fn cache_snapshot(&self) -> Vec<CachedWorld> {
        let initial = self.settings.as_ref().map_or(0, |s| s.remote_initial_lines as usize);
        let cap = if initial == 0 { 100 } else { initial }.max(10);
        self.worlds
            .iter()
            .filter(|w| w.epoch != 0 && !w.lines.is_empty())
            .map(|w| {
                let lines: Vec<&Line> = w.lines.iter().filter(|l| !l.from_archive).collect();
                let lines = lines[lines.len().saturating_sub(cap)..]
                    .iter()
                    .map(|l| CachedLine {
                        text: l.text.clone(),
                        ts: l.ts,
                        seq: l.seq,
                        from_server: l.from_server,
                        gagged: l.gagged,
                        highlight: l.highlight.clone(),
                    })
                    .collect();
                CachedWorld { name: w.name.clone(), epoch: w.epoch, lines, seen: w.seen.ranges().to_vec() }
            })
            .collect()
    }

    /// Changes whenever what `cache_snapshot` would return may have.
    pub fn cache_revision(&self) -> u64 {
        self.worlds.iter().map(|w| w.revision.wrapping_add(w.epoch)).fold(self.worlds.len() as u64, u64::wrapping_add)
    }

    fn world_from_cache(&mut self, c: CachedWorld) -> World {
        let lines = c
            .lines
            .iter()
            .map(|cl| {
                let mut l = self.new_line(&cl.text, cl.ts, cl.seq);
                l.from_server = cl.from_server;
                l.gagged = cl.gagged;
                l.highlight = cl.highlight.clone();
                // Shown before, and what's new is the server's to say (ClaimedNew): taken as
                // unviewed, every cached line was guessed ▶ when its world was shown (app.js
                // caches its lines whole, `viewed` and all, so its guesses stay right).
                l.viewed = true;
                l
            })
            .collect();
        let mut seen = SeenRanges::new();
        for &(a, b) in &c.seen {
            seen.mark(a, b);
        }
        World { name: c.name, epoch: c.epoch, lines, seen, ..World::default() }
    }

    /// How far the scrollback download after a connect has got, in tens of percent
    /// (the status bar's Scrollback badge, app.js `updateScrollbackProgress`); `None` once
    /// it's done, or when there was nothing to fetch.
    pub fn backfill_progress(&self) -> Option<u32> {
        if self.backfill_phase == 0 {
            return None;
        }
        let rows = self.view.map_or(24, |v| v.0);
        let initial = self.settings.as_ref().map_or(0, |s| s.remote_initial_lines as usize);
        let target = if initial == 0 { 100 } else { initial }.max(75.max(rows));
        let (mut got, mut goal) = (0, 0);
        for w in &self.worlds {
            let g = target.min(w.total_visible);
            goal += g;
            got += w.lines.iter().filter(|l| !l.gagged).count().min(g);
        }
        (goal > 0 && got < goal).then(|| (got * 100 / goal) as u32 / 10 * 10)
    }

    fn take(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.out)
    }

    fn send(&mut self, msg: WsMessage) {
        self.out.push(Effect::Send(Box::new(msg)));
    }

    fn changed(&mut self) {
        if !matches!(self.out.last(), Some(Effect::Changed)) {
            self.out.push(Effect::Changed);
        }
    }

    fn set_status(&mut self, s: Status) {
        if self.status != s {
            self.status = s;
            self.changed();
        }
    }

    // ---- what the owner calls ----

    /// Begin: open the first connection.
    pub fn start(&mut self, _now: Instant) -> Vec<Effect> {
        self.connect();
        self.take()
    }

    fn connect(&mut self) {
        self.phase = Phase::Connecting;
        self.reconnect_at = None;
        self.set_status(Status::Connecting { attempt: self.failures + 1 });
        self.out.push(Effect::Connect);
    }

    /// The user typed credentials (after `Status::NeedLogin`).
    pub fn login(&mut self, creds: Credentials, now: Instant) -> Vec<Effect> {
        let keep_key = self.creds.auth_key.clone();
        self.creds = Credentials { auth_key: creds.auth_key.or(keep_key), ..creds };
        self.held = false;
        if self.phase == Phase::NeedLogin && !self.challenge.is_empty() {
            self.authenticate();
        } else if matches!(self.phase, Phase::Closed | Phase::Idle) {
            self.connect();
        }
        let _ = now;
        self.take()
    }

    /// Start over on a fresh connection, as reloading the web page does (the menu's
    /// Reload and Resync): nothing kept, everything fetched again.
    pub fn restart(&mut self) -> Vec<Effect> {
        self.worlds.clear();
        self.actions.clear();
        self.current = 0;
        self.activity = 0;
        self.paused = false;
        self.changed();
        match self.phase {
            Phase::Idle | Phase::Closed => {
                self.reconnect_at = None;
                self.connect();
            }
            Phase::NeedLogin => {}
            _ => {
                self.restarting = true;
                self.out.push(Effect::Disconnect);
            }
        }
        self.take()
    }

    /// Try again now rather than wait out the countdown (app.js `forceReconnect`: the
    /// connection log's Retry, Connection Lost's Reconnect).
    pub fn reconnect_now(&mut self) -> Vec<Effect> {
        if self.phase == Phase::Closed && matches!(self.status, Status::Reconnecting { .. } | Status::Stopped(_)) {
            self.held = false;
            self.connect();
            self.changed();
        }
        self.take()
    }

    /// What this client logged in with, for another window to log in the same way.
    pub fn credentials(&self) -> Credentials {
        self.creds.clone()
    }

    /// The key was deleted (Settings' Modify Key → Delete): it shows as none.
    pub fn forget_auth_key(&mut self) {
        if let Some(s) = self.settings.as_mut() {
            self.settings_rev = self.settings_rev.wrapping_add(1);
            s.auth_key.clear();
            self.changed();
        }
    }

    /// A line this client shows itself, in the current world and never sent anywhere
    /// (app.js `appendClientLine`).
    /// `/update` typed here (src/selfupdate.rs): the server's half first - its answer
    /// (`UpdateOutcome`), an old server's bounce, a reconnect after its reload, or the
    /// timeout in `tick` then yields `Effect::SelfUpdate` for this program's own half.
    pub fn start_update(&mut self, force: bool, now: Instant) -> Vec<Effect> {
        if self.update_flow.is_some() {
            return self.client_line("An update is already in progress.");
        }
        self.update_flow = Some(crate::selfupdate::ClientFlow::WaitingServer { force, since: now });
        let world_index = self.current;
        self.send(WsMessage::SendCommand { world_index, command: (if force { "/update -f" } else { "/update" }).to_string() });
        self.take()
    }

    /// Is a `/update` under way (waiting for the server, or for it to come back)?
    pub fn update_in_progress(&self) -> bool {
        self.update_flow.is_some()
    }

    pub fn client_line(&mut self, text: &str) -> Vec<Effect> {
        let mut line = self.new_line(text, unix_now(), None);
        line.from_server = false;
        line.viewed = true;
        if let Some(w) = self.worlds.get_mut(self.current) {
            w.lines.push(line);
            w.touch();
            self.changed();
        }
        self.take()
    }

    /// A command was sent: the current world's prompt and splash go, as app.js clears
    /// them (the server sends the next prompt when there is one).
    pub fn command_sent(&mut self) -> Vec<Effect> {
        if let Some(w) = self.worlds.get_mut(self.current) {
            w.prompt.clear();
            w.showing_splash = false;
            self.changed();
        }
        self.take()
    }

    /// Multiuser: end this user's session (the server answers `LoggedOut`).
    pub fn logout(&mut self) -> Vec<Effect> {
        if self.multiuser && self.is_online() {
            self.send(WsMessage::Logout);
        }
        self.take()
    }

    /// The user chose to trust a changed certificate.
    pub fn trust_cert(&mut self) -> Vec<Effect> {
        if let Status::CertMismatch(m) = self.status.clone() {
            self.out.push(Effect::TrustCert { host: m.host, fingerprint: m.new_fingerprint });
            self.held = false;
            self.connect();
        }
        self.take()
    }

    /// The window became visible or hidden (minimised).
    pub fn set_visible(&mut self, visible: bool, now: Instant) -> Vec<Effect> {
        if visible != self.visible {
            self.visible = visible;
            if self.is_online() {
                self.send(WsMessage::ClientVisibility { visible });
                if visible {
                    let cur = self.current;
                    self.claim_locally(cur, now);
                }
            }
        }
        self.take()
    }

    /// The output area holds this many rows and columns.
    pub fn set_view(&mut self, rows: usize, cols: usize) -> Vec<Effect> {
        if self.view != Some((rows, cols)) {
            self.view = Some((rows, cols));
            if self.is_online() && !self.worlds.is_empty() {
                self.send_view_state();
            }
        }
        self.take()
    }

    fn send_view_state(&mut self) {
        if let Some((rows, cols)) = self.view {
            self.send(WsMessage::UpdateViewState { world_index: self.current, visible_lines: rows, visible_columns: Some(cols) });
        }
    }

    /// The worlds this connection's other windows (`/window`) show, for the server
    /// (`WsMessage::OtherViews`): they send nothing of their own, so without it a world
    /// on screen in one of them was unseen to the server, and Unseen First sent that
    /// window back to it from everywhere. Sent when the set changes, and again at each
    /// login (a new connection is a new client to the server).
    pub fn set_other_views(&mut self, mut worlds: Vec<usize>) -> Vec<Effect> {
        worlds.sort_unstable();
        worlds.dedup();
        if worlds != self.other_views {
            self.other_views = worlds;
            if self.is_online() && self.server_takes_other_views {
                self.send(WsMessage::OtherViews { worlds: self.other_views.clone() });
            }
        }
        self.take()
    }

    /// Show another world (the client keeps its own current world, as app.js does).
    pub fn switch_world(&mut self, index: usize, now: Instant) -> Vec<Effect> {
        self.switch_to(index, now);
        self.take()
    }

    /// True if the current world is a shell world the server still reports live (keys go
    /// to it, src/shell).
    pub fn current_is_live_shell(&self) -> bool {
        self.worlds.get(self.current).and_then(|w| w.shell.as_ref()).is_some_and(|s| s.live)
    }

    /// Send a key to the current shell world's program.
    pub fn shell_key(&mut self, key: crate::websocket::ShellKey) -> Vec<Effect> {
        let world_index = self.current;
        self.send(WsMessage::ShellInput { world_index, key: Some(key), text: None, paste: false });
        self.take()
    }

    /// Send typed/IME text to the current shell world's program.
    pub fn shell_text(&mut self, text: String) -> Vec<Effect> {
        let world_index = self.current;
        self.send(WsMessage::ShellInput { world_index, key: None, text: Some(text), paste: false });
        self.take()
    }

    /// Send a paste to the current shell world's program.
    pub fn shell_paste(&mut self, text: String) -> Vec<Effect> {
        let world_index = self.current;
        self.send(WsMessage::ShellInput { world_index, key: None, text: Some(text), paste: true });
        self.take()
    }

    /// Send a mouse event over the current shell world's grid.
    #[allow(clippy::too_many_arguments)]
    pub fn shell_mouse(&mut self, kind: &str, button: u8, col: u16, row: u16, ctrl: bool, alt: bool, shift: bool) -> Vec<Effect> {
        let world_index = self.current;
        self.send(WsMessage::ShellMouse { world_index, kind: kind.to_string(), button, col, row, ctrl, alt, shift });
        self.take()
    }

    fn switch_to(&mut self, index: usize, now: Instant) {
        if index < self.worlds.len() && index != self.current {
            let previous = self.current;
            self.current = index;
            // A world that has had output since its splash shows the output (app.js
            // switchWorldLocal).
            if let Some(w) = self.worlds.get_mut(index) {
                if w.showing_splash && w.was_connected && !w.lines.is_empty() {
                    w.showing_splash = false;
                }
            }
            self.claim_locally(index, now);
            if self.is_online() {
                self.send(WsMessage::MarkWorldSeen { world_index: index, previous_world_index: Some(previous) });
                self.send(WsMessage::RequestWorldState { world_index: index });
                self.send_view_state();
            }
            self.changed();
        }
    }

    /// Ask the server which world comes next, previous, or has the oldest pending output
    /// (its switching rules: unseen-first or alphabetical). The answer, `CalculatedWorld`,
    /// switches.
    pub fn request_world(&mut self, which: WorldRequest) -> Vec<Effect> {
        let current_index = self.current;
        let msg = match which {
            WorldRequest::Next => WsMessage::CalculateNextWorld { current_index },
            WorldRequest::Prev => WsMessage::CalculatePrevWorld { current_index },
            WorldRequest::OldestPending => WsMessage::CalculateOldestPending { current_index },
        };
        if self.is_online() {
            self.send(msg);
        }
        self.take()
    }

    /// Step through the connected worlds in list order, `n` at a time (app.js
    /// `cycleConnectedWorldLocal`); negative goes back.
    pub fn cycle_connected(&mut self, n: isize, now: Instant) -> Vec<Effect> {
        let connected: Vec<usize> = (0..self.worlds.len()).filter(|&i| self.worlds[i].connected).collect();
        if !connected.is_empty() {
            let pos = connected.iter().position(|&i| i == self.current);
            let len = connected.len() as isize;
            let next = match pos {
                Some(p) => (p as isize + n).rem_euclid(len),
                None if n >= 0 => 0,
                None => len - 1,
            };
            self.switch_to(connected[next as usize], now);
        }
        self.take()
    }

    /// Release `count` screen rows of the server's held output for `world` (0 = all),
    /// counting them released straight away as app.js does.
    pub fn release_pending(&mut self, world: usize, count: usize) -> Vec<Effect> {
        if let Some(w) = self.worlds.get_mut(world) {
            if w.pending_count > 0 {
                w.pending_count = if count == 0 { 0 } else { w.pending_count.saturating_sub(count) };
                let msg = WsMessage::ReleasePending { world_index: world, count };
                if self.phase == Phase::Online {
                    self.send(msg);
                }
                self.changed();
            }
        }
        self.take()
    }

    /// Ctrl+L (app.js `redraw`): drop this client's own notices from the current world and
    /// clear its ▶ markers there.
    pub fn redraw(&mut self) -> Vec<Effect> {
        let me = self.display_id;
        if let Some(w) = self.worlds.get_mut(self.current) {
            w.lines.retain(|l| l.from_server);
            for l in &mut w.lines {
                if l.display_id == Some(me) {
                    l.display_id = None;
                }
            }
            w.claim = None;
            w.touch();
            self.changed();
        }
        self.take()
    }

    /// Anything else the GUI sends (commands and the like), if connected.
    /// A copy for another window of this connection (see `Session::follower`): the same
    /// worlds, output and settings, then the same messages as they come. Its own requests
    /// are numbered apart from this one's, so the two can never answer to each other's.
    pub fn follower(&self) -> ClientCore {
        let mut c = self.clone();
        c.out.clear();
        c.next_request_id = c.next_request_id.max(1) + (1 << 40);
        c
    }

    /// The scrollback request `id`, if this core made it and it's still waiting.
    pub fn pending_request(&self, id: u64) -> Option<PendingRequest> {
        self.requests.get(&id).map(|&(k, w, at)| PendingRequest(k, w, at))
    }

    /// Take on another core's request (a follower, before the answer to its leader's
    /// request): so the answer lands in the same place in both.
    pub fn adopt_request(&mut self, id: u64, request: PendingRequest) {
        self.requests.insert(id, (request.0, request.1, request.2));
    }

    pub fn send_message(&mut self, msg: WsMessage) -> Vec<Effect> {
        if self.is_online() {
            if let WsMessage::SendCommand { world_index, .. } = &msg {
                if let Some(w) = self.worlds.get_mut(*world_index) {
                    w.last_activity = Some(Instant::now());
                }
            }
            self.send(msg);
        }
        self.take()
    }

    pub fn handle(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        match input {
            Input::Opened => {
                self.phase = Phase::AwaitHello;
                self.challenge.clear();
                self.last_rx = now;
                self.hello_deadline = Some(now + HELLO_TIMEOUT);
                self.set_status(Status::LoggingIn);
            }
            Input::Closed(reason) => self.on_closed(reason, now),
            Input::CertMismatch(m) => {
                self.held = true;
                self.phase = Phase::Closed;
                self.reconnect_at = None;
                self.set_status(Status::CertMismatch(m));
            }
            Input::Message(msg) => {
                self.last_rx = now;
                self.on_message(*msg, now);
            }
        }
        self.take()
    }

    /// The clock moved on: keepalives, watchdogs, reconnects, the backfill pump.
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        let slept = self.last_tick.is_some_and(|t| now.saturating_duration_since(t) > SLEEP_GAP);
        self.last_tick = Some(now);
        if let Some(at) = self.reconnect_at {
            if now >= at {
                self.connect();
            }
        }
        if self.phase == Phase::AwaitHello && self.hello_deadline.is_some_and(|d| now >= d) {
            self.hello_deadline = None;
            self.out.push(Effect::Disconnect);
        }
        // The server never answered the /update: go on with our own half.
        if let Some(crate::selfupdate::ClientFlow::WaitingServer { force, since }) = self.update_flow {
            if now.saturating_duration_since(since) >= crate::selfupdate::SERVER_ANSWER_TIMEOUT {
                self.update_flow = None;
                self.client_line(&crate::selfupdate::server_no_answer());
                self.out.push(Effect::SelfUpdate { force });
            }
        }
        if self.is_online() {
            if slept && self.wake_deadline.is_none() {
                // The machine slept: a socket can look open and be long dead.
                self.wake_deadline = Some(now + WAKE_TIMEOUT);
                self.send(WsMessage::Ping);
            }
            if self.wake_deadline.is_some_and(|d| now >= d) || now.saturating_duration_since(self.last_rx) > LIVENESS {
                let silent = now.saturating_duration_since(self.last_rx).as_secs();
                match self.wake_deadline.take() {
                    Some(_) => self.reports.push(("resumeReconnect", format!("reason=pongTimeout after sleep, silent {silent}s"))),
                    None => self.reports.push(("forceReconnect", format!("reason=silent {silent}s"))),
                }
                self.out.push(Effect::Disconnect);
                return self.take();
            }
            if self.next_keepalive.is_none_or(|t| now >= t) {
                self.next_keepalive = Some(now + KEEPALIVE);
                self.send(WsMessage::Ping);
                let acked = self.ack_list();
                self.send(WsMessage::PongCheck { nonce: 0, acked });
            }
            self.watchdogs(now);
            self.pump_backfill(now);
        }
        self.take()
    }

    // ---- connection ----

    fn on_closed(&mut self, reason: String, now: Instant) {
        let was_online = self.is_online();
        let awaiting_user = self.phase == Phase::NeedLogin;
        self.phase = Phase::Closed;
        self.hello_deadline = None;
        self.wake_deadline = None;
        self.next_keepalive = None;
        self.requests.clear();
        self.backfill_inflight = false;
        self.backfill_at = None;
        self.resume_sent.clear();
        for w in &mut self.worlds {
            w.connected = false;
        }
        if awaiting_user {
            // The server gave up waiting for a login; reconnect once the user has typed it,
            // not on a timer under their login screen.
            self.held = true;
        }
        if self.held {
            return;
        }
        if was_online {
            self.failures = 0;
        }
        if std::mem::take(&mut self.restarting) {
            self.reconnect_at = None;
            self.connect();
            self.changed();
            return;
        }
        let delay = match self.reload_retries.as_mut() {
            Some(n) if *n < RELOAD_RETRIES => {
                *n += 1;
                Duration::from_secs(if *n == 1 { 2 } else { 1 })
            }
            _ => {
                self.reload_retries = None;
                self.failures += 1;
                Duration::from_secs(BACKOFF[(self.failures as usize - 1).min(BACKOFF.len() - 1)])
            }
        };
        let at = now + delay;
        self.reconnect_at = Some(at);
        self.set_status(Status::Reconnecting { at, attempt: self.failures + 1, reason });
        self.changed();
    }

    fn on_message(&mut self, msg: WsMessage, now: Instant) {
        match msg {
            WsMessage::ServerHello { multiuser_mode, challenge } => {
                self.hello_deadline = None;
                self.multiuser = multiuser_mode;
                if challenge.is_empty() {
                    // Never answer without a challenge: a bare hash replays (CLAUDE.md).
                    self.held = true;
                    self.set_status(Status::Stopped("The server sent no login challenge.".into()));
                    self.out.push(Effect::Disconnect);
                    return;
                }
                self.challenge = challenge;
                self.authenticate();
            }
            WsMessage::AuthResponse { success, error, multiuser_mode, .. } => self.on_auth_response(success, error, multiuser_mode),
            WsMessage::KeyGenerated { auth_key } => self.adopt_key(auth_key),
            WsMessage::ThemeCssVarsUpdated { colors_json, .. } => {
                // A live theme edit (the theme editor): the colours as the settings carry them.
                if let Some(s) = self.settings.as_mut() {
                    self.settings_rev = self.settings_rev.wrapping_add(1);
                    if !colors_json.is_empty() && s.theme_colors_json != colors_json {
                        s.theme_colors_json = colors_json;
                        self.changed();
                    }
                }
            }
            // The server's half of this client's /update is over (src/selfupdate.rs):
            // wait to be connected again if it reloads, else go on with our own.
            WsMessage::UpdateOutcome { reloading, .. } => {
                if let Some(flow) = self.update_flow.take() {
                    if reloading {
                        self.update_flow = Some(crate::selfupdate::ClientFlow::WaitingReconnect { force: flow.force() });
                    } else {
                        self.out.push(Effect::SelfUpdate { force: flow.force() });
                    }
                }
            }
            // A server from before UpdateOutcome bounces /update back instead of
            // answering it: it can't be updated from here, so go on with our own.
            WsMessage::ExecuteLocalCommand { command } if self.update_flow.is_some() && is_update_command(&command) => {
                let flow = self.update_flow.take().expect("checked");
                self.client_line(&crate::selfupdate::server_too_old());
                self.out.push(Effect::SelfUpdate { force: flow.force() });
            }
            // A command the server parsed that opens something on this side (app.js
            // `executeLocalCommand`): the window acts on it.
            m @ (WsMessage::ExecuteLocalCommand { .. } | WsMessage::SetInputBuffer { .. }) => self.out.push(Effect::Event(Box::new(m))),
            WsMessage::KeybindingsUpdated { bindings_json } => {
                // The key table changed (the keybind editor): the window's keys follow.
                if let Some(s) = self.settings.as_mut() {
                    self.settings_rev = self.settings_rev.wrapping_add(1);
                    if !bindings_json.is_empty() && s.keybindings_json != bindings_json {
                        s.keybindings_json = bindings_json;
                        self.changed();
                    }
                }
            }
            WsMessage::LoggedOut => {
                // Multiuser: this login is over; the connection stays for the next one.
                self.worlds.clear();
                self.actions.clear();
                self.splash_lines.clear();
                self.current = 0;
                self.activity = 0;
                self.creds.password = None;
                self.phase = Phase::NeedLogin;
                self.set_status(Status::NeedLogin { multiuser: self.multiuser, error: None });
                self.changed();
            }
            WsMessage::InitialState { worlds, settings, current_world_index, your_display_id, actions, tf_status, splash_lines, other_views, expand_line, picture_upload, .. } => {
                self.server_takes_other_views = other_views;
                self.server_expands_line = expand_line;
                self.server_picture_upload = picture_upload;
                self.initial_states += 1;
                self.actions = actions;
                self.splash_lines = splash_lines;
                self.on_initial_state(worlds, settings, current_world_index, your_display_id, now);
                // Connected again after the server's /update reload: our own half now.
                if let Some(crate::selfupdate::ClientFlow::WaitingReconnect { force }) = self.update_flow.take() {
                    self.out.push(Effect::SelfUpdate { force });
                }
                for entry in tf_status {
                    if let Some(w) = self.worlds.get_mut(entry.world_index) {
                        w.tf_status = Some(entry.view);
                    }
                }
                // Until the server's next ActivityUpdate: the other worlds with something
                // to read (app.js recomputes it the same way on connect).
                let cur = self.current;
                self.activity =
                    self.worlds.iter().enumerate().filter(|(i, w)| *i != cur && (w.unseen > 0 || w.pending_count > 0)).count();
            }
            WsMessage::GlobalSettingsUpdated { settings, .. } => {
                if !settings.auth_key.is_empty() {
                    self.adopt_key(settings.auth_key.clone());
                }
                self.settings = Some(settings);
                self.settings_rev = self.settings_rev.wrapping_add(1);
                self.changed();
            }
            WsMessage::ServerData { world_index, data, is_viewed, ts, from_server, seq, end_seq, flush, gagged, archive_sourced, highlight_colors } => {
                let d = Data { world_index, data, is_viewed, ts, from_server, seq, end_seq, flush, gagged, archive_sourced, highlight_colors };
                self.on_server_data(d);
            }
            WsMessage::ScrollbackLines { world_index, lines, backfill_complete, clamped_by_pending, request_id } => {
                self.on_scrollback(world_index, lines, backfill_complete, clamped_by_pending, request_id, now)
            }
            WsMessage::ResyncRequired { world_index, from_seq } => self.request_gap_fill(world_index, Some(from_seq), now),
            WsMessage::WorldStateResponse { world_index, pending_count, prompt, deliverable_high_seq, .. } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.pending_count = pending_count;
                    w.prompt = prompt;
                    let have = w.seen.frontier();
                    let pending = w.gap_fill_pending;
                    self.changed();
                    if have > 0 && have < deliverable_high_seq && !pending {
                        self.request_gap_fill(world_index, Some(have), now);
                    }
                }
            }
            WsMessage::PendingReleased { world_index, count } => {
                // A gap-fill held back behind the backlog can go now.
                if self.worlds.get(world_index).is_some_and(|w| w.gap_fill_pending) {
                    self.worlds[world_index].gap_fill_pending = false;
                    self.request_gap_fill(world_index, None, now);
                }
                self.out.push(Effect::Event(Box::new(WsMessage::PendingReleased { world_index, count })));
            }
            WsMessage::Ping => self.send(WsMessage::Pong),
            WsMessage::Pong => {
                if self.wake_deadline.take().is_some() {
                    // Awake and still connected: catch up on what happened meanwhile.
                    self.send(WsMessage::RequestState);
                }
            }
            WsMessage::PingCheck { nonce } => {
                let acked = self.ack_list();
                self.send(WsMessage::PongCheck { nonce, acked });
            }
            WsMessage::ServerReloading => self.reload_retries = Some(0),
            WsMessage::ClaimedNew { world_index, seqs } => self.on_claimed(world_index, seqs, now),
            WsMessage::ReleasedNew { world_index } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.claim = None;
                    let me = self.display_id;
                    let mut any = false;
                    for l in &mut w.lines {
                        if l.display_id == Some(me) {
                            l.display_id = None;
                            any = true;
                        }
                    }
                    if any {
                        w.touch();
                        self.changed();
                    }
                }
            }
            WsMessage::WorldFlushed { world_index } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.showing_splash = false;
                    w.lines.clear();
                    w.partial.clear();
                    w.touch();
                    self.changed();
                }
            }
            WsMessage::PromptUpdate { world_index, prompt } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.prompt = prompt;
                    self.changed();
                }
            }
            WsMessage::PendingLinesUpdate { world_index, count } | WsMessage::PendingCountUpdate { world_index, count } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.pending_count = count;
                    self.changed();
                }
            }
            WsMessage::UnseenUpdate { world_index, count } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.unseen = count;
                    self.changed();
                }
            }
            WsMessage::UnseenCleared { world_index } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.unseen = 0;
                    self.changed();
                }
            }
            WsMessage::WorldConnected { world_index, name } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.connected = true;
                    w.was_connected = true;
                    w.name = name;
                    self.changed();
                }
            }
            WsMessage::WorldDisconnected { world_index } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.connected = false;
                    // The connection's echo mask (a password prompt's IAC WILL ECHO) ends
                    // with it; keeping it masked every key typed into the next connection.
                    w.echo_masked = false;
                    // A shell world: freeze its screen (the session is gone server-side).
                    if let Some(screen) = w.shell.as_mut() {
                        screen.set_exited();
                    }
                    self.changed();
                }
            }
            WsMessage::ShellScreen { world_index, frame, base_frame, cols, rows, lines, cursor, mode, title, live, .. } => {
                let accepted = match self.worlds.get_mut(world_index) {
                    Some(w) => match w.shell.as_mut() {
                        Some(screen) => screen.apply_msg(frame, base_frame, cols, rows, &lines, cursor, mode, title.clone(), live),
                        None if base_frame.is_none() => {
                            let mut screen = crate::shell::ShellScreen::new(cols, rows);
                            let ok = screen.apply_msg(frame, None, cols, rows, &lines, cursor, mode, title.clone(), live);
                            w.shell = Some(screen);
                            ok
                        }
                        None => false,
                    },
                    None => true,
                };
                if !accepted {
                    // A delta we can't build on: ask for the whole screen.
                    self.send(WsMessage::RequestWorldState { world_index });
                }
                self.changed();
            }
            WsMessage::WorldAdded { world } => {
                let at = world.index.min(self.worlds.len());
                let w = self.world_from_msg(&world, true);
                self.worlds.insert(at, w);
                if self.current >= at && self.worlds.len() > 1 {
                    self.current += 1;
                }
                self.changed();
            }
            WsMessage::WorldRemoved { world_index } => {
                if world_index < self.worlds.len() {
                    self.worlds.remove(world_index);
                    if self.current > world_index || self.current >= self.worlds.len() {
                        self.current = self.current.saturating_sub(1);
                    }
                    self.requests.retain(|_, r| r.1 != world_index);
                    for r in self.requests.values_mut() {
                        if r.1 > world_index {
                            r.1 -= 1;
                        }
                    }
                    self.changed();
                }
            }
            WsMessage::ShowTagsChanged { show_tags } => {
                if let Some(s) = self.settings.as_mut() {
                    self.settings_rev = self.settings_rev.wrapping_add(1);
                    s.show_tags = show_tags;
                }
                self.changed();
            }
            // The client keeps its own current world (app.js ignores the console's switch).
            WsMessage::WorldSwitched { .. } => {}
            WsMessage::ActionsUpdated { actions } => {
                self.actions = actions;
                self.changed();
            }
            WsMessage::ActivityUpdate { count } => {
                self.activity = count;
                self.changed();
            }
            WsMessage::PausedState { paused } => {
                self.paused = paused;
                self.changed();
            }
            WsMessage::StatsUpdate { world_index, stats } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.stats = stats;
                    self.changed();
                }
            }
            WsMessage::TfStatus { world_index, view } => {
                match view {
                    // No view: TF's status area is gone everywhere.
                    None => self.worlds.iter_mut().for_each(|w| w.tf_status = None),
                    Some(v) => {
                        if let Some(w) = self.worlds.get_mut(world_index) {
                            w.tf_status = Some(v);
                        }
                    }
                }
                self.changed();
            }
            WsMessage::GmcpUserToggled { world_index, enabled } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.gmcp_user_enabled = enabled;
                    self.changed();
                }
            }
            WsMessage::EchoMaskChanged { world_index, masked } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.echo_masked = masked;
                    self.changed();
                }
            }
            WsMessage::NotesChanged { world_index, has_notes } => {
                if let Some(s) = self.worlds.get_mut(world_index).and_then(|w| w.settings.as_mut()) {
                    s.has_notes = has_notes;
                    self.changed();
                }
            }
            WsMessage::WorldSettingsUpdated { world_index, settings, name } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.settings = Some(settings);
                    w.name = name;
                    self.changed();
                }
            }
            // The server's answer to Esc-w / Esc-{ / Esc-} (or a /world). switch_to, not
            // switch_world: that one takes the queued effects, and dropping them here threw
            // away the MarkWorldSeen that clears the world's activity on the server and
            // releases this client's ▶ in the world it left.
            WsMessage::CalculatedWorld { index: Some(index) } => {
                self.switch_to(index, now);
                self.out.push(Effect::Changed);
            }
            WsMessage::CalculatedWorld { index: None } => {}
            WsMessage::WorldSwitchResult { world_index, pending_count, .. } => {
                if let Some(w) = self.worlds.get_mut(world_index) {
                    w.pending_count = pending_count;
                }
                self.switch_to(world_index, now);
                self.out.push(Effect::Changed);
            }
            other => self.out.push(Effect::Event(Box::new(other))),
        }
    }

    fn authenticate(&mut self) {
        let resume = self.ack_list();
        let resume_epochs: Vec<(usize, u64)> = self
            .worlds
            .iter()
            .enumerate()
            .filter(|(_, w)| w.seen.frontier() > 0 && w.epoch != 0)
            .map(|(i, w)| (i, w.epoch))
            .collect();
        self.resume_sent =
            resume.iter().filter_map(|&(i, _)| self.worlds.get(i).map(|w| (w.name.clone(), i))).collect();
        let base = |password_hash: String, auth_key: Option<String>, username: Option<String>| WsMessage::AuthRequest {
            username,
            password_hash,
            current_world: None,
            auth_key,
            request_key: false,
            challenge_response: true,
            resume: resume.clone(),
            resume_epochs: resume_epochs.clone(),
            client_version: self.client_version.clone(),
            client_uid: self.client_uid.clone(),
        };
        let key = self.creds.auth_key.clone().filter(|k| !k.is_empty());
        if let (Some(key), false, false) = (key, self.multiuser, self.key_failed) {
            let msg = base(String::new(), Some(hash_with_challenge(&key, &self.challenge)), None);
            self.phase = Phase::AwaitAuth { with_key: true };
            self.send(msg);
            return;
        }
        let username = self.creds.username.clone().filter(|u| !u.is_empty());
        match self.creds.password.clone() {
            Some(pw) if !self.multiuser || username.is_some() => {
                let hash = hash_with_challenge(&hash_password(pw.trim()), &self.challenge);
                let msg = base(hash, None, if self.multiuser { username } else { None });
                self.phase = Phase::AwaitAuth { with_key: false };
                self.send(msg);
            }
            _ => {
                self.phase = Phase::NeedLogin;
                self.set_status(Status::NeedLogin { multiuser: self.multiuser, error: None });
            }
        }
    }

    fn on_auth_response(&mut self, success: bool, error: Option<String>, multiuser_mode: bool) {
        let with_key = matches!(self.phase, Phase::AwaitAuth { with_key: true });
        if success {
            self.phase = Phase::Online;
            self.failures = 0;
            self.reload_retries = None;
            self.key_failed = false;
            self.next_keepalive = None;
            self.send(WsMessage::ClientTypeDeclaration { client_type: RemoteClientType::RemoteGUI });
            for (event, detail) in std::mem::take(&mut self.reports) {
                self.send(WsMessage::ReportClientLifecycle { event: event.into(), detail, source: SOURCE.into() });
            }
            self.set_status(Status::Online);
            return;
        }
        self.multiuser |= multiuser_mode || error.as_deref() == Some("Username required");
        if with_key {
            // A stale key: fall back to the password on the same connection.
            self.key_failed = true;
            self.authenticate();
            return;
        }
        self.creds.password = None;
        self.phase = Phase::NeedLogin;
        self.set_status(Status::NeedLogin { multiuser: self.multiuser, error: Some(error.unwrap_or_else(|| "Login refused".into())) });
    }

    fn adopt_key(&mut self, key: String) {
        // What Settings shows as the Auth Key (app.js serverAuthKey: KeyGenerated or
        // the settings, whichever came last).
        if let Some(s) = self.settings.as_mut() {
            self.settings_rev = self.settings_rev.wrapping_add(1);
            if !key.is_empty() && s.auth_key != key {
                s.auth_key = key.clone();
                self.changed();
            }
        }
        if !key.is_empty() && self.creds.auth_key.as_deref() != Some(key.as_str()) {
            self.creds.auth_key = Some(key.clone());
            self.key_failed = false;
            self.out.push(Effect::StoreAuthKey(key));
        }
    }

    /// `(world, contiguous frontier)` for every world with one: what `PongCheck` acks and
    /// `AuthRequest` resumes from (app.js `buildResumeAckList`).
    fn ack_list(&self) -> Vec<(usize, u64)> {
        self.worlds.iter().enumerate().filter(|(_, w)| w.seen.frontier() > 0).map(|(i, w)| (i, w.seen.frontier())).collect()
    }

    // ---- state ----

    fn new_line(&mut self, text: &str, ts: u64, seq: Option<u64>) -> Line {
        let id = self.next_line_id;
        self.next_line_id += 1;
        let text = super::decor::truncate(text).into_owned();
        Line {
            id,
            text,
            ts,
            seq,
            from_server: true,
            gagged: false,
            highlight: None,
            viewed: false,
            display_id: None,
            archive_sourced: false,
            from_archive: false,
            scrollback: false,
        }
    }

    fn line_from_ts(&mut self, t: &TimestampedLine) -> Line {
        let mut l = self.new_line(&t.text, t.ts, Some(t.seq));
        l.from_server = t.from_server;
        l.gagged = t.gagged;
        l.highlight = t.highlight_color.clone();
        l.viewed = t.viewed;
        l.display_id = t.display_id;
        l.archive_sourced = t.archive_sourced;
        l.from_archive = t.from_archive;
        l
    }

    fn world_from_msg(&mut self, m: &WorldStateMsg, with_lines: bool) -> World {
        let lines: Vec<Line> = if !with_lines {
            Vec::new()
        } else if !m.output_lines_ts.is_empty() {
            m.output_lines_ts.iter().map(|t| self.line_from_ts(t)).collect()
        } else {
            let now_secs = unix_now();
            m.output_lines.iter().map(|t| self.new_line(t, now_secs, None)).collect()
        };
        let mut w = World {
            name: m.name.clone(),
            connected: m.connected,
            was_connected: m.was_connected,
            showing_splash: m.showing_splash,
            prompt: m.prompt.clone(),
            pending_count: m.pending_count,
            unseen: m.unseen_lines,
            epoch: m.seq_epoch,
            total_visible: m.total_visible_lines.unwrap_or(m.total_output_lines),
            settings: Some(m.settings.clone()),
            gmcp_user_enabled: m.gmcp_user_enabled,
            echo_masked: m.echo_masked,
            stats: m.stats.clone(),
            shell: m.shell.as_ref().map(crate::shell::ShellScreen::from_state_msg),
            last_activity: [m.last_recv_secs, m.last_send_secs]
                .into_iter()
                .flatten()
                .min()
                .and_then(|ago| Instant::now().checked_sub(std::time::Duration::from_secs(ago))),
            lines,
            ..World::default()
        };
        rebuild_seen(&mut w, None);
        w
    }

    /// Up to six worlds other than the one shown, by their last line to or from the MUD,
    /// newest first, with how long ago (the console's Recent Worlds popup).
    pub fn recent_worlds(&self, now: Instant) -> Vec<(usize, std::time::Duration)> {
        let mut list: Vec<(usize, std::time::Duration)> = self
            .worlds
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != self.current)
            .filter_map(|(i, w)| w.last_activity.map(|at| (i, now.saturating_duration_since(at))))
            .collect();
        list.sort_by_key(|&(_, ago)| ago);
        list.truncate(6);
        list
    }

    fn on_initial_state(
        &mut self,
        worlds: Vec<WorldStateMsg>,
        settings: GlobalSettingsMsg,
        current_world_index: usize,
        your_display_id: u64,
        now: Instant,
    ) {
        self.display_id = your_display_id;
        let shown_name = self.worlds.get(self.current).map(|w| w.name.clone()).or_else(|| self.preferred_world.clone());
        let mut prior: HashMap<String, World> =
            std::mem::take(&mut self.worlds).into_iter().filter(|w| !w.lines.is_empty()).map(|w| (w.name.clone(), w)).collect();
        // A cold start: the disk cache stands in for the buffers a reconnect would keep
        // (and is checked the same way).
        for (name, c) in std::mem::take(&mut self.cached) {
            if let std::collections::hash_map::Entry::Vacant(e) = prior.entry(name) {
                e.insert(self.world_from_cache(c));
            }
        }
        let mut out = Vec::with_capacity(worlds.len());
        for (index, m) in worlds.iter().enumerate() {
            let keep = prior.remove(&m.name).filter(|p| keep_buffer(p, m));
            let mut w = match keep {
                Some(p) => {
                    // The buffer this client already has stands; the server's copy of the
                    // history is ignored (the same as app.js's in-memory hydration).
                    let mut w = self.world_from_msg(m, false);
                    w.lines = dedup_by_seq(p.lines);
                    w.seen = p.seen;
                    rebuild_seen(&mut w, None);
                    w.revision = p.revision + 1;
                    w.hydrated = true;
                    w.resumed = self.resume_sent.get(&m.name) == Some(&index);
                    w
                }
                None => self.world_from_msg(m, true),
            };
            w.oldest_seq = w.lines.iter().filter_map(|l| l.seq).min();
            w.gap_fill_pending = w.resumed;
            w.replay_deadline = w.resumed.then_some(now + REPLAY_WATCHDOG);
            if w.was_connected && !w.lines.is_empty() {
                w.showing_splash = false;
            }
            out.push(w);
        }
        self.worlds = out;
        self.current = shown_name
            .and_then(|n| self.worlds.iter().position(|w| w.name == n))
            .unwrap_or(current_world_index)
            .min(self.worlds.len().saturating_sub(1));
        self.settings = Some(settings);
        self.settings_rev = self.settings_rev.wrapping_add(1);
        if let Some(key) = self.settings.as_ref().map(|s| s.auth_key.clone()) {
            self.adopt_key(key);
        }
        // The resume record is used by exactly one InitialState (CLAUDE.md: a RequestState
        // answer is not a reconnect).
        self.resume_sent.clear();
        if !self.worlds.is_empty() {
            self.send_view_state();
            if !self.other_views.is_empty() && self.server_takes_other_views {
                self.send(WsMessage::OtherViews { worlds: self.other_views.clone() });
            }
            if self.visible {
                self.send(WsMessage::MarkWorldSeen { world_index: self.current, previous_world_index: None });
            } else {
                // app.js only says so when it changes; a window that starts minimised must
                // say so too, or text arriving meanwhile is born "viewed".
                self.send(WsMessage::ClientVisibility { visible: false });
            }
        }
        self.start_backfill(now);
        self.changed();
    }

    fn on_server_data(&mut self, d: Data) {
        if d.world_index >= self.worlds.len() {
            return;
        }
        let has_real = d.seq > 0 || d.end_seq.is_some();
        let mut gap_fill = false;
        if has_real {
            let approx_end = d.end_seq.unwrap_or(d.seq + split_lines(&d.data).len().saturating_sub(1) as u64);
            let w = &self.worlds[d.world_index];
            if !d.flush && w.seen.contains_range(d.seq, approx_end) {
                let max_seq = w.seen.max_seen();
                self.send(WsMessage::ReportDuplicate {
                    world_index: d.world_index,
                    line_seq: d.seq,
                    max_seq,
                    line_text: d.data.chars().take(200).collect(),
                    source: SOURCE.into(),
                });
                return;
            }
            gap_fill = approx_end < w.seen.max_seen();
        }
        if d.flush {
            let w = &mut self.worlds[d.world_index];
            w.showing_splash = false;
            w.lines.clear();
            w.seen.clear();
            w.partial.clear();
            w.touch();
            gap_fill = false;
        }
        let mut data = d.data;
        if d.from_server && !gap_fill {
            // Output from the MUD now, not a refill of history.
            self.worlds[d.world_index].last_activity = Some(Instant::now());
        }
        if d.from_server {
            let held = std::mem::take(&mut self.worlds[d.world_index].partial);
            if !held.is_empty() {
                data = held + &data;
            }
        }
        let ends_complete = data.ends_with('\n') || data.ends_with('\r');
        let mut raw: Vec<String> = split_lines(&data).into_iter().map(str::to_string).collect();
        if ends_complete && raw.last().is_some_and(|l| l.is_empty()) {
            raw.pop();
        }
        if d.from_server && !ends_complete {
            if let Some(p) = raw.pop() {
                self.worlds[d.world_index].partial = p;
            }
        }
        let ts = if d.ts > 0 { d.ts } else { unix_now() };
        let mut fresh = Vec::new();
        for (k, text) in raw.iter().enumerate() {
            let seq = has_real.then_some(d.seq + k as u64);
            if seq.is_some_and(|s| self.worlds[d.world_index].seen.contains(s)) {
                continue;
            }
            let mut line = self.new_line(text, ts, seq);
            line.from_server = d.from_server;
            line.gagged = d.gagged;
            line.highlight = d.highlight_colors.get(k).cloned().flatten();
            line.viewed = d.is_viewed;
            line.archive_sourced = d.archive_sourced;
            fresh.push(line);
        }
        let w = &mut self.worlds[d.world_index];
        if gap_fill {
            if let Some(first) = fresh.first().and_then(|l| l.seq) {
                let count = fresh.len();
                insert_by_seq(w, fresh);
                self.send(WsMessage::ReportOutOfOrder { world_index: d.world_index, line_seq: first, recovered_count: count, source: SOURCE.into() });
            }
        } else {
            for line in fresh {
                if let (true, Some(prev), Some(s)) = (d.seq > 0, self.worlds[d.world_index].last_real_seq(), line.seq) {
                    if s <= prev {
                        self.send(WsMessage::ReportSeqMismatch {
                            world_index: d.world_index,
                            expected_seq_gt: prev,
                            actual_seq: s,
                            line_text: line.text.chars().take(80).collect(),
                            source: SOURCE.into(),
                        });
                    }
                }
                self.worlds[d.world_index].lines.push(line);
            }
        }
        let w = &mut self.worlds[d.world_index];
        if has_real {
            // The whole span, even lines filtered out above: they used up their seqs.
            let end = d.end_seq.unwrap_or_else(|| (d.seq + raw.len() as u64).saturating_sub(1));
            if d.end_seq.is_some() || !raw.is_empty() {
                w.seen.mark(d.seq, end);
            }
        }
        w.touch();
        self.changed();
    }

    fn on_scrollback(
        &mut self,
        world_index: usize,
        lines: Vec<TimestampedLine>,
        complete: bool,
        clamped: bool,
        request_id: Option<u64>,
        now: Instant,
    ) {
        let Some(pending) = self.worlds.get(world_index).map(|w| w.gap_fill_pending) else { return };
        let registered = request_id.and_then(|id| self.requests.remove(&id));
        let kind = match (request_id, registered) {
            (Some(0), _) => {
                // The server's unprompted replay of a resumed world's gap.
                self.worlds[world_index].replay_deadline = None;
                RequestKind::GapFill
            }
            (_, Some((k, _, _))) => k,
            _ if pending => RequestKind::GapFill,
            _ => RequestKind::Backfill,
        };
        let new_lines: Vec<Line> = lines
            .iter()
            .map(|t| {
                let mut l = self.line_from_ts(t);
                l.scrollback = true;
                l
            })
            .collect();
        match kind {
            RequestKind::GapFill => {
                let w = &mut self.worlds[world_index];
                let mut appended = false;
                for line in new_lines {
                    let Some(s) = line.seq else { continue };
                    if w.seen.contains(s) {
                        continue;
                    }
                    if s > w.seen.max_seen() || w.seen.is_empty() {
                        w.lines.push(line);
                    } else {
                        insert_by_seq(w, vec![line]);
                    }
                    w.seen.mark(s, s);
                    appended = true;
                }
                if appended {
                    w.touch();
                }
                w.gap_no_progress = if appended || clamped { 0 } else { w.gap_no_progress + 1 };
                if w.gap_no_progress >= GAPFILL_MAX_NO_PROGRESS {
                    // The server can't produce this hole (flushed, or past its buffer):
                    // give it up, say so, and carry on from past it.
                    w.gap_no_progress = 0;
                    let closed = w.seen.oldest_hole().filter(|_| w.seen.close_oldest_hole());
                    if let Some((hole_start, hole_end)) = closed {
                        self.send(WsMessage::ReportGap { world_index, hole_start, hole_end, attempts: GAPFILL_MAX_NO_PROGRESS, source: SOURCE.into() });
                    } else {
                        w.gap_fill_pending = false;
                        self.changed();
                        return;
                    }
                }
                let w = &mut self.worlds[world_index];
                if complete {
                    w.gap_fill_pending = false;
                } else if !clamped {
                    // Stay armed while clamped: `PendingReleased` asks again.
                    self.request_gap_fill(world_index, None, now);
                }
            }
            RequestKind::InitialFill | RequestKind::Backfill => {
                self.backfill_inflight = false;
                let w = &mut self.worlds[world_index];
                if kind == RequestKind::InitialFill {
                    let mut add = Vec::new();
                    for line in new_lines {
                        if let Some(s) = line.seq {
                            if !w.seen.contains(s) {
                                w.seen.mark(s, s);
                                add.push(line);
                            }
                        }
                    }
                    insert_by_seq(w, add);
                } else {
                    // Older lines: prepended, and deliberately not marked as delivered —
                    // marking them would pull the frontier back to them.
                    let held: HashSet<u64> = w.lines.iter().filter_map(|l| l.seq).collect();
                    let mut older: Vec<Line> = new_lines.into_iter().filter(|l| l.seq.is_none_or(|s| !held.contains(&s))).collect();
                    older.append(&mut w.lines);
                    w.lines = older;
                }
                if let Some(min) = lines.iter().map(|l| l.seq).min() {
                    w.oldest_seq = Some(w.oldest_seq.map_or(min, |o| o.min(min)));
                }
                if complete {
                    w.backfill_exhausted = true;
                }
                w.touch();
                if self.backfill_phase == 2 && !self.backfill_queue.contains(&world_index) {
                    self.backfill_queue.push(world_index);
                }
                self.backfill_at = Some(now + Duration::from_millis(if self.backfill_phase == 1 { 0 } else { 30 }));
            }
        }
        self.changed();
    }

    /// Ask for everything after `from` (default: this client's frontier).
    fn request_gap_fill(&mut self, world_index: usize, from: Option<u64>, now: Instant) {
        if self.phase != Phase::Online {
            return;
        }
        let Some(w) = self.worlds.get_mut(world_index) else { return };
        if from.is_none() && w.seen.is_empty() {
            return;
        }
        let after = from.unwrap_or_else(|| w.seen.frontier());
        w.gap_fill_pending = true;
        let id = self.next_request_id;
        self.next_request_id += 1;
        self.requests.insert(id, (RequestKind::GapFill, world_index, now + SCROLLBACK_WATCHDOG));
        self.send(WsMessage::RequestScrollback {
            world_index,
            count: GAPFILL_COUNT,
            before_seq: None,
            after_seq: Some(after),
            request_id: Some(id),
        });
    }

    fn watchdogs(&mut self, now: Instant) {
        let expired: Vec<(u64, RequestKind, usize)> =
            self.requests.iter().filter(|(_, r)| now >= r.2).map(|(&id, r)| (id, r.0, r.1)).collect();
        for (id, kind, world) in expired {
            self.requests.remove(&id);
            match kind {
                RequestKind::GapFill => {
                    if let Some(w) = self.worlds.get_mut(world) {
                        w.gap_fill_pending = false;
                    }
                }
                _ => {
                    self.backfill_inflight = false;
                    self.backfill_at = Some(now);
                }
            }
        }
        let replays: Vec<usize> =
            self.worlds.iter().enumerate().filter(|(_, w)| w.replay_deadline.is_some_and(|d| now >= d)).map(|(i, _)| i).collect();
        for i in replays {
            let w = &mut self.worlds[i];
            w.replay_deadline = None;
            if w.gap_fill_pending {
                w.gap_fill_pending = false;
                self.request_gap_fill(i, None, now);
            }
        }
    }

    /// After `InitialState`: gap-fill the buffers kept across a reconnect that the server
    /// isn't replaying, then fetch history worlds are short of (app.js `startBackfill`).
    fn start_backfill(&mut self, now: Instant) {
        let rows = self.view.map_or(24, |v| v.0);
        let phase1 = 75.max(rows);
        for i in 0..self.worlds.len() {
            let w = &self.worlds[i];
            if w.hydrated && !w.seen.is_empty() && !w.resumed {
                self.request_gap_fill(i, None, now);
            }
        }
        let mut queue: Vec<usize> = (0..self.worlds.len())
            .filter(|&i| {
                let w = &self.worlds[i];
                w.total_visible > w.lines.len() && w.lines.len() < phase1 && !w.gap_fill_pending
            })
            .collect();
        queue.sort_by_key(|&i| i != self.current);
        self.backfill_queue = queue;
        self.backfill_phase = 1;
        self.backfill_inflight = false;
        self.backfill_at = Some(now + BACKFILL_DELAY);
    }

    fn pump_backfill(&mut self, now: Instant) {
        if self.backfill_inflight || self.backfill_at.is_none_or(|t| now < t) {
            return;
        }
        let rows = self.view.map_or(24, |v| v.0);
        let phase1 = 75.max(rows);
        let initial = self.settings.as_ref().map_or(0, |s| s.remote_initial_lines as usize);
        let total = if initial == 0 { 100 } else { initial }.max(phase1);
        let wanted = |w: &World, target: usize| {
            w.lines.len() < target && w.total_visible > w.lines.len() && !w.backfill_exhausted && !w.gap_fill_pending
        };
        loop {
            if self.backfill_phase == 1 && self.backfill_queue.is_empty() {
                self.backfill_phase = 2;
                self.backfill_queue = (0..self.worlds.len()).filter(|&i| wanted(&self.worlds[i], total)).collect();
            }
            let Some(i) = (!self.backfill_queue.is_empty()).then(|| self.backfill_queue.remove(0)) else {
                self.backfill_at = None;
                self.backfill_phase = 0;
                return;
            };
            let target = if self.backfill_phase == 1 { phase1 } else { total };
            let Some(w) = self.worlds.get(i) else { continue };
            if !wanted(w, target) {
                continue;
            }
            let count = if self.backfill_phase == 1 { target - w.lines.len() } else { 500.min(target - w.lines.len()) };
            let kind = if w.oldest_seq.is_none() { RequestKind::InitialFill } else { RequestKind::Backfill };
            let before_seq = w.oldest_seq;
            let id = self.next_request_id;
            self.next_request_id += 1;
            self.requests.insert(id, (kind, i, now + SCROLLBACK_WATCHDOG));
            self.backfill_inflight = true;
            self.backfill_at = None;
            self.send(WsMessage::RequestScrollback { world_index: i, count, before_seq, after_seq: None, request_id: Some(id) });
            return;
        }
    }

    /// Guess this client's ▶ claim before the server confirms it, so the first paint
    /// already shows the markers (app.js `claimUnviewedLocally`).
    fn claim_locally(&mut self, index: usize, now: Instant) {
        let me = self.display_id;
        let Some(w) = self.worlds.get_mut(index) else { return };
        // A guess not yet answered stays: a second visit before the server has answered the
        // first finds nothing new, and must not forget what the first guessed, or that
        // answer can't take a wrong guess back. (Leaving the world doesn't either: the server
        // releases only what it owned, so a wrong guess gets no ReleasedNew.)
        let mut guessed: HashSet<u64> = w.claim.take().filter(|(_, at)| now.saturating_duration_since(*at) < CLAIM_TTL).map(|(g, _)| g).unwrap_or_default();
        for l in &mut w.lines {
            if l.viewed {
                continue;
            }
            l.viewed = true;
            // The server grants by seq, so only a line with one can be guessed: a guess on
            // one without could never be withdrawn.
            if let (true, false, Some(s)) = (l.from_server, l.from_archive, l.seq) {
                l.display_id = Some(me);
                guessed.insert(s);
            }
        }
        w.claim = (!guessed.is_empty()).then_some((guessed, now));
        w.touch();
    }

    fn on_claimed(&mut self, index: usize, seqs: Vec<u64>, now: Instant) {
        let me = self.display_id;
        let Some(w) = self.worlds.get_mut(index) else { return };
        let granted: HashSet<u64> = seqs.into_iter().collect();
        let guessed = w.claim.take().filter(|(_, at)| now.saturating_duration_since(*at) < CLAIM_TTL).map(|(g, _)| g).unwrap_or_default();
        let mut any = false;
        for l in &mut w.lines {
            let Some(s) = l.seq else { continue };
            if granted.contains(&s) {
                if l.display_id != Some(me) || !l.viewed {
                    l.display_id = Some(me);
                    l.viewed = true;
                    any = true;
                }
            } else if guessed.contains(&s) && l.display_id == Some(me) {
                l.display_id = None;
                any = true;
            }
        }
        if any {
            w.touch();
            self.changed();
        }
    }

    /// Lines of the current world, for drawing.
    pub fn current_world(&self) -> Option<&World> {
        self.worlds.get(self.current)
    }
}

/// `ServerData`'s fields.
struct Data {
    world_index: usize,
    data: String,
    is_viewed: bool,
    ts: u64,
    from_server: bool,
    seq: u64,
    end_seq: Option<u64>,
    flush: bool,
    gagged: bool,
    archive_sourced: bool,
    highlight_colors: Vec<Option<String>>,
}

/// Split on any line ending (`\r\n`, `\n`, `\r`), as `data.split(/\r\n|\n|\r/)`.
fn split_lines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                out.push(&s[start..i]);
                i += if bytes.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                start = i;
            }
            b'\n' => {
                out.push(&s[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    out.push(&s[start..]);
    out
}

/// A line the output never shows: nothing but escape sequences (a trailing reset after a
/// newline), or a keep-alive idler marker. Blank lines do show.
///
/// app.js drops these only as they arrive live, so the same line shows as a blank row
/// when it comes back in history after a reconnect. Here every line is kept, so the
/// buffer matches the server's, and this decides at display time on every path.
/// Save the server's global settings with whatever this client changed in `s` (the
/// font slider): every field echoed back as the server sent it, so nothing else moves.
/// The password goes back empty, which the server reads as "unchanged".
pub fn settings_update(s: &GlobalSettingsMsg) -> WsMessage {
    WsMessage::UpdateGlobalSettings {
        more_mode_enabled: s.more_mode_enabled,
        spell_check_enabled: s.spell_check_enabled,
        temp_convert_enabled: s.temp_convert_enabled,
        world_switch_mode: s.world_switch_mode.clone(),
        show_tags: s.show_tags,
        debug_enabled: s.debug_enabled,
        ansi_music_enabled: s.ansi_music_enabled,
        console_theme: s.console_theme.clone(),
        gui_theme: s.gui_theme.clone(),
        gui_transparency: s.gui_transparency,
        color_offset_percent: s.color_offset_percent,
        wrapspace: s.wrapspace,
        remote_initial_lines: s.remote_initial_lines,
        input_height: s.input_height,
        // 0 = a server from before the split sent none: leave them be.
        input_height_phone: (s.input_height_phone > 0).then_some(s.input_height_phone),
        input_height_tablet: (s.input_height_tablet > 0).then_some(s.input_height_tablet),
        input_height_desktop: (s.input_height_desktop > 0).then_some(s.input_height_desktop),
        font_name: s.font_name.clone(),
        font_size: s.font_size,
        web_font_size_phone: s.web_font_size_phone,
        web_font_size_tablet: s.web_font_size_tablet,
        web_font_size_desktop: s.web_font_size_desktop,
        web_font_weight: s.web_font_weight,
        web_font_line_height: s.web_font_line_height,
        web_font_letter_spacing: s.web_font_letter_spacing,
        web_font_word_spacing: s.web_font_word_spacing,
        ws_allow_list: s.ws_allow_list.clone(),
        web_secure: true,
        http_enabled: s.http_enabled,
        http_port: s.http_port,
        web_path: s.web_path.clone(),
        ws_enabled: s.ws_enabled,
        ws_port: s.ws_port,
        ws_cert_file: s.ws_cert_file.clone(),
        ws_key_file: s.ws_key_file.clone(),
        ws_password: String::new(),
        tls_proxy_enabled: s.tls_proxy_enabled,
        dictionary_path: s.dictionary_path.clone(),
        mouse_enabled: s.mouse_enabled,
        zwj_enabled: s.zwj_enabled,
        new_line_indicator: s.new_line_indicator,
        tts_mode: s.tts_mode.clone(),
        tts_speak_mode: s.tts_speak_mode.clone(),
        tts_voice: s.tts_voice.clone(),
        scrollback_enabled: s.scrollback_enabled,
        log_input_enabled: s.log_input_enabled,
        keyboard_always_visible: s.keyboard_always_visible,
        tabs: s.tabs.clone(),
        icon_bar: s.icon_bar.clone(),
        // Empty: a server from before Import Picture sent none.
        picture_host: (!s.picture_host.is_empty()).then(|| s.picture_host.clone()),
    }
}

/// `/update` or `/update -f`, as the server echoes it back (src/selfupdate.rs).
fn is_update_command(command: &str) -> bool {
    command.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("/update"))
}

pub fn hidden_line(text: &str) -> bool {
    is_ansi_only(text) || (text.contains("###_idler_message_") && text.contains("_###"))
}

fn is_ansi_only(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0x1b && b.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_alphabetic() {
                i = j + 1;
                continue;
            }
        }
        return false;
    }
    true
}

/// Insert lines (ascending by seq) after the last held line with a lower seq (app.js
/// `insertLinesBySeq`).
fn insert_by_seq(w: &mut World, mut add: Vec<Line>) {
    add.sort_by_key(|l| l.seq);
    for line in add {
        let s = line.seq.unwrap_or(u64::MAX);
        let at = w.lines.iter().rposition(|l| l.seq.is_some_and(|x| x < s)).map_or(0, |p| p + 1);
        w.lines.insert(at, line);
    }
}

fn dedup_by_seq(lines: Vec<Line>) -> Vec<Line> {
    let mut seen = HashSet::new();
    lines.into_iter().filter(|l| l.seq.is_none_or(|s| seen.insert(s))).collect()
}

/// Whether a buffer kept from before a reconnect still belongs to the world the server
/// now describes (app.js's InitialState checks: epoch, then the old `next_seq` heuristic,
/// then corruption).
fn keep_buffer(prior: &World, m: &WorldStateMsg) -> bool {
    if m.seq_epoch != 0 {
        if prior.epoch != m.seq_epoch {
            return false;
        }
    } else if prior.seen.max_seen() > 0 && prior.seen.max_seen() >= m.next_seq {
        return false;
    }
    !is_corrupted(&prior.lines)
}

fn is_corrupted(lines: &[Line]) -> bool {
    let (mut prev_seq, mut prev_ts): (Option<u64>, Option<u64>) = (None, None);
    for l in lines {
        if l.from_archive {
            return true;
        }
        if l.ts > 0 {
            if prev_ts.is_some_and(|p| l.ts + CORRUPT_TS_REGRESSION < p) {
                return true;
            }
            prev_ts = Some(prev_ts.map_or(l.ts, |p| p.max(l.ts)));
        }
        let Some(s) = l.seq else { continue };
        if prev_seq.is_some_and(|p| s < p || (s == p && s != 0)) {
            return true;
        }
        prev_seq = Some(s);
    }
    false
}

/// The seen-record from a world's lines, plus whatever it already carried.
fn rebuild_seen(w: &mut World, carried: Option<&SeenRanges>) {
    if let Some(c) = carried {
        w.seen = c.clone();
    }
    for l in &w.lines {
        if let Some(s) = l.seq {
            w.seen.mark(s, s);
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
