use std::io::{self, Write as IoWrite};
use std::sync::Arc;
use std::time::Duration;

use tokio::{
    net::TcpStream,
    sync::mpsc,
};

use crate::*;
use crate::{
    App, WorldSettings, UserConnection,
    ClientViewState, Command, OutputLine,
    get_multiuser_settings_path,
    enable_tcp_keepalive, parse_command, current_timestamp_secs,
};
use crate::actions::{action_commands_to_run,
    find_invocable_action, rewrite_slashless_action};
use crate::telnet_reader::{spawn_telnet_reader, TelnetTarget};
use crate::telnet_writer::spawn_telnet_writer;
use crate::commands::{connect_slack, connect_discord, execute_send_command, execute_log_command,
    execute_disconnect_command, execute_add_world_command, execute_add_world_default_command,
    execute_remove_world_command, prepare_world_connect_host_port,
    execute_mssp_command, execute_msdp_command, execute_stats_command,
    execute_send_gmcp, execute_send_msdp};
// Only used by the #[cfg(test)] helpers below (RemoteConsole's own non-test use moved into
// the now-shared App::handle_cycle_world in main.rs - T39).
#[cfg(test)]
use crate::websocket::RemoteClientType;

/// Run headlessly as a local, loopback-only Clay instance for an embedding client — currently
/// the Android app's bundled server, spawned as a child process. No TUI, no native WebView; the
/// embedding client (e.g. the Android WebView) connects over ws://127.0.0.1:<port> using the
/// password supplied via the CLAY_WS_PASSWORD environment variable. This mirrors desktop GUI
/// master mode (`webview_gui::run_master_webgui` -> `run_app_headless`) minus opening a window —
/// the caller supplies its own client instead of a native WebView.
pub async fn run_local_server(port_override: Option<u16>) -> io::Result<()> {
    let password = match std::env::var("CLAY_WS_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("clay: --local-server requires a non-empty CLAY_WS_PASSWORD environment variable.");
            std::process::exit(1);
        }
    };

    println!("clay: starting local server (loopback only)...");
    crate::LOCAL_SERVER_LOOPBACK_ONLY.store(true, std::sync::atomic::Ordering::SeqCst);

    // These channels satisfy run_app_headless's GUI-bridge API; nothing reads/writes them here
    // since the embedding client talks to the App over the WebSocket server, not this channel.
    let (gui_tx, _gui_rx) = mpsc::unbounded_channel::<WsMessage>();
    let (_gui_to_app_tx, gui_to_app_rx) = mpsc::unbounded_channel::<WsMessage>();

    crate::run_app_headless(gui_tx, gui_to_app_rx, Some(password), None, port_override).await
}

/// Run in daemon mode (-D) - background server for remote connections only
/// No console UI, just prints listening ports and handles remote clients
pub async fn run_daemon_server() -> io::Result<()> {
    let mut app = App::new();

    // Load settings from normal settings file
    if let Err(e) = persistence::load_settings(&mut app) {
        eprintln!("Warning: Could not load settings: {}", e);
    }

    // Apply the loaded debug flag to the process-wide gate every is_debug_enabled() call
    // reads. Without this, `debug_enabled=true` in settings.dat was honoured by every other
    // mode but silently ignored in -D: the setting was loaded into app.settings, and every
    // debug_log(is_debug_enabled(), ..) call site still saw the AtomicBool's `false`
    // default, so a daemon could never be asked to produce a debug log at all.
    crate::DEBUG_ENABLED.store(app.settings.debug_enabled, std::sync::atomic::Ordering::Relaxed);

    // Pre-compile action regexes after loading settings
    crate::compile_all_action_regexes(&mut app.settings.actions);

    // Ensure at least one world exists
    app.ensure_has_world();

    // Re-create spell checker with custom dictionary path if configured
    if !app.settings.dictionary_path.is_empty() {
        app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
    }

    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(100);

    // Create WebSocket server state (for client management, no standalone listener)
    let ws_state = if !app.settings.websocket_password.is_empty() {
        let server = WebSocketServer::new(
            &app.settings.websocket_password,
            app.settings.http_port,
            &app.settings.websocket_allow_list,
            app.settings.websocket_whitelisted_host.clone(),
            false, // Not multiuser mode
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
            auth_key: app.ws_auth_key_shared.clone(),
            web_path: app.settings.web_path.clone(),
            ban_list: app.ban_list.clone(),
        }
    } else {
        SecurityGate {
            allow_list: Arc::new(std::sync::RwLock::new(
                crate::websocket::parse_allow_list_csv(&app.settings.websocket_allow_list)
            )),
            whitelisted_host: Arc::new(std::sync::RwLock::new(app.settings.websocket_whitelisted_host.clone())),
            auth_key: app.ws_auth_key_shared.clone(),
            web_path: app.settings.web_path.clone(),
            ban_list: app.ban_list.clone(),
        }
    };

    // Start unified HTTP+WS server if enabled
    if app.settings.http_enabled {
        let has_cert = !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty();
        let web_secure = app.settings.web_secure;

        if web_secure && has_cert {
            // Start HTTPS+WSS
            #[cfg(any(feature = "native-tls-backend", feature = "rustls-backend"))]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
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
                        eprintln!("Warning: Failed to start HTTPS server: {}", e);
                    }
                }
            }
        } else {
            // Start HTTP+WS
            let mut http_server = HttpServer::new(app.settings.http_port);
            match start_http_server(&mut http_server, ws_state.clone(), app.ban_list.clone(), app.gui_theme_colors().to_css_vars(), None, gate.clone()).await {
                Ok(()) => {
                    let protocol = if ws_state.is_some() { "HTTP+WS" } else { "HTTP" };
                    println!("{}: http://0.0.0.0:{}", protocol, app.settings.http_port);
                    app.http_server = Some(http_server);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to start HTTP server: {}", e);
                }
            }
        }
    }

    // Check if any servers are running
    if app.http_server.is_none() && app.https_server.is_none() {
        eprintln!("Error: No servers started. Enable HTTP in settings.");
        eprintln!("Use /web command to configure, or edit ~/.clay/settings.dat");
        return Ok(());
    }

    // Show allow list if configured (helps debug connection rejections)
    if !app.settings.websocket_allow_list.is_empty() {
        println!("Allow list: {}", app.settings.websocket_allow_list);
    }

    println!("Daemon running. Press Ctrl+C to stop.");

    // Conditional timer: sleep far-future when no processes, reset to 1s when needed
    const FAR_FUTURE: std::time::Duration = std::time::Duration::from_secs(86400);
    let process_tick_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(process_tick_sleep);

    // MUD keepalive (send an idle-connection NOP/Custom/Generic ping per world's
    // KeepAliveType, same as run_app/run_app_headless) and auto-reconnect (services
    // World.reconnect_at, scheduled by handle_disconnected). `-D` mode used to have
    // neither: an idle MUD connection on a mobile network can go silently dead (no RST,
    // so read() never errors) and Clay never noticed because it never sent anything to
    // provoke the failure, nor did it ever retry a connection that did fail (D-Termux-lines
    // investigation).
    const KEEPALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
    let mut keepalive_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    let reconnect_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(reconnect_sleep);

    // mud-status-display.md Job 2 (plan D4): same conditional dormant-timer idiom as
    // process_tick_sleep/reconnect_sleep above, at the ~150ms cadence run_app/
    // run_app_headless already use for their own (pre-existing) prompt_check_sleep - `-D`
    // mode has never had that timer, so there's nothing to reuse here; this is the same
    // cadence, not a duplicate concept. Stays dormant except while some world's stats are
    // dirty (see the bottom-of-loop rearm below).
    let stats_flush_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(stats_flush_sleep);

    // Main event loop - handles MUD connections and WebSocket messages
    loop {
        #[cfg(all(unix, not(target_os = "android")))]
        reap_zombie_children();

        tokio::select! {
            // TF repeat process tick — only fires when processes exist
            _ = &mut process_tick_sleep => {
                let now = std::time::Instant::now();
                let mut to_remove = vec![];
                let process_count = app.tf_engine.processes.len();
                for i in 0..process_count {
                    if app.tf_engine.processes[i].on_prompt { continue; }
                    if app.tf_engine.processes[i].next_run <= now {
                        let cmd = app.tf_engine.processes[i].command.clone();
                        let process_world = app.tf_engine.processes[i].world.clone();
                        app.sync_tf_world_info();
                        let result = app.tf_engine.execute(&cmd);
                        let target_idx = if let Some(ref wname) = process_world {
                            if wname.is_empty() {
                                Some(app.current_world_index)
                            } else {
                                app.find_world_index(wname)
                            }
                        } else {
                            Some(app.current_world_index)
                        };
                        let world_idx = target_idx.unwrap_or(app.current_world_index);
                        match result {
                            tf::TfCommandResult::SendToMud(text) => {
                                if let Some(idx) = target_idx {
                                    app.send_to_world(idx, text);
                                }
                            }
                            tf::TfCommandResult::Success(Some(msg)) => {
                                app.emit_client_text(world_idx, &msg, true);
                            }
                            tf::TfCommandResult::Error(err) => {
                                app.emit_tf_error(world_idx, &err, true);
                            }
                            tf::TfCommandResult::RepeatProcess(process) => {
                                app.register_repeat_process(process);
                            }
                            tf::TfCommandResult::NotTfCommand => {
                                // Plain text command - send to MUD (captured via
                                // send_to_world - a /repeat body is exactly the
                                // "/repeat batches" case /recall -i now covers).
                                if let Some(idx) = target_idx {
                                    app.send_to_world(idx, cmd.clone());
                                }
                            }
                            _ => {}
                        }
                        let interval = app.tf_engine.processes[i].interval;
                        app.tf_engine.processes[i].next_run += interval;
                        if let Some(ref mut rem) = app.tf_engine.processes[i].remaining {
                            *rem = rem.saturating_sub(1);
                            if *rem == 0 {
                                to_remove.push(i);
                            }
                        }
                    }
                }
                for i in to_remove.into_iter().rev() {
                    app.tf_engine.processes.remove(i);
                }
                // Re-arm: tick again in 1s if processes remain
                if !app.tf_engine.processes.is_empty() {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(1));
                } else {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }
            Some(event) = event_rx.recv() => {
                match event {
                    AppEvent::ServerData(ref world_name, bytes) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            // Use shared server data processing (same as console mode)
                            let commands = app.process_server_data(
                                world_idx,
                                &bytes,
                                24, // Default console height for daemon mode
                                200, // Fallback width — actual width computed from connected clients
                                true, // is_daemon_mode
                            );

                            // Execute any triggered commands
                            let saved_current_world = app.current_world_index;
                            app.current_world_index = world_idx;
                            for cmd in commands {
                                if cmd.starts_with('/') {
                                    // Unified command system - route through TF parser
                                    app.sync_tf_world_info();
                                    match app.tf_engine.execute(&cmd) {
                                        tf::TfCommandResult::SendToMud(text) => {
                                            app.send_to_world(world_idx, text);
                                        }
                                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                            // Handle Clay-specific commands in daemon mode
                                            let parsed = parse_command(&clay_cmd);
                                            if let Command::Send { text, target_world, .. } = &parsed {
                                                let target_idx = if let Some(w) = target_world {
                                                    app.find_world_index(w)
                                                } else {
                                                    Some(world_idx)
                                                };
                                                if let Some(idx) = target_idx {
                                                    app.send_to_world(idx, text.clone());
                                                }
                                            } else {
                                                app.handle_triggered_notify_or_say(parsed, world_idx);
                                            }
                                        }
                                        tf::TfCommandResult::RepeatProcess(process) => {
                                            app.register_repeat_process(process);
                                        }
                                        _ => {}
                                    }
                                } else {
                                    // Plain text - send to MUD (captured via send_to_world)
                                    app.send_to_world(world_idx, cmd);
                                }
                            }
                            app.current_world_index = saved_current_world;
                        }
                    }
                    AppEvent::Disconnected(ref world_name, conn_id) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            // Ignore stale disconnect from a previous connection
                            if conn_id != app.worlds[world_idx].connection_id {
                                continue;
                            }
                            // Shared with run_app/run_app_headless — also fires the TF
                            // DISCONNECT hook, pushes a "Disconnected." line, tracks
                            // unseen_lines, and (the part `-D` mode was missing entirely)
                            // schedules world.reconnect_at from auto_reconnect_secs. The
                            // reconnect_sleep re-arm below is what actually services it.
                            app.handle_disconnected(world_idx);
                            if let Some(next) = app.next_reconnect_instant() {
                                let dur = next.saturating_duration_since(std::time::Instant::now());
                                reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                            }
                        }
                    }
                    AppEvent::WsClientMessage(client_id, msg) => {
                        // Check if this is an AuthRequest (client just authenticated)
                        if let WsMessage::AuthRequest { ref resume, ref resume_epochs, .. } = *msg {
                            // Send initial state after successful authentication. Skips the
                            // history for worlds the client still holds - see
                            // App::build_initial_state_with_resume.
                            let initial_state = app.build_initial_state_with_resume(
                                app.display_owner_id(client_id), resume_epochs);
                            app.log_initial_state_resume_skip(resume, resume_epochs, &initial_state);
                            app.ws_send_initial_state_and_mark(client_id, initial_state);
                            // Resume-driven replay (PROTOCOL-ROADMAP.md Step 2): reuse the
                            // exact same gap-fill logic RequestScrollback{after_seq} already
                            // uses, once per named world, instead of waiting for the client
                            // to notice and ask for it. Mirrors main.rs's
                            // handle_ws_auth_initial_state for the master-WS path.
                            if !resume.is_empty() {
                                if let Some(ref server) = app.ws_server {
                                    server.record_acked_seq(client_id, resume);
                                }
                                // Mirrors the in-memory scrollback ring's cap (MAX_LINES,
                                // main.rs) so a single replay always covers the entire ring.
                                const RESUME_REPLAY_MAX: usize = 10_000;
                                for &(world_index, last_seq) in resume {
                                    // request_id: Some(0) is the reserved value marking a
                                    // server-initiated unprompted resume replay (see
                                    // ScrollbackLines' doc comment in websocket.rs).
                                    app.handle_request_scrollback(client_id, world_index, RESUME_REPLAY_MAX, None, Some(last_seq), Some(0));
                                }
                            }
                        } else {
                            handle_daemon_ws_message(&mut app, client_id, *msg, &event_tx).await;
                        }
                    }
                    AppEvent::WsClientConnected(_client_id) => {
                        // Client connected but not yet authenticated - nothing to do
                    }
                    AppEvent::WsClientDisconnected(_client_id) => {
                        // Client disconnected, nothing special to do
                    }
                    AppEvent::SystemMessage(msg) => {
                        // Print system messages (including connection rejections) to console
                        println!("{}", msg);
                    }
                    // Consolidated telnet dispatch (plan Phase 2, Step 2.7). Before this,
                    // -D mode had NO arm at all for TelnetDetected/WontEchoSeen/
                    // NawsRequested/TtypeRequested/GmcpNegotiated/MsdpNegotiated/
                    // GmcpReceived/MsdpReceived - they fell into the wildcard below and
                    // were silently dropped (finding 2's drift on the dispatch side,
                    // worse here than anywhere else: only CharsetRequested had an arm).
                    // Routing through handle_telnet_event fixes all of them at once.
                    AppEvent::Telnet(ref target, ref ev) => {
                        match target {
                            TelnetTarget::World(world_name) => {
                                if let Some(world_idx) = app.find_world_index(world_name) {
                                    app.handle_telnet_event(world_idx, ev);
                                }
                            }
                            // `-D` is single-user (see connect_daemon_world) and never
                            // constructs a Multiuser target itself, but route it anyway
                            // (plan Job 9) rather than silently drop it, exactly like
                            // main.rs's equivalent arms - the correct behaviour if this
                            // ever changes, not a placeholder.
                            TelnetTarget::Multiuser { world_index, username } => {
                                app.handle_multiuser_telnet_event(*world_index, username.clone(), ev);
                            }
                        }
                    }
                    AppEvent::ApiLookupResult(client_id, world_index, result, cursor_start) => {
                        match result {
                            Ok(text) => app.ws_send_to_client(client_id, WsMessage::SetInputBuffer { text, cursor_start }),
                            Err(e) => app.ws_send_to_client(client_id, WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: e,
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            }),
                        }
                    }
                    AppEvent::RemoteListResult(requesting_client_id, world_index, lines) => {
                        app.remote_ping_responses = None;
                        for line in &lines {
                            app.ws_send_to_client(requesting_client_id, WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: line.clone(),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });
                        }
                    }
                    AppEvent::Sigusr1Received => {
                        #[cfg(all(unix, not(target_os = "android")))]
                        {
                            app.ws_broadcast(WsMessage::ServerReloading);
                            // MCCP2 hot-reload drain (job 2 of 2 - see CLAUDE.md's
                            // hot-reload notes): defer instead of exec'ing now if any
                            // connected world is compressed - the bottom-of-loop poll
                            // below fires the real exec once every drained world's stream
                            // ends, or the drain deadline passes.
                            if !app.should_defer_reload_for_mccp2() {
                                crate::exec_reload(&mut app)?;
                                return Ok(());
                            }
                        }
                    }
                    AppEvent::WsAuthKeyValidation(client_id, msg, client_ip, challenge) => {
                        // Mirrors the console/GUI dispatch in main.rs: validate the auth key,
                        // send AuthResponse, and (on success) mark the client authenticated and
                        // send InitialState. `-D` mode now has its own reconnect_sleep timer
                        // (see run_daemon_server) driven by World.reconnect_at directly, so
                        // unlike console/GUI it doesn't need the separate
                        // app.web_reconnect_needed/trigger_web_reconnects() nudge — left unread
                        // here deliberately, it's harmless, just unused in this mode.
                        app.handle_ws_auth_key_validation(client_id, *msg, &client_ip, &challenge);
                    }
                    AppEvent::WsKeyRequest(client_id) => {
                        app.handle_ws_key_request(client_id);
                    }
                    AppEvent::WsKeyRevoke(_client_id, key) => {
                        app.handle_ws_key_revoke(&key);
                    }
                    AppEvent::ImportResult(client_id, addr, result) => {
                        app.handle_import_result(client_id, addr, result);
                    }
                    _ => {}
                }
            }

            // MUD keepalive timer (mirrors run_app_headless in main.rs): pokes any world
            // idle >= KEEPALIVE_INTERVAL with a NOP/Custom/Generic keepalive per its
            // KeepAliveType, so a connection gone silently dead (no RST — read() never
            // errors) gets a chance to be noticed rather than just looking connected
            // forever with no more lines ever arriving.
            _ = keepalive_interval.tick() => {
                // Reap long-disconnected WS clients' view-state entries (WS_VIEWER_GRACE);
                // piggybacking on this existing once-a-minute tick rather than a new timer.
                app.reap_stale_ws_client_worlds();
                for world in &mut app.worlds {
                    if world.connected {
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
                                        debug_log(is_debug_enabled(), &format!("keepalive: sent NOP to world '{}'", world.name));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Custom => {
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let idler_tag = format!("###_idler_message_{}_###", rand_num);
                                        let cmd = world.settings.keep_alive_cmd
                                            .replace("##rand##", &idler_tag);
                                        let _ = tx.try_send(WriteCommand::Text(cmd));
                                        debug_log(is_debug_enabled(), &format!("keepalive: sent Custom keepalive to world '{}'", world.name));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Generic => {
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let cmd = format!("help commands ###_idler_message_{}_###", rand_num);
                                        let _ = tx.try_send(WriteCommand::Text(cmd));
                                        debug_log(is_debug_enabled(), &format!("keepalive: sent Generic keepalive to world '{}'", world.name));
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
                    // Collect first, emit after: the notice goes out through `app`
                    // (push_and_broadcast_line + WorldDisconnected), which can't run while
                    // `app.worlds` is mutably borrowed here. This is the daemon copy - the
                    // one Android attaches to - and it previously broadcast nothing at all,
                    // so the client kept showing the world connected after the proxy died.
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

            // Auto-reconnect timer (mirrors run_app_headless): services World.reconnect_at,
            // scheduled by handle_disconnected above.
            _ = &mut reconnect_sleep => {
                let now = std::time::Instant::now();
                let to_reconnect: Vec<String> = app.worlds.iter()
                    .filter(|w| w.reconnect_at.map(|t| t <= now).unwrap_or(false))
                    .map(|w| w.name.clone())
                    .collect();
                for world_name in to_reconnect {
                    if let Some(idx) = app.find_world_index(&world_name) {
                        app.worlds[idx].reconnect_at = None;
                        if !app.worlds[idx].connected && app.worlds[idx].settings.has_connection_settings() {
                            let settings = app.worlds[idx].settings.clone();
                            app.worlds[idx].connection_id += 1;
                            let connection_id = app.worlds[idx].connection_id;
                            let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                            app.emit_reconnect_status(idx, &format!("Connecting to {}:{}{}...", settings.hostname, settings.port, ssl_msg));
                            // skip_auto_login=true: handle_connection_success sends auto-login itself.
                            match connect_daemon_world(
                                idx, world_name.clone(), &settings, event_tx.clone(), connection_id, true,
                                app.settings.tls_proxy_enabled,
                            ).await {
                                Some((cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path)) => {
                                    app.handle_connection_success(&world_name, cmd_tx, socket_fd, is_tls);
                                    if let Some(new_idx) = app.find_world_index(&world_name) {
                                        app.worlds[new_idx].proxy_pid = proxy_pid;
                                        app.worlds[new_idx].proxy_socket_path = proxy_socket_path;
                                        app.emit_reconnect_status(new_idx, "Connected!");
                                    }
                                }
                                None => {
                                    if let Some(current_idx) = app.find_world_index(&world_name) {
                                        let secs = app.worlds[current_idx].settings.auto_reconnect_secs;
                                        if secs > 0 {
                                            app.worlds[current_idx].reconnect_at = Some(
                                                std::time::Instant::now() + std::time::Duration::from_secs(secs as u64)
                                            );
                                            app.emit_reconnect_status(current_idx, &format!("Connection failed. Reconnecting in {} seconds...", secs));
                                        } else {
                                            app.emit_reconnect_status(current_idx, "Connection failed.");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Re-arm timer for next scheduled reconnect
                if let Some(next) = app.next_reconnect_instant() {
                    let dur = next.saturating_duration_since(std::time::Instant::now());
                    reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                } else {
                    reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }

            // Stats flush tick — mud-status-display.md Job 2 (plan D4). Only fires while
            // some world's stats are dirty (see the bottom-of-loop rearm below).
            _ = &mut stats_flush_sleep => {
                app.flush_dirty_stats();
                stats_flush_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
            }
        }

        // mud-status-display.md Job 2 (plan D4): pull stats_flush_sleep's deadline in to
        // ~150ms out if some world went dirty this iteration (a GMCP/MSDP update via
        // AppEvent::Telnet, or a clear on disconnect) and nothing sooner is already
        // scheduled. Never push the deadline *later* - a steady stream of updates must
        // still flush roughly every 150ms instead of debouncing forever.
        if app.worlds.iter().any(|w| w.protocol.stats_dirty)
            && stats_flush_sleep.deadline() > tokio::time::Instant::now() + std::time::Duration::from_millis(150)
        {
            stats_flush_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_millis(150));
        }

        // Activate process tick sleep if processes were added during this iteration
        if !app.tf_engine.processes.is_empty()
            && process_tick_sleep.deadline() > tokio::time::Instant::now() + std::time::Duration::from_secs(2)
        {
            process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(1));
        }

        // MCCP2 hot-reload drain (job 2 of 2 - see CLAUDE.md's hot-reload notes): fire a
        // reload deferred by should_defer_reload_for_mccp2 (the Sigusr1Received arm above)
        // once every drained world's stream has ended, or the deadline passes - see
        // mccp2_reload_ready_to_exec's doc comment. Checked once per loop pass (a cheap
        // no-op whenever no reload is pending) rather than at the trigger site itself,
        // exactly like the process-tick/stats-flush rearms just above.
        #[cfg(all(unix, not(target_os = "android")))]
        if app.mccp2_reload_ready_to_exec() {
            crate::exec_reload(&mut app)?;
            return Ok(());
        }
    }
}

/// Handle WebSocket message in daemon mode. Thin wrapper around
/// `handle_daemon_ws_message_impl` that also refreshes `tf_bound_keys_json` afterward
/// (TF-parity plan Job 22a/P2.7 - see `App::refresh_tf_bound_keys_if_changed`'s own doc
/// comment). This function is also used outside `-D`/`--multiuser` proper: main.rs's
/// GUI/headless run loops (`run_master_webgui`, `run_app_headless`) dispatch every WS
/// message through it too, so this one wrapper covers every single-user WS/GUI/web client
/// path that isn't the interactive console's own `handle_ws_send_command`/
/// `apply_pending_tf_console_ops` (main.rs/commands.rs).
pub async fn handle_daemon_ws_message(
    app: &mut App,
    client_id: u64,
    msg: WsMessage,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    handle_daemon_ws_message_impl(app, client_id, msg, event_tx).await;
    app.refresh_tf_bound_keys_if_changed();
}

async fn handle_daemon_ws_message_impl(
    app: &mut App,
    client_id: u64,
    msg: WsMessage,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    // Log every incoming command for diagnostics
    if let WsMessage::SendCommand { ref command, .. } = msg {
        crate::debug_log(is_debug_enabled(), &format!("DAEMON_CMD: client_id={} command={:?}", client_id, command));
    }
    // Auto-resume a paused session when the user interacts (mirrors "exit more mode by
    // acting") - was previously master-WS-only despite daemon sharing the same per-client
    // pause tracking (T38).
    app.auto_resume_if_user_action(client_id, &msg);
    match msg {
        // TF-parity plan Job 22a/P2.7: same handling as the master WS's own
        // `App::handle_ws_client_msg` arm (main.rs) - resolve `key` the same way the
        // console does (`chords::resolve_bound_command`), then run the result exactly like
        // a typed `SendCommand` by recursing into this same function (mirrors the existing
        // `WorldConnectHostPort` followup recursion further down). This is what makes
        // `RunKeyBinding` work for GUI/headless single-user modes too, not just the
        // interactive TUI console - `run_master_webgui`/`run_app_headless` dispatch every
        // WS message through `handle_daemon_ws_message`.
        WsMessage::RunKeyBinding { key, kbnum } => {
            if let Some(cmd) = crate::chords::resolve_bound_command(app, &key) {
                let world_index = app.ws_client_worlds.get(&client_id)
                    .map(|s| s.world_index)
                    .unwrap_or(app.current_world_index);
                // See main.rs's identical `RunKeyBinding` handler for why this swaps
                // through `app.input.kbnum` rather than writing the TF `%kbnum` global
                // directly - `sync_tf_world_info()` (called by the recursive `SendCommand`
                // dispatch below) unconditionally overwrites it from there.
                let prev_kbnum = app.input.kbnum;
                app.input.kbnum = kbnum;
                Box::pin(handle_daemon_ws_message_impl(
                    app, client_id, WsMessage::SendCommand { world_index, command: cmd }, event_tx,
                )).await;
                app.input.kbnum = prev_kbnum;
                app.tf_engine.unset_global("kbnum");
            }
            // Unknown key (stale client-side cache, or a race with an /unbind) - ignore
            // silently, per the plan's own ruling.
        }
        WsMessage::SendCommand { world_index, command } => {
            // Determine the current world name for action world-scoping.
            let world_name = app.worlds.get(world_index).map(|w| w.name.clone()).unwrap_or_default();
            // Rewrite slash-less action invocations: "common" → "/common"
            // Only rewrites if an action eligible for the current world exists.
            let command = rewrite_slashless_action(&command, &app.settings.actions, &world_name)
                .unwrap_or(command);
            // Use shared command parsing (same as console mode)
            let parsed = parse_command(&command);

            // Reset more-mode counter when user sends a command
            if world_index < app.worlds.len() {
                app.worlds[world_index].lines_since_pause = 0;
                app.worlds[world_index].last_user_command_time = Some(std::time::Instant::now());
                // Also clear paused flag if no pending lines
                if app.worlds[world_index].pending_lines.is_empty() {
                    app.worlds[world_index].paused = false;
                }
            }

            match parsed {
                Command::ActionCommand { name, args } => {
                    // Execute action if it exists (respects the action's world field).
                    if let Some(action) = find_invocable_action(&app.settings.actions, &name, &world_name) {
                        if !action.enabled {
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: format!("Action '{}' is disabled.", action.name),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });
                        } else {
                            let mut sent_to_server = false;
                            for cmd in action_commands_to_run(action, &args) {
                                // Unified command system - route through TF parser
                                if cmd.starts_with('/') {
                                    match app.tf_engine.execute(&cmd) {
                                        tf::TfCommandResult::Success(Some(msg)) => {
                                            app.emit_client_text(world_index, &msg, true);
                                        }
                                        tf::TfCommandResult::Success(None) => {}
                                        tf::TfCommandResult::Error(err) => {
                                            app.emit_tf_error(world_index, &err, true);
                                        }
                                        tf::TfCommandResult::SendToMud(text) => {
                                            // send_to_world captures via capture_sent_line -
                                            // an action's command list is the "sent by
                                            // triggers/actions" case /recall -i now covers.
                                            if app.send_to_world(world_index, text) {
                                                sent_to_server = true;
                                            }
                                        }
                                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                            app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: clay_cmd });
                                        }
                                        tf::TfCommandResult::Recall(opts) => {
                                            app.emit_recall(&opts, world_index, true);
                                        }
                                        tf::TfCommandResult::RepeatProcess(process) => {
                                            app.register_repeat_process(process);
                                        }
                                        _ => {}
                                    }
                                } else {
                                    // Plain text - send to MUD server (captured via
                                    // send_to_world too).
                                    if app.send_to_world(world_index, cmd) {
                                        sent_to_server = true;
                                    }
                                }
                            }
                            if sent_to_server && world_index < app.worlds.len() {
                                app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                            }
                        }
                    } else {
                        // No matching action - try TF engine (handles /recall, /set, /echo, etc.)
                        app.sync_tf_world_info();
                        match app.tf_engine.execute(&command) {
                            tf::TfCommandResult::Success(Some(msg)) => {
                                app.emit_client_text(world_index, &msg, true);
                            }
                            tf::TfCommandResult::Success(None) => {}
                            tf::TfCommandResult::Error(err) => {
                                app.emit_tf_error(world_index, &err, true);
                            }
                            tf::TfCommandResult::SendToMud(text) => {
                                // send_to_world() itself captures this via capture_sent_line -
                                // no separate record_user_input call needed (it would
                                // double-record the same command).
                                if app.send_to_world(world_index, text) {
                                    app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                }
                            }
                            tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: clay_cmd });
                            }
                            tf::TfCommandResult::Recall(opts) => {
                                app.emit_recall(&opts, world_index, true);
                            }
                            tf::TfCommandResult::RepeatProcess(process) => {
                                app.register_repeat_process(process);
                            }
                            _ => {
                                app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                    world_index,
                                    data: format!("Unknown command: {}", name),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0, end_seq: None,
                                    flush: false, gagged: false, highlight_colors: Vec::new(),
                                });
                            }
                        }
                    }
                }
                Command::NotACommand { text } => {
                    // SEND hook: typed plain text about to go to the MUD. A matching
                    // non-quiet hook means the original text is NOT sent - see /help
                    // hooks' SEND rule and finding C.10 / plan step P1.9 (this is the
                    // "headless/daemon" half of the same shared-helper wiring the
                    // console loop does in main.rs).
                    if world_index < app.worlds.len() {
                        let suppressed = app.fire_tf_hook(Some(world_index), tf::TfHookEvent::Send, &text, true);
                        if !suppressed && app.send_to_world(world_index, text) {
                            app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                            app.worlds[world_index].prompt.clear();
                        }
                    }
                }
                Command::Edit { .. } | Command::EditList => {
                    // Edit command is handled locally on the client, not on daemon
                    // Send back to client for local execution
                    app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                }
                Command::Tag => {
                    // Toggle MUD tag display (same as F2) - silent, no output
                    app.show_tags = !app.show_tags;
                    // Broadcast to all clients
                    app.ws_broadcast(WsMessage::ShowTagsChanged { show_tags: app.show_tags });
                }
                Command::Dict { .. } | Command::Urban { .. } | Command::Translate { .. } | Command::TinyUrl { .. } => {
                    spawn_api_lookup(event_tx.clone(), client_id, world_index, parsed, app.settings.url_shorteners.clone());
                }
                Command::DictUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /dict <word>",
                        "  Looks up <word> in the dictionary and places the definition in the input buffer.",
                        "  Example: /dict hello",
                    ], true);
                }
                Command::UrbanUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /urban <word>",
                        "  Looks up <word> in Urban Dictionary and places the definition in the input buffer.",
                        "  Example: /urban yeet",
                    ], true);
                }
                Command::TranslateUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /translate <lang> <text>",
                        "  Translates <text> to <lang> and places the result in the input buffer.",
                        "  <lang> can be a code (es, fr, de) or name (spanish, french, german).",
                        "  Example: /translate spanish Hello, how are you?",
                        "  Example: /tr es Hello",
                    ], true);
                }
                Command::TinyUrlUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /url <url>",
                        "  Shortens <url> and places the result in the input buffer.",
                        "  Example: /url https://github.com/c-hudson/clay",
                    ], true);
                }
                Command::HelpTopic { ref topic } => {
                    use crate::popup::definitions::help::get_topic_help;
                    let help_text = if let Some(lines) = get_topic_help(topic) {
                        lines.join("\n")
                    } else {
                        match app.tf_engine.execute(&format!("#help {}", topic)) {
                            crate::tf::TfCommandResult::Success(Some(msg)) => msg,
                            _ => format!("No help available for '{}'", topic),
                        }
                    };
                    app.emit_client_text(world_index, &help_text, true);
                }
                Command::Unknown { cmd } => {
                    // NOMACRO hook - see /help hooks and finding C.10 / plan step P1.9.
                    app.fire_tf_hook(Some(world_index), tf::TfHookEvent::Nomacro, &cmd, true);
                    app.emit_client_text(world_index, &format!("Unknown command: {}", cmd), true);
                }
                Command::Send { text, all_worlds, target_world, world_type, no_newline, run_hook } => {
                    execute_send_command(app, &text, all_worlds, &target_world, &world_type, no_newline, run_hook, world_index, true);
                }
                Command::Log { world, log_input, log_local: _, log_global: _, action } => {
                    execute_log_command(app, &world, log_input, &action, world_index, true);
                }
                Command::Disconnect { world } => {
                    // Uses the same World::clear_connection_state the console/master-WS paths
                    // use (via execute_disconnect_command), instead of an inline field-by-field
                    // copy - see the removed comment's own history: the old inline version here
                    // was missing several fields (skip_auto_login, negotiated_encoding,
                    // telnet_mode, naws_enabled/naws_sent_size, fansi_detect_until/
                    // fansi_login_pending, active_media, timing fields), leaking stale state
                    // into the next connection attempt on this world in daemon mode.
                    execute_disconnect_command(app, &world, world_index, true);
                }
                Command::Flush => {
                    if world_index < app.worlds.len() {
                        let line_count = app.worlds[world_index].output_lines.len();
                        app.worlds[world_index].output_lines.clear();
                        app.worlds[world_index].pending_lines.clear();
                        app.worlds[world_index].scroll_offset = 0;
                        app.worlds[world_index].lines_since_pause = 0;
                        app.worlds[world_index].paused = false;
                        app.ws_broadcast(WsMessage::WorldFlushed { world_index });
                        app.emit_client_text(world_index, &format!("Flushed {} lines from output buffer.", line_count), true);
                    }
                }
                Command::Remote => {
                    spawn_remote_ping_check(app, event_tx.clone(), client_id, world_index);
                }
                Command::RemoteKill { client_id } => {
                    let msg = if let Some(ref ws_server) = app.ws_server {
                        let ip = {
                            let clients = ws_server.clients.read().unwrap();
                            clients.get(&client_id).map(|c| c.ip_address.clone())
                        };
                        if let Some(ip) = ip {
                            let mut clients_mut = ws_server.clients.write().unwrap();
                            clients_mut.remove(&client_id);
                            format!("Disconnected remote client {} ({})", client_id, ip)
                        } else {
                            format!("No client with ID {}.", client_id)
                        }
                    } else {
                        "WebSocket server is not running.".to_string()
                    };
                    app.emit_client_text(world_index, &msg, true);
                }
                Command::RemotePause { client_id: pause_id } => {
                    let msg = match app.ws_toggle_client_paused(pause_id) {
                        Some((new_paused, ip)) => {
                            app.ws_send_to_client(pause_id, WsMessage::PausedState { paused: new_paused });
                            app.broadcast_activity();
                            if new_paused {
                                format!("Paused remote client {} ({}) — activity now visible on other sessions.", pause_id, ip)
                            } else {
                                format!("Resumed remote client {} ({}).", pause_id, ip)
                            }
                        }
                        None => format!("No client with ID {}.", pause_id),
                    };
                    app.emit_client_text(world_index, &msg, true);
                }
                Command::BanList => {
                    let bans = app.ban_list.get_ban_info();
                    if bans.is_empty() {
                        app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                            world_index,
                            data: "No hosts are currently banned.".to_string(),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0, end_seq: None,
                            flush: false, gagged: false, highlight_colors: Vec::new(),
                        });
                    } else {
                        let mut output = String::new();
                        output.push_str("\nBanned Hosts:\n");
                        output.push_str(&"\u{2500}".repeat(70));
                        output.push_str(&format!("\n{:<20} {:<12} {}\n", "Host", "Type", "Last URL/Reason"));
                        output.push_str(&"\u{2500}".repeat(70));
                        output.push('\n');
                        for (ip, ban_type, reason) in &bans {
                            let reason_display = if reason.is_empty() { "(unknown)" } else { reason };
                            output.push_str(&format!("{:<20} {:<12} {}\n", ip, ban_type, reason_display));
                        }
                        output.push_str(&"\u{2500}".repeat(70));
                        output.push_str("\nUse /unban <host> to remove a ban.");
                        app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                            world_index,
                            data: output,
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0, end_seq: None,
                            flush: false, gagged: false, highlight_colors: Vec::new(),
                        });
                    }
                    app.ws_send_to_client(client_id, WsMessage::BanListResponse { bans });
                }
                Command::Unban { host } => {
                    if app.ban_list.remove_ban(&host) {
                        let _ = persistence::save_settings(app);
                        app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                            world_index,
                            data: format!("Removed ban for: {}", host),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0, end_seq: None,
                            flush: false, gagged: false, highlight_colors: Vec::new(),
                        });
                        app.ws_broadcast(WsMessage::BanListResponse { bans: app.ban_list.get_ban_info() });
                        app.ws_send_to_client(client_id, WsMessage::UnbanResult { success: true, host, error: None });
                    } else {
                        app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                            world_index,
                            data: format!("No ban found for: {}", host),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0, end_seq: None,
                            flush: false, gagged: false, highlight_colors: Vec::new(),
                        });
                        app.ws_send_to_client(client_id, WsMessage::UnbanResult { success: false, host, error: Some("No ban found".to_string()) });
                    }
                }
                Command::TestMusic => {
                    let test_notes = crate::generate_test_music_notes();
                    crate::debug_log(is_debug_enabled(), &format!("TESTMUSIC: daemon path, client_id={}, notes={}", client_id, test_notes.len()));
                    app.ws_send_to_client(client_id, WsMessage::AnsiMusic {
                        world_index,
                        notes: test_notes,
                    });
                    app.ws_send_to_client(client_id, WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: "Playing test music (Super Mario Bros)...".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                }
                Command::Notify { message } => {
                    let title = if world_index < app.worlds.len() {
                        app.worlds[world_index].name.clone()
                    } else {
                        "Clay".to_string()
                    };
                    app.ws_broadcast(WsMessage::Notification {
                        title,
                        message: message.clone(),
                    });
                    app.emit_client_text(world_index, &format!("Notification sent: {}", message), true);
                }
                Command::Say { text } => {
                    // Speak text via TTS (console subprocess + broadcast to web clients)
                    tts::speak(&app.tts_backend, &text, app.settings.tts_mode);
                    let clean_text = strip_ansi_codes(&text);
                    app.ws_broadcast(WsMessage::ServerSpeak {
                        text: clean_text,
                        world_index,
                    });
                    app.emit_client_text(world_index, &format!("TTS: {}", text), true);
                }
                Command::Dump => {
                    // Same debug-dump content console/master-WS write (previously this
                    // was a bare 3-column CSV missing all the actual debug state -
                    // paused/pending/encoding/effective-dimensions - that's the whole
                    // point of the command). daemon.rs is the one dispatch path that
                    // already gave user-facing feedback on success/failure, so keep that.
                    let total_lines: usize = app.worlds.iter()
                        .map(|w| w.output_lines.len() + w.pending_lines.len())
                        .sum();
                    match app.write_debug_dump("daemon") {
                        Ok(dump_path) => {
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: format!("Dumped {} lines from {} worlds to {}", total_lines, app.worlds.len(), dump_path.display()),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });
                        }
                        Err(e) => {
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: format!("Failed to create dump file: {}", e),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });
                        }
                    }
                }
                Command::Mssp => {
                    execute_mssp_command(app, world_index, true);
                }
                Command::Stats => {
                    execute_stats_command(app, world_index, true);
                }
                Command::Msdp { verb, target } => {
                    execute_msdp_command(app, world_index, &verb, target.as_deref(), true);
                }
                Command::MsdpUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /msdp <LIST|REPORT|UNREPORT|SEND> [target]",
                        "  LIST targets: COMMANDS, LISTS, CONFIGURABLE_VARIABLES,",
                        "                REPORTABLE_VARIABLES, SENDABLE_VARIABLES, REPORTED_VARIABLES",
                        "  Example: /msdp LIST REPORTABLE_VARIABLES",
                        "  Example: /msdp REPORT HEALTH",
                    ], true);
                }
                Command::Reload => {
                    // Signal the event loop to perform reload
                    crate::debug_log(is_debug_enabled(), "DAEMON: Received /reload command, sending Sigusr1Received event");
                    let _ = event_tx.send(AppEvent::Sigusr1Received).await;
                }
                Command::RemoteAttach { .. } => {
                    // A headless daemon (-D) has no local console/GUI to relaunch into —
                    // /connect only makes sense for an interactive master or client.
                    app.ws_send_to_client(client_id, WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: "/connect is not available in daemon mode.".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                }
                Command::Import { .. } => {
                    // Same reasoning as the WS-bounced-command rejection in main.rs: the
                    // command line can't carry a password/auth-key, so a dedicated
                    // ImportSettings message is required (later plan step).
                    app.ws_send_to_client(client_id, WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: "Use the /import dialog (not the command line) so your password/auth-key aren't sent unprotected.".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                }
                // Commands that execute locally on the client
                Command::Quit | Command::Update { .. } => {
                    app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                }
                // UI popup commands - send back to client for local handling
                Command::Help | Command::Menu | Command::Font | Command::Setup | Command::Web | Command::Actions { .. } |
                Command::WorldsList | Command::WorldSelector | Command::WorldEdit { .. } => {
                    app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                }
                Command::Window { world } => {
                    // Send OpenWindow message to requesting client only — client opens a new browser tab
                    app.ws_send_to_client(client_id, WsMessage::OpenWindow { world });
                }
                Command::Version => {
                    app.ws_send_to_client(client_id, WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: get_version_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                }
                // AddWorld - add or update world definition
                Command::AddWorld { name, host, port, user, password, use_ssl, file } => {
                    execute_add_world_command(app, name, host, port, user, password, use_ssl, file, world_index, true);
                }
                Command::AddWorldDefault { character, password, file } => {
                    execute_add_world_default_command(app, character, password, file, world_index, true);
                }
                Command::RemoveWorld { names } => {
                    execute_remove_world_command(app, &names, world_index, true);
                }
                Command::WorldConnectHostPort { host, port, use_ssl, no_login, background } => {
                    match prepare_world_connect_host_port(app, &host, &port, use_ssl, no_login, background) {
                        Ok(followup) => return Box::pin(handle_daemon_ws_message(
                            app, client_id, WsMessage::SendCommand { world_index, command: followup }, event_tx,
                        )).await,
                        Err(e) => app.emit_client_text(world_index, &e, true),
                    }
                }
                // Connect command - use daemon connection logic
                Command::Connect { .. } => {
                    if world_index < app.worlds.len() && !app.worlds[world_index].connected {
                        // Slack/Discord worlds don't go through connect_daemon_world (that's
                        // MUD-only, telnet/proxy-socket specific) - route them through the
                        // same connect_slack/connect_discord commands.rs already uses for
                        // console/master-WS. Those operate on app.current_world_index, so
                        // temporarily point it at this request's target world and restore
                        // it after, mirroring the WsAsyncAction::Connect delegation pattern
                        // in main.rs (which ultimately calls into the same two functions).
                        let world_type = app.worlds[world_index].settings.world_type.clone();
                        if !matches!(world_type, WorldType::Mud) {
                            let prev_index = app.current_world_index;
                            app.current_world_index = world_index;
                            let connected = match world_type {
                                WorldType::Slack => connect_slack(app, event_tx.clone()).await,
                                WorldType::Discord => connect_discord(app, event_tx.clone()).await,
                                WorldType::Mud => unreachable!(),
                            };
                            app.current_world_index = prev_index;
                            if connected {
                                app.worlds[world_index].was_connected = true;
                                let name = app.worlds[world_index].name.clone();
                                app.ws_broadcast(WsMessage::WorldConnected { world_index, name });
                            }
                            return;
                        }
                        if app.worlds[world_index].settings.has_connection_settings() {
                            let settings = app.worlds[world_index].settings.clone();
                            let world_name = app.worlds[world_index].name.clone();

                            let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: format!("Connecting to {}:{}{}...\n", settings.hostname, settings.port, ssl_msg),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });

                            app.worlds[world_index].connection_id += 1;
                            let skip_login = app.worlds[world_index].skip_auto_login;
                            if let Some((cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path)) = connect_daemon_world(
                                world_index,
                                world_name.clone(),
                                &settings,
                                event_tx.clone(),
                                app.worlds[world_index].connection_id,
                                skip_login,
                                app.settings.tls_proxy_enabled,
                            ).await {
                                app.worlds[world_index].connected = true;
                                // Re-arm the login-capture guard for this fresh connection -
                                // see World::login_capture_guard's doc comment.
                                app.worlds[world_index].login_capture_guard = 6;
                                app.worlds[world_index].command_tx = Some(cmd_tx);
                                app.worlds[world_index].was_connected = true;
                                app.worlds[world_index].skip_auto_login = false;
                                app.worlds[world_index].socket_fd = socket_fd;
                                app.worlds[world_index].is_tls = is_tls;
                                app.worlds[world_index].proxy_pid = proxy_pid;
                                app.worlds[world_index].proxy_socket_path = proxy_socket_path;
                                let now = std::time::Instant::now();
                                app.worlds[world_index].last_send_time = Some(now);
                                app.worlds[world_index].last_receive_time = Some(now);
                                app.ws_broadcast(WsMessage::WorldConnected { world_index, name: world_name });
                            } else {
                                app.worlds[world_index].skip_auto_login = false;
                                app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                    world_index,
                                    data: "Connection failed.\n".to_string(),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0, end_seq: None,
                                    flush: false, gagged: false, highlight_colors: Vec::new(),
                                });
                            }
                        } else {
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: "No connection settings configured for this world.".to_string(),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });
                        }
                    }
                }
                Command::WorldConnectBackground { ref name } => {
                    if let Some(idx) = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                        if !app.worlds[idx].connected && app.worlds[idx].settings.has_connection_settings() {
                            let settings = app.worlds[idx].settings.clone();
                            let world_name = app.worlds[idx].name.clone();
                            app.worlds[idx].connection_id += 1;
                            if let Some((cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path)) = connect_daemon_world(
                                idx, world_name.clone(), &settings, event_tx.clone(),
                                app.worlds[idx].connection_id, false, app.settings.tls_proxy_enabled,
                            ).await {
                                app.worlds[idx].connected = true;
                                app.worlds[idx].login_capture_guard = 6; // see World::login_capture_guard
                                app.worlds[idx].command_tx = Some(cmd_tx);
                                app.worlds[idx].was_connected = true;
                                app.worlds[idx].socket_fd = socket_fd;
                                app.worlds[idx].is_tls = is_tls;
                                app.worlds[idx].proxy_pid = proxy_pid;
                                app.worlds[idx].proxy_socket_path = proxy_socket_path;
                                let now = std::time::Instant::now();
                                app.worlds[idx].last_send_time = Some(now);
                                app.worlds[idx].last_receive_time = Some(now);
                                app.ws_broadcast(WsMessage::WorldConnected { world_index: idx, name: world_name });
                            }
                        }
                    } else {
                        app.emit_client_text(world_index, &format!("World '{}' not found.", name), true);
                    }
                }
                Command::WorldSwitch { ref name } | Command::WorldConnectNoLogin { ref name } => {
                    if let Some(idx) = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                        app.switch_world(idx);
                        app.ws_broadcast(WsMessage::WorldSwitched { new_index: idx });
                        // Also send ExecuteLocalCommand so web clients can switch their local view
                        app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                        // Connect if not connected and has settings
                        if !app.worlds[idx].connected
                            && app.worlds[idx].settings.has_connection_settings()
                        {
                            if matches!(parsed, Command::WorldConnectNoLogin { .. }) {
                                app.worlds[idx].skip_auto_login = true;
                            }
                            let settings = app.worlds[idx].settings.clone();
                            let world_name = app.worlds[idx].name.clone();

                            let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                            app.emit_client_text(idx, &format!("Connecting to {}:{}{}...", settings.hostname, settings.port, ssl_msg), true);

                            app.worlds[idx].connection_id += 1;
                            let skip_login = app.worlds[idx].skip_auto_login;
                            if let Some((cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path)) = connect_daemon_world(
                                idx,
                                world_name.clone(),
                                &settings,
                                event_tx.clone(),
                                app.worlds[idx].connection_id,
                                skip_login,
                                app.settings.tls_proxy_enabled,
                            ).await {
                                app.worlds[idx].connected = true;
                                app.worlds[idx].login_capture_guard = 6; // see World::login_capture_guard
                                app.worlds[idx].command_tx = Some(cmd_tx);
                                app.worlds[idx].was_connected = true;
                                app.worlds[idx].skip_auto_login = false;
                                app.worlds[idx].socket_fd = socket_fd;
                                app.worlds[idx].is_tls = is_tls;
                                app.worlds[idx].proxy_pid = proxy_pid;
                                app.worlds[idx].proxy_socket_path = proxy_socket_path;
                                let now = std::time::Instant::now();
                                app.worlds[idx].last_send_time = Some(now);
                                app.worlds[idx].last_receive_time = Some(now);
                                app.ws_broadcast(WsMessage::WorldConnected { world_index: idx, name: world_name });
                            } else {
                                app.worlds[idx].skip_auto_login = false;
                                app.emit_client_text(idx, "Connection failed.", true);
                            }
                        }
                    } else {
                        app.emit_client_text(world_index, &format!("World '{}' not found.", name), true);
                    }
                }
            }
        }
        WsMessage::ConnectWorld { world_index } => {
            if world_index < app.worlds.len() && !app.worlds[world_index].connected {
                let settings = app.worlds[world_index].settings.clone();
                let world_name = app.worlds[world_index].name.clone();

                // Check if world has connection settings
                if !settings.has_connection_settings() {
                    app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: "Configure host/port in world settings.\n".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                    return;
                }

                let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                    world_index,
                    data: format!("Connecting to {}:{}{}...\n", settings.hostname, settings.port, ssl_msg),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0, end_seq: None,
                    flush: false, gagged: false, highlight_colors: Vec::new(),
                });

                // Attempt connection
                app.worlds[world_index].connection_id += 1;
                let skip_login = app.worlds[world_index].skip_auto_login;
                if let Some((cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path)) = connect_daemon_world(
                    world_index,
                    world_name.clone(),
                    &settings,
                    event_tx.clone(),
                    app.worlds[world_index].connection_id,
                    skip_login,
                    app.settings.tls_proxy_enabled,
                ).await {
                    // Connection succeeded
                    app.worlds[world_index].connected = true;
                    app.worlds[world_index].login_capture_guard = 6; // see World::login_capture_guard
                    app.worlds[world_index].command_tx = Some(cmd_tx);
                    app.worlds[world_index].was_connected = true;
                    app.worlds[world_index].skip_auto_login = false;
                    app.worlds[world_index].socket_fd = socket_fd;
                    app.worlds[world_index].is_tls = is_tls;
                    app.worlds[world_index].proxy_pid = proxy_pid;
                    app.worlds[world_index].proxy_socket_path = proxy_socket_path;
                    let now = std::time::Instant::now();
                    app.worlds[world_index].last_send_time = Some(now);
                    app.worlds[world_index].last_receive_time = Some(now);

                    app.ws_broadcast(WsMessage::WorldConnected { world_index, name: world_name });
                } else {
                    // Connection failed
                    app.worlds[world_index].skip_auto_login = false;
                    app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: "Connection failed.\n".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                }
            }
        }
        WsMessage::DisconnectWorld { world_index } => {
            if world_index < app.worlds.len() && app.worlds[world_index].connected {
                app.worlds[world_index].connected = false;
                app.worlds[world_index].command_tx = None;
                app.ws_broadcast(WsMessage::WorldDisconnected { world_index });
            }
        }
        // Was entirely absent from this handler (T40) - a daemon-mode client that hit a TLS
        // pin mismatch had no way to trust the new certificate and reconnect; the "Trust new
        // certificate" button in the cert-mismatch dialog silently did nothing. Re-pin, then
        // reconnect using the same inline pattern ConnectWorld above uses (daemon.rs doesn't
        // have a shared "connect this world" helper to call here - see Phase C's ConnectWorld
        // consolidation note for that separate, larger duplication).
        WsMessage::TrustCertificate { world_index, host, new_fingerprint } => {
            if world_index < app.worlds.len() {
                persistence::replace_pin(&host, &new_fingerprint);
                app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                    world_index,
                    data: format!("Trusting new certificate for {}, reconnecting...\n", host),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0, end_seq: None,
                    flush: false, gagged: false, highlight_colors: Vec::new(),
                });
                if !app.worlds[world_index].connected {
                    let settings = app.worlds[world_index].settings.clone();
                    let world_name = app.worlds[world_index].name.clone();
                    app.worlds[world_index].connection_id += 1;
                    let skip_login = app.worlds[world_index].skip_auto_login;
                    if let Some((cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path)) = connect_daemon_world(
                        world_index,
                        world_name.clone(),
                        &settings,
                        event_tx.clone(),
                        app.worlds[world_index].connection_id,
                        skip_login,
                        app.settings.tls_proxy_enabled,
                    ).await {
                        app.worlds[world_index].connected = true;
                        app.worlds[world_index].login_capture_guard = 6; // see World::login_capture_guard
                        app.worlds[world_index].command_tx = Some(cmd_tx);
                        app.worlds[world_index].was_connected = true;
                        app.worlds[world_index].skip_auto_login = false;
                        app.worlds[world_index].socket_fd = socket_fd;
                        app.worlds[world_index].is_tls = is_tls;
                        app.worlds[world_index].proxy_pid = proxy_pid;
                        app.worlds[world_index].proxy_socket_path = proxy_socket_path;
                        let now = std::time::Instant::now();
                        app.worlds[world_index].last_send_time = Some(now);
                        app.worlds[world_index].last_receive_time = Some(now);
                        app.ws_broadcast(WsMessage::WorldConnected { world_index, name: world_name });
                    } else {
                        app.worlds[world_index].skip_auto_login = false;
                        app.emit_client_text(world_index, "Connection failed.", true);
                    }
                }
            }
        }
        WsMessage::SwitchWorld { world_index } => {
            if world_index < app.worlds.len() {
                app.current_world_index = world_index;
                app.ws_broadcast(WsMessage::WorldSwitched { new_index: world_index });
            }
        }
        WsMessage::UpdateGlobalSettings { more_mode_enabled, spell_check_enabled, temp_convert_enabled, world_switch_mode, show_tags, debug_enabled, ansi_music_enabled, console_theme, gui_theme, gui_transparency, color_offset_percent, wrapspace, remote_initial_lines, input_height, font_name, font_size, web_font_size_phone, web_font_size_tablet, web_font_size_desktop, web_font_weight, web_font_line_height, web_font_letter_spacing, web_font_word_spacing, ws_allow_list, web_secure, http_enabled, http_port, web_path, ws_enabled: _, ws_port: _, ws_cert_file, ws_key_file, ws_password, tls_proxy_enabled, dictionary_path, mouse_enabled, zwj_enabled, new_line_indicator, tts_mode, tts_speak_mode, scrollback_enabled, log_input_enabled, keyboard_always_visible, tabs, icon_bar } => {
            app.update_global_settings(
                client_id, more_mode_enabled, spell_check_enabled, temp_convert_enabled,
                world_switch_mode, show_tags, debug_enabled, ansi_music_enabled, console_theme,
                gui_theme, gui_transparency, color_offset_percent, wrapspace, remote_initial_lines,
                input_height, font_name, font_size, web_font_size_phone, web_font_size_tablet,
                web_font_size_desktop, web_font_weight, web_font_line_height, web_font_letter_spacing,
                web_font_word_spacing, ws_allow_list, web_secure, http_enabled, http_port, web_path,
                ws_cert_file, ws_key_file, ws_password, tls_proxy_enabled, dictionary_path,
                mouse_enabled, zwj_enabled, new_line_indicator, tts_mode, tts_speak_mode, scrollback_enabled,
                log_input_enabled, keyboard_always_visible, tabs, icon_bar,
            );
        }
        WsMessage::ToggleWorldGmcp { world_index } => {
            if world_index < app.worlds.len() {
                app.worlds[world_index].gmcp_user_enabled = !app.worlds[world_index].gmcp_user_enabled;
                if !app.worlds[world_index].gmcp_user_enabled {
                    app.stop_world_media(world_index);
                }
                app.needs_output_redraw = true;
                app.ws_broadcast(WsMessage::GmcpUserToggled {
                    world_index,
                    enabled: app.worlds[world_index].gmcp_user_enabled,
                });
            }
        }
        // Plan Job 13 (Phase 4, 4.4): folded into one shared function with main.rs's
        // App::handle_ws_client_msg and this file's other copy below - see
        // commands::execute_send_gmcp/execute_send_msdp.
        WsMessage::SendGmcp { world_index, package, data } => {
            execute_send_gmcp(app, world_index, &package, &data);
        }
        WsMessage::SendMsdp { world_index, variable, value } => {
            execute_send_msdp(app, world_index, &variable, &value);
        }
        // Theme editor, keybind editor, and action editor state messages were entirely absent
        // from this handler (T40) - opening any of these editors from a daemon-attached
        // client showed no data, and Save/Add/Delete actions silently did nothing.
        WsMessage::RequestThemeEditorState => {
            let themes_json = app.theme_file.to_json_all();
            let theme_names: Vec<String> = app.theme_file.themes.keys().cloned().collect();
            let active_theme = app.settings.gui_theme.name().to_string();
            let separator_style = app.settings.separator_style.name().to_string();
            app.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                themes_json,
                theme_names,
                active_theme,
                separator_style,
            });
        }
        WsMessage::UpdateThemeColors { theme_name, colors_json } => {
            let base = if theme_name == "light" {
                theme::ThemeColors::light_default()
            } else {
                theme::ThemeColors::dark_default()
            };
            let colors = theme::ThemeColors::from_json(&colors_json, &base);
            app.theme_file.set_theme(&theme_name, colors);
            // If updated theme is the active GUI theme, broadcast CSS vars update
            if theme_name == app.settings.gui_theme.name() {
                let css_vars = app.gui_theme_colors().to_css_vars();
                let colors_json = app.gui_theme_colors().to_json();
                app.ws_broadcast(WsMessage::ThemeCssVarsUpdated {
                    css_vars,
                    colors_json: colors_json.clone(),
                });
                // Also broadcast GlobalSettingsUpdated for GUI clients
                let settings_msg = app.build_global_settings_msg();
                app.ws_broadcast(WsMessage::GlobalSettingsUpdated {
                    settings: settings_msg,
                    input_height: app.input_height,
                });
            }
        }
        WsMessage::AddTheme { name, copy_from } => {
            let base_colors = app.theme_file.get(&copy_from).clone();
            app.theme_file.set_theme(&name, base_colors);
            let themes_json = app.theme_file.to_json_all();
            let theme_names: Vec<String> = app.theme_file.themes.keys().cloned().collect();
            let active_theme = app.settings.gui_theme.name().to_string();
            let separator_style = app.settings.separator_style.name().to_string();
            app.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                themes_json,
                theme_names,
                active_theme,
                separator_style,
            });
        }
        WsMessage::DeleteTheme { name } => {
            app.theme_file.remove_theme(&name);
            let themes_json = app.theme_file.to_json_all();
            let theme_names: Vec<String> = app.theme_file.themes.keys().cloned().collect();
            let active_theme = app.settings.gui_theme.name().to_string();
            let separator_style = app.settings.separator_style.name().to_string();
            app.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                themes_json,
                theme_names,
                active_theme,
                separator_style,
            });
        }
        WsMessage::UpdateSeparatorStyle { style } => {
            app.settings.separator_style = crate::SeparatorStyle::from_name(&style);
            app.needs_output_redraw = true;
            let settings_msg = app.build_global_settings_msg();
            app.ws_broadcast(WsMessage::GlobalSettingsUpdated {
                settings: settings_msg,
                input_height: app.input_height,
            });
        }
        WsMessage::SaveThemeFile => {
            // The separator style lives in settings.dat, not theme.dat, but the theme
            // editor's single Save button owns both.
            let _ = persistence::save_settings(app);
            let content = app.theme_file.generate_file_content();
            let path = clay_config_path("theme.dat");
            match std::fs::write(&path, &content) {
                Ok(_) => {
                    app.ws_send_to_client(client_id, WsMessage::ThemeFileSaved { success: true, error: None });
                }
                Err(e) => {
                    app.ws_send_to_client(client_id, WsMessage::ThemeFileSaved { success: false, error: Some(e.to_string()) });
                }
            }
        }
        WsMessage::RequestActionEditorState => {
            let actions_json = serde_json::to_string(&app.settings.actions).unwrap_or_default();
            let world_names: Vec<&str> = app.worlds.iter().map(|w| w.name.as_str()).collect();
            let world_names_json = serde_json::to_string(&world_names).unwrap_or_default();
            app.ws_send_to_client(client_id, WsMessage::ActionEditorState {
                actions_json,
                world_names_json,
            });
        }
        WsMessage::RequestKeybindEditorState => {
            let bindings_json = app.keybindings.to_json();
            let defaults_json = keybindings::KeyBindings::tf_defaults().to_json();
            let actions_json = keybindings::KeyBindings::actions_json();
            app.ws_send_to_client(client_id, WsMessage::KeybindEditorState {
                bindings_json,
                defaults_json,
                actions_json,
            });
        }
        WsMessage::UpdateKeybindEditorBindings { bindings_json } => {
            app.keybindings = keybindings::KeyBindings::from_json(&bindings_json);
            app.ws_broadcast(WsMessage::KeybindingsUpdated {
                bindings_json: app.keybindings.to_json(),
            });
        }
        WsMessage::SaveKeybindFile => {
            let path = clay_config_path("keybindings.dat");
            match app.keybindings.save(&path) {
                Ok(_) => {
                    app.ws_send_to_client(client_id, WsMessage::KeybindFileSaved { success: true, error: None });
                }
                Err(e) => {
                    app.ws_send_to_client(client_id, WsMessage::KeybindFileSaved { success: false, error: Some(e.to_string()) });
                }
            }
        }
        WsMessage::ResetKeybindDefaults => {
            app.keybindings = keybindings::KeyBindings::tf_defaults();
            let bindings_json = app.keybindings.to_json();
            let defaults_json = keybindings::KeyBindings::tf_defaults().to_json();
            let actions_json = keybindings::KeyBindings::actions_json();
            app.ws_send_to_client(client_id, WsMessage::KeybindEditorState {
                bindings_json,
                defaults_json,
                actions_json,
            });
            app.ws_broadcast(WsMessage::KeybindingsUpdated {
                bindings_json: app.keybindings.to_json(),
            });
        }
        // Note editor (single-user daemon/--local-server: no ownership check
        // needed, same as the theme/keybind editors above; see
        // handle_multiuser_ws_message for the multiuser-owner-checked version).
        WsMessage::RequestNoteEditorState { world_index } => {
            if let Some(world) = app.worlds.get(world_index) {
                app.ws_send_to_client(client_id, WsMessage::NoteEditorState {
                    world_index,
                    world_name: world.name.clone(),
                    notes: world.settings.notes.clone(),
                });
            }
        }
        WsMessage::UpdateNote { world_index, notes } => {
            if let Some(world) = app.worlds.get_mut(world_index) {
                world.settings.notes = notes;
                let has_notes = !world.settings.notes.is_empty();
                let _ = persistence::save_settings(app);
                app.ws_broadcast(WsMessage::NotesChanged { world_index, has_notes });
            }
        }
        // MCP simpleedit reply (plan Job 15): no ownership check needed, same as the
        // note editor above (single-user daemon). `content` is untrusted user-edited
        // text, framed but never interpreted; `reference`/`edit_type` are echoed back
        // exactly as the client received them in McpEditOpen.
        WsMessage::McpEditSet { world_index, reference, edit_type, content } => {
            if let Some(world) = app.worlds.get_mut(world_index) {
                match world.mcp.build_simpleedit_set(&reference, mcp::SimpleEditType::parse(&edit_type), &content) {
                    Some(lines) => {
                        for line in lines {
                            app.send_to_world(world_index, line);
                        }
                    }
                    None => app.emit_client_text(world_index, "Could not send the edited text: the MCP session is no longer available.", false),
                }
            }
        }
        WsMessage::RequestConnectionsList => {
            let current_idx = app.current_world_index;
            const KEEPALIVE_SECS: u64 = 5 * 60;
            let worlds_info: Vec<util::WorldListInfo> = app.worlds.iter().enumerate().map(|(idx, world)| {
                let now = std::time::Instant::now();
                let next_nop = if world.connected {
                    world.last_send_time.map(|t| KEEPALIVE_SECS.saturating_sub(t.elapsed().as_secs()))
                } else {
                    None
                };
                util::WorldListInfo {
                    name: world.name.clone(),
                    connected: world.connected,
                    is_current: idx == current_idx,
                    is_ssl: world.is_tls,
                    is_proxy: world.proxy_pid.is_some(),
                    unseen_lines: world.unseen_lines,
                    last_send_secs: world.last_user_command_time.map(|t| now.duration_since(t).as_secs()),
                    last_recv_secs: world.last_receive_time.map(|t| now.duration_since(t).as_secs()),
                    last_nop_secs: world.last_nop_time.map(|t| now.duration_since(t).as_secs()),
                    next_nop_secs: next_nop,
                    buffer_size: world.output_lines.len() + world.pending_lines.len(),
                }
            }).collect();
            let output = util::format_worlds_list(&worlds_info);
            let lines: Vec<String> = output.lines().map(|s| s.to_string()).collect();
            app.ws_send_to_client(client_id, WsMessage::ConnectionsListResponse { lines });
        }
        // Were entirely absent from this handler (T40) - a daemon-attached client could
        // never view or manage the ban list (the request just silently fell into the
        // catch-all, no reply).
        WsMessage::BanListRequest => {
            let bans = app.ban_list.get_ban_info();
            app.ws_send_to_client(client_id, WsMessage::BanListResponse { bans });
        }
        WsMessage::UnbanRequest { host } => {
            if app.ban_list.remove_ban(&host) {
                let _ = persistence::save_settings(app);
                app.ws_broadcast(WsMessage::BanListResponse { bans: app.ban_list.get_ban_info() });
                app.ws_send_to_client(client_id, WsMessage::UnbanResult { success: true, host, error: None });
            } else {
                app.ws_send_to_client(client_id, WsMessage::UnbanResult { success: false, host, error: Some("No ban found".to_string()) });
            }
        }
        // Was entirely absent from this handler (T40) - a /remote liveness check
        // (spawn_remote_ping_check) against a daemon-attached client would always time out,
        // since the client's PongCheck reply had nowhere to land.
        WsMessage::PongCheck { nonce, acked } => {
            if nonce == app.remote_ping_nonce {
                if let Some(ref responses) = app.remote_ping_responses {
                    if let Ok(mut set) = responses.lock() {
                        set.insert(client_id);
                    }
                }
            }
            // Record the client's per-world delivery ack (PROTOCOL-ROADMAP.md Step 2) so a
            // future resume/backpressure path knows how caught-up it already is.
            if !acked.is_empty() {
                if let Some(ref server) = app.ws_server {
                    server.record_acked_seq(client_id, &acked);
                }
            }
            // ...then check it against what we owe (PROTOCOL-ROADMAP.md Phase C) - same
            // audit the master-WS PongCheck handler runs, since `-D` serves the same
            // single-user clients over the same protocol.
            app.audit_client_acks(client_id);
            // ...and the server's audit of itself (PROTOCOL-ROADMAP.md Phase F). Must be
            // here as well as in the master-WS handler: `-D` is what Android's local-server
            // mode runs, so leaving it out would blind the detector on the platform this
            // whole bug class keeps showing up on.
            app.audit_broadcast_ledger();
        }
        WsMessage::Ping => {
            app.ws_send_to_client(client_id, WsMessage::Pong);
        }
        WsMessage::UpdateViewState { world_index, visible_lines, visible_columns } => {
            // Track client's view state for more-mode threshold calculation
            if world_index < app.worlds.len() {
                let dimensions = app.ws_client_worlds.get(&client_id).and_then(|s| s.dimensions);
                let vc = visible_columns.unwrap_or_else(|| app.ws_client_worlds.get(&client_id).map(|v| v.visible_columns).unwrap_or(0));
                let paused = app.ws_client_worlds.get(&client_id).map(|v| v.paused).unwrap_or(false);
                let visible = app.ws_client_worlds.get(&client_id).map(|v| v.visible).unwrap_or(true);
                app.ws_client_worlds.insert(client_id, ClientViewState { world_index, visible_lines, visible_columns: vc, dimensions, paused, visible, disconnected_at: None });
            }
        }
        // Was entirely absent from this handler (T40) - a daemon-attached client reporting its
        // output dimensions (for NAWS) got no response, so NAWS updates never propagated to
        // the MUD server for daemon-attached clients.
        WsMessage::UpdateDimensions { width, height } => {
            if let Some(state) = app.ws_client_worlds.get_mut(&client_id) {
                let old_dims = state.dimensions;
                state.dimensions = Some((width, height));
                if old_dims != Some((width, height)) {
                    app.send_naws_to_all_worlds();
                }
            }
        }
        WsMessage::MarkWorldSeen { world_index, previous_world_index } => {
            app.handle_mark_world_seen(client_id, world_index, previous_world_index);
        }
        WsMessage::ReleasePending { world_index, count } => {
            app.release_pending_lines(client_id, world_index, count);
        }
        WsMessage::SelectiveFlush { world_index } => {
            app.selective_flush(world_index);
        }
        WsMessage::ReportSeqMismatch { world_index, expected_seq_gt, actual_seq, line_text, source } => {
            // Always-on (not gated behind is_debug_enabled()): only fires on a real
            // connection-level fault, so no log-spam risk, and it was invisible in the field
            // until now (D-Termux-lines investigation).
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("SEQ-MISMATCH", &ip, &format!("[{}] in '{}': expected seq>{}, got seq={}, text={:?}",
                source, world_name, expected_seq_gt, actual_seq,
                line_text.chars().take(80).collect::<String>()));
        }
        WsMessage::ReportDuplicate { world_index, line_seq, max_seq, line_text, source } => {
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("DUPLICATE", &ip, &format!("[{}] in '{}': line_seq={}, max_seq={}, text={:?}",
                source, world_name, line_seq, max_seq,
                line_text.chars().take(200).collect::<String>()));
        }
        WsMessage::ReportOutOfOrder { world_index, line_seq, recovered_count, source } => {
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("OUT-OF-ORDER", &ip, &format!("[{}] in '{}': recovered {} line(s) starting at seq={} that had arrived out of order",
                source, world_name, recovered_count, line_seq));
        }
        WsMessage::ReportGap { world_index, hole_start, hole_end, attempts, source } => {
            // Always-on, same reasoning as the sibling reports above: only fires when a
            // client has genuinely given up on a range of output, which is exactly the
            // event that has been invisible in the field (PROTOCOL-ROADMAP.md Phase F).
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("SEQ-HOLE", &ip, &format!("[{}] in '{}': gave up on seq {}..={} ({} line(s)) after {} gap-fill attempt(s) returned nothing for it",
                source, world_name, hole_start, hole_end,
                hole_end.saturating_sub(hole_start).saturating_add(1), attempts));
        }
        WsMessage::ReportClientLifecycle { event, detail, source } => {
            // Same reasoning as the sibling reports above; see WsMessage::ReportClientLifecycle
            // in websocket.rs for why an Android lifecycle transition is worth a server log line.
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("CLIENT-LIFECYCLE", &ip, &format!("[{}] {}: {}", source, event, detail));
        }
        WsMessage::ClientTypeDeclaration { client_type } => {
            // Update client type in WebSocket server
            if let Some(ref server) = app.ws_server {
                server.set_client_type(client_id, client_type);
            }
        }
        WsMessage::CycleWorld { direction } => {
            app.handle_cycle_world(client_id, &direction);
        }
        WsMessage::RequestScrollback { world_index, count, before_seq, after_seq, request_id } => {
            app.handle_request_scrollback(client_id, world_index, count, before_seq, after_seq, request_id);
        }
        WsMessage::RequestWorldState { world_index } => {
            app.handle_request_world_state(client_id, world_index);
        }
        WsMessage::UpdateActions { actions } => {
            app.handle_update_actions(actions);
        }
        WsMessage::CreateWorld { name } => {
            app.create_world(client_id, &name);
        }
        WsMessage::DeleteWorld { world_index } => {
            app.delete_world(world_index);
        }
        WsMessage::UpdateWorldSettings { world_index, name, hostname, port, user, password, use_ssl, log_enabled, encoding, auto_login, keep_alive_type, keep_alive_cmd, gmcp_packages, auto_reconnect_secs, msp_enabled, mcp_enabled, mccp2_enabled } => {
            app.update_world_settings(
                world_index, name, hostname, port, user, password, use_ssl, log_enabled,
                encoding, auto_login, keep_alive_type, keep_alive_cmd, gmcp_packages, auto_reconnect_secs,
                msp_enabled, mcp_enabled, mccp2_enabled,
            );
        }
        WsMessage::CalculateNextWorld { current_index } => {
            let next_idx = app.calculate_next_world_from(current_index);
            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: next_idx });
        }
        WsMessage::CalculatePrevWorld { current_index } => {
            let prev_idx = app.calculate_prev_world_from(current_index);
            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: prev_idx });
        }
        WsMessage::CalculateOldestPending { current_index } => {
            let oldest_idx = app.calculate_oldest_pending_world_from(current_index);
            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: oldest_idx });
        }
        // /import export side: mirrors the handler in main.rs's App::handle_ws_message —
        // needed here too since --local-server (Android on-device mode) and -D (run_daemon_server)
        // both dispatch through this function instead. See plan `i-d-like-to-make-snuggly-rain.md`.
        WsMessage::RequestSettingsExport => {
            let settings_dat = persistence::serialize_settings_for_export(app);
            let theme_dat = app.theme_file.generate_file_content();
            let keybindings_dat = app.keybindings.to_dat_string();
            app.ws_send_to_client(client_id, WsMessage::SettingsExport { settings_dat, theme_dat, keybindings_dat });
        }
        // /import trigger side: mirrors the handler in main.rs's App::handle_ws_message.
        // Result comes back via AppEvent::ImportResult, handled by both callers of this
        // function's event loops (run_app_headless in main.rs, run_daemon_server in this module).
        WsMessage::ImportSettings { addr, password, auth_key, allow_insecure } => {
            spawn_import_settings(event_tx.clone(), client_id, addr, password, auth_key, allow_insecure);
        }
        // Full state resync. Needed here too since --local-server (Android on-device mode)
        // and -D (run_daemon_server) both dispatch through this function instead of
        // main.rs's - without an arm here the message silently falls into the catch-all
        // below: no reply, no error (web/Android ping the server on visibility change and
        // call RequestState when the connection looks stale).
        WsMessage::RequestState => {
            app.handle_request_state(client_id);
        }
        _ => {}
    }
}

/// Install every user's credential onto a `WebSocketServer`, respecting
/// `password_is_hash` (C1, security remediation) so a value already hashed by a prior
/// `ChangePassword` (see `User::password_is_hash`) isn't hashed a second time — that
/// would silently lock the user out on every subsequent login. Shared by multiuser
/// server startup (mirroring a settings reload/restart after a password change) and
/// exercised directly by tests below.
fn install_user_credentials(server: &WebSocketServer, users: &[User]) {
    for user in users {
        if user.password_is_hash {
            server.set_user_password_hash(&user.name, user.password.clone());
        } else {
            server.add_user(&user.name, &user.password);
        }
    }
}

/// Run in multiuser server mode - web interface only, no console
pub async fn run_multiuser_server() -> io::Result<()> {
    let mut app = App::new();
    app.multiuser_mode = true;

    // Load multiuser settings from separate file
    let settings_path = get_multiuser_settings_path();
    if !settings_path.exists() {
        println!("Multiuser settings file not found: {}", settings_path.display());
        print!("Would you like to create a sample configuration? (y/n): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim().eq_ignore_ascii_case("y") || input.trim().eq_ignore_ascii_case("yes") {
            // Create sample multiuser configuration
            let sample_config = r#"[global]
http_enabled=true
http_port=9000

[user:star]
password=xyzzy

[world:ascii:star]
world_type=mud
hostname=teenymush.dynu.net
port=4096
use_ssl=false
encoding=utf8
auto_connect_type=Connect
keep_alive_type=Generic
"#;
            std::fs::write(&settings_path, sample_config)?;
            println!("Created sample configuration at: {}", settings_path.display());
            println!("Default user: star, password: xyzzy");
            println!("IMPORTANT: Change the user password before production use!");
            println!();
        } else {
            println!("Multiuser mode requires a configuration file.");
            println!("Create {} with [user:NAME] and [world:NAME:OWNER] sections.", settings_path.display());
            return Ok(());
        }
    }

    if let Err(e) = load_multiuser_settings(&mut app) {
        eprintln!("Error loading multiuser settings: {}", e);
        return Ok(());
    }

    // Pre-compile action regexes after loading settings
    crate::compile_all_action_regexes(&mut app.settings.actions);

    // Validate: must have at least one user
    if app.users.is_empty() {
        eprintln!("Error: No users defined in multiuser settings.");
        eprintln!("Add [user:NAME] sections to {}", settings_path.display());
        return Ok(());
    }

    // Validate: all worlds must have owners
    for world in &app.worlds {
        if world.owner.is_none() {
            eprintln!("Error: World '{}' has no owner.", world.name);
            eprintln!("Use [world:{}:OWNERNAME] format in settings file.", world.name);
            return Ok(());
        }
    }

    // Validate: all actions must have owners
    for action in &app.settings.actions {
        if action.owner.is_none() {
            eprintln!("Error: Action '{}' has no owner.", action.name);
            eprintln!("Use [action:{}:OWNERNAME] format in settings file.", action.name);
            return Ok(());
        }
    }

    println!("Starting multiuser server...");
    println!("Users: {}", app.users.iter().map(|u| u.name.as_str()).collect::<Vec<_>>().join(", "));
    println!("Worlds: {}", app.worlds.iter().map(|w| format!("{} ({})", w.name, w.owner.as_ref().unwrap())).collect::<Vec<_>>().join(", "));

    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(100);

    // Create WebSocket server state (for client management, no standalone listener)
    let server = WebSocketServer::new(
        &app.settings.websocket_password,
        app.settings.http_port,
        &app.settings.websocket_allow_list,
        app.settings.websocket_whitelisted_host.clone(),
        app.multiuser_mode,
        app.ban_list.clone(),
    );

    // Add user credentials to the WebSocket server for multiuser authentication.
    install_user_credentials(&server, &app.users);

    let gate = SecurityGate {
        allow_list: server.allow_list.clone(),
        whitelisted_host: server.whitelisted_host.clone(),
        auth_key: app.ws_auth_key_shared.clone(),
        web_path: app.settings.web_path.clone(),
        ban_list: app.ban_list.clone(),
    };

    let ws_state = Arc::new(server.connection_state(event_tx.clone()));
    app.ws_server = Some(server);

    // Start unified HTTP+WS server
    {
        let mut http_server = HttpServer::new(app.settings.http_port);
        match start_http_server(&mut http_server, Some(ws_state.clone()), app.ban_list.clone(), app.gui_theme_colors().to_css_vars(), None, gate.clone()).await {
            Ok(()) => {
                println!("HTTP+WS: http://0.0.0.0:{}", app.settings.http_port);
                app.http_server = Some(http_server);
            }
            Err(e) => {
                eprintln!("Warning: Failed to start HTTP+WS server: {}", e);
                return Ok(());
            }
        }
    }

    println!("Multiuser server running. Press Ctrl+C to stop.");

    // Conditional timer: sleep far-future when no processes, reset to 1s when needed
    const FAR_FUTURE_MU: std::time::Duration = std::time::Duration::from_secs(86400);
    let process_tick_sleep = tokio::time::sleep(FAR_FUTURE_MU);
    tokio::pin!(process_tick_sleep);

    // mud-status-display.md Job 2 (plan D4): same conditional dormant-timer idiom as
    // process_tick_sleep above, at the ~150ms cadence run_app/run_app_headless already use
    // for prompt_check_sleep - multiuser has never had that timer, so this is the same
    // cadence, not a duplicate concept. GMCP/MSDP ingestion isn't wired into multiuser's
    // per-user connections yet (see connect_multiuser_world's doc comment), so nothing marks
    // a world dirty here today and this stays dormant in practice - wired now so a future
    // per-user GMCP job doesn't also have to remember this loop.
    let stats_flush_sleep = tokio::time::sleep(FAR_FUTURE_MU);
    tokio::pin!(stats_flush_sleep);

    // Main event loop - only handles WebSocket events
    loop {
        // Reap any zombie child processes (TLS proxies that have exited)
        #[cfg(all(unix, not(target_os = "android")))]
        reap_zombie_children();

        tokio::select! {
            // TF repeat process tick — only fires when processes exist
            _ = &mut process_tick_sleep => {
                let now = std::time::Instant::now();
                let mut to_remove = vec![];
                let process_count = app.tf_engine.processes.len();
                for i in 0..process_count {
                    if app.tf_engine.processes[i].on_prompt { continue; }
                    if app.tf_engine.processes[i].next_run <= now {
                        let cmd = app.tf_engine.processes[i].command.clone();
                        let process_world = app.tf_engine.processes[i].world.clone();
                        app.sync_tf_world_info();
                        let result = app.tf_engine.execute(&cmd);
                        let target_idx = if let Some(ref wname) = process_world {
                            if wname.is_empty() {
                                Some(app.current_world_index)
                            } else {
                                app.find_world_index(wname)
                            }
                        } else {
                            Some(app.current_world_index)
                        };
                        let world_idx = target_idx.unwrap_or(app.current_world_index);
                        match result {
                            tf::TfCommandResult::SendToMud(text) => {
                                if let Some(idx) = target_idx {
                                    app.send_to_world(idx, text);
                                }
                            }
                            tf::TfCommandResult::Success(Some(msg)) => {
                                // This tick isn't actually per-user scoped - TF repeat
                                // processes run against the shared app.tf_engine/app.worlds,
                                // not any specific user's own UserConnection - so routing
                                // through the regular gated path (like every other TF-result
                                // site) is correct here, not the multiuser per-user broadcast
                                // helpers. Was previously a raw ws.broadcast_to_all, fully
                                // bypassing more-mode gating and never reaching output_lines
                                // (so /recall could never find it, same bug class as the
                                // original /recall fix).
                                app.emit_client_text(world_idx, &msg, true);
                            }
                            tf::TfCommandResult::Error(err) => {
                                app.emit_tf_error(world_idx, &err, true);
                            }
                            tf::TfCommandResult::RepeatProcess(process) => {
                                app.register_repeat_process(process);
                            }
                            tf::TfCommandResult::NotTfCommand => {
                                // Plain text command - send to MUD (captured via
                                // send_to_world - a /repeat body is exactly the
                                // "/repeat batches" case /recall -i now covers).
                                if let Some(idx) = target_idx {
                                    app.send_to_world(idx, cmd.clone());
                                }
                            }
                            _ => {}
                        }
                        let interval = app.tf_engine.processes[i].interval;
                        app.tf_engine.processes[i].next_run += interval;
                        if let Some(ref mut rem) = app.tf_engine.processes[i].remaining {
                            *rem = rem.saturating_sub(1);
                            if *rem == 0 {
                                to_remove.push(i);
                            }
                        }
                    }
                }
                for i in to_remove.into_iter().rev() {
                    app.tf_engine.processes.remove(i);
                }
                // Re-arm: tick again in 1s if processes remain
                if !app.tf_engine.processes.is_empty() {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(1));
                } else {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE_MU);
                }
            }
            Some(event) = event_rx.recv() => {
                match event {
                    AppEvent::WsClientMessage(client_id, msg) => {
                        handle_multiuser_ws_message(&mut app, client_id, *msg, &event_tx).await;
                    }
                    // Legacy events - not used in multiuser mode (we use Multiuser* variants)
                    AppEvent::ServerData(_, _) => {}
                    AppEvent::Disconnected(..) => {}
                    AppEvent::ConnectWorldRequest(world_index, requesting_username) => {
                        // Connect a world on behalf of a user (per-user isolated connection)
                        let key = (world_index, requesting_username.clone());
                        let already_connected = app.user_connections.get(&key).map(|c| c.connected).unwrap_or(false);

                        if world_index < app.worlds.len() && !already_connected {
                            let settings = app.worlds[world_index].settings.clone();
                            let world_name = app.worlds[world_index].name.clone();

                            // Check if world has connection settings
                            if !settings.has_connection_settings() {
                                if let Some(ws) = &app.ws_server {
                                    ws.broadcast_to_owner(WsMessage::ServerData { archive_sourced: false,
                                        world_index,
                                        data: "No connection settings configured for this world.\n".to_string(),
                                        is_viewed: true,
                                        ts: current_timestamp_secs(),
                                        from_server: false,
                                        seq: 0, end_seq: None,
                                        flush: false, gagged: false, highlight_colors: Vec::new(),
                                    }, Some(&requesting_username));
                                }
                            // Create per-user connection
                            } else if let Some(cmd_tx) = connect_multiuser_world(
                                world_index,
                                requesting_username.clone(),
                                &settings,
                                event_tx.clone(),
                            ).await {
                                // Store connection in user_connections
                                let mut conn = UserConnection::new();
                                conn.connected = true;
                                conn.command_tx = Some(cmd_tx);
                                conn.last_send_time = Some(std::time::Instant::now());
                                conn.last_receive_time = Some(std::time::Instant::now());
                                app.user_connections.insert(key, conn);

                                // Send WorldConnected only to this user
                                if let Some(ws) = &app.ws_server {
                                    ws.broadcast_to_owner(
                                        WsMessage::WorldConnected { world_index, name: world_name },
                                        Some(&requesting_username)
                                    );
                                }
                            } else {
                                // Connection failed - send error to user
                                if let Some(ws) = &app.ws_server {
                                    ws.broadcast_to_owner(WsMessage::ServerData { archive_sourced: false,
                                        world_index,
                                        data: "Connection failed.\n".to_string(),
                                        is_viewed: true,
                                        ts: current_timestamp_secs(),
                                        from_server: false,
                                        seq: 0, end_seq: None,
                                        flush: false, gagged: false, highlight_colors: Vec::new(),
                                    }, Some(&requesting_username));
                                }
                            }
                        }
                    }
                    AppEvent::MultiuserServerData(world_index, username, data) => {
                        // Route server data to specific user's connection
                        let key = (world_index, username.clone());
                        let world_settings = app.worlds.get(world_index).map(|w| w.settings.clone());
                        if let Some(conn) = app.user_connections.get_mut(&key) {
                            // Job 9 (T3.2): decode with THIS connection's own negotiated
                            // CHARSET encoding, not the (never set by multiuser)
                            // World::negotiated_encoding - see conn.protocol's doc
                            // comment and CharsetRequest's per-variant table entry.
                            let encoding = match &world_settings {
                                Some(settings) => conn.protocol.effective_encoding(settings),
                                None => Encoding::Utf8,
                            };
                            let decoded = encoding.decode(&data);

                            // Add to user's output buffer
                            for line in decoded.lines() {
                                let seq = conn.output_lines.len() as u64;
                                conn.output_lines.push(OutputLine::new(line.to_string(), seq));
                            }

                            // Send to this user's WebSocket clients only. This is real MUD
                            // server output (OutputLine::new above already defaults
                            // from_server: true internally) - from_server must match here
                            // too, or the client-line "✨ " marker would wrongly apply to
                            // every line of MUD text once clients key off this flag.
                            if let Some(ws) = &app.ws_server {
                                ws.broadcast_to_owner(WsMessage::ServerData { archive_sourced: false,
                                    world_index,
                                    data: decoded,
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: true,
                                    seq: 0, end_seq: None,
                                    flush: false, gagged: false, highlight_colors: Vec::new(),
                                }, Some(&username));
                            }
                        }
                    }
                    AppEvent::MultiuserDisconnected(world_index, username) => {
                        handle_multiuser_disconnect(&mut app, world_index, &username);
                    }
                    AppEvent::MultiuserTelnetDetected(world_index, username) => {
                        let key = (world_index, username.clone());
                        if let Some(conn) = app.user_connections.get_mut(&key) {
                            conn.telnet_mode = true;
                        }
                    }
                    AppEvent::MultiuserPrompt(world_index, username, prompt_bytes) => {
                        let key = (world_index, username.clone());
                        let world_settings = app.worlds.get(world_index).map(|w| w.settings.clone());
                        if let Some(conn) = app.user_connections.get_mut(&key) {
                            // Job 9 (T3.2): same fix as MultiuserServerData above.
                            let encoding = match &world_settings {
                                Some(settings) => conn.protocol.effective_encoding(settings),
                                None => Encoding::Utf8,
                            };
                            let prompt_text = encoding.decode(&prompt_bytes);
                            conn.prompt = prompt_text.trim_end().to_string() + " ";

                            // Send prompt update to this user
                            if let Some(ws) = &app.ws_server {
                                ws.broadcast_to_owner(WsMessage::PromptUpdate {
                                    world_index,
                                    prompt: conn.prompt.clone(),
                                }, Some(&username));
                            }
                        }
                    }
                    // Job 9 (T3.2): this is the fix for the bug this job exists for -
                    // before it, this loop had no arm for AppEvent::Telnet at all, so
                    // every telnet event to_app_event now forwards for a Multiuser
                    // target (NAWS, TTYPE/MTTS, CHARSET, GMCP incl. Core.Hello, MSDP,
                    // MSSP, MSP, ECHO masking) fell into the wildcard below and was
                    // silently dropped. Route it exactly like the World-target loops do,
                    // through the per-user sibling of App::handle_telnet_event.
                    AppEvent::Telnet(TelnetTarget::Multiuser { world_index, ref username }, ref ev) => {
                        app.handle_multiuser_telnet_event(world_index, username.clone(), ev);
                    }
                    AppEvent::WsAuthKeyValidation(client_id, _msg, client_ip, _challenge) => {
                        // Auth-key login is a single per-install device key and doesn't map to
                        // multiuser's per-account model, so it's intentionally unsupported here.
                        // Reply with a clean failure instead of silently dropping the request,
                        // which would otherwise leave the client waiting for a response.
                        crate::http::log_remote_event("WS-KEY-REJECT", &client_ip,
                            "auth keys not supported in multiuser mode");
                        app.ws_send_to_client(client_id, WsMessage::AuthResponse {
                            success: false,
                            error: Some("Auth key login not supported in multiuser mode".to_string()),
                            username: None,
                            multiuser_mode: true,
                        });
                    }
                    // WsKeyRequest / WsKeyRevoke: also intentionally unsupported in multiuser
                    // mode (single per-install key model), so they fall through to the
                    // wildcard below and are silently ignored.
                    _ => {} // Ignore other events in multiuser mode
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down multiuser server...");
                break;
            }

            // Stats flush tick — mud-status-display.md Job 2 (plan D4). Only fires while
            // some world's stats are dirty (see the bottom-of-loop rearm below); see this
            // timer's own declaration above for why it is dormant in practice today.
            _ = &mut stats_flush_sleep => {
                app.flush_dirty_stats();
                stats_flush_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE_MU);
            }
        }

        // Activate process tick sleep if processes were added during this iteration
        if !app.tf_engine.processes.is_empty()
            && process_tick_sleep.deadline() > tokio::time::Instant::now() + std::time::Duration::from_secs(2)
        {
            process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(1));
        }

        // mud-status-display.md Job 2 (plan D4): pull stats_flush_sleep's deadline in to
        // ~150ms out if some world went dirty this iteration and nothing sooner is already
        // scheduled. Never push the deadline *later*.
        if app.worlds.iter().any(|w| w.protocol.stats_dirty)
            && stats_flush_sleep.deadline() > tokio::time::Instant::now() + std::time::Duration::from_millis(150)
        {
            stats_flush_sleep.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_millis(150));
        }
    }

    Ok(())
}

/// Handle a per-user multiuser disconnect (`AppEvent::MultiuserDisconnected`). Extracted
/// as its own function (plan Job 9, T3.2 - mirrors `App::apply_mccp2_reload_bailout`'s
/// reason for existing as a standalone method) so it has a real unit test:
/// `run_multiuser_server` is a large `async fn` needing a live event loop to reach at all.
///
/// Clears this connection's `ProtocolState` (`ProtocolState::clear`) so a stale mirror -
/// GMCP/MSDP data, a masked prompt, a negotiated encoding - from THIS connection can never
/// be mistaken for live state on the next one to the same world, the same class of bug
/// `World::clear_connection_state` already guards against for the single-user path. If the
/// connection was echo-masked, also tells the owner's clients to unmask - otherwise a web
/// client could stay showing a masked input box after the drop, with nothing left to clear
/// it (the next connection starts unmasked and may never see another `WILL ECHO`).
fn handle_multiuser_disconnect(app: &mut App, world_index: usize, username: &str) {
    let key = (world_index, username.to_string());
    if let Some(conn) = app.user_connections.get_mut(&key) {
        conn.connected = false;
        conn.command_tx = None;
        let was_masked = conn.protocol.echo_masked;
        conn.protocol.clear();

        // Send disconnect to this user only
        if let Some(ws) = &app.ws_server {
            ws.broadcast_to_owner(
                WsMessage::WorldDisconnected { world_index },
                Some(username)
            );
            if was_masked {
                ws.broadcast_to_owner(
                    WsMessage::EchoMaskChanged { world_index, masked: false },
                    Some(username),
                );
            }
        }
    }
}

/// Connect to a world for a specific user in multiuser mode
/// Returns the command sender if successful
pub async fn connect_multiuser_world(
    world_index: usize,
    username: String,
    settings: &WorldSettings,
    event_tx: mpsc::Sender<AppEvent>,
) -> Option<mpsc::Sender<WriteCommand>> {
    let host = &settings.hostname;
    let port = &settings.port;
    let use_ssl = settings.use_ssl;

    if host.is_empty() || port.is_empty() {
        return None;
    }

    match TcpStream::connect(format!("{}:{}", host, port)).await {
        Ok(tcp_stream) => {
            let _ = tcp_stream.set_nodelay(true);

            // Enable TCP keepalive to detect dead connections faster
            enable_tcp_keepalive(&tcp_stream);

            // Handle SSL if needed
            let (read_half, write_half): (StreamReader, StreamWriter) = if use_ssl {
                #[cfg(feature = "native-tls-backend")]
                {
                    let connector = match native_tls::TlsConnector::builder()
                        .danger_accept_invalid_certs(true)
                        .build()
                    {
                        Ok(c) => c,
                        Err(_) => return None,
                    };
                    let connector = tokio_native_tls::TlsConnector::from(connector);

                    match connector.connect(host, tcp_stream).await {
                        Ok(tls_stream) => {
                            let peer_cert = tls_stream.get_ref().peer_certificate().ok().flatten();
                            if crate::platform::check_native_tls_peer_pin(&format!("{}:{}", host, port), peer_cert).is_err() {
                                return None;
                            }
                            let (r, w) = tokio::io::split(tls_stream);
                            (StreamReader::Tls(r), StreamWriter::Tls(w))
                        }
                        Err(_) => return None,
                    }
                }

                #[cfg(feature = "rustls-backend")]
                {
                    use rustls::RootCertStore;
                    use tokio_rustls::TlsConnector;
                    use rustls::pki_types::ServerName;

                    let mut root_store = RootCertStore::empty();
                    root_store.roots = webpki_roots::TLS_SERVER_ROOTS.to_vec();

                    let config = rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(crate::platform::danger_rustls::TofuVerifier::new(format!("{}:{}", host, port))))
                        .with_no_client_auth();

                    let connector = TlsConnector::from(Arc::new(config));
                    let server_name = match ServerName::try_from(host.clone()) {
                        Ok(sn) => sn,
                        Err(_) => return None,
                    };

                    match connector.connect(server_name, tcp_stream).await {
                        Ok(tls_stream) => {
                            let (r, w) = tokio::io::split(tls_stream);
                            (StreamReader::Tls(r), StreamWriter::Tls(w))
                        }
                        Err(_) => return None,
                    }
                }

                #[cfg(not(any(feature = "native-tls-backend", feature = "rustls-backend")))]
                {
                    return None;
                }
            } else {
                let (r, w) = tcp_stream.into_split();
                (StreamReader::Plain(r), StreamWriter::Plain(w))
            };

            // Plan Job 13 (Phase 4, 4.4): migrated onto spawn_telnet_writer. Fresh
            // connect, so `settings.encoding` (== effective_encoding() - there is no
            // World object yet to have a negotiated_encoding on) is the right initial
            // encoding; a later CHARSET accept switches it in-band via
            // WriteCommand::SetEncoding.
            let cmd_tx = spawn_telnet_writer(write_half, settings.encoding);

            // Send auto-login if configured
            let user = settings.user.clone();
            let password = settings.password.clone();
            let auto_connect_type = settings.auto_connect_type;
            if !user.is_empty() && auto_connect_type == AutoConnectType::Connect {
                let tx = cmd_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let connect_cmd = format!("connect {} {}", user, password);
                    let _ = tx.send(WriteCommand::Text(connect_cmd)).await;
                });
            }

            // Clone for reader task
            let event_tx_read = event_tx.clone();
            let username_read = username.clone();

            // Plan Job 5, 2.3: migrated onto spawn_telnet_reader. Gains over the old
            // hand-rolled loop: MCCP2 decompression (finding 1 — this path used to accept
            // IAC DO MCCP2 and then render the compressed stream as text) and the unified
            // "Connection closed by server." message on EOF. CHARSET/NAWS/TTYPE/GMCP/MSDP/
            // MSSP/WontEchoSeen/EchoOff/EchoOn used to be dead here too (the old loop's
            // `AppEvent::CharsetRequested(String::new(), ...)` was rejected by
            // `find_world_index("")`, and every other one had no `(usize, String)`-shaped
            // `AppEvent` at all) — as of Job 9 (T3.2), `to_app_event` forwards all of them
            // for a `Multiuser` target and `run_multiuser_server`'s `AppEvent::Telnet` arm
            // routes them through `App::handle_multiuser_telnet_event`, so this reader now
            // carries the same protocol events for a multiuser connection that it always
            // has for a single-user `World` one.
            let telnet_cfg = TelnetConfig {
                term_type: std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string()),
                msp_enabled: settings.msp_enabled,
                mccp2_enabled: settings.mccp2_enabled,
                is_tls: use_ssl, // Job 12 (plan Phase 4, 4.1)
                ..TelnetConfig::default()
            };
            spawn_telnet_reader(
                read_half,
                cmd_tx.clone(),
                event_tx_read,
                TelnetTarget::Multiuser { world_index, username: username_read },
                0, // conn_id: unused for TelnetTarget::Multiuser (see make_disconnected_event)
                telnet_cfg,
            );

            Some(cmd_tx)
        }
        Err(_) => None,
    }
}

/// Connect a world in daemon mode (non-multiuser)
/// Returns (cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path) on success
pub async fn connect_daemon_world(
    _world_index: usize,
    world_name: String,
    settings: &WorldSettings,
    event_tx: mpsc::Sender<AppEvent>,
    connection_id: u64,
    skip_auto_login: bool,
    tls_proxy_enabled: bool,
) -> Option<(mpsc::Sender<WriteCommand>, Option<SocketFd>, bool, Option<u32>, Option<std::path::PathBuf>)> {
    #[cfg(target_os = "android")]
    let _ = tls_proxy_enabled;
    let host = &settings.hostname;
    let port = &settings.port;
    let use_ssl = settings.use_ssl;

    if host.is_empty() || port.is_empty() {
        return None;
    }

    // TLS proxy path — spawn a separate proxy process that holds the TLS connection
    // so it survives hot reload. Platform-specific IPC: Unix sockets on Unix,
    // Named Pipes on Windows.
    #[cfg(all(unix, not(target_os = "android")))]
    if use_ssl && tls_proxy_enabled {
        if let Ok((proxy_pid, socket_path)) = spawn_tls_proxy(&world_name, host, port) {
            let mut connected = false;
            for attempt in 0..20 {
                match tokio::net::UnixStream::connect(&socket_path).await {
                    Ok(unix_stream) => {
                        let (r, w) = unix_stream.into_split();
                        let read_half = StreamReader::Proxy(r);
                        let write_half = StreamWriter::Proxy(w);
                        // Plan Job 13 (Phase 4, 4.4): migrated onto spawn_telnet_writer.
                        // Fresh connect - settings.encoding is the right initial encoding.
                        let cmd_tx = spawn_telnet_writer(write_half, settings.encoding);
                        if !skip_auto_login {
                            let user = settings.user.clone();
                            let password = settings.password.clone();
                            let auto_connect_type = settings.auto_connect_type;
                            if !user.is_empty() && !password.is_empty() && auto_connect_type == AutoConnectType::Connect {
                                let tx = cmd_tx.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    let _ = tx.send(WriteCommand::Text(format!("connect {} {}", user, password))).await;
                                });
                            }
                        }
                        let event_tx_read = event_tx.clone();
                        let world_name_read = world_name.clone();
                        let reader_conn_id = connection_id;
                        // Plan Job 5, 2.3: migrated onto spawn_telnet_reader. Gains over
                        // the old hand-rolled loop: MCCP2 decompression (this proxy path
                        // used to accept IAC DO MCCP2 and then render the compressed
                        // stream as text — finding 1), TTYPE/CHARSET events, WontEchoSeen,
                        // and the unified "Connection closed by server." message on EOF.
                        let telnet_cfg = TelnetConfig {
                            term_type: std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string()),
                            msp_enabled: settings.msp_enabled,
                            mccp2_enabled: settings.mccp2_enabled,
                            // Job 12 (plan Phase 4, 4.1): this is the TLS-proxy path - only
                            // reached when use_ssl is true.
                            is_tls: use_ssl,
                ..TelnetConfig::default()
                        };
                        spawn_telnet_reader(
                            read_half,
                            cmd_tx.clone(),
                            event_tx_read,
                            TelnetTarget::World(world_name_read),
                            reader_conn_id,
                            telnet_cfg,
                        );
                        return Some((cmd_tx, None, true, Some(proxy_pid), Some(socket_path)));
                    }
                    Err(_) => {
                        if attempt < 19 { tokio::time::sleep(tokio::time::Duration::from_millis(100)).await; }
                        connected = false;
                    }
                }
                if connected { break; }
            }
            if !connected {
                unsafe { libc::kill(proxy_pid as libc::pid_t, libc::SIGTERM); }
            }
        }
        // Fall through to direct TLS if proxy failed
    }

    #[cfg(windows)]
    if use_ssl && tls_proxy_enabled {
        if let Ok((proxy_pid, pipe_path)) = spawn_tls_proxy(&world_name, host, port) {
            match connect_to_proxy_pipe(&pipe_path, 10).await {
                Some(pipe_client) => {
                    let (r, w) = tokio::io::split(pipe_client);
                    let read_half = StreamReader::NamedPipeProxy(r);
                    let write_half = StreamWriter::NamedPipeProxy(w);
                        // Plan Job 13 (Phase 4, 4.4): migrated onto spawn_telnet_writer -
                        // mirrors the Unix-socket proxy block above; unverified in this
                        // sandbox (no mingw toolchain to cross-check a #[cfg(windows)]
                        // path - see the plan's Windows verification note).
                        let cmd_tx = spawn_telnet_writer(write_half, settings.encoding);
                        if !skip_auto_login {
                            let user = settings.user.clone();
                            let password = settings.password.clone();
                            let auto_connect_type = settings.auto_connect_type;
                            if !user.is_empty() && !password.is_empty() && auto_connect_type == AutoConnectType::Connect {
                                let tx = cmd_tx.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(500)).await;
                                    let _ = tx.send(WriteCommand::Text(format!("connect {} {}", user, password))).await;
                                });
                            }
                        }
                        let event_tx_read = event_tx.clone();
                        let world_name_read = world_name.clone();
                        let reader_conn_id = connection_id;
                        // Plan Job 5, 2.3: migrated onto spawn_telnet_reader. Gains over
                        // the old hand-rolled loop: MCCP2 decompression (this proxy path
                        // used to accept IAC DO MCCP2 and then render the compressed
                        // stream as text — finding 1), TTYPE/CHARSET events, WontEchoSeen,
                        // and the unified "Connection closed by server." message on EOF.
                        let telnet_cfg = TelnetConfig {
                            term_type: std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string()),
                            msp_enabled: settings.msp_enabled,
                            mccp2_enabled: settings.mccp2_enabled,
                            // Job 12 (plan Phase 4, 4.1): this is the TLS-proxy path - only
                            // reached when use_ssl is true.
                            is_tls: use_ssl,
                ..TelnetConfig::default()
                        };
                        spawn_telnet_reader(
                            read_half,
                            cmd_tx.clone(),
                            event_tx_read,
                            TelnetTarget::World(world_name_read),
                            reader_conn_id,
                            telnet_cfg,
                        );
                        return Some((cmd_tx, None, true, Some(proxy_pid), Some(pipe_path)));
                }
                None => {
                    kill_proxy_process(proxy_pid);
                }
            }
        }
        // Fall through to direct TLS
    }

    match TcpStream::connect(format!("{}:{}", host, port)).await {
        Ok(tcp_stream) => {
            let _ = tcp_stream.set_nodelay(true);

            // Store the socket fd for hot reload (before splitting)
            #[cfg(unix)]
            let socket_fd: Option<SocketFd> = {
                use std::os::unix::io::AsRawFd;
                Some(tcp_stream.as_raw_fd())
            };
            #[cfg(windows)]
            let socket_fd: Option<SocketFd> = {
                use std::os::windows::io::AsRawSocket;
                Some(tcp_stream.as_raw_socket() as i64)
            };
            #[cfg(not(any(unix, windows)))]
            let socket_fd: Option<SocketFd> = None;

            // Enable TCP keepalive to detect dead connections faster
            enable_tcp_keepalive(&tcp_stream);

            // Handle SSL if needed
            let is_tls;
            let (read_half, write_half): (StreamReader, StreamWriter) = if use_ssl {
                is_tls = true;
                #[cfg(feature = "native-tls-backend")]
                {
                    let connector = match native_tls::TlsConnector::builder()
                        .danger_accept_invalid_certs(true)
                        .build()
                    {
                        Ok(c) => c,
                        Err(_) => return None,
                    };
                    let connector = tokio_native_tls::TlsConnector::from(connector);

                    match connector.connect(host, tcp_stream).await {
                        Ok(tls_stream) => {
                            let peer_cert = tls_stream.get_ref().peer_certificate().ok().flatten();
                            if crate::platform::check_native_tls_peer_pin(&format!("{}:{}", host, port), peer_cert).is_err() {
                                return None;
                            }
                            let (r, w) = tokio::io::split(tls_stream);
                            (StreamReader::Tls(r), StreamWriter::Tls(w))
                        }
                        Err(_) => return None,
                    }
                }

                #[cfg(feature = "rustls-backend")]
                {
                    use rustls::RootCertStore;
                    use tokio_rustls::TlsConnector;
                    use rustls::pki_types::ServerName;

                    let mut root_store = RootCertStore::empty();
                    root_store.roots = webpki_roots::TLS_SERVER_ROOTS.to_vec();

                    let config = rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(crate::platform::danger_rustls::TofuVerifier::new(format!("{}:{}", host, port))))
                        .with_no_client_auth();

                    let connector = TlsConnector::from(Arc::new(config));
                    let server_name = match ServerName::try_from(host.clone()) {
                        Ok(sn) => sn,
                        Err(_) => return None,
                    };

                    match connector.connect(server_name, tcp_stream).await {
                        Ok(tls_stream) => {
                            let (r, w) = tokio::io::split(tls_stream);
                            (StreamReader::Tls(r), StreamWriter::Tls(w))
                        }
                        Err(_) => return None,
                    }
                }

                #[cfg(not(any(feature = "native-tls-backend", feature = "rustls-backend")))]
                {
                    return None;
                }
            } else {
                is_tls = false;
                let (r, w) = tcp_stream.into_split();
                (StreamReader::Plain(r), StreamWriter::Plain(w))
            };

            // For TLS, socket_fd should be None (can't preserve across reload)
            let final_socket_fd = if is_tls { None } else { socket_fd };

            // Plan Job 13 (Phase 4, 4.4): migrated onto spawn_telnet_writer. Fresh
            // connect - settings.encoding is the right initial encoding.
            let cmd_tx = spawn_telnet_writer(write_half, settings.encoding);

            // Send auto-login if configured (skip if /worlds -l was used)
            if !skip_auto_login {
                let user = settings.user.clone();
                let password = settings.password.clone();
                let auto_connect_type = settings.auto_connect_type;
                if !user.is_empty() && !password.is_empty() && auto_connect_type == AutoConnectType::Connect {
                    let tx = cmd_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        let connect_cmd = format!("connect {} {}", user, password);
                        let _ = tx.send(WriteCommand::Text(connect_cmd)).await;
                    });
                }
            }

            // Clone for reader task
            let event_tx_read = event_tx.clone();
            let world_name_read = world_name.clone();
            let reader_conn_id = connection_id;

            // Terminal type for TTYPE IS responses — as of Job 12 (plan Phase 4,
            // 4.1) the session itself answers with the MTTS-cycled value (client
            // name, then this term_type, then MTTS <bitmask>) the instant a TTYPE
            // SEND arrives; `App` no longer builds a reply. Matches the existing
            // `$TERM`-or-"ANSI" fallback exactly.
            let telnet_cfg = TelnetConfig {
                term_type: std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string()),
                msp_enabled: settings.msp_enabled,
                mccp2_enabled: settings.mccp2_enabled,
                is_tls, // Job 12 (plan Phase 4, 4.1): set above by the TLS/plain match
                ..TelnetConfig::default()
            };

            // Spawn reader task (plan Job 5, 2.3: migrated onto the shared
            // `spawn_telnet_reader`, src/telnet_reader.rs — this was already the richest
            // of daemon.rs's four reader loops, so nothing is gained or lost here; see the
            // Job 5 report for what the other three gained).
            spawn_telnet_reader(
                read_half,
                cmd_tx.clone(),
                event_tx_read,
                TelnetTarget::World(world_name_read),
                reader_conn_id,
                telnet_cfg,
            );

            Some((cmd_tx, final_socket_fd, is_tls, None, None))
        }
        Err(_) => None,
    }
}

/// Build initial state message for a specific user (multiuser mode)
/// World definitions are shared, but connection state is per-user
/// Actions are still filtered per-user
///
/// B4 (security remediation): every world's `hostname`/`port`/`user` used to be sent to
/// every authenticated user regardless of ownership (`has_password`/`password` were
/// already correctly scoped). All worlds must have an `owner` in multiuser mode (enforced
/// at startup, see the "all worlds must have owners" validation) — there is no un-owned
/// / global-world concept to special-case here, so the predicate is a straight ownership
/// match, mirroring `SwitchWorld`/`ConnectWorld`/etc. below and the action filter a few
/// lines down. World *entries* (and their `index`) are still sent for every world — not
/// just the ones this user owns — because `index` is the same global `app.worlds`
/// position used by every other WS message (`ServerData`, `WorldConnected`, ...); dropping
/// entries would desync that indexing for the console remote-client and web/GUI clients,
/// which both key off position == server-global `world_index`. Only the sensitive
/// connection-identifying fields are redacted for worlds this user doesn't own.
pub fn build_multiuser_initial_state(app: &App, username: &str) -> WsMessage {
    // Send only the most recent lines in InitialState for fast initial load, same
    // remote_initial_lines-driven aggregate-plus-per-world budget single-user's
    // build_initial_state uses (main.rs) - previously this function had NO cap at all, so a
    // long-running multiuser connection could send its entire accumulated per-user output
    // buffer in one InitialState message. Uses the same shared
    // App::build_initial_output_lines helper so the two implementations can't silently
    // diverge on this budget again.
    let per_world_cap = app.settings.remote_initial_lines.max(1) as usize;
    let total_line_budget = per_world_cap.max(500);
    let mut budget_remaining = total_line_budget;

    // Show all worlds with per-user connection state
    let worlds: Vec<WorldStateMsg> = app.worlds.iter().enumerate()
        .map(|(idx, world)| {
            let is_owner = world.owner.as_deref() == Some(username);

            // Get user's connection state for this world (if any). Non-owned worlds never
            // have a connection entry keyed by this username (ConnectWorld is gated to the
            // owner), so this is already correctly empty for them independent of the
            // redaction below — but we redact the static settings explicitly too, rather
            // than relying on that as the only guard.
            let key = (idx, username.to_string());
            let user_conn = app.user_connections.get(&key);

            // Use user's connection state or empty defaults
            let empty_output: Vec<OutputLine> = vec![];
            let (connected, output_lines, prompt, scroll_offset, paused, unseen_lines, last_send, last_recv, has_trailing_partial) =
                if let Some(conn) = user_conn {
                    (
                        conn.connected,
                        &conn.output_lines,
                        conn.prompt.clone(),
                        conn.scroll_offset,
                        conn.paused,
                        conn.unseen_lines,
                        conn.last_send_time,
                        conn.last_receive_time,
                        !conn.partial_line.is_empty() && !conn.partial_in_pending,
                    )
                } else {
                    (false, &empty_output, String::new(), 0, false, 0, None, None, false)
                };

            let max_initial_lines = per_world_cap.min(budget_remaining);
            // Text stays prefix-free here, same as live ServerData broadcasts - the
            // "✨ " client-line marker is added at display time only (rendering.rs::
            // process_output_line for console/remote console, applyClientPrefix() in
            // web/app.js), keyed on `from_server` below.
            let (output_lines_ts, visible_count) = App::build_initial_output_lines(output_lines, has_trailing_partial, max_initial_lines);
            // Decrement by the VISIBLE count, not the raw slice length - see the identical
            // comment on the single-user build_initial_state (main.rs).
            budget_remaining = budget_remaining.saturating_sub(visible_count);

            WorldStateMsg {
                index: idx,
                name: world.name.clone(),
                connected,
                // Legacy output_lines/pending_lines left empty - all clients prefer the
                // _ts variants (see CLAUDE.md's "WebSocket InitialState" pattern; matches
                // single-user's build_initial_state). pending_lines_ts is also left empty
                // deliberately: pending lines stay server-side and are released via
                // PgDn/Tab/ReleasePending, then broadcast to clients normally - sending them
                // here too would double-deliver them once released (App::init_from_initial_state,
                // the --console remote client's InitialState handler, populates
                // world.pending_lines straight from pending_lines_ts with no dedup).
                output_lines: Vec::new(),
                pending_lines: Vec::new(),
                output_lines_ts,
                pending_lines_ts: Vec::new(),
                prompt: prompt.replace('\r', ""),
                scroll_offset,
                paused,
                unseen_lines,
                settings: WorldSettingsMsg {
                    // B4 (security remediation): hostname/port/user (and has_password) are
                    // only meaningful to the owner — a non-owner gets empty/false instead
                    // of another user's connection details.
                    hostname: if is_owner { world.settings.hostname.clone() } else { String::new() },
                    port: if is_owner { world.settings.port.clone() } else { String::new() },
                    user: if is_owner { world.settings.user.clone() } else { String::new() },
                    password: String::new(),
                    has_password: is_owner && !world.settings.password.is_empty(),
                    use_ssl: world.settings.use_ssl,
                    log_enabled: world.settings.log_enabled,
                    encoding: world.settings.encoding.name().to_string(),
                    auto_connect_type: world.settings.auto_connect_type.name().to_string(),
                    keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                    keep_alive_cmd: if is_owner { world.settings.keep_alive_cmd.clone() } else { String::new() },
                    gmcp_packages: if is_owner { world.settings.gmcp_packages.clone() } else { String::new() },
                    auto_reconnect_secs: world.settings.auto_reconnect_display(),
                    has_notes: is_owner && !world.settings.notes.is_empty(),
                    msp_enabled: world.settings.msp_enabled,
                    mcp_enabled: world.settings.mcp_enabled,
                    mccp2_enabled: world.settings.mccp2_enabled,
                },
                last_send_secs: last_send.map(|t| t.elapsed().as_secs()),
                last_recv_secs: last_recv.map(|t| t.elapsed().as_secs()),
                last_nop_secs: None,
                keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                showing_splash: world.showing_splash,
                was_connected: world.was_connected,
                is_proxy: world.proxy_pid.is_some(),
                gmcp_user_enabled: world.gmcp_user_enabled,
                // ECHO masking (Job 9, T3.2 - the cleartext-password fix): now read from
                // THIS user's own connection, not the shared World (which multiuser never
                // wrote to, even after this job - see UserConnection::protocol's doc
                // comment). A user with no connection entry yet is unmasked, matching
                // user_conn's other "no connection" defaults above.
                echo_masked: user_conn.map(|c| c.protocol.echo_masked).unwrap_or(false),
                total_output_lines: world.output_lines.len(),
                // Matches total_output_lines' existing source (world.output_lines, not the
                // per-user conn.output_lines) - see the field's doc comment in websocket.rs.
                total_visible_lines: Some(world.output_lines.iter().filter(|l| !l.gagged).count()),
                pending_count: world.pending_lines.len(),
                // Multiuser has never supported the ▶ new-text indicator - MUD data for
                // multiuser worlds flows through per-user UserConnection.output_lines, not
                // World, and its ServerData broadcasts already hardcode marked_new: false /
                // is_viewed: true unconditionally (see AppEvent::MultiuserServerData). A
                // constant 0 ("everything already displayed") preserves that, rather than
                // reading World::new_from_seq, which tracks the shared World's own
                // (unused-by-multiuser) output_lines and would be meaningless here.
                // Same reasoning as new_from_seq above: multiuser has never supported the ▶
                // window's upper bound either. u64::MAX ("no viewing episode in progress")
                // is the neutral/inert value, matching a fresh World's own default.
                // Same reasoning as new_from_seq above: multiuser ServerData always hardcodes
                // seq: 0, so a multiuser client's cached/in-memory _max_seq for this world can
                // never be a real positive value either - the app.js server-restart-detection
                // guard this field feeds only fires when the cache claims a real (> 0) seq, so
                // a constant 0 here is inert, not a false "server restarted" trigger.
                next_seq: 0,
                // Multiuser emits seq: 0 on every line and has no real sequence space, so it
                // has no epoch to report. 0 tells the client to fall back to the older
                // heuristic rather than compare this as a concrete value.
                seq_epoch: 0,
                // mud-status-display.md Job 2 / Job 9 (T3.2): owner-only, same redaction
                // rule as hostname/port/user/keep_alive_cmd/gmcp_packages/notes above
                // (CLAUDE.md's "Multiuser handlers ... must check world.owner ==
                // username"). Now read from this user's own UserConnection - GMCP/MSDP
                // ingestion is wired into multiuser's per-user connections as of this job
                // (App::handle_multiuser_telnet_event), so this is no longer always empty.
                stats: if is_owner {
                    user_conn.map(|c| c.protocol.stats.entries().to_vec()).unwrap_or_default()
                } else {
                    Vec::new()
                },
            }
        }).collect();

    // Include actions owned by this user and un-owned (global/master-created) actions
    let actions: Vec<Action> = app.settings.actions.iter()
        .filter(|a| a.owner.as_deref() == Some(username) || a.owner.is_none())
        .cloned()
        .collect();

    // Build settings (uses build_global_settings_msg to avoid leaking sensitive data)
    let settings = app.build_global_settings_msg();

    // Find current world index for this user
    // Use the first world they have a connection to, or 9999 if none (no world selected)
    let current_world_index = app.user_connections.keys()
        .filter(|(_, u)| u == username)
        .map(|(idx, _)| *idx)
        .min()
        .unwrap_or(9999);

    // Generate splash lines for multiuser mode
    let splash_lines = generate_splash_strings();

    WsMessage::InitialState {
        // Multiuser has no per-line ▶ ownership (it emits seq: 0 universally and never
        // claims), so there is no id to report - 0 means "no markers are mine".
        your_display_id: 0,
        worlds,
        settings,
        current_world_index,
        actions,
        splash_lines,
        server_version: crate::VERSION.to_string(),
        android_app_version: crate::ANDROID_APP_VERSION.to_string(),
        // Multiuser stays on the legacy pull download permanently (PROTOCOL-ROADMAP.md
        // Phase J). The push protocol is driven entirely by per-world sequence numbers and
        // multiuser has none - it emits `seq: 0, end_seq: None` on every line and skips the
        // ack audit outright - so there is nothing for a newest-first walk to walk. Reporting
        // `false` keeps multiuser clients on `RequestScrollback`, which does work here, in
        // preference to advertising the capability and then refusing every request.
        scrollback_push: false,
    }
}

/// Generate splash screen content as strings (for web client)
pub fn generate_splash_strings() -> Vec<String> {
    vec![
        "".to_string(),
        "\x1b[38;5;180m          (\\/\\__o     \x1b[38;5;209m \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2557}      \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2557}   \u{2588}\u{2588}\u{2557}\x1b[0m".to_string(),
        "\x1b[38;5;180m  __      `-/ `_/     \x1b[38;5;208m\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2557}\u{255a}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2554}\u{255d}\x1b[0m".to_string(),
        "\x1b[38;5;180m `--\\______/  |       \x1b[38;5;215m\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2551} \u{255a}\u{2588}\u{2588}\u{2588}\u{2588}\u{2554}\u{255d} \x1b[0m".to_string(),
        "\x1b[38;5;180m    /        /        \x1b[38;5;216m\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2551}  \u{255a}\u{2588}\u{2588}\u{2554}\u{255d}  \x1b[0m".to_string(),
        "\x1b[38;5;180m -`/_------'\\_.       \x1b[38;5;217m\u{255a}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2551}  \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2551}   \x1b[0m".to_string(),
        "\x1b[38;5;218m                       \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{255d}  \u{255a}\u{2550}\u{255d}   \u{255a}\u{2550}\u{255d}   \x1b[0m".to_string(),
        "".to_string(),
        "\x1b[38;5;213m\u{2728} A 90dies mud client written today \u{2728}\x1b[0m".to_string(),
        "".to_string(),
        "\x1b[38;5;244mSelect a world to connect\x1b[0m".to_string(),
        "".to_string(),
    ]
}

/// Handle WebSocket message in multiuser mode
/// Filters a `(world_index, seq)` list down to entries whose world is owned by
/// `uname` (PROTOCOL-ROADMAP.md Step 6a). Shared by the `AuthRequest.resume` and
/// `PongCheck.acked` arms of `handle_multiuser_ws_message` below so neither can
/// seed/record state for — or, via `handle_request_scrollback_owned`, replay from —
/// a `world_index` belonging to another user.
fn owner_filtered_pairs(app: &App, uname: &str, pairs: &[(usize, u64)]) -> Vec<(usize, u64)> {
    pairs.iter()
        .filter(|(wi, _)| app.worlds.get(*wi).map(|w| w.owner.as_deref() == Some(uname)).unwrap_or(false))
        .cloned()
        .collect()
}

/// Broadcast the actual text of just-released pending lines to one owner's clients
/// (multiuser `ReleasePending`/`SelectiveFlush` - see their call sites below). Deliberately
/// NOT `App::broadcast_released_lines`/`ws_broadcast_to_world`, which fan out to every
/// connected client regardless of world ownership and would leak this owner's MUD output to
/// other users (CLAUDE.md's D7 owner-scoping invariant - see `ConnectWorld`/`SwitchWorld`'s
/// `world.owner == username` checks elsewhere in this file for the same rule). Grouped by
/// `(from_server, gagged)` like the single-user path so gagged status survives release intact
/// (the same bug class fixed for `App::broadcast_released_lines` in v1.5.8). Always uses
/// `seq: 0, end_seq: None` - multiuser never sends real per-line seqs today (its `World`
/// instances hold only owner-agnostic template data; the real per-user MUD data lives in
/// `UserConnection`), so this stays consistent with every other multiuser `ServerData`
/// broadcast rather than introducing multiuser's first real-seq broadcast.
fn broadcast_owner_scoped_released_lines(ws: &WebSocketServer, world_index: usize, owner: Option<&str>, released: &[OutputLine]) {
    if released.is_empty() {
        return;
    }
    let ts = current_timestamp_secs();
    let mut batch: Vec<String> = Vec::new();
    let mut batch_from_server = released[0].from_server;
    let mut batch_gagged = released[0].gagged;
    for line in released {
        if (line.from_server != batch_from_server || line.gagged != batch_gagged) && !batch.is_empty() {
            let ws_data = batch.join("\n") + "\n";
            ws.broadcast_to_owner(WsMessage::ServerData { archive_sourced: false,
                world_index,
                data: ws_data,
                is_viewed: true,
                ts,
                from_server: batch_from_server,
                seq: 0, end_seq: None,
                flush: false, gagged: batch_gagged, highlight_colors: Vec::new(),
            }, owner);
            batch.clear();
            batch_from_server = line.from_server;
            batch_gagged = line.gagged;
        }
        batch.push(line.text.replace('\r', ""));
    }
    if !batch.is_empty() {
        let ws_data = batch.join("\n") + "\n";
        ws.broadcast_to_owner(WsMessage::ServerData { archive_sourced: false,
            world_index,
            data: ws_data,
            is_viewed: true,
            ts,
            from_server: batch_from_server,
            seq: 0, end_seq: None,
            flush: false, gagged: batch_gagged, highlight_colors: Vec::new(),
        }, owner);
    }
}

pub async fn handle_multiuser_ws_message(
    app: &mut App,
    client_id: u64,
    msg: WsMessage,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    // Get the username for this client
    let username = if let Some(ws) = &app.ws_server {
        ws.get_client_username(client_id)
    } else {
        None
    };

    match msg {
        WsMessage::AuthRequest { ref resume, .. } => {
            // Client just authenticated - send them their InitialState filtered by username
            if let Some(ref uname) = username {
                let initial_state = build_multiuser_initial_state(app, uname);
                if let Some(ws) = &app.ws_server {
                    ws.send_initial_state_and_mark(client_id, initial_state);
                }
                // Resume-driven replay (PROTOCOL-ROADMAP.md Step 6a): owner-filtered
                // first, so a user's `resume` list can only ever seed acked_seq for, and
                // replay scrollback from, worlds they own — closes the gap left open by
                // Step 2 (see that step's Status note and handle_request_scrollback_owned).
                if !resume.is_empty() {
                    let owned = owner_filtered_pairs(app, uname, resume);
                    if !owned.is_empty() {
                        if let Some(ref server) = app.ws_server {
                            server.record_acked_seq(client_id, &owned);
                        }
                        // Mirrors the in-memory scrollback ring's cap (MAX_LINES, main.rs)
                        // so a single replay always covers the entire ring if needed.
                        const RESUME_REPLAY_MAX: usize = 10_000;
                        for (world_index, last_seq) in owned {
                            // request_id: Some(0) is the reserved value marking a
                            // server-initiated unprompted resume replay (see
                            // ScrollbackLines' doc comment in websocket.rs).
                            app.handle_request_scrollback_owned(client_id, world_index, RESUME_REPLAY_MAX, None, Some(last_seq), Some(0), uname);
                        }
                    }
                }
            }
        }
        WsMessage::RequestScrollback { world_index, count, before_seq, after_seq, request_id } => {
            // Owner-scoped (PROTOCOL-ROADMAP.md Step 6a) — multiuser previously had no
            // handler for this at all, so a client couldn't scroll back further than its
            // initial state. Reuses the same owner check as the AuthRequest.resume path.
            if let Some(ref uname) = username {
                app.handle_request_scrollback_owned(client_id, world_index, count, before_seq, after_seq, request_id, uname);
            }
        }
        WsMessage::PongCheck { acked, .. } => {
            // Record the client's per-world delivery ack (PROTOCOL-ROADMAP.md Step 6a),
            // mirroring the single-user master-WS/`-D` daemon PongCheck handlers
            // (multiuser previously had no handler for this at all). No owner check is
            // strictly required to ack your own claimed progress — acked_seq/needs_resync
            // bookkeeping is per-client (WsClientInfo) and ResyncRequired delivery only
            // ever targets worlds actually broadcast to this client via
            // broadcast_to_owner, i.e. this client's own worlds — but entries are still
            // owner-filtered as defense-in-depth so a bogus world_index in `acked` can't
            // write into this client's per-world state for a world it doesn't own.
            if !acked.is_empty() {
                if let Some(ref uname) = username {
                    let owned = owner_filtered_pairs(app, uname, &acked);
                    if !owned.is_empty() {
                        if let Some(ref server) = app.ws_server {
                            server.record_acked_seq(client_id, &owned);
                        }
                    }
                }
            }
            // Deliberately NO App::audit_client_acks() here, unlike the two single-user
            // PongCheck handlers (PROTOCOL-ROADMAP.md Phase C). Multiuser emits
            // `seq: 0, end_seq: None` on every ServerData (see build_multiuser_initial_state
            // and the ~60 broadcast sites), so a client's ack and the server's
            // deliverable_high_seq are not measured in the same units - auditing here would
            // compare a real seq against a constant 0 and fire nonstop. Multiuser's missing
            // seq support is tracked separately in PROTOCOL-ROADMAP.md.
        }
        WsMessage::SendCommand { world_index, command } => {
            // Send command to user's own connection
            if let Some(ref uname) = username {
                let key = (world_index, uname.clone());
                if let Some(conn) = app.user_connections.get(&key) {
                    if let Some(tx) = &conn.command_tx {
                        let _ = tx.send(WriteCommand::Text(command)).await;
                    }
                }
            }
        }
        WsMessage::ConnectWorld { world_index } => {
            // Verify the client owns this world (mirrors SwitchWorld/MarkWorldSeen/
            // ReleasePending below). Without this check, any authenticated user could
            // request a connect on another user's world_index and
            // connect_multiuser_world() would auto-replay that world's stored
            // `connect <user> <password>` credential, seizing the owner's character.
            if let Some(world) = app.worlds.get(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    if let Some(ref uname) = username {
                        let _ = event_tx.send(AppEvent::ConnectWorldRequest(world_index, uname.clone())).await;
                    }
                }
            }
        }
        WsMessage::DisconnectWorld { world_index } => {
            // Disconnect user's own connection. Job 9 (T3.2): shares
            // handle_multiuser_disconnect with AppEvent::MultiuserDisconnected (a real
            // socket-side drop) so a user-initiated disconnect gets the exact same
            // ProtocolState::clear() + unmask-if-was-masked treatment - a manually
            // disconnected, still-masked connection must not leave a web client's input
            // box stuck masked either.
            if let Some(ref uname) = username {
                handle_multiuser_disconnect(app, world_index, uname);
            }
        }
        WsMessage::ChangePassword { old_password_hash, new_password_hash } => {
            if let Some(ref uname) = username {
                // Find the user and verify old password
                if let Some(user) = app.users.iter_mut().find(|u| &u.name == uname) {
                    // C1 (security remediation): `user.password` may itself already be
                    // an already-hashed value from a prior password change
                    // (password_is_hash) — only hash it here if it's still plaintext,
                    // otherwise compare it directly. Hashing an already-hashed value
                    // again would make every subsequent password change compare against
                    // the wrong old hash too.
                    let old_hash = if user.password_is_hash {
                        user.password.clone()
                    } else {
                        hash_password(&user.password)
                    };
                    if old_hash == old_password_hash {
                        // Store the new credential as an already-hashed value (the
                        // client only ever sends SHA256(new_password), never the
                        // plaintext, so it cannot be stored as a plaintext `password`
                        // without being re-hashed — and therefore corrupted — the next
                        // time settings are loaded; see User::password_is_hash).
                        user.password = new_password_hash.clone();
                        user.password_is_hash = true;
                        // Update the live WebSocket server's credential immediately so
                        // the running server accepts the new password right away,
                        // without requiring a reload.
                        if let Some(ws) = &app.ws_server {
                            ws.set_user_password_hash(uname, new_password_hash.clone());
                        }
                        // Save settings
                        if let Err(e) = persistence::save_multiuser_settings(app) {
                            eprintln!("Failed to save settings after password change: {}", e);
                        }
                        // Send success response
                        if let Some(ws) = &app.ws_server {
                            ws.send_to_client(client_id, WsMessage::PasswordChanged {
                                success: true,
                                error: None,
                            });
                        }
                    } else {
                        // Wrong old password
                        if let Some(ws) = &app.ws_server {
                            ws.send_to_client(client_id, WsMessage::PasswordChanged {
                                success: false,
                                error: Some("Invalid current password".to_string()),
                            });
                        }
                    }
                }
            }
        }
        WsMessage::Logout => {
            if let Some(ref uname) = username {
                // Close all connections for this user
                let keys_to_remove: Vec<_> = app.user_connections.keys()
                    .filter(|(_, u)| u == uname)
                    .cloned()
                    .collect();

                for key in &keys_to_remove {
                    // Send shutdown command to gracefully close the TCP connection
                    if let Some(conn) = app.user_connections.get(key) {
                        if let Some(tx) = &conn.command_tx {
                            let _ = tx.try_send(WriteCommand::Shutdown);
                        }
                    }
                }

                for key in keys_to_remove {
                    // Remove the connection entry
                    app.user_connections.remove(&key);
                }

                // Clear the client's authentication state
                if let Some(ws) = &app.ws_server {
                    ws.clear_client_auth(client_id);
                    // Send LoggedOut response
                    ws.send_to_client(client_id, WsMessage::LoggedOut);
                }
            }
        }
        WsMessage::RequestState => {
            // Client requests full state resync
            if let Some(ref uname) = username {
                let initial_state = build_multiuser_initial_state(app, uname);
                if let Some(ws) = &app.ws_server {
                    ws.send_initial_state_and_mark(client_id, initial_state);
                }
                // Also send activity count and pause state, matching the single-user
                // RequestState handler (App::handle_request_state, main.rs) - previously
                // omitted here entirely (CLAUDE.md's three-dispatch-path rule). Computed
                // per-user rather than reusing App::activity_count()/a shared paused flag:
                // those iterate/read global World state with no owner filtering, which would
                // leak another user's unseen-activity count or pause state across the
                // multiuser boundary. current_world_index mirrors
                // build_multiuser_initial_state's own "first world this user is connected
                // to" convention, so the reported paused state matches what the InitialState
                // just sent.
                let activity_count = app.worlds.iter().enumerate()
                    .filter(|(idx, _)| {
                        app.user_connections.get(&(*idx, uname.clone()))
                            .map(|c| c.unseen_lines > 0)
                            .unwrap_or(false)
                    })
                    .count();
                if let Some(ws) = &app.ws_server {
                    ws.send_to_client(client_id, WsMessage::ActivityUpdate { count: activity_count });
                }
                let current_world_index = app.user_connections.keys()
                    .filter(|(_, u)| u == uname)
                    .map(|(idx, _)| *idx)
                    .min();
                let is_paused = current_world_index
                    .and_then(|idx| app.user_connections.get(&(idx, uname.clone())))
                    .map(|c| c.paused)
                    .unwrap_or(false);
                if is_paused {
                    if let Some(ws) = &app.ws_server {
                        ws.send_to_client(client_id, WsMessage::PausedState { paused: true });
                    }
                }
            }
        }
        WsMessage::SwitchWorld { world_index } => {
            // Verify the client owns this world
            if let Some(world) = app.worlds.get(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    // Send WorldSwitched message to the client
                    if let Some(ws) = &app.ws_server {
                        ws.send_to_client(client_id, WsMessage::WorldSwitched { new_index: world_index });
                    }
                }
            }
        }
        // Job 9 (T3.2): multiuser previously tracked no client dimensions at all - see
        // App::user_min_dimensions's doc comment. Mirrors the single-user `-D` daemon
        // handler's UpdateViewState/UpdateDimensions arms (handle_daemon_ws_message
        // above), owner-gated per CLAUDE.md's rule since world_index is client-supplied.
        WsMessage::UpdateViewState { world_index, visible_lines, visible_columns } => {
            let is_owner = app.worlds.get(world_index)
                .map(|w| w.owner.as_ref() == username.as_ref())
                .unwrap_or(false);
            if is_owner {
                let dimensions = app.ws_client_worlds.get(&client_id).and_then(|s| s.dimensions);
                let vc = visible_columns.unwrap_or_else(|| app.ws_client_worlds.get(&client_id).map(|v| v.visible_columns).unwrap_or(0));
                let paused = app.ws_client_worlds.get(&client_id).map(|v| v.paused).unwrap_or(false);
                let visible = app.ws_client_worlds.get(&client_id).map(|v| v.visible).unwrap_or(true);
                app.ws_client_worlds.insert(client_id, ClientViewState { world_index, visible_lines, visible_columns: vc, dimensions, paused, visible, disconnected_at: None });
            }
        }
        WsMessage::UpdateDimensions { width, height } => {
            if let Some(ref uname) = username {
                if let Some(state) = app.ws_client_worlds.get_mut(&client_id) {
                    let old_dims = state.dimensions;
                    state.dimensions = Some((width, height));
                    if old_dims != Some((width, height)) {
                        app.send_naws_to_all_multiuser_worlds(uname);
                    }
                }
            }
        }
        // Note editor: verify ownership before reading or writing another user's
        // world notes (mirrors SwitchWorld above).
        WsMessage::RequestNoteEditorState { world_index } => {
            if let Some(world) = app.worlds.get(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    if let Some(ws) = &app.ws_server {
                        ws.send_to_client(client_id, WsMessage::NoteEditorState {
                            world_index,
                            world_name: world.name.clone(),
                            notes: world.settings.notes.clone(),
                        });
                    }
                }
            }
        }
        WsMessage::UpdateNote { world_index, notes } => {
            if let Some(world) = app.worlds.get_mut(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    world.settings.notes = notes;
                    let has_notes = !world.settings.notes.is_empty();
                    let owner = world.owner.clone();
                    let _ = persistence::save_settings(app);
                    if let Some(ws) = &app.ws_server {
                        ws.broadcast_to_owner(WsMessage::NotesChanged { world_index, has_notes }, owner.as_deref());
                    }
                }
            }
        }
        // MCP simpleedit reply (plan Job 15): verify ownership before sending down
        // another user's world connection, same reasoning as the note editor above.
        WsMessage::McpEditSet { world_index, reference, edit_type, content } => {
            if let Some(world) = app.worlds.get_mut(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    match world.mcp.build_simpleedit_set(&reference, mcp::SimpleEditType::parse(&edit_type), &content) {
                        Some(lines) => {
                            for line in lines {
                                app.send_to_world(world_index, line);
                            }
                        }
                        None => app.emit_client_text(world_index, "Could not send the edited text: the MCP session is no longer available.", true),
                    }
                }
            }
        }
        WsMessage::MarkWorldSeen { world_index, previous_world_index } => {
            // Verify the client owns this world
            if let Some(world) = app.worlds.get_mut(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    // mark_seen() also resets first_unseen_at (unlike the bare
                    // `unseen_lines = 0` this replaced), matching the single-user path's
                    // World::mark_seen().
                    world.mark_seen();
                    let owner = world.owner.clone();
                    // Clear the previous world's new-line indicators too, same as the
                    // single-user handle_mark_world_seen - only meaningful when the client
                    // also owns that world (an owner check, not just an index bound check,
                    // since world_index in this message is client-supplied - see
                    // CLAUDE.md's D7 ConnectWorld/SwitchWorld ownership invariant).
                    if let Some(old_idx) = previous_world_index {
                        if old_idx != world_index {
                            if let Some(old_world) = app.worlds.get_mut(old_idx) {
                                if old_world.owner.as_ref() == username.as_ref() {
                                    // Drop only THIS client's ▶ markers on the world it
                                    // left - another user's markers on their own worlds are
                                    // untouched (per-line ownership, OutputLine::display_id).
                                    old_world.release_claims(client_id);
                                    if old_world.pending_lines.is_empty() {
                                        old_world.lines_since_pause = 0;
                                    }
                                }
                            }
                        }
                    }
                    // Broadcast to all clients of this owner
                    if let Some(ws) = &app.ws_server {
                        ws.broadcast_to_owner(WsMessage::UnseenCleared { world_index }, owner.as_deref());
                    }
                    // Owner-scoped activity count, mirroring RequestState's per-user
                    // computation above - NOT App::broadcast_activity()/activity_count(),
                    // which read global World state with no owner filtering and would leak
                    // another user's activity across the multiuser boundary.
                    // Note: UserConnection::unseen_lines (the field this count *should*
                    // read, matching RequestState's) is never incremented anywhere in
                    // multiuser mode, so it's permanently 0 - a pre-existing bug, out of
                    // scope here. Recomputed from World::has_activity() (owner-filtered)
                    // instead so this broadcast isn't equally dead on arrival.
                    if let Some(ref uname) = username {
                        let activity_count = app.worlds.iter()
                            .filter(|w| w.owner.as_deref() == Some(uname.as_str()) && w.has_activity())
                            .count();
                        if let Some(ws) = &app.ws_server {
                            ws.broadcast_to_owner(WsMessage::ActivityUpdate { count: activity_count }, Some(uname.as_str()));
                        }
                    }
                }
            }
        }
        WsMessage::ReleasePending { world_index, count } => {
            // Verify the client owns this world
            if let Some(world) = app.worlds.get_mut(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    let release_count = if count == 0 { world.pending_lines.len() } else { count.min(world.pending_lines.len()) };
                    let released: Vec<OutputLine> = world.pending_lines.drain(..release_count).collect();
                    world.output_lines.extend(released.iter().cloned());

                    if world.pending_lines.is_empty() {
                        world.paused = false;
                    }

                    let owner = world.owner.clone();
                    // Broadcast to all clients of this owner
                    if let Some(ws) = &app.ws_server {
                        // Send the actual released text first, then the count update - a
                        // client that only sees PendingReleased's count would clear its
                        // "More" indicator with no matching content ever having arrived
                        // (the exact "indicator clears, no output appears" symptom this
                        // whole audit round is about, just scoped to multiuser mode).
                        broadcast_owner_scoped_released_lines(ws, world_index, owner.as_deref(), &released);
                        ws.broadcast_to_owner(WsMessage::PendingReleased { world_index, count: release_count }, owner.as_deref());
                    }
                }
            }
        }
        WsMessage::TrustCertificate { world_index, host, new_fingerprint } => {
            // Verify the client owns this world (mirrors ConnectWorld above) before
            // re-pinning and reconnecting on their behalf.
            if let Some(world) = app.worlds.get(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    persistence::replace_pin(&host, &new_fingerprint);
                    if let Some(ref uname) = username {
                        let _ = event_tx.send(AppEvent::ConnectWorldRequest(world_index, uname.clone())).await;
                    }
                }
            }
        }
        WsMessage::SelectiveFlush { world_index } => {
            if let Some(world) = app.worlds.get_mut(world_index) {
                if world.owner.as_ref() == username.as_ref() && world.paused {
                    let pending = std::mem::take(&mut world.pending_lines);
                    let kept: Vec<OutputLine> = pending.into_iter().filter(|l| l.highlight_color.is_some()).collect();
                    world.output_lines.extend(kept.iter().cloned());
                    world.paused = false;
                    world.lines_since_pause = 0;
                    let owner = world.owner.clone();
                    if let Some(ws) = &app.ws_server {
                        // Same reasoning as ReleasePending above: broadcast the kept lines'
                        // actual text before the count update, not just the count.
                        broadcast_owner_scoped_released_lines(ws, world_index, owner.as_deref(), &kept);
                        ws.broadcast_to_owner(WsMessage::PendingLinesUpdate { world_index, count: 0 }, owner.as_deref());
                    }
                }
            }
        }
        WsMessage::CalculateNextWorld { current_index } | WsMessage::CalculatePrevWorld { current_index } => {
            // Calculate next/prev world owned by this user
            if let Some(ref uname) = username {
                let user_worlds: Vec<usize> = app.worlds.iter().enumerate()
                    .filter(|(_, w)| w.owner.as_deref() == Some(uname))
                    .map(|(idx, _)| idx)
                    .collect();

                let current_pos = user_worlds.iter().position(|&idx| idx == current_index);
                let next_index = match msg {
                    WsMessage::CalculateNextWorld { .. } => {
                        current_pos.map(|p| user_worlds[(p + 1) % user_worlds.len()])
                    }
                    WsMessage::CalculatePrevWorld { .. } => {
                        current_pos.map(|p| {
                            if p == 0 { user_worlds[user_worlds.len() - 1] }
                            else { user_worlds[p - 1] }
                        })
                    }
                    _ => None,
                };

                if let Some(ws) = &app.ws_server {
                    ws.send_to_client(client_id, WsMessage::CalculatedWorld { index: next_index });
                }
            }
        }
        // Reject world editing in multiuser mode
        WsMessage::UpdateWorldSettings { .. } | WsMessage::DeleteWorld { .. } | WsMessage::CreateWorld { .. } => {
            // Silently reject - users can't edit worlds in multiuser mode
        }
        WsMessage::ReportSeqMismatch { world_index, expected_seq_gt, actual_seq, line_text, source } => {
            // Always-on (not gated behind is_debug_enabled()) — see the single-user daemon
            // handler above for why (D-Termux-lines investigation).
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("SEQ-MISMATCH", &ip, &format!("[{}] in '{}': expected seq>{}, got seq={}, text={:?}",
                source, world_name, expected_seq_gt, actual_seq,
                line_text.chars().take(80).collect::<String>()));
        }
        WsMessage::ReportDuplicate { world_index, line_seq, max_seq, line_text, source } => {
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("DUPLICATE", &ip, &format!("[{}] in '{}': line_seq={}, max_seq={}, text={:?}",
                source, world_name, line_seq, max_seq,
                line_text.chars().take(200).collect::<String>()));
        }
        WsMessage::ReportOutOfOrder { world_index, line_seq, recovered_count, source } => {
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("OUT-OF-ORDER", &ip, &format!("[{}] in '{}': recovered {} line(s) starting at seq={} that had arrived out of order",
                source, world_name, recovered_count, line_seq));
        }
        WsMessage::ReportGap { world_index, hole_start, hole_end, attempts, source } => {
            // Always-on, same reasoning as the sibling reports above: only fires when a
            // client has genuinely given up on a range of output, which is exactly the
            // event that has been invisible in the field (PROTOCOL-ROADMAP.md Phase F).
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("SEQ-HOLE", &ip, &format!("[{}] in '{}': gave up on seq {}..={} ({} line(s)) after {} gap-fill attempt(s) returned nothing for it",
                source, world_name, hole_start, hole_end,
                hole_end.saturating_sub(hole_start).saturating_add(1), attempts));
        }
        WsMessage::ReportClientLifecycle { event, detail, source } => {
            // Same reasoning as the sibling reports above; see WsMessage::ReportClientLifecycle
            // in websocket.rs for why an Android lifecycle transition is worth a server log line.
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("CLIENT-LIFECYCLE", &ip, &format!("[{}] {}: {}", source, event, detail));
        }
        WsMessage::ToggleWorldGmcp { world_index } => {
            if world_index < app.worlds.len() {
                app.worlds[world_index].gmcp_user_enabled = !app.worlds[world_index].gmcp_user_enabled;
                if !app.worlds[world_index].gmcp_user_enabled {
                    app.stop_world_media(world_index);
                }
                app.needs_output_redraw = true;
                app.ws_broadcast(WsMessage::GmcpUserToggled {
                    world_index,
                    enabled: app.worlds[world_index].gmcp_user_enabled,
                });
            }
        }
        // Plan Job 13 (Phase 4, 4.4): folded into one shared function - see
        // commands::execute_send_gmcp/execute_send_msdp.
        WsMessage::SendGmcp { world_index, package, data } => {
            execute_send_gmcp(app, world_index, &package, &data);
        }
        WsMessage::SendMsdp { world_index, variable, value } => {
            execute_send_msdp(app, world_index, &variable, &value);
        }
        _ => {} // Handle other messages as needed
    }
}

#[cfg(test)]
mod change_password_tests {
    use super::*;

    /// Register a fake authenticated WS client so `get_client_username` resolves.
    /// Returns the client id and a receiver for whatever the handler sends back.
    /// Item type is `Outbound`, not `WsMessage` (PROTOCOL-ROADMAP.md Step 8) - matches
    /// the real per-client channel's item type since `WsClientInfo.tx` changed.
    fn register_client(server: &WebSocketServer, client_id: u64, username: &str) -> mpsc::Receiver<crate::websocket::Outbound> {
        // Bounded (PROTOCOL-ROADMAP.md Step 3) — matches the real per-client channel created
        // in `handle_ws_client`.
        let (tx, rx) = mpsc::channel::<crate::websocket::Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        // `clients` is a std::sync::RwLock (see WebSocketServer::clients) — try_write here
        // is just for the "uncontended in test setup" assertion, not async-safety.
        let mut clients = server.clients.try_write().expect("clients lock should be uncontended in test setup");
        clients.insert(client_id, WsClientInfo {
            authenticated: true,
            tx,
            current_world: None,
            username: Some(username.to_string()),
            received_initial_state: true,
            client_type: RemoteClientType::Web,
            viewport_height: 24,
            ip_address: "127.0.0.1".to_string(),
            connected_at: std::time::Instant::now(),
            last_activity: std::time::Instant::now(),
            paused: false,
            acked_seq: std::collections::HashMap::new(),
            audit_prev_acked: std::collections::HashMap::new(),
            audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
            needs_resync: std::collections::HashSet::new(),
        });
        rx
    }

    /// C1 (security remediation) regression test: before the fix, `ChangePassword`
    /// stored the client-sent hash into the plaintext `User.password` field, which was
    /// then re-hashed (`SHA256(SHA256(pw))`) the next time credentials were installed
    /// on a `WebSocketServer` (originally only at startup/reload) — permanently locking
    /// the user out of the new password. This drives the actual `ChangePassword`
    /// handler and then reinstalls credentials the same way a restart/reload would
    /// (`install_user_credentials`, shared with `run_multiuser_server`), proving a
    /// login with the new password succeeds both immediately (live server) and after
    /// a reload (persisted `password_is_hash` form).
    #[tokio::test]
    async fn change_password_then_login_with_new_password_succeeds() {
        let old_password = "correct horse battery staple";
        let new_password = "new-super-secret-password";

        let mut app = App::new();
        app.multiuser_mode = true;
        app.users.push(User::new("alice", old_password));

        let server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        install_user_credentials(&server, &app.users);

        let client_id = 1u64;
        let mut rx = register_client(&server, client_id, "alice");
        app.ws_server = Some(server);

        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(8);

        // Change the password via the real handler, exactly as a client would trigger it.
        let msg = WsMessage::ChangePassword {
            old_password_hash: hash_password(old_password),
            new_password_hash: hash_password(new_password),
        };
        handle_multiuser_ws_message(&mut app, client_id, msg, &event_tx).await;

        // Handler must report success. PasswordChanged is a single-recipient send, so it
        // always arrives as Outbound::Message (PROTOCOL-ROADMAP.md Step 8).
        match rx.try_recv() {
            Ok(crate::websocket::Outbound::Message(msg)) => match *msg {
                WsMessage::PasswordChanged { success, error } => {
                    assert!(success, "ChangePassword should succeed, got error: {:?}", error);
                }
                other => panic!("expected PasswordChanged, got {:?}", other),
            },
            other => panic!("expected PasswordChanged, got {:?}", other),
        }

        // In-memory User record must now hold the new credential in hashed form.
        let user = app.users.iter().find(|u| u.name == "alice").unwrap();
        assert!(user.password_is_hash, "password_is_hash should be set after a change");
        assert_eq!(user.password, hash_password(new_password));

        // The LIVE server's credential must already accept the new password —
        // no reload required (this is what makes an immediate reconnect work).
        {
            let ws = app.ws_server.as_ref().unwrap();
            let users = ws.users.read().unwrap();
            let cred = users.get("alice").expect("alice should still be a known user");
            assert_eq!(
                cred.password_hash,
                hash_password(new_password),
                "live server credential must match the NEW password, not a double-hash of it"
            );
            assert_ne!(
                cred.password_hash,
                hash_password(&hash_password(new_password)),
                "live server credential must not be double-hashed (the original C1 bug)"
            );
        }

        // Simulate a restart/reload: reinstall credentials from the persisted User
        // records the same way run_multiuser_server does at startup.
        let reloaded_server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        install_user_credentials(&reloaded_server, &app.users);
        let users = reloaded_server.users.read().unwrap();
        let cred = users.get("alice").expect("alice should survive a reload");
        assert_eq!(
            cred.password_hash,
            hash_password(new_password),
            "after a reload, login with the NEW password must still succeed"
        );

        // And the OLD password must no longer work, live or after reload.
        assert_ne!(cred.password_hash, hash_password(old_password));
    }

    /// A second, consecutive password change must also work — guards against a fix
    /// that only handles the plaintext-to-hash transition once (e.g. by
    /// unconditionally hashing `user.password` for the old-password check instead of
    /// checking `password_is_hash`).
    #[tokio::test]
    async fn second_consecutive_password_change_succeeds() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.users.push(User::new("bob", "first-password"));

        let server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        install_user_credentials(&server, &app.users);
        let client_id = 1u64;
        let _rx1 = register_client(&server, client_id, "bob");
        app.ws_server = Some(server);
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(8);

        // First change: first-password -> second-password
        handle_multiuser_ws_message(&mut app, client_id, WsMessage::ChangePassword {
            old_password_hash: hash_password("first-password"),
            new_password_hash: hash_password("second-password"),
        }, &event_tx).await;
        assert!(app.users[0].password_is_hash);
        assert_eq!(app.users[0].password, hash_password("second-password"));

        // Second change: second-password -> third-password. Must authenticate against
        // the (already-hashed) current password correctly.
        handle_multiuser_ws_message(&mut app, client_id, WsMessage::ChangePassword {
            old_password_hash: hash_password("second-password"),
            new_password_hash: hash_password("third-password"),
        }, &event_tx).await;

        let ws = app.ws_server.as_ref().unwrap();
        assert_eq!(app.users[0].password, hash_password("third-password"));
        let users = ws.users.read().unwrap();
        assert_eq!(users.get("bob").unwrap().password_hash, hash_password("third-password"));
    }
}

#[cfg(test)]
mod resume_owner_scoping_tests {
    // PROTOCOL-ROADMAP.md Step 6a: multiuser's AuthRequest.resume / RequestScrollback /
    // PongCheck handlers in `handle_multiuser_ws_message` must never let one user pull
    // scrollback from a world they don't own.
    use super::*;

    /// Registers a fake authenticated WS client the same way
    /// `change_password_tests::register_client` does (bounded per-client channel matching
    /// the real `handle_ws_client` setup), returning the receiver so a test can inspect
    /// exactly what the handler sent back.
    fn register_client(server: &WebSocketServer, client_id: u64, username: &str) -> mpsc::Receiver<crate::websocket::Outbound> {
        // Item type is `Outbound`, not `WsMessage` (PROTOCOL-ROADMAP.md Step 8).
        let (tx, rx) = mpsc::channel::<crate::websocket::Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let mut clients = server.clients.try_write().expect("clients lock should be uncontended in test setup");
        clients.insert(client_id, WsClientInfo {
            authenticated: true,
            tx,
            current_world: None,
            username: Some(username.to_string()),
            received_initial_state: false,
            client_type: RemoteClientType::Web,
            viewport_height: 24,
            ip_address: "127.0.0.1".to_string(),
            connected_at: std::time::Instant::now(),
            last_activity: std::time::Instant::now(),
            paused: false,
            acked_seq: std::collections::HashMap::new(),
            audit_prev_acked: std::collections::HashMap::new(),
            audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
            needs_resync: std::collections::HashSet::new(),
        });
        rx
    }

    /// Builds a two-user, two-world multiuser `App`: world 0 owned by "alice", world 1
    /// owned by "bob", each pre-populated with output_lines seq 1..=10 so a resume/
    /// scrollback replay has real data to (not) leak.
    fn two_owner_app() -> App {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();

        let mut alice_world = World::new("alice-world");
        alice_world.owner = Some("alice".to_string());
        for seq in 1..=10u64 {
            alice_world.output_lines.push(OutputLine::new(format!("alice secret line {seq}"), seq));
        }
        app.worlds.push(alice_world);

        let mut bob_world = World::new("bob-world");
        bob_world.owner = Some("bob".to_string());
        for seq in 1..=10u64 {
            bob_world.output_lines.push(OutputLine::new(format!("bob secret line {seq}"), seq));
        }
        app.worlds.push(bob_world);

        app.current_world_index = 0;
        app
    }

    /// Drains `rx` and returns every `ScrollbackLines` payload received, as
    /// `(world_index, seqs)` pairs, in receipt order. `ScrollbackLines` is always sent
    /// per-client (`Outbound::Message`, PROTOCOL-ROADMAP.md Step 8) - any `Outbound::Shared`
    /// item drained here (there shouldn't be any in these tests) is simply not a
    /// `ScrollbackLines` and is skipped.
    fn drain_scrollback_replies(rx: &mut mpsc::Receiver<crate::websocket::Outbound>) -> Vec<(usize, Vec<u64>)> {
        let mut out = Vec::new();
        while let Ok(item) = rx.try_recv() {
            if let crate::websocket::Outbound::Message(msg) = item {
                if let WsMessage::ScrollbackLines { world_index, lines, .. } = *msg {
                    out.push((world_index, lines.iter().map(|l| l.seq).collect()));
                }
            }
        }
        out
    }

    /// The security property this step exists to establish: alice's `resume` list (sent
    /// on her own `AuthRequest`) names bob's `world_index` (1). She must receive nothing
    /// for it — no `ScrollbackLines` for world 1 at all, i.e. bob's MUD output never
    /// reaches her socket. Also drives the same attack via `RequestScrollback` directly,
    /// since Step 6a wires both through the same owner-scoped path.
    ///
    /// This test is NOT vacuous against the pre-fix code: before this step,
    /// `handle_multiuser_ws_message`'s `AuthRequest` arm didn't touch `resume` at all (so
    /// this exact leak was merely latent/unwired, not yet reachable) and had no
    /// `RequestScrollback` arm whatsoever (it fell into the `_ => {}` catch-all). Naively
    /// wiring `resume`/`RequestScrollback` straight into
    /// `App::handle_request_scrollback` without the new owner check — the literal bug
    /// this step fixes — would make this test fail: it would deliver bob's lines to
    /// alice's channel. Confirmed by temporarily reverting the owner-check call sites to
    /// call `handle_request_scrollback` directly during development; restored before
    /// landing.
    #[tokio::test]
    async fn resume_and_request_scrollback_cannot_read_another_users_world() {
        let mut app = two_owner_app();

        let server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        let alice_id = 1u64;
        let mut alice_rx = register_client(&server, alice_id, "alice");
        app.ws_server = Some(server);
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(8);

        // Attack 1: AuthRequest.resume names bob's world_index (1).
        handle_multiuser_ws_message(&mut app, alice_id, WsMessage::AuthRequest {
            username: Some("alice".to_string()),
            password_hash: String::new(),
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: false,
            resume: vec![(1, 0)], resume_epochs: Vec::new(), client_version: String::new(), client_uid: String::new(),
        }, &event_tx).await;

        let replies = drain_scrollback_replies(&mut alice_rx);
        assert!(replies.iter().all(|(wi, _)| *wi != 1),
            "alice must never receive ScrollbackLines for bob's world_index via AuthRequest.resume, got {replies:?}");

        // Bob's world_index must not have been seeded into alice's acked_seq either -
        // that would be state about a world she doesn't own leaking into her session.
        {
            let clients = app.ws_server.as_ref().unwrap().clients.read().unwrap();
            let alice_client = clients.get(&alice_id).unwrap();
            assert!(!alice_client.acked_seq.contains_key(&1),
                "acked_seq must not be seeded for a world_index alice doesn't own");
        }

        // Attack 2: same thing, but via a direct RequestScrollback (the pre-existing gap
        // this step also closes) instead of resume. request_id is a real, non-None value
        // here (Step 11 of the seq-drift fix) - the correlator is purely a reply-routing
        // hint for the client and must carry no authority of its own; attaching one must
        // not change the security outcome.
        handle_multiuser_ws_message(&mut app, alice_id, WsMessage::RequestScrollback {
            world_index: 1,
            count: 10_000,
            before_seq: None,
            after_seq: Some(0),
            request_id: Some(99),
        }, &event_tx).await;

        let replies = drain_scrollback_replies(&mut alice_rx);
        assert!(replies.is_empty(),
            "alice must never receive ScrollbackLines for bob's world via a direct RequestScrollback naming his world_index, got {replies:?} (attaching a request_id must not bypass the owner check)");
    }

    /// Positive companion to the leak test above (mirrors Step 2's single-user resume
    /// test, but driven through the multiuser path): a user resuming with their OWN
    /// world_index must still get the exact gap replayed, no gap, no duplicate.
    #[tokio::test]
    async fn resume_replays_own_world_scrollback_correctly() {
        let mut app = two_owner_app();

        let server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        let alice_id = 1u64;
        let mut alice_rx = register_client(&server, alice_id, "alice");
        app.ws_server = Some(server);
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(8);

        // Alice already has seq 1..=7 of her OWN world (index 0); resume should replay
        // exactly 8, 9, 10.
        handle_multiuser_ws_message(&mut app, alice_id, WsMessage::AuthRequest {
            username: Some("alice".to_string()),
            password_hash: String::new(),
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: false,
            resume: vec![(0, 7)], resume_epochs: Vec::new(), client_version: String::new(), client_uid: String::new(),
        }, &event_tx).await;

        let replies = drain_scrollback_replies(&mut alice_rx);
        assert_eq!(replies.len(), 1, "expected exactly one ScrollbackLines reply, got {replies:?}");
        let (world_index, seqs) = &replies[0];
        assert_eq!(*world_index, 0);
        assert_eq!(seqs, &vec![8, 9, 10],
            "resume replay of alice's own world must send exactly seq 8,9,10 - no gap, no duplicate");

        // acked_seq must be seeded from the resume payload for her own world.
        let clients = app.ws_server.as_ref().unwrap().clients.read().unwrap();
        let alice_client = clients.get(&alice_id).unwrap();
        assert_eq!(alice_client.acked_seq.get(&0), Some(&7));
    }
}

#[cfg(test)]
mod multiuser_initial_state_tests {
    // Step 3 of the seq-drift/scrollback-reachability plan (on-the-android-app-calm-curry):
    // build_multiuser_initial_state used to send a user's ENTIRE accumulated per-connection
    // output buffer with no cap at all, unlike single-user's build_initial_state (which
    // deliberately limits to remote_initial_lines-driven per-world/aggregate budgets - see
    // CLAUDE.md's "WebSocket InitialState" pattern), and also populated pending_lines_ts
    // (single-user deliberately sends none, since pending lines are delivered later via
    // ServerData on release).
    use super::*;

    #[test]
    fn test_multiuser_initial_state_caps_lines_and_omits_pending() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();
        let mut world = World::new("alice-world");
        world.owner = Some("alice".to_string());
        app.worlds.push(world);

        let mut conn = UserConnection::new();
        conn.connected = true;
        for seq in 0..3000u64 {
            conn.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        app.user_connections.insert((0, "alice".to_string()), conn);

        let initial_state = build_multiuser_initial_state(&app, "alice");
        if let WsMessage::InitialState { worlds, .. } = initial_state {
            assert_eq!(worlds.len(), 1);
            let cap = app.settings.remote_initial_lines.max(1) as usize;
            assert!(worlds[0].output_lines_ts.len() <= cap,
                "output_lines_ts must be capped (remote_initial_lines-driven budget), got {} lines with cap {}",
                worlds[0].output_lines_ts.len(), cap);
            assert!(worlds[0].output_lines_ts.len() < 3000,
                "sanity check: the full 3000-line buffer must not have been sent uncapped");
            assert!(worlds[0].pending_lines_ts.is_empty(),
                "pending_lines_ts must stay empty in InitialState - pending lines are delivered via later ServerData on release, not here");
            assert!(worlds[0].pending_lines.is_empty(), "legacy pending_lines must also stay empty");
            assert!(worlds[0].output_lines.is_empty(), "legacy output_lines must also stay empty - clients prefer output_lines_ts");
        } else {
            panic!("expected InitialState");
        }
    }

    /// A world the requesting user has no connection to (never connected, or a world owned
    /// by someone else) must still produce a well-formed empty entry, not panic or leak
    /// another user's per-connection buffer.
    #[test]
    fn test_multiuser_initial_state_empty_for_user_with_no_connection() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();
        let mut world = World::new("bobs-world");
        world.owner = Some("bob".to_string());
        app.worlds.push(world);
        // No entry in app.user_connections for ("alice", 0) at all.

        let initial_state = build_multiuser_initial_state(&app, "alice");
        if let WsMessage::InitialState { worlds, .. } = initial_state {
            assert_eq!(worlds.len(), 1);
            assert!(worlds[0].output_lines_ts.is_empty());
            assert!(worlds[0].pending_lines_ts.is_empty());
            assert!(!worlds[0].connected);
        } else {
            panic!("expected InitialState");
        }
    }

    /// Multiuser equivalent of single-user's
    /// `test_build_initial_state_budget_counts_only_visible_lines` (main.rs): the aggregate
    /// cross-world budget must be spent only on VISIBLE lines, shared via
    /// `App::build_initial_output_lines` so the two implementations can't silently diverge.
    #[test]
    fn test_multiuser_initial_state_budget_counts_only_visible_lines() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();

        // World 0 (alice's): 100 lines in her connection buffer, 90% gagged.
        let mut alice_world = World::new("alice-world");
        alice_world.owner = Some("alice".to_string());
        app.worlds.push(alice_world);
        let mut alice_conn = UserConnection::new();
        alice_conn.connected = true;
        for seq in 0..100u64 {
            if seq % 10 == 0 {
                alice_conn.output_lines.push(OutputLine::new(format!("visible {seq}"), seq));
            } else {
                alice_conn.output_lines.push(OutputLine::new_gagged(format!("gagged {seq}"), seq));
            }
        }
        app.user_connections.insert((0, "alice".to_string()), alice_conn);

        // World 1 (also alice's, a second world): plenty of plain visible history.
        let mut alice_world2 = World::new("alice-world-2");
        alice_world2.owner = Some("alice".to_string());
        app.worlds.push(alice_world2);
        let mut alice_conn2 = UserConnection::new();
        alice_conn2.connected = true;
        for seq in 0..300u64 {
            alice_conn2.output_lines.push(OutputLine::new(format!("line {seq}"), seq));
        }
        app.user_connections.insert((1, "alice".to_string()), alice_conn2);

        let per_world_cap = app.settings.remote_initial_lines.max(1) as usize;

        let initial_state = build_multiuser_initial_state(&app, "alice");
        let WsMessage::InitialState { worlds, .. } = initial_state else {
            panic!("expected InitialState");
        };

        let world0_visible = worlds[0].output_lines_ts.iter().filter(|l| !l.gagged).count();
        assert_eq!(world0_visible, 10, "world 0 should get all 10 of its visible lines");
        assert!(worlds[0].output_lines_ts.len() >= 10, "world 0's gagged lines must ride along in its own slice");

        assert_eq!(worlds[1].output_lines_ts.len(), per_world_cap,
            "world 1 must get its full per-world cap - world 0's gagged lines must not have \
             eaten into the shared aggregate budget on world 1's behalf. Got {} (cap {per_world_cap})",
            worlds[1].output_lines_ts.len());
    }

    /// mud-status-display.md Job 2 / Job 9 (T3.2): a non-owner's `WorldStateMsg.stats`
    /// must stay empty, same redaction rule as hostname/port/user/etc above - a stat is
    /// per-world game state, no different from a connection detail. Stats now live on
    /// the owner's `UserConnection` (Job 9), not the shared `World` - see
    /// `UserConnection::protocol`'s doc comment.
    #[test]
    fn multiuser_initial_state_redacts_stats_for_non_owner() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();
        let mut alice_world = World::new("alice-world");
        alice_world.owner = Some("alice".to_string());
        app.worlds.push(alice_world);
        let mut alice_conn = UserConnection::new();
        alice_conn.protocol.stats.update_from_gmcp("Char.Vitals", r#"{"hp":"80"}"#);
        app.user_connections.insert((0, "alice".to_string()), alice_conn);

        let owner_view = build_multiuser_initial_state(&app, "alice");
        let WsMessage::InitialState { worlds, .. } = owner_view else { panic!("expected InitialState") };
        assert_eq!(worlds[0].stats.len(), 1, "the owner must see her own world's stats");

        let other_view = build_multiuser_initial_state(&app, "bob");
        let WsMessage::InitialState { worlds, .. } = other_view else { panic!("expected InitialState") };
        assert!(worlds[0].stats.is_empty(), "a non-owner must never see another user's world stats");
    }
}

#[cfg(test)]
mod multiuser_stats_flush_tests {
    // mud-status-display.md Job 2 (plan D5 / CLAUDE.md's "world.owner == username" rule):
    // App::flush_dirty_stats must route a multiuser world's StatsUpdate only to that
    // world's owner, exactly like broadcast_owner_scoped_released_lines already does for
    // a world's output text.
    use super::*;

    /// Same shape as `change_password_tests::register_client` (bounded per-client channel
    /// matching the real `handle_ws_client` setup, `received_initial_state: true` so
    /// broadcast_to_owner's gate lets the message through).
    fn register_client(server: &WebSocketServer, client_id: u64, username: &str) -> mpsc::Receiver<crate::websocket::Outbound> {
        let (tx, rx) = mpsc::channel::<crate::websocket::Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let mut clients = server.clients.try_write().expect("clients lock should be uncontended in test setup");
        clients.insert(client_id, WsClientInfo {
            authenticated: true,
            tx,
            current_world: None,
            username: Some(username.to_string()),
            received_initial_state: true,
            client_type: RemoteClientType::Web,
            viewport_height: 24,
            ip_address: "127.0.0.1".to_string(),
            connected_at: std::time::Instant::now(),
            last_activity: std::time::Instant::now(),
            paused: false,
            acked_seq: std::collections::HashMap::new(),
            audit_prev_acked: std::collections::HashMap::new(),
            audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
            needs_resync: std::collections::HashSet::new(),
        });
        rx
    }

    fn try_recv_stats_update(rx: &mut mpsc::Receiver<crate::websocket::Outbound>) -> Option<(usize, Vec<crate::stats::StatEntry>)> {
        match rx.try_recv() {
            Ok(crate::websocket::Outbound::Shared(json)) => {
                let msg: WsMessage = serde_json::from_str(&json).expect("broadcast JSON must parse");
                match msg {
                    WsMessage::StatsUpdate { world_index, stats } => Some((world_index, stats)),
                    other => panic!("expected StatsUpdate, got {:?}", other),
                }
            }
            _ => None,
        }
    }

    #[test]
    fn stats_flush_reaches_the_owner_and_not_other_users() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();
        let mut alice_world = World::new("alice-world");
        alice_world.owner = Some("alice".to_string());
        app.worlds.push(alice_world);

        let server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        let mut alice_rx = register_client(&server, 1, "alice");
        let mut bob_rx = register_client(&server, 2, "bob");
        app.ws_server = Some(server);

        // Multiuser doesn't wire real per-user GMCP ingestion yet (connect_multiuser_world's
        // doc comment), but the broadcast-scoping logic under test doesn't care how a world
        // became dirty - feed the model directly, same as a future per-user wiring job would.
        app.worlds[0].protocol.stats.update_from_gmcp("Char.Vitals", r#"{"hp":"80"}"#);
        app.worlds[0].protocol.stats_dirty = true;

        app.flush_dirty_stats();

        let (world_index, stats) = try_recv_stats_update(&mut alice_rx)
            .expect("alice (the owner) must receive the StatsUpdate");
        assert_eq!(world_index, 0);
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].key, "hp");

        assert!(bob_rx.try_recv().is_err(), "bob must not receive alice's world stats");
    }

    #[test]
    fn stats_flush_for_unowned_world_reaches_nobody() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();
        app.worlds.push(World::new("unowned-world")); // owner: None

        let server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        let mut someone_rx = register_client(&server, 1, "someone");
        app.ws_server = Some(server);

        app.worlds[0].protocol.stats.update_from_gmcp("Char.Vitals", r#"{"hp":"80"}"#);
        app.worlds[0].protocol.stats_dirty = true;
        app.flush_dirty_stats();

        assert!(someone_rx.try_recv().is_err(), "an unowned world's stats must reach no one");
    }
}

#[cfg(test)]
mod multiuser_telnet_tests {
    // Plan Job 9 (T3.2): "Multiuser drops 15 of 17 telnet events" - NAWS, TTYPE/MTTS,
    // CHARSET, GMCP (incl. the Core.Hello that makes servers send anything), MSDP, MSSP,
    // MSP, and ECHO masking (**passwords in cleartext**) were all dead for every
    // multiuser user. These tests drive App::handle_multiuser_telnet_event directly -
    // the per-user sibling of App::handle_telnet_event - proving each fix at the same
    // level tests.rs's handle_telnet_event_* suite proves the World path.
    use super::*;

    /// Same shape as `multiuser_stats_flush_tests::register_client` (this file already
    /// has several independent copies of this exact harness - `change_password_tests`,
    /// `multiuser_stats_flush_tests` - rather than one shared helper reached across
    /// `#[cfg(test)]` module boundaries).
    fn register_client(server: &WebSocketServer, client_id: u64, username: &str) -> mpsc::Receiver<crate::websocket::Outbound> {
        let (tx, rx) = mpsc::channel::<crate::websocket::Outbound>(crate::websocket::WS_CLIENT_CHANNEL_CAPACITY);
        let mut clients = server.clients.try_write().expect("clients lock should be uncontended in test setup");
        clients.insert(client_id, WsClientInfo {
            authenticated: true,
            tx,
            current_world: None,
            username: Some(username.to_string()),
            received_initial_state: true,
            client_type: RemoteClientType::Web,
            viewport_height: 24,
            ip_address: "127.0.0.1".to_string(),
            connected_at: std::time::Instant::now(),
            last_activity: std::time::Instant::now(),
            paused: false,
            acked_seq: std::collections::HashMap::new(),
            audit_prev_acked: std::collections::HashMap::new(),
            audit_fired_at: std::collections::HashMap::new(),
            audit_stall_ticks: std::collections::HashMap::new(),
            push: None,
            needs_resync: std::collections::HashSet::new(),
        });
        rx
    }

    fn try_recv_msg(rx: &mut mpsc::Receiver<crate::websocket::Outbound>) -> Option<WsMessage> {
        match rx.try_recv() {
            Ok(crate::websocket::Outbound::Shared(json)) => {
                Some(serde_json::from_str(&json).expect("broadcast JSON must parse"))
            }
            Ok(crate::websocket::Outbound::Message(msg)) => Some(*msg),
            _ => None,
        }
    }

    /// alice owns world 0 and has a live connection; bob is just another authenticated
    /// user with no connection of his own to world 0. Returns (app, alice_rx, bob_rx).
    fn setup_alice_and_bob() -> (App, mpsc::Receiver<crate::websocket::Outbound>, mpsc::Receiver<crate::websocket::Outbound>) {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();
        let mut alice_world = World::new("alice-world");
        alice_world.owner = Some("alice".to_string());
        app.worlds.push(alice_world);
        app.user_connections.insert((0, "alice".to_string()), UserConnection::new());

        let server = WebSocketServer::new("", 9000, "", None, true, BanList::new());
        let alice_rx = register_client(&server, 1, "alice");
        let bob_rx = register_client(&server, 2, "bob");
        app.ws_server = Some(server);

        (app, alice_rx, bob_rx)
    }

    #[test]
    fn echo_off_masks_only_alices_connection_and_reaches_only_alice() {
        let (mut app, mut alice_rx, mut bob_rx) = setup_alice_and_bob();

        app.handle_multiuser_telnet_event(0, "alice".to_string(), &TelnetEvent::EchoOff);

        assert!(app.user_connections[&(0, "alice".to_string())].protocol.echo_masked,
            "alice's own connection must be masked");
        // The bug this job exists for: masking must never land on the shared World.
        assert!(!app.worlds[0].protocol.echo_masked,
            "echo masking must live on UserConnection, not the shared World");

        assert!(matches!(
            try_recv_msg(&mut alice_rx),
            Some(WsMessage::EchoMaskChanged { world_index: 0, masked: true })
        ), "alice must see EchoMaskChanged{{masked: true}}");
        assert!(bob_rx.try_recv().is_err(), "bob must not see alice's echo-mask change");

        // Change-only: a repeat with no actual transition sends nothing.
        app.handle_multiuser_telnet_event(0, "alice".to_string(), &TelnetEvent::EchoOff);
        assert!(alice_rx.try_recv().is_err(), "a repeated EchoOff with no state change must not re-broadcast");
    }

    #[test]
    fn option_enabled_gmcp_announces_on_this_connections_command_tx_only() {
        let (mut app, mut alice_rx, mut bob_rx) = setup_alice_and_bob();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(4);
        app.user_connections.get_mut(&(0, "alice".to_string())).unwrap().command_tx = Some(cmd_tx);

        app.handle_multiuser_telnet_event(0, "alice".to_string(), &TelnetEvent::OptionEnabled(TELNET_OPT_GMCP));

        assert!(app.user_connections[&(0, "alice".to_string())].protocol.gmcp_enabled);
        match cmd_rx.try_recv() {
            Ok(WriteCommand::Raw(bytes)) => {
                assert!(String::from_utf8_lossy(&bytes).contains("Core.Hello"));
            }
            other => panic!("expected a Raw Core.Hello wire write, got {:?}", other),
        }
        match cmd_rx.try_recv() {
            Ok(WriteCommand::Raw(bytes)) => {
                assert!(String::from_utf8_lossy(&bytes).contains("Core.Supports.Set"));
            }
            other => panic!("expected a Raw Core.Supports.Set wire write, got {:?}", other),
        }
        assert!(cmd_rx.try_recv().is_err(), "exactly two GMCP messages expected");

        // The Core.Hello/Core.Supports.Set announcement is wire-only - nothing goes out
        // over any WebSocket for a bare OptionEnabled.
        assert!(alice_rx.try_recv().is_err(), "OptionEnabled must not itself broadcast to alice");
        assert!(bob_rx.try_recv().is_err(), "OptionEnabled must not itself broadcast to bob");
    }

    #[test]
    fn char_vitals_dirties_the_connection_and_flush_reaches_alice_only() {
        let (mut app, mut alice_rx, mut bob_rx) = setup_alice_and_bob();

        app.handle_multiuser_telnet_event(0, "alice".to_string(), &TelnetEvent::GmcpMessage(
            "Char.Vitals".to_string(), r#"{"hp":"80"}"#.to_string(),
        ));
        assert!(app.user_connections[&(0, "alice".to_string())].protocol.stats_dirty,
            "a Char.* package must dirty this connection's stats");

        // The GmcpData broadcast from the event itself must reach alice only.
        assert!(matches!(try_recv_msg(&mut alice_rx), Some(WsMessage::GmcpData { world_index: 0, .. })));
        assert!(bob_rx.try_recv().is_err());

        app.flush_dirty_stats();
        assert!(!app.user_connections[&(0, "alice".to_string())].protocol.stats_dirty,
            "flush must clear the dirty flag");
        match try_recv_msg(&mut alice_rx) {
            Some(WsMessage::StatsUpdate { world_index: 0, stats }) => {
                assert_eq!(stats.len(), 1);
                assert_eq!(stats[0].key, "hp");
            }
            other => panic!("expected alice's StatsUpdate, got {:?}", other),
        }
        assert!(bob_rx.try_recv().is_err(), "bob must never see alice's stats");
    }

    #[test]
    fn event_for_a_world_the_username_does_not_own_is_dropped() {
        let (mut app, mut alice_rx, mut bob_rx) = setup_alice_and_bob();
        // Anomalous: a UserConnection keyed under bob's name for alice's world, standing
        // in for a stale/forged event - proves the owner gate runs before any lookup into
        // it, not just that bob usually has no entry at all.
        app.user_connections.insert((0, "bob".to_string()), UserConnection::new());

        app.handle_multiuser_telnet_event(0, "bob".to_string(), &TelnetEvent::EchoOff);

        assert!(!app.user_connections[&(0, "bob".to_string())].protocol.echo_masked,
            "an event for a world this username does not own must change nothing");
        assert!(alice_rx.try_recv().is_err());
        assert!(bob_rx.try_recv().is_err());
    }

    #[test]
    fn disconnect_clears_protocol_state_and_unmasks() {
        let (mut app, mut alice_rx, _bob_rx) = setup_alice_and_bob();
        {
            let conn = app.user_connections.get_mut(&(0, "alice".to_string())).unwrap();
            conn.protocol.echo_masked = true;
            conn.protocol.gmcp_data.insert("Core.Hello".to_string(), "{}".to_string());
        }
        let _ = alice_rx.try_recv(); // drain anything setup left behind (none expected)

        handle_multiuser_disconnect(&mut app, 0, "alice");

        let conn = &app.user_connections[&(0, "alice".to_string())];
        assert!(!conn.protocol.echo_masked, "disconnect must clear echo masking");
        assert!(conn.protocol.gmcp_data.is_empty(), "disconnect must clear the protocol mirror");
        assert!(matches!(try_recv_msg(&mut alice_rx), Some(WsMessage::WorldDisconnected { world_index: 0 })));
        assert!(matches!(
            try_recv_msg(&mut alice_rx),
            Some(WsMessage::EchoMaskChanged { world_index: 0, masked: false })
        ), "a masked connection must be explicitly unmasked on disconnect");
    }

    #[test]
    fn initial_state_reports_per_user_echo_masked_and_stats() {
        let mut app = App::new();
        app.multiuser_mode = true;
        app.worlds.clear();
        let mut alice_world = World::new("alice-world");
        alice_world.owner = Some("alice".to_string());
        app.worlds.push(alice_world);
        let mut conn = UserConnection::new();
        conn.protocol.echo_masked = true;
        conn.protocol.stats.update_from_gmcp("Char.Vitals", r#"{"hp":"80"}"#);
        app.user_connections.insert((0, "alice".to_string()), conn);

        let owner_view = build_multiuser_initial_state(&app, "alice");
        let WsMessage::InitialState { worlds, .. } = owner_view else { panic!("expected InitialState") };
        assert!(worlds[0].echo_masked, "the owner's own masked state must be reported");
        assert_eq!(worlds[0].stats.len(), 1);

        let other_view = build_multiuser_initial_state(&app, "bob");
        let WsMessage::InitialState { worlds, .. } = other_view else { panic!("expected InitialState") };
        assert!(!worlds[0].echo_masked, "a non-owner must never see another user's masked state");
        assert!(worlds[0].stats.is_empty(), "a non-owner must never see another user's stats");
    }

    #[test]
    fn charset_request_switches_this_connections_decoding_not_the_worlds() {
        let (mut app, _alice_rx, _bob_rx) = setup_alice_and_bob();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(4);
        app.user_connections.get_mut(&(0, "alice".to_string())).unwrap().command_tx = Some(cmd_tx);

        app.handle_multiuser_telnet_event(0, "alice".to_string(), &TelnetEvent::CharsetRequest(vec!["UTF-8".to_string()]));

        assert_eq!(
            app.user_connections[&(0, "alice".to_string())].protocol.negotiated_encoding,
            Some(Encoding::Utf8),
        );
        assert_eq!(app.worlds[0].protocol.negotiated_encoding, None,
            "CHARSET negotiation must never touch the shared World's mirror");
        match cmd_rx.try_recv() {
            Ok(WriteCommand::Raw(bytes)) => assert_eq!(bytes, crate::telnet::build_charset_accepted("UTF-8")),
            other => panic!("expected a Raw CHARSET ACCEPTED response, got {:?}", other),
        }
        assert!(matches!(cmd_rx.try_recv(), Ok(WriteCommand::SetEncoding(Encoding::Utf8))));
    }

    /// Wire-level proof (plan Job 9's required test), mirroring
    /// `telnet_reader_migration_tests::connect_daemon_world_carries_gmcp_to_the_app_event_
    /// channel` but for the multiuser connect path: a real fake MUD server negotiates
    /// GMCP and sends a message, and both `OptionEnabled(GMCP)` and `GmcpMessage` arrive
    /// on `run_multiuser_server`'s own `AppEvent` channel, carrying a `Multiuser` target -
    /// the exact stream `run_multiuser_server`'s new `AppEvent::Telnet` arm now routes
    /// through `App::handle_multiuser_telnet_event`.
    #[tokio::test]
    async fn connect_multiuser_world_carries_gmcp_to_the_app_event_channel() {
        use crate::testserver::{self, ServerAction, PortScenario};
        use crate::telnet::{TELNET_IAC, TELNET_WILL, TELNET_SB, TELNET_SE};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut raw = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP];
        raw.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP]);
        raw.extend_from_slice(b"Core.Hello {\"foo\":\"bar\"}");
        raw.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        let scenario = PortScenario {
            actions: vec![
                ServerAction::SendRaw(raw),
                ServerAction::Sleep(Duration::from_millis(300)),
                ServerAction::Disconnect,
            ],
            telnet_negotiate: false,
        };
        let server = tokio::spawn(testserver::run_server_port(port, scenario));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let settings = WorldSettings {
            hostname: "127.0.0.1".to_string(),
            port: port.to_string(),
            ..Default::default()
        };
        let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(32);
        let conn = connect_multiuser_world(0, "alice".to_string(), &settings, event_tx).await;
        assert!(conn.is_some(), "connect_multiuser_world should have connected to the fake server");

        let mut saw_negotiated = false;
        let mut saw_gmcp: Option<(String, String)> = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline && (!saw_negotiated || saw_gmcp.is_none()) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, event_rx.recv()).await {
                Ok(Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 0, ref username }, TelnetEvent::OptionEnabled(opt))))
                    if username == "alice" && opt == TELNET_OPT_GMCP =>
                {
                    saw_negotiated = true;
                }
                Ok(Some(AppEvent::Telnet(TelnetTarget::Multiuser { world_index: 0, ref username }, TelnetEvent::GmcpMessage(pkg, json))))
                    if username == "alice" =>
                {
                    saw_gmcp = Some((pkg, json));
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }

        assert!(saw_negotiated, "expected Telnet(Multiuser{{0,\"alice\"}}, OptionEnabled(GMCP)) on the multiuser loop's channel");
        let (pkg, json) = saw_gmcp.expect("expected Telnet(Multiuser{0,\"alice\"}, GmcpMessage(..)) on the multiuser loop's channel");
        assert_eq!(pkg, "Core.Hello");
        assert_eq!(json, "{\"foo\":\"bar\"}");

        let _ = server.await;
    }
}

#[cfg(test)]
mod telnet_reader_migration_tests {
    // Plan Job 5 (2.3 of investigate-differences-between-tinyfugu-fluffy-stallman.md):
    // daemon.rs's four telnet reader loops (connect_daemon_world's direct-connect, its
    // two TLS-proxy loops, and connect_multiuser_world) were migrated from a hand-rolled
    // process_telnet/find_safe_split_point loop onto the shared spawn_telnet_reader
    // (telnet_reader.rs). This test drives a real fake MUD server (testserver.rs)
    // through the actual `connect_daemon_world` production function - not the separate
    // testharness.rs connection path - and proves a GMCP message survives the migrated
    // direct-connect loop all the way to the AppEvent channel App's own dispatch loop
    // (run_daemon_server) reads from. The proxy loops' MCCP2 gain and the close-message
    // unification are covered at the spawn_telnet_reader level in telnet_reader.rs's own
    // tests (`proxy_shaped_reader_decompresses_mccp2_and_still_closes_cleanly`), driven
    // over `TelnetTarget::World` - the exact shape both proxy loops now pass - since
    // wiring a real Unix-socket TLS proxy (spawned process, certs) into a test would be
    // disproportionate to what's being proven.
    use super::*;
    use crate::testserver::{self, ServerAction, PortScenario};
    use crate::telnet::{TELNET_IAC, TELNET_WILL, TELNET_SB, TELNET_SE, TELNET_OPT_GMCP};

    fn find_free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    /// IAC WILL GMCP, then IAC SB GMCP Core.Hello {"foo":"bar"} IAC SE, as a single raw
    /// write (mirrors how a real MUD server frames a negotiation + immediate message).
    fn gmcp_scenario() -> PortScenario {
        let mut raw = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP];
        raw.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP]);
        raw.extend_from_slice(b"Core.Hello {\"foo\":\"bar\"}");
        raw.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        PortScenario {
            actions: vec![
                ServerAction::SendRaw(raw),
                ServerAction::Sleep(Duration::from_millis(300)),
                ServerAction::Disconnect,
            ],
            telnet_negotiate: false,
        }
    }

    /// End-to-end proof (plan Job 5's required test) that the migrated
    /// `connect_daemon_world` direct-connect reader loop carries a real protocol event —
    /// GMCP, matching finding 8's "one package family" scope, not a synthetic one — from
    /// the wire through `TelnetSession`/`spawn_telnet_reader` to the `AppEvent` channel
    /// `run_daemon_server`'s dispatch loop consumes. This is the same channel and the
    /// same production connect function daemon.rs uses for a real `-D`/single-user world;
    /// only the fake MUD server on the other end is test infrastructure.
    #[tokio::test]
    async fn connect_daemon_world_carries_gmcp_to_the_app_event_channel() {
        let port = find_free_port();
        let server = tokio::spawn(testserver::run_server_port(port, gmcp_scenario()));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let settings = WorldSettings {
            hostname: "127.0.0.1".to_string(),
            port: port.to_string(),
            ..Default::default()
        };

        let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(32);
        let conn = connect_daemon_world(
            0,
            "gmcptest".to_string(),
            &settings,
            event_tx,
            1,     // connection_id
            true,  // skip_auto_login - no login needed for this scenario
            false, // tls_proxy_enabled - plain TCP, exercises the direct-connect loop
        ).await;
        assert!(conn.is_some(), "connect_daemon_world should have connected to the fake server");

        // Drain events until both the negotiation mirror and the actual GMCP payload
        // have arrived, or time out.
        let mut saw_negotiated = false;
        let mut saw_gmcp: Option<(String, String)> = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline && (!saw_negotiated || saw_gmcp.is_none()) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, event_rx.recv()).await {
                Ok(Some(AppEvent::Telnet(TelnetTarget::World(name), TelnetEvent::OptionEnabled(opt))))
                    if name == "gmcptest" && opt == TELNET_OPT_GMCP =>
                {
                    saw_negotiated = true;
                }
                Ok(Some(AppEvent::Telnet(TelnetTarget::World(name), TelnetEvent::GmcpMessage(pkg, json))))
                    if name == "gmcptest" =>
                {
                    saw_gmcp = Some((pkg, json));
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }

        assert!(saw_negotiated, "expected AppEvent::Telnet(.., OptionEnabled(TELNET_OPT_GMCP)) on the channel App reads from");
        let (pkg, json) = saw_gmcp.expect("expected AppEvent::Telnet(.., GmcpMessage(..)) on the channel App reads from");
        assert_eq!(pkg, "Core.Hello");
        assert_eq!(json, "{\"foo\":\"bar\"}");

        let _ = server.await;
    }
}
