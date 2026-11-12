//! The one event loop of every Clay without a console of its own.
//!
//! `prepare_app` builds an `App` the way a headless Clay always has (settings or the
//! reload state, theme, keybindings, archive, the reconnection of carried-over sockets,
//! the WebSocket client table, the web server, TinyFugue's startup) and `run_app_loop`
//! is the loop that drives it. The GUI masters (`run_app_headless`), the `-D` daemon
//! (`daemon::run_daemon_server`) and every instance of the two server builds
//! (`instance`, a clay_multi user or a clay_web visitor) are wrappers around these two:
//! they differ in `SetupOptions`/`LoopOptions`, never in a loop of their own. Before this
//! the daemon had a copy that had drifted - no reload state, no GA/EOR prompts, no FANSI
//! login timeout, no timed prompts, no pending-count broadcasts (2026-11-09).

use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::{App, AppEvent, OutputLine, Settings, WorldSettings, WriteCommand, WsOverride};
use crate::websocket::{WebSocketServer, WsMessage};
use crate::http::SecurityGate;
use crate::telnet::{KeepAliveType, StreamReader, StreamWriter, TelnetConfig, TELNET_IAC, TELNET_NOP};
use crate::telnet_reader::{spawn_telnet_reader, TelnetTarget};
use crate::telnet_writer::spawn_telnet_writer;
use crate::{debug_log, is_debug_enabled};
use crate::{persistence, tf, tfrun, tfstartup};

/// How the web server is started, if at all.
pub(crate) enum WebServer {
    /// The GUI masters and `--local-server`: `start_web_server_at_startup` (the next free
    /// port when the configured one is taken, the owner named in the output).
    Headless,
    /// `-D`: the configured port, a bind retry only for a reload's successor, a banner on
    /// stdout.
    Daemon,
    /// An instance: the front has the listener; this Clay only manages its clients.
    None,
}

/// Which of TinyFugue's startup steps run (`tfstartup::run_tf_startup_daemon`).
pub(crate) enum TfStartup {
    /// Startup actions, and the rc/`-c`/command-line world unless this is the Android
    /// app's embedded server.
    Headless,
    /// `-D`: the rc and the command line, no startup actions (as it always was).
    Daemon,
    /// An instance: its own startup actions, never the operator's rc.
    StartupActionsOnly,
}

/// Where `AppEvent::SystemMessage` goes.
pub(crate) enum SystemMessageSink {
    /// Into the current world's output, broadcast (the GUI masters, instances).
    Output,
    /// stdout (`-D`, which has a terminal and no display of its own).
    Stdout,
}

/// What an instance's manager tells its loop.
pub(crate) enum InstanceCmd {
    /// Flush and end: `quit_cleanup`, telling the clients `ServerReloading` first when the
    /// server is about to exec itself.
    Shutdown { reloading: bool },
}

/// A visitor's Clay is built from these instead of files: the operator's settings and
/// the one world, both already scrubbed (`Settings::visitor_template`).
pub struct InstanceTemplate {
    pub settings: Settings,
    pub world_name: String,
    pub world: WorldSettings,
    /// The operator's themes: a visitor's `InitialState` names its colours from these (the
    /// built-ins alone showed the operator's theme only until the first message).
    pub theme: crate::theme::ThemeFile,
}

pub(crate) struct SetupOptions {
    /// Read `~/.clay/reload.<pid>` when started with `--reload`/`--crash` and reconstruct
    /// the carried-over connections (the main Clay only).
    pub load_reload_state: bool,
    pub ws_override: Option<WsOverride>,
    pub port_override: Option<u16>,
    pub web_server: WebServer,
    /// A visitor's in-memory settings; `None` loads settings.dat.
    pub template: Option<InstanceTemplate>,
    /// Register TF's SIGTERM/SIGHUP/SIGUSR2 hooks and the SIGUSR1 reload (once per
    /// process: the main Clay).
    pub signals: bool,
    /// The embedded GUI's channel (client 0) and repaint callback.
    pub gui_tx: Option<mpsc::UnboundedSender<WsMessage>>,
    pub gui_repaint: Option<Arc<dyn Fn() + Send + Sync>>,
    /// An instance's client table, made by its manager (sharing the front's client ids);
    /// `None` makes the usual one when a password or key is set.
    pub ws_server: Option<WebSocketServer>,
}

pub(crate) struct Prepared {
    pub app: App,
    pub event_tx: mpsc::Sender<AppEvent>,
    pub event_rx: mpsc::Receiver<AppEvent>,
}

pub(crate) struct LoopOptions {
    /// The embedded GUI's messages (client 0).
    pub gui_rx: Option<mpsc::UnboundedReceiver<WsMessage>>,
    /// Watch `GUI_QUIT` and the GUI's reload flag (the GUI masters).
    pub gui_quit: bool,
    /// An instance's commands from its manager.
    pub cmd_rx: Option<mpsc::Receiver<InstanceCmd>>,
    pub system_message: SystemMessageSink,
    /// A reload or an installed update execs this process (the main Clay); an instance
    /// returns `LoopExit::ReloadRequested` instead (it never gets one: its `/reload` and
    /// `/update` are refused).
    pub hot_reload: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LoopExit {
    /// The GUI this Clay runs in closed; `quit_cleanup` has run.
    Quit,
    /// The manager asked; `quit_cleanup` has run.
    Shutdown,
    /// SIGTERM or SIGHUP: TF's hook has run and its sends had their moment; the caller ends
    /// the process.
    Signal(tf::TfHookEvent),
    /// A reload was asked for where this loop may not exec.
    ReloadRequested,
}

const FAR_FUTURE: Duration = Duration::from_secs(86400);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Build the App: files or the reload state, the connections carried over, the client
/// table, the web server, TinyFugue's startup.
pub(crate) async fn prepare_app(opts: SetupOptions) -> io::Result<Prepared> {
    let SetupOptions { load_reload_state, ws_override, port_override, web_server, template, signals, gui_tx, gui_repaint, ws_server } = opts;
    let mut app = App::new();
    app.gui_tx = gui_tx.clone();
    app.gui_repaint = gui_repaint.clone();

    let is_reload = load_reload_state && std::env::args().any(|a| a == "--reload");
    let is_crash = load_reload_state && std::env::args().any(|a| a == "--crash");
    app.is_reload = is_reload;
    if load_reload_state {
        crate::platform::CRASH_COUNT.store(crate::platform::get_crash_count(), Ordering::SeqCst);
    }
    let should_load_state = is_reload || is_crash;
    let from_template = template.is_some();

    if let Some(template) = template {
        // A visitor: the operator's settings in memory and one world, nothing from disk.
        app.settings = template.settings;
        app.theme_file = template.theme;
        let mut world = crate::World::new(&template.world_name);
        world.settings = template.world;
        app.worlds.push(world);
        app.current_world_index = 0;
    } else if should_load_state {
        debug_log(is_debug_enabled(), "HEADLESS STARTUP: Loading reload state...");
        match persistence::load_reload_state(&mut app) {
            Ok(true) => {
                debug_log(is_debug_enabled(), "HEADLESS STARTUP: Reload state loaded successfully");
                for (idx, world) in app.worlds.iter().enumerate() {
                    debug_log(is_debug_enabled(), &format!(
                        "HEADLESS RELOAD STATE: World[{}] '{}' showing_splash={} output_lines={} connected={}",
                        idx, world.name, world.showing_splash, world.output_lines.len(), world.connected
                    ));
                }
            }
            Ok(false) => {
                debug_log(is_debug_enabled(), "HEADLESS STARTUP: No reload state found");
                if let Err(e) = persistence::load_settings(&mut app) {
                    eprintln!("Warning: Could not load settings: {}", e);
                }
            }
            Err(e) => {
                debug_log(is_debug_enabled(), &format!("HEADLESS STARTUP: Failed to load reload state: {}", e));
                if let Err(e) = persistence::load_settings(&mut app) {
                    eprintln!("Warning: Could not load settings: {}", e);
                }
            }
        }
    } else {
        // Normal startup - load settings
        if let Err(e) = persistence::load_settings(&mut app) {
            eprintln!("Warning: Could not load settings: {}", e);
        }
        // Clear runtime state on fresh start
        for world in &mut app.worlds {
            world.connected = false;
            world.command_tx = None;
            world.socket_fd = None;
            world.pending_lines.clear();
            world.pending_since = None;
            world.paused = false;
            world.unseen_lines = 0;
            world.first_unseen_at = None;
            world.lines_since_pause = 0;
        }
    }

    // Sync global debug flag from loaded settings (the main Clay's only)
    app.apply_debug_setting();

    // Pre-compile action regexes after loading settings or reload state
    crate::actions::compile_all_action_regexes(&mut app.settings.actions);
    // Kokoro as the engine: load its model now, not on the first line spoken.
    app.warm_kokoro();

    if !from_template {
        // Load theme file (~/.clay/theme.dat)
        crate::load_theme_file(&mut app);
        // Load keyboard bindings (~/.clay/keybindings.dat)
        let key_path = crate::clay_config_path("keybindings.dat");
        app.keybindings = crate::keybindings::KeyBindings::load(&key_path);
    }

    app.ensure_has_world();
    if !from_template {
        app.ensure_activity_world();
    }

    // Open scrollback archive if enabled - a Clay with files of its own.
    if app.policy.user_paths {
        app.init_scrollback();
    } else {
        // A visitor never opens maps.db either.
        app.maps_tried = true;
    }

    let (event_tx, event_rx) = mpsc::channel::<AppEvent>(100);
    app.event_tx = Some(event_tx.clone());
    app.tf_event_tx = Some(event_tx.clone());
    // TF's SIGTERM/SIGHUP/SIGUSR2 hooks (and a clean end on TERM/HUP).
    #[cfg(all(unix, not(target_os = "android")))]
    if signals {
        crate::spawn_tf_signal_handlers(&event_tx);
    }

    // Reconstruct connections from saved fds if in reload/crash mode
    #[cfg(any(unix, windows))]
    if should_load_state {
        reconstruct_connections(&mut app, &event_tx, is_crash).await?;
    }

    // In GUI master mode, enable the web interface with the GUI's password: the one the
    // user saved, or a session one that is never written to settings.dat (WsOverride).
    // The server always runs TLS-capable now (see http::route_connection's loopback
    // branch): localhost - where the WebView connects - is served plain regardless,
    // so there's no cert prompt for the desktop GUI; remote clients get TLS.
    crate::apply_ws_override(&mut app, ws_override);
    if let Some(port) = port_override {
        app.settings.http_port = port;
    }

    // Create WebSocket server state (for client management, no standalone listener).
    // Needed when a password is set OR when an auth key is set (auth-key-only mode) - or
    // handed in by an instance's manager.
    let ws_state = if let Some(server) = ws_server {
        let state = Arc::new(server.connection_state(event_tx.clone()));
        app.ws_server = Some(server);
        Some(state)
    } else if !app.settings.websocket_password.is_empty() || app.settings.websocket_auth_key.is_some() {
        let server = WebSocketServer::new(
            &app.settings.websocket_password,
            app.settings.http_port,
            &app.settings.websocket_allow_list,
            app.settings.websocket_whitelisted_host.clone(),
            false,
            app.ban_list.clone(),
        );
        let state = Arc::new(server.connection_state(event_tx.clone()));
        app.ws_server = Some(server);
        Some(state)
    } else {
        None
    };
    let gate = if let Some(server) = app.ws_server.as_ref() {
        SecurityGate {
            allow_list: server.allow_list.clone(),
            whitelisted_host: server.whitelisted_host.clone(),
            auth_key: crate::http::KeySource::Single(app.ws_auth_key_shared.clone()),
            web_path: app.settings.web_path.clone(),
            ban_list: app.ban_list.clone(),
        }
    } else {
        SecurityGate {
            allow_list: Arc::new(std::sync::RwLock::new(
                crate::websocket::parse_allow_list_csv(&app.settings.websocket_allow_list)
            )),
            whitelisted_host: Arc::new(std::sync::RwLock::new(app.settings.websocket_whitelisted_host.clone())),
            auth_key: crate::http::KeySource::Single(app.ws_auth_key_shared.clone()),
            web_path: app.settings.web_path.clone(),
            ban_list: app.ban_list.clone(),
        }
    };

    match web_server {
        // The web server, if enabled: one function for both GUI loops (the plain HTTP
        // listener and its CLAY_HTTP_LISTENER handle-passing are gone; a hot reload relies
        // on start_https_server's bind retry).
        WebServer::Headless => crate::start_web_server_at_startup(&mut app, ws_state.clone(), gate.clone()).await,
        // `-D`: always TLS for remote clients, like the TUI/GUI (see
        // resolve_web_cert_files: a configured cert wins, else an auto-generated one);
        // localhost is served plain by the accept loop itself. The configured port, kept
        // (people connect to it); a reload's successor waits for the port its
        // predecessor held, a cold start binds once.
        WebServer::Daemon => {
            if app.settings.http_enabled {
                match crate::resolve_web_cert_files(&mut app) {
                    Some((cert_file, key_file)) => {
                        let mut https_server = crate::http::HttpsServer::new(app.settings.http_port);
                        https_server.bind_retry = crate::http::port_released_by_predecessor(app.settings.http_port);
                        match crate::http::start_https_server(
                            &mut https_server,
                            Some((cert_file.as_str(), key_file.as_str())),
                            ws_state.clone(),
                            app.ban_list.clone(),
                            app.gui_theme_colors().to_css_vars(),
                            gate.clone(),
                        ).await {
                            Ok(()) => {
                                let protocol = if ws_state.is_some() { "HTTPS+WSS" } else { "HTTPS" };
                                println!("{}: https://0.0.0.0:{}", protocol, app.settings.http_port);
                                app.https_server = Some(https_server);
                            }
                            Err(e) => {
                                eprintln!("{}", crate::http::start_warning("HTTPS", e.as_ref()));
                            }
                        }
                    }
                    None => eprintln!("Error: no TLS certificate available; not starting the web server."),
                }
            }
        }
        WebServer::None => {}
    }

    // Re-set gui_tx and repaint callback after potential reload state load (reload clears them)
    app.gui_tx = gui_tx;
    app.gui_repaint = gui_repaint;

    Ok(Prepared { app, event_tx, event_rx })
}

/// TinyFugue's startup (the rc, -c, the world named on the command line) and Clay's
/// startup actions - not again after a reload or crash restart that carried the TF state
/// over (tf::snapshot): that would define every unnamed trigger a second time. A step of
/// its own so each wrapper runs it where it did: the GUI masters after their window's
/// InitialState, -D after its banner.
pub(crate) async fn tf_startup(app: &mut App, event_tx: &mpsc::Sender<AppEvent>, which: TfStartup) {
    match which {
        TfStartup::Headless => {
            // An embedded --local-server only runs the startup actions, as before.
            let local_server = crate::LOCAL_SERVER_LOOPBACK_ONLY.load(Ordering::SeqCst);
            tfstartup::run_tf_startup_daemon(app, event_tx, true, !local_server).await;
        }
        TfStartup::Daemon => tfstartup::run_tf_startup_daemon(app, event_tx, false, true).await,
        TfStartup::StartupActionsOnly => tfstartup::run_tf_startup_daemon(app, event_tx, true, false).await,
    }
}

/// A hot reload's or crash restart's successor: the worlds' plain TCP sockets came
/// through the exec (`CLAY_RELOAD_FDS`) and TLS worlds through their proxy process;
/// give each its reader and writer again.
#[cfg(any(unix, windows))]
async fn reconstruct_connections(app: &mut App, event_tx: &mpsc::Sender<AppEvent>, is_crash: bool) -> io::Result<()> {
    #[cfg(unix)]
    use std::os::unix::io::FromRawFd;
    #[cfg(windows)]
    use std::os::windows::io::FromRawSocket;

    // Log world state for debugging (always-on)
    for (idx, world) in app.worlds.iter().enumerate() {
        if world.connected || world.socket_fd.is_some() {
            debug_log(is_debug_enabled(), &format!(
                "HEADLESS RELOAD: World[{}] '{}' connected={} is_tls={} socket_fd={:?} proxy_pid={:?}",
                idx, world.name, world.connected, world.is_tls, world.socket_fd, world.proxy_pid
            ));
        }
    }
    debug_log(is_debug_enabled(), "HEADLESS STARTUP: Reconstructing connections...");
    // First pass: disconnect TLS worlds without proxy
    let mut tls_disconnect_worlds: Vec<usize> = Vec::new();
    for (world_idx, world) in app.worlds.iter().enumerate() {
        if world.connected && world.is_tls && world.proxy_pid.is_none() {
            tls_disconnect_worlds.push(world_idx);
        }
    }
    let tls_msg = if is_crash {
        "TLS connection was closed during crash recovery. Use /worlds to reconnect."
    } else {
        "TLS connection was closed during reload. Use /worlds to reconnect."
    };
    for world_idx in tls_disconnect_worlds {
        app.worlds[world_idx].connected = false;
        app.worlds[world_idx].command_tx = None;
        app.worlds[world_idx].socket_fd = None;
        let seq = app.worlds[world_idx].next_seq;
        app.worlds[world_idx].next_seq += 1;
        app.worlds[world_idx].output_lines.push(OutputLine::new_client(tls_msg.to_string(), seq));
        if !app.world_watched(world_idx) {
            if app.worlds[world_idx].unseen_lines == 0 {
                app.worlds[world_idx].first_unseen_at = Some(std::time::Instant::now());
            }
            app.worlds[world_idx].unseen_lines += 1;
        }
    }

    // MCCP2 hot-reload guard: see `App::apply_mccp2_reload_bailout`'s doc comment. Runs
    // after the TLS-without-proxy pass above so a world already disconnected there isn't
    // double-counted here.
    app.apply_mccp2_reload_bailout(is_crash);

    // Second pass: reconstruct plain TCP connections
    for world_idx in 0..app.worlds.len() {
        let world = &app.worlds[world_idx];
        if let Some(fd) = world.socket_fd.filter(|_| world.connected && !world.is_tls) {
            debug_log(is_debug_enabled(), &format!("HEADLESS RELOAD: Reconstructing plain TCP for '{}' fd={}", world.name, fd));
            #[cfg(unix)]
            let tcp_stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
            #[cfg(windows)]
            let tcp_stream = unsafe { std::net::TcpStream::from_raw_socket(fd as u64) };
            tcp_stream.set_nonblocking(true)?;
            let tcp_stream = tokio::net::TcpStream::from_std(tcp_stream)?;
            let (r, w) = tcp_stream.into_split();
            let read_half = StreamReader::Plain(r);
            let write_half = StreamWriter::Plain(w);
            // A reload *reconstruction* of an already-connected world, not a fresh
            // connect, so effective_encoding() (not the bare settings default) is the
            // right initial encoding - it already reflects a CHARSET negotiated before
            // the reload (negotiated_encoding survives reload/crash recovery).
            let initial_encoding = app.worlds[world_idx].effective_encoding();
            let cmd_tx = spawn_telnet_writer(write_half, initial_encoding);
            app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
            app.worlds[world_idx].skip_auto_login = true;
            // MCCP2 hot-reload drain: resume compression if this world was marked before
            // the exec - see resume_mccp2_after_reload's doc comment.
            app.worlds[world_idx].resume_mccp2_after_reload();
            app.worlds[world_idx].open_log_file();
            let world_name = app.worlds[world_idx].name.clone();
            app.worlds[world_idx].connection_id += 1;
            app.worlds[world_idx].reader_name = Some(world_name.clone());
            spawn_reconstructed_reader(app, world_idx, read_half, cmd_tx, event_tx);
        }

        // Reconstruct TLS proxy connections
        #[cfg(all(unix, not(target_os = "android")))]
        {
            let world = &app.worlds[world_idx];
            if world.connected && world.is_tls && world.proxy_pid.is_some() {
                if let Some(ref socket_path) = world.proxy_socket_path {
                    match tokio::net::UnixStream::connect(socket_path).await {
                        Ok(unix_stream) => {
                            let (r, w) = unix_stream.into_split();
                            let read_half = StreamReader::Proxy(r);
                            let write_half = StreamWriter::Proxy(w);
                            let initial_encoding = app.worlds[world_idx].effective_encoding();
                            let cmd_tx = spawn_telnet_writer(write_half, initial_encoding);
                            app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
                            app.worlds[world_idx].skip_auto_login = true;
                            app.worlds[world_idx].resume_mccp2_after_reload();
                            app.worlds[world_idx].open_log_file();
                            let world_name = app.worlds[world_idx].name.clone();
                            app.worlds[world_idx].connection_id += 1;
                            app.worlds[world_idx].reader_name = Some(world_name.clone());
                            spawn_reconstructed_reader(app, world_idx, read_half, cmd_tx, event_tx);
                        }
                        Err(e) => {
                            debug_log(is_debug_enabled(), &format!("HEADLESS: Failed to reconnect proxy for {}: {}", app.worlds[world_idx].name, e));
                            app.worlds[world_idx].clear_connection_state(false, false);
                        }
                    }
                }
            }
        }

        // Reconstruct TLS proxy connections on Windows via Named Pipe
        #[cfg(windows)]
        {
            let world = &app.worlds[world_idx];
            if let Some(proxy_pid) = world.proxy_pid.filter(|_| world.connected && world.is_tls) {
                let pipe_path = world.proxy_socket_path.clone();

                let alive = crate::platform::is_process_alive(proxy_pid);
                debug_log(is_debug_enabled(), &format!("HEADLESS RELOAD: proxy pid={} alive={} path={:?}", proxy_pid, alive, pipe_path));

                if !alive {
                    app.worlds[world_idx].clear_connection_state(false, false);
                    let seq = app.worlds[world_idx].next_seq;
                    app.worlds[world_idx].next_seq += 1;
                    app.worlds[world_idx].output_lines.push(OutputLine::new_client(
                        "TLS proxy terminated during reload. Use /worlds to reconnect.".to_string(), seq
                    ));
                } else if let Some(ref path) = pipe_path {
                    let client_opt = crate::platform::connect_to_proxy_pipe(path, 5).await;
                    debug_log(is_debug_enabled(), &format!("HEADLESS RELOAD: pipe connect result: {}", client_opt.is_some()));
                    match client_opt {
                        Some(pipe_client) => {
                            let (r, w) = tokio::io::split(pipe_client);
                            let read_half = StreamReader::NamedPipeProxy(r);
                            let write_half = StreamWriter::NamedPipeProxy(w);
                            let initial_encoding = app.worlds[world_idx].effective_encoding();
                            let cmd_tx = spawn_telnet_writer(write_half, initial_encoding);
                            app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
                            app.worlds[world_idx].skip_auto_login = true;
                            app.worlds[world_idx].resume_mccp2_after_reload();
                            app.worlds[world_idx].open_log_file();
                            let world_name = app.worlds[world_idx].name.clone();
                            app.worlds[world_idx].connection_id += 1;
                            app.worlds[world_idx].reader_name = Some(world_name.clone());
                            spawn_reconstructed_reader(app, world_idx, read_half, cmd_tx, event_tx);
                        }
                        None => {
                            debug_log(is_debug_enabled(), &format!("HEADLESS RELOAD: failed to reconnect Windows proxy for '{}'", app.worlds[world_idx].name));
                            app.worlds[world_idx].clear_connection_state(false, false);
                            let seq = app.worlds[world_idx].next_seq;
                            app.worlds[world_idx].next_seq += 1;
                            app.worlds[world_idx].output_lines.push(OutputLine::new_client(
                                "TLS proxy pipe unavailable after reload. Use /worlds to reconnect.".to_string(), seq
                            ));
                        }
                    }
                }
            }
        }
    }

    app.resume_chat_worlds_after_reload(event_tx);
    #[cfg(target_os = "linux")]
    app.adopt_shell_worlds_after_reload();
    // Cleanup: mark disconnected worlds that claim to be connected but have no command channel
    // (a re-adopted shell world has none - its session is its connection)
    for world in &mut app.worlds {
        if world.connected && world.command_tx.is_none() && world.shell.is_none() {
            debug_log(is_debug_enabled(), &format!(
                "HEADLESS RELOAD CLEANUP: World '{}' connected but no command_tx - marking disconnected (socket_fd={:?})",
                world.name, world.socket_fd
            ));
            world.connected = false;
            world.socket_fd = None;
            world.pending_lines.clear();
            world.paused = false;
            let seq = world.next_seq;
            world.next_seq += 1;
            world.output_lines.push(OutputLine::new_client(
                "Connection was not restored during reload. Use /worlds to reconnect.".to_string(), seq
            ));
        }
    }

    // The current world may be on its splash (restored, or regenerated by
    // ensure_has_world for an empty one) and stays there: the splash renderer draws
    // the output lines centred under the art, so the message shows. Forcing the splash
    // off here left the regenerated art at column 0 after every reload.
    let reload_msg = if is_crash {
        crate::crashlog::restart_notice()
    } else {
        crate::selfupdate::RELOADED.to_string()
    };
    app.add_output(&reload_msg);
    if let Some(note) = crate::platform::restart_fallback_note() {
        app.add_output(note);
    }
    Ok(())
}

/// The reader task of a reconstructed connection: telnet is handled as on a fresh
/// connect (negotiation answered on `cmd_tx`, GA/EOR prompts, GMCP/MSDP) - this used to
/// pipe raw bytes with no telnet processing at all after a reload.
#[cfg(any(unix, windows))]
fn spawn_reconstructed_reader(app: &mut App, world_idx: usize, read_half: StreamReader, cmd_tx: mpsc::Sender<WriteCommand>, event_tx: &mpsc::Sender<AppEvent>) {
    let world_name = app.worlds[world_idx].name.clone();
    let reader_conn_id = app.worlds[world_idx].connection_id;
    let telnet_cfg = TelnetConfig {
        term_type: std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string()),
        msp_enabled: app.worlds[world_idx].settings.msp_enabled,
        mccp2_enabled: app.worlds[world_idx].settings.mccp2_enabled,
        gmcp_enabled: app.policy.gmcp,
        is_tls: app.worlds[world_idx].is_tls,
        diag: Some(app.worlds[world_idx].protocol.diag_for_reader()),
        ..TelnetConfig::default()
    };
    spawn_telnet_reader(
        read_half,
        cmd_tx,
        event_tx.clone(),
        TelnetTarget::World(world_name),
        reader_conn_id,
        telnet_cfg,
    );
}

/// Drive the App until it ends: the MUD connections, the clients, TinyFugue's timers.
pub(crate) async fn run_app_loop(app: &mut App, mut event_rx: mpsc::Receiver<AppEvent>, event_tx: mpsc::Sender<AppEvent>, opts: LoopOptions) -> io::Result<LoopExit> {
    let LoopOptions { mut gui_rx, gui_quit, mut cmd_rx, system_message, hot_reload } = opts;

    // Keepalive interval
    let mut keepalive_interval = tokio::time::interval(Duration::from_secs(60));
    keepalive_interval.tick().await;

    // Conditional timers: sleep far-future when inactive, reset to short interval when needed.

    // FANSI login timeouts, dirty stats and timed-prompt deadlines (`App::next_timed_prompt_deadline`)
    let prompt_check_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(prompt_check_sleep);

    // Timed-prompt inference: a restored partial from a hot-reload/crash-recovery
    // Timed Prompt world needs its deadline picked up immediately, or it would sit
    // parked until some unrelated event happens to rearm this timer.
    if let Some(deadline) = app.next_timed_prompt_deadline() {
        crate::pull_in_deadline(prompt_check_sleep.as_mut(), deadline);
    }

    // TF repeat process ticks - only active when processes exist
    let process_tick_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(process_tick_sleep);

    // Pending count broadcast - only active when worlds have pending lines
    let pending_update_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(pending_update_sleep);

    // Auto-reconnect timer - fires when a world is due for reconnection
    let reconnect_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(reconnect_sleep);
    // The mapper's walks (src/map): a step whose room the game hasn't reported in time ends
    // its walk. Armed at the bottom of every iteration from `next_walk_deadline`.
    let walk_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(walk_sleep);
    // A hot reload restores `World::reconnect_at` from the state file, but this timer
    // always starts at FAR_FUTURE and is only ever re-armed by later events or by firing
    // once - arm it here, once, from whatever `load_reload_state` just restored.
    if let Some(next) = app.next_reconnect_instant() {
        let dur = next.saturating_duration_since(std::time::Instant::now());
        reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
    }

    // GUI reload check - polls atomic flag set by IPC handler (100ms interval)
    let mut gui_reload_check = tokio::time::interval(Duration::from_millis(100));
    gui_reload_check.tick().await; // consume first immediate tick

    // SIGUSR1 handler for hot reload: the GUI masters' (as it was; -D reloads from a
    // client's /reload, an instance never from a signal of its own).
    #[cfg(all(unix, not(target_os = "android")))]
    if hot_reload && gui_quit {
        let sigusr1_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut sigusr1 = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to set up SIGUSR1 handler: {}", e);
                    return;
                }
            };
            loop {
                sigusr1.recv().await;
                let _ = sigusr1_tx.send(AppEvent::Sigusr1Received).await;
            }
        });
    }

    debug_log(is_debug_enabled(), "HEADLESS: Entering main event loop");

    // Main event loop
    loop {
        #[cfg(all(unix, not(target_os = "android")))]
        crate::platform::reap_zombie_children();

        tokio::select! {
            // The GUI this Clay runs in is closing (quit_in_process_clay): /quit's
            // cleanup, then nothing more - the process ends with the GUI.
            _ = crate::GUI_QUIT.notified(), if gui_quit => {
                debug_log(is_debug_enabled(), "HEADLESS: GUI quitting, running /quit's cleanup");
                app.quit_cleanup().await;
                crate::GUI_QUIT_DONE.store(true, Ordering::SeqCst);
                return Ok(LoopExit::Quit);
            }

            // The manager of an instance: flush and end (a server shutdown or its cold reload).
            Some(cmd) = async { match cmd_rx.as_mut() { Some(rx) => rx.recv().await, None => None } }, if cmd_rx.is_some() => {
                match cmd {
                    InstanceCmd::Shutdown { reloading } => {
                        if reloading {
                            app.ws_broadcast(WsMessage::ServerReloading);
                        }
                        app.quit_cleanup().await;
                        return Ok(LoopExit::Shutdown);
                    }
                }
            }

            // App events (server data, disconnects, WS client messages)
            Some(event) = event_rx.recv() => {
                // Slack/Discord events and connection-close notices: handled once, for every
                // loop (App::preprocess_event). What comes back - chat lines or a close message
                // as ServerData, a lost chat session as Disconnected - is handled below exactly
                // as if it had arrived by itself.
                // A fully-handled event (None) must still fall through to the draw/flush at
                // the bottom (NOT `continue`): a shell world's Wakeup is handled here and
                // its frame is flushed to clients below, which `continue` would skip.
                if let Some(event) = app.preprocess_event(event) {
                match event {
                    AppEvent::ServerData(ref world_name, ref bytes) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            // Use large width in daemon mode so text is not pre-wrapped -
                            // GUI/web clients handle wrapping via CSS
                            let commands = app.process_server_data(
                                world_idx, bytes, 24, 1000, true,
                            );
                            // Activate timer for FANSI login timeout
                            if app.worlds[world_idx].fansi_detect_until.is_some() {
                                prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                            }
                            // Activate pending update if lines were paused
                            if !app.worlds[world_idx].pending_lines.is_empty() {
                                pending_update_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(2));
                            }
                            let saved_current_world = app.current_world_index;
                            app.current_world_index = world_idx;
                            for cmd in commands {
                                if cmd.starts_with('/') {
                                    // Route through the TF parser and apply what the command
                                    // did, in order (see tfrun): every Clay command now runs,
                                    // not just /send, /notify and /say.
                                    app.sync_tf_world_info();
                                    let effects = app.run_tf_in_world(world_idx, &cmd);
                                    let ctx = tfrun::TfEffectCtx::for_world(world_idx, true);
                                    tfrun::run_tf_effects_daemon(app, 0, effects, ctx, &event_tx).await;
                                } else {
                                    // Plain text - send to MUD (captured via send_to_world)
                                    app.send_to_world(world_idx, cmd);
                                }
                            }
                            app.current_world_index = saved_current_world;
                        }
                    }
                    AppEvent::Chat(..) | AppEvent::Shell(..) | AppEvent::CloseNotice(..) | AppEvent::ConnectAttemptsFailed(..) | AppEvent::TlsCipher(..) => {} // handled by the prelude above
                    AppEvent::ChatLookupResult(client_id, request_id, world_index, result) => {
                        app.apply_chat_lookup_result(client_id, request_id, world_index, result);
                    }
                    AppEvent::WorldNotice(world_name, text) => {
                        if let Some(world_idx) = app.find_world_index(&world_name) {
                            app.add_output_to_world(world_idx, &text);
                        }
                    }
                    AppEvent::WorldConnectResult(world_name, connection_id, origin, result) => {
                        app.handle_world_connect_result(&world_name, connection_id, origin, result);
                        if let Some(next) = app.next_reconnect_instant() {
                            let dur = next.saturating_duration_since(std::time::Instant::now());
                            reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                        }
                    }
                    AppEvent::Disconnected(ref world_name, conn_id, ref reason) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            if conn_id != app.worlds[world_idx].connection_id { continue; }
                            app.handle_disconnected(world_idx, reason);
                            if let Some(next) = app.next_reconnect_instant() {
                                let dur = next.saturating_duration_since(std::time::Instant::now());
                                reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                            }
                        }
                    }
                    AppEvent::WsClientMessage(client_id, msg) => {
                        if let WsMessage::AuthRequest { current_world, resume, resume_epochs, client_uid, interface_key, instance_id, .. } = &*msg {
                            // The stable ownership id first, as the console master does: the
                            // InitialState's your_display_id must match what claims stamp,
                            // or a password login loses its ▶ markers on every reconnect.
                            app.set_display_owner_id(client_id, client_uid);
                            app.set_client_interface(client_id, interface_key);
                            app.set_client_instance(client_id, instance_id);
                            app.handle_ws_auth_initial_state(client_id, *current_world, resume.clone(), resume_epochs.clone());
                            // A visitor's page: its one world connects now (`%host%` is its address).
                            app.after_client_login(client_id, &event_tx);
                            if app.web_reconnect_needed {
                                app.web_reconnect_needed = false;
                                if app.trigger_web_reconnects() {
                                    if let Some(next) = app.next_reconnect_instant() {
                                        let dur = next.saturating_duration_since(std::time::Instant::now());
                                        reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                                    }
                                }
                            }
                        } else {
                            crate::daemon::handle_daemon_ws_message(app, client_id, *msg, &event_tx).await;
                            if app.web_restart_needed {
                                app.web_restart_needed = false;
                                crate::restart_http_server(app, event_tx.clone()).await;
                            }
                        }
                    }
                    AppEvent::WsClientConnected(_client_id) => {}
                    AppEvent::WsClientDisconnected(client_id) => {
                        app.handle_ws_client_disconnected(client_id);
                    }
                    AppEvent::WsAuthKeyValidation(client_id, msg, client_ip, challenge) => {
                        app.handle_ws_auth_key_validation(client_id, *msg, &client_ip, &challenge);
                        if app.web_reconnect_needed {
                            app.web_reconnect_needed = false;
                            if app.trigger_web_reconnects() {
                                if let Some(next) = app.next_reconnect_instant() {
                                    let dur = next.saturating_duration_since(std::time::Instant::now());
                                    reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                                }
                            }
                        }
                    }
                    AppEvent::WsKeyRequest(client_id) => {
                        app.handle_ws_key_request(client_id);
                    }
                    AppEvent::WsKeyRevoke(_client_id, key) => {
                        app.handle_ws_key_revoke(&key);
                    }
                    AppEvent::TfShellDone(world_idx, output, status) => {
                        app.finish_tf_shell(world_idx, &output, status, true);
                    }
                    // SIGTERM/SIGHUP: TF's hook of that name, then Clay ends (after a
                    // moment for anything the hook sent to reach its world); SIGUSR2: the
                    // hook only.
                    AppEvent::TfSignal(event) => {
                        app.fire_tf_hook(None, event, "", true);
                        tfrun::drain_deferred_tf_daemon(app, &event_tx).await;
                        if event != tf::TfHookEvent::Sigusr2 {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            return Ok(LoopExit::Signal(event));
                        }
                    }
                    AppEvent::SystemMessage(msg) => match system_message {
                        // Add to current world's output and broadcast. Routed through
                        // push_and_broadcast_line so the broadcast carries the line's REAL
                        // seq: it used to store a real seq server-side but send `seq: 0,
                        // end_seq: None`, so the client never recorded that seq as delivered
                        // and the Phase C ack audit saw a permanent gap there forever.
                        SystemMessageSink::Output => {
                            let world_idx = app.current_world_index;
                            let seq = app.worlds[world_idx].next_seq;
                            app.worlds[world_idx].next_seq += 1;
                            let more_mode = app.settings.more_mode_enabled;
                            let line = OutputLine::new_client(msg.clone(), seq);
                            app.push_and_broadcast_line(world_idx, line, more_mode);
                        }
                        // -D: to its terminal (connection rejections included).
                        SystemMessageSink::Stdout => println!("{}", msg),
                    },
                    AppEvent::Sigusr1Received => {
                        if !hot_reload {
                            return Ok(LoopExit::ReloadRequested);
                        }
                        // A Clay on Android has no hot reload: say so rather than nothing.
                        #[cfg(target_os = "android")]
                        app.add_output("Hot reload is not available on this platform.");
                        #[cfg(not(target_os = "android"))]
                        {
                            // Always log reload trigger (not gated by debug flag)
                            debug_log(is_debug_enabled(), "HEADLESS: Reload triggered, calling exec_reload");
                            for (idx, world) in app.worlds.iter().enumerate() {
                                debug_log(is_debug_enabled(), &format!(
                                    "HEADLESS PRE-RELOAD: World[{}] '{}' showing_splash={} output_lines={} connected={}",
                                    idx, world.name, world.showing_splash, world.output_lines.len(), world.connected
                                ));
                            }
                            app.ws_broadcast(WsMessage::ServerReloading);
                            // MCCP2 hot-reload drain (job 2 of 2 - see CLAUDE.md's
                            // hot-reload notes): defer instead of exec'ing now if any
                            // connected world is compressed - the bottom-of-loop poll
                            // fires the real exec once every drained world's stream ends,
                            // or the drain deadline passes.
                            if !app.should_defer_reload_for_mccp2() {
                                crate::platform::exec_reload(app)?;
                                return Ok(LoopExit::ReloadRequested);
                            }
                        }
                    }
                    AppEvent::ConnectionSuccess(world_name, cmd_tx, socket_fd, is_tls) => {
                        app.handle_connection_success(&world_name, cmd_tx, socket_fd, is_tls);
                    }
                    AppEvent::ConnectionFailed(world_name, conn_id, failure) => {
                        // CONFAIL (and ICONFAIL) hooks, the message and the reconnect; a
                        // certificate mismatch is offered through the CertMismatch dialog
                        // report_connect_failure broadcasts.
                        if let Some(world_idx) = app.find_world_index(&world_name) {
                            if app.worlds[world_idx].connection_id == conn_id {
                                app.report_connect_failure(world_idx, &failure, true, true);
                            }
                        }
                    }
                    // Consolidated telnet dispatch: handle_telnet_event is exhaustive over
                    // TelnetEvent, so no event kind can be left out here.
                    AppEvent::Telnet(ref target, ref ev) => {
                        match target {
                            TelnetTarget::World(world_name) => {
                                if let Some(world_idx) = app.find_world_index(world_name) {
                                    app.handle_telnet_event(world_idx, ev);
                                }
                            }
                        }
                    }
                    AppEvent::MediaFileReady(world_idx, key, path, volume, loops, is_music) => {
                        app.on_media_file_ready(world_idx, key, path, volume, loops, is_music);
                    }
                    AppEvent::ApiLookupResult(client_id, world_index, result, cursor_start) => {
                        app.apply_api_lookup_result(client_id, world_index, result, cursor_start);
                    }
                    AppEvent::RemoteListResult(requesting_client_id, world_index, lines) => {
                        app.remote_ping_responses = None;
                        if requesting_client_id == 0 {
                            for line in &lines {
                                app.add_output(line);
                            }
                        } else {
                            for line in &lines {
                                app.ws_send_to_client(requesting_client_id, WsMessage::ServerData { archive_sourced: false,
                                    world_index,
                                    data: line.clone(),
                                    is_viewed: false,
                                    ts: crate::current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0, end_seq: None,
                                    flush: false, gagged: false, highlight_colors: Vec::new(),
                                });
                            }
                        }
                    }
                    AppEvent::PublicIpResult(result, reply_to) => {
                        let line = app.apply_public_ip_result(result);
                        app.reply_reach_line(reply_to, &line);
                    }
                    AppEvent::FirewallResult(status, action_error, reply_to) => {
                        let line = app.apply_firewall_result(status, action_error);
                        app.reply_reach_line(reply_to, &line);
                    }
                    AppEvent::PortMapResult(status, reply_to) => {
                        let line = app.apply_port_map_result(status);
                        app.reply_reach_line(reply_to, &line);
                    }
                    AppEvent::UpdateResult { result, reply_to } => {
                        // src/selfupdate.rs: install + answer whoever asked; on a new
                        // binary, hot reload (deferred by the MCCP2 drain when a
                        // connected world is compressed - see the Sigusr1Received arm
                        // above and the bottom-of-loop poll).
                        #[cfg(not(target_os = "android"))]
                        {
                            if app.finish_server_update(result, reply_to) {
                                if !hot_reload {
                                    return Ok(LoopExit::ReloadRequested);
                                }
                                crate::platform::exec_reload(app)?;
                                return Ok(LoopExit::ReloadRequested);
                            }
                        }
                        #[cfg(target_os = "android")]
                        {
                            // No hot reload on Android; the install is refused upstream.
                            let _ = app.finish_server_update(result, reply_to);
                        }
                    }
                    // Consumed by App::preprocess_event above; never reaches here.
                    AppEvent::TtsLine { .. } => {}
                    AppEvent::UpdateLine { .. } => {}
                    AppEvent::PictureUploaded { .. } => {}
                    AppEvent::ImportResult(client_id, addr, result) => {
                        app.handle_import_result(client_id, addr, result);
                    }
                    AppEvent::Prompt(ref world_name, ref prompt_bytes) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_prompt(world_idx, prompt_bytes, crate::PromptSource::Marker);
                        }
                    }
                    _ => {}
                }
                }
            }

            // GUI messages (same format as WsMessage - reuse daemon handler)
            Some(msg) = async { match gui_rx.as_mut() { Some(rx) => rx.recv().await, None => None } }, if gui_rx.is_some() => {
                // Use client_id 0 for the embedded GUI (not a real WS client)
                crate::daemon::handle_daemon_ws_message(app, 0, msg, &event_tx).await;
            }

            // Keepalive timer
            _ = keepalive_interval.tick() => {
                // Reap long-disconnected WS clients' view-state entries (WS_VIEWER_GRACE);
                // piggybacking on this existing once-a-minute tick rather than a new timer.
                app.reap_stale_ws_client_worlds();
                app.note_crash_context();
                if app.policy.server_wide {
                    app.maybe_renew_port_mapping();
                }
                for world in &mut app.worlds {
                    if world.connected && world.wants_keepalive() {
                        // Only check last_send_time: server kicks us when WE go idle.
                        let should_send = match world.last_send_time {
                            Some(t) => t.elapsed() >= KEEPALIVE_INTERVAL,
                            None => true,
                        };
                        if should_send {
                            if let Some(tx) = &world.command_tx {
                                let now = std::time::Instant::now();
                                match world.settings.keep_alive_type {
                                    KeepAliveType::None => {}
                                    KeepAliveType::Nop => {
                                        let nop = vec![TELNET_IAC, TELNET_NOP];
                                        let _ = tx.try_send(WriteCommand::Raw(nop));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Custom => {
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let idler_tag = crate::telnet::keep_alive_marker(rand_num);
                                        let cmd = world.settings.keep_alive_cmd
                                            .replace(crate::telnet::KEEP_ALIVE_RAND, &idler_tag);
                                        let _ = tx.try_send(WriteCommand::Text(cmd));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Generic => {
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let cmd = format!("help commands {}", crate::telnet::keep_alive_marker(rand_num));
                                        let _ = tx.try_send(WriteCommand::Text(cmd));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                }
                            }
                        }
                    }
                }

                // Check proxy health
                #[cfg(all(unix, not(target_os = "android")))]
                {
                    // Collect first: the notice has to be emitted through `app`
                    // (push_and_broadcast_line + WorldDisconnected), which can't be called
                    // while `app.worlds` is mutably borrowed by this loop.
                    let mut proxy_died: Vec<usize> = Vec::new();
                    for (world_idx, world) in app.worlds.iter_mut().enumerate() {
                        if world.connected {
                            if let Some(proxy_pid) = world.proxy_pid {
                                if !crate::platform::is_process_alive(proxy_pid) {
                                    world.clear_connection_state(false, false);
                                    proxy_died.push(world_idx);
                                }
                            }
                        }
                    }
                    let more_mode = app.settings.more_mode_enabled;
                    for world_idx in proxy_died {
                        let seq = app.worlds[world_idx].next_seq;
                        app.worlds[world_idx].next_seq += 1;
                        let line = OutputLine::new_client("TLS proxy terminated. Connection lost.".to_string(), seq);
                        app.push_and_broadcast_line(world_idx, line, more_mode);
                        app.ws_broadcast(WsMessage::WorldDisconnected { world_index: world_idx });
                    }
                }
            }

            // FANSI login timeouts, dirty stats and timed-prompt deadlines
            _ = &mut prompt_check_sleep => {
                let now = std::time::Instant::now();
                // Flush any world whose stats changed since the last tick. Reuses this
                // 150ms timer instead of a second one - see the bottom-of-loop block below
                // for how this timer gets rearmed while a world stays dirty.
                app.flush_dirty_stats();
                // Check for expired FANSI detection windows
                app.finish_due_fansi_logins(std::time::Instant::now());
                // Timed-prompt inference: promote any WorldType::MudTimedPrompt world
                // whose parked partial has waited past prompt_wait_ms.
                app.promote_due_timed_prompts(now);
                // Re-arm: check again in 150ms if any world still needs it
                let any_pending = app.worlds.iter().any(|w| w.fansi_detect_until.is_some());
                if any_pending {
                    prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                } else {
                    prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
                // Clamp to the next timed-prompt deadline if one is sooner than whatever
                // was just armed above (pull_in_deadline never pushes it later).
                if let Some(deadline) = app.next_timed_prompt_deadline() {
                    crate::pull_in_deadline(prompt_check_sleep.as_mut(), deadline);
                }
            }

            // TF repeat process tick - only fires when processes exist
            _ = &mut process_tick_sleep => {
                for (ctx, effects) in app.tick_tf_processes(std::time::Instant::now(), true) {
                    tfrun::run_tf_effects_daemon(app, 0, effects, ctx, &event_tx).await;
                }
                // Re-arm for the next process due, if any remain
                let next = app.next_tf_process_due()
                    .map(tokio::time::Instant::from_std)
                    .unwrap_or_else(|| tokio::time::Instant::now() + FAR_FUTURE);
                process_tick_sleep.as_mut().reset(next);
            }

            // GUI reload check - polls atomic flag set by WebView IPC handler
            _ = gui_reload_check.tick(), if gui_quit => {
                if crate::GUI_RELOAD_REQUESTED.swap(false, Ordering::SeqCst) {
                    #[cfg(not(target_os = "android"))]
                    {
                        debug_log(is_debug_enabled(), "HEADLESS: GUI reload flag detected, calling exec_reload");
                        for (idx, world) in app.worlds.iter().enumerate() {
                            debug_log(is_debug_enabled(), &format!(
                                "HEADLESS GUI-RELOAD: World[{}] '{}' showing_splash={} output_lines={} connected={}",
                                idx, world.name, world.showing_splash, world.output_lines.len(), world.connected
                            ));
                        }
                        app.ws_broadcast(WsMessage::ServerReloading);
                        // MCCP2 hot-reload drain: defer instead of exec'ing now if any
                        // connected world is compressed - see the Sigusr1Received arm above
                        // and the bottom-of-loop poll.
                        if !app.should_defer_reload_for_mccp2() {
                            crate::platform::exec_reload(app)?;
                            return Ok(LoopExit::ReloadRequested);
                        }
                    }
                }
            }

            // Pending count update - only fires when worlds have pending lines
            _ = &mut pending_update_sleep => {
                let now = std::time::Instant::now();
                for world in app.worlds.iter_mut() {
                    // Only send updates if world has pending lines and count has changed
                    let current_count = world.pending_lines.len();
                    if current_count > 0 && current_count != world.last_pending_count_broadcast {
                        // Check if 2 seconds have passed since last broadcast for this world
                        let should_broadcast = world.last_pending_broadcast
                            .map(|t| now.duration_since(t) >= Duration::from_secs(2))
                            .unwrap_or(true);
                        if should_broadcast {
                            world.last_pending_broadcast = Some(now);
                            world.last_pending_count_broadcast = current_count;
                        }
                    } else if current_count == 0 && world.last_pending_count_broadcast > 0 {
                        // Pending was cleared - reset tracking
                        world.last_pending_count_broadcast = 0;
                        world.last_pending_broadcast = None;
                    }
                }
                // Broadcast pending updates for worlds that need it
                for idx in 0..app.worlds.len() {
                    let current_count = app.worlds[idx].pending_lines.len();
                    if current_count > 0 {
                        app.ws_broadcast_to_world(idx, WsMessage::PendingCountUpdate {
                            world_index: idx,
                            count: current_count,
                        });
                    }
                }
                // Re-arm: check again in 2s if any world still has pending lines
                let any_pending = app.worlds.iter().any(|w| !w.pending_lines.is_empty());
                if any_pending {
                    pending_update_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(2));
                } else {
                    pending_update_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }

            // The mapper's walk timer
            _ = &mut walk_sleep => {
                app.map_expire_walks();
                walk_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
            }
            // Auto-reconnect timer
            _ = &mut reconnect_sleep => {
                app.start_due_reconnects(&event_tx);
                // Re-arm timer for next scheduled reconnect
                if let Some(next) = app.next_reconnect_instant() {
                    let dur = next.saturating_duration_since(std::time::Instant::now());
                    reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                } else {
                    reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }
        }

        // TF effects that synchronous code (hooks, triggers) left for the loop
        tfrun::drain_deferred_tf_daemon(app, &event_tx).await;
        // TF's mail check, and its status area when a script has it, to the clients
        app.check_tf_mail();
        app.refresh_tf_status();
        // Shell worlds (src/shell): send any live PTY's screen change to clients now, every
        // iteration, so a shell world's output isn't stranded waiting on a timer.
        app.flush_shell_frames();
        // The mapper: changed rooms saved, map views told; the walk timer armed.
        app.flush_map();
        app.flush_logs();
        if let Some(due) = app.next_walk_deadline() {
            walk_sleep.as_mut().reset(tokio::time::Instant::from_std(due));
        }

        // Arm the process timer for the next process due (processes may have been added
        // during this iteration)
        if let Some(due) = app.next_tf_process_due() {
            let due = tokio::time::Instant::from_std(due);
            if process_tick_sleep.deadline() > due {
                process_tick_sleep.as_mut().reset(due);
            }
        }

        // Pull prompt_check_sleep's deadline in to ~150ms out if some world went dirty this
        // iteration (a GMCP/MSDP update via AppEvent::Telnet, or a clear on disconnect) and
        // nothing sooner is already scheduled. Never push the deadline *later* - a steady
        // stream of updates must still flush roughly every 150ms instead of debouncing
        // forever.
        if app.worlds.iter().any(|w| w.protocol.stats_dirty)
            && prompt_check_sleep.deadline() > tokio::time::Instant::now() + Duration::from_millis(150)
        {
            prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
        }

        // Same pull-in, for a timed-prompt deadline that just became sooner than
        // whatever prompt_check_sleep already had armed.
        if let Some(deadline) = app.next_timed_prompt_deadline() {
            crate::pull_in_deadline(prompt_check_sleep.as_mut(), deadline);
        }

        // MCCP2 hot-reload drain: fire a reload deferred by should_defer_reload_for_mccp2
        // (any of this loop's trigger sites above) once every drained world's stream has
        // ended, or the deadline passes - see mccp2_reload_ready_to_exec's doc comment.
        #[cfg(not(target_os = "android"))]
        if hot_reload && app.mccp2_reload_ready_to_exec() {
            debug_log(is_debug_enabled(), "HEADLESS: MCCP2 drain complete (or deadline hit), calling exec_reload");
            crate::platform::exec_reload(app)?;
            return Ok(LoopExit::ReloadRequested);
        }
    }
}
