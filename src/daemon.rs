use std::io::{self, Write as IoWrite};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use bytes::BytesMut;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};

use crate::*;
use crate::{
    App, WorldSettings, UserConnection,
    ClientViewState, Command, OutputLine,
    get_multiuser_settings_path,
    enable_tcp_keepalive, parse_command, current_timestamp_secs,
    get_home_dir, clay_filename,
};
use crate::actions::{split_action_commands, substitute_action_args, execute_recall};
use crate::util::local_time_from_epoch;
use crate::websocket::{TimestampedLine, RemoteClientType};

/// Run in daemon mode (-D) - background server for remote connections only
/// No console UI, just prints listening ports and handles remote clients
pub async fn run_daemon_server() -> io::Result<()> {
    let mut app = App::new();

    // Load settings from normal settings file
    if let Err(e) = persistence::load_settings(&mut app) {
        eprintln!("Warning: Could not load settings: {}", e);
    }

    // Ensure at least one world exists
    app.ensure_has_world();

    // Re-create spell checker with custom dictionary path if configured
    if !app.settings.dictionary_path.is_empty() {
        app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
    }

    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(100);

    // Start WebSocket server if enabled
    if app.settings.ws_enabled && !app.settings.websocket_password.is_empty() {
        let mut server = WebSocketServer::new(
            &app.settings.websocket_password,
            app.settings.ws_port,
            &app.settings.websocket_allow_list,
            app.settings.websocket_whitelisted_host.clone(),
            false, // Not multiuser mode
            app.ban_list.clone(),
        );

        // Configure TLS if secure mode enabled
        #[cfg(feature = "native-tls-backend")]
        let tls_configured = if app.settings.web_secure
            && !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty()
        {
            match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Warning: Failed to configure TLS: {}", e);
                    false
                }
            }
        } else {
            false
        };
        #[cfg(feature = "rustls-backend")]
        let tls_configured = if app.settings.web_secure
            && !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty()
        {
            match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Warning: Failed to configure TLS: {}", e);
                    false
                }
            }
        } else {
            false
        };

        if let Err(e) = start_websocket_server(&mut server, event_tx.clone()).await {
            eprintln!("Failed to start WebSocket server: {}", e);
            return Ok(());
        }
        let protocol = if tls_configured { "wss" } else { "ws" };
        println!("WebSocket: {}://0.0.0.0:{}", protocol, app.settings.ws_port);
        app.ws_server = Some(server);
    }

    // Start HTTP/HTTPS server if enabled
    if app.settings.http_enabled {
        let has_cert = !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty();
        let web_secure = app.settings.web_secure;

        if web_secure && has_cert {
            // Start HTTPS
            #[cfg(any(feature = "native-tls-backend", feature = "rustls-backend"))]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
                    app.settings.ws_port,
                    true,
                    app.gui_theme_colors().to_css_vars(),
                ).await {
                    Ok(()) => {
                        println!("HTTPS: https://0.0.0.0:{}", app.settings.http_port);
                        app.https_server = Some(https_server);
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to start HTTPS server: {}", e);
                    }
                }
            }
        } else {
            // Start HTTP
            let mut http_server = HttpServer::new(app.settings.http_port);
            match start_http_server(&mut http_server, app.settings.ws_port, false, app.ban_list.clone(), app.gui_theme_colors().to_css_vars()).await {
                Ok(()) => {
                    println!("HTTP: http://0.0.0.0:{}", app.settings.http_port);
                    app.http_server = Some(http_server);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to start HTTP server: {}", e);
                }
            }
        }
    }

    // Check if any servers are running
    if app.ws_server.is_none() && app.http_server.is_none() && app.https_server.is_none() {
        eprintln!("Error: No servers started. Enable WebSocket or HTTP in settings.");
        eprintln!("Use /web command to configure, or edit ~/.clay.dat");
        return Ok(());
    }

    // Show allow list if configured (helps debug connection rejections)
    if !app.settings.websocket_allow_list.is_empty() {
        println!("Allow list: {}", app.settings.websocket_allow_list);
    }

    println!("Daemon running. Press Ctrl+C to stop.");

    // Create interval for TF repeat process ticks (1 second)
    let mut process_tick_interval = tokio::time::interval(std::time::Duration::from_secs(1));
    process_tick_interval.tick().await; // Skip first immediate tick

    // Main event loop - handles MUD connections and WebSocket messages
    loop {
        #[cfg(all(unix, not(target_os = "android")))]
        reap_zombie_children();

        tokio::select! {
            // TF repeat process tick
            _ = process_tick_interval.tick() => {
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
                                    if let Some(tx) = &app.worlds[idx].command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(text));
                                    }
                                }
                            }
                            tf::TfCommandResult::Success(Some(msg)) => {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: msg,
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                            tf::TfCommandResult::Error(err) => {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: format!("Error: {}", err),
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                            tf::TfCommandResult::RepeatProcess(process) => {
                                app.tf_engine.processes.push(process);
                            }
                            tf::TfCommandResult::NotTfCommand => {
                                // Plain text command - send to MUD
                                if let Some(idx) = target_idx {
                                    if let Some(tx) = &app.worlds[idx].command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                                    }
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
                                80, // Default console width
                                true, // is_daemon_mode
                            );

                            // Execute any triggered commands
                            let saved_current_world = app.current_world_index;
                            app.current_world_index = world_idx;
                            for cmd in commands {
                                if cmd.starts_with('/') || cmd.starts_with('#') {
                                    // Unified command system - route through TF parser
                                    app.sync_tf_world_info();
                                    match app.tf_engine.execute(&cmd) {
                                        tf::TfCommandResult::SendToMud(text) => {
                                            if let Some(tx) = &app.worlds[world_idx].command_tx {
                                                let _ = tx.try_send(WriteCommand::Text(text));
                                            }
                                        }
                                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                            // Handle Clay-specific commands in daemon mode
                                            let parsed = parse_command(&clay_cmd);
                                            if let Command::Send { text, target_world, .. } = parsed {
                                                let target_idx = if let Some(ref w) = target_world {
                                                    app.find_world_index(w)
                                                } else {
                                                    Some(world_idx)
                                                };
                                                if let Some(idx) = target_idx {
                                                    if let Some(tx) = &app.worlds[idx].command_tx {
                                                        let _ = tx.try_send(WriteCommand::Text(text));
                                                    }
                                                }
                                            }
                                        }
                                        tf::TfCommandResult::RepeatProcess(process) => {
                                            app.tf_engine.processes.push(process);
                                        }
                                        _ => {}
                                    }
                                } else if let Some(tx) = &app.worlds[world_idx].command_tx {
                                    // Plain text - send to MUD
                                    let _ = tx.try_send(WriteCommand::Text(cmd));
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
                            app.worlds[world_idx].connected = false;
                            app.worlds[world_idx].command_tx = None;

                            // Broadcast disconnect to clients
                            app.ws_broadcast(WsMessage::WorldDisconnected { world_index: world_idx });
                        }
                    }
                    AppEvent::WsClientMessage(client_id, msg) => {
                        // Check if this is an AuthRequest (client just authenticated)
                        if matches!(*msg, WsMessage::AuthRequest { .. }) {
                            // Send initial state after successful authentication
                            let initial_state = app.build_initial_state();
                            app.ws_send_to_client(client_id, initial_state);
                            // Mark client as having received initial state so it receives broadcasts
                            app.ws_mark_initial_state_sent(client_id);
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
                    AppEvent::ApiLookupResult(client_id, world_index, result) => {
                        match result {
                            Ok(text) => app.ws_send_to_client(client_id, WsMessage::SetInputBuffer { text }),
                            Err(e) => app.ws_send_to_client(client_id, WsMessage::ServerData {
                                world_index,
                                data: e,
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                            }),
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Handle WebSocket message in daemon mode
pub async fn handle_daemon_ws_message(
    app: &mut App,
    client_id: u64,
    msg: WsMessage,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    match msg {
        WsMessage::SendCommand { world_index, command } => {
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
                    // Execute action if it exists
                    if let Some(action) = app.settings.actions.iter().find(|a| a.name.eq_ignore_ascii_case(&name)) {
                        if !action.enabled {
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: format!("\u{2728} Action '{}' is disabled.", name),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                            });
                        } else {
                            let commands = split_action_commands(&action.command);
                            let mut sent_to_server = false;
                            for cmd in commands {
                                let cmd = substitute_action_args(&cmd, &args);

                                if cmd.eq_ignore_ascii_case("/gag") || cmd.to_lowercase().starts_with("/gag ") {
                                    continue;
                                }
                                // Unified command system - route through TF parser
                                if cmd.starts_with('/') || cmd.starts_with('#') {
                                    match app.tf_engine.execute(&cmd) {
                                        tf::TfCommandResult::Success(Some(msg)) => {
                                            app.ws_broadcast(WsMessage::ServerData {
                                                world_index,
                                                data: msg,
                                                is_viewed: false,
                                                ts: current_timestamp_secs(),
                                                from_server: false,
                                            });
                                        }
                                        tf::TfCommandResult::Success(None) => {}
                                        tf::TfCommandResult::Error(err) => {
                                            app.ws_broadcast(WsMessage::ServerData {
                                                world_index,
                                                data: format!("Error: {}", err),
                                                is_viewed: false,
                                                ts: current_timestamp_secs(),
                                                from_server: false,
                                            });
                                        }
                                        tf::TfCommandResult::SendToMud(text) => {
                                            if world_index < app.worlds.len() {
                                                if let Some(tx) = &app.worlds[world_index].command_tx {
                                                    let _ = tx.try_send(WriteCommand::Text(text));
                                                    sent_to_server = true;
                                                }
                                            }
                                        }
                                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                            app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: clay_cmd });
                                        }
                                        tf::TfCommandResult::Recall(opts) => {
                                            if world_index < app.worlds.len() {
                                                let output_lines = app.worlds[world_index].output_lines.clone();
                                                let (matches, header) = execute_recall(&opts, &output_lines);
                                                let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                                                let ts = current_timestamp_secs();

                                                if !opts.quiet {
                                                    if let Some(h) = header {
                                                        app.ws_broadcast(WsMessage::ServerData { world_index, data: h, is_viewed: false, ts, from_server: false });
                                                    }
                                                }
                                                if matches.is_empty() {
                                                    app.ws_broadcast(WsMessage::ServerData { world_index, data: format!("\u{2728} No matches for '{}'", pattern_str), is_viewed: false, ts, from_server: false });
                                                } else {
                                                    for m in matches {
                                                        app.ws_broadcast(WsMessage::ServerData { world_index, data: m, is_viewed: false, ts, from_server: false });
                                                    }
                                                }
                                                if !opts.quiet {
                                                    app.ws_broadcast(WsMessage::ServerData { world_index, data: "================= Recall end =================".to_string(), is_viewed: false, ts, from_server: false });
                                                }
                                            }
                                        }
                                        tf::TfCommandResult::RepeatProcess(process) => {
                                            app.tf_engine.processes.push(process);
                                        }
                                        _ => {}
                                    }
                                } else if world_index < app.worlds.len() {
                                    // Plain text - send to MUD server
                                    if let Some(tx) = &app.worlds[world_index].command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(cmd));
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
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index, data: msg, is_viewed: false,
                                    ts: current_timestamp_secs(), from_server: false,
                                });
                            }
                            tf::TfCommandResult::Success(None) => {}
                            tf::TfCommandResult::Error(err) => {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index, data: format!("Error: {}", err), is_viewed: false,
                                    ts: current_timestamp_secs(), from_server: false,
                                });
                            }
                            tf::TfCommandResult::SendToMud(text) => {
                                if world_index < app.worlds.len() {
                                    if let Some(tx) = &app.worlds[world_index].command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(text));
                                        app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                    }
                                }
                            }
                            tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: clay_cmd });
                            }
                            tf::TfCommandResult::Recall(opts) => {
                                if world_index < app.worlds.len() {
                                    let output_lines = app.worlds[world_index].output_lines.clone();
                                    let (matches, header) = execute_recall(&opts, &output_lines);
                                    let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                                    let ts = current_timestamp_secs();
                                    if !opts.quiet {
                                        if let Some(h) = header {
                                            app.ws_broadcast(WsMessage::ServerData { world_index, data: h, is_viewed: false, ts, from_server: false });
                                        }
                                    }
                                    if matches.is_empty() {
                                        app.ws_broadcast(WsMessage::ServerData { world_index, data: format!("\u{2728} No matches for '{}'", pattern_str), is_viewed: false, ts, from_server: false });
                                    } else {
                                        for m in matches {
                                            app.ws_broadcast(WsMessage::ServerData { world_index, data: m, is_viewed: false, ts, from_server: false });
                                        }
                                    }
                                    if !opts.quiet {
                                        app.ws_broadcast(WsMessage::ServerData { world_index, data: "================= Recall end =================".to_string(), is_viewed: false, ts, from_server: false });
                                    }
                                }
                            }
                            tf::TfCommandResult::RepeatProcess(process) => {
                                app.tf_engine.processes.push(process);
                            }
                            _ => {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index,
                                    data: format!("Unknown command: /{}", name),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                        }
                    }
                }
                Command::NotACommand { text } => {
                    // Check if this is a TF command (starts with #)
                    if text.starts_with('#') {
                        // TinyFugue command - execute on server
                        app.sync_tf_world_info();
                        match app.tf_engine.execute(&text) {
                            tf::TfCommandResult::Success(Some(msg)) => {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index,
                                    data: msg,
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                            tf::TfCommandResult::Success(None) => {}
                            tf::TfCommandResult::Error(err) => {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index,
                                    data: format!("Error: {}", err),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                            tf::TfCommandResult::SendToMud(mud_text) => {
                                if world_index < app.worlds.len() {
                                    if let Some(tx) = &app.worlds[world_index].command_tx {
                                        if tx.try_send(WriteCommand::Text(mud_text)).is_ok() {
                                            app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                        }
                                    }
                                }
                            }
                            tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: clay_cmd });
                            }
                            tf::TfCommandResult::Recall(opts) => {
                                if world_index < app.worlds.len() {
                                    let output_lines = app.worlds[world_index].output_lines.clone();
                                    let (matches, header) = execute_recall(&opts, &output_lines);
                                    let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                                    let ts = current_timestamp_secs();

                                    if !opts.quiet {
                                        if let Some(h) = header {
                                            app.ws_broadcast(WsMessage::ServerData { world_index, data: h, is_viewed: false, ts, from_server: false });
                                        }
                                    }
                                    if matches.is_empty() {
                                        app.ws_broadcast(WsMessage::ServerData { world_index, data: format!("✨ No matches for '{}'", pattern_str), is_viewed: false, ts, from_server: false });
                                    } else {
                                        for m in matches {
                                            app.ws_broadcast(WsMessage::ServerData { world_index, data: m, is_viewed: false, ts, from_server: false });
                                        }
                                    }
                                    if !opts.quiet {
                                        app.ws_broadcast(WsMessage::ServerData { world_index, data: "================= Recall end =================".to_string(), is_viewed: false, ts, from_server: false });
                                    }
                                }
                            }
                            tf::TfCommandResult::RepeatProcess(process) => {
                                let id = process.id;
                                let interval = format_duration_short(process.interval);
                                let count_str = process.count.map_or("infinite".to_string(), |c| c.to_string());
                                let cmd_str = process.command.clone();
                                app.tf_engine.processes.push(process);
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index,
                                    data: format!("% Process {} started: {} every {} ({} times)", id, cmd_str, interval, count_str),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                            _ => {}
                        }
                    } else {
                        // Regular text - send to MUD
                        if world_index < app.worlds.len() {
                            if let Some(tx) = &app.worlds[world_index].command_tx {
                                if tx.try_send(WriteCommand::Text(text)).is_ok() {
                                    app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                    app.worlds[world_index].prompt.clear();
                                }
                            }
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
                Command::Dict { .. } | Command::Urban { .. } | Command::Translate { .. } => {
                    spawn_api_lookup(event_tx.clone(), client_id, world_index, parsed);
                }
                Command::DictUsage => {
                    app.ws_send_to_client(client_id, WsMessage::ServerData {
                        world_index,
                        data: "Usage: /dict <prefix> <word>".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                Command::UrbanUsage => {
                    app.ws_send_to_client(client_id, WsMessage::ServerData {
                        world_index,
                        data: "Usage: /urban <prefix> <word>".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                Command::TranslateUsage => {
                    app.ws_send_to_client(client_id, WsMessage::ServerData {
                        world_index,
                        data: "Usage: /translate <lang> <prefix> <text>".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                Command::HelpTf => {
                    // Execute TF help command and send the result
                    match app.tf_engine.execute("#help") {
                        crate::tf::TfCommandResult::Success(Some(msg)) => {
                            for line in msg.lines() {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index,
                                    data: line.to_string(),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                        }
                        _ => {
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: "TF help not available.".to_string(),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                            });
                        }
                    }
                }
                Command::Unknown { cmd } => {
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: format!("Unknown command: {}", cmd),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                Command::Send { text, all_worlds, target_world, no_newline } => {
                    let make_write_cmd = |t: &str| -> WriteCommand {
                        if no_newline {
                            WriteCommand::Raw(t.as_bytes().to_vec())
                        } else {
                            WriteCommand::Text(t.to_string())
                        }
                    };

                    if all_worlds {
                        for world in app.worlds.iter_mut() {
                            if world.connected {
                                if let Some(tx) = &world.command_tx {
                                    let _ = tx.try_send(make_write_cmd(&text));
                                    world.last_send_time = Some(std::time::Instant::now());
                                }
                            }
                        }
                    } else if let Some(ref target) = target_world {
                        if let Some(world) = app.worlds.iter_mut().find(|w| w.name.eq_ignore_ascii_case(target)) {
                            if world.connected {
                                if let Some(tx) = &world.command_tx {
                                    let _ = tx.try_send(make_write_cmd(&text));
                                    world.last_send_time = Some(std::time::Instant::now());
                                }
                            } else {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index,
                                    data: format!("World '{}' is not connected.", target),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                        } else {
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: format!("Unknown world: {}", target),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                            });
                        }
                    } else if world_index < app.worlds.len() {
                        if let Some(tx) = &app.worlds[world_index].command_tx {
                            let _ = tx.try_send(make_write_cmd(&text));
                            app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                        }
                    }
                }
                Command::Disconnect => {
                    if world_index < app.worlds.len() && app.worlds[world_index].connected {
                        #[cfg(unix)]
                        if let Some(proxy_pid) = app.worlds[world_index].proxy_pid {
                            unsafe { libc::kill(proxy_pid as libc::pid_t, libc::SIGTERM); }
                        }
                        if let Some(ref socket_path) = app.worlds[world_index].proxy_socket_path {
                            let _ = std::fs::remove_file(socket_path);
                        }
                        app.worlds[world_index].proxy_pid = None;
                        app.worlds[world_index].proxy_socket_path = None;
                        app.worlds[world_index].command_tx = None;
                        app.worlds[world_index].connected = false;
                        app.worlds[world_index].socket_fd = None;
                        app.worlds[world_index].close_log_file();
                        app.worlds[world_index].prompt.clear();
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: "Disconnected.".to_string(),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                        app.ws_broadcast(WsMessage::WorldDisconnected { world_index });
                    } else {
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: "Not connected.".to_string(),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                    }
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
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: format!("Flushed {} lines from output buffer.", line_count),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                    }
                }
                Command::Keepalive => {
                    if world_index < app.worlds.len() {
                        let world = &app.worlds[world_index];
                        let info = format!(
                            "Keepalive: {} ({})",
                            world.settings.keep_alive_type.name(),
                            if world.settings.keep_alive_type == KeepAliveType::Custom {
                                world.settings.keep_alive_cmd.clone()
                            } else {
                                world.settings.keep_alive_type.name().to_string()
                            }
                        );
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: info,
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                    }
                }
                Command::Gag { pattern } => {
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: format!("Gag pattern set: {}", pattern),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                Command::BanList => {
                    let bans = app.ban_list.get_ban_info();
                    if bans.is_empty() {
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: "No hosts are currently banned.".to_string(),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
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
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: output,
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                    }
                    app.ws_send_to_client(client_id, WsMessage::BanListResponse { bans });
                }
                Command::Unban { host } => {
                    if app.ban_list.remove_ban(&host) {
                        let _ = persistence::save_settings(app);
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: format!("Removed ban for: {}", host),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                        app.ws_broadcast(WsMessage::BanListResponse { bans: app.ban_list.get_ban_info() });
                        app.ws_send_to_client(client_id, WsMessage::UnbanResult { success: true, host, error: None });
                    } else {
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: format!("No ban found for: {}", host),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                        app.ws_send_to_client(client_id, WsMessage::UnbanResult { success: false, host, error: Some("No ban found".to_string()) });
                    }
                }
                Command::TestMusic => {
                    let test_notes = vec![
                        ansi_music::MusicNote { frequency: 261.63, duration_ms: 250 },
                        ansi_music::MusicNote { frequency: 293.66, duration_ms: 250 },
                        ansi_music::MusicNote { frequency: 329.63, duration_ms: 250 },
                        ansi_music::MusicNote { frequency: 349.23, duration_ms: 250 },
                        ansi_music::MusicNote { frequency: 392.00, duration_ms: 250 },
                    ];
                    app.ws_broadcast(WsMessage::AnsiMusic {
                        world_index,
                        notes: test_notes,
                    });
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: "Playing test music (C-D-E-F-G)...".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
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
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: format!("Notification sent: {}", message),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                Command::Dump => {
                    use std::io::Write;
                    let ts = current_timestamp_secs();
                    let home = get_home_dir();
                    let dump_path = format!("{}/{}", home, clay_filename("clay.dmp.log"));

                    match std::fs::File::create(&dump_path) {
                        Ok(mut file) => {
                            let mut total_lines = 0;
                            for world in app.worlds.iter() {
                                for line in &world.output_lines {
                                    let line_ts = line.timestamp
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs())
                                        .unwrap_or(0) as i64;
                                    let lt = local_time_from_epoch(line_ts);
                                    let datetime = format!(
                                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                                        lt.year, lt.month, lt.day,
                                        lt.hour, lt.minute, lt.second
                                    );
                                    let _ = writeln!(file, "{},{},{}", world.name, datetime, line.text);
                                    total_lines += 1;
                                }
                                for line in &world.pending_lines {
                                    let line_ts = line.timestamp
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs())
                                        .unwrap_or(0) as i64;
                                    let lt = local_time_from_epoch(line_ts);
                                    let datetime = format!(
                                        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                                        lt.year, lt.month, lt.day,
                                        lt.hour, lt.minute, lt.second
                                    );
                                    let _ = writeln!(file, "{},{},{}", world.name, datetime, line.text);
                                    total_lines += 1;
                                }
                            }
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: format!("Dumped {} lines from {} worlds to {}", total_lines, app.worlds.len(), dump_path),
                                is_viewed: false,
                                ts,
                                from_server: false,
                            });
                        }
                        Err(e) => {
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: format!("Failed to create dump file: {}", e),
                                is_viewed: false,
                                ts,
                                from_server: false,
                            });
                        }
                    }
                }
                // Commands that should be blocked from remote
                Command::Quit | Command::Reload | Command::Update => {
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: "This command is not available from remote interfaces.".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                // UI popup commands - send back to client for local handling
                Command::Help | Command::Menu | Command::Setup | Command::Web | Command::Actions { .. } |
                Command::WorldsList | Command::WorldSelector | Command::WorldEdit { .. } => {
                    app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                }
                Command::Version => {
                    app.ws_send_to_client(client_id, WsMessage::ServerData {
                        world_index,
                        data: get_version_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                // AddWorld - add or update world definition
                Command::AddWorld { name, host, port, user, password, use_ssl } => {
                    let existing_idx = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(&name));

                    let world_idx = if let Some(idx) = existing_idx {
                        idx
                    } else {
                        let new_world = World::new(&name);
                        app.worlds.push(new_world);
                        app.worlds.len() - 1
                    };

                    if let Some(h) = host {
                        app.worlds[world_idx].settings.hostname = h;
                    }
                    if let Some(p) = port {
                        app.worlds[world_idx].settings.port = p;
                    }
                    if let Some(u) = user {
                        app.worlds[world_idx].settings.user = u;
                    }
                    if let Some(p) = password {
                        app.worlds[world_idx].settings.password = p;
                    }
                    app.worlds[world_idx].settings.use_ssl = use_ssl;

                    let _ = persistence::save_settings(app);

                    let action = if existing_idx.is_some() { "Updated" } else { "Added" };
                    let host_info = if !app.worlds[world_idx].settings.hostname.is_empty() {
                        format!(" ({}:{}{})",
                            app.worlds[world_idx].settings.hostname,
                            app.worlds[world_idx].settings.port,
                            if use_ssl { " SSL" } else { "" })
                    } else {
                        " (connectionless)".to_string()
                    };
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: format!("{} world '{}'{}.", action, name, host_info),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                }
                // Connect command - use daemon connection logic
                Command::Connect { .. } => {
                    if world_index < app.worlds.len() && !app.worlds[world_index].connected {
                        if app.worlds[world_index].settings.has_connection_settings() {
                            let settings = app.worlds[world_index].settings.clone();
                            let world_name = app.worlds[world_index].name.clone();

                            let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: format!("Connecting to {}:{}{}...\n", settings.hostname, settings.port, ssl_msg),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                            });

                            app.worlds[world_index].connection_id += 1;
                            if let Some(cmd_tx) = connect_daemon_world(
                                world_index,
                                world_name.clone(),
                                &settings,
                                event_tx.clone(),
                                app.worlds[world_index].connection_id,
                            ).await {
                                app.worlds[world_index].connected = true;
                                app.worlds[world_index].command_tx = Some(cmd_tx);
                                app.worlds[world_index].was_connected = true;
                                let now = std::time::Instant::now();
                                app.worlds[world_index].last_send_time = Some(now);
                                app.worlds[world_index].last_receive_time = Some(now);
                                app.ws_broadcast(WsMessage::WorldConnected { world_index, name: world_name });
                            } else {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index,
                                    data: "Connection failed.\n".to_string(),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                        } else {
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: "No connection settings configured for this world.".to_string(),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                            });
                        }
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
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index: idx,
                                data: format!("Connecting to {}:{}{}...\n", settings.hostname, settings.port, ssl_msg),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                            });

                            app.worlds[idx].connection_id += 1;
                            if let Some(cmd_tx) = connect_daemon_world(
                                idx,
                                world_name.clone(),
                                &settings,
                                event_tx.clone(),
                                app.worlds[idx].connection_id,
                            ).await {
                                app.worlds[idx].connected = true;
                                app.worlds[idx].command_tx = Some(cmd_tx);
                                app.worlds[idx].was_connected = true;
                                let now = std::time::Instant::now();
                                app.worlds[idx].last_send_time = Some(now);
                                app.worlds[idx].last_receive_time = Some(now);
                                app.ws_broadcast(WsMessage::WorldConnected { world_index: idx, name: world_name });
                            } else {
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: idx,
                                    data: "Connection failed.\n".to_string(),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                });
                            }
                        }
                    } else {
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: format!("World '{}' not found.", name),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
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
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: "Configure host/port in world settings.\n".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                    });
                    return;
                }

                let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                app.ws_broadcast(WsMessage::ServerData {
                    world_index,
                    data: format!("Connecting to {}:{}{}...\n", settings.hostname, settings.port, ssl_msg),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                });

                // Attempt connection
                app.worlds[world_index].connection_id += 1;
                if let Some(cmd_tx) = connect_daemon_world(
                    world_index,
                    world_name.clone(),
                    &settings,
                    event_tx.clone(),
                    app.worlds[world_index].connection_id,
                ).await {
                    // Connection succeeded
                    app.worlds[world_index].connected = true;
                    app.worlds[world_index].command_tx = Some(cmd_tx);
                    app.worlds[world_index].was_connected = true;
                    let now = std::time::Instant::now();
                    app.worlds[world_index].last_send_time = Some(now);
                    app.worlds[world_index].last_receive_time = Some(now);

                    app.ws_broadcast(WsMessage::WorldConnected { world_index, name: world_name });
                } else {
                    // Connection failed
                    app.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: "Connection failed.\n".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
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
        WsMessage::SwitchWorld { world_index } => {
            if world_index < app.worlds.len() {
                app.current_world_index = world_index;
                app.ws_broadcast(WsMessage::WorldSwitched { new_index: world_index });
            }
        }
        WsMessage::UpdateGlobalSettings { more_mode_enabled, spell_check_enabled, temp_convert_enabled, world_switch_mode, show_tags, debug_enabled, ansi_music_enabled, console_theme, gui_theme, gui_transparency, color_offset_percent, input_height, font_name, font_size, web_font_size_phone, web_font_size_tablet, web_font_size_desktop, ws_allow_list, web_secure, http_enabled, http_port, ws_enabled, ws_port, ws_cert_file, ws_key_file, tls_proxy_enabled, dictionary_path } => {
            app.settings.more_mode_enabled = more_mode_enabled;
            app.settings.spell_check_enabled = spell_check_enabled;
            app.settings.temp_convert_enabled = temp_convert_enabled;
            app.settings.world_switch_mode = WorldSwitchMode::from_name(&world_switch_mode);
            app.show_tags = show_tags;
            app.settings.debug_enabled = debug_enabled;
            app.settings.ansi_music_enabled = ansi_music_enabled;
            app.input_height = input_height;
            app.settings.theme = Theme::from_name(&console_theme);
            app.settings.gui_theme = Theme::from_name(&gui_theme);
            app.settings.gui_transparency = gui_transparency;
            app.settings.color_offset_percent = color_offset_percent;
            app.settings.font_name = font_name;
            app.settings.font_size = font_size;
            app.settings.web_font_size_phone = web_font_size_phone;
            app.settings.web_font_size_tablet = web_font_size_tablet;
            app.settings.web_font_size_desktop = web_font_size_desktop;
            app.settings.websocket_allow_list = ws_allow_list;
            app.settings.web_secure = web_secure;
            app.settings.http_enabled = http_enabled;
            app.settings.http_port = http_port;
            app.settings.ws_enabled = ws_enabled;
            app.settings.ws_port = ws_port;
            app.settings.websocket_cert_file = ws_cert_file;
            app.settings.websocket_key_file = ws_key_file;
            app.settings.tls_proxy_enabled = tls_proxy_enabled;
            if app.settings.dictionary_path != dictionary_path {
                app.settings.dictionary_path = dictionary_path;
                app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
            }

            // Save settings
            let _ = persistence::save_settings(app);

            // Broadcast updated settings
            let settings = GlobalSettingsMsg {
                more_mode_enabled: app.settings.more_mode_enabled,
                spell_check_enabled: app.settings.spell_check_enabled,
                temp_convert_enabled: app.settings.temp_convert_enabled,
                world_switch_mode: app.settings.world_switch_mode.name().to_string(),
                debug_enabled: app.settings.debug_enabled,
                show_tags: app.show_tags,
                ansi_music_enabled: app.settings.ansi_music_enabled,
                input_height: app.input_height,
                console_theme: app.settings.theme.name().to_string(),
                gui_theme: app.settings.gui_theme.name().to_string(),
                gui_transparency: app.settings.gui_transparency,
                color_offset_percent: app.settings.color_offset_percent,
                font_name: app.settings.font_name.clone(),
                font_size: app.settings.font_size,
                web_font_size_phone: app.settings.web_font_size_phone,
                web_font_size_tablet: app.settings.web_font_size_tablet,
                web_font_size_desktop: app.settings.web_font_size_desktop,
                ws_allow_list: app.settings.websocket_allow_list.clone(),
                web_secure: app.settings.web_secure,
                http_enabled: app.settings.http_enabled,
                http_port: app.settings.http_port,
                ws_enabled: app.settings.ws_enabled,
                ws_port: app.settings.ws_port,
                ws_cert_file: app.settings.websocket_cert_file.clone(),
                ws_key_file: app.settings.websocket_key_file.clone(),
                tls_proxy_enabled: app.settings.tls_proxy_enabled,
                dictionary_path: app.settings.dictionary_path.clone(),
                theme_colors_json: app.gui_theme_colors().to_json(),
            };
            app.ws_broadcast(WsMessage::GlobalSettingsUpdated { settings, input_height: app.input_height });
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
        WsMessage::SendGmcp { world_index, package, data } => {
            if world_index < app.worlds.len() {
                if let Some(ref tx) = app.worlds[world_index].command_tx {
                    let msg = crate::telnet::build_gmcp_message(&package, &data);
                    let _ = tx.try_send(WriteCommand::Raw(msg));
                }
            }
        }
        WsMessage::SendMsdp { world_index, variable, value } => {
            if world_index < app.worlds.len() {
                if let Some(ref tx) = app.worlds[world_index].command_tx {
                    let msg = crate::telnet::build_msdp_set(&variable, &value);
                    let _ = tx.try_send(WriteCommand::Raw(msg));
                }
            }
        }
        WsMessage::Ping => {
            app.ws_send_to_client(client_id, WsMessage::Pong);
        }
        WsMessage::UpdateViewState { world_index, visible_lines } => {
            // Track client's view state for more-mode threshold calculation
            if world_index < app.worlds.len() {
                let dimensions = app.ws_client_worlds.get(&client_id).and_then(|s| s.dimensions);
                app.ws_client_worlds.insert(client_id, ClientViewState { world_index, visible_lines, dimensions });
            }
        }
        WsMessage::MarkWorldSeen { world_index } => {
            if world_index < app.worlds.len() {
                app.worlds[world_index].unseen_lines = 0;
                app.worlds[world_index].first_unseen_at = None;
                app.ws_broadcast(WsMessage::UnseenCleared { world_index });
                app.broadcast_activity();
                // Trigger console redraw to update activity indicator
                app.needs_output_redraw = true;
            }
        }
        WsMessage::ReleasePending { world_index, count } => {
            // Release pending lines for the specified world
            if world_index < app.worlds.len() {
                let pending_count = app.worlds[world_index].pending_lines.len();
                if pending_count > 0 {
                    // Get client's output width for visual line calculation
                    let client_width = app.ws_client_worlds.get(&client_id)
                        .and_then(|s| s.dimensions)
                        .map(|(w, _)| w as usize)
                        .unwrap_or(app.output_width as usize);

                    // count == 0 means release all; otherwise treat count as visual budget
                    let visual_budget = if count == 0 { usize::MAX } else { count };

                    // Pre-calculate logical lines to release (mirrors release_pending logic)
                    let width = client_width.max(1);
                    let mut visual_total = 0;
                    let mut logical_count = 0;
                    for line in &app.worlds[world_index].pending_lines {
                        let vl = visual_line_count(&line.text, width);
                        if visual_total > 0 && visual_total + vl > visual_budget {
                            break;
                        }
                        visual_total += vl;
                        logical_count += 1;
                        if visual_total >= visual_budget {
                            break;
                        }
                    }
                    if logical_count == 0 && pending_count > 0 {
                        logical_count = 1;
                    }
                    let to_release = logical_count.min(pending_count);

                    // Get the lines that will be released (for broadcasting)
                    let lines_to_broadcast: Vec<String> = app.worlds[world_index]
                        .pending_lines
                        .iter()
                        .take(to_release)
                        .map(|line| line.text.replace('\r', ""))
                        .collect();

                    // Release the lines
                    app.worlds[world_index].release_pending(visual_budget, client_width);

                    // Broadcast the released lines to clients viewing this world,
                    // but skip clients that already have these lines from InitialState
                    if !lines_to_broadcast.is_empty() {
                        let ws_data = lines_to_broadcast.join("\n") + "\n";
                        app.ws_broadcast_to_world(world_index, WsMessage::ServerData {
                            world_index,
                            data: ws_data,
                            is_viewed: true,
                            ts: current_timestamp_secs(),
                            from_server: false,
                        });
                    }

                    // Broadcast release event and updated pending count
                    app.ws_broadcast(WsMessage::PendingReleased { world_index, count: to_release });
                    let new_pending_count = app.worlds[world_index].pending_lines.len();
                    app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index, count: new_pending_count });

                    app.broadcast_activity();
                }
            }
        }
        WsMessage::ReportSeqMismatch { world_index, expected_seq_gt, actual_seq, line_text, source } => {
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?");
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true).append(true)
                .open("clay.output.debug")
            {
                let _ = writeln!(f, "SEQ MISMATCH [{}] in '{}': expected seq>{}, got seq={}, text={:?}",
                    source, world_name, expected_seq_gt, actual_seq,
                    line_text.chars().take(80).collect::<String>());
            }
        }
        WsMessage::ClientTypeDeclaration { client_type } => {
            // Update client type in WebSocket server
            if let Some(ref server) = app.ws_server {
                server.set_client_type(client_id, client_type);
            }
        }
        WsMessage::CycleWorld { direction } => {
            // Client requests to cycle to next/previous world
            // Apply server's world switching rules and send result
            let current = app.ws_client_worlds.get(&client_id)
                .map(|s| s.world_index)
                .unwrap_or(app.current_world_index);

            let new_index = if direction == "up" {
                app.calculate_prev_world_from(current)
            } else {
                app.calculate_next_world_from(current)
            };

            if let Some(idx) = new_index {
                // Update client's view state (sync state)
                let visible_lines = app.ws_client_worlds.get(&client_id)
                    .map(|s| s.visible_lines)
                    .unwrap_or(24);
                let dimensions = app.ws_client_worlds.get(&client_id)
                    .and_then(|s| s.dimensions);
                app.ws_client_worlds.insert(client_id, ClientViewState {
                    world_index: idx,
                    visible_lines,
                    dimensions,
                });
                // Update client's world in WebSocket server (async state)
                app.ws_set_client_world(client_id, Some(idx));

                // Send world switch result with state
                if idx < app.worlds.len() {
                    let pending_count = app.worlds[idx].pending_lines.len();
                    let paused = app.worlds[idx].paused;
                    let world_name = app.worlds[idx].name.clone();

                    app.ws_send_to_client(client_id, WsMessage::WorldSwitchResult {
                        world_index: idx,
                        world_name,
                        pending_count,
                        paused,
                    });

                    // Send initial output lines based on client type
                    let client_type = app.ws_get_client_type(client_id);
                    let world = &app.worlds[idx];
                    let total_lines = world.output_lines.len();

                    let lines_to_send = match client_type {
                        Some(RemoteClientType::RemoteConsole) => {
                            // Console: last screenful (viewport - 2)
                            visible_lines.saturating_sub(2).min(total_lines)
                        }
                        _ => {
                            // Web/GUI: full history
                            total_lines
                        }
                    };

                    if lines_to_send > 0 {
                        let start = total_lines.saturating_sub(lines_to_send);
                        let lines: Vec<TimestampedLine> = world.output_lines[start..].iter()
                            .map(|line| {
                                let ts = line.timestamp
                                    .duration_since(UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                TimestampedLine {
                                    text: line.text.clone(),
                                    ts,
                                    gagged: line.gagged,
                                    from_server: line.from_server,
                                    seq: line.seq,
                                    highlight_color: line.highlight_color.clone(),
                                }
                            })
                            .collect();

                        app.ws_send_to_client(client_id, WsMessage::OutputLines {
                            world_index: idx,
                            lines,
                            is_initial: true,
                        });
                    }

                    // Also mark world as seen if it had unseen output
                    if app.worlds[idx].unseen_lines > 0 {
                        app.worlds[idx].unseen_lines = 0;
                        app.worlds[idx].first_unseen_at = None;
                        app.ws_broadcast(WsMessage::UnseenCleared { world_index: idx });
                        app.broadcast_activity();
                    }
                }
            }
        }
        WsMessage::RequestScrollback { world_index, count, before_seq } => {
            // Console client requests scrollback from master
            if world_index < app.worlds.len() {
                let world = &app.worlds[world_index];

                // Find lines to send based on before_seq
                let lines: Vec<TimestampedLine> = if let Some(seq) = before_seq {
                    // Send lines with seq < before_seq (older than what client has)
                    let eligible: Vec<_> = world.output_lines.iter()
                        .filter(|l| l.seq < seq)
                        .collect();
                    let start = eligible.len().saturating_sub(count);
                    eligible[start..].iter()
                        .map(|line| {
                            let ts = line.timestamp
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            TimestampedLine {
                                text: line.text.clone(),
                                ts,
                                gagged: line.gagged,
                                from_server: line.from_server,
                                seq: line.seq,
                                highlight_color: line.highlight_color.clone(),
                            }
                        })
                        .collect()
                } else {
                    // No before_seq - send last N lines (backwards compatible)
                    let total_lines = world.output_lines.len();
                    let start = total_lines.saturating_sub(count);
                    world.output_lines[start..].iter()
                        .map(|line| {
                            let ts = line.timestamp
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0);
                            TimestampedLine {
                                text: line.text.clone(),
                                ts,
                                gagged: line.gagged,
                                from_server: line.from_server,
                                seq: line.seq,
                                highlight_color: line.highlight_color.clone(),
                            }
                        })
                        .collect()
                };

                app.ws_send_to_client(client_id, WsMessage::ScrollbackLines {
                    world_index,
                    lines,
                });
            }
        }
        WsMessage::CreateWorld { name } => {
            let new_world = World::new(&name);
            app.worlds.push(new_world);
            let idx = app.worlds.len() - 1;
            let world = &app.worlds[idx];
            let world_state = WorldStateMsg {
                index: idx,
                name: world.name.clone(),
                connected: false,
                output_lines: Vec::new(),
                pending_lines: Vec::new(),
                output_lines_ts: Vec::new(),
                pending_lines_ts: Vec::new(),
                prompt: String::new(),
                scroll_offset: 0,
                paused: false,
                unseen_lines: 0,
                settings: WorldSettingsMsg {
                    hostname: world.settings.hostname.clone(),
                    port: world.settings.port.clone(),
                    user: world.settings.user.clone(),
                    password: world.settings.password.clone(),
                    use_ssl: world.settings.use_ssl,
                    log_enabled: world.settings.log_enabled,
                    encoding: world.settings.encoding.name().to_string(),
                    auto_connect_type: world.settings.auto_connect_type.name().to_string(),
                    keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                    keep_alive_cmd: world.settings.keep_alive_cmd.clone(),
                    gmcp_packages: world.settings.gmcp_packages.clone(),
                },
                last_send_secs: None,
                last_recv_secs: None,
                last_nop_secs: None,
                keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                showing_splash: world.showing_splash,
                was_connected: false,
                is_proxy: false,
                gmcp_user_enabled: world.gmcp_user_enabled,
            };
            app.ws_broadcast(WsMessage::WorldAdded { world: Box::new(world_state) });
            let _ = persistence::save_settings(&app);
            app.ws_send_to_client(client_id, WsMessage::WorldCreated { world_index: idx });
        }
        WsMessage::DeleteWorld { world_index } => {
            if app.worlds.len() > 1 && world_index < app.worlds.len() {
                app.worlds.remove(world_index);
                if app.current_world_index >= app.worlds.len() {
                    app.current_world_index = app.worlds.len().saturating_sub(1);
                } else if app.current_world_index > world_index {
                    app.current_world_index -= 1;
                }
                if let Some(prev) = app.previous_world_index {
                    if prev >= app.worlds.len() {
                        app.previous_world_index = Some(app.worlds.len().saturating_sub(1));
                    } else if prev > world_index {
                        app.previous_world_index = Some(prev - 1);
                    }
                }
                app.ws_broadcast(WsMessage::WorldRemoved { world_index });
                let _ = persistence::save_settings(&app);
            }
        }
        WsMessage::UpdateWorldSettings { world_index, name, hostname, port, user, password, use_ssl, log_enabled, encoding, auto_login, keep_alive_type, keep_alive_cmd, gmcp_packages } => {
            if world_index < app.worlds.len() {
                app.worlds[world_index].name = name.clone();
                app.worlds[world_index].settings.hostname = hostname.clone();
                app.worlds[world_index].settings.port = port.clone();
                app.worlds[world_index].settings.user = user.clone();
                app.worlds[world_index].settings.password = password.clone();
                app.worlds[world_index].settings.use_ssl = use_ssl;
                app.worlds[world_index].settings.log_enabled = log_enabled;
                app.worlds[world_index].settings.encoding = match encoding.as_str() {
                    "latin1" => Encoding::Latin1,
                    "fansi" => Encoding::Fansi,
                    _ => Encoding::Utf8,
                };
                app.worlds[world_index].settings.auto_connect_type = AutoConnectType::from_name(&auto_login);
                app.worlds[world_index].settings.keep_alive_type = KeepAliveType::from_name(&keep_alive_type);
                app.worlds[world_index].settings.keep_alive_cmd = keep_alive_cmd.clone();
                app.worlds[world_index].settings.gmcp_packages = gmcp_packages.clone();
                let _ = persistence::save_settings(&app);
                let settings_msg = WorldSettingsMsg {
                    hostname, port, user,
                    password: encrypt_password(&password),
                    use_ssl, log_enabled, encoding,
                    auto_connect_type: auto_login,
                    keep_alive_type, keep_alive_cmd, gmcp_packages,
                };
                app.ws_broadcast(WsMessage::WorldSettingsUpdated { world_index, settings: settings_msg, name });
            }
        }
        _ => {}
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
ws_enabled=true
ws_port=9002
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

    // Start WebSocket server (required for multiuser mode)
    if !app.settings.ws_enabled {
        eprintln!("Warning: WebSocket server not enabled. Enabling for multiuser mode.");
        app.settings.ws_enabled = true;
    }

    // Start the WebSocket server
    let mut server = WebSocketServer::new(
        &app.settings.websocket_password,
        app.settings.ws_port,
        &app.settings.websocket_allow_list,
        app.settings.websocket_whitelisted_host.clone(),
        app.multiuser_mode,
        app.ban_list.clone(),
    );

    // Configure TLS if secure mode and cert/key files are specified
    #[cfg(feature = "native-tls-backend")]
    let tls_configured = if app.settings.web_secure
        && !app.settings.websocket_cert_file.is_empty()
        && !app.settings.websocket_key_file.is_empty()
    {
        match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("Warning: Failed to configure WSS TLS: {}", e);
                false
            }
        }
    } else {
        false
    };
    #[cfg(feature = "rustls-backend")]
    let tls_configured = if app.settings.web_secure
        && !app.settings.websocket_cert_file.is_empty()
        && !app.settings.websocket_key_file.is_empty()
    {
        match server.configure_tls(&app.settings.websocket_cert_file, &app.settings.websocket_key_file) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("Warning: Failed to configure WSS TLS: {}", e);
                false
            }
        }
    } else {
        false
    };

    // Add user credentials to the WebSocket server for multiuser authentication
    for user in &app.users {
        server.add_user(&user.name, &user.password);
    }

    if let Err(e) = start_websocket_server(&mut server, event_tx.clone()).await {
        eprintln!("Failed to start WebSocket server: {}", e);
        return Ok(());
    }
    let protocol = if tls_configured { "wss" } else { "ws" };
    println!("WebSocket server started on port {} ({})", app.settings.ws_port, protocol);
    app.ws_server = Some(server);

    // Start HTTP server if enabled
    if app.settings.http_enabled {
        let mut http_server = HttpServer::new(app.settings.http_port);
        match start_http_server(&mut http_server, app.settings.ws_port, false, app.ban_list.clone(), app.gui_theme_colors().to_css_vars()).await {
            Ok(()) => {
                println!("HTTP server started on port {}", app.settings.http_port);
                app.http_server = Some(http_server);
            }
            Err(e) => {
                eprintln!("Warning: Failed to start HTTP server: {}", e);
            }
        }
    }

    println!("Multiuser server running. Press Ctrl+C to stop.");

    // Create interval for TF repeat process ticks (1 second)
    let mut process_tick_interval = tokio::time::interval(std::time::Duration::from_secs(1));
    process_tick_interval.tick().await; // Skip first immediate tick

    // Main event loop - only handles WebSocket events
    loop {
        // Reap any zombie child processes (TLS proxies that have exited)
        #[cfg(all(unix, not(target_os = "android")))]
        reap_zombie_children();

        tokio::select! {
            // TF repeat process tick
            _ = process_tick_interval.tick() => {
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
                                    if let Some(tx) = &app.worlds[idx].command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(text));
                                    }
                                }
                            }
                            tf::TfCommandResult::Success(Some(msg)) => {
                                if let Some(ws) = &app.ws_server {
                                    ws.broadcast_to_all(WsMessage::ServerData {
                                        world_index: world_idx,
                                        data: msg,
                                        is_viewed: true,
                                        ts: current_timestamp_secs(),
                                        from_server: false,
                                    });
                                }
                            }
                            tf::TfCommandResult::Error(err) => {
                                if let Some(ws) = &app.ws_server {
                                    ws.broadcast_to_all(WsMessage::ServerData {
                                        world_index: world_idx,
                                        data: format!("Error: {}", err),
                                        is_viewed: true,
                                        ts: current_timestamp_secs(),
                                        from_server: false,
                                    });
                                }
                            }
                            tf::TfCommandResult::RepeatProcess(process) => {
                                app.tf_engine.processes.push(process);
                            }
                            tf::TfCommandResult::NotTfCommand => {
                                // Plain text command - send to MUD
                                if let Some(idx) = target_idx {
                                    if let Some(tx) = &app.worlds[idx].command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                                    }
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
                                    ws.broadcast_to_owner(WsMessage::ServerData {
                                        world_index,
                                        data: "No connection settings configured for this world.\n".to_string(),
                                        is_viewed: true,
                                        ts: current_timestamp_secs(),
                                        from_server: false,
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
                                    ws.broadcast_to_owner(WsMessage::ServerData {
                                        world_index,
                                        data: "\u{2728} Connection failed.\n".to_string(),
                                        is_viewed: true,
                                        ts: current_timestamp_secs(),
                                        from_server: false,
                                    }, Some(&requesting_username));
                                }
                            }
                        }
                    }
                    AppEvent::MultiuserServerData(world_index, username, data) => {
                        // Route server data to specific user's connection
                        let key = (world_index, username.clone());
                        if let Some(conn) = app.user_connections.get_mut(&key) {
                            let encoding = if world_index < app.worlds.len() {
                                app.worlds[world_index].settings.encoding
                            } else {
                                Encoding::Utf8
                            };
                            let decoded = encoding.decode(&data);

                            // Add to user's output buffer
                            for line in decoded.lines() {
                                let seq = conn.output_lines.len() as u64;
                                conn.output_lines.push(OutputLine::new(line.to_string(), seq));
                            }

                            // Send to this user's WebSocket clients only
                            if let Some(ws) = &app.ws_server {
                                ws.broadcast_to_owner(WsMessage::ServerData {
                                    world_index,
                                    data: decoded,
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                }, Some(&username));
                            }
                        }
                    }
                    AppEvent::MultiuserDisconnected(world_index, username) => {
                        // Handle disconnect for specific user's connection
                        let key = (world_index, username.clone());
                        if let Some(conn) = app.user_connections.get_mut(&key) {
                            conn.connected = false;
                            conn.command_tx = None;

                            // Send disconnect to this user only
                            if let Some(ws) = &app.ws_server {
                                ws.broadcast_to_owner(
                                    WsMessage::WorldDisconnected { world_index },
                                    Some(&username)
                                );
                            }
                        }
                    }
                    AppEvent::MultiuserTelnetDetected(world_index, username) => {
                        let key = (world_index, username.clone());
                        if let Some(conn) = app.user_connections.get_mut(&key) {
                            conn.telnet_mode = true;
                        }
                    }
                    AppEvent::MultiuserPrompt(world_index, username, prompt_bytes) => {
                        let key = (world_index, username.clone());
                        if let Some(conn) = app.user_connections.get_mut(&key) {
                            let encoding = if world_index < app.worlds.len() {
                                app.worlds[world_index].settings.encoding
                            } else {
                                Encoding::Utf8
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
                    _ => {} // Ignore other events in multiuser mode
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down multiuser server...");
                break;
            }
        }
    }

    Ok(())
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
            let (mut read_half, mut write_half): (StreamReader, StreamWriter) = if use_ssl {
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
                    root_store.roots = webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| { rustls::pki_types::TrustAnchor { subject: ta.subject.into(), subject_public_key_info: ta.spki.into(), name_constraints: ta.name_constraints.map(|nc| nc.into()), } }).collect();

                    let config = rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(crate::danger::NoCertificateVerification::new()))
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

            let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);

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
            let telnet_tx = cmd_tx.clone();
            let event_tx_read = event_tx.clone();
            let username_read = username.clone();

            // Spawn reader task
            tokio::spawn(async move {
                let mut buffer = BytesMut::with_capacity(4096);
                buffer.resize(4096, 0);
                let mut line_buffer: Vec<u8> = Vec::new();

                loop {
                    match read_half.read(&mut buffer).await {
                        Ok(0) => {
                            // Connection closed
                            if !line_buffer.is_empty() {
                                let result = process_telnet(&line_buffer);
                                if !result.responses.is_empty() {
                                    let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                }
                                if result.telnet_detected {
                                    let _ = event_tx_read.send(AppEvent::MultiuserTelnetDetected(world_index, username_read.clone())).await;
                                }
                                if let Some(prompt_bytes) = result.prompt {
                                    let _ = event_tx_read.send(AppEvent::MultiuserPrompt(world_index, username_read.clone(), prompt_bytes)).await;
                                }
                                if !result.cleaned.is_empty() {
                                    let _ = event_tx_read.send(AppEvent::MultiuserServerData(world_index, username_read.clone(), result.cleaned)).await;
                                }
                            }
                            let _ = event_tx_read.send(AppEvent::MultiuserServerData(
                                world_index,
                                username_read.clone(),
                                "Connection closed by server.\n".as_bytes().to_vec(),
                            )).await;
                            let _ = event_tx_read.send(AppEvent::MultiuserDisconnected(world_index, username_read.clone())).await;
                            break;
                        }
                        Ok(n) => {
                            line_buffer.extend_from_slice(&buffer[..n]);
                            let split_at = find_safe_split_point(&line_buffer);
                            let to_send = if split_at > 0 {
                                line_buffer.drain(..split_at).collect()
                            } else if !line_buffer.is_empty() {
                                std::mem::take(&mut line_buffer)
                            } else {
                                Vec::new()
                            };

                            if !to_send.is_empty() {
                                let result = process_telnet(&to_send);
                                if !result.responses.is_empty() {
                                    let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                }
                                if result.telnet_detected {
                                    let _ = event_tx_read.send(AppEvent::MultiuserTelnetDetected(world_index, username_read.clone())).await;
                                }
                                if let Some(prompt_bytes) = result.prompt {
                                    let _ = event_tx_read.send(AppEvent::MultiuserPrompt(world_index, username_read.clone(), prompt_bytes)).await;
                                }
                                if !result.cleaned.is_empty() {
                                    let _ = event_tx_read.send(AppEvent::MultiuserServerData(world_index, username_read.clone(), result.cleaned)).await;
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!("Read error: {}", e);
                            let _ = event_tx_read.send(AppEvent::MultiuserServerData(world_index, username_read.clone(), msg.into_bytes())).await;
                            let _ = event_tx_read.send(AppEvent::MultiuserDisconnected(world_index, username_read.clone())).await;
                            break;
                        }
                    }
                }
            });

            // Spawn writer task
            tokio::spawn(async move {
                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        WriteCommand::Text(text) => {
                            let bytes = format!("{}\r\n", text).into_bytes();
                            if write_half.write_all(&bytes).await.is_err() {
                                break;
                            }
                            let _ = write_half.flush().await;
                        }
                        WriteCommand::Raw(raw) => {
                            if write_half.write_all(&raw).await.is_err() {
                                break;
                            }
                            let _ = write_half.flush().await;
                        }
                        WriteCommand::Shutdown => {
                            // Gracefully shutdown the connection
                            let _ = write_half.shutdown().await;
                            break;
                        }
                    }
                }
            });

            Some(cmd_tx)
        }
        Err(_) => None,
    }
}

/// Connect a world in daemon mode (non-multiuser)
/// Returns a command sender if connection succeeds
pub async fn connect_daemon_world(
    _world_index: usize,
    world_name: String,
    settings: &WorldSettings,
    event_tx: mpsc::Sender<AppEvent>,
    connection_id: u64,
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
            let (mut read_half, mut write_half): (StreamReader, StreamWriter) = if use_ssl {
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
                    root_store.roots = webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| { rustls::pki_types::TrustAnchor { subject: ta.subject.into(), subject_public_key_info: ta.spki.into(), name_constraints: ta.name_constraints.map(|nc| nc.into()), } }).collect();

                    let config = rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(crate::danger::NoCertificateVerification::new()))
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

            let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);

            // Send auto-login if configured
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

            // Clone for reader task
            let telnet_tx = cmd_tx.clone();
            let event_tx_read = event_tx.clone();
            let world_name_read = world_name.clone();
            let reader_conn_id = connection_id;

            // Spawn reader task
            tokio::spawn(async move {
                let mut buffer = BytesMut::with_capacity(4096);
                buffer.resize(4096, 0);
                let mut line_buffer: Vec<u8> = Vec::new();

                loop {
                    match read_half.read(&mut buffer).await {
                        Ok(0) => {
                            // Connection closed
                            if !line_buffer.is_empty() {
                                let result = process_telnet(&line_buffer);
                                if !result.responses.is_empty() {
                                    let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                }
                                if !result.cleaned.is_empty() {
                                    let _ = event_tx_read.send(AppEvent::ServerData(world_name_read.clone(), result.cleaned)).await;
                                }
                            }
                            let _ = event_tx_read.send(AppEvent::ServerData(
                                world_name_read.clone(),
                                "Connection closed by server.\n".as_bytes().to_vec(),
                            )).await;
                            let _ = event_tx_read.send(AppEvent::Disconnected(world_name_read.clone(), reader_conn_id)).await;
                            break;
                        }
                        Ok(n) => {
                            line_buffer.extend_from_slice(&buffer[..n]);
                            let split_at = find_safe_split_point(&line_buffer);
                            let to_send = if split_at > 0 {
                                line_buffer.drain(..split_at).collect()
                            } else if !line_buffer.is_empty() {
                                std::mem::take(&mut line_buffer)
                            } else {
                                Vec::new()
                            };

                            if !to_send.is_empty() {
                                let result = process_telnet(&to_send);
                                if !result.responses.is_empty() {
                                    let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                }
                                if !result.cleaned.is_empty() {
                                    let _ = event_tx_read.send(AppEvent::ServerData(world_name_read.clone(), result.cleaned)).await;
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!("Read error: {}", e);
                            let _ = event_tx_read.send(AppEvent::ServerData(world_name_read.clone(), msg.into_bytes())).await;
                            let _ = event_tx_read.send(AppEvent::Disconnected(world_name_read.clone(), reader_conn_id)).await;
                            break;
                        }
                    }
                }
            });

            // Spawn writer task
            tokio::spawn(async move {
                while let Some(cmd) = cmd_rx.recv().await {
                    match cmd {
                        WriteCommand::Text(text) => {
                            let bytes = format!("{}\r\n", text).into_bytes();
                            if write_half.write_all(&bytes).await.is_err() {
                                break;
                            }
                            let _ = write_half.flush().await;
                        }
                        WriteCommand::Raw(raw) => {
                            if write_half.write_all(&raw).await.is_err() {
                                break;
                            }
                            let _ = write_half.flush().await;
                        }
                        WriteCommand::Shutdown => {
                            let _ = write_half.shutdown().await;
                            break;
                        }
                    }
                }
            });

            Some(cmd_tx)
        }
        Err(_) => None,
    }
}

/// Build initial state message for a specific user (multiuser mode)
/// World definitions are shared, but connection state is per-user
/// Actions are still filtered per-user
pub fn build_multiuser_initial_state(app: &App, username: &str) -> WsMessage {
    // Show all worlds with per-user connection state
    let worlds: Vec<WorldStateMsg> = app.worlds.iter().enumerate()
        .map(|(idx, world)| {
            // Get user's connection state for this world (if any)
            let key = (idx, username.to_string());
            let user_conn = app.user_connections.get(&key);

            // Use user's connection state or empty defaults
            let empty_output: Vec<OutputLine> = vec![];
            let empty_pending: Vec<OutputLine> = vec![];
            let (connected, output_lines, pending_lines, prompt, scroll_offset, paused, unseen_lines, last_send, last_recv) =
                if let Some(conn) = user_conn {
                    (
                        conn.connected,
                        &conn.output_lines,
                        &conn.pending_lines,
                        conn.prompt.clone(),
                        conn.scroll_offset,
                        conn.paused,
                        conn.unseen_lines,
                        conn.last_send_time,
                        conn.last_receive_time,
                    )
                } else {
                    (false, &empty_output, &empty_pending, String::new(), 0, false, 0, None, None)
                };

            let clean_output: Vec<String> = output_lines.iter()
                .map(|s| s.text.replace('\r', ""))
                .collect();
            let clean_pending: Vec<String> = pending_lines.iter()
                .map(|s| s.text.replace('\r', ""))
                .collect();
            let output_lines_ts: Vec<TimestampedLine> = output_lines.iter()
                .map(|s| {
                    let text = s.text.replace('\r', "");
                    let text = if !s.from_server {
                        format!("\u{2728} {}", text)
                    } else {
                        text
                    };
                    TimestampedLine {
                        text,
                        ts: s.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                        gagged: s.gagged,
                        from_server: s.from_server,
                        seq: s.seq,
                        highlight_color: s.highlight_color.clone(),
                    }
                })
                .collect();
            let pending_lines_ts: Vec<TimestampedLine> = pending_lines.iter()
                .map(|s| {
                    let text = s.text.replace('\r', "");
                    let text = if !s.from_server {
                        format!("\u{2728} {}", text)
                    } else {
                        text
                    };
                    TimestampedLine {
                        text,
                        ts: s.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                        gagged: s.gagged,
                        from_server: s.from_server,
                        seq: s.seq,
                        highlight_color: s.highlight_color.clone(),
                    }
                })
                .collect();

            WorldStateMsg {
                index: idx,
                name: world.name.clone(),
                connected,
                output_lines: clean_output,
                pending_lines: clean_pending,
                output_lines_ts,
                pending_lines_ts,
                prompt: prompt.replace('\r', ""),
                scroll_offset,
                paused,
                unseen_lines,
                settings: WorldSettingsMsg {
                    hostname: world.settings.hostname.clone(),
                    port: world.settings.port.clone(),
                    user: world.settings.user.clone(),
                    password: world.settings.password.clone(),
                    use_ssl: world.settings.use_ssl,
                    log_enabled: world.settings.log_enabled,
                    encoding: world.settings.encoding.name().to_string(),
                    auto_connect_type: world.settings.auto_connect_type.name().to_string(),
                    keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                    keep_alive_cmd: world.settings.keep_alive_cmd.clone(),
                    gmcp_packages: world.settings.gmcp_packages.clone(),
                },
                last_send_secs: last_send.map(|t| t.elapsed().as_secs()),
                last_recv_secs: last_recv.map(|t| t.elapsed().as_secs()),
                last_nop_secs: None,
                keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                showing_splash: world.showing_splash,
                was_connected: world.was_connected,
                is_proxy: world.proxy_pid.is_some(),
                gmcp_user_enabled: world.gmcp_user_enabled,
            }
        }).collect();

    // Filter actions owned by this user
    let actions: Vec<Action> = app.settings.actions.iter()
        .filter(|a| a.owner.as_deref() == Some(username))
        .cloned()
        .collect();

    // Build settings (same for all users for now)
    let settings = GlobalSettingsMsg {
        more_mode_enabled: app.settings.more_mode_enabled,
        spell_check_enabled: app.settings.spell_check_enabled,
        temp_convert_enabled: app.settings.temp_convert_enabled,
        world_switch_mode: app.settings.world_switch_mode.name().to_string(),
        debug_enabled: app.settings.debug_enabled,
        show_tags: app.show_tags,
        ansi_music_enabled: app.settings.ansi_music_enabled,
        console_theme: app.settings.theme.name().to_string(),
        gui_theme: app.settings.gui_theme.name().to_string(),
        gui_transparency: app.settings.gui_transparency,
        color_offset_percent: app.settings.color_offset_percent,
        input_height: app.input_height,
        font_name: app.settings.font_name.clone(),
        font_size: app.settings.font_size,
        web_font_size_phone: app.settings.web_font_size_phone,
        web_font_size_tablet: app.settings.web_font_size_tablet,
        web_font_size_desktop: app.settings.web_font_size_desktop,
        ws_allow_list: app.settings.websocket_allow_list.clone(),
        web_secure: app.settings.web_secure,
        http_enabled: app.settings.http_enabled,
        http_port: app.settings.http_port,
        ws_enabled: app.settings.ws_enabled,
        ws_port: app.settings.ws_port,
        ws_cert_file: app.settings.websocket_cert_file.clone(),
        ws_key_file: app.settings.websocket_key_file.clone(),
        tls_proxy_enabled: app.settings.tls_proxy_enabled,
        dictionary_path: app.settings.dictionary_path.clone(),
        theme_colors_json: app.gui_theme_colors().to_json(),
    };

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
        worlds,
        settings,
        current_world_index,
        actions,
        splash_lines,
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
        WsMessage::AuthRequest { .. } => {
            // Client just authenticated - send them their InitialState filtered by username
            if let Some(ref uname) = username {
                let initial_state = build_multiuser_initial_state(app, uname);
                if let Some(ws) = &app.ws_server {
                    ws.send_to_client(client_id, initial_state);
                }
            }
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
            // Any user can connect to any world
            if world_index < app.worlds.len() {
                if let Some(ref uname) = username {
                    let _ = event_tx.send(AppEvent::ConnectWorldRequest(world_index, uname.clone())).await;
                }
            }
        }
        WsMessage::DisconnectWorld { world_index } => {
            // Disconnect user's own connection
            if let Some(ref uname) = username {
                let key = (world_index, uname.clone());
                if let Some(conn) = app.user_connections.get_mut(&key) {
                    conn.command_tx = None;
                    conn.connected = false;
                    // Notify the user
                    if let Some(ws) = &app.ws_server {
                        ws.broadcast_to_owner(
                            WsMessage::WorldDisconnected { world_index },
                            Some(uname)
                        );
                    }
                }
            }
        }
        WsMessage::ChangePassword { old_password_hash, new_password_hash } => {
            if let Some(ref uname) = username {
                // Find the user and verify old password
                if let Some(user) = app.users.iter_mut().find(|u| &u.name == uname) {
                    let old_hash = hash_password(&user.password);
                    if old_hash == old_password_hash {
                        // Update password (store the hash, will be encrypted on save)
                        user.password = new_password_hash.clone();
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
                    ws.send_to_client(client_id, initial_state);
                    // Mark client as having received initial state so it receives broadcasts
                    ws.mark_initial_state_sent(client_id);
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
        WsMessage::MarkWorldSeen { world_index } => {
            // Verify the client owns this world
            if let Some(world) = app.worlds.get_mut(world_index) {
                if world.owner.as_ref() == username.as_ref() {
                    world.unseen_lines = 0;
                    // Broadcast to all clients of this owner
                    if let Some(ws) = &app.ws_server {
                        ws.broadcast_to_owner(WsMessage::UnseenCleared { world_index }, world.owner.as_deref());
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
                    world.output_lines.extend(released);

                    if world.pending_lines.is_empty() {
                        world.paused = false;
                    }

                    // Broadcast to all clients of this owner
                    if let Some(ws) = &app.ws_server {
                        ws.broadcast_to_owner(WsMessage::PendingReleased { world_index, count: release_count }, world.owner.as_deref());
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
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?");
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true).append(true)
                .open("clay.output.debug")
            {
                let _ = writeln!(f, "SEQ MISMATCH [{}] in '{}': expected seq>{}, got seq={}, text={:?}",
                    source, world_name, expected_seq_gt, actual_seq,
                    line_text.chars().take(80).collect::<String>());
            }
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
        WsMessage::SendGmcp { world_index, package, data } => {
            if world_index < app.worlds.len() {
                if let Some(ref tx) = app.worlds[world_index].command_tx {
                    let msg = crate::telnet::build_gmcp_message(&package, &data);
                    let _ = tx.try_send(WriteCommand::Raw(msg));
                }
            }
        }
        WsMessage::SendMsdp { world_index, variable, value } => {
            if world_index < app.worlds.len() {
                if let Some(ref tx) = app.worlds[world_index].command_tx {
                    let msg = crate::telnet::build_msdp_set(&variable, &value);
                    let _ = tx.try_send(WriteCommand::Raw(msg));
                }
            }
        }
        _ => {} // Handle other messages as needed
    }
}
