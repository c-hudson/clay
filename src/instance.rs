//! Instances: one Clay among several in one process.
//!
//! A regular Clay is one `App` in one process (`InstanceKind::Main`). The two server
//! builds run many: `clay_multi` gives every logged-in user a full Clay of their own
//! (`User`, files under `~/.clay/users/<name>/`), `clay_web` gives every page that opens
//! a visitor's Clay (`Visitor`, no files of its own). Each is an OS thread with a
//! `current_thread` tokio runtime, so every task it spawns stays on that thread, and this
//! module's thread-local tells `clay_config_dir()` where that Clay's files are. Nothing
//! else about the `App` knows: it loads, saves and logs as a Clay always has.
//!
//! What an instance may not do is its `Policy`, read by the shared `App` methods both
//! server handlers call. `Policy::MAIN` is all-true, so a regular Clay takes exactly the
//! branches it took before instances existed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use crate::apploop::{InstanceCmd, InstanceTemplate};
use crate::http::{Attached, Attacher, BanList, Principal, Refused, Ticket};
use crate::websocket::WsClientInfo;
use crate::AppEvent;

/// Which Clay this thread runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceKind {
    /// The one Clay of a regular process (console, GUI, `-D`, `--local-server`), and the
    /// operator's own settings in the two server builds.
    Main,
    /// A `clay_multi` user: a full Clay in `~/.clay/users/<name>/`.
    User { name: String },
    /// A `clay_web` visitor: a Clay built from the operator's settings in memory, one
    /// world, no files of its own.
    Visitor { id: u64 },
}

/// The thread-local context of an instance thread.
#[derive(Debug, Clone)]
pub struct InstanceCtx {
    pub kind: InstanceKind,
    /// Where `clay_config_dir()` points on this thread; `None` = the process's `~/.clay`.
    pub config_dir: Option<PathBuf>,
    /// `user:alice`, `visitor:7` - the prefix of this instance's debug.log lines and the
    /// key of the per-instance slots that used to be process-wide.
    pub label: String,
}

impl InstanceCtx {
    pub fn user(name: &str, config_dir: PathBuf) -> Self {
        Self { kind: InstanceKind::User { name: name.to_string() }, config_dir: Some(config_dir), label: format!("user:{name}") }
    }

    pub fn visitor(id: u64) -> Self {
        Self { kind: InstanceKind::Visitor { id }, config_dir: None, label: format!("visitor:{id}") }
    }
}

thread_local! {
    static INSTANCE: RefCell<Option<InstanceCtx>> = const { RefCell::new(None) };
}

/// Set once some instance thread has started: from then on a config-dir lookup with no
/// context on a thread other than the main one is suspicious (see `clay_config_dir`).
static ANY_INSTANCE_STARTED: AtomicBool = AtomicBool::new(false);

/// clay_web runs from one directory of files: the one its `web.dat` is in (the current
/// directory, or `--conf=`'s). Set once at start (`server::init_web_only`), before anything
/// could touch `~/.clay`, which is then never read, written or created:
/// `clay_root_config_dir()` returns this directory instead (so `remote.log`, the
/// certificate and an SSL world's `known_hosts.dat` land beside web.dat), `debug_log` and
/// `output_debug_log` write nothing, the crash log goes to stderr, there is no `run/<pid>`
/// marker, `migrate_legacy_config_files` does nothing, settings are never saved and no
/// `secure.key` is made (user request and rulings 2026-11-10; the inventory of every disk
/// touch point is in .claude/rules/instances.md).
static WEB_ONLY: OnceLock<PathBuf> = OnceLock::new();

/// Make this process clay_web's: `dir` is web.dat's directory, absolute and existing.
pub fn set_web_only(dir: PathBuf) {
    let _ = WEB_ONLY.set(dir);
}

/// clay_web's one directory, when this process is its server.
pub fn web_only_dir() -> Option<&'static Path> {
    WEB_ONLY.get().map(|p| p.as_path())
}

pub fn is_web_only() -> bool {
    WEB_ONLY.get().is_some()
}

/// Make this thread an instance's. Called once, first thing on the instance thread,
/// before its `App` is built.
pub fn set_current(ctx: InstanceCtx) {
    ANY_INSTANCE_STARTED.store(true, Ordering::Relaxed);
    let _ = INSTANCE.try_with(|slot| *slot.borrow_mut() = Some(ctx));
}

/// Run `f` as if on `ctx`'s thread, then put the thread's own context back. For work
/// the main thread does on an instance's behalf before it exists (reading a user's
/// settings.dat to seed the auth-key index).
pub fn with_scoped<R>(ctx: InstanceCtx, f: impl FnOnce() -> R) -> R {
    let previous = INSTANCE.try_with(|slot| slot.borrow_mut().replace(ctx)).unwrap_or(None);
    let result = f();
    let _ = INSTANCE.try_with(|slot| *slot.borrow_mut() = previous);
    result
}

/// This thread's context, or None for the main Clay - and None once the thread's locals
/// are being destroyed: `debug_log` (and so the panic hook) reads this from `Drop`s that
/// run while a thread ends, and `LocalKey::with` panics there ("cannot access a Thread
/// Local Storage value during or after destruction"), which turned the console's `/quit`
/// into an abort, the hook panicking over the panic (user report 2026-11-10). `try_with`
/// everywhere here, never `with`.
fn read<R>(f: impl FnOnce(&InstanceCtx) -> R) -> Option<R> {
    INSTANCE.try_with(|slot| slot.borrow().as_ref().map(f)).ok().flatten()
}

/// The directory this thread's Clay keeps its files in, when it isn't `~/.clay`.
pub fn config_dir_override() -> Option<PathBuf> {
    read(|c| c.config_dir.clone()).flatten()
}

/// `main`, `user:<name>` or `visitor:<id>`.
pub fn label() -> String {
    read(|c| c.label.clone()).unwrap_or_else(|| "main".to_string())
}

/// What a debug.log line from this thread starts with: nothing for the main Clay,
/// `[user:alice] ` for an instance.
pub fn log_prefix() -> String {
    read(|c| format!("[{}] ", c.label)).unwrap_or_default()
}

pub fn kind() -> InstanceKind {
    read(|c| c.kind.clone()).unwrap_or(InstanceKind::Main)
}

pub fn is_main() -> bool {
    read(|_| ()).is_none()
}

/// A config-dir lookup with no context after instances have started, on a thread that is
/// not the process's main thread: the one way an instance's file could land in the
/// operator's `~/.clay`. Logged once per thread name so a blocking-pool thread that does
/// it is named in debug.log without flooding it.
pub(crate) fn note_unscoped_access() {
    if !ANY_INSTANCE_STARTED.load(Ordering::Relaxed) {
        return;
    }
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    if name == "main" {
        return;
    }
    static NOTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut noted) = NOTED.lock() {
        if noted.iter().any(|n| n == name) {
            return;
        }
        noted.push(name.to_string());
    }
    crate::debug_log(true, &format!("CTX-MISS: config path asked for on thread '{name}' with no instance context"));
}

/// What an instance may do. Every field is a permission; `MAIN` grants them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// A typed `/line` is a command (else every typed line is text for world 0).
    pub parse_commands: bool,
    /// settings.dat, theme.dat and keybindings.dat may be written.
    pub save_files: bool,
    /// Files the user names (`/note <file>`, `/log <file>`), and Clay's own per-instance
    /// files (archive, maps, session logs) exist.
    pub user_paths: bool,
    /// `/sh` and the other process-starting commands.
    pub processes: bool,
    /// `/reload`, `/update`, `/server`, bans, the port mapping, the firewall, `/reach`.
    pub server_wide: bool,
    /// Sound and speech may be played on this machine.
    pub sound_here: bool,
    /// `ChangePassword` is answered.
    pub change_password: bool,
    /// `UpdateGlobalSettings`' web-server fields (port, path, password, certificate, allow
    /// list) take effect.
    pub web_fields: bool,
    /// May write process-wide state: the debug flag, the process environment.
    pub process_globals: bool,
    /// May regenerate or delete its auth key.
    pub auth_key: bool,
    /// `RequestSettingsExport` / `ImportSettings` are answered.
    pub settings_export: bool,
    /// Connect world 0 as soon as a client has logged in (a visitor's page).
    pub auto_connect_on_login: bool,
    /// Import Picture: `InitialState.picture_upload` and `PictureUpload` are answered.
    pub pictures: bool,
    /// GMCP is accepted from the game (`TelnetConfig::gmcp_enabled`) and a client may toggle
    /// or send it. A clay_web visitor's game gets `DONT GMCP` (user ruling 2026-11-11).
    pub gmcp: bool,
}

impl Policy {
    /// A regular Clay: everything, and no automatic connect.
    pub const MAIN: Policy = Policy {
        parse_commands: true,
        save_files: true,
        user_paths: true,
        processes: true,
        server_wide: true,
        sound_here: true,
        change_password: true,
        web_fields: true,
        process_globals: true,
        auth_key: true,
        settings_export: true,
        auto_connect_on_login: false,
        pictures: true,
        gmcp: true,
    };

    /// A `clay_multi` user: a full Clay minus the server's files, processes, sound and
    /// server-wide commands. Passwords are the operator's (password.dat).
    pub const fn user() -> Policy {
        Policy {
            parse_commands: true,
            save_files: true,
            user_paths: true,
            processes: false,
            server_wide: false,
            sound_here: false,
            change_password: false,
            web_fields: false,
            process_globals: false,
            auth_key: true,
            settings_export: true,
            auto_connect_on_login: false,
            pictures: true,
            gmcp: true,
        }
    }

    /// A `clay_web` visitor: text to one world, nothing written, nothing server-wide.
    pub const fn visitor() -> Policy {
        Policy {
            parse_commands: false,
            save_files: false,
            user_paths: false,
            processes: false,
            server_wide: false,
            sound_here: false,
            change_password: false,
            web_fields: false,
            process_globals: false,
            auth_key: false,
            settings_export: false,
            auto_connect_on_login: true,
            pictures: false,
            gmcp: false,
        }
    }

    pub fn for_kind(kind: &InstanceKind) -> Policy {
        match kind {
            InstanceKind::Main => Policy::MAIN,
            InstanceKind::User { .. } => Policy::user(),
            InstanceKind::Visitor { .. } => Policy::visitor(),
        }
    }

    /// Without `parse_commands` (a clay_web visitor), the messages a client may still send
    /// besides its typed lines: the view's upkeep, output sync, its one world's connect and
    /// disconnect, spell suggestions, `/info`, MSDP - and a picture upload or GMCP only where
    /// `pictures` / `gmcp` allow (a visitor's don't). Everything else - editors, settings,
    /// other worlds, TF expansion, notes, server-wide requests - is refused with this line.
    /// `None` = handled as on any Clay.
    pub fn message_refusal(&self, msg: &crate::websocket::WsMessage) -> Option<&'static str> {
        use crate::websocket::WsMessage as M;
        if self.parse_commands {
            return None;
        }
        let allowed = match msg {
            M::ConnectWorld { world_index } | M::DisconnectWorld { world_index } | M::SwitchWorld { world_index } => *world_index == 0,
            M::UpdateViewState { .. } | M::UpdateDimensions { .. } | M::ClientTypeDeclaration { .. } | M::ClientVisibility { .. }
            | M::ResumeSession | M::MarkWorldSeen { .. } | M::OtherViews { .. } | M::MapWatch { .. }
            | M::Ping | M::PongCheck { .. } | M::RequestState | M::RequestScrollback { .. } | M::ReleasePending { .. }
            | M::SelectiveFlush { .. } | M::RequestConnectionsList | M::RequestInfo { .. }
            | M::CalculateNextWorld { .. } | M::CalculatePrevWorld { .. } | M::CalculateOldestPending { .. }
            | M::ReportSeqMismatch { .. } | M::ReportDuplicate { .. } | M::ReportOutOfOrder { .. } | M::ReportGap { .. }
            | M::ReportClientLifecycle { .. } | M::SpellSuggest { .. } | M::SendMsdp { .. } | M::Logout => true,
            M::PictureUpload { .. } => self.pictures,
            M::ToggleWorldGmcp { .. } | M::SendGmcp { .. } => self.gmcp,
            _ => false,
        };
        if allowed { None } else { Some("Not available on this page.") }
    }
}

/// clay_multi's accounts, `~/.clay/password.dat`: one `username password` per line, split
/// at the FIRST space (a password may have spaces; a username may not), blank and `#`
/// lines ignored. Clay only ever reads it - at every login attempt, so the operator adds a
/// user without a restart - and never writes it. A bad line is logged by number, never by
/// content.
pub struct Credentials {
    /// username -> `websocket::hash_password` of the password.
    users: HashMap<String, String>,
}

impl Credentials {
    pub const FILE: &'static str = "password.dat";

    /// A username is also a directory name (`~/.clay/users/<name>/`): letters, digits,
    /// `_`, `.` and `-`, up to 32, not starting with `.`.
    pub fn valid_username(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 32
            && !name.starts_with('.')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    }

    /// Parse the file's text: the accounts, and each problem found as `line N: ...`.
    pub fn parse(text: &str) -> (Credentials, Vec<String>) {
        let mut users = HashMap::new();
        let mut problems = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = raw.trim_end_matches('\r');
            let line = line.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, password)) = line.split_once(' ') else {
                problems.push(format!("line {n}: no space between the username and the password"));
                continue;
            };
            if !Self::valid_username(name) {
                problems.push(format!("line {n}: a username is letters, digits, _ . and -, up to 32, not starting with ."));
                continue;
            }
            if password.is_empty() {
                problems.push(format!("line {n}: an empty password"));
                continue;
            }
            if users.insert(name.to_string(), crate::websocket::hash_password(password)).is_some() {
                problems.push(format!("line {n}: '{name}' is listed twice; this line wins"));
            }
        }
        (Credentials { users }, problems)
    }

    /// Read the file; problems go to debug.log.
    pub fn load(path: &Path) -> std::io::Result<Credentials> {
        let text = std::fs::read_to_string(path)?;
        let (creds, problems) = Self::parse(&text);
        for p in problems {
            crate::debug_log(true, &format!("{} {p}", Self::FILE));
        }
        Ok(creds)
    }

    /// Does `client_hash` answer for `username`? With a challenge the client sent
    /// `SHA256(SHA256(password) + challenge)`; from loopback it may send the bare hash.
    /// An unknown user and a wrong password are the same `false` (no enumeration).
    pub fn check(&self, username: &str, client_hash: &str, challenge: &str, uses_challenge: bool) -> bool {
        let Some(stored) = self.users.get(username) else { return false };
        if uses_challenge {
            let expected = crate::websocket::hash_with_challenge(stored, challenge);
            crate::util::constant_time_eq(expected.as_bytes(), client_hash.as_bytes())
        } else {
            crate::util::constant_time_eq(stored.as_bytes(), client_hash.as_bytes())
        }
    }

    pub fn has_user(&self, name: &str) -> bool {
        self.users.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.users.len()
    }

    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    pub fn usernames(&self) -> Vec<String> {
        let mut names: Vec<String> = self.users.keys().cloned().collect();
        names.sort();
        names
    }
}

/// The auth keys of clay_multi's users, key -> user, so a key login or a knock names its
/// user before that user's Clay is asked. Seeded from every `~/.clay/users/*/settings.dat`
/// at start, kept current by each instance when it regenerates or deletes its key
/// (`App::publish_auth_key`). Every lookup compares every key in constant time, with no
/// early exit.
#[derive(Default)]
pub struct KeyIndex {
    keys: RwLock<HashMap<String, String>>,
}

impl KeyIndex {
    /// `user`'s key is now `key` (None: the user has none).
    pub fn set(&self, user: &str, key: Option<&str>) {
        let mut keys = self.keys.write().unwrap_or_else(|e| e.into_inner());
        keys.retain(|_, u| u != user);
        if let Some(k) = key.filter(|k| !k.is_empty()) {
            keys.insert(k.to_string(), user.to_string());
        }
    }

    /// The user whose key answers a WebSocket key login: `response` is
    /// `SHA256(key + challenge)` as hex, or the bare key from loopback.
    pub fn user_for_response(&self, challenge: &str, response: &str, uses_challenge: bool) -> Option<String> {
        let keys = self.keys.read().unwrap_or_else(|e| e.into_inner());
        let mut found = None;
        for (key, user) in keys.iter() {
            let matches = if uses_challenge {
                crate::util::constant_time_eq(crate::websocket::hash_with_challenge(key, challenge).as_bytes(), response.as_bytes())
            } else {
                crate::util::constant_time_eq(key.as_bytes(), response.as_bytes())
            };
            if matches && found.is_none() {
                found = Some(user.clone());
            }
        }
        found
    }

    /// The user whose key answers a CLAY-KNOCK challenge (`http::knock_expected_response`).
    pub fn knock_user(&self, challenge: &[u8; 32], response: &[u8; 32]) -> Option<String> {
        let keys = self.keys.read().unwrap_or_else(|e| e.into_inner());
        let mut found = None;
        for (key, user) in keys.iter() {
            let expected = crate::http::knock_expected_response(key, challenge);
            if crate::util::constant_time_eq(&expected, response) && found.is_none() {
                found = Some(user.clone());
            }
        }
        found
    }

    pub fn len(&self) -> usize {
        self.keys.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Visitor sessions one address may hold at once on clay_web (user ruling 2026-11-09).
pub const MAX_VISITORS_PER_IP: usize = 5;
/// Visitor sessions a clay_web holds at once, unless settings.dat says otherwise
/// (`web_front_max_sessions`).
pub const DEFAULT_MAX_VISITORS: usize = 200;

/// Which server an `InstanceManager` runs.
pub enum ManagerKind {
    /// clay_multi: the users of password.dat, each a Clay in `users_dir/<name>/`.
    Multi { password_path: PathBuf, users_dir: PathBuf },
    /// clay_web: visitors, each a Clay built from the operator's scrubbed settings.
    WebFront { template: Arc<InstanceTemplate>, max_visitors: usize },
}

/// A running instance, as its manager and the front know it.
pub struct InstanceHandle {
    pub clients: Arc<RwLock<HashMap<u64, WsClientInfo>>>,
    pub event_tx: tokio::sync::mpsc::Sender<AppEvent>,
    cmd_tx: tokio::sync::mpsc::Sender<InstanceCmd>,
    alive: Arc<AtomicBool>,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// A visitor's address, whose per-address count is given back when it ends.
    ip: Option<String>,
}

impl InstanceHandle {
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Ask the instance to flush and end (`InstanceCmd::Shutdown`).
    fn ask_shutdown(&self, reloading: bool) {
        let _ = self.cmd_tx.try_send(InstanceCmd::Shutdown { reloading });
    }

    /// Wait up to `timeout` for the instance to end, then join its thread; one still
    /// running at the deadline is left to itself (its files may be mid-write, as the
    /// console's bounded /quit accepts too).
    fn wait_ended(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while self.is_alive() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if let Some(join) = self.join.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = join.join();
        }
        true
    }
}

/// The Clays a server runs (`http::Attacher` for the front): a user's starts at the first
/// login and lives until the process ends (its worlds stay connected while nobody is
/// attached, as a Clay's do); a visitor's starts with its page and ends with its socket.
pub struct InstanceManager {
    kind: ManagerKind,
    users: Mutex<HashMap<String, Arc<InstanceHandle>>>,
    visitors: Mutex<HashMap<u64, Arc<InstanceHandle>>>,
    per_ip: Mutex<HashMap<String, usize>>,
    next_visitor: AtomicU64,
    /// Every user's auth key, for key logins and the knock.
    pub key_index: Arc<KeyIndex>,
    /// Client ids are the front's, shared by every instance's table.
    pub next_client_id: Arc<Mutex<u64>>,
    ban_list: BanList,
}

impl InstanceManager {
    pub fn new(kind: ManagerKind, ban_list: BanList) -> Self {
        Self {
            kind,
            users: Mutex::new(HashMap::new()),
            visitors: Mutex::new(HashMap::new()),
            per_ip: Mutex::new(HashMap::new()),
            next_visitor: AtomicU64::new(0),
            key_index: Arc::new(KeyIndex::default()),
            next_client_id: Arc::new(Mutex::new(1)),
            ban_list,
        }
    }

    pub fn is_multi(&self) -> bool {
        matches!(self.kind, ManagerKind::Multi { .. })
    }

    /// Every user directory's auth key into the index, at start (a user's instance keeps
    /// it current afterwards, `App::publish_auth_key`).
    pub fn seed_key_index(&self) {
        let ManagerKind::Multi { users_dir, .. } = &self.kind else { return };
        let Ok(entries) = std::fs::read_dir(users_dir) else { return };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else { continue };
            if !Credentials::valid_username(&name) || !entry.path().is_dir() {
                continue;
            }
            let key = with_scoped(InstanceCtx::user(&name, entry.path()), || {
                let mut app = crate::App::new();
                let _ = crate::persistence::load_settings(&mut app);
                app.settings.websocket_auth_key.as_ref().map(|k| k.key.clone())
            });
            self.key_index.set(&name, key.as_deref());
        }
    }

    /// Start a Clay on a thread of its own and wait for it to be ready for clients.
    fn spawn(&self, ctx: InstanceCtx, template: Option<InstanceTemplate>, ip: Option<String>) -> Result<Arc<InstanceHandle>, String> {
        type Boot = Result<(Arc<RwLock<HashMap<u64, WsClientInfo>>>, tokio::sync::mpsc::Sender<AppEvent>), String>;
        let (boot_tx, boot_rx) = std::sync::mpsc::channel::<Boot>();
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<InstanceCmd>(8);
        let alive = Arc::new(AtomicBool::new(true));
        let alive_in_thread = alive.clone();
        let label = ctx.label.clone();
        let label_in_thread = label.clone();
        let ban_list = self.ban_list.clone();
        let counter = self.next_client_id.clone();
        let key_index = self.key_index.clone();
        let user_name = match &ctx.kind {
            InstanceKind::User { name } => Some(name.clone()),
            _ => None,
        };
        let thread = std::thread::Builder::new()
            .name(format!("clay-{}", label.replace(':', "-")))
            .spawn(move || {
                set_current(ctx);
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                        Ok(rt) => rt,
                        Err(e) => {
                            let _ = boot_tx.send(Err(format!("no runtime: {e}")));
                            return;
                        }
                    };
                    rt.block_on(async move {
                        let server = crate::websocket::WebSocketServer::new("", 0, "", None, false, ban_list)
                            .with_shared_client_ids(counter);
                        let prepared = crate::apploop::prepare_app(crate::apploop::SetupOptions {
                            load_reload_state: false,
                            ws_override: None,
                            port_override: None,
                            web_server: crate::apploop::WebServer::None,
                            template,
                            signals: false,
                            gui_tx: None,
                            gui_repaint: None,
                            ws_server: Some(server),
                        }).await;
                        let crate::apploop::Prepared { mut app, event_tx, event_rx, .. } = match prepared {
                            Ok(p) => p,
                            Err(e) => {
                                let _ = boot_tx.send(Err(e.to_string()));
                                return;
                            }
                        };
                        // An instance's TinyFugue is restricted from its first line: no
                        // server files, no processes (and no script of its own can lower it).
                        app.tf_engine.restrict_level = crate::tf::RestrictLevel::File;
                        if let Some(name) = user_name {
                            app.key_index = Some((key_index, name));
                            app.publish_auth_key();
                        }
                        crate::apploop::tf_startup(&mut app, &event_tx, crate::apploop::TfStartup::StartupActionsOnly).await;
                        let clients = match app.ws_server.as_ref() {
                            Some(s) => s.clients.clone(),
                            None => {
                                let _ = boot_tx.send(Err("no client table".to_string()));
                                return;
                            }
                        };
                        let _ = boot_tx.send(Ok((clients, event_tx.clone())));
                        let exit = crate::apploop::run_app_loop(&mut app, event_rx, event_tx, crate::apploop::LoopOptions {
                            gui_rx: None,
                            gui_quit: false,
                            cmd_rx: Some(cmd_rx),
                            system_message: crate::apploop::SystemMessageSink::Output,
                            hot_reload: false,
                        }).await;
                        crate::debug_log(true, &format!("instance ended: {exit:?}"));
                    });
                }));
                alive_in_thread.store(false, Ordering::SeqCst);
                let how = if outcome.is_err() { "panic" } else { "ended" };
                crate::http::log_remote_event("INSTANCE-ENDED", &label_in_thread, how);
            })
            .map_err(|e| format!("no thread: {e}"))?;
        match boot_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok((clients, event_tx))) => {
                crate::http::log_remote_event("INSTANCE-START", &label, "");
                Ok(Arc::new(InstanceHandle { clients, event_tx, cmd_tx, alive, join: Mutex::new(Some(thread)), ip }))
            }
            Ok(Err(e)) => Err(e),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err("did not start in time".to_string()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("ended before it started".to_string()),
        }
    }

    fn attached_of(handle: &InstanceHandle, username: Option<String>, ticket: Ticket) -> Attached {
        Attached { clients: handle.clients.clone(), event_tx: handle.event_tx.clone(), username, ticket }
    }

    fn give_back_ip(&self, ip: &str) {
        let mut per_ip = self.per_ip.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = per_ip.get_mut(ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                per_ip.remove(ip);
            }
        }
    }

    /// Every instance: flush and end, telling their clients `ServerReloading` first when
    /// the server is about to exec itself; waits (bounded) for each.
    pub fn shutdown_all(&self, reloading: bool) {
        let mut handles: Vec<Arc<InstanceHandle>> = Vec::new();
        handles.extend(self.users.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, h)| h));
        handles.extend(self.visitors.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, h)| h));
        for h in &handles {
            h.ask_shutdown(reloading);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        for h in &handles {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            h.wait_ended(left);
        }
    }

    /// How many instances run now: (users, visitors).
    pub fn counts(&self) -> (usize, usize) {
        let users = self.users.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|h| h.is_alive()).count();
        let visitors = self.visitors.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|h| h.is_alive()).count();
        (users, visitors)
    }
}

impl Attacher for InstanceManager {
    fn key_user(&self, challenge: &str, response: &str, uses_challenge: bool) -> Option<String> {
        self.key_index.user_for_response(challenge, response, uses_challenge)
    }

    fn check_user_password(&self, username: &str, client_hash: &str, challenge: &str, uses_challenge: bool) -> bool {
        let ManagerKind::Multi { password_path, .. } = &self.kind else { return false };
        match Credentials::load(password_path) {
            Ok(creds) => creds.check(username, client_hash, challenge, uses_challenge),
            Err(e) => {
                crate::debug_log(true, &format!("{} unreadable: {e}", Credentials::FILE));
                false
            }
        }
    }

    fn attach(&self, who: Principal, _client_id: u64) -> Result<Attached, Refused> {
        match who {
            Principal::User(name) => {
                let ManagerKind::Multi { users_dir, .. } = &self.kind else { return Err(Refused::UnknownUser) };
                if !Credentials::valid_username(&name) {
                    return Err(Refused::UnknownUser);
                }
                let mut users = self.users.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(handle) = users.get(&name) {
                    if handle.is_alive() {
                        return Ok(Self::attached_of(handle, Some(name.clone()), Ticket::User(name)));
                    }
                }
                // The user's first login, or their Clay ended (a panic): start it from its files.
                let handle = self.spawn(InstanceCtx::user(&name, users_dir.join(&name)), None, None).map_err(Refused::InstanceFailed)?;
                users.insert(name.clone(), handle.clone());
                Ok(Self::attached_of(&handle, Some(name.clone()), Ticket::User(name)))
            }
            Principal::Visitor { ip } => {
                let ManagerKind::WebFront { template, max_visitors } = &self.kind else { return Err(Refused::UnknownUser) };
                {
                    let visitors = self.visitors.lock().unwrap_or_else(|e| e.into_inner());
                    let mut per_ip = self.per_ip.lock().unwrap_or_else(|e| e.into_inner());
                    if visitors.values().filter(|h| h.is_alive()).count() >= *max_visitors {
                        return Err(Refused::ServerFull);
                    }
                    let count = per_ip.entry(ip.clone()).or_insert(0);
                    if *count >= MAX_VISITORS_PER_IP {
                        return Err(Refused::TooManyFromIp);
                    }
                    *count += 1;
                }
                let id = self.next_visitor.fetch_add(1, Ordering::SeqCst) + 1;
                let tpl = InstanceTemplate { settings: template.settings.clone(), world_name: template.world_name.clone(), world: template.world.clone(), theme: template.theme.clone() };
                match self.spawn(InstanceCtx::visitor(id), Some(tpl), Some(ip.clone())) {
                    Ok(handle) => {
                        self.visitors.lock().unwrap_or_else(|e| e.into_inner()).insert(id, handle.clone());
                        Ok(Self::attached_of(&handle, None, Ticket::Visitor(id)))
                    }
                    Err(e) => {
                        self.give_back_ip(&ip);
                        Err(Refused::InstanceFailed(e))
                    }
                }
            }
        }
    }

    fn release(&self, ticket: &Ticket, _client_id: u64) {
        if let Ticket::Visitor(id) = ticket {
            let handle = self.visitors.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
            if let Some(handle) = handle {
                handle.ask_shutdown(false);
                handle.wait_ended(Duration::from_secs(5));
                if let Some(ip) = &handle.ip {
                    self.give_back_ip(ip);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Drop` that runs while a thread's locals are being destroyed may still log (a
    /// writer's, a runtime's): the accessors answer their defaults there instead of
    /// panicking. With `LocalKey::with` the console's `/quit` aborted - the panic hook's
    /// own `debug_log` panicked over the panic (user report 2026-11-10). On Linux the
    /// locals are destroyed in the reverse order of their first use, so a local used
    /// before `INSTANCE` is destroyed after it, and its `Drop` reads a destroyed key.
    #[test]
    fn the_accessors_survive_a_threads_teardown() {
        use std::sync::mpsc;
        struct LogsOnDrop(mpsc::Sender<String>);
        impl Drop for LogsOnDrop {
            fn drop(&mut self) {
                let _ = self.0.send(format!("{}|{}|{:?}|{}", log_prefix(), label(), config_dir_override(), is_main()));
                crate::debug_log(true, "a Drop at thread end logs");
            }
        }
        thread_local! { static LATE: std::cell::RefCell<Option<LogsOnDrop>> = const { std::cell::RefCell::new(None) }; }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            LATE.with(|l| *l.borrow_mut() = Some(LogsOnDrop(tx)));
            set_current(InstanceCtx::user("alice", PathBuf::from("/nonexistent/alice")));
            assert_eq!(log_prefix(), "[user:alice] ");
        })
        .join()
        .expect("the thread ends without a panic in its locals' destruction");
        let seen = rx.recv().expect("the Drop ran and logged");
        // Whatever the destruction order gave, every accessor answered.
        assert!(seen == "|main|None|true" || seen == "[user:alice] |user:alice|Some(\"/nonexistent/alice\")|false", "{seen}");
    }

    #[test]
    fn the_test_thread_is_the_main_clay() {
        assert!(is_main());
        assert_eq!(kind(), InstanceKind::Main);
        assert_eq!(label(), "main");
        assert_eq!(log_prefix(), "");
        assert_eq!(config_dir_override(), None);
        assert_eq!(Policy::for_kind(&kind()), Policy::MAIN);
    }

    #[test]
    fn with_scoped_sets_and_restores_the_context() {
        let dir = std::env::temp_dir().join(format!("clay-instance-test-{}", std::process::id()));
        let seen = with_scoped(InstanceCtx::user("alice", dir.clone()), || {
            (kind(), label(), log_prefix(), config_dir_override(), is_main())
        });
        assert_eq!(seen.0, InstanceKind::User { name: "alice".into() });
        assert_eq!(seen.1, "user:alice");
        assert_eq!(seen.2, "[user:alice] ");
        assert_eq!(seen.3, Some(dir));
        assert!(!seen.4);
        assert!(is_main(), "the thread's own context comes back");
        assert_eq!(config_dir_override(), None);
    }

    #[test]
    fn a_visitor_has_no_directory_of_its_own() {
        let seen = with_scoped(InstanceCtx::visitor(7), || (kind(), label(), config_dir_override()));
        assert_eq!(seen.0, InstanceKind::Visitor { id: 7 });
        assert_eq!(seen.1, "visitor:7");
        assert_eq!(seen.2, None);
    }

    #[test]
    fn policies_grant_what_each_kind_may_do() {
        let main = Policy::MAIN;
        assert!(main.parse_commands && main.save_files && main.user_paths && main.processes && main.server_wide
            && main.sound_here && main.change_password && main.web_fields && main.process_globals
            && main.auth_key && main.settings_export && !main.auto_connect_on_login);
        let user = Policy::user();
        assert!(user.parse_commands && user.save_files && user.user_paths && user.auth_key && user.settings_export);
        assert!(!user.processes && !user.server_wide && !user.sound_here && !user.change_password
            && !user.web_fields && !user.process_globals && !user.auto_connect_on_login);
        let visitor = Policy::visitor();
        assert!(!visitor.parse_commands && !visitor.save_files && !visitor.user_paths && !visitor.processes
            && !visitor.server_wide && !visitor.sound_here && !visitor.change_password && !visitor.web_fields
            && !visitor.process_globals && !visitor.auth_key && !visitor.settings_export);
        assert!(visitor.auto_connect_on_login);
        assert_eq!(Policy::for_kind(&InstanceKind::User { name: "a".into() }), Policy::user());
        assert_eq!(Policy::for_kind(&InstanceKind::Visitor { id: 1 }), Policy::visitor());
    }

    #[test]
    fn password_dat_is_split_at_the_first_space() {
        let text = "# accounts\nalice secret word\n\n  bob\tpw\ncarol\nd.ave x\n.hidden x\nbad name x\nalice again\nemptypw \n";
        let (creds, problems) = Credentials::parse(text);
        assert_eq!(creds.usernames(), ["alice", "bad", "d.ave"], "{problems:?}");
        assert!(creds.check("bad", &crate::websocket::hash_password("name x"), "", false), "the first space splits: 'bad' / 'name x'");
        assert!(creds.check("alice", &crate::websocket::hash_password("again"), "", false), "the later line wins");
        assert!(!creds.check("alice", &crate::websocket::hash_password("secret word"), "", false));
        assert!(problems.iter().any(|p| p.starts_with("line 4:")), "bob with a tab, not a space: {problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("line 5:")), "carol alone: {problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("line 7:")), ".hidden: {problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("line 9:") && p.contains("twice")), "{problems:?}");
        assert!(problems.iter().any(|p| p.starts_with("line 10:")), "an empty password: {problems:?}");
        assert_eq!(problems.len(), 5, "{problems:?}");
        assert!(!problems.iter().any(|p| p.contains("secret") || p.contains("pw")), "never a line's content");
    }

    #[test]
    fn credentials_check_answers_a_challenge_or_a_bare_hash() {
        let (creds, problems) = Credentials::parse("alice secret word\nd.ave x-y_z\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert!(creds.has_user("alice") && creds.has_user("d.ave"));
        let challenge = "0123456789abcdef";
        let hash = crate::websocket::hash_password("secret word");
        assert!(creds.check("alice", &crate::websocket::hash_with_challenge(&hash, challenge), challenge, true));
        assert!(creds.check("alice", &hash, "", false), "a bare hash (loopback)");
        assert!(!creds.check("alice", &crate::websocket::hash_password("secret"), "", false), "the first word is not the password");
        assert!(!creds.check("alice", &crate::websocket::hash_with_challenge(&hash, "other"), challenge, true));
        assert!(!creds.check("nobody", &hash, "", false));
        assert!(Credentials::valid_username("a_b.c-d9") && !Credentials::valid_username(".x") && !Credentials::valid_username("a b") && !Credentials::valid_username("") && !Credentials::valid_username(&"a".repeat(33)));
    }

    #[test]
    fn key_index_names_the_user_of_a_key() {
        let index = KeyIndex::default();
        index.set("alice", Some("key-a"));
        index.set("bob", Some("key-b"));
        assert_eq!(index.len(), 2);
        let challenge = "feedface";
        assert_eq!(index.user_for_response(challenge, &crate::websocket::hash_with_challenge("key-b", challenge), true), Some("bob".to_string()));
        assert_eq!(index.user_for_response("", "key-a", false), Some("alice".to_string()));
        assert_eq!(index.user_for_response(challenge, "nope", true), None);
        let knock = [7u8; 32];
        assert_eq!(index.knock_user(&knock, &crate::http::knock_expected_response("key-a", &knock)), Some("alice".to_string()));
        assert_eq!(index.knock_user(&knock, &[0u8; 32]), None);
        // A regenerated key replaces the old one; a deleted key is gone.
        index.set("alice", Some("key-a2"));
        assert_eq!(index.user_for_response("", "key-a", false), None);
        assert_eq!(index.user_for_response("", "key-a2", false), Some("alice".to_string()));
        index.set("bob", None);
        assert_eq!(index.len(), 1);
        assert_eq!(index.user_for_response("", "key-b", false), None);
    }

    fn login_of(user: &str) -> crate::websocket::WsMessage {
        crate::websocket::WsMessage::AuthRequest {
            username: Some(user.to_string()), password_hash: String::new(), current_world: None, auth_key: None,
            request_key: false, challenge_response: false, resume: Vec::new(), resume_epochs: Vec::new(),
            client_version: String::new(), client_uid: "uid".to_string(), instance_id: String::new(), interface_key: String::new(),
        }
    }

    fn as_message(out: crate::websocket::Outbound) -> Option<crate::websocket::WsMessage> {
        match out {
            crate::websocket::Outbound::Message(m) => Some(*m),
            crate::websocket::Outbound::Shared(json) => serde_json::from_str(&json).ok(),
        }
    }

    /// clay_multi: a user's Clay starts at the first attach (its own directory), a second
    /// login joins it, its InitialState says what a user's Clay is, and it outlives its
    /// connections until the server shuts everything down.
    #[test]
    fn a_users_instance_starts_on_attach_and_answers_a_login() {
        let base = crate::test_home_dir().join("instance-manager-user");
        let users_dir = base.join("users");
        std::fs::create_dir_all(&users_dir).unwrap();
        let password_path = base.join(Credentials::FILE);
        std::fs::write(&password_path, "alice pw\n").unwrap();
        let manager = InstanceManager::new(ManagerKind::Multi { password_path, users_dir: users_dir.clone() }, BanList::new());
        assert!(manager.is_multi());
        assert!(manager.check_user_password("alice", &crate::websocket::hash_password("pw"), "", false));
        assert!(!manager.check_user_password("alice", &crate::websocket::hash_password("nope"), "", false));
        assert!(!manager.check_user_password("bob", &crate::websocket::hash_password("pw"), "", false));
        assert!(matches!(manager.attach(Principal::User("../x".into()), 1), Err(Refused::UnknownUser)), "no path tricks");

        let attached = manager.attach(Principal::User("alice".into()), 1).expect("alice's Clay starts");
        assert_eq!(attached.username.as_deref(), Some("alice"));
        assert_eq!(attached.ticket, Ticket::User("alice".into()));
        assert!(users_dir.join("alice").is_dir(), "her own directory");
        assert_eq!(manager.counts(), (1, 0));
        let again = manager.attach(Principal::User("alice".into()), 2).expect("a second login joins the same Clay");
        assert!(Arc::ptr_eq(&attached.clients, &again.clients));
        assert_eq!(manager.counts(), (1, 0));

        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::websocket::Outbound>(64);
        attached.clients.write().unwrap().insert(1, crate::websocket::new_client_info(tx, "10.0.0.1", true, Some("alice".into())));
        attached.event_tx.blocking_send(AppEvent::WsClientConnected(1)).unwrap();
        attached.event_tx.blocking_send(AppEvent::WsClientMessage(1, Box::new(login_of("alice")))).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut seen = None;
        while std::time::Instant::now() < deadline && seen.is_none() {
            match rx.try_recv() {
                Ok(out) => {
                    if let Some(crate::websocket::WsMessage::InitialState { web_front, server_sound, change_password, .. }) = as_message(out) {
                        seen = Some((web_front, server_sound, change_password));
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert_eq!(seen, Some((false, false, false)), "a user's InitialState: not a page, no server sound, no Change Password");

        manager.release(&attached.ticket, 1);
        assert_eq!(manager.counts(), (1, 0), "a user's Clay stays when a connection ends");
        manager.shutdown_all(false);
        assert_eq!(manager.counts(), (0, 0));
    }

    /// clay_web: each visitor is a Clay of its own, at most five per address, none at all
    /// when the server is full, gone when its ticket is given back; a web front has no users.
    #[test]
    fn visitors_are_capped_per_address_and_end_with_their_ticket() {
        let template = || InstanceTemplate {
            settings: crate::Settings::default().visitor_template(),
            world_name: "Gate".to_string(),
            world: {
                let mut w = crate::WorldSettings::default();
                w.hostname = "127.0.0.1".to_string();
                w.port = "1".to_string();
                w.visitor_copy()
            },
            theme: crate::theme::ThemeFile::with_defaults(),
        };
        let manager = InstanceManager::new(ManagerKind::WebFront { template: Arc::new(template()), max_visitors: 50 }, BanList::new());
        assert!(!manager.is_multi());
        assert!(matches!(manager.attach(Principal::User("alice".into()), 1), Err(Refused::UnknownUser)), "no users on a web front");
        let ip = || Principal::Visitor { ip: "203.0.113.9".to_string() };
        let mut tickets = Vec::new();
        for i in 0..MAX_VISITORS_PER_IP {
            let a = manager.attach(ip(), i as u64 + 1).expect("a visitor");
            assert!(a.username.is_none());
            assert!(matches!(a.ticket, Ticket::Visitor(_)));
            tickets.push(a.ticket);
        }
        assert_eq!(manager.counts(), (0, MAX_VISITORS_PER_IP));
        assert!(matches!(manager.attach(ip(), 9), Err(Refused::TooManyFromIp)), "the sixth from one address");
        let other = manager.attach(Principal::Visitor { ip: "203.0.113.10".to_string() }, 10).expect("another address");
        manager.release(&tickets[0], 1);
        assert_eq!(manager.counts(), (0, MAX_VISITORS_PER_IP), "one gone, one from elsewhere");
        let back = manager.attach(ip(), 11).expect("room again for that address");
        manager.release(&other.ticket, 10);
        manager.release(&back.ticket, 11);
        manager.shutdown_all(false);
        assert_eq!(manager.counts(), (0, 0));

        let small = InstanceManager::new(ManagerKind::WebFront { template: Arc::new(template()), max_visitors: 1 }, BanList::new());
        let only = small.attach(Principal::Visitor { ip: "198.51.100.1".to_string() }, 1).expect("the one seat");
        assert!(matches!(small.attach(Principal::Visitor { ip: "198.51.100.2".to_string() }, 2), Err(Refused::ServerFull)));
        small.release(&only.ticket, 1);
        assert_eq!(small.counts(), (0, 0));
    }

    /// A visitor's page has no Import Picture and no GMCP (user rulings 2026-11-11): the
    /// two client messages are refused like an editor's would be; MSDP, a spell suggestion
    /// and the world's connect stay. Users and the main Clay keep both.
    #[test]
    fn a_visitor_may_not_upload_pictures_or_touch_gmcp() {
        use crate::websocket::WsMessage as M;
        let picture = M::PictureUpload { id: 1, name: "a.png".into(), size: 1, offset: 0, data: String::new(), last: true };
        let toggle = M::ToggleWorldGmcp { world_index: 0 };
        let gmcp = M::SendGmcp { world_index: 0, package: "Core.Hello".into(), data: "{}".into() };
        let msdp = M::SendMsdp { world_index: 0, variable: "LIST".into(), value: "COMMANDS".into() };
        let v = Policy::visitor();
        assert!(!v.pictures && !v.gmcp);
        for refused in [&picture, &toggle, &gmcp] {
            assert_eq!(v.message_refusal(refused), Some("Not available on this page."));
        }
        assert_eq!(v.message_refusal(&msdp), None);
        assert_eq!(v.message_refusal(&M::ConnectWorld { world_index: 0 }), None);
        for p in [Policy::MAIN, Policy::user()] {
            assert!(p.pictures && p.gmcp);
            assert_eq!(p.message_refusal(&picture), None);
            assert_eq!(p.message_refusal(&toggle), None);
        }
    }
}
