//! What this machine's Clay starts as: the Preferred Interface and the default server
//! (user request 2026-11-21). Kept in `~/.clay/client.dat` on the machine that starts
//! Clay and written only by the program running there - never in settings.dat, never on
//! the wire: an attached GUI's other settings are the server's, these are its own
//! machine's ("on a per host basis and not stored on the main instance").
//!
//! ```text
//! [client]
//! preferred_interface=gui        tui | gui | web-gui; absent = Clay's own default
//! default_server=host:port       absent = none
//! [server:host:port]
//! username=...                   a clay_multi server's account
//! password=ENC:...               encrypted as world passwords are (secure.key)
//! auth_key=ENC:...
//! ```
//!
//! **When it applies** (user rulings 2026-11-21): only to a start whose command line names
//! no interface (a bare `clay`, the desktop launcher). `--gui` and `--console` are always
//! a standalone Clay that connects to no other, and a host on the command line is used
//! as given - with the login saved for it here, unless `CLAY_PASSWORD`/`CLAY_AUTH_KEY` say
//! otherwise. A login is kept per server (`host:port`), so every attach to a server with
//! one saved uses it: the default server, `--gui=host`, `/server host` and a relaunch.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub const FILE: &str = "client.dat";
/// The port a server address without one means (as `--console=host` and `--gui=host`).
pub const DEFAULT_PORT: u16 = 9000;

/// The interfaces a Clay on this machine can start in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interface {
    Tui,
    Gui,
    WebGui,
}

impl Interface {
    /// Every one, in the order a select lists them.
    pub const ALL: [Interface; 3] = [Interface::Tui, Interface::Gui, Interface::WebGui];
    /// The order a start falls back in when the preferred one can't run here.
    pub const FALLBACK: [Interface; 3] = [Interface::Gui, Interface::WebGui, Interface::Tui];

    /// The name client.dat and the wire use.
    pub fn name(self) -> &'static str {
        match self {
            Interface::Tui => "tui",
            Interface::Gui => "gui",
            Interface::WebGui => "web-gui",
        }
    }

    /// What a select shows (the user's words: "TUI, gui, or web gui").
    pub fn label(self) -> &'static str {
        match self {
            Interface::Tui => "TUI",
            Interface::Gui => "GUI",
            Interface::WebGui => "Web GUI",
        }
    }

    /// From a name or a label, case-insensitively (`console` is the TUI).
    pub fn from_name(s: &str) -> Option<Interface> {
        let s = s.trim();
        Interface::ALL.into_iter().find(|i| s.eq_ignore_ascii_case(i.name()) || s.eq_ignore_ascii_case(i.label()))
            .or_else(|| s.eq_ignore_ascii_case("console").then_some(Interface::Tui))
    }

    /// The command-line flag that starts it.
    pub fn flag(self) -> &'static str {
        match self {
            Interface::Tui => "--console",
            Interface::Gui => "--gui",
            Interface::WebGui => "--web-gui",
        }
    }
}

/// The login saved for one server. Empty strings are "none".
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ServerLogin {
    pub username: String,
    pub password: String,
    pub auth_key: String,
}

/// What `{:?}` shows of a secret: whether there is one, never what it is (a log line or a
/// panic message must not carry a password; webview_gui.rs's `InheritedAuth` does the same).
fn redacted(secret: &str) -> &'static str {
    if secret.is_empty() { "" } else { "<redacted>" }
}

impl std::fmt::Debug for ServerLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerLogin")
            .field("username", &self.username)
            .field("password", &redacted(&self.password))
            .field("auth_key", &redacted(&self.auth_key))
            .finish()
    }
}

impl ServerLogin {
    pub fn is_empty(&self) -> bool {
        self.username.is_empty() && self.password.is_empty() && self.auth_key.is_empty()
    }

    /// The user name, when one is saved.
    pub fn username(&self) -> Option<&str> {
        Some(self.username.as_str()).filter(|u| !u.is_empty())
    }

    pub fn password(&self) -> Option<&str> {
        Some(self.password.as_str()).filter(|p| !p.is_empty())
    }

    pub fn auth_key(&self) -> Option<&str> {
        Some(self.auth_key.as_str()).filter(|k| !k.is_empty())
    }
}

/// The name a server's login is kept under: `host:port`, lower-cased, the port 9000 when
/// none is given, any scheme and web path left off - what `--console=host`'s pin and the
/// native GUI's `Target::key` are kept under too. Empty for an empty address.
pub fn server_key(addr: &str) -> String {
    let addr = addr.trim();
    if addr.is_empty() {
        return String::new();
    }
    let rest = addr.split_once("://").map(|(_, r)| r).unwrap_or(addr);
    let host_port = rest.split('/').next().unwrap_or(rest);
    let host_port = if let Some(v6) = host_port.strip_prefix('[') {
        match v6.split_once(']') {
            Some((h, after)) if after.starts_with(':') => format!("[{h}]{after}"),
            Some((h, _)) => format!("[{h}]:{DEFAULT_PORT}"),
            None => host_port.to_string(),
        }
    } else {
        match host_port.rsplit_once(':') {
            Some((h, _)) if !h.contains(':') => host_port.to_string(),
            Some(_) => format!("[{host_port}]:{DEFAULT_PORT}"), // a bare IPv6 address
            None => format!("{host_port}:{DEFAULT_PORT}"),
        }
    };
    host_port.to_lowercase()
}

/// Whether `addr` holds only what an address is made of: ASCII letters and digits and
/// `. - _ : / [ ] %` (a name or address, a port, a `ws://` scheme, a web path, an IPv6
/// zone). The value goes into a command line, the log and the WebView page, so nothing
/// else - no space, quote, backslash or control character - is ever kept.
pub fn valid_address(addr: &str) -> bool {
    !addr.is_empty() && addr.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:/[]%".contains(c))
}

/// What the Server page edits: the default server and its login.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ServerPage {
    /// The host, with a web path after it when the server has one (`host/path`). Blank:
    /// no default server.
    pub host: String,
    /// Blank is 9000.
    pub port: String,
    pub username: String,
    pub password: String,
    pub auth_key: String,
}

impl std::fmt::Debug for ServerPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerPage")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &redacted(&self.password))
            .field("auth_key", &redacted(&self.auth_key))
            .finish()
    }
}

/// The Server page as a page that is never given the stored secrets edits it (the WebView
/// GUI's, `ClientConf::set_server_page_edit`): a secret is `None` when it wasn't typed.
#[derive(Clone, Default)]
pub struct ServerPageEdit {
    pub host: String,
    pub port: String,
    pub username: String,
    /// `None`: keep what is saved for this server. `Some`: this one ("" removes it).
    pub password: Option<String>,
    pub auth_key: Option<String>,
    /// Drop what is saved for this server first (the page's Forget).
    pub forget: bool,
}

impl ServerPage {
    /// The address it names (`host:port[/path]`), or `None` with no host. A port typed
    /// into the Host field (`host:4000`) is used when the Port field is blank.
    pub fn address(&self) -> Result<Option<String>, String> {
        let host = self.host.trim();
        if host.is_empty() {
            return Ok(None);
        }
        if host.chars().any(char::is_whitespace) {
            return Err("A host has no spaces.".to_string());
        }
        if !valid_address(host) {
            return Err("A host has only letters, digits and . - _ : / [ ] %".to_string());
        }
        let (hostpart, path) = match host.find('/') {
            Some(i) if !host[..i].ends_with(':') => (&host[..i], &host[i..]),
            _ => (host, ""),
        };
        let typed_port = self.port.trim();
        if !typed_port.is_empty() {
            match typed_port.parse::<u16>() {
                Ok(p) if p > 0 => {}
                _ => return Err(format!("'{typed_port}' is not a port (1-65535).")),
            }
        }
        let has_port = match hostpart.strip_prefix('[') {
            Some(v6) => v6.split_once(']').is_some_and(|(_, after)| after.starts_with(':')),
            None => hostpart.rsplit_once(':').is_some_and(|(h, _)| !h.contains(':')),
        };
        let addr = if has_port {
            if !typed_port.is_empty() {
                // `host:4000` with a Port too: the Port field wins.
                let bare = match hostpart.strip_prefix('[') {
                    Some(v6) => format!("[{}]", v6.split_once(']').map(|(h, _)| h).unwrap_or(v6)),
                    None => hostpart.rsplit_once(':').map(|(h, _)| h.to_string()).unwrap_or_default(),
                };
                format!("{bare}:{typed_port}{path}")
            } else {
                format!("{hostpart}{path}")
            }
        } else {
            let port = if typed_port.is_empty() { DEFAULT_PORT.to_string() } else { typed_port.to_string() };
            let hostpart = if hostpart.contains(':') && !hostpart.starts_with('[') { format!("[{hostpart}]") } else { hostpart.to_string() };
            format!("{hostpart}:{port}{path}")
        };
        Ok(Some(addr))
    }
}

/// The Server page's help, in every interface that has one.
pub const SERVER_PAGE_HELP: &[&str] = &[
    "Server - this machine's default server",
    "",
    "A bare `clay` on this machine (no --console, --gui or",
    "--web-gui; the desktop launcher too) attaches to this",
    "Clay instead of running its own, in the Preferred",
    "Interface (Settings > General). `clay --gui` and",
    "`clay --console` always run this machine's own Clay.",
    "",
    "Host: the other Clay's address, with its web path",
    "  after it if it has one (host/path). Blank: no",
    "  default server.",
    "Port: its web port (9000 when blank).",
    "Username: only for a multiuser (clay_multi) server.",
    "Password / Auth Key: its login. The key is tried",
    "  first, then the password; without either Clay",
    "  asks. Any attach to this server uses them, as",
    "  `--gui=host` and `/server host` do.",
    "",
    "Kept on this machine (client.dat), the password and",
    "key encrypted as world passwords are; never sent to",
    "the Clay you are attached to.",
];

/// A bare `/server` where nothing starts Clay on this machine (a browser; app.js's copy is
/// pinned to this one).
pub const SERVER_PAGE_ELSEWHERE: &str = "A bare /server sets the default server of a machine that starts Clay, in its GUI or console; a browser starts no Clay, so it has none.";

/// What a Save of the Server page says.
pub fn saved_message(conf: &ClientConf) -> String {
    match conf.default_server() {
        Some(addr) => format!("Saved. A bare `clay` on this machine now attaches to {addr}."),
        None => "Saved. No default server: a bare `clay` runs this machine's own Clay.".to_string(),
    }
}

/// This machine's client.dat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientConf {
    /// `None`: Clay's own default (the GUI where a window can open, else the console).
    pub preferred: Option<Interface>,
    /// `host:port[/path]`, empty for none.
    pub default_server: String,
    /// Logins by `server_key`.
    pub servers: BTreeMap<String, ServerLogin>,
}

fn path() -> PathBuf {
    crate::clay_config_path(FILE)
}

/// Tests that write client.dat (one per test process, `get_home_dir`'s temp home) hold
/// this, or two of them running at once would read each other's file.
#[cfg(test)]
pub(crate) static FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl ClientConf {
    /// Read client.dat's text; secrets are decrypted with this machine's key. An `ENC:`
    /// value this machine can't decrypt (another machine's file, a replaced secure.key) is
    /// no secret at all: used as a literal password it would be refused at every start,
    /// and five refusals ban this address on the server. An address with anything but
    /// address characters (`valid_address`) is dropped the same way.
    pub fn parse(text: &str) -> ClientConf {
        fn secret(stored: &str) -> String {
            let plain = crate::persistence::decrypt_password(stored);
            if stored.starts_with("ENC:") && plain == stored { String::new() } else { plain }
        }
        let mut conf = ClientConf::default();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_string();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if section == "client" {
                match k {
                    "preferred_interface" => conf.preferred = Interface::from_name(v),
                    "default_server" if valid_address(v) => conf.default_server = v.to_string(),
                    _ => {}
                }
            } else if let Some(server) = section.strip_prefix("server:").filter(|s| valid_address(s.trim())) {
                let login = conf.servers.entry(server_key(server)).or_default();
                match k {
                    "username" => login.username = v.chars().filter(|c| !c.is_control()).collect(),
                    "password" => login.password = secret(v),
                    "auth_key" => login.auth_key = secret(v),
                    _ => {}
                }
            }
        }
        conf.servers.retain(|k, l| !k.is_empty() && !l.is_empty());
        conf
    }

    /// client.dat's text, secrets encrypted.
    pub fn to_text(&self) -> String {
        let mut text = String::from("# This machine's Clay start-up choices (Settings > General's Preferred Interface,\n# Settings > Server). Written by Clay; the passwords and keys are encrypted.\n[client]\n");
        if let Some(i) = self.preferred {
            text.push_str(&format!("preferred_interface={}\n", i.name()));
        }
        if !self.default_server.is_empty() {
            text.push_str(&format!("default_server={}\n", self.default_server));
        }
        for (server, login) in &self.servers {
            if server.is_empty() || login.is_empty() {
                continue;
            }
            text.push_str(&format!("\n[server:{server}]\n"));
            if !login.username.is_empty() {
                text.push_str(&format!("username={}\n", login.username));
            }
            if !login.password.is_empty() {
                text.push_str(&format!("password={}\n", crate::persistence::encrypt_password(&login.password)));
            }
            if !login.auth_key.is_empty() {
                text.push_str(&format!("auth_key={}\n", crate::persistence::encrypt_password(&login.auth_key)));
            }
        }
        text
    }

    pub fn load() -> ClientConf {
        std::fs::read_to_string(path()).map(|t| ClientConf::parse(&t)).unwrap_or_default()
    }

    /// Write client.dat (owner-only: it holds credentials), unless it already says this
    /// with every secret encrypted (a file written by hand is rewritten encrypted).
    pub fn save(&self) -> Result<(), String> {
        if let Ok(text) = std::fs::read_to_string(path()) {
            let encrypted = text.lines().filter_map(|l| l.trim().split_once('='))
                .filter(|(k, _)| matches!(k.trim(), "password" | "auth_key"))
                .all(|(_, v)| v.trim().starts_with("ENC:"));
            if encrypted && ClientConf::parse(&text) == *self {
                return Ok(());
            }
        }
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // A file made by hand keeps its mode through a write (`write_secret_file` sets the
        // mode only on a file it creates): owner-only before a secret goes into it.
        #[cfg(unix)]
        if p.exists() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("Couldn't save {}: {e}", p.display()))?;
        }
        crate::util::write_secret_file(&p, self.to_text().as_bytes())
            .map_err(|e| format!("Couldn't save {}: {e}", p.display()))
    }

    /// The login saved for the server at `addr`, if any.
    pub fn login_for(&self, addr: &str) -> Option<ServerLogin> {
        self.servers.get(&server_key(addr)).filter(|l| !l.is_empty()).cloned()
    }

    /// The default server, if one is set.
    pub fn default_server(&self) -> Option<&str> {
        Some(self.default_server.trim()).filter(|s| !s.is_empty())
    }

    /// The Server page as this file has it.
    pub fn server_page(&self) -> ServerPage {
        let Some(addr) = self.default_server() else { return ServerPage::default() };
        let (hostport, path) = match addr.find('/') {
            Some(i) => (&addr[..i], &addr[i..]),
            None => (addr, ""),
        };
        let (host, port) = match hostport.strip_prefix('[') {
            Some(v6) => match v6.split_once(']') {
                Some((h, after)) => (format!("[{h}]"), after.strip_prefix(':').unwrap_or("").to_string()),
                None => (hostport.to_string(), String::new()),
            },
            None => match hostport.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') => (h.to_string(), p.to_string()),
                _ => (hostport.to_string(), String::new()),
            },
        };
        let login = self.login_for(addr).unwrap_or_default();
        ServerPage {
            host: format!("{host}{path}"),
            port: if port.is_empty() { DEFAULT_PORT.to_string() } else { port },
            username: login.username,
            password: login.password,
            auth_key: login.auth_key,
        }
    }

    /// Take the Server page's values: the default server and that server's login. A blank
    /// Host clears the default and keeps every saved login.
    pub fn set_server_page(&mut self, page: &ServerPage) -> Result<(), String> {
        let Some(addr) = page.address()? else {
            self.default_server.clear();
            return Ok(());
        };
        if page.username.chars().any(char::is_control) {
            return Err("A user name has no control characters.".to_string());
        }
        let key = server_key(&addr);
        let login = ServerLogin {
            username: page.username.trim().to_string(),
            password: page.password.clone(),
            auth_key: page.auth_key.trim().to_string(),
        };
        if login.is_empty() {
            self.servers.remove(&key);
        } else {
            self.servers.insert(key, login);
        }
        self.default_server = addr;
        Ok(())
    }

    /// The same from a page that was never given the stored secrets (the WebView GUI's):
    /// a secret it didn't supply stays as saved **for that same server** - a page that
    /// changes the host never moves one server's login to another - and `forget` drops
    /// what is saved for it first.
    pub fn set_server_page_edit(&mut self, edit: &ServerPageEdit) -> Result<(), String> {
        let base = ServerPage { host: edit.host.clone(), port: edit.port.clone(), username: edit.username.clone(), ..Default::default() };
        let Some(addr) = base.address()? else {
            self.default_server.clear();
            return Ok(());
        };
        let saved = if edit.forget { ServerLogin::default() } else { self.login_for(&addr).unwrap_or_default() };
        self.set_server_page(&ServerPage {
            password: edit.password.clone().unwrap_or(saved.password),
            auth_key: edit.auth_key.clone().unwrap_or(saved.auth_key),
            ..base
        })
    }
}

/// What can run on this machine for a start: each interface's build and, for the GUIs,
/// whether a window can open here; for the TUI, whether there is a terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Can {
    pub tui: bool,
    pub gui: bool,
    pub web_gui: bool,
}

impl Can {
    pub fn has(self, i: Interface) -> bool {
        match i {
            Interface::Tui => self.tui,
            Interface::Gui => self.gui,
            Interface::WebGui => self.web_gui,
        }
    }

    /// This start: a foreground terminal for the TUI; for the GUIs, the build and
    /// `platform::gui_can_open` (not over SSH, a display listening).
    pub fn at_start() -> Can {
        let window = crate::platform::GUI_BUILT && crate::platform::gui_can_open();
        Can {
            tui: crate::platform::console_can_run(),
            gui: cfg!(feature = "native-gui") && window,
            web_gui: cfg!(feature = "webview-gui") && window,
        }
    }

    /// What a Settings page offers (the user's "only provide which options are
    /// available"): the TUI always, since any start from a terminal can run it; a GUI
    /// only when it is built in and a window can open in this session.
    pub fn for_settings() -> Can {
        Can { tui: true, ..Can::at_start() }
    }
}

/// The interfaces a Settings page offers, in select order.
pub fn offered() -> Vec<Interface> {
    let can = Can::for_settings();
    Interface::ALL.into_iter().filter(|i| can.has(*i)).collect()
}

/// What a bare start of this build opens with no preference saved: the GUI where a window
/// can be shown, else the console (main.rs's own default, which this mirrors).
pub fn clay_default(can: Can) -> Interface {
    if can.gui {
        Interface::Gui
    } else if can.web_gui {
        Interface::WebGui
    } else {
        Interface::Tui
    }
}

/// The interface a bare start opens: the preferred one when it can run here, else the
/// first of GUI, Web GUI and TUI that can, else Clay's own default (which then says why
/// it can't run).
pub fn pick_interface(preferred: Option<Interface>, can: Can) -> Interface {
    match preferred {
        Some(p) if can.has(p) => p,
        Some(_) => Interface::FALLBACK.into_iter().find(|i| can.has(*i)).unwrap_or_else(|| clay_default(can)),
        None => clay_default(can),
    }
}

/// What a select shows: the saved choice when it is offered, else what a bare start here
/// would open.
pub fn shown_preference(conf: &ClientConf) -> Interface {
    let offered = offered();
    match conf.preferred {
        Some(p) if offered.contains(&p) => p,
        _ => {
            let pick = pick_interface(conf.preferred, Can::for_settings());
            if offered.contains(&pick) { pick } else { offered.first().copied().unwrap_or(Interface::Tui) }
        }
    }
}

/// A Settings page's Save of the select: `chosen` is written only when it differs from
/// what was shown, so a page that offered less than the saved choice (Settings over SSH
/// offers no GUI) never overwrites it.
pub fn save_preference(conf: &mut ClientConf, shown: Interface, chosen: Interface) -> bool {
    if chosen == shown {
        return false;
    }
    conf.preferred = Some(chosen);
    true
}

/// What a bare `clay` turns into: the interface, and the host to attach to (the default
/// server) or `None` for this machine's own Clay. `None` overall: nothing is saved that
/// changes the start (no preference, no default server), so main's own default decides.
/// `local_only`: the command line names a TinyFugue world or option, which a local Clay
/// runs.
pub fn resolve_start(conf: &ClientConf, can: Can, local_only: bool) -> Option<(Interface, Option<String>)> {
    let host = (!local_only).then(|| conf.default_server().map(str::to_string)).flatten();
    if conf.preferred.is_none() && host.is_none() {
        return None;
    }
    Some((pick_interface(conf.preferred, can), host))
}

// The flag a bare start resolved to an attach (`--gui=host:port`), for an attached
// relaunch: a start that came from the default server has no host on its command line.
static RESOLVED_ATTACH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Record that this start attached by the default server, as `flag=host`.
pub fn note_resolved_attach(flag: &str, host: &str) {
    let _ = RESOLVED_ATTACH.set(format!("{flag}={host}"));
}

/// This process's command line (without the program) for an attached relaunch: as it was
/// started, plus the attach a bare start resolved to, so `/reload`, `/reload --console`
/// and an update come back attached to the same server.
pub fn launch_args() -> Vec<String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(flag) = RESOLVED_ATTACH.get() {
        if !args.iter().any(|a| a.starts_with("--console") || a.starts_with("--gui") || a.starts_with("--web-gui")) {
            args.push(flag.clone());
        }
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Can = Can { tui: true, gui: true, web_gui: true };

    #[test]
    fn a_server_is_kept_under_host_and_port() {
        assert_eq!(server_key("Example.com"), "example.com:9000");
        assert_eq!(server_key("example.com:4000"), "example.com:4000");
        assert_eq!(server_key("wss://example.com:4000/clay"), "example.com:4000");
        assert_eq!(server_key("example.com/stealth"), "example.com:9000");
        assert_eq!(server_key("[::1]:4000"), "[::1]:4000");
        assert_eq!(server_key("[::1]"), "[::1]:9000");
        assert_eq!(server_key("::1"), "[::1]:9000");
        assert_eq!(server_key("  "), "");
    }

    #[test]
    fn the_server_page_names_an_address() {
        let page = |host: &str, port: &str| ServerPage { host: host.into(), port: port.into(), ..Default::default() };
        assert_eq!(page("", "9000").address(), Ok(None));
        assert_eq!(page("example.com", "").address(), Ok(Some("example.com:9000".into())));
        assert_eq!(page("example.com", "4000").address(), Ok(Some("example.com:4000".into())));
        assert_eq!(page("example.com:4000", "").address(), Ok(Some("example.com:4000".into())));
        assert_eq!(page("example.com:4000", "5000").address(), Ok(Some("example.com:5000".into())));
        assert_eq!(page("example.com/stealth", "4000").address(), Ok(Some("example.com:4000/stealth".into())));
        assert_eq!(page("::1", "").address(), Ok(Some("[::1]:9000".into())));
        assert!(page("example.com", "x").address().is_err());
        assert!(page("example.com", "70000").address().is_err());
        assert!(page("a host", "").address().is_err());
    }

    #[test]
    fn the_file_round_trips_with_its_secrets_encrypted() {
        let mut conf = ClientConf { preferred: Some(Interface::Gui), ..Default::default() };
        conf.set_server_page(&ServerPage {
            host: "Mud.Example".into(),
            port: "4000".into(),
            username: "alice".into(),
            password: "pw-secret".into(),
            auth_key: "key-secret".into(),
        }).unwrap();
        let text = conf.to_text();
        assert!(!text.contains("pw-secret") && !text.contains("key-secret"), "{text}");
        assert!(text.contains("password=ENC:") && text.contains("auth_key=ENC:"), "{text}");
        assert!(text.contains("[server:mud.example:4000]"), "{text}");
        let back = ClientConf::parse(&text);
        assert_eq!(back, conf);
        assert_eq!(back.default_server(), Some("Mud.Example:4000"));
        let login = back.login_for("mud.example:4000").unwrap();
        assert_eq!((login.username(), login.password(), login.auth_key()), (Some("alice"), Some("pw-secret"), Some("key-secret")));
        assert_eq!(back.login_for("mud.example"), None, "another port is another server");
        let page = back.server_page();
        assert_eq!((page.host.as_str(), page.port.as_str(), page.password.as_str()), ("Mud.Example", "4000", "pw-secret"));
    }

    #[test]
    fn a_blank_host_clears_the_default_and_keeps_the_logins() {
        let mut conf = ClientConf::default();
        conf.set_server_page(&ServerPage { host: "a.example".into(), password: "x".into(), ..Default::default() }).unwrap();
        conf.set_server_page(&ServerPage::default()).unwrap();
        assert_eq!(conf.default_server(), None);
        assert!(conf.login_for("a.example").is_some(), "kept for an attach by name");
        // A server saved with no login keeps none.
        conf.set_server_page(&ServerPage { host: "a.example".into(), ..Default::default() }).unwrap();
        assert!(conf.login_for("a.example").is_none());
        assert_eq!(conf.default_server(), Some("a.example:9000"));
    }

    #[test]
    fn a_secret_this_machine_cannot_decrypt_is_no_secret() {
        // Another machine's client.dat: valid base64 of a nonce and ciphertext this key
        // never made. Used as a literal it would be a wrong password at every start.
        let text = "[client]\ndefault_server=h.example:9000\n[server:h.example:9000]\nusername=bob\npassword=ENC:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\nauth_key=ENC:not-base64!\n";
        let conf = ClientConf::parse(text);
        let login = conf.login_for("h.example:9000").expect("the user name is kept");
        assert_eq!((login.username(), login.password(), login.auth_key()), (Some("bob"), None, None));
        // A plain value written by hand is still read (and encrypted at the next save).
        let conf = ClientConf::parse("[server:h.example:9000]\npassword=typed by hand\n");
        assert_eq!(conf.login_for("h.example").unwrap().password(), Some("typed by hand"));
    }

    #[test]
    fn debug_output_never_shows_a_secret() {
        let mut conf = ClientConf::default();
        conf.set_server_page(&ServerPage { host: "h.example".into(), username: "bob".into(), password: "pw-secret".into(), auth_key: "key-secret".into(), ..Default::default() }).unwrap();
        for shown in [format!("{conf:?}"), format!("{:?}", conf.server_page()), format!("{:?}", conf.login_for("h.example"))] {
            assert!(!shown.contains("pw-secret") && !shown.contains("key-secret"), "{shown}");
            assert!(shown.contains("<redacted>") && shown.contains("bob"), "{shown}");
        }
    }

    #[test]
    fn an_address_holds_only_address_characters() {
        for ok in ["mud.example", "mud.example:4000/stealth", "[fe80::1%eth0]:9000", "wss://mud.example:9000", "10.0.0.2"] {
            assert!(valid_address(ok), "{ok}");
        }
        for bad in ["", "a b", "a'b", "a\"b", "a\\b", "a<b", "a\u{1b}[2J", "a;b", "a\u{e9}"] {
            assert!(!valid_address(bad), "{bad:?}");
        }
        let page = |host: &str| ServerPage { host: host.into(), ..Default::default() };
        assert!(page("mud.example';alert(1)//").address().is_err());
        assert!(page("mud.example\\").address().is_err());
        // A file edited by hand: a bad address is dropped, not passed to a command line.
        let conf = ClientConf::parse("[client]\ndefault_server=h.example';x\n[server:h\"x:9000]\npassword=p\n");
        assert_eq!(conf.default_server(), None);
        assert!(conf.servers.is_empty());
        let mut conf = ClientConf::default();
        assert!(conf.set_server_page(&ServerPage { host: "h.example".into(), username: "bob\ndefault_server=evil".into(), ..Default::default() }).is_err());
        assert!(!ClientConf::parse("[server:h.example:9000]\nusername=a\u{7}b\npassword=p\n").login_for("h.example").unwrap().username.contains('\u{7}'));
    }

    #[test]
    fn a_page_without_the_secrets_keeps_them_for_the_same_server_only() {
        let mut conf = ClientConf::default();
        conf.set_server_page(&ServerPage { host: "a.example".into(), password: "pw-a".into(), auth_key: "key-a".into(), ..Default::default() }).unwrap();
        let edit = |host: &str| ServerPageEdit { host: host.into(), ..Default::default() };
        // Saved untouched: both stay.
        conf.set_server_page_edit(&edit("a.example")).unwrap();
        let login = conf.login_for("a.example").unwrap();
        assert_eq!((login.password(), login.auth_key()), (Some("pw-a"), Some("key-a")));
        // A typed password replaces it; the key it didn't type stays.
        conf.set_server_page_edit(&ServerPageEdit { password: Some("new".into()), ..edit("a.example") }).unwrap();
        let login = conf.login_for("a.example").unwrap();
        assert_eq!((login.password(), login.auth_key()), (Some("new"), Some("key-a")));
        // Another host never inherits it, and a.example's stays its own.
        conf.set_server_page_edit(&edit("b.example")).unwrap();
        assert_eq!(conf.default_server(), Some("b.example:9000"));
        assert!(conf.login_for("b.example").is_none());
        assert_eq!(conf.login_for("a.example").unwrap().password(), Some("new"));
        // Forget drops what is saved for the server named.
        conf.set_server_page_edit(&ServerPageEdit { forget: true, ..edit("a.example") }).unwrap();
        assert!(conf.login_for("a.example").is_none());
        // A blank host clears the default, as the full page does.
        conf.set_server_page_edit(&edit("")).unwrap();
        assert_eq!(conf.default_server(), None);
    }

    #[test]
    fn the_preferred_interface_falls_back_to_one_that_can_run() {
        assert_eq!(pick_interface(Some(Interface::Tui), ALL), Interface::Tui);
        assert_eq!(pick_interface(Some(Interface::WebGui), ALL), Interface::WebGui);
        // Over SSH: no window.
        let ssh = Can { tui: true, gui: false, web_gui: false };
        assert_eq!(pick_interface(Some(Interface::Gui), ssh), Interface::Tui);
        // The desktop launcher: no terminal.
        let launcher = Can { tui: false, gui: true, web_gui: false };
        assert_eq!(pick_interface(Some(Interface::Tui), launcher), Interface::Gui);
        // A Termux build: the WebView GUI and no native one.
        let termux = Can { tui: false, gui: false, web_gui: true };
        assert_eq!(pick_interface(Some(Interface::Gui), termux), Interface::WebGui);
        // Nothing can run: Clay's own default, which then says why.
        assert_eq!(pick_interface(Some(Interface::Gui), Can::default()), Interface::Tui);
        // No preference: the GUI where a window opens, else the console.
        assert_eq!(pick_interface(None, ALL), Interface::Gui);
        assert_eq!(pick_interface(None, ssh), Interface::Tui);
    }

    #[test]
    fn a_bare_start_resolves_to_the_preference_and_the_default_server() {
        let mut conf = ClientConf::default();
        assert_eq!(resolve_start(&conf, ALL, false), None, "nothing saved: Clay's own default");
        conf.preferred = Some(Interface::Tui);
        assert_eq!(resolve_start(&conf, ALL, false), Some((Interface::Tui, None)));
        conf.default_server = "mud.example:9000".into();
        assert_eq!(resolve_start(&conf, ALL, false), Some((Interface::Tui, Some("mud.example:9000".into()))));
        // A TinyFugue world on the command line: this machine's own Clay.
        assert_eq!(resolve_start(&conf, ALL, true), Some((Interface::Tui, None)));
        conf.preferred = None;
        assert_eq!(resolve_start(&conf, ALL, false), Some((Interface::Gui, Some("mud.example:9000".into()))));
        assert_eq!(resolve_start(&conf, ALL, true), None);
    }

    #[test]
    fn a_save_writes_the_preference_only_when_it_was_changed() {
        let mut conf = ClientConf { preferred: Some(Interface::Gui), ..Default::default() };
        // Settings over SSH offered no GUI and showed the TUI; Save with it unchanged.
        assert!(!save_preference(&mut conf, Interface::Tui, Interface::Tui));
        assert_eq!(conf.preferred, Some(Interface::Gui));
        assert!(save_preference(&mut conf, Interface::Gui, Interface::Tui));
        assert_eq!(conf.preferred, Some(Interface::Tui));
    }

    #[cfg(any(not(target_os = "android"), feature = "webview-gui"))]
    #[test]
    fn a_relaunch_keeps_the_attach_a_bare_start_resolved_to() {
        note_resolved_attach("--gui", "mud.example:9000");
        let args = launch_args();
        assert_eq!(args.last().map(String::as_str), Some("--gui=mud.example:9000"));
        // `/reload --console` from it: the same server in the console.
        let switched = crate::platform::attached_switch_args(args, crate::platform::UiKind::Console);
        assert_eq!(switched.last().map(String::as_str), Some("--console=mud.example:9000"));
    }

    #[test]
    fn the_help_and_the_browsers_line_are_app_js_own() {
        let js = crate::http::WEB_APP_JS;
        assert!(js.contains(&format!("const SERVER_PAGE_ELSEWHERE = \"{SERVER_PAGE_ELSEWHERE}\";")));
        let quoted: Vec<String> = SERVER_PAGE_HELP.iter().map(|l| format!("'{}'", l.replace('\'', "\\'"))).collect();
        let entry = format!("        server: [\n            {}\n        ],", quoted.join(",\n            "));
        assert!(js.contains(&entry), "app.js popupHelpTexts.server is not clientconf::SERVER_PAGE_HELP");
    }

    #[test]
    fn names_and_labels_read_back() {
        for i in Interface::ALL {
            assert_eq!(Interface::from_name(i.name()), Some(i));
            assert_eq!(Interface::from_name(i.label()), Some(i));
        }
        assert_eq!(Interface::from_name("Console"), Some(Interface::Tui));
        assert_eq!(Interface::from_name("x"), None);
    }

    #[test]
    fn the_file_is_written_owner_only_and_read_back() {
        let _lock = FILE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let conf = ClientConf {
            preferred: Some(Interface::WebGui),
            default_server: "h.example:9000".into(),
            servers: [("h.example:9000".to_string(), ServerLogin { password: "p".into(), ..Default::default() })].into(),
        };
        conf.save().unwrap();
        assert_eq!(ClientConf::load(), conf);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_file(path());
    }
}
