//! Remote client support: console client mode and grep client mode.
//! These functions handle connecting to a Clay daemon via WebSocket and
//! providing a TUI interface or command-line grep for remote sessions.
#![allow(unused_imports, unused_variables)]

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
    ActionsListAction, RecentWorldsAction,
    EditorSide, TabsMode, IconBarMode, AutoConnectType, KeepAliveType,
    web_settings_from_custom_data, handle_new_popup_key,
    websocket, popup, keybindings, tf, platform, persistence,
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
    let remote = crate::util::RemoteAddr::parse(addr);
    let (ws_url, try_fallback) = match remote.scheme {
        Some(scheme) => (remote.url(scheme), false),
        None => (remote.url("wss"), true),
    };
    // Plaintext ws:// only to this machine; see util::is_loopback_host.
    let plaintext_ok = crate::util::is_loopback_host(crate::util::host_of_addr(&remote.host_port));
    if ws_url.starts_with("ws://") && !plaintext_ok {
        eprintln!("Refusing unencrypted ws:// to a remote host (the login would be sent in the clear). Use wss:// or --ssh.");
        std::process::exit(1);
    }
    let try_fallback = try_fallback && plaintext_ok;

    let host_port_key = remote.host_port.clone();

    #[cfg(feature = "rustls-backend")]
    let connect_result = if ws_url.starts_with("wss://") {
        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(platform::danger_rustls::TofuVerifier::new(host_port_key.clone())))
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
            let fallback_url = remote.url("ws");
            match connect_async(&fallback_url).await {
                Ok(result) => result,
                Err(e2) => {
                    eprintln!("Failed to connect to {}: {}{}", fallback_url, e2, remote.path_hint(&e2));
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            #[cfg(feature = "rustls-backend")]
            if let Some(mismatch) = platform::danger::take_cert_mismatch() {
                eprintln!(
                    "TLS certificate for {} has changed! Was {}, now {}.",
                    mismatch.host, mismatch.old_fingerprint, mismatch.new_fingerprint
                );
                eprintln!("Refusing to connect (possible MITM, or the server was reinstalled/re-keyed).");
                eprintln!("If you trust the new certificate, remove its old pin from ~/.clay/known_hosts.dat and reconnect.");
                std::process::exit(1);
            }
            eprintln!("Failed to connect to {}: {}{}", ws_url, e, remote.path_hint(&e));
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
    // `resume` is intentionally empty: this process holds no prior per-world seq state at
    // this point (it's a one-shot invocation, not a reconnect - there is no persisted
    // client-side state to resume from). See PROTOCOL-ROADMAP.md Step 6.
    let _ = ws_tx.send(WsMessage::AuthRequest {
        password_hash: challenge_hash,
        username: None,
        current_world: None,
        auth_key: None,
        request_key: false,
        challenge_response: true,
        resume: Vec::new(),
        resume_epochs: Vec::new(),
        client_version: crate::VERSION.to_string(),
        // One-shot invocation: never renders output, so it owns no ▶ markers and needs no
        // stable identity.
        client_uid: String::new(),
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
                                    // `acked` stays empty: --grep --follow never buffers or
                                    // tracks per-world seq state (it prints matches as an
                                    // ephemeral stream and keeps nothing to resume), so there is
                                    // no contiguous-seq boundary to report. See PROTOCOL-ROADMAP.md
                                    // Step 6.
                                    let _ = ws_tx.send(WsMessage::PongCheck { nonce, acked: Vec::new() });
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

        // Collect all lines per world: scrollback + initial state lines
        let mut world_lines: std::collections::HashMap<usize, Vec<TimestampedLine>> = std::collections::HashMap::new();
        let mut pending_worlds: std::collections::HashSet<usize> = search_worlds.iter().map(|&(idx, _)| idx).collect();

        // Pre-populate with initial state lines (InitialState already gave us the most
        // recent `remote_initial_lines` lines per world, so scrollback below only needs
        // to fetch older history than that).
        for w in &worlds {
            if matches_world(w.index) {
                world_lines.insert(w.index, w.output_lines_ts.clone());
            }
        }

        // Request scrollback for each world, anchored just before whatever InitialState
        // already gave us. Anchoring here (rather than `before_seq: None`, which would ask
        // the server for its own most-recent lines - the same range InitialState already
        // covered) means this and every follow-up request below only ever fetch strictly
        // older, non-overlapping ranges. Combined with the guaranteed in-order, unreordered
        // delivery of a direct tokio_tungstenite connection (PROTOCOL-ROADMAP.md Step 6),
        // that removes the need to sort+dedup by seq afterward: nothing overlapping or
        // out-of-order is ever received in the first place.
        for &(world_index, _) in &search_worlds {
            let before_seq = world_lines.get(&world_index)
                .and_then(|lines| lines.iter().map(|l| l.seq).min())
                .filter(|&s| s > 0);
            // request_id: None - this client (history-search mode) has no correlator state
            // to track; it drains ScrollbackLines replies purely by pending_worlds/
            // backfill_complete and never has more than one outstanding request per world at
            // a time, so there's nothing for a request_id to disambiguate here.
            let _ = ws_tx.send(WsMessage::RequestScrollback {
                world_index,
                count: 10000,
                before_seq,
                after_seq: None,
                request_id: None,
            });
        }

        // Receive scrollback responses
        while !pending_worlds.is_empty() {
            match ws_read.next().await {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<WsMessage>(&text) {
                        Ok(WsMessage::ScrollbackLines { world_index, lines, backfill_complete, .. }) => {
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
                                        after_seq: None,
                                        request_id: None,
                                    });
                                }
                            }
                        }
                        Ok(WsMessage::Ping) => {
                            let _ = ws_tx.send(WsMessage::Pong);
                        }
                        Ok(WsMessage::PingCheck { nonce }) => {
                            // `acked` stays empty: this is the one-shot history-search fetch
                            // loop (connect, backfill everything, print, exit) - by the time
                            // any PingCheck could arrive we're either still actively backfilling
                            // (no stable "caught up to N" boundary worth reporting) or about to
                            // disconnect, and there's no reconnect path here to benefit from a
                            // resume anchor. See PROTOCOL-ROADMAP.md Step 6.
                            let _ = ws_tx.send(WsMessage::PongCheck { nonce, acked: Vec::new() });
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

        // No per-world sort+dedup needed here (PROTOCOL-ROADMAP.md Step 6): each world's
        // `lines` is the InitialState seed followed by zero or more strictly-older,
        // non-overlapping before_seq batches (see the request loop above), each already
        // seq-ascending as sent by the server and prepended in oldest-first request order -
        // so the accumulated Vec is already fully sorted with no duplicates.

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

/// Why a `run_import_client` attempt failed — lets the caller (the server-side `/import`
/// driver) turn this into the right `WsMessage` reply instead of just a string.
pub enum ImportClientError {
    /// The target didn't accept a TLS connection and `allow_insecure` was false. The caller
    /// should ask the user to confirm sending the password/auth-key in cleartext, then retry
    /// with `allow_insecure: true`. Only ever returned for a loopback target: a remote one
    /// that refuses TLS is a `ConnectFailed`, never an offer to go plaintext.
    NeedsInsecureConfirm,
    /// Connected, but the target rejected the password/auth-key.
    AuthFailed(String),
    /// Couldn't reach or complete a handshake with the target at all (DNS, refused, timeout,
    /// protocol error, or — treated as fatal rather than falling back to plaintext — a
    /// TLS certificate that no longer matches a previously trusted pin).
    ConnectFailed(String),
}

/// Timeout for each network step of an import attempt (connect, or a single expected-message
/// wait). An import runs as a background task inside a live server, so unlike the CLI client
/// modes above it must never hang indefinitely on an unresponsive target.
const IMPORT_NET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Connects outbound to another Clay instance at `addr` (for `/import host[:port]`,
/// plan `i-d-like-to-make-snuggly-rain.md`), authenticates with `password` or `auth_key`,
/// and fetches its settings/theme/keybindings with secrets decrypted. Reuses the same
/// TOFU-pinned wss://-then-ws:// connection pattern as `run_console_client`/`run_grep_client`,
/// but — unlike those CLI-mode functions — never writes to stdout/stderr or exits the
/// process: this runs as a background task inside a live TUI/server (CLAUDE.md), and reports
/// outcomes back through its return value instead.
pub(crate) async fn run_import_client(
    addr: &str,
    password: Option<&str>,
    auth_key: Option<&str>,
    allow_insecure: bool,
) -> Result<(String, String, String), ImportClientError> {
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use futures::SinkExt;

    let remote = crate::util::RemoteAddr::parse(addr);
    let addr_with_port = remote.host_port.clone();
    let host_port_key = addr_with_port.clone();
    let ws_url = remote.url("wss");

    let ws_stream = {
        #[cfg(feature = "rustls-backend")]
        let attempt = tokio::time::timeout(IMPORT_NET_TIMEOUT, connect_wss(&ws_url, &host_port_key)).await;
        #[cfg(not(feature = "rustls-backend"))]
        let attempt = tokio::time::timeout(IMPORT_NET_TIMEOUT, connect_async(&ws_url)).await;

        match attempt {
            Ok(Ok((stream, _))) => stream,
            Ok(Err(e)) => {
                // A pin mismatch means the target's certificate changed since we last trusted
                // it (possible MITM, or a reinstalled/re-keyed server) — never silently fall
                // back to plaintext ws:// for that, unlike an ordinary connection failure.
                #[cfg(feature = "rustls-backend")]
                if let Some(mismatch) = platform::danger::take_cert_mismatch() {
                    return Err(ImportClientError::ConnectFailed(format!(
                        "TLS certificate for {} has changed (was {}, now {}) - refusing to connect",
                        mismatch.host, mismatch.old_fingerprint, mismatch.new_fingerprint
                    )));
                }
                // Never offer plaintext to a remote host: the import sends the password
                // or auth key, and the reply carries the target's secrets.
                if !crate::util::is_loopback_host(crate::util::host_of_addr(&addr_with_port)) {
                    return Err(ImportClientError::ConnectFailed(format!(
                        "Secure (TLS) connection to {} failed: {}{} - not falling back to unencrypted ws:// for a remote host",
                        addr_with_port, e, remote.path_hint(&e)
                    )));
                }
                if !allow_insecure {
                    return Err(ImportClientError::NeedsInsecureConfirm);
                }
                let fallback_url = remote.url("ws");
                match tokio::time::timeout(IMPORT_NET_TIMEOUT, connect_async(&fallback_url)).await {
                    Ok(Ok((stream, _))) => stream,
                    Ok(Err(e2)) => return Err(ImportClientError::ConnectFailed(format!("Failed to connect to {}: {} (after wss:// failed: {})", fallback_url, e2, e))),
                    Err(_) => return Err(ImportClientError::ConnectFailed(format!("Timed out connecting to {}", fallback_url))),
                }
            }
            Err(_) => return Err(ImportClientError::ConnectFailed(format!("Timed out connecting to {}", ws_url))),
        }
    };

    let (mut ws_write, mut ws_read) = ws_stream.split();
    let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<WsMessage>();
    tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // Wait for ServerHello to get the auth challenge.
    let server_challenge = match tokio::time::timeout(IMPORT_NET_TIMEOUT, async {
        loop {
            match ws_read.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(WsMessage::ServerHello { challenge, .. }) = serde_json::from_str::<WsMessage>(&text) {
                        return Some(challenge);
                    }
                }
                Some(Ok(Message::Close(_))) | None => return None,
                _ => {}
            }
        }
    }).await {
        Ok(Some(challenge)) => challenge,
        Ok(None) => return Err(ImportClientError::ConnectFailed("Connection closed before authentication".to_string())),
        Err(_) => return Err(ImportClientError::ConnectFailed("Timed out waiting for server hello".to_string())),
    };

    // Authenticate: auth-key path takes precedence when both are supplied, matching the
    // server's own routing in websocket.rs (checks `auth_key.is_some()` first). The stored
    // auth-key is never sent in the clear — only its challenge-response hash — mirroring
    // `handle_ws_auth_key_validation`'s `hash_with_challenge(&ak.key, challenge) == key` check.
    // `resume` is empty in both branches below: this client never touches world/scrollback
    // state at all (it only fetches settings/theme/keybindings for /import), so there is
    // nothing to resume.
    let auth_request = if let Some(key) = auth_key {
        WsMessage::AuthRequest {
            password_hash: String::new(),
            username: None,
            current_world: None,
            auth_key: Some(hash_with_challenge(key, &server_challenge)),
            request_key: false,
            challenge_response: true,
            resume: Vec::new(),
            resume_epochs: Vec::new(),
            client_version: crate::VERSION.to_string(),
            // /import client: fetches settings only, renders nothing, owns no ▶ markers.
            client_uid: String::new(),
        }
    } else {
        let password_hash = hash_password(password.unwrap_or_default());
        WsMessage::AuthRequest {
            password_hash: hash_with_challenge(&password_hash, &server_challenge),
            username: None,
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: true,
            resume: Vec::new(),
            resume_epochs: Vec::new(),
            client_version: crate::VERSION.to_string(),
            // /import client: fetches settings only, renders nothing, owns no ▶ markers.
            client_uid: String::new(),
        }
    };
    let _ = ws_tx.send(auth_request);

    match tokio::time::timeout(IMPORT_NET_TIMEOUT, async {
        loop {
            match ws_read.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(WsMessage::AuthResponse { success, error, .. }) = serde_json::from_str::<WsMessage>(&text) {
                        return Some((success, error));
                    }
                }
                Some(Ok(Message::Close(_))) | None => return None,
                _ => {}
            }
        }
    }).await {
        Ok(Some((true, _))) => {}
        Ok(Some((false, error))) => return Err(ImportClientError::AuthFailed(error.unwrap_or_else(|| "Authentication failed".to_string()))),
        Ok(None) => return Err(ImportClientError::ConnectFailed("Connection closed during authentication".to_string())),
        Err(_) => return Err(ImportClientError::ConnectFailed("Timed out waiting for authentication response".to_string())),
    }

    // Authenticated — request the export. Other post-auth traffic (InitialState etc.) may
    // arrive first; only the messages we're waiting for below stop the loop.
    let _ = ws_tx.send(WsMessage::RequestSettingsExport);

    match tokio::time::timeout(IMPORT_NET_TIMEOUT, async {
        loop {
            match ws_read.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(WsMessage::SettingsExport { settings_dat, theme_dat, keybindings_dat }) = serde_json::from_str::<WsMessage>(&text) {
                        return Some((settings_dat, theme_dat, keybindings_dat));
                    }
                }
                Some(Ok(Message::Close(_))) | None => return None,
                _ => {}
            }
        }
    }).await {
        Ok(Some(export)) => Ok(export),
        Ok(None) => Err(ImportClientError::ConnectFailed("Connection closed before receiving settings export".to_string())),
        Err(_) => Err(ImportClientError::ConnectFailed("Timed out waiting for settings export".to_string())),
    }
}

/// Run as console client connecting to remote daemon (--console=host:port)
/// Uses the same App struct and ui() function as the normal console interface
/// Boxed WebSocket sink/stream halves - the common type both the direct (TCP+TLS)
/// and `--ssh`-tunneled connect paths converge to, since their underlying stream
/// generics differ (`MaybeTlsStream<TcpStream>` vs. `ssh::SshTunnel`). Everything
/// past the connect (the `WsMessage` protocol, the read/write loop) is transport-
/// agnostic and untouched by which path was taken.
type BoxedWsWrite = Box<dyn futures::Sink<tokio_tungstenite::tungstenite::Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin + Send>;
type BoxedWsRead = Box<dyn futures::Stream<Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>> + Unpin + Send>;

/// Where this console connects. Kept after the first connect, so a reconnect (the server
/// reloaded, or the connection dropped) can open the same WebSocket again without asking.
#[derive(Clone)]
enum ConnectTarget {
    Direct {
        ws_url: String,
        /// Plain `ws://`, tried when `wss://` fails - only ever for this machine.
        fallback_url: Option<String>,
        remote: crate::util::RemoteAddr,
    },
    #[cfg(feature = "ssh-transport")]
    Ssh {
        target: crate::ssh::SshTarget,
        session: crate::ssh::SshSession,
    },
}

/// Why opening a connection, or logging in on a reconnect, failed.
enum ConnectError {
    /// The server's certificate isn't the pinned one: never trusted without asking.
    CertChanged(platform::danger::CertMismatch),
    /// The server refused the login.
    Refused(String),
    /// Anything else (refused connection, timeout, closed early): worth trying again.
    Failed(String),
}

/// What a reconnect logs in with: the password's hash, never the password itself (the
/// challenge answer needs only the hash), and on a multiuser server the user name.
#[derive(Clone)]
struct SavedLogin {
    username: Option<String>,
    password_hash: String,
}

impl ConnectTarget {
    /// The WebSocket, over TLS (or plain `ws://` to this machine) or the SSH tunnel.
    /// `first`: the console's first connect, which may print progress (the TUI isn't up).
    async fn open(&mut self, first: bool) -> Result<(BoxedWsWrite, BoxedWsRead), ConnectError> {
        use tokio_tungstenite::connect_async;
        match self {
            ConnectTarget::Direct { ws_url, fallback_url, remote } => {
                #[cfg(feature = "rustls-backend")]
                let result = if ws_url.starts_with("wss://") {
                    connect_wss(ws_url, &remote.host_port).await
                } else {
                    connect_async(ws_url.as_str()).await.map_err(Box::new)
                };
                #[cfg(not(feature = "rustls-backend"))]
                let result = connect_async(ws_url.as_str()).await;
                match result {
                    Ok((stream, _)) => {
                        let (w, r) = stream.split();
                        Ok((Box::new(w), Box::new(r)))
                    }
                    Err(e) => {
                        // A changed certificate, as opposed to an ordinary network error, is
                        // never quietly worked around with plaintext ws:// (that would mask a
                        // possible MITM).
                        if let Some(mismatch) = platform::danger::take_cert_mismatch() {
                            return Err(ConnectError::CertChanged(mismatch));
                        }
                        let Some(fallback) = fallback_url else {
                            return Err(ConnectError::Failed(format!("Failed to connect to {}: {}{}", ws_url, e, remote.path_hint(&e))));
                        };
                        if first {
                            eprintln!("Secure connection failed, trying {}...", fallback);
                        }
                        match connect_async(fallback.as_str()).await {
                            Ok((stream, _)) => {
                                let (w, r) = stream.split();
                                Ok((Box::new(w), Box::new(r)))
                            }
                            Err(e2) => Err(ConnectError::Failed(format!("Failed to connect to {}: {}{}", fallback, e2, remote.path_hint(&e2)))),
                        }
                    }
                }
            }
            #[cfg(feature = "ssh-transport")]
            ConnectTarget::Ssh { target, session } => {
                // A new channel on the same SSH login. If the SSH connection itself is gone
                // (a reconnect after a network drop), log in again without prompting: the
                // agent or a key file, never a typed password.
                let tunnel = match session.open_tunnel(target.clay_port).await {
                    Ok(t) => t,
                    Err(_) if !first => {
                        let creds = crate::ssh::SshCredentials::desktop_default();
                        *session = crate::ssh::establish_session(target, &creds, crate::ssh::AuthContext::NonInteractive)
                            .await
                            .map_err(|e| ConnectError::Failed(crate::ssh::render_ssh_error(&e)))?;
                        session.open_tunnel(target.clay_port).await.map_err(|e| ConnectError::Failed(crate::ssh::render_ssh_error(&e)))?
                    }
                    Err(e) => return Err(ConnectError::Failed(crate::ssh::render_ssh_error(&e))),
                };
                // Plaintext ws:// inside the tunnel: SSH already encrypts everything, and
                // the daemon's loopback listener (which is where this direct-tcpip channel
                // lands) never presents a TLS cert - see src/ssh.rs's doc comment.
                let ws_url = format!("ws://127.0.0.1:{}/", target.clay_port);
                if first {
                    println!("Connecting to {} (via SSH tunnel)...", ws_url);
                }
                match tokio_tungstenite::client_async(ws_url.as_str(), tunnel).await {
                    Ok((stream, _)) => {
                        let (w, r) = stream.split();
                        Ok((Box::new(w), Box::new(r)))
                    }
                    Err(e) => Err(ConnectError::Failed(format!("WebSocket handshake over SSH tunnel failed: {e}"))),
                }
            }
        }
    }
}

/// `wss://` with Clay's trust-on-first-use pinning (`~/.clay/known_hosts.dat`, see
/// platform::danger_rustls) rather than CA verification: Clay's own server is self-signed.
/// The error is boxed: tungstenite's is large, and these results are passed around.
#[cfg(feature = "rustls-backend")]
async fn connect_wss(ws_url: &str, host_port_key: &str) -> Result<
    (tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
     tokio_tungstenite::tungstenite::handshake::client::Response),
    Box<tokio_tungstenite::tungstenite::Error>,
> {
    let tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(platform::danger_rustls::TofuVerifier::new(host_port_key.to_string())))
        .with_no_client_auth();
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(tls_config));
    tokio_tungstenite::connect_async_tls_with_config(ws_url, None, false, Some(connector)).await.map_err(Box::new)
}

/// The task that writes this connection's messages to its socket, and the sender for it
/// (what `App::ws_client_tx` holds while connected).
fn spawn_ws_writer(mut ws_write: BoxedWsWrite) -> (mpsc::UnboundedSender<WsMessage>, tokio::task::JoinHandle<()>) {
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<WsMessage>();
    let handle = tokio::spawn(async move {
        while let Some(msg) = ws_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });
    (ws_tx, handle)
}

/// How long a reconnect waits for each step of the server's answer.
const RECONNECT_STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Seconds before each reconnect attempt, by attempt (the last repeats). The first is
/// quick: a server's hot reload is back within a second or two.
const RECONNECT_BACKOFF: [u64; 7] = [1, 2, 2, 5, 10, 20, 30];

/// One reconnect: the transport, the server's hello, the saved login and the server's
/// state (its `InitialState`). Never prompts: a changed certificate or a refused login
/// ends the retries. Returns the target too, since an SSH one may have logged in again.
async fn reconnect_once(
    mut target: ConnectTarget,
    login: SavedLogin,
) -> (ConnectTarget, Result<(BoxedWsWrite, BoxedWsRead, Box<WsMessage>), ConnectError>) {
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    let result = async {
        let (mut write, mut read) = target.open(false).await?;
        let failed = |what: &str| ConnectError::Failed(what.to_string());
        let (multiuser, challenge) = tokio::time::timeout(RECONNECT_STEP_TIMEOUT, async {
            loop {
                match read.next().await {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(WsMessage::ServerHello { multiuser_mode, challenge }) = serde_json::from_str::<WsMessage>(&text) {
                            return Ok((multiuser_mode, challenge));
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return Err(failed("the connection closed before the login")),
                    Some(Err(e)) => return Err(ConnectError::Failed(e.to_string())),
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| failed("no answer from the server"))??;
        if multiuser && login.username.is_none() {
            return Err(ConnectError::Refused("the server now asks for a user name".to_string()));
        }
        let auth = WsMessage::AuthRequest {
            password_hash: hash_with_challenge(&login.password_hash, &challenge),
            username: login.username.clone(),
            current_world: None,
            auth_key: None,
            request_key: false,
            challenge_response: true,
            resume: Vec::new(),
            resume_epochs: Vec::new(),
            client_version: String::new(),
            client_uid: String::new(),
        };
        let json = serde_json::to_string(&auth).map_err(|e| ConnectError::Failed(e.to_string()))?;
        write.send(Message::Text(json)).await.map_err(|e| ConnectError::Failed(e.to_string()))?;
        let initial = tokio::time::timeout(RECONNECT_STEP_TIMEOUT, async {
            loop {
                match read.next().await {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<WsMessage>(&text) {
                        Ok(WsMessage::AuthResponse { success: false, error, .. }) => {
                            return Err(ConnectError::Refused(error.filter(|e| !e.is_empty()).unwrap_or_else(|| "login refused".to_string())));
                        }
                        Ok(msg @ WsMessage::InitialState { .. }) => return Ok(Box::new(msg)),
                        _ => {}
                    },
                    Some(Ok(Message::Close(_))) | None => return Err(failed("the connection closed during the login")),
                    Some(Err(e)) => return Err(ConnectError::Failed(e.to_string())),
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| failed("no answer from the server"))??;
        Ok((write, read, initial))
    }
    .await;
    (target, result)
}

/// The server's `InitialState`, at the first connect and after every reconnect: this
/// console's worlds, settings and view become the server's again. What belongs to the
/// console - the input line, its history, an open editor - stays.
fn apply_initial_state(app: &mut App, msg: WsMessage, ws_tx: &mpsc::UnboundedSender<WsMessage>, version_mismatch_shown: &mut bool) {
    let WsMessage::InitialState { worlds, current_world_index, settings, splash_lines, actions, server_version, server_version_detail, tf_status, expand_line, picture_upload, .. } = msg else {
        return;
    };
    app.server_expands_line = expand_line;
    app.server_picture_upload = picture_upload;
    app.server_version_detail = crate::server_version_detail(&server_version_detail, &server_version);
    // An open editor is kept by its world's name: the world list is the server's again.
    let notes_world = app.editor.world_index.and_then(|i| app.worlds.get(i)).map(|w| w.name.clone());
    let mcp_world = app.editor.mcp_edit.as_ref().and_then(|c| app.worlds.get(c.world_index)).map(|w| w.name.clone());
    // Requests made on the old connection get no answer on this one.
    app.pending_scrollback_requests.clear();
    app.pending_note_open = None;
    app.pending_note_list = None;
    // Save world totals for backfill before consuming worlds vec
    let world_totals: Vec<(usize, usize)> = worlds.iter()
        .map(|w| (w.index, w.total_output_lines))
        .collect();
    // Initialize app state from server
    app.init_from_initial_state(worlds, current_world_index, settings, splash_lines, actions);
    app.tf_status = tf_status.into_iter().map(|e| (e.world_index, e.view)).collect();
    let find = |app: &App, name: &str| app.worlds.iter().position(|w| w.name == name);
    if let Some(name) = notes_world {
        match find(app, &name) {
            Some(i) => app.editor.world_index = Some(i),
            None => {
                app.editor.close();
                app.needs_terminal_clear = true;
                app.add_output("The world whose notes were open is gone; the editor is closed.");
            }
        }
    }
    if let Some(name) = mcp_world {
        match find(app, &name) {
            Some(i) => {
                if let Some(ctx) = app.editor.mcp_edit.as_mut() {
                    ctx.world_index = i;
                }
            }
            None => {
                app.editor.close();
                app.needs_terminal_clear = true;
                app.add_output("The world being edited is gone; the editor is closed.");
            }
        }
    }
    // Warn once per session if our version differs from the server's. An empty
    // server_version means an old server that predates this field (#[serde(default)]) -
    // stay silent, not a mismatch.
    if !*version_mismatch_shown && !server_version.is_empty() && server_version != crate::VERSION {
        app.add_output(&format!(
            "Version mismatch: {} (local) ≠ {} (remote).",
            crate::VERSION, server_version
        ));
        *version_mismatch_shown = true;
    }
    // Initialize backfill queue: a one-time guaranteed screenful per world
    // (max(75, visible rows)) so switching to any world shows content immediately.
    // That's the only auto-fetch this client ever does - it does NOT deep-fill further
    // toward "Remote Lines" (that setting only bounds the app/gui/web clients'
    // download); anything older than the initial screenful is fetched exclusively on
    // demand, via scroll_page_up. See init_backfill.
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let visible_lines = height.saturating_sub(4) as usize; // Account for input area and separator
    app.init_backfill(&world_totals, visible_lines.max(75));
    // Declare client type to server (RemoteConsole for TUI clients)
    let _ = ws_tx.send(WsMessage::ClientTypeDeclaration {
        client_type: websocket::RemoteClientType::RemoteConsole,
    });
    // Send initial view state for more-mode sync
    let _ = ws_tx.send(WsMessage::UpdateViewState {
        world_index: current_world_index,
        visible_lines,
        visible_columns: Some(width as usize),
    });
    // Mark the world we're displaying as seen. Only a world *switch* ever sends
    // MarkWorldSeen otherwise, so the world on screen at connect would keep its
    // server-side unseen_lines and a stale first_unseen_at - which Unseen First cycling
    // later mistakes for the oldest activity and bounces back to. Same fix as app.js's
    // InitialState handler.
    let _ = ws_tx.send(WsMessage::MarkWorldSeen {
        world_index: current_world_index,
        previous_world_index: None,
    });
    app.needs_output_redraw = true;
}

pub(crate) async fn run_console_client(addr: &str, ssh: Option<crate::ssh::SshTarget>) -> io::Result<()> {
    use tokio_tungstenite::tungstenite::Message;

    let mut target = if let Some(target) = ssh {
        // --ssh: tunnel through an embedded SSH client instead of connecting
        // directly. Must happen before any TUI setup (this whole branch runs in
        // the same plain-stdio region as the println!/prompt calls below it) so
        // an encrypted-key passphrase prompt is safe here - see CLAUDE.md's
        // no-println!-after-TUI-init rule and src/ssh.rs's module doc comment.
        #[cfg(feature = "ssh-transport")]
        {
            println!("Establishing SSH tunnel to {}@{}:{}...", target.user, target.host, target.ssh_port);
            let creds = crate::ssh::SshCredentials::desktop_default();
            match crate::ssh::establish_session(&target, &creds, crate::ssh::AuthContext::InteractiveTerminal).await {
                Ok(session) => ConnectTarget::Ssh { target, session },
                Err(e) => {
                    eprintln!("{}", crate::ssh::render_ssh_error(&e));
                    return Ok(());
                }
            }
        }
        #[cfg(not(feature = "ssh-transport"))]
        {
            // Unreachable in practice: main.rs already refuses --ssh when this
            // feature isn't compiled in, before it ever reaches here.
            let _ = target;
            eprintln!("clay: --ssh requires the 'ssh-transport' feature.");
            return Ok(());
        }
    } else {
        // Parse address - default port 9000 (same as --gui), then the web path's endpoint
        let remote = crate::util::RemoteAddr::parse(addr);
        let (ws_url, try_fallback) = match remote.scheme {
            Some(scheme) => (remote.url(scheme), false),
            // Default to wss:// for security; falls back to ws:// only for this machine
            None => (remote.url("wss"), true),
        };
        // Plaintext ws:// only to this machine; see util::is_loopback_host.
        let plaintext_ok = crate::util::is_loopback_host(crate::util::host_of_addr(&remote.host_port));
        if ws_url.starts_with("ws://") && !plaintext_ok {
            eprintln!("Refusing unencrypted ws:// to a remote host (the login would be sent in the clear). Use wss:// or --ssh.");
            return Ok(());
        }
        let fallback_url = (try_fallback && plaintext_ok).then(|| remote.url("ws"));

        println!("Connecting to {}...", ws_url);

        // Relaunched by /update while the server reloads: give it time to come back.
        wait_for_server_if_relaunched(&remote.host_port).await;
        ConnectTarget::Direct { ws_url, fallback_url, remote }
    };

    let (ws_write, mut ws_read) = loop {
        match target.open(true).await {
            Ok(pair) => break pair,
            Err(ConnectError::CertChanged(mismatch)) => {
                // Prompt interactively: the TUI isn't up yet.
                eprintln!();
                eprintln!("*** TLS CERTIFICATE FOR {} HAS CHANGED ***", mismatch.host);
                eprintln!("  old fingerprint: {}", mismatch.old_fingerprint);
                eprintln!("  new fingerprint: {}", mismatch.new_fingerprint);
                eprintln!("This could mean the server was reinstalled, or that someone is intercepting your connection.");
                eprint!("Trust the new certificate and reconnect? [y/N] ");
                let _ = io::stdout().flush();
                let mut answer = String::new();
                let _ = io::stdin().read_line(&mut answer);
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Not trusting new certificate; aborting connection.");
                    return Ok(());
                }
                persistence::replace_pin(&mismatch.host, &mismatch.new_fingerprint);
            }
            Err(ConnectError::Failed(msg) | ConnectError::Refused(msg)) => {
                eprintln!("{msg}");
                return Ok(());
            }
        }
    };

    // Messages to the server go through this channel to the writer task.
    let (mut ws_tx, mut ws_write_handle) = spawn_ws_writer(ws_write);

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
    // Kept for reconnecting (the hash only).
    let mut saved_login: Option<SavedLogin> = None;
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
                            password.clear();
                            let challenge_hash = hash_with_challenge(&password_hash, &server_challenge);
                            saved_login = Some(SavedLogin { username: username.clone(), password_hash });
                            // `resume` is empty: a reconnect gets the server's whole state
                            // again (apply_initial_state replaces the mirrored worlds), so
                            // there is no per-world state to resume from. Live gap recovery
                            // for a connection that stays up is handled separately via
                            // `ResyncRequired` in `App::handle_remote_ws_message`
                            // (PROTOCOL-ROADMAP.md Step 6).
                            let _ = ws_tx.send(WsMessage::AuthRequest { password_hash: challenge_hash, username: username.clone(), current_world: None, auth_key: None, request_key: false, challenge_response: true, resume: Vec::new(), resume_epochs: Vec::new(), client_version: String::new(), client_uid: String::new() });
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
                    Some(Ok(Message::Close(frame))) => {
                        disable_raw_mode()?;
                        println!();
                        match frame.map(|f| f.reason.to_string()).filter(|r| !r.is_empty()) {
                            Some(reason) => eprintln!("Connection closed: {reason}"),
                            None => eprintln!("Connection closed"),
                        }
                        return Ok(());
                    }
                    None => {
                        disable_raw_mode()?;
                        println!();
                        eprintln!("Connection closed");
                        return Ok(());
                    }
                    // Ping/Pong/Binary/etc. and transport errors: not a close, keep waiting.
                    _ => {}
                }
            }
        }
    }
    let saved_login = saved_login.unwrap_or(SavedLogin { username, password_hash: String::new() });

    // Create App struct and set it up for remote client mode
    let mut app = App::new();
    app.ws_client_tx = Some(ws_tx.clone());
    app.is_master = false;

    // Warn at most once per session (not on every reconnect) if this console's own
    // version drifts from the server's.
    let mut version_mismatch_shown = false;

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
        match ws_read.next().await {
            Some(Ok(Message::Text(text))) => {
                match serde_json::from_str::<WsMessage>(&text) {
                    Ok(WsMessage::AuthResponse { success: false, error, .. }) => {
                        disable_raw_mode()?;
                        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                        eprintln!("Authentication failed: {}", error.unwrap_or_default());
                        return Ok(());
                    }
                    Ok(msg @ WsMessage::InitialState { .. }) => {
                        apply_initial_state(&mut app, msg, &ws_tx, &mut version_mismatch_shown);
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // Invalid JSON / undeserializable message while waiting for
                        // InitialState - log and keep waiting rather than vanishing it
                        // with no trace (T2.4; this used to be a bare `if let Ok(..)`).
                        let type_field = serde_json::from_str::<serde_json::Value>(&text)
                            .ok()
                            .and_then(|v| v.get("type").and_then(|t| t.as_str().map(String::from)))
                            .unwrap_or_else(|| "?".to_string());
                        crate::http::log_remote_event("WS-BAD-MESSAGE", addr,
                            &format!("type={} err={} len={}", type_field, e, text.len()));
                    }
                }
            }
            // Keepalive frames, not a close - tungstenite surfaces these on the read
            // stream while we're waiting; treating them as "connection closed" (as a
            // blanket catch-all previously did) would wrongly abort a live connection.
            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(frame))) => {
                disable_raw_mode()?;
                let _ = execute!(terminal.backend_mut(), crossterm::event::DisableBracketedPaste);
                execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                match frame.map(|f| f.reason.to_string()).filter(|r| !r.is_empty()) {
                    Some(reason) => eprintln!("Connection closed while waiting for initial state: {reason}"),
                    None => eprintln!("Connection closed while waiting for initial state"),
                }
                return Ok(());
            }
            Some(Ok(Message::Binary(_) | Message::Frame(_))) => {}
            Some(Err(e)) => {
                disable_raw_mode()?;
                let _ = execute!(terminal.backend_mut(), crossterm::event::DisableBracketedPaste);
                execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                eprintln!("Connection error while waiting for initial state: {e}");
                return Ok(());
            }
            None => {
                disable_raw_mode()?;
                let _ = execute!(terminal.backend_mut(), crossterm::event::DisableBracketedPaste);
                execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                eprintln!("Connection closed while waiting for initial state");
                return Ok(());
            }
        }
    }

    // Main event loop - use the same ui() function as the normal console
    app.needs_output_redraw = true;
    let mut needs_redraw = true;

    // Backfill timer: fires after initial delay to start requesting scrollback history
    let mut backfill_timer = std::pin::pin!(tokio::time::sleep(std::time::Duration::from_millis(500)));
    let mut backfill_timer_active = app.backfill_needed();

    // Channel for local /update results
    let (update_tx, mut update_rx) = mpsc::channel::<OwnUpdate>(4);
    // /update (src/selfupdate.rs): this console's own download is running; the socket
    // closed under it (the server reloaded); the wait for the server's answer.
    let mut own_update_running = false;
    let mut ws_closed = false;
    let mut update_timeout = std::pin::pin!(tokio::time::sleep(crate::selfupdate::SERVER_ANSWER_TIMEOUT));
    let mut update_timeout_armed = false;
    // /note -l (App::open_notes_list): when it stops waiting for the worlds' notes.
    let mut notes_list_timeout = std::pin::pin!(tokio::time::sleep(std::time::Duration::from_secs(3)));
    // Import Picture: a lost ack is resent, a server that never answers given up on.
    let mut picture_tick = tokio::time::interval(std::time::Duration::from_secs(1));
    let mut notes_list_armed = false;

    // The output area's size as last reported to the server (UpdateViewState).
    let mut reported_view: Option<(u16, u16)> = None;

    // Reconnecting (the server reloaded, or the connection dropped): the attempt count,
    // the timer for the next attempt, and the attempt's answer. While disconnected,
    // `ws_tx` is `offline_tx`, whose messages are caught so a line typed meanwhile is
    // reported rather than silently lost.
    let mut server_reloading = false;
    let mut reconnect_attempt: Option<usize> = None;
    let mut reconnect_timer = std::pin::pin!(tokio::time::sleep(std::time::Duration::from_secs(1)));
    let mut reconnect_armed = false;
    let (reconnect_tx, mut reconnect_rx) =
        mpsc::channel::<(ConnectTarget, Result<(BoxedWsWrite, BoxedWsRead, Box<WsMessage>), ConnectError>)>(1);
    let (offline_tx, mut offline_rx) = mpsc::unbounded_channel::<WsMessage>();
    // Set by an arm below when the connection has just gone.
    let mut connection_lost = false;
    // The text-reveal effect's next frame (src/reveal.rs), set only by a draw that found a
    // line still revealing on screen.
    let mut reveal_due: Option<tokio::time::Instant> = None;

    loop {
        // /reload or /update on Windows: the new console starts once this loop is over.
        #[cfg(windows)]
        if RELAUNCH_ON_EXIT.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
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
                let want_mouse = any_popup_visible || app.shell_live(app.current_world_index);
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
            reveal_due = render_output_crossterm(&app).map(|d| tokio::time::Instant::now() + d);
            needs_redraw = false;
            app.needs_output_redraw = false;
            // The output area as just laid out - not the terminal's height less a guess at
            // the rest - since the server sizes more-mode and a shell world's PTY by it: a
            // PTY taller than this area has its bottom rows drawn off the screen. Sent on
            // any change (a terminal resize, the input area, a status line).
            let view = (app.output_height, app.output_width);
            if reported_view != Some(view) {
                reported_view = Some(view);
                let _ = ws_tx.send(WsMessage::UpdateViewState {
                    world_index: app.current_world_index,
                    visible_lines: view.0 as usize,
                    visible_columns: Some(view.1 as usize),
                });
            }
        }

        // Arm the wait for the server's /update answer when a request starts; disarm it
        // once the answer (or an old server's bounce) has moved things on.
        match app.update_flow {
            Some(crate::selfupdate::ClientFlow::WaitingServer { since, .. }) if !update_timeout_armed => {
                update_timeout.as_mut().reset(tokio::time::Instant::from_std(since + crate::selfupdate::SERVER_ANSWER_TIMEOUT));
                update_timeout_armed = true;
            }
            Some(_) => {}
            None => update_timeout_armed = false,
        }
        match app.pending_note_list {
            Some((_, until)) if !notes_list_armed => {
                notes_list_timeout.as_mut().reset(tokio::time::Instant::from_std(until));
                notes_list_armed = true;
            }
            Some(_) => {}
            None => notes_list_armed = false,
        }

        tokio::select! {
            maybe_event = event_stream.next() => {
                if let Some(Ok(event)) = maybe_event {
                    match event {
                        Event::Mouse(mouse)
                            if app.settings.mouse_enabled
                                && !app.has_new_popup()
                                && (app.input_resize_drag.is_some()
                                    || (matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) && app.on_separator_row(mouse.row))) =>
                        {
                            // The separator bar (the input's top edge): drag it up or down
                            // to resize the input; the height goes to the server, which
                            // keeps it, when it's let go.
                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => app.input_resize_drag = Some((mouse.row, app.input_height)),
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    app.input_resize_drag_to(mouse.row);
                                }
                                MouseEventKind::Up(MouseButton::Left) if app.input_resize_end() => {
                                    send_global_settings(&mut app, String::new(), &ws_tx);
                                }
                                _ => {}
                            }
                        }
                        Event::Mouse(mouse) if app.settings.mouse_enabled && !app.has_new_popup() && app.shell_live(app.current_world_index) => {
                            // A live shell world: forward the mouse to the server (it
                            // decides what the program receives). 1-based → 0-based grid.
                            let kind = match mouse.kind {
                                MouseEventKind::ScrollUp => Some(("wheel_up", 0u8)),
                                MouseEventKind::ScrollDown => Some(("wheel_down", 0)),
                                MouseEventKind::Down(b) => Some(("down", remote_mouse_button(b))),
                                MouseEventKind::Up(b) => Some(("up", remote_mouse_button(b))),
                                MouseEventKind::Drag(b) => Some(("drag", remote_mouse_button(b))),
                                MouseEventKind::Moved => Some(("move", 0)),
                                _ => None,
                            };
                            if let (Some((kind, button)), true) = (kind, mouse.row >= 1) {
                                let _ = ws_tx.send(WsMessage::ShellMouse {
                                    world_index: app.current_world_index,
                                    kind: kind.to_string(),
                                    button,
                                    col: mouse.column.saturating_sub(1),
                                    row: mouse.row - 1,
                                    ctrl: mouse.modifiers.contains(KeyModifiers::CONTROL),
                                    alt: mouse.modifiers.contains(KeyModifiers::ALT),
                                    shift: mouse.modifiers.contains(KeyModifiers::SHIFT),
                                });
                            }
                        }
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
                                        popup::console_renderer::render_popup_content_direct(&mut terminal, state, &tc);
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
                                        popup::console_renderer::render_popup_content_direct(&mut terminal, state, &tc);
                                    }
                                    continue; // Skip full redraw
                                }
                                _ => {}
                            }
                            needs_redraw = true;
                        }
                        // The notes editor (/note) has the focus: the paste is its text.
                        Event::Paste(text) if app.editor.visible && app.editor.focus == crate::EditorFocus::Editor => {
                            app.drop_sniffer.reset();
                            crate::input_handler::paste_into_editor(&mut app, &text);
                            needs_redraw = true;
                        }
                        Event::Paste(text) if app.shell_live(app.current_world_index) => {
                            app.drop_sniffer.reset();
                            // A live shell world: the paste goes to the PTY over the wire.
                            if let Some(msg) = crate::input_handler::oversized_paste_message(&text) {
                                app.add_output(&msg);
                            } else {
                                let _ = ws_tx.send(WsMessage::ShellInput {
                                    world_index: app.current_world_index,
                                    key: None,
                                    text: Some(text),
                                    paste: true,
                                });
                            }
                            needs_redraw = true;
                        }
                        Event::Paste(text) => {
                            // Splice the paste in as one string. Feeding it through
                            // `insert_char` per character made this quadratic: every
                            // insert re-derived the cursor's display line by walking
                            // the buffer from the start, so an N-character paste cost
                            // O(N^2) - minutes of hang for the 700KB paste that
                            // prompted this, versus milliseconds now.
                            app.drop_sniffer.reset();
                            if app.has_new_popup() {
                                // A popup's text field takes the paste, as locally.
                                crate::input_handler::handle_paste(&mut app, &text);
                            } else if let Some(paths) = app.picture_drop(&text) {
                                // A picture dropped onto the terminal (Import Picture): asked
                                // about, never typed.
                                app.ask_picture_upload(paths);
                            } else if let Some(msg) = crate::input_handler::oversized_paste_message(&text) {
                                app.add_output(&msg);
                            } else {
                                let clean: String = text
                                    .replace("\r\n", "\n")
                                    .replace('\r', "\n")
                                    .chars()
                                    .filter(|c| *c == '\n' || !c.is_control())
                                    .collect();
                                app.input.insert_str(&clean);
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
                            // Misspelled words underlined as they're typed, as locally.
                            app.check_word_ended();
                            // Check if a /reload was requested (re-exec local binary)
                            if app.pending_reload {
                                app.pending_reload = false;
                                relaunch_console(&mut app);
                            }
                            // Check if a /server --close was requested (detach and become
                            // an independent master)
                            if app.pending_remote_detach {
                                app.pending_remote_detach = false;
                                if let Err(e) = crate::platform::exec_relaunch(None, false) {
                                    // The terminal was handed back for the exec.
                                    app.needs_terminal_clear = true;
                                    app.add_output(&format!("Detach failed: {}", e));
                                }
                            }
                            // Check if a /server host:port was requested (switch to a
                            // different remote server)
                            if let Some(addr) = app.pending_remote_switch.take() {
                                if let Err(e) = crate::platform::exec_relaunch(Some(&addr), false) {
                                    // The terminal was handed back for the exec.
                                    app.needs_terminal_clear = true;
                                    app.add_output(&format!("Connect failed: {}", e));
                                }
                            }
                            // The server's half of /update is over: this console's own.
                            if let Some(force) = app.pending_update.take() {
                                own_update_running = start_own_update(&mut app, &update_tx, force);
                            }
                            needs_redraw = true;
                            // Import Picture: a drop the terminal types is a burst with more
                            // input already waiting - no draw while it lasts; when the input
                            // runs dry, see whether it was one (src/picture.rs).
                            app.drop_sniffer.end_key();
                            if app.drop_sniffer.is_active() {
                                if crossterm::event::poll(std::time::Duration::ZERO).unwrap_or(false) {
                                    needs_redraw = false;
                                } else {
                                    app.finish_typed_burst();
                                }
                            }
                        }
                        Event::Resize(_, _) => {
                            // The redraw lays the output area out again and reports its
                            // new size (see reported_view above).
                            needs_redraw = true;
                        }
                        // A release (Windows reports one after every press): nothing, unless
                        // it ends a typed burst.
                        Event::Key(_) if app.release_ends_typed_burst() => {
                            needs_redraw = true;
                        }
                        _ => {}
                    }
                }
            }
            msg = ws_read.next(), if !ws_closed => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<WsMessage>(&text) {
                        Ok(ws_msg) => {
                            // The server is about to reload: the close that follows is
                            // that, and the reconnect can start quickly.
                            if matches!(ws_msg, WsMessage::ServerReloading) {
                                server_reloading = true;
                            }
                            app.handle_remote_ws_message(ws_msg);
                            // The server ran /quit for this console (an action, a macro).
                            if app.pending_quit {
                                break;
                            }
                            // After processing ScrollbackLines, backfill_next may be set
                            if let Some((world_idx, before_seq, count, request_id)) = app.backfill_next.take() {
                                let _ = ws_tx.send(WsMessage::RequestScrollback {
                                    world_index: world_idx,
                                    count,
                                    before_seq,
                                    after_seq: None,
                                    request_id: Some(request_id),
                                });
                            }
                            // The server's half of /update is over (UpdateOutcome, or an
                            // old server's bounce): this console's own.
                            if let Some(force) = app.pending_update.take() {
                                own_update_running = start_own_update(&mut app, &update_tx, force);
                            }
                            needs_redraw = true;
                        }
                        Err(e) => {
                            // Invalid JSON / undeserializable message - log and keep the
                            // connection open rather than vanishing it with no trace
                            // (T2.4; this used to be a bare `if let Ok(..)`).
                            let type_field = serde_json::from_str::<serde_json::Value>(&text)
                                .ok()
                                .and_then(|v| v.get("type").and_then(|t| t.as_str().map(String::from)))
                                .unwrap_or_else(|| "?".to_string());
                            crate::http::log_remote_event("WS-BAD-MESSAGE", addr,
                                &format!("type={} err={} len={}", type_field, e, text.len()));
                        }
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        // Respond to ping - handled by tungstenite automatically
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        ws_closed = true;
                        if own_update_running || app.pending_update.is_some() || app.update_flow.is_some() {
                            // The server reloading under a /update takes the connection
                            // with it; this console restarts itself (attached again) once
                            // its own update is done, so keep going without the socket
                            // until then.
                            app.update_server_reloading = true;
                        } else {
                            connection_lost = true;
                        }
                    }
                    _ => {}
                }
            }
            // The next reconnect attempt is due: it runs on its own task, so the keys
            // keep working while it waits for the server.
            () = &mut reconnect_timer, if reconnect_armed => {
                reconnect_armed = false;
                let (tx, target, login) = (reconnect_tx.clone(), target.clone(), saved_login.clone());
                tokio::spawn(async move {
                    let _ = tx.send(reconnect_once(target, login).await).await;
                });
            }
            Some((new_target, result)) = reconnect_rx.recv() => {
                target = new_target;
                match result {
                    Ok((write, read, initial)) => {
                        let (tx, handle) = spawn_ws_writer(write);
                        ws_tx = tx;
                        ws_write_handle = handle;
                        ws_read = read;
                        ws_closed = false;
                        server_reloading = false;
                        reconnect_attempt = None;
                        app.ws_client_tx = Some(ws_tx.clone());
                        apply_initial_state(&mut app, *initial, &ws_tx, &mut version_mismatch_shown);
                        app.add_output("Reconnected.");
                        // Report the view again, and fetch each world's first screenful.
                        reported_view = None;
                        backfill_timer.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_millis(500));
                        backfill_timer_active = app.backfill_needed();
                    }
                    Err(ConnectError::Refused(why)) => {
                        reconnect_attempt = None;
                        app.add_output(&format!("Reconnecting stopped: the server refused the login ({why}). Start this console again to log in."));
                    }
                    Err(ConnectError::CertChanged(_)) => {
                        reconnect_attempt = None;
                        app.add_output("Reconnecting stopped: the server's certificate has changed. Start this console again to check and trust it.");
                    }
                    Err(ConnectError::Failed(why)) => {
                        let attempt = reconnect_attempt.map_or(1, |a| a + 1);
                        reconnect_attempt = Some(attempt);
                        let secs = RECONNECT_BACKOFF[attempt.min(RECONNECT_BACKOFF.len() - 1)];
                        app.add_output(&format!("Reconnect failed ({why}); trying again in {secs}s."));
                        reconnect_timer.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
                        reconnect_armed = true;
                    }
                }
                needs_redraw = true;
            }
            // This console's own /update found a release, or is over (src/selfupdate.rs).
            step = update_rx.recv() => {
                if let Some(OwnUpdate::Found(version)) = &step {
                    app.add_output(&crate::selfupdate::updating(Some(OWN_LABEL), version));
                    needs_redraw = true;
                }
                if let Some(OwnUpdate::Done(result)) = step {
                    own_update_running = false;
                    finish_own_update(&mut app, result);
                    if ws_closed && reconnect_attempt.is_none() {
                        // The server reloaded and the relaunch didn't happen (or failed):
                        // connect to it again.
                        connection_lost = true;
                    }
                    needs_redraw = true;
                }
            }
            // The server never answered the /update: go on with this console's own.
            () = &mut update_timeout, if update_timeout_armed => {
                update_timeout_armed = false;
                if let Some(crate::selfupdate::ClientFlow::WaitingServer { force, .. }) = app.update_flow.take() {
                    app.add_output(&crate::selfupdate::server_no_answer());
                    own_update_running = start_own_update(&mut app, &update_tx, force);
                    needs_redraw = true;
                }
            }
            // Text reveal: the next frame of a line being typed onto the screen.
            _ = tokio::time::sleep_until(reveal_due.unwrap_or_else(|| tokio::time::Instant::now() + std::time::Duration::from_secs(86400))), if reveal_due.is_some() => {
                reveal_due = None;
                needs_redraw = true;
                app.needs_output_redraw = true;
            }
            // Import Picture: resend after a lost ack, give up on no answer.
            _ = picture_tick.tick(), if app.picture_client.is_busy() => {
                let (msgs, notes) = app.picture_client.tick(std::time::Instant::now());
                app.send_picture_messages(msgs);
                if !notes.is_empty() {
                    app.apply_picture_notes(notes);
                    needs_redraw = true;
                }
            }
            // /note -l: not every world's notes came (a multiuser server never answers
            // for another user's world) - list what did.
            () = &mut notes_list_timeout, if notes_list_armed => {
                notes_list_armed = false;
                app.expire_notes_list_wait();
                needs_redraw = true;
            }
            // Backfill timer: start requesting scrollback history after initial delay
            () = &mut backfill_timer, if backfill_timer_active => {
                backfill_timer_active = false;
                // Start backfill from the first world in the queue
                app.backfill_advance_to_next();
                if let Some((world_idx, before_seq, count, request_id)) = app.backfill_next.take() {
                    let _ = ws_tx.send(WsMessage::RequestScrollback {
                        world_index: world_idx,
                        count,
                        before_seq,
                        after_seq: None,
                        request_id: Some(request_id),
                    });
                }
            }
        }

        // /suspend or Ctrl+Z: stop this console, as the local console stops itself.
        if !app.pending_terminal_ops.is_empty() {
            crate::run_terminal_ops(&mut app, &mut terminal)?;
            app.needs_terminal_clear = true;
            needs_redraw = true;
        }

        // The connection has just gone: keep the console up and connect again, as the
        // web and GUI clients do. The input line, its history and an open editor stay.
        if std::mem::take(&mut connection_lost) {
            ws_write_handle.abort();
            ws_tx = offline_tx.clone();
            app.ws_client_tx = None;
            app.server_version_detail = None;
            let notes = app.picture_client.connection_lost();
            app.apply_picture_notes(notes);
            app.add_output(if server_reloading {
                "The server is reloading; reconnecting..."
            } else {
                "The connection to the server was lost; reconnecting..."
            });
            reconnect_attempt = Some(0);
            reconnect_timer.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_secs(RECONNECT_BACKOFF[0]));
            reconnect_armed = true;
            needs_redraw = true;
        }
        // Whatever was sent while disconnected never left: a typed line says so.
        while let Ok(msg) = offline_rx.try_recv() {
            if let WsMessage::SendCommand { command, .. } = msg {
                app.add_output(&format!("Not connected - not sent: {command}"));
                needs_redraw = true;
            }
        }
    }

    // Cleanup
    ws_write_handle.abort();
    disable_raw_mode()?;
    let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    let _ = execute!(terminal.backend_mut(), crossterm::event::DisableBracketedPaste);
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    #[cfg(windows)]
    if RELAUNCH_ON_EXIT.load(std::sync::atomic::Ordering::SeqCst) {
        // This console's key reader must be gone first: crossterm's wake thread reads
        // console input itself, and the window's input is the new console's from now on.
        drop(event_stream);
        relaunch_and_wait();
    }
    Ok(())
}
/// Handle key events for remote console client mode
/// Returns true if quit was requested
/// Crossterm mouse button → the code `shell::keys::mouse` expects (0 left, 1 middle, 2
/// right).
/// What this console calls itself in `/update`'s lines (src/selfupdate.rs).
const OWN_LABEL: &str = "This console";

/// The current world's splash goes on the user's input.
fn clear_splash_on_input(app: &mut App) {
    if app.current_world().showing_splash {
        let world = app.current_world_mut();
        world.showing_splash = false;
        world.needs_redraw = true;
        world.output_lines.clear();
        world.scroll_offset = 0;
        app.needs_output_redraw = true;
    }
}

/// `/version` (and `^X^V`): this console's build and, while it is connected, the
/// server's (`crate::attached_version_lines`).
fn version_command(app: &mut App) {
    match app.server_version_detail.clone() {
        Some(remote) => {
            for line in crate::attached_version_lines(&crate::version_detail(), &remote) {
                app.add_output(&line);
            }
        }
        None => app.add_output(&get_version_string()),
    }
}

/// The server this console restarts against, for `/update`'s lines.
fn attached_to() -> String {
    crate::selfupdate::attached_target_from_args().unwrap_or_else(|| "the same server".to_string())
}

/// This console starts again as it was started: `/reload`, and the end of `/update`.
/// Returns only if the exec failed (the terminal is then back as it was) - or, on
/// Windows, at once: the loop ends at its next pass and `relaunch_and_wait` follows.
fn relaunch_console(app: &mut App) {
    #[cfg(all(unix, not(target_os = "android")))]
    {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen
        );
        let failure = if let Ok((exe, _)) = get_executable_path() {
            use std::os::unix::process::CommandExt;
            let args: Vec<String> = std::env::args().skip(1).collect();
            // The new process may start while the server is still coming back from
            // its own reload: let it wait for the port (run_console_client).
            std::env::set_var(CONNECT_RETRY_ENV, "1");
            let err = std::process::Command::new(&exe).args(&args).exec();
            std::env::remove_var(CONNECT_RETRY_ENV);
            format!("Reload failed: {}", err)
        } else {
            "Reload failed: cannot find executable path".to_string()
        };
        // The exec failed: take the terminal back, and repaint all of it - the alternate
        // screen comes back blank, while ratatui still takes it for its last frame.
        let _ = crossterm::terminal::enable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableBracketedPaste
        );
        app.needs_terminal_clear = true;
        app.needs_output_redraw = true;
        app.add_output(&failure);
    }
    // Windows can't replace a running process: the console's loop ends, and the new one
    // is started in the same window (relaunch_and_wait).
    #[cfg(windows)]
    {
        let _ = app;
        RELAUNCH_ON_EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(not(any(all(unix, not(target_os = "android")), windows)))]
    {
        app.add_output("Reload is not available on this platform.");
    }
}

/// Set by `relaunch_console` on Windows: end the console's loop, then start it again.
#[cfg(windows)]
static RELAUNCH_ON_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The Windows half of `relaunch_console`, once the TUI is torn down: the new console
/// runs in this window with the same arguments, and this one waits for it and exits with
/// its code, so the shell's prompt never comes back in between (as the Windows console's
/// own `/reload` does, platform::exec_reload). Returns only if it could not start.
#[cfg(windows)]
fn relaunch_and_wait() {
    // Give crossterm's wake thread, woken by the EventStream's drop, time to end.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let exe = match get_executable_path() {
        Ok((exe, _)) => exe,
        Err(e) => {
            eprintln!("Clay could not start again ({e}); start it again by hand.");
            return;
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The new console may start while the server is still coming back from its own
    // reload: let it wait for the port (run_console_client).
    match std::process::Command::new(&exe).args(&args).env(CONNECT_RETRY_ENV, "1").spawn() {
        Ok(mut child) => {
            let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0);
            std::process::exit(code);
        }
        Err(e) => eprintln!("Clay could not start again ({e}, {}); start it again by hand.", exe.display()),
    }
}

/// Set in the environment of a console relaunched by `/update`: its first connect waits
/// for the server, which may still be restarting, instead of failing at once.
const CONNECT_RETRY_ENV: &str = "CLAY_CONNECT_RETRY";

/// A relaunched console's wait for the server's port (see `CONNECT_RETRY_ENV`): up to
/// this long, probing once a second. Not a reconnect loop - once the port answers, the
/// ordinary connect follows.
async fn wait_for_server_if_relaunched(host_port: &str) {
    if std::env::var_os(CONNECT_RETRY_ENV).is_none() {
        return;
    }
    std::env::remove_var(CONNECT_RETRY_ENV);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut said = false;
    while std::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(host_port).await.is_ok() {
            return;
        }
        if !said {
            println!("Waiting for {} to come back...", host_port);
            said = true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// What this console's own update download reports to its loop (none on Android, which
/// can't update itself).
#[cfg_attr(target_os = "android", allow(dead_code))]
enum OwnUpdate {
    /// A newer release, being downloaded: its version.
    Found(String),
    Done(Result<UpdateSuccess, String>),
}

/// This console's own half of `/update` (src/selfupdate.rs), once the server's is
/// over. Returns true when a download is running (its result comes through
/// `update_tx`); false when there was nothing to download - the server's install
/// already replaced this binary (same machine, same file), so the console restarts
/// at once - or when this platform can't self-update.
fn start_own_update(app: &mut App, update_tx: &mpsc::Sender<OwnUpdate>, force: bool) -> bool {
    if crate::selfupdate::exe_replaced() {
        app.add_output(&crate::selfupdate::replaced_on_disk(OWN_LABEL, &attached_to()));
        relaunch_console(app);
        return false;
    }
    #[cfg(not(target_os = "android"))]
    {
        let update_tx = update_tx.clone();
        tokio::spawn(async move {
            let found_tx = update_tx.clone();
            let result = check_and_download_update(force, move |version| async move {
                let _ = found_tx.send(OwnUpdate::Found(version)).await;
            })
            .await;
            let _ = update_tx.send(OwnUpdate::Done(result)).await;
        });
        true
    }
    #[cfg(target_os = "android")]
    {
        let _ = (update_tx, force);
        app.add_output(&crate::selfupdate::unavailable(OWN_LABEL));
        if app.update_server_reloading {
            relaunch_console(app);
        }
        false
    }
}

/// This console's own download is over: install it and restart attached again. With
/// the server reloading under us the restart happens whatever the result - it is how
/// this console gets its connection back.
fn finish_own_update(app: &mut App, result: Result<UpdateSuccess, String>) {
    let outcome: Result<String, String> = match result {
        #[cfg(not(target_os = "android"))]
        Ok(success) => install_update(&success.temp_path).map(|()| success.version),
        #[cfg(target_os = "android")]
        Ok(success) => Err(format!(
            "update v{} downloaded to {}; replace the binary by hand and restart.",
            success.version, success.temp_path.display()
        )),
        Err(e) => Err(e),
    };
    if let Some(line) = crate::selfupdate::failed(Some(OWN_LABEL), &outcome) {
        app.add_output(&line);
    }
    if outcome.is_ok() || app.update_server_reloading {
        relaunch_console(app);
    }
}

fn remote_mouse_button(b: MouseButton) -> u8 {
    match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

pub(crate) fn handle_remote_client_key(
    app: &mut App,
    key: KeyEvent,
    ws_tx: &mpsc::UnboundedSender<WsMessage>,
) -> bool {
    use KeyCode::*;

    // Import Picture: the separate trailing space of a drop the terminal typed belongs to
    // the drop (src/picture.rs DropSniffer) - before any popup could take it as a press.
    let plain_space = key.code == Char(' ') && !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    if app.drop_sniffer.swallow(plain_space, std::time::Instant::now()) {
        return false;
    }
    app.drop_sniffer.begin_key();

    // The notes editor (/note), as on the console: before popups, so that while it has
    // the focus Ctrl+S saves (here, to the server) rather than pausing output.
    if crate::input_handler::handle_editor_key(app, key).is_some() {
        return false;
    }

    // New unified popup system - handles help popup and others
    if app.has_new_popup() {
        match handle_new_popup_key(app, key) {
            NewPopupAction::Command(cmd) => {
                // A menu item that a key opens (the emoji picker and the like).
                if let Some(action) = popup::definitions::menu::menu_action(&cmd) {
                    return dispatch_remote_action(action, app, ws_tx);
                }
                // Menu command selected: the same dispatcher as a typed line.
                if run_remote_command(app, &cmd, ws_tx, false) {
                    return true;
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
                } else if let (Some(world_index), Some(host), Some(new_fp)) = (
                    data.get(popup::definitions::confirm::CERT_MISMATCH_WORLD_INDEX).and_then(|i| i.parse::<usize>().ok()),
                    data.get(popup::definitions::confirm::CERT_MISMATCH_HOST),
                    data.get(popup::definitions::confirm::CERT_MISMATCH_NEW_FP),
                ) {
                    // A MUD's changed certificate, trusted: the server re-pins it and
                    // connects (the pin is the server's, as the connection is).
                    let _ = ws_tx.send(WsMessage::TrustCertificate { world_index, host: host.clone(), new_fingerprint: new_fp.clone() });
                } else if let Some(world_index) = data.get("world_index").and_then(|i| i.parse::<usize>().ok()) {
                    // A world's deletion, confirmed: the server removes it and tells every
                    // client (WorldRemoved), this one's mirror included.
                    let _ = ws_tx.send(WsMessage::DeleteWorld { world_index });
                } else if data.contains_key(popup::definitions::confirm::PICTURE_UPLOAD) {
                    app.picture_drop_answered(true);
                }
            }
            NewPopupAction::ConfirmCancelled(data) => {
                // Reopen the parent list popup when confirm dialog is cancelled
                if data.contains_key("world_index") {
                    app.open_world_selector_new();
                } else if data.contains_key("action_index") {
                    app.open_actions_list_popup();
                } else if data.contains_key(popup::definitions::confirm::PICTURE_UPLOAD) {
                    app.picture_drop_answered(false);
                }
            }
            // The World Selector's buttons, as the local console's - the worlds are the
            // server's, so connecting, deleting and adding are its to do.
            NewPopupAction::WorldSelector(action) => match action {
                crate::WorldSelectorAction::Connect(name) => {
                    if let Some(idx) = app.find_world(&name) {
                        let previous_idx = app.current_world_index;
                        app.current_world_index = idx;
                        let _ = ws_tx.send(WsMessage::MarkWorldSeen { world_index: idx, previous_world_index: Some(previous_idx) });
                        if !app.worlds[idx].connected {
                            if app.worlds[idx].settings.has_connection_settings() {
                                let _ = ws_tx.send(WsMessage::ConnectWorld { world_index: idx });
                            } else {
                                app.add_output(&format!("World '{}' has no connection settings.", name));
                            }
                        }
                        app.needs_output_redraw = true;
                    }
                }
                crate::WorldSelectorAction::Edit(name) => {
                    if let Some(idx) = app.find_world(&name) {
                        app.open_world_editor_popup_new(idx, false);
                    }
                }
                crate::WorldSelectorAction::Delete(name) => {
                    if let Some(idx) = app.find_world(&name) {
                        if app.worlds.len() > 1 {
                            app.open_delete_world_confirm(&name, idx);
                        } else {
                            app.add_output("Cannot delete the last world.");
                        }
                    }
                }
                // The server creates it; its editor opens on the server's WorldCreated.
                crate::WorldSelectorAction::Add => {
                    let _ = ws_tx.send(WsMessage::CreateWorld { name: format!("World {}", app.worlds.len() + 1) });
                }
            },
            NewPopupAction::WorldSelectorFilter => app.refilter_world_selector(),
            NewPopupAction::SetupSaved(settings) => {
                // Send settings update to master daemon
                // Update local app settings first
                // Debug, as the local console's Setup sets it (it was never sent from here).
                app.settings.debug_enabled = settings.debug;
                crate::DEBUG_ENABLED.store(settings.debug, std::sync::atomic::Ordering::Relaxed);
                app.settings.more_mode_enabled = settings.more_mode;
                app.settings.spell_check_enabled = settings.spell_check;
                app.settings.temp_convert_enabled = settings.temp_convert;
                app.settings.world_switch_mode = WorldSwitchMode::from_name(&settings.world_switching);
                // Note: show_tags is not in setup anymore - controlled by F2 or /tag
                app.input_height = settings.input_height as u16;
                app.input.visible_height = app.input_height;
                app.settings.gui_theme = crate::theme::normalize_theme_name(&settings.gui_theme);
                app.settings.tls_proxy_enabled = settings.tls_proxy;
                if app.settings.dictionary_path != settings.dictionary_path {
                    app.settings.dictionary_path = settings.dictionary_path.clone();
                    app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
                }
                app.settings.editor_side = EditorSide::from_name(&settings.editor_side);
                // Applied here at once; the master keeps it under this console's own key
                // (REMOTE_CONSOLE_REVEAL_KEY) in the UpdateGlobalSettings below.
                app.settings.text_reveal = crate::TextReveal::from_name(&settings.text_reveal);
                // Update mouse setting; if disabled, turn off capture immediately
                app.settings.mouse_enabled = settings.mouse_enabled;
                if !settings.mouse_enabled && app.mouse_capture_active {
                    let _ = execute!(std::io::stdout(), DisableMouseCapture);
                    app.mouse_capture_active = false;
                }
                app.settings.zwj_enabled = settings.zwj_enabled;
                app.settings.ansi_music_enabled = settings.ansi_music;
                app.settings.new_line_indicator = settings.new_line_indicator;
                app.settings.keyboard_always_visible = settings.keyboard_always_visible;
                app.settings.tabs = TabsMode::from_name(&settings.tabs);
                app.settings.icon_bar = IconBarMode::from_name(&settings.icon_bar);
                app.settings.picture_host = crate::picture::PictureService::from_name(&settings.picture_host);
                app.settings.tts_mode = crate::tts::TtsMode::from_name(&settings.tts_mode);
                app.settings.tts_speak_mode = crate::tts::TtsSpeakMode::from_name(&settings.tts_speak_mode);
                app.settings.tts_voice = crate::kokoro::normalize_voice(&settings.tts_voice);
                app.settings.scrollback_enabled = settings.scrollback;
                app.settings.log_input_enabled = settings.log_input;
                app.settings.clear_input_on_send = settings.clear_input_on_send;
                // No local needs_output_redraw here — this is a remote console client; the
                // master (which owns rendering for its own TUI) applies its own redraw when
                // it processes the resulting UpdateGlobalSettings message below.
                app.settings.wrapspace = settings.wrapspace.clamp(0, 20) as u8;

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
                    gui_theme: app.settings.gui_theme.clone(),
                    gui_transparency: app.settings.gui_transparency,
                    color_offset_percent: app.settings.color_offset_percent,
                    // app.settings.wrapspace was already updated from the popup above.
                    wrapspace: app.settings.wrapspace,
                    // remote_initial_lines now lives on the web popup, not setup — this
                    // message always carries the current value regardless of which popup
                    // last changed it.
                    remote_initial_lines: app.settings.remote_initial_lines,
                    input_height: app.input_height,
                    input_height_phone: None,
                    input_height_tablet: None,
                    input_height_desktop: None,
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
                    web_path: app.settings.web_path.clone(),
                    ws_enabled: false,  // Legacy
                    ws_port: 0,        // Legacy
                    ws_cert_file: app.settings.websocket_cert_file.clone(),
                    ws_key_file: app.settings.websocket_key_file.clone(),
                    ws_password: String::new(),  // Empty = unchanged (see apply_remote_web_settings)
                    tls_proxy_enabled: app.settings.tls_proxy_enabled,
                    dictionary_path: app.settings.dictionary_path.clone(),
                    mouse_enabled: app.settings.mouse_enabled,
                    zwj_enabled: app.settings.zwj_enabled,
                    new_line_indicator: app.settings.new_line_indicator,
                    tts_mode: app.settings.tts_mode.name().to_string(),
                    tts_speak_mode: app.settings.tts_speak_mode.name().to_string(),
                    tts_voice: app.settings.tts_voice.clone(),
                    scrollback_enabled: app.settings.scrollback_enabled,
                    log_input_enabled: app.settings.log_input_enabled,
                    clear_input_on_send: Some(app.settings.clear_input_on_send),
                    keyboard_always_visible: app.settings.keyboard_always_visible,
                    tabs: app.settings.tabs.name().to_string(),
                    icon_bar: app.settings.icon_bar.name().to_string(),
                    picture_host: Some(app.settings.picture_host.name()),
                    // This console's own Text Reveal, kept by the server under its key.
                    text_reveal_profile: Some(crate::websocket::TextRevealProfile {
                        key: crate::REMOTE_CONSOLE_REVEAL_KEY.to_string(),
                        value: app.settings.text_reveal.name().to_string(),
                    }),
                    font_profile: None,
                });
            }
            NewPopupAction::WebSaved(settings) => {
                // Check for wildcard '*' in allow list — warn user
                if crate::websocket::allow_list_has_wildcard(&settings.ws_allow_list) {
                    use popup::definitions::confirm::create_allow_list_warning_dialog;
                    let mut def = create_allow_list_warning_dialog();
                    def.custom_data.insert("web_save_remote".to_string(), "1".to_string());
                    def.custom_data.insert("http_enabled".to_string(), settings.http_enabled.to_string());
                    def.custom_data.insert("http_port".to_string(), settings.http_port);
                    def.custom_data.insert("web_path".to_string(), settings.web_path);
                    def.custom_data.insert("ws_password".to_string(), settings.ws_password);
                    def.custom_data.insert("ws_allow_list".to_string(), settings.ws_allow_list);
                    def.custom_data.insert("custom_cert".to_string(), settings.custom_cert.to_string());
                    def.custom_data.insert("ws_cert_file".to_string(), settings.ws_cert_file);
                    def.custom_data.insert("ws_key_file".to_string(), settings.ws_key_file);
                    def.custom_data.insert("remote_initial_lines".to_string(), settings.remote_initial_lines.to_string());
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
                                        pattern: a.display_pattern().to_string(),
                                        enabled: a.enabled,
                                        index: i,
                                    })
                                    .collect();
                                let mut filtered = filter_actions(&all_actions, &filter_text);
                                filtered.sort_by_key(|a| a.name.to_lowercase());
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
                            pattern: a.display_pattern().to_string(),
                            enabled: a.enabled,
                            index: i,
                        })
                        .collect();
                    let mut filtered = filter_actions(&all_actions, &filter_text);
                    filtered.sort_by_key(|a| a.name.to_lowercase());
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
                    // Update world type. A change away from MudTimedPrompt invalidates
                    // any parked timed-prompt promotion, mirroring the same guard in
                    // App::update_world_settings and input_handler.rs's local console
                    // apply - this mirror was previously the one place that never
                    // hydrated world_type at all, silently converting a Slack/Discord
                    // world back to MUD on every remote save.
                    let new_world_type = crate::WorldType::from_name(&settings.world_type);
                    if app.worlds[idx].settings.world_type == crate::WorldType::MudTimedPrompt
                        && new_world_type != crate::WorldType::MudTimedPrompt {
                        app.worlds[idx].timed_prompt_since = None;
                    }
                    app.worlds[idx].settings.world_type = new_world_type;
                    app.worlds[idx].settings.prompt_wait_ms = settings.prompt_wait_ms;
                    app.worlds[idx].settings.tf_type = settings.tf_type.clone();
                    app.worlds[idx].settings.slack_token = settings.slack_token.clone();
                    app.worlds[idx].settings.slack_channel = settings.slack_channel.clone();
                    app.worlds[idx].settings.slack_workspace = settings.slack_workspace.clone();
                    app.worlds[idx].settings.discord_token = settings.discord_token.clone();
                    app.worlds[idx].settings.discord_guild = settings.discord_guild.clone();
                    app.worlds[idx].settings.discord_channel = settings.discord_channel.clone();
                    app.worlds[idx].settings.discord_dm_user = settings.discord_dm_user.clone();
                    app.worlds[idx].settings.discord_channels = settings.discord_channels.clone();
                    app.worlds[idx].settings.slack_app_token = settings.slack_app_token.clone();
                    app.worlds[idx].settings.slack_channels = settings.slack_channels.clone();
                    app.worlds[idx].settings.hostname = settings.hostname.clone();
                    app.worlds[idx].settings.port = settings.port.clone();
                    app.worlds[idx].settings.user = settings.user.clone();
                    app.worlds[idx].settings.password = settings.password.clone();
                    app.worlds[idx].settings.use_ssl = settings.use_ssl;
                    app.worlds[idx].settings.log_enabled = settings.log_enabled;
                    app.worlds[idx].settings.msp_enabled = settings.msp_enabled;
                    app.worlds[idx].settings.mcp_enabled = settings.mcp_enabled;
                    // Toggle a live connection's MCCP2 negotiation before overwriting the
                    // stored value, same as main.rs's WorldEditorSaved handler - the SSH
                    // remote console mirrors settings locally but has no real command_tx
                    // of its own (the real connection lives on the daemon side), so this is
                    // a no-op here in practice; kept for parity with the other two sites.
                    let old_mccp2_enabled = app.worlds[idx].settings.mccp2_enabled;
                    app.worlds[idx].settings.mccp2_enabled = settings.mccp2_enabled;
                    crate::send_mccp2_toggle_if_changed(
                        app.worlds[idx].command_tx.as_ref(),
                        old_mccp2_enabled,
                        settings.mccp2_enabled,
                    );
                    app.worlds[idx].settings.encoding = Encoding::from_name(&settings.encoding);
                    app.worlds[idx].settings.auto_connect_type = AutoConnectType::from_name(&settings.auto_connect);
                    app.worlds[idx].settings.keep_alive_type = KeepAliveType::from_name(&settings.keep_alive);
                    app.worlds[idx].settings.keep_alive_cmd = settings.keep_alive_cmd.clone();
                    app.worlds[idx].settings.gmcp_packages = settings.gmcp_packages.clone();
                    let (ar_secs, ar_on_web) = crate::WorldSettings::parse_auto_reconnect(&settings.auto_reconnect_secs);
                    app.worlds[idx].settings.auto_reconnect_secs = ar_secs;
                    app.worlds[idx].settings.auto_reconnect_on_web = ar_on_web;

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
                        msp_enabled: settings.msp_enabled,
                        mcp_enabled: settings.mcp_enabled,
                        mccp2_enabled: settings.mccp2_enabled,
                        world_type: settings.world_type,
                        prompt_wait_ms: settings.prompt_wait_ms,
                        tf_type: Some(settings.tf_type.clone()),
                        slack_token: settings.slack_token,
                        slack_channel: settings.slack_channel.clone(),
                        slack_workspace: settings.slack_workspace.clone(),
                        discord_token: settings.discord_token,
                        discord_guild: settings.discord_guild.clone(),
                        chat: Some(crate::websocket::ChatSettingsUpdate {
                            discord_guild: settings.discord_guild.clone(),
                            discord_channel: settings.discord_channel.clone(),
                            discord_channels: settings.discord_channels,
                            slack_channel: settings.slack_channel.clone(),
                            slack_channels: settings.slack_channels,
                            slack_workspace: settings.slack_workspace.clone(),
                            slack_app_token: settings.slack_app_token,
                        }),
                        discord_channel: settings.discord_channel,
                        discord_dm_user: settings.discord_dm_user,
                    });
                }
            }
            NewPopupAction::WorldEditorDelete(idx) => {
                // Asked first, as on the local console; the confirm sends DeleteWorld.
                if let Some(name) = app.worlds.get(idx).map(|w| w.name.clone()) {
                    if app.worlds.len() > 1 {
                        app.open_delete_world_confirm(&name, idx);
                    } else {
                        app.add_output("Cannot delete the last world.");
                    }
                }
            }
            NewPopupAction::WorldEditorChatLookup { world_index, world_type, token, app_token, server, request_id } => {
                // Fetch runs on the server (it holds the stored tokens when the editor
                // was sent blank ones); the reply is applied in the receive loop.
                let _ = ws_tx.send(WsMessage::ChatLookup { request_id, world_index, world_type, token, app_token, server });
            }
            NewPopupAction::WorldEditorConnect(idx) => {
                // Send connect request to daemon
                let _ = ws_tx.send(WsMessage::ConnectWorld { world_index: idx });
            }
            // The notes list's Open (/note -l): that world's notes, fresh from the server.
            NewPopupAction::NotesList(crate::NotesListAction::Open(name)) => {
                if let Some(idx) = app.find_world(&name) {
                    app.open_notes_editor(idx);
                }
            }
            NewPopupAction::RecentWorlds(action) => {
                match action {
                    RecentWorldsAction::Switch(name) => {
                        // Route through the daemon's world-switch mechanism.
                        if let Some(idx) = app.find_world(&name) {
                            let _ = ws_tx.send(WsMessage::SwitchWorld { world_index: idx });
                        }
                    }
                    RecentWorldsAction::Close => {}
                }
            }
            NewPopupAction::EmojiFilter => {
                // The emoji table is compiled into this binary too (it's static
                // data, not something the daemon owns), so the SSH remote
                // console can refilter locally exactly like the main console.
                use popup::definitions::emoji::refilter_emoji_console;
                if let Some(state) = app.popup_manager.current_mut() {
                    refilter_emoji_console(state);
                }
            }
            NewPopupAction::InsertText(text) => {
                app.input.insert_str(&text);
                app.popup_manager.close();
            }
            NewPopupAction::ImportSubmit { .. } => {
                // /import's in-process driver (plan i-d-like-to-make-snuggly-rain.md, step 8)
                // is master-console-only for now — this popup is never opened from a
                // --console remote-attach client (Command::Import checks is_master first),
                // so reaching here shouldn't be possible. Kept as a defensive no-op rather
                // than an unreachable!() since a future change to that gate could make it
                // reachable, and silently dropping a submitted import is safer than a panic.
            }
            NewPopupAction::None => {}
        }
        return false;
    }
    if app.search_popup.visible {
        crate::input_handler::handle_search_popup_key(app, key);
        return false;
    }
    if app.filter_popup.visible {
        handle_remote_filter_popup_key(app, key);
        return false;
    }

    // Ctrl+V literal next: insert next character literally
    if app.literal_next {
        app.literal_next = false;
        // Insert the literal character this keypress stands for, including control
        // codes (^V ^A -> 0x01) and Esc (0x1B) — the reason `^V` exists. Previously
        // this matched only `KeyCode::Char` and dropped the modifier, so `^V ^A`
        // inserted a plain "a" and `^V Esc` inserted nothing at all.
        if let Some(c) = crate::input::literal_char(key.code, key.modifiers) {
            app.input.insert_char(c);
        }
        return false;
    }

    // A live shell world (src/shell): the keyboard goes to the PTY over the wire, except
    // the ten world-switch actions. Mirrors input_handler::shell_key_intercept, but sends
    // ShellInput rather than calling the local (absent) session.
    let shell_idx = app.current_world_index;
    if app.shell_live(shell_idx) {
        if key.code == Esc && key.modifiers.is_empty() {
            let _ = ws_tx.send(WsMessage::ShellInput {
                world_index: shell_idx,
                key: Some(crate::websocket::ShellKey { key: "Escape".into(), ..Default::default() }),
                text: None,
                paste: false,
            });
            app.shell_esc_armed_at = Some(std::time::Instant::now());
            return false;
        }
        let esc_armed = app.shell_esc_armed_at.map(|t| t.elapsed() < app.chord_window).unwrap_or(false);
        app.shell_esc_armed_at = None;
        match crate::shell::keys::classify(key.code, key.modifiers, &app.keybindings, esc_armed) {
            crate::shell::keys::Decision::Switch(action) => return dispatch_remote_action(&action, app, ws_tx),
            crate::shell::keys::Decision::Forward(k) => {
                let _ = ws_tx.send(WsMessage::ShellInput { world_index: shell_idx, key: Some(k), text: None, paste: false });
                return false;
            }
            crate::shell::keys::Decision::Text(c) => {
                if c != '\0' {
                    let _ = ws_tx.send(WsMessage::ShellInput { world_index: shell_idx, key: None, text: Some(c.to_string()), paste: false });
                }
                return false;
            }
        }
    }

    // Resolve this keystroke through the shared chord machinery (plan Phase
    // 2 step P2.2). See `input_handler::handle_key_event`'s identical call -
    // `crate::chords::resolve_key_name` is the single lookup both console
    // paths now share, so they can't drift the way Job 17 found this
    // Escape-handling block had (`app.chord`, `crate::chords`).
    let key_name = match crate::chords::resolve_key_name(app, key.code, key.modifiers) {
        crate::chords::KeyResolution::Pending => return false,
        crate::chords::KeyResolution::NotAKey => None,
        crate::chords::KeyResolution::Dispatch(name) => Some(name),
    };

    // Clear history search state on any non-search key
    if let Some(ref name) = key_name {
        if name != "Esc-p" && name != "Esc-n" && name != "Escape" {
            app.input.search_prefix = None;
            app.input.search_index = None;
        }
    }

    // TF-parity plan Job 22c: this client's own `app.tf_engine` never runs macros or
    // receives `/bind`/`/def` definitions locally (typed commands go straight to
    // `WsMessage::SendCommand`, executed by the SERVER's own TF engine) - so the previous
    // `chords::resolve_bound_command(app, name)` check that used to live here was always a
    // structural no-op against an always-empty `app.tf_engine` (Job 22a's own report).
    // Instead, mirror app.js's `tfBoundKeys`: `GlobalSettingsMsg::tf_bound_keys_json`
    // (parsed into `app.tf_bound_keys` by `apply_global_settings`) tells this client which
    // keystrokes the SERVER answers with a `/bind`/`-b`/`-B` or `key_<name>` macro, so it
    // can hand those off via `WsMessage::RunKeyBinding` instead of running a built-in
    // action - the server resolves it the same way a live console keypress would
    // (`WsMessage::RunKeyBinding`'s own handler calls the shared `chords::
    // resolve_bound_command` against ITS OWN real `tf_engine`). Same dispatch order
    // app.js uses: `tf_bound_keys_json` first, then the action table below.
    if let Some(ref name) = key_name {
        if app.tf_bound_keys.contains(name) {
            // Send the raw kbnum (possibly None), not `take_kbnum()`'s defaulted-to-1
            // value - the server mirrors this straight into its own `self.input.kbnum`
            // and lets the bound command's own kbnum-consuming logic (or lack of it)
            // decide what an absent prefix means, exactly like a locally-bound key would.
            let kbnum = app.input.kbnum;
            app.input.clear_kbnum();
            // The input line goes with it, for the binding's kb* functions (TF's keyboard
            // buffer); what they do comes back as InputOps.
            let input = Some(websocket::InputSnapshot { buffer: app.input.buffer.clone(), cursor: app.input.buffer[..app.input.cursor_position.min(app.input.buffer.len())].chars().count() });
            let _ = ws_tx.send(WsMessage::RunKeyBinding { key: name.clone(), kbnum, input });
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
            // ECHO masking (plan Phase 3, step 3.4): this client's own InputArea.history
            // is local, in-memory arrow-key recall - independent of (and in addition to)
            // the server's own record_user_input gate - so it needs the same guard
            // (World::echo_masked is mirrored here via InitialState/EchoMaskChanged; see
            // InputArea::take_input's doc comment).
            let record_history = !app.current_world().protocol.echo_masked;
            let mut cmd = app.take_input_line(record_history);
            // TF's "^old^new" (see InputArea::substitute_history). This client keeps its own
            // history, so it is resolved here, before sending - never at a password prompt,
            // and not in a chat world, where "^^" is just text.
            if !app.current_world().protocol.echo_masked && app.current_world().settings.world_type.is_mud() {
                match app.input.substitute_history(&cmd) {
                    crate::input::HistorySub::NoMatch => {
                        app.add_output("% No match.");
                        return false;
                    }
                    crate::input::HistorySub::Substituted(line) => {
                        app.add_output(&line);
                        cmd = line;
                    }
                    crate::input::HistorySub::NotSubstitution => {}
                }
            }
            // The splash goes on the first user input (as for server data) - before the
            // command runs, or what a command prints here (/version) goes with it.
            clear_splash_on_input(app);
            if cmd.is_empty() {
                // Send empty command to server (some MUDs use this for "look")
                let _ = ws_tx.send(WsMessage::SendCommand {
                    world_index: app.current_world_index,
                    command: cmd,
                });
            } else if run_remote_command(app, &cmd, ws_tx, false) {
                return true;
            }
            // Reset lines_since_pause for more-mode
            app.current_world_mut().lines_since_pause = 0;
            // A command that switched to a world still showing its splash.
            clear_splash_on_input(app);
        }
    // Fall through to character input (unbound keys)
    if let Char(c) = key.code {
        if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            let at = app.input.cursor_position;
            let insert_mode = app.input.insert;
            app.input.insert_char(c);
            app.last_input_was_delete = false;
            // A drop the terminal types arrives a character at a time (Import Picture).
            if insert_mode && !app.settings.picture_host.is_off() {
                app.drop_sniffer.typed(at, c);
            }
            app.check_temp_conversion();
        }
    }
    false
}
/// A command line in the remote console, wherever it came from: typed (Enter), the menu,
/// or the server (`ExecuteLocalCommand`: an action or a macro ran a command whose window is
/// this console's). One dispatcher, as `App::run_note_command` is for `/note`: three copies
/// is how commands went missing on one path. `from_server`: never send it back (the server
/// has already passed on it; sending it again would bounce for ever). Returns true to quit.
pub(crate) fn run_remote_command(app: &mut App, cmd: &str, ws_tx: &mpsc::UnboundedSender<WsMessage>, from_server: bool) -> bool {
    let to_server = |app: &App, command: String| {
        let _ = ws_tx.send(WsMessage::SendCommand { world_index: app.current_world_index, command });
    };
    let has_args = cmd.split_whitespace().nth(1).is_some();
    // TF's /suspend stops this console, as Ctrl+Z does (user ruling 2026-10-16: the remote
    // console suspends itself).
    if cmd.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("/suspend")) && !has_args {
        app.pending_terminal_ops.push_back(crate::TerminalOp::Suspend);
        return false;
    }
    let parsed = parse_command(cmd);
    match parsed {
        Command::Quit => return true,
        // What the console's typed `/help` shows (TF's help text) is the server's to answer,
        // to this console alone. A server from before that bounces it back: the popup.
        Command::Help | Command::HelpTopic { .. } if !from_server => to_server(app, cmd.to_string()),
        Command::Help => app.open_help_popup_new(),
        Command::HelpTopic { ref topic, .. } => {
            use popup::definitions::help::{get_topic_help, create_topic_help_popup, HELP_FIELD_CONTENT};
            if let Some(lines) = get_topic_help(topic) {
                app.popup_manager.open(create_topic_help_popup(lines));
                if let Some(state) = app.popup_manager.current_mut() {
                    state.select_field(HELP_FIELD_CONTENT);
                }
            }
        }
        Command::Version => version_command(app),
        Command::Menu => app.open_menu_popup_new(),
        Command::Font => app.add_output("Font settings are available in the web and GUI interfaces."),
        Command::Setup => app.open_setup_popup_new(),
        Command::Web => app.open_web_popup_new(),
        Command::WorldSelector => app.open_world_selector_new(),
        // `/l -s` and the like: TF's listing (the server's); bare `/l`, this console's.
        Command::WorldsList if has_args && !from_server => to_server(app, cmd.to_string()),
        Command::WorldsList => {
            let _ = ws_tx.send(WsMessage::RequestConnectionsList);
        }
        Command::WorldSwitch { ref name } | Command::WorldConnectNoLogin { ref name } => {
            let no_login = matches!(parsed, Command::WorldConnectNoLogin { .. });
            if let Some(idx) = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                let previous_idx = app.current_world_index;
                app.current_world_index = idx;
                let _ = ws_tx.send(WsMessage::MarkWorldSeen { world_index: idx, previous_world_index: Some(previous_idx) });
                // From the server, it has connected the world already if it could.
                if !from_server && !app.worlds[idx].connected {
                    if no_login {
                        // As a command, so the server sets skip_auto_login.
                        let _ = ws_tx.send(WsMessage::SendCommand { world_index: idx, command: format!("/worlds -l {}", name) });
                    } else {
                        let _ = ws_tx.send(WsMessage::ConnectWorld { world_index: idx });
                    }
                }
            } else {
                app.add_output(&format!("World '{}' not found.", name));
            }
        }
        Command::WorldEdit { ref name } => match name {
            None => {
                let idx = app.current_world_index;
                app.open_world_editor_popup_new(idx, false);
            }
            Some(n) => match app.find_world(n) {
                Some(idx) => app.open_world_editor_popup_new(idx, false),
                // A new world is the server's to create; its editor opens on the
                // server's WorldCreated (as the web and GUI do).
                None => {
                    let _ = ws_tx.send(WsMessage::CreateWorld { name: n.clone() });
                }
            },
        },
        Command::Actions { world } => match world {
            Some(world_name) => app.open_actions_list_popup_with_filter(&world_name),
            None => app.open_actions_list_popup(),
        },
        // Only a server from before `/note <file>` went to the server bounces it back;
        // sending it to it again would just bounce again.
        Command::Edit { filename: Some(_) } if from_server => {
            app.add_output("This server is too old to open its files with /note <file>; update it.");
        }
        // /note, /note -l, /note <file>: the editor here, the notes and files the server's.
        Command::Edit { .. } | Command::EditList => app.run_note_command(&parsed),
        Command::Update { force } if !from_server => {
            // /update updates what can be updated (src/selfupdate.rs): the server first,
            // then - once it has answered (and come back, if it reloaded) - this console's
            // own binary. The answer arrives as UpdateOutcome (App::handle_remote_ws_message).
            if app.update_flow.is_some() || app.pending_update.is_some() {
                app.add_output("An update is already in progress.");
            } else {
                app.update_flow = Some(crate::selfupdate::ClientFlow::WaitingServer { force, since: std::time::Instant::now() });
                to_server(app, (if force { "/update -f" } else { "/update" }).to_string());
            }
        }
        // This console's own binary (the server's /reload is the server's).
        Command::Reload if !from_server => app.pending_reload = true,
        Command::Connect { .. } if !from_server => {
            let _ = ws_tx.send(WsMessage::ConnectWorld { world_index: app.current_world_index });
        }
        Command::RemoteAttach { addr, close, cancel } if !from_server => {
            // A remote client owns no server/worlds of its own, so switching or detaching
            // relaunches immediately - no confirmation needed.
            if cancel {
                app.add_output("No pending /server to cancel.");
            } else if close {
                app.pending_remote_detach = true;
            } else if addr.is_empty() {
                app.add_output("Usage: /server host:port  (or)  /server host port  (or)  /server --close");
            } else {
                app.pending_remote_switch = Some(addr);
            }
        }
        // Import Settings stays the main instance's own (user ruling 2026-10-16).
        Command::Import { .. } => app.add_output(crate::REMOTE_IMPORT_IS_MAIN_ONLY),
        // As the console says it: windows are a web or GUI client's.
        Command::Window { .. } => app.add_output("Use /window from a web or GUI client to open a new window."),
        _ if from_server => {
            let first = cmd.split_whitespace().next().unwrap_or(cmd);
            app.add_output(&format!("% {} isn't available in this console.", first));
        }
        _ => {
            // Web-only editors: show URL hint in console
            if cmd == "/theme-editor" || cmd == "/keybind-editor" {
                let page = cmd.trim_start_matches('/');
                // Localhost is always served plain (see http::route_connection).
                if app.settings.http_enabled {
                    app.add_output(&format!("Open in browser: http://localhost:{}/{}", app.settings.http_port, page));
                } else {
                    app.add_output(&format!("Enable HTTP in /web settings, then open /{} in a browser.", page));
                }
            } else {
                // Everything else (text for the MUD, TF commands, Clay commands the
                // server runs): the server's.
                to_server(app, cmd.to_string());
            }
        }
    }
    false
}

/// Compute the target world index for TF SOCKETB/SOCKETF (`world_socket_prev/next`,
/// TF-parity plan Job 22a, finding 38) - same connected-only, list-order cycling as the
/// master console's own `App::cycle_connected_world`, but read-only: this client has no
/// authority to switch worlds locally (only the server's `SwitchWorld` broadcast actually
/// moves any client's `current_world_index`), so it only computes the target and leaves
/// sending `SwitchWorld` to the caller. `None` if there are no connected worlds, or if the
/// computed target is the world already current (nothing to do).
fn cycle_connected_world_target(app: &App, direction: i32) -> Option<usize> {
    let connected: Vec<usize> = app.worlds.iter().enumerate()
        .filter(|(_, w)| w.connected)
        .map(|(i, _)| i)
        .collect();
    if connected.is_empty() {
        return None;
    }
    let current_pos = connected.iter().position(|&i| i == app.current_world_index);
    let len = connected.len() as i32;
    let target_pos = match current_pos {
        Some(pos) => (((pos as i32 + direction) % len) + len) % len,
        None => 0,
    };
    let target = connected[target_pos as usize];
    if target == app.current_world_index {
        None
    } else {
        Some(target)
    }
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
        // Cursor Movement - kbnum-aware (TF-parity plan Job 20/P2.4): kbnum is purely
        // client-side here (this client has no TF engine execution of its own bound
        // commands - `/bind`-bound keys are forwarded to the server as-is, below), so this
        // only needs to mirror `input_handler::dispatch_action_impl`'s local editing
        // behavior, not the engine-variable round trip that function's doc comment
        // describes for the master console.
        "cursor_left" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.move_cursor_left(); } }
            else { for _ in 0..(-n) { app.input.move_cursor_right(); } }
        }
        "cursor_right" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.move_cursor_right(); } }
            else { for _ in 0..(-n) { app.input.move_cursor_left(); } }
        }
        "cursor_word_left" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.word_left(); } }
            else { for _ in 0..(-n) { app.input.word_right(); } }
        }
        "cursor_word_right" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.word_right(); } }
            else { for _ in 0..(-n) { app.input.word_left(); } }
        }
        "cursor_home" => { app.input.home(); }
        "cursor_end" => { app.input.end(); }
        "cursor_up" => {
            let n = app.input.take_kbnum();
            if n >= 0 {
                for _ in 0..n { if app.input.move_cursor_up() { app.input.history_prev(); } }
            } else {
                for _ in 0..(-n) { if app.input.move_cursor_down() { app.input.history_next(); } }
            }
        }
        "cursor_down" => {
            let n = app.input.take_kbnum();
            if n >= 0 {
                for _ in 0..n { if app.input.move_cursor_down() { app.input.history_next(); } }
            } else {
                for _ in 0..(-n) { if app.input.move_cursor_up() { app.input.history_prev(); } }
            }
        }

        // Editing
        "delete_backward" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.delete_char(); } }
            else { for _ in 0..(-n) { app.input.delete_char_forward(); } }
            app.last_input_was_delete = true;
        }
        "delete_forward" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.delete_char_forward(); } }
            else { for _ in 0..(-n) { app.input.delete_char(); } }
            app.last_input_was_delete = true;
        }
        "delete_word_backward" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.delete_word_before_cursor(); } }
            else { for _ in 0..(-n) { app.input.delete_word_forward(); } }
        }
        "delete_word_forward" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.delete_word_forward(); } }
            else { for _ in 0..(-n) { app.input.delete_word_before_cursor(); } }
        }
        "delete_word_backward_punct" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.backward_kill_word_punctuation(); } }
            else { for _ in 0..(-n) { app.input.delete_word_forward(); } }
        }
        "kill_to_end" => { app.input.kill_to_end(); }
        "clear_line" => { app.input.clear(); }
        "transpose_chars" => {
            let n = app.input.take_kbnum_positive();
            for _ in 0..n { app.input.transpose_chars(); }
        }
        "literal_next" => { app.literal_next = true; }
        "capitalize_word" => {
            let n = app.input.take_kbnum_positive();
            for _ in 0..n { app.input.capitalize_word(); }
        }
        "lowercase_word" => {
            let n = app.input.take_kbnum_positive();
            for _ in 0..n { app.input.lowercase_word(); }
        }
        "uppercase_word" => {
            let n = app.input.take_kbnum_positive();
            for _ in 0..n { app.input.uppercase_word(); }
        }
        "collapse_spaces" => { app.input.collapse_spaces(); }
        "scramble_words" => { app.input.scramble_words(); }
        "goto_matching_bracket" => { app.input.goto_matching_bracket(); }
        "insert_last_arg" => { app.input.last_argument(); }
        "yank" => { app.input.yank(); }
        // TF `Insert`/`Esc-v` and numeric-prefix entry (Job 20/P2.4) - purely client-side,
        // same as the editing actions above.
        "toggle_insert" => { app.input.insert = !app.input.insert; }
        "kbnum_0" => { app.input.kbnum_digit(0); }
        "kbnum_1" => { app.input.kbnum_digit(1); }
        "kbnum_2" => { app.input.kbnum_digit(2); }
        "kbnum_3" => { app.input.kbnum_digit(3); }
        "kbnum_4" => { app.input.kbnum_digit(4); }
        "kbnum_5" => { app.input.kbnum_digit(5); }
        "kbnum_6" => { app.input.kbnum_digit(6); }
        "kbnum_7" => { app.input.kbnum_digit(7); }
        "kbnum_8" => { app.input.kbnum_digit(8); }
        "kbnum_9" => { app.input.kbnum_digit(9); }
        "kbnum_negative" => { app.input.kbnum_negative(); }
        // TF-parity plan Job 19 (P2.3).
        "kill_to_start" => { app.input.kill_to_start(); app.last_input_was_delete = true; }
        "expand_line" => {
            // TF's variables are the server's: it substitutes them and sends the line back
            // (SetInputBuffer). An older server can't; this console's own engine does then.
            let text = app.input.buffer.clone();
            let asked = app.server_expands_line && ws_tx.send(WsMessage::ExpandLine { text: text.clone() }).is_ok();
            if !asked {
                let expanded = crate::tf::variables::substitute_commands(&mut app.tf_engine, &text);
                app.input.buffer = expanded;
                app.input.cursor_position = app.input.buffer.len();
            }
        }
        "completion" => { perform_completion(app); }

        // History
        "history_prev" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.history_prev(); } }
            else { for _ in 0..(-n) { app.input.history_next(); } }
        }
        "history_next" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.history_next(); } }
            else { for _ in 0..(-n) { app.input.history_prev(); } }
        }
        "history_search_backward" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.history_search_backward(); } }
            else { for _ in 0..(-n) { app.input.history_search_forward(); } }
        }
        "history_search_forward" => {
            let n = app.input.take_kbnum();
            if n >= 0 { for _ in 0..n { app.input.history_search_forward(); } }
            else { for _ in 0..(-n) { app.input.history_search_backward(); } }
        }
        // TF RECALLBEG/RECALLEND (Job 19), kbnum-aware since Job 20 - see
        // `InputArea::history_begin_n`'s doc comment.
        "recall_begin" => { let n = app.input.take_kbnum(); app.input.history_begin_n(n); }
        "recall_end" => { let n = app.input.take_kbnum(); app.input.history_end_n(n); }

        // Scrollback, kbnum-aware as the master console's (`input_handler::dispatch_action`):
        // N pages, a negative N the other way. A page up that reaches the top asks the server
        // for as many rows as it moved.
        "scroll_page_up" => {
            let n = app.input.take_kbnum();
            let page = (app.output_height as usize).saturating_sub(2).max(1);
            let rows = page.saturating_mul(n.unsigned_abs() as usize).max(1);
            if n < 0 {
                app.move_viewport_down(rows);
                return false;
            }
            // Row-exact, measured the way render_output_crossterm actually wraps (this client
            // shares that renderer). The old logical-index subtraction ignored wrapping
            // entirely, so a single wrapped paragraph could swallow a whole page.
            app.move_viewport_up(rows);
            if app.viewport_at_top() {
                let before_seq = app.current_world().output_lines.first().map(|l| l.seq);
                // Registered as Backfill (PROTOCOL-ROADMAP.md's seq-drift fix, Step 12b): a
                // scroll-triggered request must never be misrouted into the gap-fill splice
                // path even if pending_gap happens to be set from an unrelated outstanding
                // resync at the same time - the fix this client-side correlator exists for.
                let request_id = app.register_scrollback_request(app.current_world_index, crate::ScrollbackRequestKind::Backfill);
                let _ = ws_tx.send(WsMessage::RequestScrollback {
                    world_index: app.current_world_index,
                    count: rows,
                    before_seq,
                    after_seq: None,
                    request_id: Some(request_id),
                });
            }
            app.needs_output_redraw = true;
        }
        "scroll_page_down" => {
            let n = app.input.take_kbnum();
            if n < 0 {
                app.input.kbnum = Some(-n);
                return dispatch_remote_action("scroll_page_up", app, ws_tx);
            }
            let page = (app.output_height as usize).saturating_sub(2).max(1);
            app.move_viewport_down(page.saturating_mul(n as usize).max(1));
        }
        // TF PAGEBACK: real alias of scroll_page_up, including its backfill request -
        // recurses into this same function rather than duplicating that body.
        "scroll_page_back" => { dispatch_remote_action("scroll_page_up", app, ws_tx); }
        "scroll_half_page" => {
            let half = (app.output_height as usize).saturating_sub(2) / 2;
            let has_pending = !app.current_world().pending_lines.is_empty() || app.current_world().pending_count > 0;
            if app.current_world().paused && has_pending {
                let _ = ws_tx.send(WsMessage::ReleasePending {
                    world_index: app.current_world_index,
                    count: half.max(1),
                });
            } else {
                // Forward = toward newer output (TF HPAGE), same as the console and web.
                app.move_viewport_down(half.max(1));
            }
        }
        // TF HPAGEBACK/LINE/LINEBACK (Job 19) - local viewport moves only, same fidelity
        // level as this client's existing scroll_half_page above (no backfill request for
        // a small move; scroll_page_up/scroll_page_back are the ones that trigger one).
        "scroll_half_page_back" => {
            let half = (app.output_height as usize).saturating_sub(2) / 2;
            app.move_viewport_up(half.max(1));
        }
        "scroll_line_forward" => { app.move_viewport_down(1); }
        "scroll_line_back" => { app.move_viewport_up(1); }
        // TF CLEAR/PAUSE (Job 19).
        "clear_screen" => app.clear_console_view(),
        "pause_output" => { app.current_world_mut().paused = true; }
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
            } else if app.input.buffer.starts_with('/') {
                // Nothing to page: a command is completed, as by the local console's Tab.
                perform_completion(app);
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
        "recent_worlds" => {
            app.open_recent_worlds_popup();
        }
        "world_previous" => {
            let _ = ws_tx.send(WsMessage::CalculatePrevWorld { current_index: app.current_world_index });
        }
        "world_forward" => {
            let _ = ws_tx.send(WsMessage::CalculateNextWorld { current_index: app.current_world_index });
        }
        // TF /bg (= /fg -n): no-op in Clay's single-pane console (Job 19; see
        // dispatch_action's own version for the full rationale).
        "bg_all_worlds" => {}
        // world_socket_prev/world_socket_next (TF SOCKETB/SOCKETF): cycle CONNECTED worlds
        // only, in list order - TF-parity plan Job 22a, finding 38. This client already
        // caches the full world list with each world's `connected` flag kept current by
        // `App::handle_remote_ws_message`'s `WorldConnected`/`WorldDisconnected` handlers, so
        // the target can be computed locally exactly like the master console's own
        // `App::cycle_connected_world` - no new WsMessage needed, just the existing
        // `SwitchWorld` once the target index is known.
        "world_socket_prev" => {
            let n = app.input.take_kbnum() as i32;
            if let Some(idx) = cycle_connected_world_target(app, -n) {
                let _ = ws_tx.send(WsMessage::SwitchWorld { world_index: idx });
            }
        }
        "world_socket_next" => {
            let n = app.input.take_kbnum() as i32;
            if let Some(idx) = cycle_connected_world_target(app, n) {
                let _ = ws_tx.send(WsMessage::SwitchWorld { world_index: idx });
            }
        }

        // System
        "help" => {
            if app.has_new_popup() {
                app.close_new_popup();
            } else {
                app.open_help_popup_new();
            }
        }
        "redraw" | "redraw_server_only" => {
            // TF's redraw clears its input window too, lines left by %cleardone included.
            app.input.clear_sent();
            app.current_world_mut().filter_to_server_output();
            app.needs_terminal_clear = true;
            app.needs_output_redraw = true;
        }
        // TF's real REFRESH: plain repaint, no line filtering. NOT the ^L default - ^L
        // keeps Clay's own filtering redraw (the arm just above), per the user ruling
        // that put ^L alongside ^Q/^R. This action is unbound by default and reachable
        // via /dokey REFRESH or an explicit binding; `dispatch_action`'s
        // `KeyAction::Refresh` is the master console's equivalent.
        "refresh_line" => {
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
        // Stop this console, as the local console stops itself (the loop does it: it has
        // the terminal).
        "suspend" => app.pending_terminal_ops.push_back(crate::TerminalOp::Suspend),
        "bell" => { print!("\x07"); }
        // This console's own dictionary, as the local console's.
        "spell_check" => app.handle_spell_check(),
        "search_popup" => {
            app.search_popup.open();
            app.current_world_mut().search_active = true;
            let output_lines = app.current_world().output_lines.clone();
            let show_tags = app.show_tags;
            app.search_popup.update_search(&output_lines, show_tags);
            app.needs_output_redraw = true;
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
        // TF-parity plan Job 22a (`Esc-L`): same open/close toggle as `filter_popup` above -
        // this client's `FilterPopup` is purely local UI state (finding 33: the real
        // `/limit`/`/unlimit`/`/relimit` pattern-matching semantics are console-only, driven
        // through the TF engine this client doesn't run), so "toggle" here just means
        // show/hide the popup rather than reapplying a remembered pattern.
        "toggle_limit" => {
            // As the local console: off, or the last limit again.
            let op = if app.filter_popup.visible {
                crate::tf::PendingLimitOp::Clear
            } else {
                crate::tf::PendingLimitOp::Reapply
            };
            crate::commands::apply_limit_op(app, op);
        }
        // TF-parity plan Job 22a (`^X^V`): same text as a typed `/version`.
        "show_version" => version_command(app),
        "toggle_action_highlight" => {
            app.highlight_actions = !app.highlight_actions;
            app.needs_output_redraw = true;
        }
        "toggle_gmcp_media" => {
            // The world's media are the server's: it toggles them (and tells every client,
            // GmcpUserToggled). This console's own speech mutes with it, as local F9's does.
            let _ = ws_tx.send(WsMessage::ToggleWorldGmcp { world_index: app.current_world_index });
            app.settings.tts_muted = !app.settings.tts_muted;
            if app.settings.tts_muted {
                crate::tts::stop(&app.tts_backend);
            }
            app.needs_output_redraw = true;
        }
        "input_grow" | "input_shrink" => {
            let rows = if action == "input_grow" { app.input_height + 1 } else { app.input_height.saturating_sub(1) }.clamp(1, 15);
            if rows != app.input_height {
                app.apply_input_height(rows);
                send_global_settings(app, String::new(), ws_tx);
            }
        }
        "emoji_picker" => {
            app.open_emoji_popup();
        }
        _ => {}
    }
    // TF-parity plan Job 20 (P2.4): same auto-clear rule as
    // `input_handler::dispatch_action` - every action other than the numeric-prefix-
    // building ones cancels a pending kbnum, whether or not it consumed one.
    if !is_kbnum_setting_action(action) {
        app.input.clear_kbnum();
    }
    false
}
/// Apply web settings from a remote console client (sends UpdateGlobalSettings to daemon)
pub(crate) fn apply_remote_web_settings(
    app: &mut App,
    settings: &WebSettings,
    ws_tx: &tokio::sync::mpsc::UnboundedSender<crate::websocket::WsMessage>,
) {
    app.settings.http_enabled = settings.http_enabled;
    app.settings.http_port = settings.http_port.parse().unwrap_or(9000);
    app.settings.web_path = crate::sanitize_web_path(&settings.web_path);
    app.settings.websocket_allow_list = settings.ws_allow_list.clone();
    if settings.custom_cert {
        app.settings.websocket_cert_file = settings.ws_cert_file.clone();
        app.settings.websocket_key_file = settings.ws_key_file.clone();
    } else {
        app.settings.websocket_cert_file = String::new();
        app.settings.websocket_key_file = String::new();
    }
    app.settings.remote_initial_lines = settings.remote_initial_lines.clamp(10, 5000) as u16;

    send_global_settings(app, settings.ws_password.clone(), ws_tx);
}

/// Send the daemon every global setting as this client holds it, with `ws_password`
/// (empty = unchanged). The web popup's save, and the input height after Alt-Up/Down or
/// a drag of the separator bar - the SSH console keeps no settings.dat of its own, so a
/// setting it changes is kept by the server it is attached to.
pub(crate) fn send_global_settings(app: &mut App, ws_password: String, ws_tx: &tokio::sync::mpsc::UnboundedSender<crate::websocket::WsMessage>) {

    let _ = ws_tx.send(crate::websocket::WsMessage::UpdateGlobalSettings {
        more_mode_enabled: app.settings.more_mode_enabled,
        spell_check_enabled: app.settings.spell_check_enabled,
        temp_convert_enabled: app.settings.temp_convert_enabled,
        world_switch_mode: app.settings.world_switch_mode.name().to_string(),
        show_tags: app.show_tags,
        debug_enabled: app.settings.debug_enabled,
        ansi_music_enabled: app.settings.ansi_music_enabled,
        console_theme: app.settings.theme.name().to_string(),
        gui_theme: app.settings.gui_theme.clone(),
        gui_transparency: app.settings.gui_transparency,
        color_offset_percent: app.settings.color_offset_percent,
        wrapspace: app.settings.wrapspace,  // unchanged — this function only touches web settings
        // app.settings.remote_initial_lines was already updated from the popup above —
        // Remote Lines now lives on the web popup.
        remote_initial_lines: app.settings.remote_initial_lines,
        input_height: app.input_height,
        input_height_phone: None,
        input_height_tablet: None,
        input_height_desktop: None,
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
        web_path: app.settings.web_path.clone(),
        ws_enabled: false,  // Legacy
        ws_port: 0,         // Legacy
        ws_cert_file: app.settings.websocket_cert_file.clone(),
        ws_key_file: app.settings.websocket_key_file.clone(),
        // Empty = unchanged (the server never sends us its password, so the popup's
        // field starts blank); a value typed into the popup sets a new one.
        ws_password,
        tls_proxy_enabled: app.settings.tls_proxy_enabled,
        dictionary_path: app.settings.dictionary_path.clone(),
        mouse_enabled: app.settings.mouse_enabled,
        zwj_enabled: app.settings.zwj_enabled,
        new_line_indicator: app.settings.new_line_indicator,
        tts_mode: app.settings.tts_mode.name().to_string(),
        tts_speak_mode: app.settings.tts_speak_mode.name().to_string(),
        tts_voice: app.settings.tts_voice.clone(),
        scrollback_enabled: app.settings.scrollback_enabled,
        log_input_enabled: app.settings.log_input_enabled,  // unchanged — this function only touches web settings
        clear_input_on_send: None,
        keyboard_always_visible: app.settings.keyboard_always_visible,
        tabs: app.settings.tabs.name().to_string(),  // unchanged — this function only touches web settings
        icon_bar: app.settings.icon_bar.name().to_string(),  // unchanged — this function only touches web settings
        picture_host: None,
        text_reveal_profile: None,
        font_profile: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal `App` with `connected.len()` worlds named `w0`, `w1`, ... whose
    /// `connected` flags match `connected` in order - enough to exercise
    /// `cycle_connected_world_target` without any real network/WS setup (this remote
    /// client's own `App.worlds`/`connected` cache is kept current in practice by
    /// `App::handle_remote_ws_message`'s `WorldConnected`/`WorldDisconnected` handlers -
    /// see that function's doc comment).
    fn make_test_app_with_worlds(connected: &[bool]) -> App {
        let mut app = App::new();
        app.worlds.clear();
        for (i, &is_connected) in connected.iter().enumerate() {
            let mut w = crate::World::new(&format!("w{i}"));
            w.connected = is_connected;
            app.worlds.push(w);
        }
        app.current_world_index = 0;
        app
    }

    #[test]
    fn test_cycle_connected_world_target_skips_disconnected_worlds() {
        // worlds: 0 connected, 1 DISCONNECTED, 2 connected, 3 connected.
        let mut app = make_test_app_with_worlds(&[true, false, true, true]);
        app.current_world_index = 0;
        assert_eq!(cycle_connected_world_target(&app, 1), Some(2),
            "next connected world after 0 is 2 - 1 must be skipped");
        assert_eq!(cycle_connected_world_target(&app, -1), Some(3),
            "previous connected world from 0 wraps around to the last connected one (3)");
    }

    #[test]
    fn test_cycle_connected_world_target_none_when_only_current_is_connected() {
        let app = make_test_app_with_worlds(&[true, false, false]);
        assert_eq!(cycle_connected_world_target(&app, 1), None,
            "only one connected world exists (the current one) - nothing to cycle to");
    }

    #[test]
    fn test_cycle_connected_world_target_none_when_nothing_connected() {
        let app = make_test_app_with_worlds(&[false, false]);
        assert_eq!(cycle_connected_world_target(&app, 1), None);
    }

    #[test]
    fn test_cycle_connected_world_target_falls_back_to_first_when_current_is_disconnected() {
        // current_world_index (1) is itself disconnected - falls back to the first
        // connected world in list order, mirroring the master's own
        // `App::cycle_connected_world`'s `None => 0` rule.
        let mut app = make_test_app_with_worlds(&[false, false, true, true]);
        app.current_world_index = 1;
        assert_eq!(cycle_connected_world_target(&app, 1), Some(2));
    }

    #[test]
    fn test_cycle_connected_world_target_multi_step_kbnum_direction() {
        // Mirrors the kbnum-as-step-count behavior world_socket_prev/next give this: a
        // direction magnitude greater than 1 steps that many connected worlds at once.
        let app = make_test_app_with_worlds(&[true, true, true, true]);
        assert_eq!(cycle_connected_world_target(&app, 2), Some(2));
        assert_eq!(cycle_connected_world_target(&app, -2), Some(2));
    }

    // ---- TF-parity plan Job 22c: this client's `app.tf_bound_keys` (its local mirror of
    // `GlobalSettingsMsg::tf_bound_keys_json`) must route a matching keypress to
    // `WsMessage::RunKeyBinding` instead of the dead `chords::resolve_bound_command(app,
    // name)` lookup this used to run against its own always-empty `app.tf_engine` (Job
    // 22a's own report). ----

    /// Pull exactly one queued message off `rx` without blocking (this is a plain, non-async
    /// `#[test]`, and `UnboundedSender::send` never needs a runtime).
    fn try_recv(rx: &mut mpsc::UnboundedReceiver<WsMessage>) -> Option<WsMessage> {
        rx.try_recv().ok()
    }

    #[test]
    fn test_server_bound_key_sends_run_key_binding_not_a_local_action() {
        let mut app = make_test_app_with_worlds(&[true]);
        app.tf_bound_keys.insert("^T".to_string());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let quit = handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL), &tx);
        assert!(!quit);
        match try_recv(&mut rx) {
            Some(WsMessage::RunKeyBinding { key, kbnum, .. }) => {
                assert_eq!(key, "^T");
                assert_eq!(kbnum, None, "no kbnum was set, so None should be forwarded as-is");
            }
            other => panic!("expected RunKeyBinding for ^T, got {other:?}"),
        }
    }

    #[test]
    fn test_server_bound_key_takes_priority_over_the_local_action_table() {
        // "Home" defaults to the local "cursor_home" action - but if the SERVER also has a
        // /bind or key_<name> macro on it (tf_bound_keys), that must win, exactly matching
        // app.js's dispatch order (tf_bound_keys_json first, then the action table).
        let mut app = make_test_app_with_worlds(&[true]);
        app.input.buffer = "hello".to_string();
        app.input.cursor_position = 5;
        app.tf_bound_keys.insert("Home".to_string());
        let (tx, mut rx) = mpsc::unbounded_channel();

        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), &tx);

        assert_eq!(app.input.cursor_position, 5,
            "the local cursor_home action must NOT have run - the server-bound path wins");
        match try_recv(&mut rx) {
            Some(WsMessage::RunKeyBinding { key, .. }) => assert_eq!(key, "Home"),
            other => panic!("expected RunKeyBinding for Home, got {other:?}"),
        }
    }

    #[test]
    fn test_non_server_bound_key_still_falls_through_to_the_local_action_table() {
        // Regression guard: a key NOT in tf_bound_keys must behave exactly as before this
        // job - "Home" runs the local cursor_home action, no WsMessage sent for it.
        let mut app = make_test_app_with_worlds(&[true]);
        app.input.buffer = "hello".to_string();
        app.input.cursor_position = 5;
        let (tx, mut rx) = mpsc::unbounded_channel();

        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), &tx);

        assert_eq!(app.input.cursor_position, 0, "plain Home should still move the cursor locally");
        assert!(try_recv(&mut rx).is_none(), "no RunKeyBinding should be sent for an unbound key");
    }

    #[test]
    fn test_server_bound_key_forwards_and_clears_kbnum() {
        let mut app = make_test_app_with_worlds(&[true]);
        app.tf_bound_keys.insert("^T".to_string());
        app.input.kbnum = Some(7);
        let (tx, mut rx) = mpsc::unbounded_channel();

        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL), &tx);

        match try_recv(&mut rx) {
            Some(WsMessage::RunKeyBinding { kbnum, .. }) => assert_eq!(kbnum, Some(7)),
            other => panic!("expected RunKeyBinding carrying kbnum=7, got {other:?}"),
        }
        assert_eq!(app.input.kbnum, None, "kbnum must be consumed (cleared) like any other dispatch");
    }

    #[test]
    fn test_esc_x_bound_only_on_the_server_still_completes_as_a_chord() {
        // Nothing local (no /bind, no action-table entry) recognizes "Esc-x" - only
        // app.tf_bound_keys (the server's own /bind) does. Without folding tf_bound_keys
        // into chords::resolve_key_name's has_binding closure, the second keystroke would
        // abandon the chord (has_binding("Esc-x") false) and resolve to NotAKey instead of
        // completing - this is finding A's chord-prefix-awareness requirement for Job 22c.
        let mut app = make_test_app_with_worlds(&[true]);
        app.tf_bound_keys.insert("Esc-x".to_string());
        let (tx, mut rx) = mpsc::unbounded_channel();

        // Bare Escape always just buffers (hardcoded exception) - no dispatch yet.
        let quit1 = handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &tx);
        assert!(!quit1);
        assert!(try_recv(&mut rx).is_none(), "a bare Escape must never itself dispatch anything");

        // Now 'x': the compound "Esc-x" token must resolve as a real, complete binding.
        let quit2 = handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), &tx);
        assert!(!quit2);
        match try_recv(&mut rx) {
            Some(WsMessage::RunKeyBinding { key, .. }) => assert_eq!(key, "Esc-x"),
            other => panic!("expected RunKeyBinding for Esc-x (server-only binding), got {other:?}"),
        }
    }

    #[test]
    fn test_two_token_chord_bound_only_on_the_server_buffers_then_completes() {
        // A genuine (non-Escape) two-keystroke chord, "^X^Y", registered ONLY in
        // tf_bound_keys - exercises the `is_prefix` half specifically: without folding
        // server_bound_keys into is_prefix too, the lone "^X" keystroke would resolve
        // immediately (nothing else makes it look like a prefix) instead of buffering, and
        // "^X^Y" would never be seen as one sequence.
        let mut app = make_test_app_with_worlds(&[true]);
        app.tf_bound_keys.insert("^X^Y".to_string());
        let (tx, mut rx) = mpsc::unbounded_channel();

        let quit1 = handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL), &tx);
        assert!(!quit1);
        assert!(try_recv(&mut rx).is_none(), "^X alone must only buffer, not dispatch");

        let quit2 = handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL), &tx);
        assert!(!quit2);
        match try_recv(&mut rx) {
            Some(WsMessage::RunKeyBinding { key, .. }) => assert_eq!(key, "^X^Y"),
            other => panic!("expected RunKeyBinding for the completed ^X^Y chord, got {other:?}"),
        }
    }

    #[test]
    fn test_apply_global_settings_populates_tf_bound_keys_from_json() {
        // GlobalSettingsMsg::tf_bound_keys_json arrives both in InitialState and in a live
        // GlobalSettingsUpdated broadcast - drive it through the latter (handle_remote_ws_message),
        // the actual message type a running remote console receives it from.
        let mut app = App::new();
        let mut settings = app.build_global_settings_msg();
        settings.tf_bound_keys_json = r#"["^X","Esc-Left"]"#.to_string();
        app.handle_remote_ws_message(WsMessage::GlobalSettingsUpdated { settings, input_height: 3 });

        assert!(app.tf_bound_keys.contains("^X"));
        assert!(app.tf_bound_keys.contains("Esc-Left"));
        assert_eq!(app.tf_bound_keys.len(), 2);
    }

    #[test]
    fn test_apply_global_settings_clears_tf_bound_keys_when_json_is_empty_array() {
        // An empty `[]` is a real, meaningful state (every /bind was /unbind/purged on the
        // server), not "field absent" - a stale non-empty local set from a previous
        // GlobalSettingsUpdated must be cleared, not left stuck.
        let mut app = App::new();
        app.tf_bound_keys.insert("^X".to_string());
        let mut settings = app.build_global_settings_msg();
        settings.tf_bound_keys_json = "[]".to_string();
        app.handle_remote_ws_message(WsMessage::GlobalSettingsUpdated { settings, input_height: 3 });

        assert!(app.tf_bound_keys.is_empty());
    }

    // ---- /note in the remote console (App::run_note_command) ----

    /// A remote console's App: not the master, its messages on `tx`.
    fn remote_note_app(worlds: usize) -> (App, mpsc::UnboundedSender<WsMessage>, mpsc::UnboundedReceiver<WsMessage>) {
        let mut app = make_test_app_with_worlds(&vec![true; worlds]);
        let (tx, rx) = mpsc::unbounded_channel();
        app.is_master = false;
        app.ws_client_tx = Some(tx.clone());
        app.output_width = 80;
        app.output_height = 24;
        (app, tx, rx)
    }

    fn type_line(app: &mut App, tx: &mpsc::UnboundedSender<WsMessage>, line: &str) {
        for c in line.chars() {
            handle_remote_client_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), tx);
        }
        handle_remote_client_key(app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), tx);
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<WsMessage>) -> Vec<WsMessage> {
        std::iter::from_fn(|| try_recv(rx)).collect()
    }

    fn notes_reply(world_index: usize, notes: &str) -> WsMessage {
        WsMessage::NoteEditorState { world_index, world_name: format!("w{world_index}"), notes: notes.to_string() }
    }

    /// `/note` asks the server for the current world's notes (it used to go to the server
    /// as a command and come back as an ExecuteLocalCommand nobody handled); the answer
    /// opens the editor, keys type into it, and Ctrl+S sends the text back and closes.
    #[test]
    fn note_opens_the_servers_notes_and_ctrl_s_saves_them_back() {
        let (mut app, tx, mut rx) = remote_note_app(2);
        app.current_world_index = 1;
        type_line(&mut app, &tx, "/note");
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::RequestNoteEditorState { world_index: 1 }]), "{sent:?}");
        assert!(!app.editor.visible, "nothing to show until the notes come");

        app.handle_remote_ws_message(notes_reply(1, "old"));
        assert!(app.editor.visible);
        assert_eq!((app.editor.world_index, app.editor.buffer.as_str()), (Some(1), "old"));

        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT), &tx);
        assert_eq!(app.editor.buffer, "oldX", "keys go to the editor");
        assert!(app.input.buffer.is_empty(), "not to the input line");

        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL), &tx);
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::UpdateNote { world_index: 1, notes }] if notes == "oldX"), "{sent:?}");
        assert!(!app.editor.visible, "Ctrl+S saves and closes");
        assert_eq!(app.worlds[1].settings.notes, "oldX", "the mirror keeps the saved text");
    }

    #[test]
    fn note_esc_closes_without_sending_and_a_lost_connection_keeps_the_edits() {
        let (mut app, tx, mut rx) = remote_note_app(1);
        app.run_note_command(&Command::Edit { filename: None });
        app.handle_remote_ws_message(notes_reply(0, "text"));
        drain(&mut rx);
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &tx);
        assert!(!app.editor.visible);
        assert!(drain(&mut rx).is_empty(), "Esc sends nothing");

        app.run_note_command(&Command::Edit { filename: None });
        app.handle_remote_ws_message(notes_reply(0, "text"));
        drop(rx); // the connection's writer is gone
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL), &tx);
        assert!(app.editor.visible, "nothing was sent: the edits stay");
        assert_eq!(app.editor.buffer, "text");
    }

    /// `/note -l` collects every world's notes, then lists the worlds that have some; Open
    /// fetches that world's notes afresh and edits them.
    #[test]
    fn note_list_waits_for_the_worlds_then_open_edits_one() {
        let (mut app, tx, mut rx) = remote_note_app(3);
        type_line(&mut app, &tx, "/note -l");
        let mut asked: Vec<usize> = drain(&mut rx).into_iter().filter_map(|m| match m {
            WsMessage::RequestNoteEditorState { world_index } => Some(world_index),
            _ => None,
        }).collect();
        asked.sort();
        assert_eq!(asked, vec![0, 1, 2]);
        app.handle_remote_ws_message(notes_reply(0, "zero's notes"));
        app.handle_remote_ws_message(notes_reply(1, ""));
        assert!(!app.has_new_popup(), "not until every world has answered");
        app.handle_remote_ws_message(notes_reply(2, "two"));
        assert!(app.has_new_popup(), "the list opens");
        // Enter on the list opens the selected world (the current one, w0).
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::RequestNoteEditorState { world_index: 0 }]), "{sent:?}");
        app.handle_remote_ws_message(notes_reply(0, "zero's notes"));
        assert!(app.editor.visible && app.editor.world_index == Some(0));
    }

    #[test]
    fn note_list_opens_with_what_came_when_the_wait_is_over() {
        let (mut app, tx, mut rx) = remote_note_app(2);
        type_line(&mut app, &tx, "/note -l");
        drain(&mut rx);
        app.handle_remote_ws_message(notes_reply(0, "only this one answered"));
        assert!(!app.has_new_popup());
        if let Some((_, until)) = app.pending_note_list.as_mut() {
            *until = std::time::Instant::now();
        }
        app.expire_notes_list_wait();
        assert!(app.has_new_popup() && app.pending_note_list.is_none());
    }

    /// `/note <file>`: the file is on the main instance's machine (user ruling 2026-10-16).
    /// The server reads it (FileEditorState), Ctrl+S sends it back (SaveFile), and the
    /// editor closes when the server says it is saved (FileSaved); a failed save keeps it.
    #[test]
    fn note_with_a_file_edits_it_on_the_servers_machine() {
        let (mut app, tx, mut rx) = remote_note_app(1);
        type_line(&mut app, &tx, "/note /srv/notes.txt");
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::SendCommand { command, .. }] if command == "/note /srv/notes.txt"), "{sent:?}");
        assert!(!app.editor.visible, "it opens on the server's answer");
        app.handle_remote_ws_message(WsMessage::FileEditorState { path: "/srv/notes.txt".into(), content: "file text".into() });
        assert!(app.editor.visible);
        assert_eq!(app.editor.buffer, "file text");
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE), &tx);
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL), &tx);
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::SaveFile { path, content }] if path == "/srv/notes.txt" && content == "file text!"), "{sent:?}");
        assert!(app.editor.visible, "open until the server says it is saved");
        app.handle_remote_ws_message(WsMessage::FileSaved { path: "/srv/notes.txt".into(), ok: false, error: Some("Permission denied".into()) });
        assert!(app.editor.visible, "a failed save keeps the edits");
        app.handle_remote_ws_message(WsMessage::FileSaved { path: "/srv/notes.txt".into(), ok: true, error: None });
        assert!(!app.editor.visible);
        // An older server bounces `/note <file>` back: said, not sent again.
        app.handle_remote_ws_message(WsMessage::ExecuteLocalCommand { command: "/note /srv/notes.txt".into() });
        assert!(drain(&mut rx).is_empty(), "no ping-pong with an older server");
    }

    /// MCP simpleedit in the remote console: the server's McpEditOpen opens the same editor
    /// the master console opens, Ctrl+S sends McpEditSet back and closes it, and with the
    /// connection gone nothing is sent and the editor keeps the text.
    #[test]
    fn an_mcp_edit_opens_the_editor_and_ctrl_s_sends_it_to_the_server() {
        let (mut app, tx, mut rx) = remote_note_app(2);
        app.handle_remote_ws_message(WsMessage::McpEditOpen {
            world_index: 1, reference: "#12.desc".into(), name: "My Room".into(), edit_type: "string-list".into(), content: "A room.".into(),
        });
        assert!(app.editor.visible);
        assert_eq!(app.editor.buffer, "A room.");
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE), &tx);
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL), &tx);
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::McpEditSet { world_index: 1, reference, edit_type, content }]
            if reference == "#12.desc" && edit_type == "string-list" && content == "A room.!"), "{sent:?}");
        assert!(!app.editor.visible, "Save closes it, as on the master console");

        // A second edit while one is open is refused, not swapped in.
        app.handle_remote_ws_message(WsMessage::McpEditOpen {
            world_index: 0, reference: "a".into(), name: "A".into(), edit_type: "string".into(), content: "first".into(),
        });
        app.handle_remote_ws_message(WsMessage::McpEditOpen {
            world_index: 0, reference: "b".into(), name: "B".into(), edit_type: "string".into(), content: "second".into(),
        });
        assert_eq!(app.editor.buffer, "first");

        // Not connected: nothing sent, the editor stays with the text.
        app.ws_client_tx = None;
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL), &tx);
        assert!(app.editor.visible);
        assert!(drain(&mut rx).is_empty());
    }

    /// A world removed before the MCP edit's world moves its index (McpEditSet is by index);
    /// removing that world closes the editor.
    #[test]
    fn the_mcp_edits_world_moves_with_the_worlds() {
        let (mut app, _tx, _rx) = remote_note_app(3);
        app.handle_remote_ws_message(WsMessage::McpEditOpen {
            world_index: 2, reference: "r".into(), name: "N".into(), edit_type: "string".into(), content: "x".into(),
        });
        app.handle_remote_ws_message(WsMessage::WorldRemoved { world_index: 0 });
        assert_eq!(app.editor.mcp_edit.as_ref().map(|c| c.world_index), Some(1));
        app.handle_remote_ws_message(WsMessage::WorldRemoved { world_index: 1 });
        assert!(!app.editor.visible);
    }

    /// Commands the server runs for this console (an action, a macro) go through the same
    /// dispatcher as typed ones, and never back to the server: /quit ends the console, a
    /// /world switches the view (the server has connected it), /menu opens the menu, an
    /// unknown one is said rather than bounced back.
    #[test]
    fn commands_the_server_runs_here_are_this_consoles() {
        let (mut app, _tx, mut rx) = remote_note_app(2);
        app.handle_remote_ws_message(WsMessage::ExecuteLocalCommand { command: "/world w1".into() });
        assert_eq!(app.current_world_index, 1);
        let sent = drain(&mut rx);
        assert!(!sent.iter().any(|m| matches!(m, WsMessage::ConnectWorld { .. } | WsMessage::SendCommand { .. })), "{sent:?}");
        app.handle_remote_ws_message(WsMessage::ExecuteLocalCommand { command: "/menu".into() });
        assert!(app.has_new_popup());
        app.popup_manager.close();
        app.handle_remote_ws_message(WsMessage::ExecuteLocalCommand { command: "/nonesuch".into() });
        assert!(drain(&mut rx).is_empty(), "never sent back");
        assert!(!app.pending_quit);
        app.handle_remote_ws_message(WsMessage::ExecuteLocalCommand { command: "/quit".into() });
        assert!(app.pending_quit);
    }

    /// The World Selector's Add asks the server for the world; the server's WorldCreated
    /// opens its editor. `/worlds -e <new>` does the same.
    #[test]
    fn new_worlds_are_the_servers_to_create() {
        let (mut app, tx, mut rx) = remote_note_app(1);
        type_line(&mut app, &tx, "/worlds -e brand new");
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::CreateWorld { name }] if name == "brand new"), "{sent:?}");
        assert_eq!(app.worlds.len(), 1, "not created in this console's mirror");
        app.worlds.push(crate::World::new("brand new"));
        app.handle_remote_ws_message(WsMessage::WorldCreated { world_index: 1 });
        assert!(app.has_new_popup(), "the editor opens on the server's answer");
    }

    /// TF's /input, /dokey and /limit run by this console come back as steps on its own
    /// input line and filter.
    #[test]
    fn input_ops_and_limits_from_the_server_act_here() {
        let (mut app, _tx, _rx) = remote_note_app(1);
        use crate::websocket::InputOp;
        app.handle_remote_ws_message(WsMessage::InputOps { ops: vec![
            InputOp::Insert { text: "abc".into(), insert: true },
            InputOp::Goto { pos: 1 },
            InputOp::Delete { count: 1 },
        ] });
        assert_eq!(app.input.buffer, "ac");
        app.handle_remote_ws_message(WsMessage::LimitOp { action: "apply".into(), pattern: "orc".into(), invert: false, attrs_only: false, style: "glob".into() });
        assert!(app.filter_popup.visible);
        assert_eq!(app.filter_popup.filter_text, "orc");
        app.handle_remote_ws_message(WsMessage::LimitOp { action: "clear".into(), pattern: String::new(), invert: false, attrs_only: false, style: String::new() });
        assert!(!app.filter_popup.visible);
    }

    /// The expand_line key asks a server that takes ExpandLine; an older one's line is
    /// expanded here, as before.
    #[test]
    fn expand_line_asks_the_server() {
        let (mut app, tx, mut rx) = remote_note_app(1);
        app.input.buffer = "say %{x}".into();
        app.server_expands_line = true;
        dispatch_remote_action("expand_line", &mut app, &tx);
        assert!(matches!(drain(&mut rx).as_slice(), [WsMessage::ExpandLine { text }] if text == "say %{x}"));
    }

    /// The other ways in - the menu's Notes, and /note bounced back by the server (an
    /// action ran it) - open the editor too.
    #[test]
    fn note_from_the_menu_and_from_the_server_opens_the_editor() {
        let (mut app, tx, mut rx) = remote_note_app(2);
        let menu = popup::definitions::menu::MenuItem::all(false);
        assert!(menu.contains(&popup::definitions::menu::MenuItem::Notes));
        app.handle_remote_ws_message(WsMessage::ExecuteLocalCommand { command: "/note".into() });
        assert!(matches!(drain(&mut rx).as_slice(), [WsMessage::RequestNoteEditorState { world_index: 0 }]));
        app.open_menu_popup_new();
        // Down to Notes (Help, Settings, Web, Actions, World Selector, Recent Worlds, Notes).
        for _ in 0..6 {
            handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &tx);
        }
        handle_remote_client_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &tx);
        let sent = drain(&mut rx);
        assert!(matches!(sent.as_slice(), [WsMessage::RequestNoteEditorState { world_index: 0 }]), "{sent:?}");
    }

    /// The editor's world is an index, and `UpdateNote` goes by index: removing a world
    /// before it moves it down, removing its own world closes it, and a world inserted
    /// before it moves it up.
    #[test]
    fn the_editors_world_moves_with_the_worlds() {
        let (mut app, _tx, _rx) = remote_note_app(4);
        app.run_note_command(&Command::Edit { filename: None });
        app.pending_note_open = Some(2);
        app.handle_remote_ws_message(notes_reply(2, "w2's"));
        assert_eq!(app.editor.world_index, Some(2));
        app.handle_remote_ws_message(WsMessage::WorldRemoved { world_index: 0 });
        assert_eq!(app.editor.world_index, Some(1), "w2 is index 1 now");
        app.shift_note_indices_after_insert(0);
        assert_eq!(app.editor.world_index, Some(2), "a world inserted before it");
        app.handle_remote_ws_message(WsMessage::WorldRemoved { world_index: 2 });
        assert!(!app.editor.visible, "its world is gone: the editor closes");
    }
}
