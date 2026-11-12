//! clay_web's one settings file, `web.dat`: a flat `key=value` file whose keys are exactly
//! those of `src/web.dat.sample` ([`KEYS`]) - the web port, a certificate pair, the one game
//! and a few telnet options - and nothing else (user request 2026-11-11: "really simple for
//! the end user"; a key not in the sample must not be accepted). An unknown key, a
//! `[section]` header, a bad value or a missing `hostname`/`port` stops clay_web at start,
//! naming the line. Everything else a visitor's Clay could be configured with is Clay's
//! default or a fixed value ([`WebDat::apply`], pinned by `web_front_settings_are_pinned`):
//! the Dark - New theme (a visitor switches to Light - New on the page itself), the status
//! panel off, Import Picture off, no triggers, no custom themes; GMCP is refused through
//! `instance::Policy::gmcp`.
//!
//! No certificate means plain http, open to all (user ruling 2026-11-11, the one exception
//! to "the web server is always TLS-capable for remote clients"); clay_web never makes a
//! self-signed certificate. The grammar used to be settings.dat's (`[global]`, `[world:]`,
//! `[action:]`, `[theme:]`, ~45 keys) and is deliberately not any more.

use std::path::Path;

use crate::encoding::Encoding;
use crate::telnet::KeepAliveType;
use crate::{App, Settings, World, WorldSettings, WorldType};

/// The web.dat written where none is found: every key, each explained.
pub const SAMPLE: &str = include_str!("web.dat.sample");

/// Every key web.dat may hold, in the sample's order. `keys_match_the_sample` pins the two.
pub const KEYS: &[&str] = &[
    "http_port",
    "cert_file",
    "key_file",
    "world_name",
    "hostname",
    "port",
    "ssl",
    "encoding",
    "first_command",
    "world_type",
    "prompt_wait_ms",
    "keep_alive_type",
    "keep_alive_cmd",
    "keep_alive_gag_enabled",
    "keep_alive_gag",
    "msp_enabled",
    "mcp_enabled",
    "mccp2_enabled",
];

/// What web.dat said, validated. A key the file leaves out has the sample's value, except
/// `hostname` and `port`, which are required, and `world_name`, which is the hostname.
#[derive(Debug, Clone, PartialEq)]
pub struct WebDat {
    pub http_port: u16,
    /// Absolute (a relative path is resolved against web.dat's directory), or both empty:
    /// no certificate, plain http.
    pub cert_file: String,
    pub key_file: String,
    pub world_name: String,
    pub hostname: String,
    pub port: u16,
    pub ssl: bool,
    pub encoding: Encoding,
    pub first_command: String,
    pub world_type: WorldType,
    pub prompt_wait_ms: u64,
    pub keep_alive_type: KeepAliveType,
    pub keep_alive_cmd: String,
    pub keep_alive_gag_enabled: bool,
    pub keep_alive_gag: String,
    pub msp_enabled: bool,
    pub mcp_enabled: bool,
    pub mccp2_enabled: bool,
}

impl WebDat {
    /// Parse web.dat's text; `dir` is the file's directory, for relative certificate paths.
    /// The error is one line for the operator, `web.dat line N: ...`.
    pub fn parse(text: &str, dir: &Path) -> Result<WebDat, String> {
        let d = WorldSettings::default();
        let mut w = WebDat {
            http_port: 9000,
            cert_file: String::new(),
            key_file: String::new(),
            world_name: String::new(),
            hostname: String::new(),
            port: 0,
            ssl: false,
            encoding: Encoding::Utf8,
            first_command: String::new(),
            world_type: WorldType::Mud,
            prompt_wait_ms: d.prompt_wait_ms,
            keep_alive_type: d.keep_alive_type,
            keep_alive_cmd: String::new(),
            keep_alive_gag_enabled: false,
            keep_alive_gag: String::new(),
            msp_enabled: d.msp_enabled,
            mcp_enabled: d.mcp_enabled,
            mccp2_enabled: d.mccp2_enabled,
        };
        let mut seen: Vec<&str> = Vec::new();
        for (idx, raw) in text.lines().enumerate() {
            let n = idx + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                return Err(format!("web.dat line {n}: web.dat has no sections ({line}); it is key=value lines only, as in the sample"));
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("web.dat line {n}: not key=value: {line}"));
            };
            let (key, value) = (key.trim(), value.trim());
            let Some(&key) = KEYS.iter().find(|k| **k == key) else {
                return Err(format!("web.dat line {n}: unknown key {key}; the keys are {}", KEYS.join(", ")));
            };
            if seen.contains(&key) {
                return Err(format!("web.dat line {n}: {key} is given twice"));
            }
            seen.push(key);
            let bad = |what: &str| format!("web.dat line {n}: {key}={value} - {what}");
            match key {
                "http_port" => w.http_port = parse_port(value).ok_or_else(|| bad("a port is 1 to 65535"))?,
                "cert_file" => w.cert_file = cert_path(value, dir),
                "key_file" => w.key_file = cert_path(value, dir),
                "world_name" => w.world_name = value.to_string(),
                "hostname" => w.hostname = value.to_string(),
                "port" => w.port = parse_port(value).ok_or_else(|| bad("a port is 1 to 65535"))?,
                "ssl" => w.ssl = parse_bool(value).ok_or_else(|| bad("true or false"))?,
                "encoding" => {
                    w.encoding = match value.to_ascii_lowercase().as_str() {
                        "utf8" | "utf-8" => Encoding::Utf8,
                        "latin1" | "latin-1" => Encoding::Latin1,
                        "fansi" => Encoding::Fansi,
                        _ => return Err(bad("utf8, latin1 or fansi")),
                    }
                }
                "first_command" => w.first_command = crate::persistence::unescape_string(value),
                "world_type" => {
                    w.world_type = match value.to_ascii_lowercase().as_str() {
                        "mud" => WorldType::Mud,
                        "mud_timed_prompt" => WorldType::MudTimedPrompt,
                        _ => return Err(bad("mud or mud_timed_prompt")),
                    }
                }
                "prompt_wait_ms" => w.prompt_wait_ms = value.parse().ok().filter(|ms| *ms > 0).ok_or_else(|| bad("milliseconds, more than 0"))?,
                "keep_alive_type" => {
                    if !matches!(value.to_ascii_lowercase().as_str(), "nop" | "none" | "generic" | "custom") {
                        return Err(bad("NOP, None, Generic or Custom"));
                    }
                    w.keep_alive_type = KeepAliveType::from_name(value);
                }
                "keep_alive_cmd" => w.keep_alive_cmd = value.to_string(),
                "keep_alive_gag_enabled" => w.keep_alive_gag_enabled = parse_bool(value).ok_or_else(|| bad("true or false"))?,
                "keep_alive_gag" => w.keep_alive_gag = value.to_string(),
                "msp_enabled" => w.msp_enabled = parse_bool(value).ok_or_else(|| bad("true or false"))?,
                "mcp_enabled" => w.mcp_enabled = parse_bool(value).ok_or_else(|| bad("true or false"))?,
                "mccp2_enabled" => w.mccp2_enabled = parse_bool(value).ok_or_else(|| bad("true or false"))?,
                _ => unreachable!("every KEYS entry has an arm"),
            }
        }
        if w.hostname.is_empty() {
            return Err("web.dat: hostname= is missing (the game's address)".to_string());
        }
        if w.port == 0 {
            return Err("web.dat: port= is missing (the game's port)".to_string());
        }
        if w.cert_file.is_empty() != w.key_file.is_empty() {
            return Err("web.dat: cert_file and key_file go together - set both for https, or neither for plain http".to_string());
        }
        if w.world_name.is_empty() {
            w.world_name = w.hostname.clone();
        }
        Ok(w)
    }

    /// Plain http (no certificate named) or https.
    pub fn has_certificate(&self) -> bool {
        !self.cert_file.is_empty()
    }

    /// Make `app` the operator Clay every visitor's Clay is copied from: Clay's default
    /// settings with web.dat's values and clay_web's fixed choices (the only `Settings`
    /// clay_web ever sets), the built-in themes alone, no triggers, and the one world.
    pub fn apply(&self, app: &mut App) {
        app.settings = Settings {
            http_port: self.http_port,
            web_path: "clay".to_string(),
            websocket_cert_file: self.cert_file.clone(),
            websocket_key_file: self.key_file.clone(),
            // Fixed for every visitor (user rulings 2026-11-11); the rest is Clay's default.
            gui_theme: "dark-new".to_string(),
            status_panel: false,
            picture_host: crate::picture::PictureService::OFF,
            ..Settings::default()
        };
        app.theme_file = crate::theme::ThemeFile::with_defaults();

        let mut world = World::new(&self.world_name);
        let ws = &mut world.settings;
        ws.hostname = self.hostname.clone();
        ws.port = self.port.to_string();
        ws.use_ssl = self.ssl;
        ws.encoding = self.encoding;
        ws.first_command = self.first_command.clone();
        ws.world_type = self.world_type.clone();
        ws.prompt_wait_ms = self.prompt_wait_ms;
        ws.keep_alive_type = self.keep_alive_type;
        ws.keep_alive_cmd = self.keep_alive_cmd.clone();
        ws.keep_alive_gag_enabled = self.keep_alive_gag_enabled;
        ws.keep_alive_gag = self.keep_alive_gag.clone();
        ws.msp_enabled = self.msp_enabled;
        ws.mcp_enabled = self.mcp_enabled;
        ws.mccp2_enabled = self.mccp2_enabled;
        ws.status_panel = false;
        app.worlds.retain(|w| w.is_activity);
        app.worlds.insert(0, world);
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_port(value: &str) -> Option<u16> {
    value.parse::<u16>().ok().filter(|p| *p > 0)
}

/// A certificate path as web.dat gives it: empty stays empty, a relative path is from the
/// file's directory (clay_web may be started from elsewhere with `--conf=`).
fn cert_path(value: &str, dir: &Path) -> String {
    if value.is_empty() || Path::new(value).is_absolute() {
        value.to_string()
    } else {
        dir.join(value).display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> &'static Path {
        Path::new("/srv/clay")
    }

    /// The sample is the reference: every key known, every value read as written.
    #[test]
    fn the_sample_parses_and_every_value_lands() {
        let w = WebDat::parse(SAMPLE, dir()).expect("the sample parses");
        assert_eq!(w.http_port, 9000);
        assert!(!w.has_certificate());
        assert_eq!(w.world_name, "MyMUD");
        assert_eq!(w.hostname, "mud.example.org");
        assert_eq!(w.port, 4000);
        assert!(!w.ssl);
        assert_eq!(w.encoding, Encoding::Utf8);
        assert_eq!(w.first_command, "REMOTE_HOST %host%");
        assert_eq!(w.world_type, WorldType::Mud);
        assert_eq!(w.prompt_wait_ms, 1000);
        assert_eq!(w.keep_alive_type, KeepAliveType::Nop);
        assert_eq!(w.keep_alive_cmd, "");
        assert!(!w.keep_alive_gag_enabled);
        assert_eq!(w.keep_alive_gag, "");
        assert!(w.msp_enabled && w.mcp_enabled && w.mccp2_enabled);
        // The sample's values are the defaults: a file with only the two required keys is
        // the same configuration.
        let minimal = WebDat::parse("hostname=mud.example.org\nport=4000\nworld_name=MyMUD\nfirst_command=REMOTE_HOST %host%\n", dir()).unwrap();
        assert_eq!(minimal, w);
    }

    /// `KEYS` and the sample name the same keys, once each, in the same order.
    #[test]
    fn keys_match_the_sample() {
        let in_sample: Vec<&str> = SAMPLE
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.split_once('=').unwrap_or_else(|| panic!("not key=value: {l}")).0.trim())
            .collect();
        assert_eq!(in_sample, KEYS);
        let mut sorted = KEYS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), KEYS.len(), "a key twice in KEYS");
        for line in SAMPLE.lines() {
            assert!(!line.contains(" #") || line.trim_start().starts_with('#'), "inline comment in {line}");
        }
    }

    /// What every visitor's Clay gets, whatever Clay's defaults become: the user's list of
    /// 2026-11-11, each pinned here so a changed default is noticed.
    #[test]
    fn web_front_settings_are_pinned() {
        let w = WebDat::parse(SAMPLE, dir()).unwrap();
        let mut app = App::new();
        w.apply(&mut app);
        let s = &app.settings;
        assert_eq!(s.http_port, 9000);
        assert_eq!(s.web_path, "clay");
        assert_eq!(s.websocket_cert_file, "");
        assert_eq!(s.websocket_key_file, "");
        assert_eq!(s.websocket_allow_list, "", "everyone may connect");
        assert_eq!(s.web_front_max_sessions, crate::instance::DEFAULT_MAX_VISITORS);
        assert_eq!(s.gui_theme, "dark-new");
        assert!(s.more_mode_enabled);
        assert_eq!(s.wrapspace, 0);
        assert!(s.spell_check_enabled);
        assert!(s.ansi_music_enabled, "the browser plays it; the server never does for a visitor");
        assert!(!s.temp_convert_enabled);
        assert!(!s.status_panel);
        assert_eq!((s.input_height_phone, s.input_height_tablet, s.input_height_desktop), (3, 3, 3));
        assert!(!s.clear_input_on_send);
        assert_eq!(s.text_reveal, crate::TextReveal::Off);
        assert!(s.text_reveal_profiles.is_empty(), "every browser key falls back to Off");
        assert_eq!(s.new_indicator, crate::encoding::NewIndicator::None);
        assert_eq!(s.picture_host, crate::picture::PictureService::OFF);
        assert!(s.actions.is_empty(), "no triggers");
        assert!(!s.debug_enabled);
        assert_eq!(app.theme_file.to_json_all(), crate::theme::ThemeFile::with_defaults().to_json_all(), "no custom themes");
        let worlds: Vec<&World> = app.worlds.iter().filter(|w| !w.is_activity).collect();
        assert_eq!(worlds.len(), 1);
        let ws = &worlds[0].settings;
        assert_eq!(worlds[0].name, "MyMUD");
        assert_eq!((ws.hostname.as_str(), ws.port.as_str()), ("mud.example.org", "4000"));
        assert!(!ws.use_ssl);
        assert_eq!(ws.encoding, Encoding::Utf8);
        assert_eq!(ws.first_command, "REMOTE_HOST %host%");
        assert!(ws.world_type.is_mud());
        assert!(ws.has_connection_settings());
        assert!(!ws.status_panel);
        assert_eq!(ws.keep_alive_type, KeepAliveType::Nop);
        assert!(ws.msp_enabled && ws.mcp_enabled && ws.mccp2_enabled);
        assert_eq!(ws.prompt_wait_ms, 1000);
        assert!(!ws.log_enabled);
    }

    #[test]
    fn values_are_read_leniently_where_it_costs_nothing() {
        let text = "  hostname = mud.example.org \nport=4000\nssl=TRUE\nencoding=Latin-1\nworld_type=MUD_TIMED_PROMPT\n\
                    keep_alive_type=custom\nkeep_alive_cmd=look\nfirst_command=a\\nb\ncert_file=certs/fullchain.pem\nkey_file=/etc/key.pem\n";
        let w = WebDat::parse(text, dir()).unwrap();
        assert_eq!(w.hostname, "mud.example.org");
        assert!(w.ssl);
        assert_eq!(w.encoding, Encoding::Latin1);
        assert_eq!(w.world_type, WorldType::MudTimedPrompt);
        assert_eq!(w.keep_alive_type, KeepAliveType::Custom);
        assert_eq!(w.first_command, "a\nb", "\\n separates lines, as in settings.dat");
        assert_eq!(w.cert_file, "/srv/clay/certs/fullchain.pem", "relative to web.dat's directory");
        assert_eq!(w.key_file, "/etc/key.pem");
        assert!(w.has_certificate());
        assert_eq!(w.world_name, "mud.example.org", "world_name defaults to the hostname");
    }

    #[test]
    fn every_mistake_names_its_line() {
        let base = "hostname=h\nport=4000\n";
        let err = |text: &str| WebDat::parse(text, dir()).unwrap_err();
        let e = err(&format!("{base}gui_theme=dark\n"));
        assert!(e.starts_with("web.dat line 3: unknown key gui_theme"), "{e}");
        assert!(e.contains("first_command"), "the error lists the keys: {e}");
        let e = err(&format!("{base}[world:MyMUD]\n"));
        assert!(e.starts_with("web.dat line 3:") && e.contains("no sections"), "{e}");
        let e = err(&format!("{base}# fine\n\nport=5000\n"));
        assert!(e.starts_with("web.dat line 5: port is given twice"), "{e}");
        let e = err(&format!("{base}just words\n"));
        assert!(e.starts_with("web.dat line 3: not key=value"), "{e}");
        let e = err(&format!("{base}ssl=yes\n"));
        assert!(e.starts_with("web.dat line 3: ssl=yes - true or false"), "{e}");
        let e = err(&format!("{base}encoding=ascii\n"));
        assert!(e.contains("utf8, latin1 or fansi"), "{e}");
        let e = err(&format!("{base}world_type=slack\n"));
        assert!(e.contains("mud or mud_timed_prompt"), "{e}");
        let e = err(&format!("{base}keep_alive_type=often\n"));
        assert!(e.contains("NOP, None, Generic or Custom"), "{e}");
        let e = err(&format!("{base}http_port=70000\n"));
        assert!(e.contains("a port is 1 to 65535"), "{e}");
        let e = err(&format!("{base}prompt_wait_ms=0\n"));
        assert!(e.contains("more than 0"), "{e}");
        let e = err(&format!("{base}cert_file=/etc/cert.pem\n"));
        assert!(e.contains("cert_file and key_file go together"), "{e}");
        let e = err("port=4000\n");
        assert!(e.contains("hostname= is missing"), "{e}");
        let e = err("hostname=h\n");
        assert!(e.contains("port= is missing"), "{e}");
        let e = err("hostname=h\nport=0\n");
        assert!(e.contains("a port is 1 to 65535"), "{e}");
    }
}
