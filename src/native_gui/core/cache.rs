//! The world cache: each world's newest lines kept on disk per server, so a cold start
//! shows them at once and asks only for what came after (app.js's IndexedDB cache,
//! `scheduleWorldCacheSave` / the InitialState hydration).
//!
//! - Capped at `remote_initial_lines` lines a world (at least 10), so it never holds more
//!   than a fresh connect would download.
//! - Stamped with the world's `seq_epoch` and its delivered-seq ranges. At the first
//!   InitialState a cached world is offered as the "prior" buffer, and the same checks as an
//!   in-memory reconnect decide whether it stands: a different epoch, or a corrupted buffer,
//!   and it's dropped. A world without an epoch (multiuser) is never cached.
//! - Never sent as a resume list: world indices in a cache can be stale, and the server's
//!   resume replay trusts them. A cached world catches up by gap fill instead, as app.js's
//!   does.
//!
//! One file per server in `~/.clay/native-gui-cache/`, owner-only (it holds MUD text).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Bumped when the file's shape changes (an older file is then ignored).
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedLine {
    pub text: String,
    pub ts: u64,
    pub seq: Option<u64>,
    pub from_server: bool,
    pub gagged: bool,
    pub highlight: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedWorld {
    pub name: String,
    pub epoch: u64,
    pub lines: Vec<CachedLine>,
    /// Every seq delivered, as inclusive ranges (more than `lines` holds: a hole known
    /// before the cap stays known).
    pub seen: Vec<(u64, u64)>,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    worlds: Vec<CachedWorld>,
}

fn path(server: &str) -> PathBuf {
    let name: String = server.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    crate::clay_config_path("native-gui-cache").join(format!("{name}.json"))
}

/// What's cached for a server (nothing, if the file is missing, old or unreadable).
pub fn load(server: &str) -> Vec<CachedWorld> {
    let Ok(text) = std::fs::read_to_string(path(server)) else { return Vec::new() };
    match serde_json::from_str::<File>(&text) {
        Ok(f) if f.version == VERSION => f.worlds,
        _ => Vec::new(),
    }
}

/// Write a server's cache: whole or not at all (a temporary file renamed over it), one
/// save at a time.
pub fn save(server: &str, worlds: &[CachedWorld]) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = path(server);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = File { version: VERSION, worlds: worlds.to_vec() };
    let bytes = match serde_json::to_vec(&file) {
        Ok(b) => b,
        Err(e) => return crate::debug_log(true, &format!("native-gui: can't encode the world cache: {e}")),
    };
    let tmp = p.with_extension("json.tmp");
    if let Err(e) = crate::util::write_secret_file(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, &p)) {
        crate::debug_log(true, &format!("native-gui: can't save {}: {e}", p.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cache_round_trips_per_server() {
        let w = CachedWorld {
            name: "Mud".into(),
            epoch: 42,
            lines: vec![CachedLine { text: "hi".into(), ts: 5, seq: Some(7), from_server: true, gagged: false, highlight: None }],
            seen: vec![(0, 7)],
        };
        save("Cache.Test:9000", std::slice::from_ref(&w));
        assert_eq!(load("cache.test:9000"), vec![w]);
        assert!(load("other:9000").is_empty());
    }
}
