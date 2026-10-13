//! `/update`, as one command that updates what can be updated.
//!
//! Typed in the console (or in a GUI running Clay in-process) it updates that Clay and
//! hot-reloads it. Typed in a remote client - the native GUI, the WebView GUI, the SSH
//! console, a browser page - it first asks the server to update itself, and once the
//! server has answered (and come back, if it reloaded), updates the client's own
//! binary and restarts it attached to the same server. Two programs can be updating in
//! one go, so every line says which one it is about: `Server: ...` for the Clay being
//! attached to, `This GUI: ...`/`This console: ...` for the program the user typed in.
//!
//! The two steps run one after the other, never at the same time: the client's own
//! download starts only once the server's attempt has finished. That matters when both
//! are the same file - `clay --gui=127.0.0.1:9000` started from the binary the server
//! runs - where two concurrent installs would write the same staging file. For that
//! case [`exe_replaced`] notices that the server's install already put a new binary at
//! this program's path, so the client just restarts with it instead of downloading
//! it again.
//!
//! The server answers a client's request with [`WsMessage::UpdateOutcome`]
//! (`src/websocket.rs`); an older server, which bounces `/update` back as
//! `ExecuteLocalCommand`, is answered by the client moving on to its own step.

#[cfg(not(target_os = "android"))]
use std::path::Path;
#[cfg(not(target_os = "android"))]
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How long a client waits for the server's answer before updating itself anyway.
pub const SERVER_ANSWER_TIMEOUT: Duration = Duration::from_secs(90);

/// The label of the Clay a client is attached to (and of the console's own Clay).
pub const SERVER: &str = "Server";

/// A client's `/update`, step by step (the native GUI's core and the SSH console keep
/// one of these; app.js keeps the same thing as `updateFlow`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientFlow {
    /// `/update` went to the server; waiting for its `UpdateOutcome`.
    WaitingServer { force: bool, since: Instant },
    /// The server updated and is reloading; waiting to be connected to it again.
    WaitingReconnect { force: bool },
}

impl ClientFlow {
    pub fn force(&self) -> bool {
        match self {
            ClientFlow::WaitingServer { force, .. } | ClientFlow::WaitingReconnect { force } => *force,
        }
    }
}

/// `{label} (Clay v1.7.4, linux/x86_64): checking for updates...`
pub fn checking(label: &str, force: bool) -> String {
    let what = if force { "fetching the latest release (forced)" } else { "checking for updates" };
    format!("{label} (Clay v{}, {}/{}): {what}...", crate::VERSION, crate::platform_tag(), std::env::consts::ARCH)
}

/// The server's attempt is over: what it says, by outcome.
pub fn server_result(result: &Result<String, String>) -> String {
    match result {
        Ok(version) => format!("{SERVER}: updated to Clay v{version} \u{2014} hot reloading, connections kept."),
        Err(e) => format!("{SERVER}: {e}"),
    }
}

/// A client's own attempt is over. `attached_to` names the server it restarts against.
pub fn client_result(label: &str, result: &Result<String, String>, attached_to: &str) -> String {
    match result {
        Ok(version) => format!("{label}: updated to Clay v{version} \u{2014} restarting, still attached to {attached_to}."),
        Err(e) => format!("{label}: {e}"),
    }
}

/// The server's install already replaced this program's own binary (same machine, same
/// file): nothing to download.
pub fn replaced_on_disk(label: &str, attached_to: &str) -> String {
    format!("{label}: its binary was replaced by the server's update \u{2014} restarting, still attached to {attached_to}.")
}

/// An older server bounced `/update` back instead of answering it.
pub fn server_too_old() -> String {
    format!("{SERVER}: this Clay is too old to be updated from a client (before v1.7.5); update it where it runs.")
}

/// The server never answered.
pub fn server_no_answer() -> String {
    format!("{SERVER}: no answer to the update request; going on with this program's own update.")
}

/// The multiuser server is not updated by its users.
pub fn multiuser_refused() -> String {
    format!("{SERVER}: a multiuser server isn't updated from a client; its operator updates it.")
}

/// Self-update isn't built for this platform (Android, the browser).
pub fn unavailable(label: &str) -> String {
    format!("{label}: update is not available on this platform.")
}

/// The target a relaunched client attaches to, from this run's own arguments
/// (`--gui=host`, `--web-gui=host`, `--console=host`), for the "still attached to"
/// part of a message.
pub fn attached_target_from_args() -> Option<String> {
    attached_target(std::env::args())
}

pub(crate) fn attached_target(args: impl IntoIterator<Item = String>) -> Option<String> {
    for a in args {
        for flag in ["--gui=", "--web-gui=", "--console=", "--webgui="] {
            if let Some(t) = a.strip_prefix(flag) {
                if !t.is_empty() {
                    return Some(t.to_string());
                }
            }
        }
    }
    None
}

/// What this program's binary looked like when it started: enough to tell, later, that
/// the file at that path is a different one.
#[cfg(not(target_os = "android"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExeStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    dev: u64,
}

#[cfg(not(target_os = "android"))]
impl ExeStamp {
    pub fn of(path: &Path) -> Option<Self> {
        let md = std::fs::metadata(path).ok()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(ExeStamp {
            len: md.len(),
            modified: md.modified().ok(),
            #[cfg(unix)]
            ino: md.ino(),
            #[cfg(unix)]
            dev: md.dev(),
        })
    }
}

#[cfg(not(target_os = "android"))]
static STARTED_AS: OnceLock<Option<ExeStamp>> = OnceLock::new();

/// Record what this program's binary looks like now. Called once, early in `main`;
/// [`exe_replaced`] compares against it.
#[cfg(not(target_os = "android"))]
pub fn stamp_exe() {
    let stamp = crate::platform::get_executable_path().ok().and_then(|(p, _)| ExeStamp::of(&p));
    let _ = STARTED_AS.set(stamp);
}

/// Is the file at this program's own path no longer the one it started from? True after
/// the server (same machine, same file) installed an update over it; false when nothing
/// changed, when the file is gone, or when the start-up stamp was never taken.
#[cfg(not(target_os = "android"))]
pub fn exe_replaced() -> bool {
    let Some(Some(started)) = STARTED_AS.get() else { return false };
    let Ok((path, _)) = crate::platform::get_executable_path() else { return false };
    match ExeStamp::of(&path) {
        Some(now) => now != *started,
        None => false,
    }
}

/// Android (Termux) has no self-install: a server there never writes over its binary
/// (`finish_server_update` leaves the download for the user to install), so nothing can
/// have replaced this one, and there is no stamp to take - `platform` has no
/// `get_executable_path` for it either.
#[cfg(target_os = "android")]
pub fn stamp_exe() {}

#[cfg(target_os = "android")]
pub fn exe_replaced() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attached_target_is_read_from_any_client_flag() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(attached_target(args(&["clay", "--gui=mini:9000"])), Some("mini:9000".into()));
        assert_eq!(attached_target(args(&["clay", "--conf=x", "--web-gui=10.0.0.2"])), Some("10.0.0.2".into()));
        assert_eq!(attached_target(args(&["clay", "--console=host:9000", "--ssh"])), Some("host:9000".into()));
        assert_eq!(attached_target(args(&["clay", "--gui"])), None);
        assert_eq!(attached_target(args(&["clay"])), None);
    }

    #[test]
    fn a_stamp_changes_when_the_file_is_replaced_and_not_otherwise() {
        let dir = std::env::temp_dir().join(format!("clay-exe-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("clay");
        std::fs::write(&exe, b"old binary").unwrap();
        let started = ExeStamp::of(&exe).unwrap();
        assert_eq!(ExeStamp::of(&exe).unwrap(), started, "nothing changed");
        // The install path: a new file renamed over the old one (new inode, new length).
        let staged = dir.join(".clay-update-1.7.5-1");
        std::fs::write(&staged, b"new binary, longer").unwrap();
        std::fs::rename(&staged, &exe).unwrap();
        assert_ne!(ExeStamp::of(&exe).unwrap(), started, "replaced");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_line_names_its_program() {
        assert!(checking("This GUI", false).starts_with("This GUI (Clay v"));
        assert!(checking("Server", true).contains("forced"));
        assert_eq!(
            server_result(&Ok("1.7.5".into())),
            "Server: updated to Clay v1.7.5 \u{2014} hot reloading, connections kept."
        );
        assert_eq!(server_result(&Err("Already up to date (current: v1.7.5, latest: v1.7.5)".into())),
            "Server: Already up to date (current: v1.7.5, latest: v1.7.5)");
        assert_eq!(
            client_result("This console", &Ok("1.7.5".into()), "mini:9000"),
            "This console: updated to Clay v1.7.5 \u{2014} restarting, still attached to mini:9000."
        );
        assert!(replaced_on_disk("This GUI", "mini:9000").starts_with("This GUI: its binary was replaced"));
        assert!(server_too_old().starts_with("Server: "));
        assert!(multiuser_refused().starts_with("Server: "));
        assert!(unavailable("This app").starts_with("This app: "));
    }
}
