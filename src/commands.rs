//! Command execution functions extracted from main.rs
//! Includes: handle_command (main command router), API lookups, chat connections,
//! transliteration, and deferred command processing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use async_recursion::async_recursion;
use bytes::BytesMut;
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::{
    App, AppEvent, Command, World, WorldType, SocketFd,
    WsMessage, WriteCommand, StreamReader, StreamWriter,
    Encoding, AutoConnectType,
    parse_command, get_version_string,
    split_action_commands, substitute_action_args, execute_recall,
    format_duration_short, current_timestamp_secs,
    generate_test_music_notes,
    process_telnet, find_safe_split_point,
    get_home_dir, clay_filename,
    local_time_from_epoch,
    VERSION, BUILD_DATE, BUILD_HASH,
    tf, persistence, telnet, util,
    popup,
};

use crate::platform::enable_tcp_keepalive;

#[cfg(all(unix, not(target_os = "android")))]
use crate::platform::{
    get_executable_path, exec_reload, check_and_download_update,
    spawn_tls_proxy,
};

pub(crate) async fn connect_slack(app: &mut App, event_tx: mpsc::Sender<AppEvent>) -> bool {
    // Capture world name for the reader task (stable across world deletions)
    let world_name = app.current_world().name.clone();

    // Clone settings values first to avoid borrow conflicts
    let token = app.current_world().settings.slack_token.clone();
    let channel = app.current_world().settings.slack_channel.clone();

    if token.is_empty() {
        app.add_output("Error: Slack token is required.");
        app.add_output("Configure the token in world settings (/worlds -e)");
        return false;
    }

    app.add_output("");
    app.add_output("Connecting to Slack...");
    app.add_output("");

    // Get WebSocket URL from Slack API
    let client = reqwest::Client::new();
    let response = match client
        .post("https://slack.com/api/apps.connections.open")
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            app.add_output(&format!("Failed to connect to Slack API: {}", e));
            return false;
        }
    };

    let body: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            app.add_output(&format!("Failed to parse Slack response: {}", e));
            return false;
        }
    };

    if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let error = body.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        app.add_output(&format!("Slack API error: {}", error));
        return false;
    }

    let ws_url = match body.get("url").and_then(|v| v.as_str()) {
        Some(u) => u.to_string(),
        None => {
            app.add_output("Slack response missing WebSocket URL");
            return false;
        }
    };

    app.add_output("Got WebSocket URL, connecting...");

    // Connect to Slack WebSocket
    use tokio_tungstenite::tungstenite::Message as WsMsg;

    let (ws_stream, _) = match tokio_tungstenite::connect_async(&ws_url).await {
        Ok(s) => s,
        Err(e) => {
            app.add_output(&format!("Failed to connect to Slack WebSocket: {}", e));
            return false;
        }
    };

    app.add_output("Connected to Slack!");
    app.current_world_mut().connected = true;
    app.current_world_mut().was_connected = true;

    let (write, mut read) = ws_stream.split();
    let write = std::sync::Arc::new(tokio::sync::Mutex::new(write));

    // Create command channel for sending messages
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
    app.current_world_mut().command_tx = Some(cmd_tx);

    // Spawn reader task
    app.current_world_mut().connection_id += 1;
    let reader_conn_id = app.current_world().connection_id;
    let event_tx_clone = event_tx.clone();
    let _token_clone = token.clone(); // Reserved for future use (user lookup, etc.)
    let channel_clone = channel.clone();
    let write_clone = write.clone();

    tokio::spawn(async move {
        use futures::SinkExt;

        while let Some(msg) = read.next().await {
            match msg {
                Ok(WsMsg::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        // Handle envelope acknowledgment
                        if let Some(envelope_id) = json.get("envelope_id").and_then(|v| v.as_str()) {
                            let ack = serde_json::json!({ "envelope_id": envelope_id });
                            let mut w = write_clone.lock().await;
                            let _ = w.send(WsMsg::Text(ack.to_string())).await;
                        }

                        // Handle events
                        if let Some(payload) = json.get("payload") {
                            if let Some(event) = payload.get("event") {
                                let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

                                if event_type == "message" {
                                    // Check if it's the right channel
                                    let msg_channel = event.get("channel").and_then(|v| v.as_str()).unwrap_or("");
                                    if channel_clone.is_empty() || msg_channel == channel_clone || msg_channel.contains(&channel_clone) {
                                        let user = event.get("user").and_then(|v| v.as_str()).unwrap_or("unknown");
                                        let text = event.get("text").and_then(|v| v.as_str()).unwrap_or("");
                                        let formatted = format!("[{}] <{}> {}", msg_channel, user, text);
                                        let _ = event_tx_clone.send(AppEvent::SlackMessage(world_name.clone(), formatted)).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(WsMsg::Close(_)) => {
                    let _ = event_tx_clone.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
                    break;
                }
                Err(_) => {
                    let _ = event_tx_clone.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
                    break;
                }
                _ => {}
            }
        }
    });

    // Spawn writer task for sending messages
    let token_for_writer = token.clone();
    let channel_for_writer = channel.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        while let Some(cmd) = cmd_rx.recv().await {
            let text = match cmd {
                WriteCommand::Text(t) => t,
                WriteCommand::Raw(r) => String::from_utf8_lossy(&r).to_string(),
                WriteCommand::Shutdown => break,
            };
            // Send message via chat.postMessage API
            let _ = client
                .post("https://slack.com/api/chat.postMessage")
                .header("Authorization", format!("Bearer {}", token_for_writer))
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "channel": channel_for_writer,
                    "text": text
                }))
                .send()
                .await;
        }
    });

    false
}

// ============================================================================
// Discord Gateway Bot Connection
// ============================================================================

pub(crate) async fn connect_discord(app: &mut App, event_tx: mpsc::Sender<AppEvent>) -> bool {
    // Capture world name for the reader task (stable across world deletions)
    let world_name = app.current_world().name.clone();

    // Clone settings values first to avoid borrow conflicts
    let token = app.current_world().settings.discord_token.clone();
    let _guild_id = app.current_world().settings.discord_guild.clone();
    let mut channel_id = app.current_world().settings.discord_channel.clone();
    let dm_user = app.current_world().settings.discord_dm_user.clone();

    if token.is_empty() {
        app.add_output("Error: Discord token is required.");
        app.add_output("Configure the token in world settings (/worlds -e)");
        return false;
    }

    app.add_output("");
    app.add_output("Connecting to Discord...");

    // If DM user is set, create a DM channel first
    if !dm_user.is_empty() && channel_id.is_empty() {
        app.add_output(&format!("Creating DM channel with user {}...", dm_user));

        let client = reqwest::Client::new();
        let response = match client
            .post("https://discord.com/api/v10/users/@me/channels")
            .header("Authorization", format!("Bot {}", token))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "recipient_id": dm_user
            }))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                app.add_output(&format!("Failed to create DM channel: {}", e));
                return false;
            }
        };

        let status = response.status();
        let body: serde_json::Value = match response.json().await {
            Ok(j) => j,
            Err(e) => {
                app.add_output(&format!("Failed to parse DM response: {} (HTTP {})", e, status));
                return false;
            }
        };

        if status.is_success() {
            if let Some(id) = body.get("id").and_then(|v| v.as_str()) {
                channel_id = id.to_string();
                app.add_output(&format!("DM channel created: {}", channel_id));
            } else {
                app.add_output("Failed to create DM channel: no channel ID in response");
                app.add_output(&format!("Response: {}", body));
                return false;
            }
        } else {
            let error_msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("unknown error");
            let error_code = body.get("code").and_then(|v| v.as_i64());
            app.add_output(&format!("Failed to create DM channel (HTTP {}): {}", status.as_u16(), error_msg));
            if let Some(code) = error_code {
                app.add_output(&format!("Discord error code: {}", code));
            }
            // Show hint for common errors
            if status.as_u16() == 401 {
                app.add_output("Hint: Check that your bot token is correct and includes 'Bot ' prefix if needed");
            } else if status.as_u16() == 403 {
                app.add_output("Hint: Bot may lack permissions or the user has DMs disabled");
            }
            return false;
        }
    }

    // Resolve channel name to ID if needed
    let guild_id = app.current_world().settings.discord_guild.clone();
    if !channel_id.is_empty() && !channel_id.chars().all(|c| c.is_ascii_digit()) {
        // Channel is a name, not an ID - need to resolve it
        if guild_id.is_empty() {
            app.add_output("Error: Guild ID is required when using a channel name.");
            app.add_output("Either use a numeric channel ID or configure the Guild ID.");
            return false;
        }

        app.add_output(&format!("Resolving channel name '{}'...", channel_id));

        let client = reqwest::Client::new();
        let url = format!("https://discord.com/api/v10/guilds/{}/channels", guild_id);
        let response = match client
            .get(&url)
            .header("Authorization", format!("Bot {}", token))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                app.add_output(&format!("Failed to fetch guild channels: {}", e));
                return false;
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let error_msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("unknown error");
            app.add_output(&format!("Failed to fetch guild channels (HTTP {}): {}", status.as_u16(), error_msg));
            return false;
        }

        let channels: Vec<serde_json::Value> = match response.json().await {
            Ok(c) => c,
            Err(e) => {
                app.add_output(&format!("Failed to parse channels response: {}", e));
                return false;
            }
        };

        // Find channel by name (case-insensitive)
        let channel_name_lower = channel_id.to_lowercase();
        let found_channel = channels.iter().find(|ch| {
            ch.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.to_lowercase() == channel_name_lower)
                .unwrap_or(false)
        });

        match found_channel {
            Some(ch) => {
                if let Some(id) = ch.get("id").and_then(|v| v.as_str()) {
                    app.add_output(&format!("Resolved '{}' to channel ID {}", channel_id, id));
                    channel_id = id.to_string();
                } else {
                    app.add_output(&format!("Channel '{}' found but has no ID", channel_id));
                    return false;
                }
            }
            None => {
                app.add_output(&format!("Channel '{}' not found in guild", channel_id));
                // List available channels as a hint
                let available: Vec<&str> = channels.iter()
                    .filter_map(|ch| ch.get("name").and_then(|n| n.as_str()))
                    .take(10)
                    .collect();
                if !available.is_empty() {
                    app.add_output(&format!("Available channels: {}", available.join(", ")));
                }
                return false;
            }
        }
    }

    app.add_output("");

    // Connect to Discord Gateway
    use tokio_tungstenite::tungstenite::Message as WsMsg;

    let (ws_stream, _) = match tokio_tungstenite::connect_async("wss://gateway.discord.gg/?v=10&encoding=json").await {
        Ok(s) => s,
        Err(e) => {
            app.add_output(&format!("Failed to connect to Discord Gateway: {}", e));
            return false;
        }
    };

    let (write, mut read) = ws_stream.split();
    let write = std::sync::Arc::new(tokio::sync::Mutex::new(write));

    // Create command channel for sending messages
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
    app.current_world_mut().command_tx = Some(cmd_tx);

    // Clone world_name for both spawns before moving into first one
    let world_name_for_writer = world_name.clone();

    // Spawn reader/heartbeat task
    app.current_world_mut().connection_id += 1;
    let reader_conn_id = app.current_world().connection_id;
    let event_tx_clone = event_tx.clone();
    let token_clone = token.clone();
    let channel_id_clone = channel_id.clone();
    let write_clone = write.clone();

    tokio::spawn(async move {
        use futures::SinkExt;

        let mut _heartbeat_interval: Option<u64> = None; // Reserved for dynamic heartbeat
        let mut last_sequence: Option<u64> = None;
        let mut _identified = false; // Track READY state

        while let Some(msg) = read.next().await {
            match msg {
                Ok(WsMsg::Text(text)) => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        let op = json.get("op").and_then(|v| v.as_u64()).unwrap_or(0);

                        // Update sequence number
                        if let Some(s) = json.get("s").and_then(|v| v.as_u64()) {
                            last_sequence = Some(s);
                        }

                        match op {
                            10 => {
                                // Hello - start heartbeat
                                if let Some(d) = json.get("d") {
                                    if let Some(interval) = d.get("heartbeat_interval").and_then(|v| v.as_u64()) {
                                        _heartbeat_interval = Some(interval);

                                        // Send IDENTIFY
                                        let identify = serde_json::json!({
                                            "op": 2,
                                            "d": {
                                                "token": token_clone,
                                                "intents": 2099200, // GUILDS + GUILD_MESSAGES + MESSAGE_CONTENT
                                                "properties": {
                                                    "os": "linux",
                                                    "browser": "clay",
                                                    "device": "clay"
                                                }
                                            }
                                        });
                                        let mut w = write_clone.lock().await;
                                        let _ = w.send(WsMsg::Text(identify.to_string())).await;
                                    }
                                }
                            }
                            0 => {
                                // Dispatch event
                                let event_type = json.get("t").and_then(|v| v.as_str()).unwrap_or("");

                                if event_type == "READY" {
                                    _identified = true;
                                    let _ = event_tx_clone.send(AppEvent::DiscordMessage(world_name.clone(), "Connected to Discord!".to_string())).await;
                                } else if event_type == "MESSAGE_CREATE" {
                                    if let Some(d) = json.get("d") {
                                        let msg_channel = d.get("channel_id").and_then(|v| v.as_str()).unwrap_or("");
                                        if channel_id_clone.is_empty() || msg_channel == channel_id_clone {
                                            let author = d.get("author").and_then(|a| a.get("username")).and_then(|v| v.as_str()).unwrap_or("unknown");
                                            let content = d.get("content").and_then(|v| v.as_str()).unwrap_or("");
                                            let formatted = format!("[#{}] <{}> {}", msg_channel, author, content);
                                            let _ = event_tx_clone.send(AppEvent::DiscordMessage(world_name.clone(), formatted)).await;
                                        }
                                    }
                                }
                            }
                            11 => {
                                // Heartbeat ACK - good, keep going
                            }
                            1 => {
                                // Heartbeat request - send heartbeat
                                let hb = serde_json::json!({
                                    "op": 1,
                                    "d": last_sequence
                                });
                                let mut w = write_clone.lock().await;
                                let _ = w.send(WsMsg::Text(hb.to_string())).await;
                            }
                            _ => {}
                        }
                    }
                }
                Ok(WsMsg::Close(_)) => {
                    let _ = event_tx_clone.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
                    break;
                }
                Err(_) => {
                    let _ = event_tx_clone.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
                    break;
                }
                _ => {}
            }
        }
    });

    // Spawn heartbeat task
    let write_for_heartbeat = write.clone();
    tokio::spawn(async move {
        use futures::SinkExt;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(41)); // ~41s heartbeat
        loop {
            interval.tick().await;
            let hb = serde_json::json!({
                "op": 1,
                "d": null
            });
            let mut w = write_for_heartbeat.lock().await;
            if w.send(WsMsg::Text(hb.to_string())).await.is_err() {
                break;
            }
        }
    });

    // Spawn writer task for sending messages via REST API
    let token_for_writer = token.clone();
    let channel_for_writer = channel_id.clone();
    let event_tx_for_writer = event_tx.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        while let Some(cmd) = cmd_rx.recv().await {
            let text = match cmd {
                WriteCommand::Text(t) => t,
                WriteCommand::Raw(r) => String::from_utf8_lossy(&r).to_string(),
                WriteCommand::Shutdown => break,
            };
            if channel_for_writer.is_empty() {
                let _ = event_tx_for_writer.send(AppEvent::DiscordMessage(world_name_for_writer.clone(), "Error: No channel configured".to_string())).await;
                continue;
            }
            let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_for_writer);
            match client
                .post(&url)
                .header("Authorization", format!("Bot {}", token_for_writer))
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "content": text
                }))
                .send()
                .await
            {
                Ok(response) => {
                    if !response.status().is_success() {
                        let status = response.status();
                        let body: serde_json::Value = response.json().await.unwrap_or_default();
                        let error_msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("unknown error");
                        let msg = format!("Discord send failed (HTTP {}): {}", status.as_u16(), error_msg);
                        let _ = event_tx_for_writer.send(AppEvent::DiscordMessage(world_name_for_writer.clone(), msg)).await;
                    }
                }
                Err(e) => {
                    let msg = format!("Discord send error: {}", e);
                    let _ = event_tx_for_writer.send(AppEvent::DiscordMessage(world_name_for_writer.clone(), msg)).await;
                }
            }
        }
    });

    app.current_world_mut().connected = true;
    app.current_world_mut().was_connected = true;

    false
}

/// Convert UTF-8 characters with diacritics to ASCII equivalents
/// for better compatibility with non-UTF-8 MUD worlds
pub fn transliterate_to_ascii(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // Accented vowels - lowercase
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => result.push('a'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => result.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => result.push('i'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' | 'ø' => result.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => result.push('u'),
            'ý' | 'ÿ' | 'ŷ' => result.push('y'),
            // Accented vowels - uppercase
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => result.push('A'),
            'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => result.push('E'),
            'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'İ' => result.push('I'),
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ō' | 'Ŏ' | 'Ő' | 'Ø' => result.push('O'),
            'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => result.push('U'),
            'Ý' | 'Ÿ' | 'Ŷ' => result.push('Y'),
            // Accented consonants - lowercase
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => result.push('c'),
            'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' => result.push('n'),
            'ś' | 'ŝ' | 'ş' | 'š' => result.push('s'),
            'ź' | 'ż' | 'ž' => result.push('z'),
            'ð' | 'đ' => result.push('d'),
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => result.push('g'),
            'ĥ' | 'ħ' => result.push('h'),
            'ĵ' => result.push('j'),
            'ķ' | 'ĸ' => result.push('k'),
            'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => result.push('l'),
            'ŕ' | 'ŗ' | 'ř' => result.push('r'),
            'ţ' | 'ť' | 'ŧ' => result.push('t'),
            'ŵ' => result.push('w'),
            // Accented consonants - uppercase
            'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => result.push('C'),
            'Ñ' | 'Ń' | 'Ņ' | 'Ň' => result.push('N'),
            'Ś' | 'Ŝ' | 'Ş' | 'Š' => result.push('S'),
            'Ź' | 'Ż' | 'Ž' => result.push('Z'),
            'Ð' | 'Đ' => result.push('D'),
            'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => result.push('G'),
            'Ĥ' | 'Ħ' => result.push('H'),
            'Ĵ' => result.push('J'),
            'Ķ' => result.push('K'),
            'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => result.push('L'),
            'Ŕ' | 'Ŗ' | 'Ř' => result.push('R'),
            'Ţ' | 'Ť' | 'Ŧ' => result.push('T'),
            'Ŵ' => result.push('W'),
            // Ligatures (multi-char replacements)
            'æ' => result.push_str("ae"),
            'Æ' => result.push_str("AE"),
            'œ' => result.push_str("oe"),
            'Œ' => result.push_str("OE"),
            'ß' => result.push_str("ss"),
            'þ' => result.push_str("th"),
            'Þ' => result.push_str("Th"),
            'ĳ' => result.push_str("ij"),
            'Ĳ' => result.push_str("IJ"),
            // Quotes and punctuation (using unicode escapes for curly quotes)
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => result.push('\''),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => result.push('"'),
            '–' | '—' => result.push('-'),
            '…' => result.push_str("..."),
            // Cyrillic - lowercase
            'а' => result.push('a'),
            'б' => result.push('b'),
            'в' => result.push('v'),
            'г' => result.push('g'),
            'д' => result.push('d'),
            'е' | 'э' => result.push('e'),
            'ё' => result.push_str("yo"),
            'ж' => result.push_str("zh"),
            'з' => result.push('z'),
            'и' | 'ы' => result.push('i'),
            'й' => result.push('y'),
            'к' => result.push('k'),
            'л' => result.push('l'),
            'м' => result.push('m'),
            'н' => result.push('n'),
            'о' => result.push('o'),
            'п' => result.push('p'),
            'р' => result.push('r'),
            'с' => result.push('s'),
            'т' => result.push('t'),
            'у' => result.push('u'),
            'ф' => result.push('f'),
            'х' => result.push_str("kh"),
            'ц' => result.push_str("ts"),
            'ч' => result.push_str("ch"),
            'ш' => result.push_str("sh"),
            'щ' => result.push_str("shch"),
            'ъ' => {} // hard sign - omit
            'ь' => {} // soft sign - omit
            'ю' => result.push_str("yu"),
            'я' => result.push_str("ya"),
            // Cyrillic - uppercase
            'А' => result.push('A'),
            'Б' => result.push('B'),
            'В' => result.push('V'),
            'Г' => result.push('G'),
            'Д' => result.push('D'),
            'Е' | 'Э' => result.push('E'),
            'Ё' => result.push_str("Yo"),
            'Ж' => result.push_str("Zh"),
            'З' => result.push('Z'),
            'И' | 'Ы' => result.push('I'),
            'Й' => result.push('Y'),
            'К' => result.push('K'),
            'Л' => result.push('L'),
            'М' => result.push('M'),
            'Н' => result.push('N'),
            'О' => result.push('O'),
            'П' => result.push('P'),
            'Р' => result.push('R'),
            'С' => result.push('S'),
            'Т' => result.push('T'),
            'У' => result.push('U'),
            'Ф' => result.push('F'),
            'Х' => result.push_str("Kh"),
            'Ц' => result.push_str("Ts"),
            'Ч' => result.push_str("Ch"),
            'Ш' => result.push_str("Sh"),
            'Щ' => result.push_str("Shch"),
            'Ъ' => {} // hard sign - omit
            'Ь' => {} // soft sign - omit
            'Ю' => result.push_str("Yu"),
            'Я' => result.push_str("Ya"),
            // Greek - lowercase
            'α' => result.push('a'),
            'β' => result.push('b'),
            'γ' => result.push('g'),
            'δ' => result.push('d'),
            'ε' => result.push('e'),
            'ζ' => result.push('z'),
            'η' => result.push('i'),
            'θ' => result.push_str("th"),
            'ι' => result.push('i'),
            'κ' => result.push('k'),
            'λ' => result.push('l'),
            'μ' => result.push('m'),
            'ν' => result.push('n'),
            'ξ' => result.push('x'),
            'ο' => result.push('o'),
            'π' => result.push('p'),
            'ρ' => result.push('r'),
            'σ' | 'ς' => result.push('s'),
            'τ' => result.push('t'),
            'υ' => result.push('y'),
            'φ' => result.push_str("ph"),
            'χ' => result.push_str("ch"),
            'ψ' => result.push_str("ps"),
            'ω' => result.push('o'),
            // Greek - uppercase
            'Α' => result.push('A'),
            'Β' => result.push('B'),
            'Γ' => result.push('G'),
            'Δ' => result.push('D'),
            'Ε' => result.push('E'),
            'Ζ' => result.push('Z'),
            'Η' => result.push('I'),
            'Θ' => result.push_str("Th"),
            'Ι' => result.push('I'),
            'Κ' => result.push('K'),
            'Λ' => result.push('L'),
            'Μ' => result.push('M'),
            'Ν' => result.push('N'),
            'Ξ' => result.push('X'),
            'Ο' => result.push('O'),
            'Π' => result.push('P'),
            'Ρ' => result.push('R'),
            'Σ' => result.push('S'),
            'Τ' => result.push('T'),
            'Υ' => result.push('Y'),
            'Φ' => result.push_str("Ph"),
            'Χ' => result.push_str("Ch"),
            'Ψ' => result.push_str("Ps"),
            'Ω' => result.push('O'),
            // Everything else passes through unchanged
            _ => result.push(c),
        }
    }
    result
}

/// Look up a word definition from the Free Dictionary API
/// Returns the first definition, with multiple definitions joined by spaces
/// Result is a single line with newlines replaced by spaces
pub async fn lookup_definition(word: &str) -> Result<String, String> {
    let url = format!("https://api.dictionaryapi.dev/api/v2/entries/en/{}", word);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if response.status() == 404 {
        return Err(format!("Word '{}' not found", word));
    }

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Parse the response - it's an array of entries
    let entries = json.as_array()
        .ok_or_else(|| "Invalid response format".to_string())?;

    if entries.is_empty() {
        return Err(format!("No definitions found for '{}'", word));
    }

    // Collect definitions from the first entry
    let mut definitions = Vec::new();

    if let Some(meanings) = entries[0].get("meanings").and_then(|m| m.as_array()) {
        for meaning in meanings {
            if let Some(defs) = meaning.get("definitions").and_then(|d| d.as_array()) {
                for def in defs {
                    if let Some(definition) = def.get("definition").and_then(|d| d.as_str()) {
                        definitions.push(definition.to_string());
                    }
                }
            }
        }
    }

    if definitions.is_empty() {
        return Err(format!("No definitions found for '{}'", word));
    }

    // Join all definitions with spaces and ensure single line
    let result = definitions.join(" ")
        .replace('\n', " ")
        .replace('\r', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    Ok(result)
}

/// Look up a word definition from Urban Dictionary API
/// Returns the first definition only, as a single line
pub async fn lookup_urban_definition(word: &str) -> Result<String, String> {
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("term", word)
        .finish();
    let url = format!("https://api.urbandictionary.com/v0/define?{}", encoded);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Parse the response - it has a "list" array
    let list = json.get("list")
        .and_then(|l| l.as_array())
        .ok_or_else(|| "Invalid response format".to_string())?;

    if list.is_empty() {
        return Err(format!("No definitions found for '{}'", word));
    }

    // Get the first definition only
    let definition = list[0].get("definition")
        .and_then(|d| d.as_str())
        .ok_or_else(|| "No definition text found".to_string())?;

    // Clean up: remove brackets (Urban Dictionary uses [word] for links), ensure single line
    let result = definition
        .replace(['[', ']'], "")
        .replace('\n', " ")
        .replace('\r', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    Ok(result)
}

/// Convert language name or code to ISO 639-1 code (case-insensitive)
pub fn normalize_language_code(lang: &str) -> String {
    let lower = lang.to_lowercase();
    match lower.as_str() {
        // Already a 2-letter code - return as-is
        "en" | "es" | "fr" | "de" | "it" | "pt" | "ru" | "zh" | "ja" | "ko" |
        "ar" | "nl" | "pl" | "sv" | "da" | "no" | "fi" | "cs" | "el" | "he" |
        "hi" | "hu" | "id" | "ms" | "ro" | "sk" | "th" | "tr" | "uk" | "vi" |
        "bg" | "ca" | "hr" | "et" | "lv" | "lt" | "sl" | "sr" | "tl" | "fa" => lower,
        // Full language names
        "english" => "en".to_string(),
        "spanish" | "espanol" | "español" => "es".to_string(),
        "french" | "francais" | "français" => "fr".to_string(),
        "german" | "deutsch" => "de".to_string(),
        "italian" | "italiano" => "it".to_string(),
        "portuguese" | "portugues" | "português" => "pt".to_string(),
        "russian" | "russkiy" | "русский" => "ru".to_string(),
        "chinese" | "mandarin" | "zhongwen" => "zh".to_string(),
        "japanese" | "nihongo" => "ja".to_string(),
        "korean" | "hangugeo" => "ko".to_string(),
        "arabic" => "ar".to_string(),
        "dutch" | "nederlands" => "nl".to_string(),
        "polish" | "polski" => "pl".to_string(),
        "swedish" | "svenska" => "sv".to_string(),
        "danish" | "dansk" => "da".to_string(),
        "norwegian" | "norsk" => "no".to_string(),
        "finnish" | "suomi" => "fi".to_string(),
        "czech" | "cesky" | "čeština" => "cs".to_string(),
        "greek" | "ellinika" => "el".to_string(),
        "hebrew" | "ivrit" => "he".to_string(),
        "hindi" => "hi".to_string(),
        "hungarian" | "magyar" => "hu".to_string(),
        "indonesian" | "bahasa" => "id".to_string(),
        "malay" | "melayu" => "ms".to_string(),
        "romanian" | "romana" | "română" => "ro".to_string(),
        "slovak" | "slovencina" => "sk".to_string(),
        "thai" => "th".to_string(),
        "turkish" | "turkce" | "türkçe" => "tr".to_string(),
        "ukrainian" | "ukrainska" => "uk".to_string(),
        "vietnamese" | "tiengviet" => "vi".to_string(),
        "bulgarian" | "balgarski" => "bg".to_string(),
        "catalan" | "catala" | "català" => "ca".to_string(),
        "croatian" | "hrvatski" => "hr".to_string(),
        "estonian" | "eesti" => "et".to_string(),
        "latvian" | "latviesu" => "lv".to_string(),
        "lithuanian" | "lietuviu" => "lt".to_string(),
        "slovenian" | "slovenscina" => "sl".to_string(),
        "serbian" | "srpski" => "sr".to_string(),
        "tagalog" | "filipino" => "tl".to_string(),
        "persian" | "farsi" => "fa".to_string(),
        // Default: assume it's a code and pass through
        _ => lower,
    }
}

/// Shorten a URL using is.gd (direct redirect, no landing page)
pub async fn lookup_tinyurl(url: &str) -> Result<String, String> {
    let encoded: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("format", "simple")
        .append_pair("url", url)
        .finish();
    let api_url = format!("https://is.gd/create.php?{}", encoded);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()));
    }

    let body = response.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let trimmed = body.trim().to_string();
    if trimmed.is_empty() {
        return Err("Empty response from is.gd".to_string());
    }

    Ok(trimmed)
}

/// Translate text using MyMemory API (free, no API key required for up to 1000 words/day)
pub async fn lookup_translation(text: &str, target_lang: &str) -> Result<String, String> {
    // Normalize language input (accepts both codes and names)
    let lang_code = normalize_language_code(target_lang);

    // MyMemory API uses langpair format: source|target
    // Using "autodetect" as source to auto-detect the input language
    let encoded_text: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", text)
        .append_pair("langpair", &format!("autodetect|{}", lang_code))
        .finish();
    let url = format!("https://api.mymemory.translated.net/get?{}", encoded_text);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API error: {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Check for error response
    if let Some(response_status) = json.get("responseStatus").and_then(|s| s.as_i64()) {
        if response_status != 200 {
            let error_msg = json.get("responseDetails")
                .and_then(|d| d.as_str())
                .unwrap_or("Unknown error");
            return Err(format!("Translation failed: {}", error_msg));
        }
    }

    // Get the translated text
    let translated = json.get("responseData")
        .and_then(|d| d.get("translatedText"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| "No translation found in response".to_string())?;

    // Clean up: ensure single line
    let result = translated
        .replace('\n', " ")
        .replace('\r', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    Ok(result)
}

/// Cap a string to max_len bytes at a valid char boundary.
pub(crate) fn cap_text(s: String, max_len: usize) -> String {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }
}

/// Spawn an async API lookup task for /dict, /urban, /translate, or /tiny commands.
/// Results are sent back via event_tx as ApiLookupResult for the main loop to route to the client.
pub fn spawn_api_lookup(
    event_tx: mpsc::Sender<AppEvent>,
    client_id: u64,
    world_index: usize,
    command: Command,
) {
    match command {
        Command::Dict { word } => {
            tokio::spawn(async move {
                let result = match lookup_definition(&word).await {
                    Ok(def) => {
                        let ascii = transliterate_to_ascii(&def);
                        Ok(cap_text(format!("{}: {}", word, ascii), 1024))
                    }
                    Err(e) => Err(format!("Definition lookup failed: {}", e)),
                };
                let _ = event_tx.send(AppEvent::ApiLookupResult(client_id, world_index, result, true)).await;
            });
        }
        Command::Urban { word } => {
            tokio::spawn(async move {
                let result = match lookup_urban_definition(&word).await {
                    Ok(def) => {
                        let ascii = transliterate_to_ascii(&def);
                        Ok(cap_text(format!("{}: {}", word, ascii), 1024))
                    }
                    Err(e) => Err(format!("Urban Dictionary lookup failed: {}", e)),
                };
                let _ = event_tx.send(AppEvent::ApiLookupResult(client_id, world_index, result, true)).await;
            });
        }
        Command::Translate { lang, text } => {
            tokio::spawn(async move {
                let result = match lookup_translation(&text, &lang).await {
                    Ok(trans) => {
                        let ascii = transliterate_to_ascii(&trans);
                        Ok(cap_text(ascii, 1024))
                    }
                    Err(e) => Err(format!("Translation failed: {}", e)),
                };
                let _ = event_tx.send(AppEvent::ApiLookupResult(client_id, world_index, result, true)).await;
            });
        }
        Command::TinyUrl { url } => {
            tokio::spawn(async move {
                let result = match lookup_tinyurl(&url).await {
                    Ok(short) => Ok(short),
                    Err(e) => Err(format!("TinyURL failed: {}", e)),
                };
                let _ = event_tx.send(AppEvent::ApiLookupResult(client_id, world_index, result, true)).await;
            });
        }
        _ => {}
    }
}

/// Snapshot of a remote client for the /remote ping check
pub(crate) struct RemoteClientSnapshot {
    id: u64,
    ip: String,
    ctype: String,
    connected: String,
    idle: String,
    world: String,
}

/// Start an async /remote ping check: send PingCheck to all clients, wait 2s, build output.
/// `requesting_client_id` is 0 for console, or the WS client_id that invoked /remote.
pub(crate) fn spawn_remote_ping_check(
    app: &mut App,
    event_tx: mpsc::Sender<AppEvent>,
    requesting_client_id: u64,
    world_index: usize,
) {
    let responses = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::<u64>::new()));
    app.remote_ping_nonce += 1;
    let nonce = app.remote_ping_nonce;
    app.remote_ping_responses = Some(responses.clone());

    // Gather client snapshot and send PingCheck to each
    let mut snapshots = Vec::new();
    if let Some(ref ws_server) = app.ws_server {
        if let Ok(clients) = ws_server.clients.try_read() {
            let mut sorted: Vec<_> = clients.iter()
                .filter(|(_, c)| c.authenticated)
                .collect();
            sorted.sort_by_key(|(id, _)| *id);
            for (&id, client) in &sorted {
                let world_name = client.current_world
                    .and_then(|wi| app.worlds.get(wi))
                    .map(|w| w.name.clone())
                    .unwrap_or_else(|| "-".to_string());
                snapshots.push(RemoteClientSnapshot {
                    id,
                    ip: client.ip_address.clone(),
                    ctype: format!("{:?}", client.client_type),
                    connected: format_duration_short(client.connected_at.elapsed()),
                    idle: format_duration_short(client.last_activity.elapsed()),
                    world: world_name,
                });
            }
            // Send PingCheck to each authenticated client
            for (&id, client) in &sorted {
                let _ = client.tx.send(WsMessage::PingCheck { nonce });
                let _ = id;
            }
        }
    }

    // Console line showing which world the console/server is viewing
    let console_world = app.worlds.get(app.current_world_index)
        .map(|w| w.name.clone())
        .unwrap_or_else(|| "-".to_string());

    if snapshots.is_empty() {
        // No remote clients — still show console line
        app.remote_ping_responses = None;
        let lines = vec![
            format!("{:<5} {:<15} {:<7} {:<6} {:<5} {:<5} {}",
                "ID", "IP", "Type", "Conn", "Idle", "Live", "World"),
            format!("{:<5} {:<15} {:<7} {:<6} {:<5} {:<5} {}",
                "-", "localhost", "Console", "-", "-", "Yes", console_world),
        ];
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            let _ = event_tx_clone.send(AppEvent::RemoteListResult(requesting_client_id, world_index, lines)).await;
        });
        return;
    }

    // Spawn a 2-second timer that builds the output
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let responded = responses.lock().unwrap().clone();

        let mut lines = Vec::new();
        lines.push(format!("{:<5} {:<15} {:<7} {:<6} {:<5} {:<5} {}",
            "ID", "IP", "Type", "Conn", "Idle", "Live", "World"));
        lines.push(format!("{:<5} {:<15} {:<7} {:<6} {:<5} {:<5} {}",
            "-", "localhost", "Console", "-", "-", "Yes", console_world));
        for snap in &snapshots {
            let alive = if responded.contains(&snap.id) { "Yes" } else { "No" };
            lines.push(format!("{:<5} {:<15} {:<7} {:<6} {:<5} {:<5} {}",
                snap.id, snap.ip, snap.ctype, snap.connected, snap.idle, alive, snap.world));
        }

        let _ = event_tx.send(AppEvent::RemoteListResult(requesting_client_id, world_index, lines)).await;
    });
}

#[async_recursion(?Send)]
pub(crate) async fn handle_command(cmd: &str, app: &mut App, event_tx: mpsc::Sender<AppEvent>) -> bool {
    let parsed = parse_command(cmd);

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
            } else {
                app.add_output(&format!("No help available for '{}'", topic));
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
        Command::Quit => {
            // Kill all TLS proxy processes before quitting
            for world in &app.worlds {
                #[cfg(unix)]
                if let Some(proxy_pid) = world.proxy_pid {
                    unsafe { libc::kill(proxy_pid as libc::pid_t, libc::SIGTERM); }
                }
                if let Some(ref socket_path) = world.proxy_socket_path {
                    let _ = std::fs::remove_file(socket_path);
                }
            }
            return true; // Signal to quit
        }
        Command::Setup => {
            // Open settings popup for global settings only
            app.open_setup_popup_new();
        }
        Command::Web => {
            // Open web settings popup
            app.open_web_popup_new();
        }
        Command::WorldSelector => {
            // /worlds (no args) - show world selector popup
            app.open_world_selector_new();
        }
        Command::WorldEdit { name } => {
            // /worlds -e or /worlds -e <name>
            let idx = if let Some(ref world_name) = name {
                // /worlds -e <name> - find or create the world, then edit
                app.find_or_create_world(world_name)
            } else {
                // /worlds -e - edit current world
                app.current_world_index
            };
            app.open_world_editor_popup_new(idx);
        }
        Command::WorldConnectNoLogin { name } => {
            // /worlds -l <name> - connect without auto-login
            if let Some(idx) = app.find_world(&name) {
                app.switch_world(idx);
                if !app.current_world().connected {
                    if app.current_world().settings.has_connection_settings() {
                        // Set flag to skip auto-login
                        app.current_world_mut().skip_auto_login = true;
                        return Box::pin(handle_command("/__connect", app, event_tx)).await;
                    } else {
                        app.add_output(&format!("World '{}' has no connection settings.", name));
                    }
                }
            } else {
                app.add_output(&format!("World '{}' not found.", name));
            }
        }
        Command::WorldSwitch { name } => {
            // /worlds <name> - switch to world and connect if not already connected
            if let Some(idx) = app.find_world(&name) {
                // World exists - switch to it
                app.switch_world(idx);
                // Connect if not already connected and has settings
                if !app.current_world().connected
                    && app.current_world().settings.has_connection_settings()
                {
                    return Box::pin(handle_command("/__connect", app, event_tx)).await;
                }
            } else {
                // World doesn't exist - show error message (goes through more-mode flow)
                app.add_output(&format!("World '{}' not found.", name));
            }
        }
        Command::WorldsList => {
            // Output connected worlds list as text
            let current_idx = app.current_world_index;
            const KEEPALIVE_SECS: u64 = 5 * 60;
            let worlds_info: Vec<util::WorldListInfo> = app.worlds.iter().enumerate().map(|(idx, world)| {
                let now = std::time::Instant::now();
                // Compute next NOP using same logic as the actual keepalive timer:
                // max(last_send_time, last_receive_time) determines when activity last happened
                let last_activity_elapsed = match (world.last_send_time, world.last_receive_time) {
                    (Some(s), Some(r)) => Some(s.max(r).elapsed().as_secs()),
                    (Some(s), None) => Some(s.elapsed().as_secs()),
                    (None, Some(r)) => Some(r.elapsed().as_secs()),
                    (None, None) => None,
                };
                let next_nop = if world.connected {
                    last_activity_elapsed.map(|elapsed| KEEPALIVE_SECS.saturating_sub(elapsed))
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
            app.add_output(&output);
        }
        Command::Actions { world } => {
            if let Some(world_name) = world {
                app.open_actions_list_popup_with_filter(&world_name);
            } else {
                app.open_actions_list_popup();
            }
        }
        Command::Connect { host: arg_host, port: arg_port, ssl: arg_ssl } => {
            // Only master client can initiate connections
            if !app.is_master {
                app.add_output("Only the master client can initiate connections.");
                return false;
            }
            if app.current_world().connected {
                app.add_output("Already connected. Use /disconnect first.");
                return false;
            }

            // Route connection based on world type
            let world_type = app.current_world().settings.world_type.clone();
            match world_type {
                WorldType::Slack => {
                    return connect_slack(app, event_tx).await;
                }
                WorldType::Discord => {
                    return connect_discord(app, event_tx).await;
                }
                WorldType::Mud => {
                    // Continue with MUD connection below
                }
            }

            // MUD connection: Determine host/port/ssl: use args if provided, else use stored settings
            let world_settings = &app.current_world().settings;
            let (raw_host, port, use_ssl) = if let (Some(h), Some(p)) = (arg_host, arg_port) {
                (h, p, arg_ssl)
            } else if !world_settings.hostname.is_empty() && !world_settings.port.is_empty() {
                (
                    world_settings.hostname.clone(),
                    world_settings.port.clone(),
                    world_settings.use_ssl,
                )
            } else {
                app.add_output("Configure host/port in world settings (/worlds)");
                return false;
            };

            // Split comma-separated hostnames (primary,fallback)
            let (host, fallback_host) = if let Some(comma) = raw_host.find(',') {
                let primary = raw_host[..comma].trim().to_string();
                let fallback = raw_host[comma+1..].trim().to_string();
                (primary, if fallback.is_empty() { None } else { Some(fallback) })
            } else {
                (raw_host, None)
            };

            // Check if using TLS proxy for connection preservation
            // TLS proxy not available on Android
            #[cfg(all(unix, not(target_os = "android")))]
            let use_tls_proxy = use_ssl && app.settings.tls_proxy_enabled;
            #[cfg(any(target_os = "android", not(unix)))]
            let use_tls_proxy = false;

            let ssl_msg = if use_ssl {
                if use_tls_proxy { " with SSL (via proxy)" } else { " with SSL" }
            } else { "" };
            app.add_output("");
            app.add_output(&format!("Connecting to {}:{}{}...", host, port, ssl_msg));
            app.add_output("");

            // Handle TLS proxy case separately (proxy does its own TCP connect)
            // TLS proxy only available on Unix (not Android or Windows)
            #[cfg(all(unix, not(target_os = "android")))]
            if use_tls_proxy {
                let world_name = app.current_world().name.clone();
                match spawn_tls_proxy(&world_name, &host, &port) {
                    Ok((proxy_pid, socket_path)) => {
                        // Connect to the proxy via Unix socket
                        match tokio::net::UnixStream::connect(&socket_path).await {
                            Ok(unix_stream) => {
                                // Store the Unix socket FD for hot reload preservation
                                #[cfg(unix)]
                                {
                                    use std::os::unix::io::AsRawFd;
                                    app.current_world_mut().proxy_socket_fd = Some(unix_stream.as_raw_fd());
                                }
                                app.current_world_mut().socket_fd = None;  // Can't preserve TLS fd directly
                                app.current_world_mut().is_tls = true;
                                app.current_world_mut().proxy_pid = Some(proxy_pid);
                                app.current_world_mut().proxy_socket_path = Some(socket_path);

                                let (r, w) = unix_stream.into_split();
                                let mut read_half = StreamReader::Proxy(r);
                                let mut write_half = StreamWriter::Proxy(w);

                                app.current_world_mut().connected = true;
                                app.current_world_mut().was_connected = true;
                                app.current_world_mut().prompt_count = 0;
                                let now = std::time::Instant::now();
                                app.current_world_mut().last_send_time = Some(now);
                                app.current_world_mut().last_receive_time = Some(now);
                                app.current_world_mut().is_initial_world = false;
                                app.discard_initial_world();

                                let world_name = app.current_world().name.clone();

                                // Open log file if enabled
                                if app.current_world().settings.log_enabled {
                                    if app.current_world_mut().open_log_file() {
                                        let log_path = app.current_world().get_log_path();
                                        app.add_output(&format!("Logging to: {}", log_path.display()));
                                    } else {
                                        app.add_output("Warning: Could not open log file");
                                    }
                                }

                                // Setup writer channel (before reader task so telnet_tx is available)
                                let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                                app.current_world_mut().command_tx = Some(cmd_tx.clone());

                                // Fire TF CONNECT hook
                                let hook_result = tf::bridge::fire_event(&mut app.tf_engine, tf::TfHookEvent::Connect);
                                for cmd in hook_result.send_commands {
                                    let _ = cmd_tx.try_send(WriteCommand::Text(cmd));
                                }
                                for cmd in hook_result.clay_commands {
                                    let _ = app.tf_engine.execute(&cmd);
                                }

                                // Send auto-login if configured (for Connect type)
                                // Requires BOTH username AND password to be set
                                let skip_login = app.current_world().skip_auto_login;
                                let auto_connect_type = app.current_world().settings.auto_connect_type;
                                // Only clear skip flag for Connect type; Prompt/MooPrompt check it later in handle_prompt
                                if auto_connect_type == AutoConnectType::Connect {
                                    app.current_world_mut().skip_auto_login = false;
                                }
                                let user = app.current_world().settings.user.clone();
                                let password = app.current_world().settings.password.clone();
                                // FANSI worlds: always set up client detection window
                                if app.current_world().settings.encoding == Encoding::Fansi {
                                    app.current_world_mut().fansi_detect_until = Some(std::time::Instant::now() + Duration::from_secs(2));
                                    if !skip_login && !user.is_empty() && !password.is_empty() && auto_connect_type == AutoConnectType::Connect {
                                        let connect_cmd = format!("connect {} {}", user, password);
                                        app.current_world_mut().fansi_login_pending = Some(connect_cmd);
                                    }
                                } else if !skip_login && !user.is_empty() && !password.is_empty() && auto_connect_type == AutoConnectType::Connect {
                                    let connect_cmd = format!("connect {} {}", user, password);
                                    let _ = cmd_tx.send(WriteCommand::Text(connect_cmd)).await;
                                }

                                // Start reader task with telnet processing
                                app.current_world_mut().connection_id += 1;
                                let reader_conn_id = app.current_world().connection_id;
                                let event_tx_read = event_tx.clone();
                                let read_world_name = world_name.clone();
                                let telnet_tx = cmd_tx;
                                tokio::spawn(async move {
                                    let mut buf = [0u8; 4096];
                                    let mut line_buffer = Vec::new();
                                    loop {
                                        match tokio::io::AsyncReadExt::read(&mut read_half, &mut buf).await {
                                            Ok(0) => {
                                                // Send any remaining buffered data
                                                if !line_buffer.is_empty() {
                                                    let result = process_telnet(&line_buffer);
                                                    if !result.responses.is_empty() {
                                                        let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                                    }
                                                    if result.telnet_detected {
                                                        let _ = event_tx_read.send(AppEvent::TelnetDetected(read_world_name.clone())).await;
                                                    }
                                                    if result.gmcp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::GmcpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    if result.msdp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::MsdpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    for (pkg, json) in &result.gmcp_data {
                                                        let _ = event_tx_read.send(AppEvent::GmcpReceived(read_world_name.clone(), pkg.clone(), json.clone())).await;
                                                    }
                                                    for (var, val) in &result.msdp_data {
                                                        let _ = event_tx_read.send(AppEvent::MsdpReceived(read_world_name.clone(), var.clone(), val.clone())).await;
                                                    }
                                                    if let Some(prompt_bytes) = result.prompt {
                                                        let _ = event_tx_read.send(AppEvent::Prompt(read_world_name.clone(), prompt_bytes)).await;
                                                    }
                                                    if !result.cleaned.is_empty() {
                                                        let _ = event_tx_read.send(AppEvent::ServerData(read_world_name.clone(), result.cleaned)).await;
                                                    }
                                                }
                                                let _ = event_tx_read.send(AppEvent::Disconnected(read_world_name.clone(), reader_conn_id)).await;
                                                break;
                                            }
                                            Ok(n) => {
                                                line_buffer.extend_from_slice(&buf[..n]);
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
                                                        let _ = event_tx_read.send(AppEvent::TelnetDetected(read_world_name.clone())).await;
                                                    }
                                                    if result.naws_requested {
                                                        let _ = event_tx_read.send(AppEvent::NawsRequested(read_world_name.clone())).await;
                                                    }
                                                    if result.ttype_requested {
                                                        let _ = event_tx_read.send(AppEvent::TtypeRequested(read_world_name.clone())).await;
                                                    }
                                                    if let Some(ref charsets) = result.charset_request {
                                                        let _ = event_tx_read.send(AppEvent::CharsetRequested(read_world_name.clone(), charsets.clone())).await;
                                                    }
                                                    if result.gmcp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::GmcpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    if result.msdp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::MsdpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    for (pkg, json) in &result.gmcp_data {
                                                        let _ = event_tx_read.send(AppEvent::GmcpReceived(read_world_name.clone(), pkg.clone(), json.clone())).await;
                                                    }
                                                    for (var, val) in &result.msdp_data {
                                                        let _ = event_tx_read.send(AppEvent::MsdpReceived(read_world_name.clone(), var.clone(), val.clone())).await;
                                                    }
                                                    if let Some(prompt_bytes) = result.prompt {
                                                        let _ = event_tx_read.send(AppEvent::Prompt(read_world_name.clone(), prompt_bytes)).await;
                                                    }
                                                    if !result.cleaned.is_empty() {
                                                        let _ = event_tx_read.send(AppEvent::ServerData(read_world_name.clone(), result.cleaned)).await;
                                                    }
                                                }
                                            }
                                            Err(_) => {
                                                let _ = event_tx_read.send(AppEvent::Disconnected(read_world_name.clone(), reader_conn_id)).await;
                                                break;
                                            }
                                        }
                                    }
                                });

                                // Spawn writer task
                                tokio::spawn(async move {
                                    while let Some(cmd) = cmd_rx.recv().await {
                                        let bytes = match &cmd {
                                            WriteCommand::Text(text) => {
                                                let mut b = text.as_bytes().to_vec();
                                                b.extend_from_slice(b"\r\n");
                                                b
                                            }
                                            WriteCommand::Raw(raw) => raw.clone(),
                                            WriteCommand::Shutdown => break,
                                        };
                                        if tokio::io::AsyncWriteExt::write_all(&mut write_half, &bytes).await.is_err() {
                                            break;
                                        }
                                    }
                                });

                                // Connection established successfully via proxy, skip regular connection code
                                return false;
                            }
                            Err(e) => {
                                app.add_output(&format!("Failed to connect to TLS proxy: {}", e));
                                // Kill the proxy process
                                #[cfg(unix)]
                                unsafe { libc::kill(proxy_pid as libc::pid_t, libc::SIGTERM); }
                                return false;
                            }
                        }
                    }
                    Err(e) => {
                        app.add_output(&format!("Failed to spawn TLS proxy: {}", e));
                        app.add_output("Falling back to direct TLS connection...");
                        // Fall through to direct TLS connection below
                    }
                }
            }

            // Spawn connection in background to avoid blocking the UI
            app.current_world_mut().connection_id += 1;
            let reader_conn_id = app.current_world().connection_id;
            let world_name = app.current_world().name.clone();
            let connect_host = host.clone();
            let connect_fallback = fallback_host.clone();
            let connect_port = port.clone();
            let connect_use_ssl = use_ssl;
            let event_tx_connect = event_tx.clone();

            tokio::spawn(async move {
                // Try connecting to a host, resolving DNS and preferring IPv4
                async fn try_connect_host(host: &str, port: &str) -> Result<TcpStream, std::io::Error> {
                    use tokio::net::lookup_host;
                    let addrs: Vec<std::net::SocketAddr> = lookup_host(
                        format!("{}:{}", host, port)
                    ).await?.collect();
                    if addrs.is_empty() {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::AddrNotAvailable,
                            format!("Could not resolve {}", host),
                        ));
                    }
                    let mut sorted = addrs.clone();
                    sorted.sort_by_key(|a| if a.is_ipv4() { 0 } else { 1 });
                    let mut last_err = None;
                    let mut ipv4_err = None;
                    for addr in &sorted {
                        match TcpStream::connect(addr).await {
                            Ok(stream) => return Ok(stream),
                            Err(e) => {
                                if addr.is_ipv4() && ipv4_err.is_none() {
                                    ipv4_err = Some(e);
                                } else {
                                    last_err = Some(e);
                                }
                            }
                        }
                    }
                    Err(ipv4_err.or(last_err).unwrap_or_else(|| {
                        std::io::Error::other("Connection failed")
                    }))
                }

                // Try primary host, fall back to secondary if configured
                let connect_result = match try_connect_host(&connect_host, &connect_port).await {
                    Ok(stream) => Ok(stream),
                    Err(primary_err) => {
                        if let Some(ref fallback) = connect_fallback {
                            let _ = event_tx_connect.send(AppEvent::ServerData(
                                world_name.clone(),
                                format!("Primary host {} failed: {}, trying {}...\r\n", connect_host, primary_err, fallback).into_bytes(),
                            )).await;
                            try_connect_host(fallback, &connect_port).await
                        } else {
                            Err(primary_err)
                        }
                    }
                };
                match connect_result {
                    Ok(tcp_stream) => {
                        // Store the socket fd/handle for hot reload (before splitting)
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
                        let connection_result: Result<(StreamReader, StreamWriter, bool), String> = if connect_use_ssl {
                            #[cfg(feature = "native-tls-backend")]
                            {
                                let connector = match native_tls::TlsConnector::builder()
                                    .danger_accept_invalid_certs(true)
                                    .build()
                                {
                                    Ok(c) => c,
                                    Err(e) => {
                                        let _ = event_tx_connect.send(AppEvent::ConnectionFailed(
                                            world_name.clone(),
                                            format!("TLS error: {}", e)
                                        )).await;
                                        return;
                                    }
                                };
                                let connector = tokio_native_tls::TlsConnector::from(connector);

                                match connector.connect(&connect_host, tcp_stream).await {
                                    Ok(tls_stream) => {
                                        let (r, w) = tokio::io::split(tls_stream);
                                        Ok((StreamReader::Tls(r), StreamWriter::Tls(w), true))
                                    }
                                    Err(e) => {
                                        Err(format!("SSL handshake failed: {}", e))
                                    }
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
                                    .with_custom_certificate_verifier(Arc::new(crate::platform::danger::NoCertificateVerification::new()))
                                    .with_no_client_auth();

                                let connector = TlsConnector::from(Arc::new(config));
                                let server_name = match ServerName::try_from(connect_host.clone()) {
                                    Ok(sn) => sn,
                                    Err(e) => {
                                        let _ = event_tx_connect.send(AppEvent::ConnectionFailed(
                                            world_name.clone(),
                                            format!("Invalid server name: {}", e)
                                        )).await;
                                        return;
                                    }
                                };

                                match connector.connect(server_name, tcp_stream).await {
                                    Ok(tls_stream) => {
                                        let (r, w) = tokio::io::split(tls_stream);
                                        Ok((StreamReader::Tls(r), StreamWriter::Tls(w), true))
                                    }
                                    Err(e) => {
                                        Err(format!("SSL handshake failed: {}", e))
                                    }
                                }
                            }

                            #[cfg(not(any(feature = "native-tls-backend", feature = "rustls-backend")))]
                            {
                                Err("No TLS backend available".to_string())
                            }
                        } else {
                            let (r, w) = tcp_stream.into_split();
                            Ok((StreamReader::Plain(r), StreamWriter::Plain(w), false))
                        };

                        match connection_result {
                            Ok((mut read_half, mut write_half, is_tls)) => {
                                // Create command channel
                                let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);

                                // Clone for reader task
                                let telnet_tx = cmd_tx.clone();
                                let event_tx_read = event_tx_connect.clone();
                                let read_world_name = world_name.clone();

                                // Spawn reader task
                                tokio::spawn(async move {
                                    let mut buffer = BytesMut::with_capacity(4096);
                                    buffer.resize(4096, 0);
                                    let mut line_buffer: Vec<u8> = Vec::new();
                                    let mut mccp2: Option<flate2::Decompress> = None;

                                    loop {
                                        match read_half.read(&mut buffer).await {
                                            Ok(0) => {
                                                if !line_buffer.is_empty() {
                                                    let result = process_telnet(&line_buffer);
                                                    if !result.responses.is_empty() {
                                                        let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                                    }
                                                    if result.telnet_detected {
                                                        let _ = event_tx_read.send(AppEvent::TelnetDetected(read_world_name.clone())).await;
                                                    }
                                                    if result.gmcp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::GmcpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    if result.msdp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::MsdpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    for (pkg, json) in &result.gmcp_data {
                                                        let _ = event_tx_read.send(AppEvent::GmcpReceived(read_world_name.clone(), pkg.clone(), json.clone())).await;
                                                    }
                                                    for (var, val) in &result.msdp_data {
                                                        let _ = event_tx_read.send(AppEvent::MsdpReceived(read_world_name.clone(), var.clone(), val.clone())).await;
                                                    }
                                                    if let Some(prompt_bytes) = result.prompt {
                                                        let _ = event_tx_read.send(AppEvent::Prompt(read_world_name.clone(), prompt_bytes)).await;
                                                    }
                                                    if !result.cleaned.is_empty() {
                                                        let _ = event_tx_read.send(AppEvent::ServerData(read_world_name.clone(), result.cleaned)).await;
                                                    }
                                                }
                                                let _ = event_tx_read
                                                    .send(AppEvent::ServerData(
                                                        read_world_name.clone(),
                                                        "Connection closed by server.\n".as_bytes().to_vec(),
                                                    ))
                                                    .await;
                                                let _ = event_tx_read.send(AppEvent::Disconnected(read_world_name.clone(), reader_conn_id)).await;
                                                break;
                                            }
                                            Ok(n) => {
                                                if let Some(ref mut decomp) = mccp2 {
                                                    let decompressed = telnet::mccp2_decompress(decomp, &buffer[..n]);
                                                    line_buffer.extend_from_slice(&decompressed);
                                                } else {
                                                    line_buffer.extend_from_slice(&buffer[..n]);
                                                }
                                                let split_at = find_safe_split_point(&line_buffer);
                                                let to_send: Vec<u8> = if split_at > 0 {
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
                                                    if result.mccp2_activated {
                                                        let mut decomp = flate2::Decompress::new(true);
                                                        if result.mccp2_offset < to_send.len() {
                                                            let tail = telnet::mccp2_decompress(&mut decomp, &to_send[result.mccp2_offset..]);
                                                            let mut new_buf = tail;
                                                            new_buf.append(&mut line_buffer);
                                                            line_buffer = new_buf;
                                                        }
                                                        mccp2 = Some(decomp);
                                                    }
                                                    if result.telnet_detected {
                                                        let _ = event_tx_read.send(AppEvent::TelnetDetected(read_world_name.clone())).await;
                                                    }
                                                    if result.naws_requested {
                                                        let _ = event_tx_read.send(AppEvent::NawsRequested(read_world_name.clone())).await;
                                                    }
                                                    if result.ttype_requested {
                                                        let _ = event_tx_read.send(AppEvent::TtypeRequested(read_world_name.clone())).await;
                                                    }
                                                    if let Some(ref charsets) = result.charset_request {
                                                        let _ = event_tx_read.send(AppEvent::CharsetRequested(read_world_name.clone(), charsets.clone())).await;
                                                    }
                                                    if result.wont_echo_seen {
                                                        let _ = event_tx_read.send(AppEvent::WontEchoSeen(read_world_name.clone())).await;
                                                    }
                                                    if result.gmcp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::GmcpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    if result.msdp_negotiated {
                                                        let _ = event_tx_read.send(AppEvent::MsdpNegotiated(read_world_name.clone())).await;
                                                    }
                                                    for (pkg, json) in &result.gmcp_data {
                                                        let _ = event_tx_read.send(AppEvent::GmcpReceived(read_world_name.clone(), pkg.clone(), json.clone())).await;
                                                    }
                                                    for (var, val) in &result.msdp_data {
                                                        let _ = event_tx_read.send(AppEvent::MsdpReceived(read_world_name.clone(), var.clone(), val.clone())).await;
                                                    }
                                                    if let Some(prompt_bytes) = result.prompt {
                                                        let _ = event_tx_read.send(AppEvent::Prompt(read_world_name.clone(), prompt_bytes)).await;
                                                    }
                                                    if !result.cleaned.is_empty() {
                                                        let _ = event_tx_read.send(AppEvent::ServerData(read_world_name.clone(), result.cleaned)).await;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                let msg = format!("Read error: {}", e);
                                                let _ = event_tx_read.send(AppEvent::ServerData(read_world_name.clone(), msg.into_bytes())).await;
                                                let _ = event_tx_read.send(AppEvent::Disconnected(read_world_name.clone(), reader_conn_id)).await;
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
                                            }
                                            WriteCommand::Raw(raw) => {
                                                if write_half.write_all(&raw).await.is_err() {
                                                    break;
                                                }
                                            }
                                            WriteCommand::Shutdown => {
                                                let _ = write_half.shutdown().await;
                                                break;
                                            }
                                        }
                                    }
                                });

                                // Notify main loop of successful connection
                                // For TLS, socket_fd should be None (can't preserve across reload)
                                let final_socket_fd = if is_tls { None } else { socket_fd };
                                let _ = event_tx_connect.send(AppEvent::ConnectionSuccess(
                                    world_name,
                                    cmd_tx,
                                    final_socket_fd,
                                    is_tls
                                )).await;
                            }
                            Err(e) => {
                                let _ = event_tx_connect.send(AppEvent::ConnectionFailed(world_name, e)).await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = event_tx_connect.send(AppEvent::ConnectionFailed(
                            world_name,
                            format!("Connection failed: {}", e)
                        )).await;
                    }
                }
            });
        }
        Command::Disconnect => {
            let world_index = app.current_world_index;
            if app.current_world().connected {
                // Kill proxy process if one exists
                #[cfg(unix)]
                if let Some(proxy_pid) = app.current_world().proxy_pid {
                    unsafe { libc::kill(proxy_pid as libc::pid_t, libc::SIGTERM); }
                }
                app.current_world_mut().clear_connection_state(true, true);
                app.add_output("Disconnected.");
                app.ws_broadcast(WsMessage::WorldDisconnected { world_index });
            } else {
                app.add_output("Not connected.");
            }
        }
        Command::Flush => {
            let line_count = app.current_world().output_lines.len();
            app.current_world_mut().output_lines.clear();
            app.current_world_mut().first_marked_new_index = None;
            app.current_world_mut().pending_lines.clear();
            app.current_world_mut().scroll_offset = 0;
            app.current_world_mut().lines_since_pause = 0;
            app.current_world_mut().paused = false;
            // Broadcast flush to WebSocket clients
            let world_index = app.current_world_index;
            app.ws_broadcast(WsMessage::WorldFlushed { world_index });
            app.add_output(&format!("Flushed {} lines from output buffer.", line_count));
        }
        Command::Send { text, all_worlds, target_world, no_newline } => {
            // Send the text
            let send_to_world = |world: &mut World, text: &str, no_newline: bool| -> bool {
                if !world.connected {
                    return false;
                }
                if let Some(tx) = &world.command_tx {
                    let result = if no_newline {
                        tx.try_send(WriteCommand::Raw(text.as_bytes().to_vec()))
                    } else {
                        tx.try_send(WriteCommand::Text(text.to_string()))
                    };
                    if result.is_ok() {
                        world.last_send_time = Some(std::time::Instant::now());
                        return true;
                    }
                }
                false
            };

            if all_worlds {
                // Send to all connected worlds
                let mut sent_count = 0;
                for world in &mut app.worlds {
                    if send_to_world(world, &text, no_newline) {
                        sent_count += 1;
                    }
                }
                if sent_count == 0 {
                    app.add_output("No connected worlds to send to.");
                }
            } else if let Some(world_name) = target_world {
                // Send to specific world
                if let Some(idx) = app.find_world(&world_name) {
                    if !send_to_world(&mut app.worlds[idx], &text, no_newline) {
                        app.add_output(&format!("World '{}' is not connected.", world_name));
                    }
                } else {
                    app.add_output(&format!("World '{}' not found.", world_name));
                }
            } else {
                // Send to current world
                let world = app.current_world_mut();
                if !world.connected {
                    app.add_output("Not connected. Use /worlds to connect.");
                } else if let Some(tx) = &world.command_tx {
                    let result = if no_newline {
                        tx.try_send(WriteCommand::Raw(text.as_bytes().to_vec()))
                    } else {
                        tx.try_send(WriteCommand::Text(text.to_string()))
                    };
                    if result.is_ok() {
                        world.last_send_time = Some(std::time::Instant::now());
                    } else {
                        app.add_output("Failed to send command.");
                    }
                }
            }
        }
        Command::Reload => {
            // Hot reload is not available on Android/Termux
            #[cfg(target_os = "android")]
            {
                app.add_output("Hot reload is not available on this platform.");
                app.add_output("Restart the app manually to apply changes.");
            }

            #[cfg(not(target_os = "android"))]
            {
                // First check if we can find the executable (handling " (deleted)" suffix)
                let exe_path = match get_executable_path() {
                    Ok((p, _)) => p,
                    Err(e) => {
                        app.add_output(&format!("Cannot find executable: {}", e));
                        return false;
                    }
                };

                if !exe_path.exists() {
                    app.add_output(&format!("Executable not found: {}", exe_path.display()));
                    app.add_output("Hot reload requires the binary to exist on disk.");
                    app.add_output("If running via 'cargo run', try running the compiled binary directly.");
                    return false;
                }

                // Check if there are any TLS connections that will be lost (only those without proxy)
                let tls_worlds: Vec<_> = app
                    .worlds
                    .iter()
                    .filter(|w| w.connected && w.is_tls && w.proxy_pid.is_none())
                    .map(|w| w.name.clone())
                    .collect();

                if !tls_worlds.is_empty() {
                    app.add_output(&format!(
                        "Warning: TLS connections will be closed: {}",
                        tls_worlds.join(", ")
                    ));
                    app.add_output("These connections will need to be re-established after reload.");
                }

                // Binary path is only shown on failure (see error handler below)

                // Notify connected web/GUI clients to reconnect after reload
                app.ws_broadcast(WsMessage::ServerReloading);

                // Disable raw mode before exec (the new process will re-enable it)
                let _ = crossterm::terminal::disable_raw_mode();
                let _ = crossterm::execute!(
                    std::io::stdout(),
                    crossterm::terminal::LeaveAlternateScreen
                );

                match exec_reload(app) {
                    Ok(()) => {
                        // This should never be reached - exec replaces the process
                        unreachable!();
                    }
                    Err(e) => {
                        // Restore terminal state if exec failed
                        let _ = crossterm::terminal::enable_raw_mode();
                        let _ = crossterm::execute!(
                            std::io::stdout(),
                            crossterm::terminal::EnterAlternateScreen,
                            crossterm::event::EnableBracketedPaste,
                            crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                            crossterm::cursor::MoveTo(0, 0)
                        );
                        app.add_output(&format!("Hot reload failed: {}", e));
                        app.add_output(&format!("Executable path: {}", exe_path.display()));
                    }
                }
            }
        }
        Command::Update { force } => {
            #[cfg(any(target_os = "android", not(unix)))]
            {
                let _ = force;
                app.add_output("Update is not available on this platform.");
            }

            #[cfg(all(unix, not(target_os = "android")))]
            {
                app.add_output(if force { "Force updating..." } else { "Checking for updates..." });
                let event_tx_clone = event_tx.clone();
                tokio::spawn(async move {
                    let result = check_and_download_update(force).await;
                    let _ = event_tx_clone.send(AppEvent::UpdateResult(result)).await;
                });
            }
        }
        Command::Remote => {
            // requesting_client_id 0 = console (results displayed via add_output in RemoteListResult handler)
            spawn_remote_ping_check(app, event_tx.clone(), 0, app.current_world_index);
        }
        Command::RemoteKill { client_id } => {
            let msg = if let Some(ref ws_server) = app.ws_server {
                if let Ok(clients) = ws_server.clients.try_read() {
                    if let Some(client) = clients.get(&client_id) {
                        let ip = client.ip_address.clone();
                        drop(clients);
                        if let Ok(mut clients_mut) = ws_server.clients.try_write() {
                            clients_mut.remove(&client_id);
                            format!("Disconnected remote client {} ({})", client_id, ip)
                        } else {
                            "Could not acquire write lock (busy).".to_string()
                        }
                    } else {
                        format!("No client with ID {}.", client_id)
                    }
                } else {
                    "Could not read client list (busy).".to_string()
                }
            } else {
                "WebSocket server is not running.".to_string()
            };
            app.add_output(&msg);
        }
        Command::BanList => {
            // Show current banned hosts
            let bans = app.ban_list.get_ban_info();
            if bans.is_empty() {
                app.add_output("No hosts are currently banned.");
            } else {
                app.add_output("");
                app.add_output("Banned Hosts:");
                app.add_output("─".repeat(70).as_str());
                app.add_output(&format!("{:<20} {:<12} {}", "Host", "Type", "Last URL/Reason"));
                app.add_output("─".repeat(70).as_str());
                for (ip, ban_type, reason) in bans {
                    let reason_display = if reason.is_empty() { "(unknown)" } else { &reason };
                    app.add_output(&format!("{:<20} {:<12} {}", ip, ban_type, reason_display));
                }
                app.add_output("─".repeat(70).as_str());
                app.add_output("Use /unban <host> to remove a ban.");
            }
            // Broadcast to remote clients
            app.ws_broadcast(WsMessage::BanListResponse { bans: app.ban_list.get_ban_info() });
        }
        Command::Unban { host } => {
            if app.ban_list.remove_ban(&host) {
                app.add_output(&format!("Removed ban for: {}", host));
                // Save settings to persist the change
                if app.multiuser_mode {
                    if let Err(e) = persistence::save_multiuser_settings(app) {
                        app.add_output(&format!("Warning: Failed to save settings: {}", e));
                    }
                } else if let Err(e) = persistence::save_settings(app) {
                    app.add_output(&format!("Warning: Failed to save settings: {}", e));
                }
                // Broadcast updated ban list to remote clients
                app.ws_broadcast(WsMessage::BanListResponse { bans: app.ban_list.get_ban_info() });
                app.ws_broadcast(WsMessage::UnbanResult { success: true, host: host.clone(), error: None });
            } else {
                app.add_output(&format!("No ban found for: {}", host));
                app.ws_broadcast(WsMessage::UnbanResult { success: false, host: host.clone(), error: Some("No ban found".to_string()) });
            }
        }
        Command::TestMusic => {
            let test_notes = generate_test_music_notes();
            app.play_ansi_music_console(&test_notes);
            app.add_output("Playing test music (Super Mario Bros)...");
        }
        Command::Notify { message } => {
            // Send notification to mobile clients
            let title = app.current_world().name.clone();
            app.ws_broadcast(WsMessage::Notification {
                title,
                message: message.clone(),
            });
            app.add_output(&format!("Notification sent: {}", message));
        }
        Command::Dict { word } => {
            match lookup_definition(&word).await {
                Ok(definition) => {
                    let ascii_def = transliterate_to_ascii(&definition);
                    let full_text = cap_text(format!("{}: {}", word, ascii_def), 1024);
                    app.input.buffer = full_text;
                    app.input.cursor_position = 0;
                }
                Err(e) => {
                    app.add_output(&format!("Definition lookup failed: {}", e));
                }
            }
        }
        Command::Urban { word } => {
            match lookup_urban_definition(&word).await {
                Ok(definition) => {
                    let ascii_def = transliterate_to_ascii(&definition);
                    let full_text = cap_text(format!("{}: {}", word, ascii_def), 1024);
                    app.input.buffer = full_text;
                    app.input.cursor_position = 0;
                }
                Err(e) => {
                    app.add_output(&format!("Urban Dictionary lookup failed: {}", e));
                }
            }
        }
        Command::DictUsage => {
            app.add_output("Usage: /dict <word>");
            app.add_output("  Looks up <word> in the dictionary and places the definition in the input buffer.");
            app.add_output("  Example: /dict hello");
        }
        Command::UrbanUsage => {
            app.add_output("Usage: /urban <word>");
            app.add_output("  Looks up <word> in Urban Dictionary and places the definition in the input buffer.");
            app.add_output("  Example: /urban yeet");
        }
        Command::Translate { lang, text } => {
            match lookup_translation(&text, &lang).await {
                Ok(translation) => {
                    let ascii_trans = transliterate_to_ascii(&translation);
                    let full_text = cap_text(ascii_trans, 1024);
                    app.input.buffer = full_text;
                    app.input.cursor_position = 0;
                }
                Err(e) => {
                    app.add_output(&format!("Translation failed: {}", e));
                }
            }
        }
        Command::TranslateUsage => {
            app.add_output("Usage: /translate <lang> <text>");
            app.add_output("  Translates <text> to <lang> and places the result in the input buffer.");
            app.add_output("  <lang> can be a code (es, fr, de) or name (spanish, french, german).");
            app.add_output("  Example: /translate spanish Hello, how are you?");
            app.add_output("  Example: /tr es Hello");
        }
        Command::TinyUrl { url } => {
            match lookup_tinyurl(&url).await {
                Ok(short) => {
                    app.input.buffer = short;
                    app.input.cursor_position = 0;
                }
                Err(e) => {
                    app.add_output(&format!("URL shortening failed: {}", e));
                }
            }
        }
        Command::TinyUrlUsage => {
            app.add_output("Usage: /url <url>");
            app.add_output("  Shortens <url> via is.gd and places the result in the input buffer.");
            app.add_output("  Example: /url https://github.com/c-hudson/clay");
        }
        Command::Dump => {
            // Dump comprehensive debug state to ~/.clay.dmp.log
            use std::io::Write;

            let home = get_home_dir();
            let dump_path = format!("{}/{}", home, clay_filename("clay.dmp.log"));

            match std::fs::File::create(&dump_path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "=== CLAY DEBUG DUMP ===");
                    let _ = writeln!(file, "Version: {} (build {}-{})", VERSION, BUILD_DATE, BUILD_HASH);
                    let now_ts = current_timestamp_secs();
                    let lt = local_time_from_epoch(now_ts as i64);
                    let _ = writeln!(file, "Timestamp: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                        lt.year, lt.month, lt.day, lt.hour, lt.minute, lt.second);
                    let _ = writeln!(file);

                    // Global state
                    let _ = writeln!(file, "=== GLOBAL STATE ===");
                    let _ = writeln!(file, "output_height: {}", app.output_height);
                    let _ = writeln!(file, "output_width: {}", app.output_width);
                    let _ = writeln!(file, "input_height: {}", app.input_height);
                    let _ = writeln!(file, "current_world_index: {}", app.current_world_index);
                    let _ = writeln!(file, "worlds_count: {}", app.worlds.len());
                    let _ = writeln!(file, "more_mode_enabled: {}", app.settings.more_mode_enabled);
                    let _ = writeln!(file, "show_tags: {}", app.show_tags);
                    let _ = writeln!(file, "new_line_indicator: {}", app.settings.new_line_indicator);
                    let _ = writeln!(file, "max_lines (output_height-2): {}", (app.output_height as usize).saturating_sub(2));
                    let _ = writeln!(file);

                    // WS client info
                    let _ = writeln!(file, "=== WS CLIENTS ===");
                    let _ = writeln!(file, "ws_client_worlds count: {}", app.ws_client_worlds.len());
                    for (cid, cv) in &app.ws_client_worlds {
                        let _ = writeln!(file, "  client {}: world_index={}, visible_lines={}, visible_columns={}, dimensions={:?}",
                            cid, cv.world_index, cv.visible_lines, cv.visible_columns, cv.dimensions);
                    }
                    let _ = writeln!(file);

                    // Per-world state
                    for (wi, world) in app.worlds.iter().enumerate() {
                        let is_current = wi == app.current_world_index;
                        let _ = writeln!(file, "=== WORLD {} [{}] {} ===",
                            wi, world.name, if is_current { "(CURRENT)" } else { "" });
                        let _ = writeln!(file, "connected: {}", world.connected);
                        let _ = writeln!(file, "paused: {}", world.paused);
                        let _ = writeln!(file, "lines_since_pause: {}", world.lines_since_pause);
                        let _ = writeln!(file, "visual_line_offset: {}", world.visual_line_offset);
                        let _ = writeln!(file, "scroll_offset: {}", world.scroll_offset);
                        let _ = writeln!(file, "output_lines.len: {}", world.output_lines.len());
                        let _ = writeln!(file, "pending_lines.len: {}", world.pending_lines.len());
                        let _ = writeln!(file, "unseen_lines: {}", world.unseen_lines);
                        let _ = writeln!(file, "showing_splash: {}", world.showing_splash);
                        let _ = writeln!(file, "partial_line: {:?}", if world.partial_line.is_empty() { "".to_string() } else { format!("({} chars) {:?}", world.partial_line.len(), &world.partial_line[..world.partial_line.len().min(100)]) });
                        let _ = writeln!(file, "partial_in_pending: {}", world.partial_in_pending);
                        if let Some(ps) = world.pending_since {
                            let _ = writeln!(file, "pending_since: {:?} ago", ps.elapsed());
                        }
                        let _ = writeln!(file, "encoding: {:?}", world.settings.encoding);

                        // Min viewer dimensions
                        let ws_min_lines = app.min_viewer_lines(wi);
                        let ws_min_width = app.min_viewer_width(wi);
                        let _ = writeln!(file, "min_viewer_lines: {:?}", ws_min_lines);
                        let _ = writeln!(file, "min_viewer_width: {:?}", ws_min_width);

                        // Effective output dimensions for more-mode
                        let console_viewing = wi == app.current_world_index;
                        let eff_height = match (console_viewing, ws_min_lines) {
                            (true, Some(ws)) => (app.output_height).min(ws as u16),
                            (true, None) => app.output_height,
                            (false, Some(ws)) => ws as u16,
                            (false, None) => app.output_height,
                        };
                        let eff_width = match (console_viewing, ws_min_width) {
                            (true, Some(ws_w)) => (app.output_width).min(ws_w as u16),
                            (true, None) => app.output_width,
                            (false, Some(ws_w)) => ws_w as u16,
                            (false, None) => app.output_width,
                        };
                        let _ = writeln!(file, "effective_output_height: {}", eff_height);
                        let _ = writeln!(file, "effective_output_width: {}", eff_width);
                        let _ = writeln!(file, "effective_max_lines: {}", (eff_height as usize).saturating_sub(2));
                        let _ = writeln!(file);

                        // Full scrollback dump (all output + pending lines)
                        let _ = writeln!(file, "--- OUTPUT LINES ({}) ---", world.output_lines.len());
                        for line in &world.output_lines {
                            let lt = local_time_from_epoch(line.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64);
                            let prefix = if line.gagged { "G" } else if !line.from_server { "C" } else { "" };
                            let _ = writeln!(file, "{},{:04}-{:02}-{:02} {:02}:{:02}:{:02},{}",
                                prefix, lt.year, lt.month, lt.day, lt.hour, lt.minute, lt.second,
                                line.text);
                        }

                        if !world.pending_lines.is_empty() {
                            let _ = writeln!(file, "--- PENDING LINES ({}) ---", world.pending_lines.len());
                            for line in &world.pending_lines {
                                let lt = local_time_from_epoch(line.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64);
                                let prefix = if line.gagged { "G" } else if !line.from_server { "C" } else { "" };
                                let _ = writeln!(file, "P,{},{:04}-{:02}-{:02} {:02}:{:02}:{:02},{}",
                                    prefix, lt.year, lt.month, lt.day, lt.hour, lt.minute, lt.second,
                                    line.text);
                            }
                        }
                        let _ = writeln!(file);
                    }

                    // Don't use add_output — it resets lines_since_pause and scrolls.
                    // /dump must be passive and not disturb more-mode state.
                }
                Err(_e) => {
                }
            }
        }
        Command::AddWorld { name, host, port, user, password, use_ssl } => {
            // Check if world already exists (case-insensitive)
            let existing_idx = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(&name));

            let world_idx = if let Some(idx) = existing_idx {
                // Update existing world
                idx
            } else {
                // Create new world - validate name
                if name.is_empty() {
                    app.add_output("Error: World name cannot be empty");
                    return false;
                }
                if name.contains(' ') {
                    app.add_output("Error: World name cannot contain spaces");
                    return false;
                }
                if name.starts_with('(') {
                    app.add_output("Error: World name cannot start with '('");
                    return false;
                }

                // Create new world
                let new_world = World::new(&name);
                app.worlds.push(new_world);
                app.worlds.len() - 1
            };

            // Update world settings
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

            // Save settings to persist the new/updated world
            if let Err(e) = persistence::save_settings(app) {
                app.add_output(&format!("Warning: Failed to save settings: {}", e));
            }

            // Confirm the operation
            let action = if existing_idx.is_some() { "Updated" } else { "Added" };
            let host_info = if !app.worlds[world_idx].settings.hostname.is_empty() {
                format!(" ({}:{}{})",
                    app.worlds[world_idx].settings.hostname,
                    app.worlds[world_idx].settings.port,
                    if use_ssl { " SSL" } else { "" })
            } else {
                " (connectionless)".to_string()
            };
            app.add_output(&format!("{} world '{}'{}.", action, name, host_info));
        }
        Command::EditList => {
            app.open_notes_list_popup();
        }
        Command::Edit { filename } => {
            // Open split-screen editor
            if app.editor.visible {
                app.add_output("Editor is already open. Close it first.");
            } else if let Some(ref path_str) = filename {
                // Edit a file
                let path = PathBuf::from(path_str);
                let content = if path.exists() {
                    match std::fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            app.add_output(&format!("Failed to read file: {}", e));
                            return false;
                        }
                    }
                } else {
                    // New file - start with empty content
                    String::new()
                };
                app.editor.open_file(path, &content);
                app.needs_terminal_clear = true;
            } else {
                // Edit current world's notes
                let world_idx = app.current_world_index;
                let notes = app.worlds[world_idx].settings.notes.clone();
                app.editor.open_notes(world_idx, &notes);
                app.needs_terminal_clear = true;
            }
        }
        Command::Tag => {
            // Toggle MUD tag display (same as F2) - silent, no output
            app.show_tags = !app.show_tags;
            app.needs_output_redraw = true;
            // Broadcast to WebSocket clients
            app.ws_broadcast(WsMessage::ShowTagsChanged { show_tags: app.show_tags });
        }
        Command::ActionCommand { name, args } => {
            // Check if this is an action command (/name)
            let action_found = app.settings.actions.iter()
                .find(|a| a.name.eq_ignore_ascii_case(&name))
                .cloned();

            if let Some(action) = action_found {
                // Skip disabled actions
                if !action.enabled {
                    app.add_output(&format!("Action '{}' is disabled.", name));
                } else {
                // Execute the action's commands - process each individually
                let commands = split_action_commands(&action.command);
                let mut sent_to_server = false;
                for cmd_str in commands {
                    // Substitute $1-$9 and $* with arguments
                    let cmd_str = substitute_action_args(&cmd_str, &args);

                    // Skip /gag commands when invoked manually
                    if cmd_str.eq_ignore_ascii_case("/gag") || cmd_str.to_lowercase().starts_with("/gag ") {
                        continue;
                    }

                    // Unified command system - route through TF parser
                    if cmd_str.starts_with('/') {
                        app.sync_tf_world_info();
                        match app.tf_engine.execute(&cmd_str) {
                            tf::TfCommandResult::Success(Some(msg)) => {
                                app.add_tf_output(&msg);
                            }
                            tf::TfCommandResult::Success(None) => {}
                            tf::TfCommandResult::Error(err) => {
                                app.add_tf_output(&format!("✨ {}", err));
                            }
                            tf::TfCommandResult::SendToMud(text) => {
                                if let Some(tx) = &app.current_world().command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(text));
                                    sent_to_server = true;
                                } else {
                                    app.add_output("Not connected. Use /worlds to connect.");
                                }
                            }
                            tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                Box::pin(handle_command(&clay_cmd, app, event_tx.clone())).await;
                            }
                            tf::TfCommandResult::Recall(opts) => {
                                let output_lines = app.current_world().output_lines.clone();
                                let (matches, header) = execute_recall(&opts, &output_lines);
                                let pattern_str = opts.pattern.as_deref().unwrap_or("*");

                                if !opts.quiet {
                                    if let Some(h) = header {
                                        app.add_output(&h);
                                    }
                                }
                                if matches.is_empty() {
                                    app.add_output(&format!("No matches for '{}'", pattern_str));
                                } else {
                                    for m in matches {
                                        app.add_output(&m);
                                    }
                                }
                                if !opts.quiet {
                                    app.add_output("================= Recall end =================");
                                }
                            }
                            _ => {}
                        }
                    } else {
                        // Plain text - send to server if connected
                        if let Some(tx) = &app.current_world().command_tx {
                            let _ = tx.try_send(WriteCommand::Text(cmd_str.clone()));
                            sent_to_server = true;
                        } else {
                            app.add_output(&format!("Not connected. Cannot send: {}", cmd_str));
                        }
                    }
                }
                if sent_to_server {
                    app.current_world_mut().last_send_time = Some(std::time::Instant::now());
                }
                }
            } else {
                // No matching action - try TF engine (handles /recall, /set, /echo, etc.)
                app.sync_tf_world_info();
                match app.tf_engine.execute(cmd) {
                    tf::TfCommandResult::Success(Some(msg)) => {
                        app.add_tf_output(&msg);
                    }
                    tf::TfCommandResult::Success(None) => {}
                    tf::TfCommandResult::Error(err) => {
                        app.add_tf_output(&format!("Error: {}", err));
                    }
                    tf::TfCommandResult::SendToMud(text) => {
                        if let Some(tx) = &app.current_world().command_tx {
                            let _ = tx.try_send(WriteCommand::Text(text));
                            app.current_world_mut().last_send_time = Some(std::time::Instant::now());
                        }
                    }
                    tf::TfCommandResult::ClayCommand(_) => {
                        // Avoid recursion - just show unknown
                        app.add_output(&format!("Unknown command: /{}", name));
                    }
                    tf::TfCommandResult::Recall(opts) => {
                        let output_lines = app.current_world().output_lines.clone();
                        let (matches, header) = execute_recall(&opts, &output_lines);
                        let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                        if !opts.quiet {
                            if let Some(h) = header {
                                app.add_output(&h);
                            }
                        }
                        if matches.is_empty() {
                            app.add_output(&format!("No matches for '{}'", pattern_str));
                        } else {
                            for m in &matches {
                                app.add_output(m);
                            }
                        }
                        if !opts.quiet {
                            app.add_output("================= Recall end =================");
                        }
                    }
                    tf::TfCommandResult::RepeatProcess(process) => {
                        app.tf_engine.processes.push(process);
                    }
                    _ => {
                        app.add_output(&format!("Unknown command: /{}", name));
                    }
                }
            }
        }
        Command::NotACommand { text } => {
            // Not a command - send to MUD as regular input
            if let Some(tx) = &app.current_world().command_tx {
                let _ = tx.try_send(WriteCommand::Text(text));
                app.current_world_mut().last_send_time = Some(std::time::Instant::now());
            }
        }
        Command::Unknown { cmd } => {
            app.add_output(&format!("Unknown command: {}", cmd));
        }
    }
    false
}

/// Process any pending world operations queued by TF functions like addworld()
pub(crate) fn process_pending_world_ops(app: &mut App) {
    // Drain pending operations
    let ops: Vec<tf::PendingWorldOp> = app.tf_engine.pending_world_ops.drain(..).collect();

    for op in ops {
        // Check if world already exists (case-insensitive)
        let existing_idx = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(&op.name));

        let world_idx = if let Some(idx) = existing_idx {
            idx
        } else {
            // Create new world
            let new_world = World::new(&op.name);
            app.worlds.push(new_world);
            app.worlds.len() - 1
        };

        // Update world settings
        if let Some(h) = op.host {
            app.worlds[world_idx].settings.hostname = h;
        }
        if let Some(p) = op.port {
            app.worlds[world_idx].settings.port = p;
        }
        if let Some(u) = op.user {
            app.worlds[world_idx].settings.user = u;
        }
        if let Some(p) = op.password {
            app.worlds[world_idx].settings.password = p;
        }
        app.worlds[world_idx].settings.use_ssl = op.use_ssl;

        // Save settings to persist the new/updated world
        let _ = persistence::save_settings(app);

        // addworld() function should be silent on success (only errors are reported)
    }
}

/// Process any pending commands queued by TF macro execution
pub(crate) fn process_pending_tf_commands(app: &mut App) {
    // Drain pending commands
    let cmds: Vec<tf::TfCommand> = app.tf_engine.pending_commands.drain(..).collect();

    for cmd in cmds {
        // Determine target world
        let world_idx = if let Some(ref world_name) = cmd.world {
            // Find world by name
            app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(world_name))
                .unwrap_or(app.current_world_index)
        } else {
            app.current_world_index
        };

        // Send command to the world
        if world_idx < app.worlds.len() && app.worlds[world_idx].connected {
            if let Some(tx) = &app.worlds[world_idx].command_tx {
                let _ = tx.try_send(WriteCommand::Text(cmd.command.clone()));
                app.worlds[world_idx].last_send_time = Some(std::time::Instant::now());
            }
        }
    }
}

