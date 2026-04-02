//! Remote client support: console client mode and grep client mode.
//! These functions handle connecting to a Clay daemon via WebSocket and
//! providing a TUI interface or command-line grep for remote sessions.

use std::io::{self, Write as IoWrite};
use std::sync::Arc;

use crossterm::{
    execute,
    event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
            MouseEventKind, MouseButton, EnableMouseCapture, DisableMouseCapture},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use tokio::sync::mpsc;

use crate::{
    App, OutputLine, Command,
    WsMessage, WorldStateMsg, TimestampedLine,
    hash_password, hash_with_challenge,
    strip_ansi_codes, local_time_from_epoch,
    Encoding, Theme, WorldSwitchMode, SpellChecker,
    get_version_string, parse_command,
    UpdateSuccess,
    NewPopupAction, WebSettings,
    ActionsListAction,
    EditorSide, AutoConnectType, KeepAliveType,
    web_settings_from_custom_data, handle_new_popup_key,
    websocket, popup, keybindings, tf, platform,
};
use crate::input_handler::*;
use crate::rendering::*;
use crate::platform::*;

/// Run as grep client connecting to remote daemon (--grep=host:port)
/// Searches world output for matching lines and prints to stdout
pub(crate) async fn run_grep_client(
    addr: &str,
    pattern: &str,
    world_filter: Option<&str>,
    use_regex: bool,
    strip_ansi: bool,
    follow_mode: bool,
) -> io::Result<()> {
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use futures::SinkExt;
    use regex::RegexBuilder;

    // Read password from environment
    let password = match std::env::var("CLAY_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("Error: CLAY_PASSWORD environment variable not set.");
            eprintln!("Set it with: export CLAY_PASSWORD=yourpassword");
            std::process::exit(1);
        }
    };

    // Compile pattern
    let regex_pattern = if use_regex {
        pattern.to_string()
    } else {
        tf::macros::glob_to_regex(pattern)
    };
    let re = match RegexBuilder::new(&regex_pattern).case_insensitive(true).build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: Invalid pattern '{}': {}", pattern, e);
            std::process::exit(1);
        }
    };

    // Connect to WebSocket server (same logic as run_console_client)
    let addr_with_port = if addr.starts_with("ws://") || addr.starts_with("wss://") || addr.contains(':') {
        addr.to_string()
    } else {
        format!("{}:9000", addr)
    };

    let (ws_url, try_fallback) = if addr_with_port.starts_with("ws://") || addr_with_port.starts_with("wss://") {
        (addr_with_port.clone(), false)
    } else {
        (format!("wss://{}", addr_with_port), true)
    };

    #[cfg(feature = "rustls-backend")]
    let connect_result = if ws_url.starts_with("wss://") {
        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(platform::danger::NoCertificateVerification::new()))
            .with_no_client_auth();
        let connector = tokio_tungstenite::Connector::Rustls(Arc::new(tls_config));
        tokio_tungstenite::connect_async_tls_with_config(
            &ws_url,
            None,
            false,
            Some(connector),
        ).await
    } else {
        connect_async(&ws_url).await
    };

    #[cfg(not(feature = "rustls-backend"))]
    let connect_result = connect_async(&ws_url).await;

    let (ws_stream, _) = match connect_result {
        Ok(result) => result,
        Err(_) if try_fallback => {
            let fallback_url = format!("ws://{}", addr_with_port);
            match connect_async(&fallback_url).await {
                Ok(result) => result,
                Err(e2) => {
                    eprintln!("Failed to connect to {}: {}", fallback_url, e2);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to connect to {}: {}", ws_url, e);
            std::process::exit(1);
        }
    };

    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Create a channel for sending messages to the WebSocket
    let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<WsMessage>();

    // Spawn task to forward messages to WebSocket
    tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // Wait for ServerHello to get challenge
    let server_challenge = loop {
        match ws_read.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(WsMessage::ServerHello { challenge, .. }) = serde_json::from_str::<WsMessage>(&text) {
                    break challenge;
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                eprintln!("Connection closed before authentication");
                std::process::exit(1);
            }
            _ => {}
        }
    };

    // Authenticate with challenge-response
    let password_hash = hash_password(&password);
    let challenge_hash = hash_with_challenge(&password_hash, &server_challenge);
    let _ = ws_tx.send(WsMessage::AuthRequest {
        password_hash: challenge_hash,
        username: None,
        current_world: None,
        auth_key: None,
        request_key: false,
        challenge_response: true,
    });

    // Wait for auth response
    loop {
        match ws_read.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(WsMessage::AuthResponse { success, error, .. }) = serde_json::from_str::<WsMessage>(&text) {
                    if !success {
                        eprintln!("Authentication failed: {}", error.unwrap_or_default());
                        std::process::exit(1);
                    }
                    break;
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                eprintln!("Connection closed during authentication");
                std::process::exit(1);
            }
            _ => {}
        }
    }

    // Wait for InitialState
    let worlds: Vec<WorldStateMsg> = loop {
        match ws_read.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(WsMessage::InitialState { worlds, .. }) = serde_json::from_str::<WsMessage>(&text) {
                    break worlds;
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                eprintln!("Connection closed before receiving state");
                std::process::exit(1);
            }
            _ => {}
        }
    };

    // Build world index→name map and validate world filter
    let mut world_names: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    for w in &worlds {
        world_names.insert(w.index, w.name.clone());
    }

    let filter_indices: Option<Vec<usize>> = if let Some(filter) = world_filter {
        let filter_lower = filter.to_lowercase();
        let matching: Vec<usize> = worlds.iter()
            .filter(|w| w.name.to_lowercase() == filter_lower)
            .map(|w| w.index)
            .collect();
        if matching.is_empty() {
            eprintln!("Error: World '{}' not found. Available worlds:", filter);
            for w in &worlds {
                eprintln!("  {}", w.name);
            }
            std::process::exit(1);
        }
        Some(matching)
    } else {
        None
    };

    let matches_world = |idx: usize| -> bool {
        match &filter_indices {
            Some(indices) => indices.contains(&idx),
            None => true,
        }
    };

    let format_line = |ts: u64, world_name: &str, text: &str| -> String {
        let lt = local_time_from_epoch(ts as i64);
        let display_text = if strip_ansi { strip_ansi_codes(text) } else { text.to_string() };
        format!("{:02}:{:02}:{:02}:{}  {}",
            lt.hour, lt.minute, lt.second, world_name, display_text)
    };

    if follow_mode {
        // Follow mode: match new output as it arrives
        // Enable raw mode to capture Ctrl+L for screen clearing
        let raw_mode_enabled = enable_raw_mode().is_ok();
        let mut key_reader = EventStream::new();

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    break;
                }
                key_event = key_reader.next() => {
                    match key_event {
                        Some(Ok(Event::Key(KeyEvent { code: KeyCode::Char('l'), modifiers, kind: KeyEventKind::Press, .. }))) if modifiers.contains(KeyModifiers::CONTROL) => {
                            // Clear screen and move cursor to top
                            print!("\x1b[2J\x1b[H");
                            use std::io::Write;
                            let _ = std::io::stdout().flush();
                        }
                        Some(Ok(Event::Key(KeyEvent { code: KeyCode::Char('c'), modifiers, kind: KeyEventKind::Press, .. }))) if modifiers.contains(KeyModifiers::CONTROL) => {
                            break;
                        }
                        _ => {}
                    }
                }
                msg = ws_read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<WsMessage>(&text) {
                                Ok(WsMessage::ServerData { world_index, data, from_server, ts, .. }) => {
                                    if !from_server { continue; }
                                    if !matches_world(world_index) { continue; }
                                    let world_name = world_names.get(&world_index).map(|s| s.as_str()).unwrap_or("?");
                                    for line in data.lines() {
                                        let clean = strip_ansi_codes(line);
                                        if re.is_match(&clean) {
                                            print!("{}\r\n", format_line(ts, world_name, line));
                                        }
                                    }
                                }
                                Ok(WsMessage::Ping) => {
                                    let _ = ws_tx.send(WsMessage::Pong);
                                }
                                Ok(WsMessage::PingCheck { nonce }) => {
                                    let _ = ws_tx.send(WsMessage::PongCheck { nonce });
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Restore terminal state
        if raw_mode_enabled {
            let _ = disable_raw_mode();
        }
    } else {
        // History search mode: request scrollback, search, exit
        let mut had_match = false;

        // Determine which worlds to search
        let search_worlds: Vec<(usize, String)> = worlds.iter()
            .filter(|w| matches_world(w.index))
            .map(|w| (w.index, w.name.clone()))
            .collect();

        // Request scrollback for each world
        for &(world_index, _) in &search_worlds {
            let _ = ws_tx.send(WsMessage::RequestScrollback {
                world_index,
                count: 10000,
                before_seq: None,
            });
        }

        // Collect all lines per world: scrollback + initial state lines
        let mut world_lines: std::collections::HashMap<usize, Vec<TimestampedLine>> = std::collections::HashMap::new();
        let mut pending_worlds: std::collections::HashSet<usize> = search_worlds.iter().map(|&(idx, _)| idx).collect();

        // Pre-populate with initial state lines
        for w in &worlds {
            if matches_world(w.index) {
                world_lines.insert(w.index, w.output_lines_ts.clone());
            }
        }

        // Receive scrollback responses
        while !pending_worlds.is_empty() {
            match ws_read.next().await {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<WsMessage>(&text) {
                        Ok(WsMessage::ScrollbackLines { world_index, lines, backfill_complete }) => {
                            if let Some(existing) = world_lines.get_mut(&world_index) {
                                // Scrollback lines come before existing lines
                                let mut merged = lines;
                                merged.append(existing);
                                *existing = merged;
                            }
                            if backfill_complete {
                                pending_worlds.remove(&world_index);
                            } else {
                                // Request more scrollback
                                if let Some(existing) = world_lines.get(&world_index) {
                                    let min_seq = existing.iter().map(|l| l.seq).min();
                                    let _ = ws_tx.send(WsMessage::RequestScrollback {
                                        world_index,
                                        count: 10000,
                                        before_seq: min_seq,
                                    });
                                }
                            }
                        }
                        Ok(WsMessage::Ping) => {
                            let _ = ws_tx.send(WsMessage::Pong);
                        }
                        Ok(WsMessage::PingCheck { nonce }) => {
                            let _ = ws_tx.send(WsMessage::PongCheck { nonce });
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    break;
                }
                _ => {}
            }
        }

        // Deduplicate by seq within each world and sort
        for lines in world_lines.values_mut() {
            lines.sort_by_key(|l| l.seq);
            lines.dedup_by_key(|l| l.seq);
        }

        // Collect all lines across worlds, sort by timestamp then seq
        let mut all_lines: Vec<(usize, &TimestampedLine)> = Vec::new();
        for (world_idx, lines) in &world_lines {
            for line in lines {
                all_lines.push((*world_idx, line));
            }
        }
        all_lines.sort_by(|a, b| a.1.ts.cmp(&b.1.ts).then(a.1.seq.cmp(&b.1.seq)));

        // Match and print
        for (world_idx, line) in &all_lines {
            if !line.from_server { continue; }
            let clean = strip_ansi_codes(&line.text);
            if re.is_match(&clean) {
                let world_name = world_names.get(world_idx).map(|s| s.as_str()).unwrap_or("?");
                println!("{}", format_line(line.ts, world_name, &line.text));
                had_match = true;
            }
        }

        // Exit code: 0 if matches found, 1 if none (grep convention)
        if !had_match {
            std::process::exit(1);
        }
    }

    Ok(())
}
/// Run as console client connecting to remote daemon (--console=host:port)
/// Uses the same App struct and ui() function as the normal console interface
pub(crate) async fn run_console_client(addr: &str) -> io::Result<()> {
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use futures::SinkExt;

    // Parse address - add default port 9000 if not specified, then wss:// prefix
    let addr_with_port = if addr.starts_with("ws://") || addr.starts_with("wss://") {
        addr.to_string()
    } else if addr.contains(':') {
        // Host:port already specified
        addr.to_string()
    } else {
        // No port specified - default to 9000 (same as --gui)
        format!("{}:9000", addr)
    };

    let (ws_url, try_fallback) = if addr_with_port.starts_with("ws://") || addr_with_port.starts_with("wss://") {
        (addr_with_port.clone(), false)
    } else {
        // Default to wss:// for security, will fall back to ws:// if it fails
        (format!("wss://{}", addr_with_port), true)
    };

    println!("Connecting to {}...", ws_url);

    // Connect to WebSocket server - for wss:// we need to configure TLS to accept self-signed certs
    #[cfg(feature = "rustls-backend")]
    let connect_result = if ws_url.starts_with("wss://") {
        // Configure rustls to accept self-signed/invalid certificates
        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(platform::danger::NoCertificateVerification::new()))
            .with_no_client_auth();
        let connector = tokio_tungstenite::Connector::Rustls(Arc::new(tls_config));
        tokio_tungstenite::connect_async_tls_with_config(
            &ws_url,
            None,
            false,
            Some(connector),
        ).await
    } else {
        connect_async(&ws_url).await
    };

    #[cfg(not(feature = "rustls-backend"))]
    let connect_result = connect_async(&ws_url).await;

    let (ws_stream, _) = match connect_result {
        Ok(result) => result,
        Err(e) if try_fallback => {
            // wss:// failed, try ws:// fallback
            let fallback_url = format!("ws://{}", addr_with_port);
            eprintln!("Secure connection failed ({}), trying {}...", e, fallback_url);
            match connect_async(&fallback_url).await {
                Ok(result) => result,
                Err(e2) => {
                    eprintln!("Failed to connect to {}: {}", fallback_url, e2);
                    return Ok(());
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to connect to {}: {}", ws_url, e);
            return Ok(());
        }
    };

    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Create a channel for sending messages to the WebSocket
    let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<WsMessage>();

    // Spawn task to forward messages to WebSocket
    let ws_write_handle = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // Wait for ServerHello to know if we're in multiuser mode and get challenge
    let (multiuser_mode, server_challenge) = loop {
        match ws_read.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(WsMessage::ServerHello { multiuser_mode: is_multiuser, challenge }) = serde_json::from_str::<WsMessage>(&text) {
                    break (is_multiuser, challenge);
                }
            }
            Some(Ok(Message::Close(_))) | None => {
                eprintln!("Connection closed before authentication");
                return Ok(());
            }
            _ => {}
        }
    };

    // Prompt for username if in multiuser mode
    let mut username: Option<String> = None;
    if multiuser_mode {
        print!("Username? ");
        let _ = io::stdout().flush();

        let mut user_input = String::new();
        enable_raw_mode()?;
        let mut event_stream = EventStream::new();
        loop {
            if let Some(Ok(Event::Key(key))) = event_stream.next().await {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Enter => {
                        disable_raw_mode()?;
                        println!();
                        username = Some(user_input);
                        break;
                    }
                    KeyCode::Char(c) => {
                        user_input.push(c);
                        print!("{}", c);
                        let _ = io::stdout().flush();
                    }
                    KeyCode::Backspace => {
                        if user_input.pop().is_some() {
                            print!("\x08 \x08"); // Backspace, space, backspace
                            let _ = io::stdout().flush();
                        }
                    }
                    KeyCode::Esc => {
                        disable_raw_mode()?;
                        println!();
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }

    // Prompt for password
    print!("Password? ");
    let _ = io::stdout().flush();

    // Read password with raw mode for character-by-character input
    let mut password = String::new();
    enable_raw_mode()?;
    let mut event_stream = EventStream::new();
    loop {
        tokio::select! {
            maybe_event = event_stream.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_event {
                    if key.kind != KeyEventKind::Press { continue; }
                    match key.code {
                        KeyCode::Enter => {
                            disable_raw_mode()?;
                            println!(); // Move to next line after password
                            // Send authentication with challenge-response
                            let password_hash = hash_password(&password);
                            let challenge_hash = hash_with_challenge(&password_hash, &server_challenge);
                            let _ = ws_tx.send(WsMessage::AuthRequest { password_hash: challenge_hash, username, current_world: None, auth_key: None, request_key: false, challenge_response: true });
                            break;
                        }
                        KeyCode::Char(c) => {
                            password.push(c);
                        }
                        KeyCode::Backspace => {
                            password.pop();
                        }
                        KeyCode::Esc => {
                            disable_raw_mode()?;
                            println!();
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(WsMessage::AuthResponse { success, error, .. }) = serde_json::from_str::<WsMessage>(&text) {
                            if !success {
                                disable_raw_mode()?;
                                println!();
                                eprintln!("Authentication failed: {}", error.unwrap_or_default());
                                return Ok(());
                            }
                            // Auth success - continue to wait for InitialState
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        disable_raw_mode()?;
                        println!();
                        eprintln!("Connection closed");
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    }

    // Create App struct and set it up for remote client mode
    let mut app = App::new();
    app.ws_client_tx = Some(ws_tx.clone());
    app.is_master = false;

    // Now set up the terminal for the main UI
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Wait for InitialState
    loop {
        if let Some(Ok(Message::Text(text))) = ws_read.next().await {
            if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                match ws_msg {
                    WsMessage::AuthResponse { success, error, .. } => {
                        if !success {
                            disable_raw_mode()?;
                            execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                            eprintln!("Authentication failed: {}", error.unwrap_or_default());
                            return Ok(());
                        }
                        // Auth success - continue waiting for InitialState
                    }
                    WsMessage::InitialState { worlds, current_world_index, settings, splash_lines, .. } => {
                        // Save world totals for backfill before consuming worlds vec
                        let world_totals: Vec<(usize, usize)> = worlds.iter()
                            .map(|w| (w.index, w.total_output_lines))
                            .collect();
                        // Initialize app state from server
                        app.init_from_initial_state(worlds, current_world_index, settings, splash_lines);
                        // Initialize backfill queue
                        app.init_backfill(&world_totals);
                        // Declare client type to server (RemoteConsole for TUI clients)
                        let _ = ws_tx.send(WsMessage::ClientTypeDeclaration {
                            client_type: websocket::RemoteClientType::RemoteConsole,
                        });
                        // Send initial view state for more-mode sync
                        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
                        let visible_lines = height.saturating_sub(4) as usize; // Account for input area and separator
                        let _ = ws_tx.send(WsMessage::UpdateViewState {
                            world_index: current_world_index,
                            visible_lines,
                            visible_columns: Some(width as usize),
                        });
                        break;
                    }
                    _ => {}
                }
            }
        } else {
            disable_raw_mode()?;
            let _ = execute!(terminal.backend_mut(), crossterm::event::DisableBracketedPaste);
            execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
            eprintln!("Connection closed while waiting for initial state");
            return Ok(());
        }
    }

    // Main event loop - use the same ui() function as the normal console
    app.needs_output_redraw = true;
    let mut needs_redraw = true;

    // Backfill timer: fires after initial delay to start requesting scrollback history
    let mut backfill_timer = std::pin::pin!(tokio::time::sleep(std::time::Duration::from_millis(500)));
    let mut backfill_timer_active = !app.backfill_queue.is_empty();

    // Channel for local /update results
    let (update_tx, mut update_rx) = mpsc::channel::<Result<UpdateSuccess, String>>(1);

    loop {
        // Draw if needed
        if needs_redraw || app.needs_output_redraw {
            // Check current popup visibility
            let any_popup_visible = app.has_new_popup() || app.confirm_dialog.visible;

            // When transitioning to popup: terminal.clear() resets ratatui's buffers
            // so the diff writes ALL popup cells (full coverage, no bleed-through).
            // Ratatui handles both output and popup rendering when popup is visible.
            if any_popup_visible && !app.popup_was_visible {
                terminal.clear()?;
            }
            app.popup_was_visible = any_popup_visible;

            // Toggle mouse capture when popup visibility changes
            if app.settings.mouse_enabled {
                let want_mouse = any_popup_visible;
                if want_mouse && !app.mouse_capture_active {
                    let _ = execute!(std::io::stdout(), EnableMouseCapture);
                    app.mouse_capture_active = true;
                } else if !want_mouse && app.mouse_capture_active {
                    let _ = execute!(std::io::stdout(), DisableMouseCapture);
                    app.mouse_capture_active = false;
                }
            }

            // Handle Ctrl+L terminal reset and redraw request
            if app.needs_terminal_clear {
                // Full terminal reset: unconditionally tear down and re-setup
                let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
                app.mouse_capture_active = false;
                let _ = execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
                let _ = crossterm::terminal::disable_raw_mode();
                let _ = execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
                crossterm::terminal::enable_raw_mode()?;
                execute!(
                    std::io::stdout(),
                    crossterm::terminal::EnterAlternateScreen,
                    crossterm::event::EnableBracketedPaste,
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                    crossterm::cursor::MoveTo(0, 0)
                )?;
                // Re-enable mouse capture only if a popup is currently visible
                if app.settings.mouse_enabled && app.has_new_popup() {
                    let _ = execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
                    app.mouse_capture_active = true;
                }
                terminal.clear()?;
                app.needs_terminal_clear = false;
            }

            terminal.draw(|f| ui(f, &mut app))?;
            // Render output with crossterm (bypasses ratatui's buggy ANSI handling)
            render_output_crossterm(&app);
            needs_redraw = false;
            app.needs_output_redraw = false;
        }

        tokio::select! {
            maybe_event = event_stream.next() => {
                if let Some(Ok(event)) = maybe_event {
                    match event {
                        Event::Mouse(mouse) if app.settings.mouse_enabled && app.has_new_popup() => {
                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    if !handle_popup_mouse_highlight_start(&mut app, mouse.column, mouse.row) {
                                        let button_clicked = handle_popup_mouse_click(&mut app, mouse.column, mouse.row);
                                        if button_clicked {
                                            let enter_key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                                            if handle_remote_client_key(&mut app, enter_key, &ws_tx) {
                                                break;
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    handle_popup_mouse_highlight_drag(&mut app, mouse.column, mouse.row);
                                }
                                MouseEventKind::Up(MouseButton::Left) => {
                                    handle_popup_mouse_highlight_end(&mut app);
                                }
                                MouseEventKind::ScrollUp => {
                                    if let Some(state) = app.popup_manager.current_mut() {
                                        state.mouse_scroll_up();
                                    }
                                    // Fast direct render bypassing ratatui
                                    let tc = app.settings.theme;
                                    if let Some(state) = app.popup_manager.current() {
                                        popup::console_renderer::render_popup_content_direct(state, &tc);
                                    }
                                    continue; // Skip full redraw
                                }
                                MouseEventKind::ScrollDown => {
                                    if let Some(state) = app.popup_manager.current_mut() {
                                        state.mouse_scroll_down();
                                    }
                                    // Fast direct render bypassing ratatui
                                    let tc = app.settings.theme;
                                    if let Some(state) = app.popup_manager.current() {
                                        popup::console_renderer::render_popup_content_direct(state, &tc);
                                    }
                                    continue; // Skip full redraw
                                }
                                _ => {}
                            }
                            needs_redraw = true;
                        }
                        Event::Paste(text) => {
                            for c in text.chars() {
                                if c == '\n' || c == '\r' {
                                    app.input.insert_char('\n');
                                } else if !c.is_control() {
                                    app.input.insert_char(c);
                                }
                            }
                            app.last_input_was_delete = false;
                            needs_redraw = true;
                        }
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            // Handle Ctrl+C with double-press to quit
                            if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
                                if let Some(last_time) = app.last_ctrl_c {
                                    if last_time.elapsed() < std::time::Duration::from_secs(15) {
                                        break; // Second Ctrl+C within 15 seconds - quit
                                    }
                                }
                                // First Ctrl+C or timeout - show message and record time
                                app.last_ctrl_c = Some(std::time::Instant::now());
                                let world = app.current_world_mut();
                                world.showing_splash = false; // Clear splash when adding output
                                let seq = world.next_seq;
                                world.next_seq += 1;
                                world.output_lines.push(
                                    OutputLine::new_client("Press Ctrl+C again within 15 seconds to exit, or use /quit".to_string(), seq)
                                );
                                // Keep scroll at bottom
                                world.scroll_offset = world.output_lines.len().saturating_sub(1);
                                needs_redraw = true;
                                continue;
                            }

                            // Handle remote client key events
                            if handle_remote_client_key(&mut app, key, &ws_tx) {
                                break; // Quit requested
                            }
                            // Check if a /reload was requested (re-exec local binary)
                            if app.pending_reload {
                                app.pending_reload = false;
                                #[cfg(all(unix, not(target_os = "android")))]
                                {
                                    let _ = crossterm::terminal::disable_raw_mode();
                                    let _ = crossterm::execute!(
                                        std::io::stdout(),
                                        crossterm::terminal::LeaveAlternateScreen
                                    );
                                    if let Ok((exe, _)) = get_executable_path() {
                                        use std::os::unix::process::CommandExt;
                                        let args: Vec<String> = std::env::args().skip(1).collect();
                                        let err = std::process::Command::new(&exe).args(&args).exec();
                                        // If exec failed, restore terminal and show error
                                        let _ = crossterm::terminal::enable_raw_mode();
                                        let _ = crossterm::execute!(
                                            std::io::stdout(),
                                            crossterm::terminal::EnterAlternateScreen,
                                            crossterm::event::EnableBracketedPaste
                                        );
                                        app.add_output(&format!("Reload failed: {}", err));
                                    } else {
                                        app.add_output("Reload failed: cannot find executable path");
                                    }
                                }
                                #[cfg(not(all(unix, not(target_os = "android"))))]
                                {
                                    app.add_output("Reload is not available on this platform.");
                                }
                            }
                            // Check if an /update was requested
                            if let Some(force) = app.pending_update.take() {
                                app.add_output(if force { "Force updating..." } else { "Checking for updates..." });
                                #[cfg(not(target_os = "android"))]
                                {
                                    let update_tx_clone = update_tx.clone();
                                    tokio::spawn(async move {
                                        let result = check_and_download_update(force).await;
                                        let _ = update_tx_clone.send(result).await;
                                    });
                                }
                                #[cfg(target_os = "android")]
                                {
                                    app.add_output("Update is not available on this platform.");
                                }
                            }
                            needs_redraw = true;
                        }
                        Event::Resize(width, height) => {
                            // Send updated view state for more-mode sync
                            let visible_lines = height.saturating_sub(4) as usize;
                            let _ = ws_tx.send(WsMessage::UpdateViewState {
                                world_index: app.current_world_index,
                                visible_lines,
                                visible_columns: Some(width as usize),
                            });
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
            }
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                            app.handle_remote_ws_message(ws_msg);
                            // After processing ScrollbackLines, backfill_next may be set
                            if let Some((world_idx, before_seq)) = app.backfill_next.take() {
                                let _ = ws_tx.send(WsMessage::RequestScrollback {
                                    world_index: world_idx,
                                    count: 500,
                                    before_seq,
                                });
                            }
                            // Check if an /update was requested via ExecuteLocalCommand
                            if let Some(force) = app.pending_update.take() {
                                app.add_output(if force { "Force updating..." } else { "Checking for updates..." });
                                #[cfg(not(target_os = "android"))]
                                {
                                    let update_tx_clone = update_tx.clone();
                                    tokio::spawn(async move {
                                        let result = check_and_download_update(force).await;
                                        let _ = update_tx_clone.send(result).await;
                                    });
                                }
                                #[cfg(target_os = "android")]
                                {
                                    app.add_output("Update is not available on this platform.");
                                }
                            }
                            needs_redraw = true;
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        // Respond to ping - handled by tungstenite automatically
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    _ => {}
                }
            }
            // Handle /update result
            result = update_rx.recv() => {
                if let Some(result) = result {
                    match result {
                        Ok(success) => {
                            #[cfg(all(unix, not(target_os = "android")))]
                            {
                                match get_executable_path() {
                                    Ok((exe_path, _)) => {
                                        use std::os::unix::fs::PermissionsExt;
                                        if let Err(e) = std::fs::set_permissions(
                                            &success.temp_path,
                                            std::fs::Permissions::from_mode(0o755),
                                        ) {
                                            app.add_output(&format!("Failed to set permissions: {}", e));
                                            let _ = std::fs::remove_file(&success.temp_path);
                                        } else if let Err(e) = std::fs::rename(&success.temp_path, &exe_path) {
                                            match std::fs::copy(&success.temp_path, &exe_path) {
                                                Ok(_) => {
                                                    let _ = std::fs::remove_file(&success.temp_path);
                                                    app.add_output(&format!("Updated to Clay v{} — please restart.", success.version));
                                                }
                                                Err(e2) => {
                                                    app.add_output(&format!("Failed to install update: {} (rename: {})", e2, e));
                                                    let _ = std::fs::remove_file(&success.temp_path);
                                                }
                                            }
                                        } else {
                                            app.add_output(&format!("Updated to Clay v{} — please restart.", success.version));
                                        }
                                    }
                                    Err(e) => {
                                        app.add_output(&format!("Cannot find current binary: {}", e));
                                        let _ = std::fs::remove_file(&success.temp_path);
                                    }
                                }
                            }
                            #[cfg(not(all(unix, not(target_os = "android"))))]
                            {
                                app.add_output(&format!("Update v{} downloaded to {}. Please replace the binary manually and restart.", success.version, success.temp_path.display()));
                            }
                        }
                        Err(e) => {
                            app.add_output(&e);
                        }
                    }
                    needs_redraw = true;
                }
            }
            // Backfill timer: start requesting scrollback history after initial delay
            () = &mut backfill_timer, if backfill_timer_active => {
                backfill_timer_active = false;
                // Start backfill from the first world in the queue
                app.backfill_advance_to_next();
                if let Some((world_idx, before_seq)) = app.backfill_next.take() {
                    let _ = ws_tx.send(WsMessage::RequestScrollback {
                        world_index: world_idx,
                        count: 500,
                        before_seq,
                    });
                }
            }
        }
    }

    // Cleanup
    ws_write_handle.abort();
    disable_raw_mode()?;
    let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    let _ = execute!(terminal.backend_mut(), crossterm::event::DisableBracketedPaste);
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
/// Handle key events for remote console client mode
/// Returns true if quit was requested
pub(crate) fn handle_remote_client_key(
    app: &mut App,
    key: KeyEvent,
    ws_tx: &mpsc::UnboundedSender<WsMessage>,
) -> bool {
    use KeyCode::*;

    // New unified popup system - handles help popup and others
    if app.has_new_popup() {
        match handle_new_popup_key(app, key) {
            NewPopupAction::Command(cmd) => {
                // Menu command selected - parse and execute locally
                let parsed = parse_command(&cmd);
                match parsed {
                    Command::Help => {
                        app.open_help_popup_new();
                    }
                    Command::HelpTopic { ref topic } => {
                        use popup::definitions::help::{get_topic_help, create_topic_help_popup, HELP_FIELD_CONTENT};
                        if let Some(lines) = get_topic_help(topic) {
                            app.popup_manager.open(create_topic_help_popup(lines));
                            if let Some(state) = app.popup_manager.current_mut() {
                                state.select_field(HELP_FIELD_CONTENT);
                            }
                        }
                    }
                    Command::Version => {
                        app.add_output(&get_version_string());
                    }
                    Command::Menu => {
                        app.open_menu_popup_new();
                    }
                    Command::Font => {
                        app.add_output("Font settings are available in the web and GUI interfaces.");
                    }
                    Command::Setup => {
                        app.open_setup_popup_new();
                    }
                    Command::Web => {
                        app.open_web_popup_new();
                    }
                    Command::WorldSelector => {
                        app.open_world_selector_new();
                    }
                    Command::WorldsList => {
                        let _ = ws_tx.send(WsMessage::RequestConnectionsList);
                    }
                    Command::Actions { world } => {
                        if let Some(world_name) = world {
                            app.open_actions_list_popup_with_filter(&world_name);
                        } else {
                            app.open_actions_list_popup();
                        }
                    }
                    _ => {
                        // Web-only editors: show URL hint in console
                        if cmd == "/theme-editor" || cmd == "/keybind-editor" {
                            let page = cmd.trim_start_matches('/');
                            let proto = if app.settings.web_secure { "https" } else { "http" };
                            if app.settings.http_enabled {
                                app.add_output(&format!("Open in browser: {}://localhost:{}/{}", proto, app.settings.http_port, page));
                            } else {
                                app.add_output(&format!("Enable HTTP in /web settings, then open /{} in a browser.", page));
                            }
                        } else {
                            // Other commands - send to server
                            let _ = ws_tx.send(WsMessage::SendCommand {
                                world_index: app.current_world_index,
                                command: cmd,
                            });
                        }
                    }
                }
            }
            NewPopupAction::Confirm(data) => {
                // Handle action deletion confirm
                if let Some(action_index_str) = data.get("action_index") {
                    if let Ok(action_index) = action_index_str.parse::<usize>() {
                        if action_index < app.settings.actions.len() {
                            let action_name = app.settings.actions[action_index].name.clone();
                            app.settings.actions.remove(action_index);
                            app.add_output(&format!("Action '{}' deleted.", action_name));
                            // Send UpdateActions to daemon
                            let _ = ws_tx.send(WsMessage::UpdateActions {
                                actions: app.settings.actions.clone()
                            });
                            // Reopen actions list to show updated list
                            app.open_actions_list_popup();
                        }
                    }
                } else if data.contains_key("web_save_remote") {
                    // Allow list wildcard warning confirmed — apply settings
                    let settings = web_settings_from_custom_data(&data);
                    apply_remote_web_settings(app, &settings, ws_tx);
                }
            }
            NewPopupAction::ConfirmCancelled(data) => {
                // Reopen the parent list popup when confirm dialog is cancelled
                if data.contains_key("world_index") {
                    app.open_world_selector_new();
                } else if data.contains_key("action_index") {
                    app.open_actions_list_popup();
                }
            }
            NewPopupAction::WorldSelector(_action) => {
                // World selector action - remote client doesn't handle locally
            }
            NewPopupAction::WorldSelectorFilter => {
                // Filter changed - remote client doesn't handle locally
            }
            NewPopupAction::SetupSaved(settings) => {
                // Send settings update to master daemon
                // Update local app settings first
                app.settings.more_mode_enabled = settings.more_mode;
                app.settings.spell_check_enabled = settings.spell_check;
                app.settings.temp_convert_enabled = settings.temp_convert;
                app.settings.world_switch_mode = WorldSwitchMode::from_name(&settings.world_switching);
                // Note: show_tags is not in setup anymore - controlled by F2 or /tag
                app.input_height = settings.input_height as u16;
                app.input.visible_height = app.input_height;
                app.settings.gui_theme = Theme::from_name(&settings.gui_theme);
                app.settings.tls_proxy_enabled = settings.tls_proxy;
                if app.settings.dictionary_path != settings.dictionary_path {
                    app.settings.dictionary_path = settings.dictionary_path.clone();
                    app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
                }
                app.settings.editor_side = EditorSide::from_name(&settings.editor_side);
                // Update mouse setting; if disabled, turn off capture immediately
                app.settings.mouse_enabled = settings.mouse_enabled;
                if !settings.mouse_enabled && app.mouse_capture_active {
                    let _ = execute!(std::io::stdout(), DisableMouseCapture);
                    app.mouse_capture_active = false;
                }
                app.settings.zwj_enabled = settings.zwj_enabled;
                app.settings.ansi_music_enabled = settings.ansi_music;
                app.settings.new_line_indicator = settings.new_line_indicator;
                app.settings.tts_mode = crate::tts::TtsMode::from_name(&settings.tts_mode);

                // Send UpdateGlobalSettings to daemon
                let _ = ws_tx.send(WsMessage::UpdateGlobalSettings {
                    more_mode_enabled: app.settings.more_mode_enabled,
                    spell_check_enabled: app.settings.spell_check_enabled,
                    temp_convert_enabled: app.settings.temp_convert_enabled,
                    world_switch_mode: app.settings.world_switch_mode.name().to_string(),
                    show_tags: app.show_tags,
                    debug_enabled: app.settings.debug_enabled,
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
                    web_font_weight: app.settings.web_font_weight,
                    web_font_line_height: app.settings.web_font_line_height,
                    web_font_letter_spacing: app.settings.web_font_letter_spacing,
                    web_font_word_spacing: app.settings.web_font_word_spacing,
                    ws_allow_list: app.settings.websocket_allow_list.clone(),
                    web_secure: app.settings.web_secure,
                    http_enabled: app.settings.http_enabled,
                    http_port: app.settings.http_port,
                    ws_enabled: false,  // Legacy
                    ws_port: 0,        // Legacy
                    ws_cert_file: app.settings.websocket_cert_file.clone(),
                    ws_key_file: app.settings.websocket_key_file.clone(),
                    ws_password: String::new(),  // Never send existing password to remote clients
                    tls_proxy_enabled: app.settings.tls_proxy_enabled,
                    dictionary_path: app.settings.dictionary_path.clone(),
                    mouse_enabled: app.settings.mouse_enabled,
                    zwj_enabled: app.settings.zwj_enabled,
                    new_line_indicator: app.settings.new_line_indicator,
                    tts_mode: app.settings.tts_mode.name().to_string(),
                    tts_speak_mode: app.settings.tts_speak_mode.name().to_string(),
                });
            }
            NewPopupAction::WebSaved(settings) => {
                // Check for wildcard '*' in allow list — warn user
                if crate::websocket::allow_list_has_wildcard(&settings.ws_allow_list) {
                    use popup::definitions::confirm::create_allow_list_warning_dialog;
                    let mut def = create_allow_list_warning_dialog();
                    def.custom_data.insert("web_save_remote".to_string(), "1".to_string());
                    def.custom_data.insert("web_secure".to_string(), settings.web_secure.to_string());
                    def.custom_data.insert("http_enabled".to_string(), settings.http_enabled.to_string());
                    def.custom_data.insert("http_port".to_string(), settings.http_port);
                    def.custom_data.insert("ws_password".to_string(), settings.ws_password);
                    def.custom_data.insert("ws_allow_list".to_string(), settings.ws_allow_list);
                    def.custom_data.insert("ws_cert_file".to_string(), settings.ws_cert_file);
                    def.custom_data.insert("ws_key_file".to_string(), settings.ws_key_file);
                    app.popup_manager.open(def);
                } else {
                    apply_remote_web_settings(app, &settings, ws_tx);
                }
            }
            NewPopupAction::ConnectionsClose => {
                // Connections popup closed - no action needed
            }
            NewPopupAction::ActionsList(action) => {
                match action {
                    ActionsListAction::Add => {
                        app.open_action_editor_popup(None);
                    }
                    ActionsListAction::Edit(idx) => {
                        if idx < app.settings.actions.len() {
                            app.open_action_editor_popup(Some(idx));
                        }
                    }
                    ActionsListAction::Delete(idx) => {
                        if idx < app.settings.actions.len() {
                            let name = app.settings.actions[idx].name.clone();
                            app.open_delete_action_confirm(&name, idx);
                        }
                    }
                    ActionsListAction::Toggle(idx) => {
                        if idx < app.settings.actions.len() {
                            app.settings.actions[idx].enabled = !app.settings.actions[idx].enabled;
                            app.settings.actions[idx].compile_regex();
                            // Send UpdateActions to daemon
                            let _ = ws_tx.send(WsMessage::UpdateActions {
                                actions: app.settings.actions.clone()
                            });
                            // Update the list display in the popup
                            use popup::definitions::actions::{ActionInfo, filter_actions, ACTIONS_FIELD_FILTER, ACTIONS_FIELD_LIST};
                            if let Some(state) = app.popup_manager.current_mut() {
                                let filter_text = if state.editing && state.is_field_selected(ACTIONS_FIELD_FILTER) {
                                    state.edit_buffer.clone()
                                } else {
                                    state.get_text(ACTIONS_FIELD_FILTER).unwrap_or("").to_string()
                                };
                                let all_actions: Vec<ActionInfo> = app.settings.actions
                                    .iter()
                                    .enumerate()
                                    .map(|(i, a)| ActionInfo {
                                        name: a.name.clone(),
                                        world: a.world.clone(),
                                        pattern: a.pattern.clone(),
                                        enabled: a.enabled,
                                        index: i,
                                    })
                                    .collect();
                                let mut filtered = filter_actions(&all_actions, &filter_text);
                                filtered.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                if let Some(field) = state.field_mut(ACTIONS_FIELD_LIST) {
                                    if let popup::FieldKind::List { items, .. } = &mut field.kind {
                                        *items = filtered.iter().map(|info| {
                                                #[cfg(not(windows))]
                                                let status = if info.enabled { "[✓]" } else { "[ ]" };
                                                #[cfg(windows)]
                                                let status = if info.enabled { "[x]" } else { "[ ]" };
                                                let world_part = if info.world.is_empty() {
                                                    String::new()
                                                } else {
                                                    format!("({})", info.world)
                                                };
                                                let pattern_preview = if info.pattern.len() > 30 {
                                                    format!("{}...", &info.pattern[..27])
                                                } else {
                                                    info.pattern.clone()
                                                };
                                                popup::ListItem {
                                                    id: info.index.to_string(),
                                                    columns: vec![
                                                        format!("{} {}", status, info.name),
                                                        world_part,
                                                        pattern_preview,
                                                    ],
                                                    style: popup::ListItemStyle {
                                                        is_disabled: !info.enabled,
                                                        ..Default::default()
                                                    },
                                                }
                                        }).collect();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            NewPopupAction::ActionsListFilter => {
                // Filter changed - update the actions list locally
                use popup::definitions::actions::{ActionInfo, filter_actions, ACTIONS_FIELD_FILTER, ACTIONS_FIELD_LIST};
                if let Some(state) = app.popup_manager.current_mut() {
                    let filter_text = if state.editing && state.is_field_selected(ACTIONS_FIELD_FILTER) {
                        state.edit_buffer.clone()
                    } else {
                        state.get_text(ACTIONS_FIELD_FILTER).unwrap_or("").to_string()
                    };
                    let all_actions: Vec<ActionInfo> = app.settings.actions
                        .iter()
                        .enumerate()
                        .map(|(i, a)| ActionInfo {
                            name: a.name.clone(),
                            world: a.world.clone(),
                            pattern: a.pattern.clone(),
                            enabled: a.enabled,
                            index: i,
                        })
                        .collect();
                    let mut filtered = filter_actions(&all_actions, &filter_text);
                    filtered.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    if let Some(field) = state.field_mut(ACTIONS_FIELD_LIST) {
                        if let popup::FieldKind::List { items, selected_index, scroll_offset, .. } = &mut field.kind {
                            let old_len = items.len();
                            *items = filtered.iter().map(|info| {
                                    #[cfg(not(windows))]
                                    let status = if info.enabled { "[✓]" } else { "[ ]" };
                                    #[cfg(windows)]
                                    let status = if info.enabled { "[x]" } else { "[ ]" };
                                    let world_part = if info.world.is_empty() {
                                        String::new()
                                    } else {
                                        format!("({})", info.world)
                                    };
                                    let pattern_preview = if info.pattern.len() > 30 {
                                        format!("{}...", &info.pattern[..27])
                                    } else {
                                        info.pattern.clone()
                                    };
                                    popup::ListItem {
                                        id: info.index.to_string(),
                                        columns: vec![
                                            format!("{} {}", status, info.name),
                                            world_part,
                                            pattern_preview,
                                        ],
                                        style: popup::ListItemStyle {
                                            is_disabled: !info.enabled,
                                            ..Default::default()
                                        },
                                    }
                            }).collect();
                            if items.is_empty() {
                                *selected_index = 0;
                                *scroll_offset = 0;
                            } else if *selected_index >= items.len() {
                                *selected_index = items.len().saturating_sub(1);
                            }
                            if old_len != items.len() {
                                *scroll_offset = 0;
                            }
                        }
                    }
                }
            }
            NewPopupAction::ActionEditorSave { action, editing_index } => {
                // Update local actions list
                if let Some(idx) = editing_index {
                    if idx < app.settings.actions.len() {
                        app.settings.actions[idx] = action;
                        app.settings.actions[idx].compile_regex();
                    }
                } else {
                    app.settings.actions.push(action);
                    app.settings.actions.last_mut().unwrap().compile_regex();
                }
                // Send UpdateActions to daemon
                let _ = ws_tx.send(WsMessage::UpdateActions {
                    actions: app.settings.actions.clone()
                });
            }
            NewPopupAction::ActionEditorDelete { editing_index } => {
                if editing_index < app.settings.actions.len() {
                    let name = app.settings.actions[editing_index].name.clone();
                    app.open_delete_action_confirm(&name, editing_index);
                }
            }
            NewPopupAction::WorldEditorSaved(settings) => {
                // Update local world settings
                let idx = settings.world_index;
                if idx < app.worlds.len() {
                    app.worlds[idx].name = settings.name.clone();
                    app.worlds[idx].settings.hostname = settings.hostname.clone();
                    app.worlds[idx].settings.port = settings.port.clone();
                    app.worlds[idx].settings.user = settings.user.clone();
                    app.worlds[idx].settings.password = settings.password.clone();
                    app.worlds[idx].settings.use_ssl = settings.use_ssl;
                    app.worlds[idx].settings.log_enabled = settings.log_enabled;
                    app.worlds[idx].settings.encoding = Encoding::from_name(&settings.encoding);
                    app.worlds[idx].settings.auto_connect_type = AutoConnectType::from_name(&settings.auto_connect);
                    app.worlds[idx].settings.keep_alive_type = KeepAliveType::from_name(&settings.keep_alive);
                    app.worlds[idx].settings.keep_alive_cmd = settings.keep_alive_cmd.clone();
                    app.worlds[idx].settings.gmcp_packages = settings.gmcp_packages.clone();
                    app.worlds[idx].settings.auto_reconnect_secs = settings.auto_reconnect_secs;

                    // Send UpdateWorldSettings to daemon
                    let _ = ws_tx.send(WsMessage::UpdateWorldSettings {
                        world_index: idx,
                        name: settings.name,
                        hostname: settings.hostname,
                        port: settings.port,
                        user: settings.user,
                        password: settings.password,
                        use_ssl: settings.use_ssl,
                        log_enabled: settings.log_enabled,
                        encoding: settings.encoding,
                        auto_login: settings.auto_connect,
                        keep_alive_type: settings.keep_alive,
                        keep_alive_cmd: settings.keep_alive_cmd,
                        gmcp_packages: settings.gmcp_packages,
                        auto_reconnect_secs: settings.auto_reconnect_secs,
                    });
                }
            }
            NewPopupAction::WorldEditorDelete(idx) => {
                // Send delete request to daemon
                let _ = ws_tx.send(WsMessage::DeleteWorld { world_index: idx });
            }
            NewPopupAction::WorldEditorConnect(idx) => {
                // Send connect request to daemon
                let _ = ws_tx.send(WsMessage::ConnectWorld { world_index: idx });
            }
            NewPopupAction::NotesList(_action) => {
                // Notes list not used in remote client
            }
            NewPopupAction::None => {}
        }
        return false;
    }
    if app.filter_popup.visible {
        handle_remote_filter_popup_key(app, key);
        return false;
    }

    // Ctrl+V literal next: insert next character literally
    if app.literal_next {
        app.literal_next = false;
        if let KeyCode::Char(c) = key.code {
            app.input.insert_char(c);
        }
        return false;
    }

    // Helper to check if escape was pressed recently (for Escape+key sequences)
    let recent_escape = app.last_escape
        .map(|t| t.elapsed() < std::time::Duration::from_millis(500))
        .unwrap_or(false);

    // Track bare Escape key presses for Escape+key sequences
    if key.code == Esc && key.modifiers.is_empty() {
        app.last_escape = Some(std::time::Instant::now());
        return false;
    }

    // Convert key event to canonical name, handling Esc+key sequences
    let key_name = if recent_escape && matches!(key.code, Char(_) | Backspace) {
        app.last_escape = None;
        keybindings::escape_key_to_name(key.code, key.modifiers)
    } else {
        keybindings::key_event_to_name(key.code, key.modifiers)
    };

    // Clear history search state on any non-search key
    if let Some(ref name) = key_name {
        if name != "Esc-p" && name != "Esc-n" && name != "Escape" {
            app.input.search_prefix = None;
            app.input.search_index = None;
        }
    }

    // Check TF /bind bindings first (runtime bindings from /bind command)
    if let Some(ref name) = key_name {
        let tf_name = canonical_to_tf_key_name(name);
        if let Some(cmd) = app.tf_engine.keybindings.get(&tf_name).cloned() {
            let _ = ws_tx.send(WsMessage::SendCommand {
                world_index: app.current_world_index,
                command: cmd,
            });
            return false;
        }
    }

    // Check configurable action bindings
    if let Some(ref name) = key_name {
        if let Some(action_id) = app.keybindings.get_action(name).map(|s| s.to_string()) {
            return dispatch_remote_action(&action_id, app, ws_tx);
        }
    }

    // Enter key (always active, not bound via action system)
    if key.code == Enter {
            let cmd = app.input.take_input();
            if cmd.is_empty() {
                // Send empty command to server (some MUDs use this for "look")
                let _ = ws_tx.send(WsMessage::SendCommand {
                    world_index: app.current_world_index,
                    command: cmd,
                });
            } else {
                // Parse command to handle local commands
                let parsed = parse_command(&cmd);
                match parsed {
                    Command::Quit => return true,
                    Command::Help => {
                        app.open_help_popup_new();
                    }
                    Command::HelpTopic { ref topic } => {
                        use popup::definitions::help::{get_topic_help, create_topic_help_popup, HELP_FIELD_CONTENT};
                        if let Some(lines) = get_topic_help(topic) {
                            app.popup_manager.open(create_topic_help_popup(lines));
                            if let Some(state) = app.popup_manager.current_mut() {
                                state.select_field(HELP_FIELD_CONTENT);
                            }
                        }
                    }
                    Command::Version => {
                        app.add_output(&get_version_string());
                    }
                    Command::Menu => {
                        app.open_menu_popup_new();
                    }
                    Command::Font => {
                        app.add_output("Font settings are available in the web and GUI interfaces.");
                    }
                    Command::Setup => {
                        app.open_setup_popup_new();
                    }
                    Command::Web => {
                        app.open_web_popup_new();
                    }
                    Command::WorldSelector => {
                        app.open_world_selector_new();
                    }
                    Command::WorldsList => {
                        // Request connections list from server (includes timing info)
                        let _ = ws_tx.send(WsMessage::RequestConnectionsList);
                    }
                    Command::WorldSwitch { ref name } => {
                        // /worlds <name> - switch to world if it exists
                        if let Some(idx) = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                            app.current_world_index = idx;
                            let _ = ws_tx.send(WsMessage::MarkWorldSeen { world_index: idx });
                            // If not connected, request connection
                            if !app.worlds[idx].connected {
                                let _ = ws_tx.send(WsMessage::ConnectWorld { world_index: idx });
                            }
                        } else {
                            // World doesn't exist - could create it, but for now just show message
                            let ci = app.current_world_index;
                            let seq = app.worlds[ci].next_seq;
                            app.worlds[ci].next_seq += 1;
                            app.worlds[ci].output_lines.push(
                                OutputLine::new_client(format!("World '{}' not found.", name), seq)
                            );
                        }
                    }
                    Command::WorldConnectNoLogin { ref name } => {
                        // /worlds -l <name> - send to server as SendCommand so skip_auto_login is set
                        if let Some(idx) = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                            app.current_world_index = idx;
                            let _ = ws_tx.send(WsMessage::MarkWorldSeen { world_index: idx });
                            if !app.worlds[idx].connected {
                                // Send as command so server parses -l flag and sets skip_auto_login
                                let _ = ws_tx.send(WsMessage::SendCommand {
                                    world_index: idx,
                                    command: format!("/worlds -l {}", name),
                                });
                            }
                        } else {
                            let ci = app.current_world_index;
                            let seq = app.worlds[ci].next_seq;
                            app.worlds[ci].next_seq += 1;
                            app.worlds[ci].output_lines.push(
                                OutputLine::new_client(format!("World '{}' not found.", name), seq)
                            );
                        }
                    }
                    Command::WorldEdit { ref name } => {
                        // /worlds -e [name] - open world editor using new popup
                        let idx = if let Some(ref n) = name {
                            app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(n))
                                .unwrap_or(app.current_world_index)
                        } else {
                            app.current_world_index
                        };
                        if idx < app.worlds.len() {
                            app.open_world_editor_popup_new(idx);
                        }
                    }
                    Command::Actions { world } => {
                        if let Some(world_name) = world {
                            app.open_actions_list_popup_with_filter(&world_name);
                        } else {
                            app.open_actions_list_popup();
                        }
                    }
                    Command::Update { force } => {
                        // Signal to the outer run_console_client loop to handle locally
                        app.pending_update = Some(force);
                    }
                    Command::Reload => {
                        // Reload the local remote client binary
                        app.pending_reload = true;
                    }
                    Command::Connect { .. } => {
                        let _ = ws_tx.send(WsMessage::ConnectWorld {
                            world_index: app.current_world_index,
                        });
                    }
                    Command::Disconnect => {
                        let _ = ws_tx.send(WsMessage::DisconnectWorld {
                            world_index: app.current_world_index,
                        });
                    }
                    Command::NotACommand { text } => {
                        // Regular text - send to server
                        let _ = ws_tx.send(WsMessage::SendCommand {
                            world_index: app.current_world_index,
                            command: text,
                        });
                    }
                    _ => {
                        // Other commands - send to server for processing
                        let _ = ws_tx.send(WsMessage::SendCommand {
                            world_index: app.current_world_index,
                            command: cmd,
                        });
                    }
                }
            }
            // Reset lines_since_pause for more-mode
            app.current_world_mut().lines_since_pause = 0;
            // Clear splash on first user input (same as server data)
            if app.current_world().showing_splash {
                let world = app.current_world_mut();
                world.showing_splash = false;
                world.needs_redraw = true;
                world.output_lines.clear();
                world.first_marked_new_index = None;
                world.scroll_offset = 0;
                app.needs_output_redraw = true;
            }
        }
    // Fall through to character input (unbound keys)
    if let Char(c) = key.code {
        if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            app.input.insert_char(c);
            app.last_input_was_delete = false;
            app.check_temp_conversion();
        }
    }
    false
}
/// Dispatch a keybinding action for remote console client.
/// Similar to dispatch_action() but sends WsMessages instead of direct App mutations for world ops.
/// Returns true if quit was requested.
pub(crate) fn dispatch_remote_action(
    action: &str,
    app: &mut App,
    ws_tx: &mpsc::UnboundedSender<WsMessage>,
) -> bool {
    match action {
        // Cursor Movement
        "cursor_left" => { app.input.move_cursor_left(); }
        "cursor_right" => { app.input.move_cursor_right(); }
        "cursor_word_left" => { app.input.word_left(); }
        "cursor_word_right" => { app.input.word_right(); }
        "cursor_home" => { app.input.home(); }
        "cursor_end" => { app.input.end(); }
        "cursor_up" => {
            if app.input.move_cursor_up() {
                app.input.history_prev();
            }
        }
        "cursor_down" => {
            if app.input.move_cursor_down() {
                app.input.history_next();
            }
        }

        // Editing
        "delete_backward" => { app.input.delete_char(); app.last_input_was_delete = true; }
        "delete_forward" => { app.input.delete_char_forward(); app.last_input_was_delete = true; }
        "delete_word_backward" => { app.input.delete_word_before_cursor(); }
        "delete_word_forward" => { app.input.delete_word_forward(); }
        "delete_word_backward_punct" => { app.input.backward_kill_word_punctuation(); }
        "kill_to_end" => { app.input.kill_to_end(); }
        "clear_line" => { app.input.clear(); }
        "transpose_chars" => { app.input.transpose_chars(); }
        "literal_next" => { app.literal_next = true; }
        "capitalize_word" => { app.input.capitalize_word(); }
        "lowercase_word" => { app.input.lowercase_word(); }
        "uppercase_word" => { app.input.uppercase_word(); }
        "collapse_spaces" => { app.input.collapse_spaces(); }
        "goto_matching_bracket" => { app.input.goto_matching_bracket(); }
        "insert_last_arg" => { app.input.last_argument(); }
        "yank" => { app.input.yank(); }

        // History
        "history_prev" => { app.input.history_prev(); }
        "history_next" => { app.input.history_next(); }
        "history_search_backward" => { app.input.history_search_backward(); }
        "history_search_forward" => { app.input.history_search_forward(); }

        // Scrollback
        "scroll_page_up" => {
            let scroll_amount = app.output_height.saturating_sub(2) as usize;
            let current_offset = app.current_world().scroll_offset;
            let new_offset = current_offset.saturating_sub(scroll_amount.max(1));
            app.current_world_mut().scroll_offset = new_offset;
            if new_offset == 0 {
                let before_seq = app.current_world().output_lines.first().map(|l| l.seq);
                let _ = ws_tx.send(WsMessage::RequestScrollback {
                    world_index: app.current_world_index,
                    count: scroll_amount.max(1),
                    before_seq,
                });
            }
            app.needs_output_redraw = true;
        }
        "scroll_page_down" => {
            let scroll_amount = app.output_height.saturating_sub(2) as usize;
            let max_offset = app.current_world().output_lines.len().saturating_sub(1);
            app.current_world_mut().scroll_offset = (app.current_world().scroll_offset + scroll_amount.max(1)).min(max_offset);
            app.needs_output_redraw = true;
        }
        "scroll_half_page" => {
            let half = (app.output_height as usize).saturating_sub(2) / 2;
            let has_pending = !app.current_world().pending_lines.is_empty() || app.current_world().pending_count > 0;
            if app.current_world().paused && has_pending {
                let _ = ws_tx.send(WsMessage::ReleasePending {
                    world_index: app.current_world_index,
                    count: half.max(1),
                });
            } else {
                let scroll_amount = half.max(1);
                let current_offset = app.current_world().scroll_offset;
                let new_offset = current_offset.saturating_sub(scroll_amount);
                app.current_world_mut().scroll_offset = new_offset;
                app.needs_output_redraw = true;
            }
        }
        "flush_output" => {
            let has_pending = !app.current_world().pending_lines.is_empty() || app.current_world().pending_count > 0;
            if app.current_world().paused && has_pending {
                let _ = ws_tx.send(WsMessage::ReleasePending {
                    world_index: app.current_world_index,
                    count: 0,
                });
            }
            app.current_world_mut().scroll_to_bottom();
            app.needs_output_redraw = true;
        }
        "selective_flush" => {
            let _ = ws_tx.send(WsMessage::SelectiveFlush {
                world_index: app.current_world_index,
            });
        }
        "tab_key" => {
            let has_pending = !app.current_world().pending_lines.is_empty() || app.current_world().pending_count > 0;
            if app.current_world().paused && has_pending {
                let release_count = app.output_height.saturating_sub(2) as usize;
                let _ = ws_tx.send(WsMessage::ReleasePending {
                    world_index: app.current_world_index,
                    count: release_count,
                });
            } else if !app.current_world().is_at_bottom() {
                let scroll_amount = app.output_height.saturating_sub(2) as usize;
                let max_offset = app.current_world().output_lines.len().saturating_sub(1);
                app.current_world_mut().scroll_offset = (app.current_world().scroll_offset + scroll_amount.max(1)).min(max_offset);
                app.needs_output_redraw = true;
            }
        }

        // World (remote: send WS messages to server for world calculations)
        "world_next" => {
            let _ = ws_tx.send(WsMessage::CalculateNextWorld { current_index: app.current_world_index });
        }
        "world_prev" => {
            let _ = ws_tx.send(WsMessage::CalculatePrevWorld { current_index: app.current_world_index });
        }
        "world_all_next" => {
            let _ = ws_tx.send(WsMessage::CalculateNextWorld { current_index: app.current_world_index });
        }
        "world_all_prev" => {
            let _ = ws_tx.send(WsMessage::CalculatePrevWorld { current_index: app.current_world_index });
        }
        "world_activity" => {
            let _ = ws_tx.send(WsMessage::CalculateOldestPending { current_index: app.current_world_index });
        }
        "world_previous" => {
            let _ = ws_tx.send(WsMessage::CalculatePrevWorld { current_index: app.current_world_index });
        }
        "world_forward" => {
            let _ = ws_tx.send(WsMessage::CalculateNextWorld { current_index: app.current_world_index });
        }

        // System
        "help" => {
            if app.has_new_popup() {
                app.close_new_popup();
            } else {
                app.open_help_popup_new();
            }
        }
        "redraw" => {
            app.current_world_mut().filter_to_server_output();
            app.needs_terminal_clear = true;
            app.needs_output_redraw = true;
        }
        "reload" => {
            app.pending_reload = true;
        }
        "quit" => {
            if let Some(last_time) = app.last_ctrl_c {
                if last_time.elapsed() < std::time::Duration::from_secs(15) {
                    return true;
                }
            }
            app.last_ctrl_c = Some(std::time::Instant::now());
            app.add_output("Press again within 15 seconds to quit.");
        }
        "suspend" => { /* Not supported in remote console */ }
        "bell" => { print!("\x07"); }
        "spell_check" => {
            // Spell check not easily supported in remote mode
        }

        // Clay Extensions
        "toggle_tags" => {
            app.show_tags = !app.show_tags;
            app.current_world_mut().visual_line_offset = 0;
            app.needs_output_redraw = true;
        }
        "filter_popup" => {
            if app.filter_popup.visible {
                app.filter_popup.visible = false;
            } else {
                app.filter_popup.open();
            }
        }
        "toggle_action_highlight" => {
            app.highlight_actions = !app.highlight_actions;
            app.needs_output_redraw = true;
        }
        "toggle_gmcp_media" => {
            let idx = app.current_world_index;
            app.worlds[idx].gmcp_user_enabled = !app.worlds[idx].gmcp_user_enabled;
            app.ws_broadcast(WsMessage::GmcpUserToggled {
                world_index: idx,
                enabled: app.worlds[idx].gmcp_user_enabled,
            });
            if app.worlds[idx].gmcp_user_enabled {
                app.restart_world_media(idx);
            } else {
                app.stop_world_media(idx);
            }
            app.needs_output_redraw = true;
        }
        "input_grow" => {
            if app.input_height < 15 { app.input_height += 1; }
        }
        "input_shrink" => {
            if app.input_height > 1 { app.input_height -= 1; }
        }
        _ => {}
    }
    false
}
/// Apply web settings from a remote console client (sends UpdateGlobalSettings to daemon)
pub(crate) fn apply_remote_web_settings(
    app: &mut App,
    settings: &WebSettings,
    ws_tx: &tokio::sync::mpsc::UnboundedSender<crate::websocket::WsMessage>,
) {
    app.settings.web_secure = settings.web_secure;
    app.settings.http_enabled = settings.http_enabled;
    app.settings.http_port = settings.http_port.parse().unwrap_or(9000);
    app.settings.websocket_allow_list = settings.ws_allow_list.clone();
    app.settings.websocket_cert_file = settings.ws_cert_file.clone();
    app.settings.websocket_key_file = settings.ws_key_file.clone();

    let _ = ws_tx.send(crate::websocket::WsMessage::UpdateGlobalSettings {
        more_mode_enabled: app.settings.more_mode_enabled,
        spell_check_enabled: app.settings.spell_check_enabled,
        temp_convert_enabled: app.settings.temp_convert_enabled,
        world_switch_mode: app.settings.world_switch_mode.name().to_string(),
        show_tags: app.show_tags,
        debug_enabled: app.settings.debug_enabled,
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
        web_font_weight: app.settings.web_font_weight,
        web_font_line_height: app.settings.web_font_line_height,
        web_font_letter_spacing: app.settings.web_font_letter_spacing,
        web_font_word_spacing: app.settings.web_font_word_spacing,
        ws_allow_list: app.settings.websocket_allow_list.clone(),
        web_secure: app.settings.web_secure,
        http_enabled: app.settings.http_enabled,
        http_port: app.settings.http_port,
        ws_enabled: false,  // Legacy
        ws_port: 0,         // Legacy
        ws_cert_file: app.settings.websocket_cert_file.clone(),
        ws_key_file: app.settings.websocket_key_file.clone(),
        ws_password: String::new(),  // Never send existing password to remote clients
        tls_proxy_enabled: app.settings.tls_proxy_enabled,
        dictionary_path: app.settings.dictionary_path.clone(),
        mouse_enabled: app.settings.mouse_enabled,
        zwj_enabled: app.settings.zwj_enabled,
        new_line_indicator: app.settings.new_line_indicator,
        tts_mode: app.settings.tts_mode.name().to_string(),
        tts_speak_mode: app.settings.tts_speak_mode.name().to_string(),
    });
}
pub(crate) fn handle_remote_filter_popup_key(app: &mut App, key: KeyEvent) {
    use KeyCode::*;

    match key.code {
        Esc | KeyCode::F(4) => {
            app.filter_popup.close();
            app.needs_output_redraw = true;
        }
        Backspace => {
            if app.filter_popup.cursor > 0 {
                app.filter_popup.cursor -= 1;
                app.filter_popup.filter_text.remove(app.filter_popup.cursor);
                app.needs_output_redraw = true;
            }
        }
        Delete => {
            if app.filter_popup.cursor < app.filter_popup.filter_text.len() {
                app.filter_popup.filter_text.remove(app.filter_popup.cursor);
                app.needs_output_redraw = true;
            }
        }
        Left => {
            if app.filter_popup.cursor > 0 {
                app.filter_popup.cursor -= 1;
            }
        }
        Right => {
            if app.filter_popup.cursor < app.filter_popup.filter_text.len() {
                app.filter_popup.cursor += 1;
            }
        }
        Home => {
            app.filter_popup.cursor = 0;
        }
        End => {
            app.filter_popup.cursor = app.filter_popup.filter_text.len();
        }
        Char(c) => {
            app.filter_popup.filter_text.insert(app.filter_popup.cursor, c);
            app.filter_popup.cursor += 1;
            app.needs_output_redraw = true;
        }
        _ => {}
    }
}
