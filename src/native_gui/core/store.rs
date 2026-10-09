//! What the native GUI remembers between runs, in `~/.clay/native-gui.dat`.
//!
//! - This client's `client_uid`: stable across reconnects *and* restarts, so the ▶
//!   markers the server records for it (`OutputLine::display_id`) are still its own the
//!   next time it starts.
//! - Per server: the auth key the server handed it (encrypted with the machine key, like
//!   the passwords in settings.dat) and the server's web path.
//!
//! - Per server: the world this client last showed.
//! - The window's own preferences, what the web client keeps in `localStorage` (the
//!   stats panel collapsed or not).
//!
//! INI, written owner-only (`util::write_secret_file`): it holds a credential.

use std::collections::BTreeMap;
use std::path::PathBuf;

const FILE: &str = "native-gui.dat";

fn path() -> PathBuf {
    crate::clay_config_path(FILE)
}

/// Section name -> key -> value.
type Ini = BTreeMap<String, BTreeMap<String, String>>;

fn load() -> Ini {
    let mut ini = Ini::new();
    let Ok(text) = std::fs::read_to_string(path()) else { return ini };
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            ini.entry(section.clone()).or_default().insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    ini
}

fn save(ini: &Ini) {
    let mut text = String::new();
    for (section, keys) in ini {
        text.push_str(&format!("[{section}]\n"));
        for (k, v) in keys {
            text.push_str(&format!("{k}={v}\n"));
        }
        text.push('\n');
    }
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = crate::util::write_secret_file(&p, text.as_bytes()) {
        crate::debug_log(true, &format!("native-gui: can't save {}: {e}", p.display()));
    }
}

fn section(server: &str) -> String {
    format!("server:{}", server.to_lowercase())
}

/// This client's stable id, created on first use.
pub fn client_uid() -> String {
    let mut ini = load();
    if let Some(uid) = ini.get("client").and_then(|c| c.get("uid")).filter(|u| !u.is_empty()) {
        return uid.clone();
    }
    let mut bytes = [0u8; 12];
    let _ = getrandom::getrandom(&mut bytes);
    let uid = format!("n-{}", hex::encode(bytes));
    ini.entry("client".into()).or_default().insert("uid".into(), uid.clone());
    save(&ini);
    uid
}

/// What's remembered about one server (`host:port`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    pub auth_key: Option<String>,
    pub web_path: Option<String>,
    /// The world this client last showed on that server, to come back to.
    pub last_world: Option<String>,
}

pub fn server(name: &str) -> Server {
    let ini = load();
    let Some(s) = ini.get(&section(name)) else { return Server::default() };
    Server {
        auth_key: s.get("auth_key").map(|k| crate::persistence::decrypt_password(k)).filter(|k| !k.is_empty()),
        web_path: s.get("web_path").cloned(),
        last_world: s.get("last_world").cloned().filter(|w| !w.is_empty()),
    }
}

pub fn set_auth_key(name: &str, key: &str) {
    let mut ini = load();
    ini.entry(section(name)).or_default().insert("auth_key".into(), crate::persistence::encrypt_password(key));
    save(&ini);
}

pub fn set_web_path(name: &str, web_path: &str) {
    let mut ini = load();
    let s = ini.entry(section(name)).or_default();
    if s.get("web_path").map(String::as_str) != Some(web_path) {
        s.insert("web_path".into(), web_path.to_string());
        save(&ini);
    }
}

pub fn set_last_world(name: &str, world: &str) {
    let mut ini = load();
    let s = ini.entry(section(name)).or_default();
    if s.get("last_world").map(String::as_str) != Some(world) {
        s.insert("last_world".into(), world.to_string());
        save(&ini);
    }
}

/// One of the window's own preferences.
pub fn pref(key: &str) -> Option<String> {
    load().get("ui").and_then(|s| s.get(key)).cloned()
}

pub fn set_pref(key: &str, value: &str) {
    let mut ini = load();
    let s = ini.entry("ui".into()).or_default();
    if s.get(key).map(String::as_str) != Some(value) {
        s.insert(key.into(), value.into());
        save(&ini);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_a_stable_uid_and_per_server_keys_encrypted() {
        let uid = client_uid();
        assert!(uid.starts_with("n-"));
        assert_eq!(client_uid(), uid, "the same id every time");
        set_auth_key("Host.Example:9000", "secret-key");
        set_web_path("host.example:9000", "stealth");
        let s = server("HOST.example:9000");
        assert_eq!(s.auth_key.as_deref(), Some("secret-key"));
        assert_eq!(s.web_path.as_deref(), Some("stealth"));
        let on_disk = std::fs::read_to_string(path()).unwrap();
        assert!(!on_disk.contains("secret-key"), "the key is stored encrypted");
        assert_eq!(server("other:9000"), Server::default());
        // (In this one test: the file is shared, and tests run in parallel.)
        assert_eq!(pref("stats_collapsed"), None);
        set_pref("stats_collapsed", "1");
        assert_eq!(pref("stats_collapsed").as_deref(), Some("1"));
        assert_eq!(server("HOST.example:9000").auth_key.as_deref(), Some("secret-key"));
        set_last_world("host.example:9000", "Midgard");
        assert_eq!(server("host.example:9000").last_world.as_deref(), Some("Midgard"));
    }
}
