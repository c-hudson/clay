//! The two server builds: `clay_multi` (feature `multiuser`) and `clay_web` (feature
//! `web-front`). One process, one HTTPS/WebSocket front (http.rs, the `SecurityGate`, the
//! bans), many Clays: each logged-in user, or each visitor's page, is an `instance` - a
//! thread running a full `App` through `apploop`. The front authenticates a WebSocket
//! connection (`handle_ws_client`, `Backend::Multi`) and the `InstanceManager` attaches it
//! to its Clay; from then on it is an ordinary client of an ordinary Clay.
//!
//! clay_multi reads the operator's own `~/.clay/settings.dat` for the web server's fields
//! (port, path, allow list, certificate). clay_web reads everything from one `web.dat` in
//! the current directory (or `--conf=`'s) and touches nothing under `~/.clay` (user
//! request 2026-11-10; `instance::set_web_only`, [`init_web_only`]): a missing web.dat is
//! written as a sample (`webdat::SAMPLE`) and clay_web ends for the operator to edit it;
//! web.dat is `webdat`'s own small, strict grammar (user request 2026-11-11), an SSL
//! world's pin and remote.log live beside it. clay_web serves https with the certificate
//! web.dat names and plain http, open to all, without one (user ruling 2026-11-11); it
//! never makes a self-signed certificate. Nothing hot-reloads: `SIGUSR1` flushes every
//! instance and execs this process afresh (the port kept), `SIGTERM`/`SIGINT`/`SIGHUP`
//! flush and exit (user rulings 2026-11-09).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::instance::{Credentials, InstanceManager, ManagerKind};
use crate::http::{Backend, KeySource, MultiLogin, SecurityGate, WsConnectionState};
use crate::webdat::WebDat;
use crate::{persistence, App};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerKind {
    Multi,
    WebFront,
}

impl ServerKind {
    fn title(self) -> &'static str {
        match self {
            ServerKind::Multi => "clay_multi",
            ServerKind::WebFront => "clay_web",
        }
    }
}

/// What a clay_multi without a password.dat is told.
pub(crate) fn password_dat_instructions(path: &std::path::Path) -> String {
    format!(
        "clay_multi: no accounts file at {}\n\
         Create it with one line per user, the username and the password separated by a space\n\
         (no spaces in a username; the password may have them), and make it readable by this\n\
         user only (chmod 600):\n\
         \n\
             alice correct horse battery staple\n\
             bob hunter2\n\
         \n\
         Clay only reads the file, at every login, so a user can be added without a restart.\n\
         Each user's own settings, worlds, scripts and scrollback live in {}/users/<name>/.",
        path.display(),
        crate::clay_root_config_dir().display()
    )
}

/// clay_web's one settings file, absolute (`init_web_only`).
static WEB_DAT: OnceLock<PathBuf> = OnceLock::new();

/// clay_web runs from one directory, web.dat's: `--conf=`'s file, else `./web.dat`. Called
/// from `main` before anything could touch `~/.clay` (`instance::set_web_only`).
pub fn init_web_only(conf: Option<&str>) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    let path = match conf {
        Some(c) => {
            let p = PathBuf::from(c);
            if p.is_absolute() { p } else { cwd.join(p) }
        }
        None => cwd.join("web.dat"),
    };
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => cwd,
    };
    if !dir.is_dir() {
        return Err(io::Error::other(format!("{} is not a directory", dir.display())));
    }
    crate::instance::set_web_only(dir);
    // `get_settings_path()` names it (the /info report); a --conf has set it already.
    crate::set_custom_config_path(path.clone());
    let _ = WEB_DAT.set(path);
    Ok(())
}

fn web_dat_path() -> PathBuf {
    WEB_DAT.get().cloned().unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("web.dat"))
}

/// web.dat's path as the operator sees it: relative to the current directory when it is
/// under it (`web.dat` for the usual start), the whole path otherwise (`--conf=`).
fn web_dat_display(path: &Path) -> String {
    web_dat_display_in(path, std::env::current_dir().ok().as_deref())
}

fn web_dat_display_in(path: &Path, cwd: Option<&Path>) -> String {
    cwd.and_then(|cwd| path.strip_prefix(cwd).ok().map(|p| p.display().to_string()))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

/// clay_web's settings: web.dat, read once and never written (`webdat`). Without one, the
/// sample is written there and clay_web ends - its world is a placeholder; a mistake in it
/// ends clay_web too, naming the line.
fn load_web_dat(operator: &mut App) -> WebDat {
    let path = web_dat_path();
    let dir = path.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            match std::fs::write(&path, crate::webdat::SAMPLE) {
                Ok(()) => eprintln!(
                    "clay_web: no web.dat in {}; a sample was written to {}.\n\
                     Set hostname and port to your game's (and the rest to taste), then start clay_web again.",
                    dir.display(),
                    path.display()
                ),
                Err(e) => eprintln!("clay_web: no web.dat at {}, and a sample could not be written there: {e}", path.display()),
            }
            crate::crashlog::exit_process(1, "no web.dat");
        }
        Err(e) => {
            eprintln!("clay_web: cannot read {}: {e}", path.display());
            crate::crashlog::exit_process(1, "web.dat unreadable");
        }
    };
    let web = match WebDat::parse(&content, &dir) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("clay_web: {e}\n(in {}; the sample written where no web.dat exists shows every key)", path.display());
            crate::crashlog::exit_process(1, "bad web.dat");
        }
    };
    web.apply(operator);
    web
}

/// Run the server until a signal ends it.
pub(crate) async fn run(kind: ServerKind) -> io::Result<()> {
    let title = kind.title();
    crate::crashlog::session(match kind { ServerKind::Multi => "multi", ServerKind::WebFront => "web" });

    // The operator's own settings: clay_multi's settings.dat for the web server's fields,
    // clay_web's web.dat for everything.
    let mut operator = App::new();
    let web = match kind {
        ServerKind::Multi => {
            if let Err(e) = persistence::load_settings(&mut operator) {
                eprintln!("Warning: Could not load settings: {}", e);
            }
            operator.apply_debug_setting();
            crate::load_theme_file(&mut operator);
            None
        }
        ServerKind::WebFront => Some(load_web_dat(&mut operator)),
    };
    let port = operator.settings.http_port;

    let manager_kind = match kind {
        ServerKind::Multi => {
            let password_path = crate::clay_root_config_path(Credentials::FILE);
            let creds = match Credentials::load(&password_path) {
                Ok(c) if !c.is_empty() => c,
                Ok(_) => {
                    eprintln!("clay_multi: {} has no accounts.", password_path.display());
                    eprintln!("{}", password_dat_instructions(&password_path));
                    crate::crashlog::exit_process(1, "no accounts");
                }
                Err(_) => {
                    eprintln!("{}", password_dat_instructions(&password_path));
                    crate::crashlog::exit_process(1, "no password.dat");
                }
            };
            println!("{title}: {} account(s) in {}", creds.len(), password_path.display());
            ManagerKind::Multi { password_path, users_dir: crate::clay_root_config_path("users") }
        }
        ServerKind::WebFront => {
            // `WebDat::apply` left exactly one MUD world with a host and port.
            let Some(world) = operator.worlds.iter().find(|w| !w.is_activity) else {
                eprintln!("clay_web: no world to serve ({}).", web_dat_path().display());
                crate::crashlog::exit_process(1, "no world");
            };
            let template = crate::apploop::InstanceTemplate {
                settings: operator.settings.visitor_template(),
                world_name: world.name.clone(),
                world: world.settings.visitor_copy(),
                theme: operator.theme_file.clone(),
            };
            ManagerKind::WebFront { template: Arc::new(template), max_visitors: operator.settings.web_front_max_sessions.max(1) }
        }
    };

    // clay_multi serves with the operator's certificate or Clay's own; clay_web only with the
    // one web.dat names (never a self-signed one), and plain http without one - every byte
    // visitors type then crosses the network in the clear (user ruling 2026-11-11).
    let tls: Option<(String, String)> = match &web {
        None => match crate::resolve_web_cert_files(&mut operator) {
            Some(pair) => Some(pair),
            None => {
                eprintln!("Error: no TLS certificate available; the server cannot start.");
                crate::crashlog::exit_process(1, "no certificate");
            }
        },
        Some(web) if web.has_certificate() => {
            if let Some(why) = crate::custom_cert_problem(&web.cert_file, &web.key_file) {
                eprintln!("clay_web: cert_file/key_file in web.dat can't serve: {why}");
                crate::crashlog::exit_process(1, "bad certificate");
            }
            Some((web.cert_file.clone(), web.key_file.clone()))
        }
        Some(_) => None,
    };

    let manager = Arc::new(InstanceManager::new(manager_kind, operator.ban_list.clone()));
    if manager.is_multi() {
        manager.seed_key_index();
        if !manager.key_index.is_empty() {
            println!("{title}: {} user auth key(s) known", manager.key_index.len());
        }
    }

    let allow_list = Arc::new(std::sync::RwLock::new(crate::websocket::parse_allow_list_csv(&operator.settings.websocket_allow_list)));
    let whitelisted_host: Arc<std::sync::RwLock<Option<String>>> = Arc::new(std::sync::RwLock::new(None));
    let login = match kind { ServerKind::Multi => MultiLogin::Users, ServerKind::WebFront => MultiLogin::Anonymous };
    let attacher: Arc<dyn crate::http::Attacher> = manager.clone();
    let ws_state = Arc::new(WsConnectionState {
        backend: Backend::Multi { attacher, login },
        next_client_id: manager.next_client_id.clone(),
        // No server-wide password: a user's is password.dat's, a visitor has none.
        password_hash: Arc::new(std::sync::RwLock::new(crate::websocket::hash_password(""))),
        password_enabled: Arc::new(std::sync::RwLock::new(false)),
        allow_list: allow_list.clone(),
        whitelisted_host: whitelisted_host.clone(),
        ban_list: operator.ban_list.clone(),
    });
    let gate = SecurityGate {
        allow_list,
        whitelisted_host,
        // Every user's key answers a knock on clay_multi; clay_web has no key, so no knock.
        auth_key: match kind {
            ServerKind::Multi => KeySource::Index(manager.key_index.clone()),
            ServerKind::WebFront => KeySource::Single(Arc::new(std::sync::RwLock::new(None))),
        },
        web_path: operator.settings.web_path.clone(),
        ban_list: operator.ban_list.clone(),
    };

    let mut https = crate::http::HttpsServer::new(port);
    // A cold reload's successor waits for the port its predecessor held; a start binds once.
    https.bind_retry = crate::http::port_released_by_predecessor(port);
    let scheme = if tls.is_some() { "https" } else { "http" };
    if let Err(e) = crate::http::start_https_server(
        &mut https, tls.as_ref().map(|(c, k)| (c.as_str(), k.as_str())), Some(ws_state), operator.ban_list.clone(),
        operator.gui_theme_colors().to_css_vars(), gate,
    ).await {
        eprintln!("{}", crate::http::start_warning(if tls.is_some() { "HTTPS" } else { "HTTP" }, e.as_ref()));
        crate::crashlog::exit_process(1, "web server failed to start");
    }
    match &web {
        // clay_web's start-up report, in the user's format (2026-11-11): the game and the
        // page's scheme and port, the file the settings came from, and which certificate -
        // or, only for a game that itself speaks TLS, that the page has none and is plain
        // http (a game reached in the clear has nothing to point out).
        Some(web) => {
            println!("Clay Portal: {} ({}:{}) on {scheme}/{port}", web.world_name, web.hostname, web.port);
            println!("   Loading settings: {}", web_dat_display(&web_dat_path()));
            match &tls {
                Some((cert, _)) => println!("   Certificate: {cert}"),
                None if web.ssl => println!("   https disabled - No certificate"),
                None => {}
            }
        }
        None => {
            let path = if operator.settings.web_path.is_empty() { String::new() } else { format!("{}/", operator.settings.web_path) };
            println!("{title}: {scheme}://0.0.0.0:{port}/{path}");
            if !operator.settings.websocket_allow_list.is_empty() {
                println!("Allow list: {}", operator.settings.websocket_allow_list);
            }
            println!("{title} running. Ctrl+C stops it; kill -USR1 reloads it (every user is told and reconnects).");
        }
    }

    wait_for_signals(manager, port).await
}

/// Park until a signal: TERM/INT/HUP flush every instance and exit; USR1 flushes them,
/// tells their clients `ServerReloading`, and execs this process afresh with the same
/// command line (`platform::exec_cold_restart`; the listener is new, the port kept).
#[cfg(unix)]
async fn wait_for_signals(manager: Arc<InstanceManager>, port: u16) -> io::Result<()> {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    let mut hup = signal(SignalKind::hangup())?;
    let mut usr1 = signal(SignalKind::user_defined1())?;
    let reason = tokio::select! {
        _ = term.recv() => "SIGTERM",
        _ = int.recv() => "SIGINT",
        _ = hup.recv() => "SIGHUP",
        _ = usr1.recv() => "SIGUSR1",
    };
    let reloading = reason == "SIGUSR1";
    let (users, visitors) = manager.counts();
    println!("{reason}: {} ({users} user Clay(s), {visitors} visitor(s))...", if reloading { "reloading" } else { "shutting down" });
    let m = manager.clone();
    let _ = tokio::task::spawn_blocking(move || m.shutdown_all(reloading)).await;
    if reloading {
        #[cfg(not(target_os = "android"))]
        if let Err(e) = crate::platform::exec_cold_restart(Some(port)) {
            eprintln!("reload failed: {e}");
            crate::crashlog::exit_reason("cold reload failed");
            return Err(e);
        }
        let _ = port;
    }
    crate::crashlog::exit_reason(reason);
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_signals(manager: Arc<InstanceManager>, _port: u16) -> io::Result<()> {
    tokio::signal::ctrl_c().await?;
    println!("Ctrl+C: shutting down...");
    let m = manager.clone();
    let _ = tokio::task::spawn_blocking(move || m.shutdown_all(false)).await;
    crate::crashlog::exit_reason("Ctrl+C");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_path_is_shown_as_the_operator_sees_it() {
        let cwd = Path::new("/srv/portal");
        assert_eq!(web_dat_display_in(Path::new("/srv/portal/web.dat"), Some(cwd)), "web.dat");
        assert_eq!(web_dat_display_in(Path::new("/srv/portal/conf/other.dat"), Some(cwd)), "conf/other.dat");
        assert_eq!(web_dat_display_in(Path::new("/etc/clay/web.dat"), Some(cwd)), "/etc/clay/web.dat");
        assert_eq!(web_dat_display_in(Path::new("/srv/portal"), Some(cwd)), "/srv/portal", "the directory itself is not an empty name");
        assert_eq!(web_dat_display_in(Path::new("/srv/portal/web.dat"), None), "/srv/portal/web.dat");
    }
}
