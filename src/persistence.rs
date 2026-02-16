use std::io::{self, BufRead, Write as IoWrite};
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

#[cfg(unix)]
use std::os::unix::io::RawFd;

use crate::*;
use crate::{
    App, World, WorldSettings, WorldType, User,
    get_settings_path, get_multiuser_settings_path, get_reload_state_path, debug_log,
};

/// Encryption key for password storage (padded to 32 bytes for AES-256)
pub(crate) const PASSWORD_ENCRYPTION_KEY: &[u8; 32] = b"nonsupersecretpassword#\0\0\0\0\0\0\0\0\0";

/// Encrypt a password using AES-256-GCM and return base64-encoded result
pub fn encrypt_password(password: &str) -> String {
    if password.is_empty() {
        return String::new();
    }

    let cipher = Aes256Gcm::new(PASSWORD_ENCRYPTION_KEY.into());

    // Use a fixed nonce derived from the password length (not cryptographically ideal,
    // but acceptable for this obfuscation use case with a known key)
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[0] = (password.len() & 0xFF) as u8;
    nonce_bytes[1] = ((password.len() >> 8) & 0xFF) as u8;
    // Add some variation based on first few chars
    for (i, b) in password.bytes().take(10).enumerate() {
        nonce_bytes[2 + i] = b;
    }
    let nonce = Nonce::from_slice(&nonce_bytes);

    match cipher.encrypt(nonce, password.as_bytes()) {
        Ok(ciphertext) => {
            // Prepend nonce to ciphertext and base64 encode
            let mut combined = nonce_bytes.to_vec();
            combined.extend(ciphertext);
            format!("ENC:{}", BASE64.encode(&combined))
        }
        Err(_) => {
            // Fallback to plain (shouldn't happen)
            password.to_string()
        }
    }
}

/// Decrypt a password. Returns the original string if it's not encrypted or decryption fails.
pub fn decrypt_password(stored: &str) -> String {
    if stored.is_empty() {
        return String::new();
    }

    // Check if it's an encrypted password
    if !stored.starts_with("ENC:") {
        // Not encrypted, return as-is (legacy plain password)
        return stored.to_string();
    }

    let encoded = &stored[4..]; // Skip "ENC:" prefix

    let combined = match BASE64.decode(encoded) {
        Ok(data) => data,
        Err(_) => return stored.to_string(), // Invalid base64, treat as plain
    };

    if combined.len() < 12 {
        // Too short to contain nonce, treat as plain
        return stored.to_string();
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new(PASSWORD_ENCRYPTION_KEY.into());

    match cipher.decrypt(nonce, ciphertext) {
        Ok(plaintext) => String::from_utf8_lossy(&plaintext).to_string(),
        Err(_) => {
            // Decryption failed - might be a plain password that happens to start with "ENC:"
            // This is unlikely but we handle it gracefully
            stored.to_string()
        }
    }
}

pub fn save_settings(app: &App) -> io::Result<()> {
    // Only master client should save settings
    if !app.is_master {
        return Ok(());
    }
    save_settings_to_path(app, &get_settings_path())
}

/// Save settings to a specific path (used by tests)
pub fn save_settings_to_path(app: &App, path: &std::path::Path) -> io::Result<()> {
    let mut file = std::fs::File::create(path)?;

    // Save global settings
    writeln!(file, "[global]")?;
    writeln!(file, "more_mode={}", app.settings.more_mode_enabled)?;
    writeln!(file, "spell_check={}", app.settings.spell_check_enabled)?;
    writeln!(file, "temp_convert={}", app.settings.temp_convert_enabled)?;
    writeln!(file, "world_switch_mode={}", app.settings.world_switch_mode.name())?;
    // Note: show_tags is now a temporary in-memory setting (F2 or /tag), not persisted
    writeln!(file, "debug_enabled={}", app.settings.debug_enabled)?;
    writeln!(file, "ansi_music_enabled={}", app.settings.ansi_music_enabled)?;
    writeln!(file, "input_height={}", app.input_height)?;
    writeln!(file, "theme={}", app.settings.theme.name())?;
    writeln!(file, "gui_theme={}", app.settings.gui_theme.name())?;
    writeln!(file, "gui_transparency={}", app.settings.gui_transparency)?;
    writeln!(file, "color_offset_percent={}", app.settings.color_offset_percent)?;
    writeln!(file, "font_name={}", app.settings.font_name)?;
    writeln!(file, "font_size={}", app.settings.font_size)?;
    writeln!(file, "web_font_size_phone={}", app.settings.web_font_size_phone)?;
    writeln!(file, "web_font_size_tablet={}", app.settings.web_font_size_tablet)?;
    writeln!(file, "web_font_size_desktop={}", app.settings.web_font_size_desktop)?;
    writeln!(file, "web_secure={}", app.settings.web_secure)?;
    writeln!(file, "http_enabled={}", app.settings.http_enabled)?;
    writeln!(file, "http_port={}", app.settings.http_port)?;
    writeln!(file, "ws_enabled={}", app.settings.ws_enabled)?;
    writeln!(file, "ws_port={}", app.settings.ws_port)?;
    if !app.settings.websocket_password.is_empty() {
        writeln!(file, "websocket_password={}", encrypt_password(&app.settings.websocket_password))?;
    }
    if !app.settings.websocket_allow_list.is_empty() {
        writeln!(file, "websocket_allow_list={}", app.settings.websocket_allow_list)?;
    }
    if !app.settings.websocket_cert_file.is_empty() {
        writeln!(file, "websocket_cert_file={}", app.settings.websocket_cert_file)?;
    }
    if !app.settings.websocket_key_file.is_empty() {
        writeln!(file, "websocket_key_file={}", app.settings.websocket_key_file)?;
    }
    // Save device auth keys (encrypted)
    for key in &app.settings.websocket_auth_keys {
        writeln!(file, "websocket_auth_key={}", encrypt_password(key))?;
    }
    writeln!(file, "tls_proxy_enabled={}", app.settings.tls_proxy_enabled)?;
    if !app.settings.dictionary_path.is_empty() {
        writeln!(file, "dictionary_path={}", app.settings.dictionary_path)?;
    }
    writeln!(file, "editor_side={}", app.settings.editor_side.name())?;

    // Save each world's settings (skip unconfigured worlds that have no connection info)
    for world in &app.worlds {
        let has_mud_config = !world.settings.hostname.is_empty();
        let has_slack_config = !world.settings.slack_token.is_empty();
        let has_discord_config = !world.settings.discord_token.is_empty();
        if !has_mud_config && !has_slack_config && !has_discord_config {
            continue; // Don't persist unconfigured worlds
        }
        writeln!(file)?;
        writeln!(file, "[world:{}]", world.name)?;
        writeln!(file, "world_type={}", world.settings.world_type.name())?;
        // MUD settings
        writeln!(file, "hostname={}", world.settings.hostname)?;
        writeln!(file, "port={}", world.settings.port)?;
        writeln!(file, "user={}", world.settings.user)?;
        writeln!(file, "password={}", encrypt_password(&world.settings.password))?;
        writeln!(file, "use_ssl={}", world.settings.use_ssl)?;
        writeln!(file, "encoding={}", world.settings.encoding.name())?;
        writeln!(file, "auto_connect_type={}", world.settings.auto_connect_type.name())?;
        writeln!(file, "keep_alive_type={}", world.settings.keep_alive_type.name())?;
        if !world.settings.keep_alive_cmd.is_empty() {
            writeln!(file, "keep_alive_cmd={}", world.settings.keep_alive_cmd)?;
        }
        if world.settings.gmcp_packages != "Client.Media 1" {
            writeln!(file, "gmcp_packages={}", world.settings.gmcp_packages)?;
        }
        if world.settings.log_enabled {
            writeln!(file, "log_enabled=true")?;
        }
        // Slack settings
        if !world.settings.slack_token.is_empty() {
            writeln!(file, "slack_token={}", encrypt_password(&world.settings.slack_token))?;
        }
        if !world.settings.slack_channel.is_empty() {
            writeln!(file, "slack_channel={}", world.settings.slack_channel)?;
        }
        if !world.settings.slack_workspace.is_empty() {
            writeln!(file, "slack_workspace={}", world.settings.slack_workspace)?;
        }
        // Discord settings
        if !world.settings.discord_token.is_empty() {
            writeln!(file, "discord_token={}", encrypt_password(&world.settings.discord_token))?;
        }
        if !world.settings.discord_guild.is_empty() {
            writeln!(file, "discord_guild={}", world.settings.discord_guild)?;
        }
        if !world.settings.discord_channel.is_empty() {
            writeln!(file, "discord_channel={}", world.settings.discord_channel)?;
        }
        if !world.settings.discord_dm_user.is_empty() {
            writeln!(file, "discord_dm_user={}", world.settings.discord_dm_user)?;
        }
        // Notes (escape newlines and special chars)
        if !world.settings.notes.is_empty() {
            let escaped_notes = world.settings.notes
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('=', "\\e");
            writeln!(file, "notes={}", escaped_notes)?;
        }
    }

    // Save actions (by name, escaping special characters)
    for action in app.settings.actions.iter() {
        writeln!(file)?;
        // Escape special chars in name for section header: ] [ = \
        let escaped_name = action.name
            .replace('\\', "\\\\")
            .replace(']', "\\]")
            .replace('[', "\\[")
            .replace('=', "\\e");
        writeln!(file, "[action:{}]", escaped_name)?;
        if !action.world.is_empty() {
            writeln!(file, "world={}", action.world)?;
        }
        // Only save match_type if not the default (regexp)
        if action.match_type != MatchType::Regexp {
            writeln!(file, "match_type={}", action.match_type.as_str().to_lowercase())?;
        }
        if !action.pattern.is_empty() {
            // Escape newlines and equals signs in pattern
            writeln!(file, "pattern={}", action.pattern.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
        }
        if !action.command.is_empty() {
            // Escape newlines and equals signs in command
            writeln!(file, "command={}", action.command.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
        }
        // Only save enabled if not the default (true)
        if !action.enabled {
            writeln!(file, "enabled=false")?;
        }
        // Only save startup if enabled (default is false)
        if action.startup {
            writeln!(file, "startup=true")?;
        }
    }

    // Save permanent bans
    let permanent_bans = app.ban_list.get_permanent_bans();
    if !permanent_bans.is_empty() {
        writeln!(file)?;
        writeln!(file, "[banned_hosts]")?;
        for ip in permanent_bans {
            writeln!(file, "ip={}", ip)?;
        }
    }

    // Save TF global variables
    if !app.tf_engine.global_vars.is_empty() {
        writeln!(file)?;
        writeln!(file, "[tf_globals]")?;
        for (name, value) in &app.tf_engine.global_vars {
            // Escape special characters in value
            let val_str = value.to_string_value()
                .replace('\\', "\\\\")
                .replace('=', "\\e")
                .replace('\n', "\\n");
            writeln!(file, "{}={}", name, val_str)?;
        }
    }

    Ok(())
}

pub fn load_settings(app: &mut App) -> io::Result<()> {
    let path = get_settings_path();
    load_settings_from_path(app, &path)
}

/// Load settings from a specific path (used by tests)
pub fn load_settings_from_path(app: &mut App, path: &std::path::Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);

    let mut current_world: Option<String> = None;
    let mut current_action: Option<usize> = None;
    let mut in_banned_hosts = false;
    let mut in_tf_globals = false;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with("[global]") {
            current_world = None;
            current_action = None;
            in_banned_hosts = false;
            in_tf_globals = false;
            continue;
        }

        if line.starts_with("[banned_hosts]") {
            current_world = None;
            current_action = None;
            in_banned_hosts = true;
            in_tf_globals = false;
            continue;
        }

        if line.starts_with("[tf_globals]") {
            current_world = None;
            current_action = None;
            in_banned_hosts = false;
            in_tf_globals = true;
            continue;
        }

        if line.starts_with("[world:") && line.ends_with(']') {
            let name = &line[7..line.len() - 1];
            // Find or create world
            let idx = app.find_or_create_world(name);
            current_world = Some(app.worlds[idx].name.clone());
            current_action = None;
            in_banned_hosts = false;
            in_tf_globals = false;
            continue;
        }

        if line.starts_with("[action:") && line.ends_with(']') {
            // Parse action section - supports both old format [action:NUMBER] and new format [action:NAME]
            current_world = None;
            in_banned_hosts = false;
            in_tf_globals = false;
            let section_content = &line[8..line.len() - 1]; // Extract between "[action:" and "]"

            // Unescape the section content (for new format names with special chars)
            let unescaped = section_content
                .replace("\\]", "]")
                .replace("\\[", "[")
                .replace("\\e", "=")
                .replace("\\\\", "\\");

            // Check if it's old format (pure number) or new format (name)
            let is_old_format = unescaped.chars().all(|c| c.is_ascii_digit());

            if is_old_format {
                // Old format: create new action, will get name from name= field
                app.settings.actions.push(Action::new());
                current_action = Some(app.settings.actions.len() - 1);
            } else {
                // New format: look for existing action with this name or create new
                let action_name = unescaped;
                if let Some(idx) = app.settings.actions.iter().position(|a| a.name == action_name) {
                    current_action = Some(idx);
                } else {
                    let mut new_action = Action::new();
                    new_action.name = action_name;
                    app.settings.actions.push(new_action);
                    current_action = Some(app.settings.actions.len() - 1);
                }
            }
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];

            // Check for banned hosts section
            if in_banned_hosts {
                if key == "ip" && !value.is_empty() {
                    app.ban_list.add_permanent_ban(value);
                }
                continue;
            }

            // Check for TF globals section
            if in_tf_globals {
                // Unescape the value
                let unescaped = value
                    .replace("\\\\", "\x00")
                    .replace("\\n", "\n")
                    .replace("\\e", "=")
                    .replace("\x00", "\\");
                app.tf_engine.set_global(key, tf::TfValue::from(unescaped));
                continue;
            }

            // Check for action settings first (current_action takes priority)
            if let Some(action_idx) = current_action {
                // Action settings
                if let Some(action) = app.settings.actions.get_mut(action_idx) {
                    // Helper to unescape saved strings
                    fn unescape_action_value(s: &str) -> String {
                        s.replace("\\\\", "\x00").replace("\\n", "\n").replace("\\e", "=").replace("\x00", "\\")
                    }
                    match key {
                        "name" => action.name = value.to_string(),
                        "world" => action.world = value.to_string(),
                        "match_type" => action.match_type = MatchType::parse(value),
                        "pattern" => action.pattern = unescape_action_value(value),
                        "command" => action.command = unescape_action_value(value),
                        "enabled" => action.enabled = value != "false",
                        "startup" => action.startup = value == "true",
                        _ => {}
                    }
                }
            } else if current_world.is_none() {
                // Global settings
                match key {
                    "more_mode" => {
                        app.settings.more_mode_enabled = value == "true";
                    }
                    "spell_check" => {
                        app.settings.spell_check_enabled = value == "true";
                    }
                    "temp_convert" => {
                        app.settings.temp_convert_enabled = value == "true";
                    }
                    "pending_first" => {
                        // Backward compatibility: pending_first=true -> UnseenFirst
                        app.settings.world_switch_mode = if value == "true" {
                            WorldSwitchMode::UnseenFirst
                        } else {
                            WorldSwitchMode::Alphabetical
                        };
                    }
                    "world_switch_mode" => {
                        app.settings.world_switch_mode = WorldSwitchMode::from_name(value);
                    }
                    "debug_enabled" => {
                        app.settings.debug_enabled = value == "true";
                    }
                    "ansi_music_enabled" => {
                        app.settings.ansi_music_enabled = value == "true";
                    }
                    "input_height" => {
                        if let Ok(h) = value.parse::<u16>() {
                            app.input_height = h.clamp(1, 15);
                            app.input.visible_height = app.input_height;
                        }
                    }
                    "theme" => {
                        app.settings.theme = Theme::from_name(value);
                    }
                    "gui_theme" => {
                        app.settings.gui_theme = Theme::from_name(value);
                    }
                    "font_name" => {
                        app.settings.font_name = value.to_string();
                    }
                    "font_size" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.font_size = s.clamp(8.0, 48.0);
                        }
                    }
                    // Backward compat: old single web_font_size sets all three
                    "web_font_size" => {
                        if let Ok(s) = value.parse::<f32>() {
                            let clamped = s.clamp(8.0, 48.0);
                            app.settings.web_font_size_phone = clamped;
                            app.settings.web_font_size_tablet = clamped;
                            app.settings.web_font_size_desktop = clamped;
                        }
                    }
                    "web_font_size_phone" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.web_font_size_phone = s.clamp(8.0, 48.0);
                        }
                    }
                    "web_font_size_tablet" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.web_font_size_tablet = s.clamp(8.0, 48.0);
                        }
                    }
                    "web_font_size_desktop" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.web_font_size_desktop = s.clamp(8.0, 48.0);
                        }
                    }
                    "gui_transparency" => {
                        if let Ok(t) = value.parse::<f32>() {
                            app.settings.gui_transparency = t.clamp(0.3, 1.0);
                        }
                    }
                    "color_offset_percent" => {
                        if let Ok(p) = value.parse::<u8>() {
                            app.settings.color_offset_percent = p.min(100);
                        }
                    }
                    "web_secure" => {
                        app.settings.web_secure = value == "true";
                    }
                    "ws_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    "ws_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_enabled maps to ws_enabled
                    "websocket_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    // Legacy: websocket_port maps to ws_port
                    "websocket_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_use_tls maps to web_secure
                    "websocket_use_tls" => {
                        app.settings.web_secure = value == "true";
                    }
                    "websocket_password" => {
                        app.settings.websocket_password = decrypt_password(value);
                    }
                    "websocket_allow_list" => {
                        app.settings.websocket_allow_list = value.to_string();
                    }
                    "websocket_cert_file" => {
                        app.settings.websocket_cert_file = value.to_string();
                    }
                    "websocket_key_file" => {
                        app.settings.websocket_key_file = value.to_string();
                    }
                    "websocket_auth_key" => {
                        // Load device auth key (decrypt it)
                        let key = decrypt_password(value);
                        if !key.is_empty() {
                            app.settings.websocket_auth_keys.push(key);
                        }
                    }
                    "http_enabled" => {
                        app.settings.http_enabled = value == "true";
                    }
                    "http_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.http_port = p;
                        }
                    }
                    // Legacy fields - map https to http when web_secure, ws_nonsecure to ws when !web_secure
                    "https_enabled" => {
                        // If https was enabled in old config, set http_enabled and web_secure
                        if value == "true" {
                            app.settings.http_enabled = true;
                            app.settings.web_secure = true;
                        }
                    }
                    "https_port" => {
                        // Legacy: https_port was separate, now http_port is used for both
                        if let Ok(p) = value.parse::<u16>() {
                            // Only use https_port if web_secure is set
                            if app.settings.web_secure {
                                app.settings.http_port = p;
                            }
                        }
                    }
                    "ws_nonsecure_enabled" => {
                        // Legacy: ws_nonsecure maps to ws_enabled when not secure
                        if value == "true" && !app.settings.web_secure {
                            app.settings.ws_enabled = true;
                        }
                    }
                    "ws_nonsecure_port" => {
                        // Legacy: ws_nonsecure_port was separate, now ws_port is used for both
                        if let Ok(p) = value.parse::<u16>() {
                            if !app.settings.web_secure {
                                app.settings.ws_port = p;
                            }
                        }
                    }
                    // Legacy: ignore global encoding, it's now per-world
                    "encoding" => {}
                    "tls_proxy_enabled" => {
                        app.settings.tls_proxy_enabled = value == "true";
                    }
                    "dictionary_path" => {
                        app.settings.dictionary_path = value.to_string();
                    }
                    "editor_side" => {
                        app.settings.editor_side = EditorSide::from_name(value);
                    }
                    _ => {}
                }
            } else if let Some(ref world_name) = current_world {
                // Find the world and update its settings
                if let Some(world) = app.worlds.iter_mut().find(|w| &w.name == world_name) {
                    match key {
                        "world_type" => world.settings.world_type = WorldType::from_name(value),
                        "hostname" => world.settings.hostname = value.to_string(),
                        "port" => world.settings.port = value.to_string(),
                        "user" => world.settings.user = value.to_string(),
                        "password" => world.settings.password = decrypt_password(value),
                        "use_ssl" => world.settings.use_ssl = value == "true",
                        "log_enabled" => world.settings.log_enabled = value == "true",
                        "log_file" => world.settings.log_enabled = true, // Backward compat: old log_file setting enables logging
                        "encoding" => {
                            world.settings.encoding = match value {
                                "latin1" => Encoding::Latin1,
                                "fansi" => Encoding::Fansi,
                                _ => Encoding::Utf8,
                            };
                        }
                        "auto_connect_type" => {
                            world.settings.auto_connect_type = AutoConnectType::from_name(value);
                        }
                        "keep_alive_type" => {
                            world.settings.keep_alive_type = KeepAliveType::from_name(value);
                        }
                        "keep_alive_cmd" => {
                            world.settings.keep_alive_cmd = value.to_string();
                        }
                        "gmcp_packages" => {
                            world.settings.gmcp_packages = value.to_string();
                        }
                        // Slack settings
                        "slack_token" => world.settings.slack_token = decrypt_password(value),
                        "slack_channel" => world.settings.slack_channel = value.to_string(),
                        "slack_workspace" => world.settings.slack_workspace = value.to_string(),
                        // Discord settings
                        "discord_token" => world.settings.discord_token = decrypt_password(value),
                        "discord_guild" => world.settings.discord_guild = value.to_string(),
                        "discord_channel" => world.settings.discord_channel = value.to_string(),
                        "discord_dm_user" => world.settings.discord_dm_user = value.to_string(),
                        // Notes
                        "notes" => world.settings.notes = unescape_string(value),
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

/// Load settings for multiuser mode from ~/.clay.multiuser.dat
pub fn load_multiuser_settings(app: &mut App) -> io::Result<()> {
    let path = get_multiuser_settings_path();
    if !path.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "Multiuser settings file not found"));
    }

    let file = std::fs::File::open(&path)?;
    let reader = std::io::BufReader::new(file);

    let mut current_world: Option<String> = None;
    let mut current_action: Option<usize> = None;
    let mut current_user: Option<String> = None;
    let mut in_banned_hosts = false;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line.starts_with("[global]") {
            current_world = None;
            current_action = None;
            current_user = None;
            in_banned_hosts = false;
            continue;
        }

        if line.starts_with("[banned_hosts]") {
            current_world = None;
            current_action = None;
            current_user = None;
            in_banned_hosts = true;
            continue;
        }

        // Parse [user:NAME] sections
        if line.starts_with("[user:") && line.ends_with(']') {
            let name = &line[6..line.len() - 1];
            // Unescape the name
            let unescaped = name
                .replace("\\]", "]")
                .replace("\\[", "[")
                .replace("\\e", "=")
                .replace("\\\\", "\\");

            // Create new user or find existing
            if !app.users.iter().any(|u| u.name == unescaped) {
                app.users.push(User::new(&unescaped, ""));
            }
            current_user = Some(unescaped);
            current_world = None;
            current_action = None;
            in_banned_hosts = false;
            continue;
        }

        // Parse [world:NAME:OWNER] sections
        if line.starts_with("[world:") && line.ends_with(']') {
            let content = &line[7..line.len() - 1];
            // Find the last colon to split name:owner
            if let Some(last_colon) = content.rfind(':') {
                let name = &content[..last_colon];
                let owner = &content[last_colon + 1..];

                // Unescape both
                let name_unescaped = name
                    .replace("\\:", ":")
                    .replace("\\]", "]")
                    .replace("\\[", "[")
                    .replace("\\e", "=")
                    .replace("\\\\", "\\");
                let owner_unescaped = owner
                    .replace("\\:", ":")
                    .replace("\\]", "]")
                    .replace("\\[", "[")
                    .replace("\\e", "=")
                    .replace("\\\\", "\\");

                // Find or create world
                let idx = app.find_or_create_world(&name_unescaped);
                app.worlds[idx].owner = Some(owner_unescaped);
                current_world = Some(app.worlds[idx].name.clone());
            } else {
                // No owner specified - this will fail validation later
                let name_unescaped = content
                    .replace("\\]", "]")
                    .replace("\\[", "[")
                    .replace("\\e", "=")
                    .replace("\\\\", "\\");
                let idx = app.find_or_create_world(&name_unescaped);
                current_world = Some(app.worlds[idx].name.clone());
            }
            current_action = None;
            current_user = None;
            in_banned_hosts = false;
            continue;
        }

        // Parse [action:NAME:OWNER] sections
        if line.starts_with("[action:") && line.ends_with(']') {
            let content = &line[8..line.len() - 1];
            // Find the last colon to split name:owner
            if let Some(last_colon) = content.rfind(':') {
                let name = &content[..last_colon];
                let owner = &content[last_colon + 1..];

                // Unescape both
                let name_unescaped = name
                    .replace("\\:", ":")
                    .replace("\\]", "]")
                    .replace("\\[", "[")
                    .replace("\\e", "=")
                    .replace("\\\\", "\\");
                let owner_unescaped = owner
                    .replace("\\:", ":")
                    .replace("\\]", "]")
                    .replace("\\[", "[")
                    .replace("\\e", "=")
                    .replace("\\\\", "\\");

                // Find or create action
                if let Some(idx) = app.settings.actions.iter().position(|a| a.name == name_unescaped) {
                    app.settings.actions[idx].owner = Some(owner_unescaped);
                    current_action = Some(idx);
                } else {
                    let mut new_action = Action::new();
                    new_action.name = name_unescaped;
                    new_action.owner = Some(owner_unescaped);
                    app.settings.actions.push(new_action);
                    current_action = Some(app.settings.actions.len() - 1);
                }
            } else {
                // No owner specified - this will fail validation later
                let name_unescaped = content
                    .replace("\\]", "]")
                    .replace("\\[", "[")
                    .replace("\\e", "=")
                    .replace("\\\\", "\\");

                if let Some(idx) = app.settings.actions.iter().position(|a| a.name == name_unescaped) {
                    current_action = Some(idx);
                } else {
                    let mut new_action = Action::new();
                    new_action.name = name_unescaped;
                    app.settings.actions.push(new_action);
                    current_action = Some(app.settings.actions.len() - 1);
                }
            }
            current_world = None;
            current_user = None;
            in_banned_hosts = false;
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];

            // Banned hosts section
            if in_banned_hosts {
                if key == "ip" && !value.is_empty() {
                    app.ban_list.add_permanent_ban(value);
                }
                continue;
            }

            // User settings
            if let Some(ref user_name) = current_user {
                if let Some(user) = app.users.iter_mut().find(|u| &u.name == user_name) {
                    if key == "password" {
                        user.password = decrypt_password(value);
                    }
                }
            }
            // Action settings
            else if let Some(action_idx) = current_action {
                if let Some(action) = app.settings.actions.get_mut(action_idx) {
                    fn unescape_action_value(s: &str) -> String {
                        s.replace("\\\\", "\x00").replace("\\n", "\n").replace("\\e", "=").replace("\x00", "\\")
                    }
                    match key {
                        "name" => action.name = value.to_string(),
                        "world" => action.world = value.to_string(),
                        "match_type" => action.match_type = MatchType::parse(value),
                        "pattern" => action.pattern = unescape_action_value(value),
                        "command" => action.command = unescape_action_value(value),
                        "enabled" => action.enabled = value != "false",
                        "startup" => action.startup = value == "true",
                        _ => {}
                    }
                }
            }
            // World settings
            else if let Some(ref world_name) = current_world {
                if let Some(world) = app.worlds.iter_mut().find(|w| &w.name == world_name) {
                    match key {
                        "world_type" => world.settings.world_type = WorldType::from_name(value),
                        "hostname" => world.settings.hostname = value.to_string(),
                        "port" => world.settings.port = value.to_string(),
                        "user" => world.settings.user = value.to_string(),
                        "password" => world.settings.password = decrypt_password(value),
                        "use_ssl" => world.settings.use_ssl = value == "true",
                        "log_enabled" => world.settings.log_enabled = value == "true",
                        "encoding" => {
                            world.settings.encoding = match value {
                                "latin1" => Encoding::Latin1,
                                "fansi" => Encoding::Fansi,
                                _ => Encoding::Utf8,
                            };
                        }
                        "auto_connect_type" => {
                            world.settings.auto_connect_type = AutoConnectType::from_name(value);
                        }
                        "keep_alive_type" => {
                            world.settings.keep_alive_type = KeepAliveType::from_name(value);
                        }
                        "keep_alive_cmd" => {
                            world.settings.keep_alive_cmd = value.to_string();
                        }
                        "gmcp_packages" => {
                            world.settings.gmcp_packages = value.to_string();
                        }
                        _ => {}
                    }
                }
            }
            // Global settings
            else {
                match key {
                    "ws_enabled" => app.settings.ws_enabled = value == "true",
                    "ws_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    "websocket_password" => app.settings.websocket_password = decrypt_password(value),
                    "websocket_allow_list" => app.settings.websocket_allow_list = value.to_string(),
                    "websocket_cert_file" => app.settings.websocket_cert_file = value.to_string(),
                    "websocket_key_file" => app.settings.websocket_key_file = value.to_string(),
                    "web_secure" => app.settings.web_secure = value == "true",
                    "http_enabled" => app.settings.http_enabled = value == "true",
                    "http_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.http_port = p;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Save settings for multiuser mode to ~/.clay.multiuser.dat
pub fn save_multiuser_settings(app: &App) -> io::Result<()> {
    let path = get_multiuser_settings_path();
    let mut file = std::fs::File::create(&path)?;

    // [global] section
    writeln!(file, "[global]")?;
    writeln!(file, "ws_enabled={}", app.settings.ws_enabled)?;
    writeln!(file, "ws_port={}", app.settings.ws_port)?;
    if !app.settings.websocket_password.is_empty() {
        writeln!(file, "websocket_password={}", encrypt_password(&app.settings.websocket_password))?;
    }
    if !app.settings.websocket_allow_list.is_empty() {
        writeln!(file, "websocket_allow_list={}", app.settings.websocket_allow_list)?;
    }
    if !app.settings.websocket_cert_file.is_empty() {
        writeln!(file, "websocket_cert_file={}", app.settings.websocket_cert_file)?;
    }
    if !app.settings.websocket_key_file.is_empty() {
        writeln!(file, "websocket_key_file={}", app.settings.websocket_key_file)?;
    }
    writeln!(file, "web_secure={}", app.settings.web_secure)?;
    writeln!(file, "http_enabled={}", app.settings.http_enabled)?;
    writeln!(file, "http_port={}", app.settings.http_port)?;

    // [user:NAME] sections
    for user in &app.users {
        writeln!(file)?;
        let escaped_name = user.name
            .replace('\\', "\\\\")
            .replace(']', "\\]")
            .replace('[', "\\[")
            .replace('=', "\\e")
            .replace(':', "\\:");
        writeln!(file, "[user:{}]", escaped_name)?;
        writeln!(file, "password={}", encrypt_password(&user.password))?;
    }

    // [world:NAME:OWNER] sections
    for world in &app.worlds {
        if let Some(ref owner) = world.owner {
            writeln!(file)?;
            let escaped_name = world.name
                .replace('\\', "\\\\")
                .replace(']', "\\]")
                .replace('[', "\\[")
                .replace('=', "\\e")
                .replace(':', "\\:");
            let escaped_owner = owner
                .replace('\\', "\\\\")
                .replace(']', "\\]")
                .replace('[', "\\[")
                .replace('=', "\\e")
                .replace(':', "\\:");
            writeln!(file, "[world:{}:{}]", escaped_name, escaped_owner)?;
            writeln!(file, "world_type={}", world.settings.world_type.name())?;
            writeln!(file, "hostname={}", world.settings.hostname)?;
            writeln!(file, "port={}", world.settings.port)?;
            if !world.settings.user.is_empty() {
                writeln!(file, "user={}", world.settings.user)?;
            }
            if !world.settings.password.is_empty() {
                writeln!(file, "password={}", encrypt_password(&world.settings.password))?;
            }
            writeln!(file, "use_ssl={}", world.settings.use_ssl)?;
            writeln!(file, "log_enabled={}", world.settings.log_enabled)?;
            writeln!(file, "encoding={}", world.settings.encoding.name())?;
            writeln!(file, "auto_connect_type={}", world.settings.auto_connect_type.name())?;
            writeln!(file, "keep_alive_type={}", world.settings.keep_alive_type.name())?;
            if !world.settings.keep_alive_cmd.is_empty() {
                writeln!(file, "keep_alive_cmd={}", world.settings.keep_alive_cmd)?;
            }
            if world.settings.gmcp_packages != "Client.Media 1" {
                writeln!(file, "gmcp_packages={}", world.settings.gmcp_packages)?;
            }
            // Slack settings
            if !world.settings.slack_token.is_empty() {
                writeln!(file, "slack_token={}", encrypt_password(&world.settings.slack_token))?;
            }
            if !world.settings.slack_channel.is_empty() {
                writeln!(file, "slack_channel={}", world.settings.slack_channel)?;
            }
            if !world.settings.slack_workspace.is_empty() {
                writeln!(file, "slack_workspace={}", world.settings.slack_workspace)?;
            }
            // Discord settings
            if !world.settings.discord_token.is_empty() {
                writeln!(file, "discord_token={}", encrypt_password(&world.settings.discord_token))?;
            }
            if !world.settings.discord_guild.is_empty() {
                writeln!(file, "discord_guild={}", world.settings.discord_guild)?;
            }
            if !world.settings.discord_channel.is_empty() {
                writeln!(file, "discord_channel={}", world.settings.discord_channel)?;
            }
            if !world.settings.discord_dm_user.is_empty() {
                writeln!(file, "discord_dm_user={}", world.settings.discord_dm_user)?;
            }
        }
    }

    // [action:NAME:OWNER] sections
    for action in &app.settings.actions {
        if let Some(ref owner) = action.owner {
            writeln!(file)?;
            let escaped_name = action.name
                .replace('\\', "\\\\")
                .replace(']', "\\]")
                .replace('[', "\\[")
                .replace('=', "\\e")
                .replace(':', "\\:");
            let escaped_owner = owner
                .replace('\\', "\\\\")
                .replace(']', "\\]")
                .replace('[', "\\[")
                .replace('=', "\\e")
                .replace(':', "\\:");
            writeln!(file, "[action:{}:{}]", escaped_name, escaped_owner)?;
            if !action.world.is_empty() {
                writeln!(file, "world={}", action.world)?;
            }
            writeln!(file, "match_type={}", action.match_type.as_str().to_lowercase())?;
            // Escape special chars in pattern and command
            let escaped_pattern = action.pattern
                .replace('\\', "\\\\")
                .replace('=', "\\e")
                .replace('\n', "\\n");
            let escaped_command = action.command
                .replace('\\', "\\\\")
                .replace('=', "\\e")
                .replace('\n', "\\n");
            writeln!(file, "pattern={}", escaped_pattern)?;
            writeln!(file, "command={}", escaped_command)?;
            if !action.enabled {
                writeln!(file, "enabled=false")?;
            }
            if action.startup {
                writeln!(file, "startup=true")?;
            }
        }
    }

    // [banned_hosts] section
    let permanent_bans = app.ban_list.get_permanent_bans();
    if !permanent_bans.is_empty() {
        writeln!(file)?;
        writeln!(file, "[banned_hosts]")?;
        for ip in permanent_bans {
            writeln!(file, "ip={}", ip)?;
        }
    }

    Ok(())
}

#[cfg(all(unix, not(target_os = "android")))]
pub fn save_reload_state(app: &App) -> io::Result<()> {
    let path = get_reload_state_path();
    let mut file = std::fs::File::create(&path)?;

    // Save global state
    writeln!(file, "[reload]")?;
    writeln!(file, "current_world_index={}", app.current_world_index)?;
    writeln!(file, "input_height={}", app.input_height)?;
    writeln!(file, "more_mode={}", app.settings.more_mode_enabled)?;
    writeln!(file, "spell_check={}", app.settings.spell_check_enabled)?;
    writeln!(file, "temp_convert={}", app.settings.temp_convert_enabled)?;
    writeln!(file, "world_switch_mode={}", app.settings.world_switch_mode.name())?;
    writeln!(file, "debug_enabled={}", app.settings.debug_enabled)?;
    writeln!(file, "ansi_music_enabled={}", app.settings.ansi_music_enabled)?;
    writeln!(file, "show_tags={}", app.show_tags)?;
    writeln!(file, "theme={}", app.settings.theme.name())?;
    writeln!(file, "gui_theme={}", app.settings.gui_theme.name())?;
    writeln!(file, "gui_transparency={}", app.settings.gui_transparency)?;
    writeln!(file, "color_offset_percent={}", app.settings.color_offset_percent)?;
    writeln!(file, "font_name={}", app.settings.font_name)?;
    writeln!(file, "font_size={}", app.settings.font_size)?;
    writeln!(file, "web_font_size_phone={}", app.settings.web_font_size_phone)?;
    writeln!(file, "web_font_size_tablet={}", app.settings.web_font_size_tablet)?;
    writeln!(file, "web_font_size_desktop={}", app.settings.web_font_size_desktop)?;
    writeln!(file, "web_secure={}", app.settings.web_secure)?;
    writeln!(file, "http_enabled={}", app.settings.http_enabled)?;
    writeln!(file, "http_port={}", app.settings.http_port)?;
    writeln!(file, "ws_enabled={}", app.settings.ws_enabled)?;
    writeln!(file, "ws_port={}", app.settings.ws_port)?;
    if !app.settings.websocket_password.is_empty() {
        writeln!(file, "websocket_password={}", encrypt_password(&app.settings.websocket_password))?;
    }
    if !app.settings.websocket_allow_list.is_empty() {
        writeln!(file, "websocket_allow_list={}", app.settings.websocket_allow_list)?;
    }
    // Get whitelisted_host from running server, or from settings
    let whitelisted_host = if let Some(ref server) = app.ws_server {
        server.get_whitelisted_host()
    } else {
        app.settings.websocket_whitelisted_host.clone()
    };
    if let Some(ref host) = whitelisted_host {
        writeln!(file, "websocket_whitelisted_host={}", host)?;
    }
    if !app.settings.websocket_cert_file.is_empty() {
        writeln!(file, "websocket_cert_file={}", app.settings.websocket_cert_file)?;
    }
    if !app.settings.websocket_key_file.is_empty() {
        writeln!(file, "websocket_key_file={}", app.settings.websocket_key_file)?;
    }
    writeln!(file, "tls_proxy_enabled={}", app.settings.tls_proxy_enabled)?;
    if !app.settings.dictionary_path.is_empty() {
        writeln!(file, "dictionary_path={}", app.settings.dictionary_path)?;
    }
    writeln!(file, "editor_side={}", app.settings.editor_side.name())?;

    // Save input history (base64 encode each line to handle special chars)
    writeln!(file, "history_count={}", app.input.history.len())?;
    for (i, hist) in app.input.history.iter().enumerate() {
        // Simple escape: replace newlines and = with escape sequences
        let escaped = hist.replace('\\', "\\\\").replace('\n', "\\n").replace('=', "\\e");
        writeln!(file, "history_{}={}", i, escaped)?;
    }

    // Save each world's state
    writeln!(file, "world_count={}", app.worlds.len())?;
    for (idx, world) in app.worlds.iter().enumerate() {
        writeln!(file)?;
        writeln!(file, "[world_state:{}]", idx)?;
        writeln!(file, "name={}", world.name.replace('=', "\\e"))?;
        writeln!(file, "scroll_offset={}", world.scroll_offset)?;
        writeln!(file, "connected={}", world.connected)?;
        writeln!(file, "unseen_lines={}", world.unseen_lines)?;
        writeln!(file, "paused={}", world.paused)?;
        writeln!(file, "lines_since_pause={}", world.lines_since_pause)?;
        writeln!(file, "is_tls={}", world.is_tls)?;
        writeln!(file, "was_connected={}", world.was_connected)?;
        writeln!(file, "telnet_mode={}", world.telnet_mode)?;
        writeln!(file, "uses_wont_echo_prompt={}", world.uses_wont_echo_prompt)?;
        writeln!(file, "next_seq={}", world.next_seq)?;
        if !world.prompt.is_empty() {
            writeln!(file, "prompt={}", world.prompt.replace('=', "\\e"))?;
        }

        // Socket fd if connected (will be passed via env var separately)
        if let Some(fd) = world.socket_fd {
            writeln!(file, "socket_fd={}", fd)?;
        }

        // TLS proxy info (for connection preservation over hot reload)
        if let Some(proxy_pid) = world.proxy_pid {
            writeln!(file, "proxy_pid={}", proxy_pid)?;
        }
        if let Some(ref proxy_socket_path) = world.proxy_socket_path {
            writeln!(file, "proxy_socket_path={}", proxy_socket_path.display())?;
        }
        #[cfg(unix)]
        if let Some(fd) = world.proxy_socket_fd {
            writeln!(file, "proxy_socket_fd={}", fd)?;
        }

        // World settings
        writeln!(file, "world_type={}", world.settings.world_type.name())?;
        writeln!(file, "hostname={}", world.settings.hostname)?;
        writeln!(file, "port={}", world.settings.port)?;
        writeln!(file, "user={}", world.settings.user.replace('=', "\\e"))?;
        writeln!(file, "password={}", world.settings.password.replace('=', "\\e"))?;
        writeln!(file, "use_ssl={}", world.settings.use_ssl)?;
        writeln!(file, "encoding={}", world.settings.encoding.name())?;
        writeln!(file, "auto_connect_type={}", world.settings.auto_connect_type.name())?;
        writeln!(file, "keep_alive_type={}", world.settings.keep_alive_type.name())?;
        if !world.settings.keep_alive_cmd.is_empty() {
            writeln!(file, "keep_alive_cmd={}", world.settings.keep_alive_cmd.replace('=', "\\e"))?;
        }
        if world.settings.gmcp_packages != "Client.Media 1" {
            writeln!(file, "gmcp_packages={}", world.settings.gmcp_packages.replace('=', "\\e"))?;
        }
        // Save GMCP/MSDP runtime state
        if world.gmcp_enabled {
            writeln!(file, "gmcp_enabled=true")?;
        }
        if world.msdp_enabled {
            writeln!(file, "msdp_enabled=true")?;
        }
        if !world.mcmp_default_url.is_empty() {
            writeln!(file, "mcmp_default_url={}", world.mcmp_default_url.replace('=', "\\e"))?;
        }
        if world.gmcp_user_enabled {
            writeln!(file, "gmcp_user_enabled=true")?;
        }
        if world.settings.log_enabled {
            writeln!(file, "log_enabled=true")?;
        }
        // Slack settings
        if !world.settings.slack_token.is_empty() {
            writeln!(file, "slack_token={}", world.settings.slack_token.replace('=', "\\e"))?;
        }
        if !world.settings.slack_channel.is_empty() {
            writeln!(file, "slack_channel={}", world.settings.slack_channel.replace('=', "\\e"))?;
        }
        if !world.settings.slack_workspace.is_empty() {
            writeln!(file, "slack_workspace={}", world.settings.slack_workspace.replace('=', "\\e"))?;
        }
        // Discord settings
        if !world.settings.discord_token.is_empty() {
            writeln!(file, "discord_token={}", world.settings.discord_token.replace('=', "\\e"))?;
        }
        if !world.settings.discord_guild.is_empty() {
            writeln!(file, "discord_guild={}", world.settings.discord_guild.replace('=', "\\e"))?;
        }
        if !world.settings.discord_channel.is_empty() {
            writeln!(file, "discord_channel={}", world.settings.discord_channel.replace('=', "\\e"))?;
        }
        if !world.settings.discord_dm_user.is_empty() {
            writeln!(file, "discord_dm_user={}", world.settings.discord_dm_user.replace('=', "\\e"))?;
        }
        // Notes (escape special chars)
        if !world.settings.notes.is_empty() {
            let escaped_notes = world.settings.notes
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('=', "\\e");
            writeln!(file, "notes={}", escaped_notes)?;
        }

        // Partial line state (for preserving incomplete lines across reload)
        if !world.partial_line.is_empty() {
            let escaped = world.partial_line
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('=', "\\e");
            writeln!(file, "partial_line={}", escaped)?;
            writeln!(file, "partial_in_pending={}", world.partial_in_pending)?;
        }

        // Output lines count (we'll save the actual lines separately due to size)
        writeln!(file, "output_count={}", world.output_lines.len())?;
        writeln!(file, "pending_count={}", world.pending_lines.len())?;
    }

    // Save output lines in a separate section (can be large)
    // Format: timestamp_secs|flags|seq|escaped_text
    // Flags: s = from_server (omit if false), g = gagged (omit if false)
    for (idx, world) in app.worlds.iter().enumerate() {
        writeln!(file)?;
        writeln!(file, "[output:{}]", idx)?;
        for line in &world.output_lines {
            let ts_secs = line.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut flags = String::new();
            if line.from_server { flags.push('s'); }
            if line.gagged { flags.push('g'); }
            let escaped = line.text.replace('\\', "\\\\").replace('\n', "\\n");
            writeln!(file, "{}|{}|{}|{}", ts_secs, flags, line.seq, escaped)?;
        }
        writeln!(file, "[pending:{}]", idx)?;
        for line in &world.pending_lines {
            let ts_secs = line.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut flags = String::new();
            if line.from_server { flags.push('s'); }
            if line.gagged { flags.push('g'); }
            let escaped = line.text.replace('\\', "\\\\").replace('\n', "\\n");
            writeln!(file, "{}|{}|{}|{}", ts_secs, flags, line.seq, escaped)?;
        }
    }

    // Save actions (by name, escaping special characters)
    for action in app.settings.actions.iter() {
        writeln!(file)?;
        // Escape special chars in name for section header: ] [ = \
        let escaped_name = action.name
            .replace('\\', "\\\\")
            .replace(']', "\\]")
            .replace('[', "\\[")
            .replace('=', "\\e");
        writeln!(file, "[action:{}]", escaped_name)?;
        if !action.world.is_empty() {
            writeln!(file, "world={}", action.world)?;
        }
        // Only save match_type if not the default (regexp)
        if action.match_type != MatchType::Regexp {
            writeln!(file, "match_type={}", action.match_type.as_str().to_lowercase())?;
        }
        if !action.pattern.is_empty() {
            writeln!(file, "pattern={}", action.pattern.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
        }
        if !action.command.is_empty() {
            writeln!(file, "command={}", action.command.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
        }
        if !action.enabled {
            writeln!(file, "enabled=false")?;
        }
        if action.startup {
            writeln!(file, "startup=true")?;
        }
    }

    Ok(())
}

pub fn unescape_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('e') => result.push('='),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn load_reload_state(app: &mut App) -> io::Result<bool> {
    debug_log(true, "LOAD_STATE: Starting load_reload_state");
    let path = get_reload_state_path();
    if !path.exists() {
        debug_log(true, "LOAD_STATE: No state file found");
        return Ok(false);
    }

    debug_log(true, &format!("LOAD_STATE: Reading state file: {:?}", path));
    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();
    debug_log(true, &format!("LOAD_STATE: State file has {} lines", lines.len()));

    // Parse the reload state
    let mut current_section = String::new();
    let mut current_world_idx: Option<usize> = None;
    let mut output_world_idx: Option<usize> = None;
    let mut pending_world_idx: Option<usize> = None;
    let mut current_action_idx: Option<usize> = None;

    // Temporary storage for world data
    struct TempWorld {
        name: String,
        output_lines: Vec<OutputLine>,
        scroll_offset: usize,
        connected: bool,
        #[cfg(unix)]
        socket_fd: Option<RawFd>,
        #[cfg(not(unix))]
        socket_fd: Option<i64>,
        proxy_pid: Option<u32>,
        proxy_socket_path: Option<PathBuf>,
        #[cfg(unix)]
        proxy_socket_fd: Option<RawFd>,
        #[cfg(not(unix))]
        proxy_socket_fd: Option<i64>,
        unseen_lines: usize,
        paused: bool,
        pending_lines: Vec<OutputLine>,
        lines_since_pause: usize,
        is_tls: bool,
        was_connected: bool,
        telnet_mode: bool,
        uses_wont_echo_prompt: bool,
        prompt: String,
        settings: WorldSettings,
        next_seq: u64,
        partial_line: String,
        partial_in_pending: bool,
        gmcp_enabled: bool,
        msdp_enabled: bool,
        mcmp_default_url: String,
        gmcp_user_enabled: bool,
    }

    // Parse a saved output/pending line with timestamp
    // Newest format: timestamp_secs|flags|seq|text (flags: s=from_server, g=gagged)
    // Older format: timestamp_secs|flags|text (flags: s=from_server, g=gagged) - seq=0
    // Old format: timestamp_secs|text (for backward compatibility) - seq=0
    fn parse_timestamped_line(line: &str) -> OutputLine {
        let parts: Vec<&str> = line.splitn(4, '|').collect();

        if parts.len() >= 2 {
            if let Ok(ts_secs) = parts[0].parse::<u64>() {
                let timestamp = UNIX_EPOCH + Duration::from_secs(ts_secs);

                if parts.len() == 4 {
                    // Newest format: timestamp|flags|seq|text
                    let flags = parts[1];
                    let seq = parts[2].parse::<u64>().unwrap_or(0);
                    let text = unescape_string(parts[3]);
                    let from_server = flags.contains('s');
                    let gagged = flags.contains('g');
                    return OutputLine {
                        text,
                        timestamp,
                        from_server,
                        gagged,
                        seq,
                        highlight_color: None,
                    };
                } else if parts.len() == 3 {
                    // Older format: timestamp|flags|text (no seq)
                    let flags = parts[1];
                    let text = unescape_string(parts[2]);
                    let from_server = flags.contains('s');
                    let gagged = flags.contains('g');
                    return OutputLine {
                        text,
                        timestamp,
                        from_server,
                        gagged,
                        seq: 0,
                        highlight_color: None,
                    };
                } else {
                    // Old format: timestamp|text (assume from_server=true for compatibility)
                    return OutputLine {
                        text: unescape_string(parts[1]),
                        timestamp,
                        from_server: true,
                        gagged: false,
                        seq: 0,
                        highlight_color: None,
                    };
                }
            }
        }
        // Fallback: no timestamp in old format, use current time
        OutputLine::new(unescape_string(line), 0)
    }

    let mut temp_worlds: Vec<TempWorld> = Vec::new();

    for line in lines {
        // Check for section headers FIRST (before output/pending line handling)
        // This prevents section headers from being added as output lines
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = &trimmed[1..trimmed.len() - 1];
            if section == "reload" {
                current_section = "reload".to_string();
            } else if let Some(suffix) = section.strip_prefix("world_state:") {
                let idx: usize = suffix.parse().unwrap_or(0);
                current_section = "world_state".to_string();
                current_world_idx = Some(idx);
                // Ensure we have enough temp worlds
                while temp_worlds.len() <= idx {
                    temp_worlds.push(TempWorld {
                        name: String::new(),
                        output_lines: Vec::new(),
                        scroll_offset: 0,
                        connected: false,
                        socket_fd: None,
                        proxy_pid: None,
                        proxy_socket_path: None,
                        proxy_socket_fd: None,
                        unseen_lines: 0,
                        paused: false,
                        pending_lines: Vec::new(),
                        lines_since_pause: 0,
                        is_tls: false,
                        was_connected: false,
                        telnet_mode: false,
                        uses_wont_echo_prompt: false,
                        prompt: String::new(),
                        settings: WorldSettings::default(),
                        next_seq: 0,
                        partial_line: String::new(),
                        partial_in_pending: false,
                        gmcp_enabled: false,
                        msdp_enabled: false,
                        mcmp_default_url: String::new(),
                        gmcp_user_enabled: false,
                    });
                }
            } else if let Some(suffix) = section.strip_prefix("output:") {
                let idx: usize = suffix.parse().unwrap_or(0);
                current_section = "output".to_string();
                output_world_idx = Some(idx);
                pending_world_idx = None;
            } else if let Some(suffix) = section.strip_prefix("pending:") {
                let idx: usize = suffix.parse().unwrap_or(0);
                current_section = "pending".to_string();
                pending_world_idx = Some(idx);
                output_world_idx = None;
            } else if let Some(suffix) = section.strip_prefix("action:") {
                // Parse action section - supports both old format [action:NUMBER] and new format [action:NAME]
                current_section = "action".to_string();

                // Unescape the section content (for new format names with special chars)
                let unescaped = suffix
                    .replace("\\]", "]")
                    .replace("\\[", "[")
                    .replace("\\e", "=")
                    .replace("\\\\", "\\");

                // Check if it's old format (pure number) or new format (name)
                let is_old_format = unescaped.chars().all(|c| c.is_ascii_digit());

                if is_old_format {
                    // Old format: create new action, will get name from name= field
                    app.settings.actions.push(Action::new());
                    current_action_idx = Some(app.settings.actions.len() - 1);
                } else {
                    // New format: look for existing action with this name or create new
                    let action_name = unescaped;
                    if let Some(idx) = app.settings.actions.iter().position(|a| a.name == action_name) {
                        current_action_idx = Some(idx);
                    } else {
                        let mut new_action = Action::new();
                        new_action.name = action_name;
                        app.settings.actions.push(new_action);
                        current_action_idx = Some(app.settings.actions.len() - 1);
                    }
                }
            }
            continue;
        }

        // Handle output/pending lines without trimming to preserve leading spaces
        // Skip empty lines (blank line separators between [output:] and [pending:] sections)
        if current_section == "output" {
            if let Some(idx) = output_world_idx {
                if idx < temp_worlds.len() && !line.is_empty() {
                    temp_worlds[idx].output_lines.push(parse_timestamped_line(line));
                }
            }
            continue;
        }
        if current_section == "pending" {
            if let Some(idx) = pending_world_idx {
                if idx < temp_worlds.len() && !line.is_empty() {
                    temp_worlds[idx].pending_lines.push(parse_timestamped_line(line));
                }
            }
            continue;
        }

        // For non-output sections, trim whitespace
        let line = trimmed;
        if line.is_empty() {
            continue;
        }

        // Parse key=value
        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];

            if current_section == "reload" {
                match key {
                    "current_world_index" => {
                        app.current_world_index = value.parse().unwrap_or(0);
                    }
                    "input_height" => {
                        app.input_height = value.parse().unwrap_or(3);
                        app.input.visible_height = app.input_height;
                    }
                    "more_mode" => {
                        app.settings.more_mode_enabled = value == "true";
                    }
                    "spell_check" => {
                        app.settings.spell_check_enabled = value == "true";
                    }
                    "temp_convert" => {
                        app.settings.temp_convert_enabled = value == "true";
                    }
                    "pending_first" => {
                        // Backward compatibility: pending_first=true -> UnseenFirst
                        app.settings.world_switch_mode = if value == "true" {
                            WorldSwitchMode::UnseenFirst
                        } else {
                            WorldSwitchMode::Alphabetical
                        };
                    }
                    "world_switch_mode" => {
                        app.settings.world_switch_mode = WorldSwitchMode::from_name(value);
                    }
                    "debug_enabled" => {
                        app.settings.debug_enabled = value == "true";
                    }
                    "ansi_music_enabled" => {
                        app.settings.ansi_music_enabled = value == "true";
                    }
                    "show_tags" => {
                        app.show_tags = value == "true";
                    }
                    "theme" => {
                        app.settings.theme = Theme::from_name(value);
                    }
                    "gui_theme" => {
                        app.settings.gui_theme = Theme::from_name(value);
                    }
                    "font_name" => {
                        app.settings.font_name = value.to_string();
                    }
                    "font_size" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.font_size = s.clamp(8.0, 48.0);
                        }
                    }
                    // Backward compat: old single web_font_size sets all three
                    "web_font_size" => {
                        if let Ok(s) = value.parse::<f32>() {
                            let clamped = s.clamp(8.0, 48.0);
                            app.settings.web_font_size_phone = clamped;
                            app.settings.web_font_size_tablet = clamped;
                            app.settings.web_font_size_desktop = clamped;
                        }
                    }
                    "web_font_size_phone" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.web_font_size_phone = s.clamp(8.0, 48.0);
                        }
                    }
                    "web_font_size_tablet" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.web_font_size_tablet = s.clamp(8.0, 48.0);
                        }
                    }
                    "web_font_size_desktop" => {
                        if let Ok(s) = value.parse::<f32>() {
                            app.settings.web_font_size_desktop = s.clamp(8.0, 48.0);
                        }
                    }
                    "gui_transparency" => {
                        if let Ok(t) = value.parse::<f32>() {
                            app.settings.gui_transparency = t.clamp(0.3, 1.0);
                        }
                    }
                    "color_offset_percent" => {
                        if let Ok(p) = value.parse::<u8>() {
                            app.settings.color_offset_percent = p.min(100);
                        }
                    }
                    "web_secure" => {
                        app.settings.web_secure = value == "true";
                    }
                    "ws_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    "ws_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_enabled maps to ws_enabled
                    "websocket_enabled" => {
                        app.settings.ws_enabled = value == "true";
                    }
                    // Legacy: websocket_port maps to ws_port
                    "websocket_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.ws_port = p;
                        }
                    }
                    // Legacy: websocket_use_tls maps to web_secure
                    "websocket_use_tls" => {
                        app.settings.web_secure = value == "true";
                    }
                    "websocket_password" => {
                        app.settings.websocket_password = decrypt_password(value);
                    }
                    "websocket_allow_list" => {
                        app.settings.websocket_allow_list = value.to_string();
                    }
                    "websocket_whitelisted_host" => {
                        app.settings.websocket_whitelisted_host = Some(value.to_string());
                    }
                    "websocket_cert_file" => {
                        app.settings.websocket_cert_file = value.to_string();
                    }
                    "websocket_key_file" => {
                        app.settings.websocket_key_file = value.to_string();
                    }
                    "http_enabled" => {
                        app.settings.http_enabled = value == "true";
                    }
                    "http_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            app.settings.http_port = p;
                        }
                    }
                    // Legacy fields
                    "https_enabled" => {
                        if value == "true" {
                            app.settings.http_enabled = true;
                            app.settings.web_secure = true;
                        }
                    }
                    "https_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            if app.settings.web_secure {
                                app.settings.http_port = p;
                            }
                        }
                    }
                    "ws_nonsecure_enabled" => {
                        if value == "true" && !app.settings.web_secure {
                            app.settings.ws_enabled = true;
                        }
                    }
                    "ws_nonsecure_port" => {
                        if let Ok(p) = value.parse::<u16>() {
                            if !app.settings.web_secure {
                                app.settings.ws_port = p;
                            }
                        }
                    }
                    // Legacy: ignore global encoding, it's now per-world
                    "encoding" => {}
                    "tls_proxy_enabled" => {
                        app.settings.tls_proxy_enabled = value == "true";
                    }
                    "dictionary_path" => {
                        app.settings.dictionary_path = value.to_string();
                    }
                    "editor_side" => {
                        app.settings.editor_side = EditorSide::from_name(value);
                    }
                    "history_count" | "world_count" => {
                        // These are informational, not needed for parsing
                    }
                    k if k.starts_with("history_") => {
                        app.input.history.push(unescape_string(value));
                    }
                    _ => {}
                }
            } else if current_section == "world_state" {
                if let Some(idx) = current_world_idx {
                    if idx < temp_worlds.len() {
                        let tw = &mut temp_worlds[idx];
                        match key {
                            "name" => tw.name = unescape_string(value),
                            "scroll_offset" => tw.scroll_offset = value.parse().unwrap_or(0),
                            "connected" => tw.connected = value == "true",
                            "unseen_lines" => tw.unseen_lines = value.parse().unwrap_or(0),
                            "paused" => tw.paused = value == "true",
                            "lines_since_pause" => tw.lines_since_pause = value.parse().unwrap_or(0),
                            "is_tls" => tw.is_tls = value == "true",
                            "was_connected" => tw.was_connected = value == "true",
                            "telnet_mode" => tw.telnet_mode = value == "true",
                            "uses_wont_echo_prompt" => tw.uses_wont_echo_prompt = value == "true",
                            "prompt" => {
                                // Prompts always end with a single trailing space (normalized on receive)
                                // but trailing spaces are trimmed during file parsing, so add it back
                                let p = unescape_string(value);
                                tw.prompt = if p.is_empty() { p } else { format!("{} ", p.trim_end()) };
                            }
                            "socket_fd" => tw.socket_fd = value.parse().ok(),
                            "proxy_pid" => tw.proxy_pid = value.parse().ok(),
                            "proxy_socket_path" => tw.proxy_socket_path = Some(PathBuf::from(value)),
                            "proxy_socket_fd" => tw.proxy_socket_fd = value.parse().ok(),
                            "next_seq" => tw.next_seq = value.parse().unwrap_or(0),
                            "partial_line" => tw.partial_line = unescape_string(value),
                            "partial_in_pending" => tw.partial_in_pending = value == "true",
                            "world_type" => tw.settings.world_type = WorldType::from_name(value),
                            "hostname" => tw.settings.hostname = value.to_string(),
                            "port" => tw.settings.port = value.to_string(),
                            "user" => tw.settings.user = unescape_string(value),
                            "password" => tw.settings.password = unescape_string(value),
                            "use_ssl" => tw.settings.use_ssl = value == "true",
                            "log_enabled" => tw.settings.log_enabled = value == "true",
                            "log_file" => tw.settings.log_enabled = true, // Backward compat
                            "encoding" => {
                                tw.settings.encoding = match value {
                                    "latin1" => Encoding::Latin1,
                                    "fansi" => Encoding::Fansi,
                                    _ => Encoding::Utf8,
                                };
                            }
                            "auto_connect_type" => {
                                tw.settings.auto_connect_type = AutoConnectType::from_name(value);
                            }
                            "keep_alive_type" => {
                                tw.settings.keep_alive_type = KeepAliveType::from_name(value);
                            }
                            "keep_alive_cmd" => {
                                tw.settings.keep_alive_cmd = value.replace("\\e", "=");
                            }
                            "gmcp_packages" => {
                                tw.settings.gmcp_packages = unescape_string(value);
                            }
                            "gmcp_enabled" => {
                                tw.gmcp_enabled = value == "true";
                            }
                            "msdp_enabled" => {
                                tw.msdp_enabled = value == "true";
                            }
                            "mcmp_default_url" => {
                                tw.mcmp_default_url = unescape_string(value);
                            }
                            "gmcp_user_enabled" => {
                                tw.gmcp_user_enabled = value == "true";
                            }
                            // Slack settings
                            "slack_token" => tw.settings.slack_token = unescape_string(value),
                            "slack_channel" => tw.settings.slack_channel = unescape_string(value),
                            "slack_workspace" => tw.settings.slack_workspace = unescape_string(value),
                            // Discord settings
                            "discord_token" => tw.settings.discord_token = unescape_string(value),
                            "discord_guild" => tw.settings.discord_guild = unescape_string(value),
                            "discord_channel" => tw.settings.discord_channel = unescape_string(value),
                            "discord_dm_user" => tw.settings.discord_dm_user = unescape_string(value),
                            // Notes
                            "notes" => tw.settings.notes = unescape_string(value),
                            _ => {}
                        }
                    }
                }
            } else if current_section == "action" {
                // Action settings
                if let Some(action_idx) = current_action_idx {
                    if let Some(action) = app.settings.actions.get_mut(action_idx) {
                        // Helper to unescape saved strings
                        fn unescape_action_value(s: &str) -> String {
                            s.replace("\\\\", "\x00").replace("\\n", "\n").replace("\\e", "=").replace("\x00", "\\")
                        }
                        match key {
                            "name" => action.name = value.to_string(),
                            "world" => action.world = value.to_string(),
                            "match_type" => action.match_type = MatchType::parse(value),
                            "pattern" => action.pattern = unescape_action_value(value),
                            "command" => action.command = unescape_action_value(value),
                            "enabled" => action.enabled = value != "false",
                            "startup" => action.startup = value == "true",
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Convert temp worlds to real worlds
    app.worlds.clear();
    for tw in temp_worlds {
        let mut world = World::new(&tw.name);
        world.output_lines = tw.output_lines;
        world.scroll_offset = tw.scroll_offset;
        world.connected = tw.connected;
        world.unseen_lines = tw.unseen_lines;
        world.paused = tw.paused;
        world.pending_lines = tw.pending_lines;
        world.lines_since_pause = tw.lines_since_pause;
        world.is_tls = tw.is_tls;
        world.was_connected = tw.was_connected;
        world.telnet_mode = tw.telnet_mode;
        world.uses_wont_echo_prompt = tw.uses_wont_echo_prompt;
        world.prompt = tw.prompt;
        world.socket_fd = tw.socket_fd;
        world.proxy_pid = tw.proxy_pid;
        world.proxy_socket_path = tw.proxy_socket_path;
        world.proxy_socket_fd = tw.proxy_socket_fd;
        world.settings = tw.settings;
        world.next_seq = tw.next_seq;
        world.partial_line = tw.partial_line;
        world.partial_in_pending = tw.partial_in_pending;
        world.gmcp_enabled = tw.gmcp_enabled;
        world.msdp_enabled = tw.msdp_enabled;
        world.mcmp_default_url = tw.mcmp_default_url;
        world.gmcp_user_enabled = tw.gmcp_user_enabled;
        // Leave timing fields as None for connected worlds after reload
        // This triggers immediate keepalive since we don't know how long connection was idle
        app.worlds.push(world);
    }

    // Note: Don't create initial world here - let ensure_has_world() handle it after
    // settings are fully loaded, to avoid creating unnecessary "clay" world

    // Validate current_world_index
    if app.current_world_index >= app.worlds.len() {
        app.current_world_index = 0;
    }

    // Clean up the reload state file
    let _ = std::fs::remove_file(&path);

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: set ALL Settings fields to non-default values.
    /// Uses explicit struct construction — if a new field is added to Settings,
    /// this function will fail to compile until updated here AND in the assertions.
    fn make_non_default_settings() -> Settings {
        Settings {
            more_mode_enabled: false,          // default: true
            spell_check_enabled: false,        // default: true
            temp_convert_enabled: true,        // default: false
            world_switch_mode: WorldSwitchMode::Alphabetical, // default: UnseenFirst
            debug_enabled: true,               // default: false
            ansi_music_enabled: false,         // default: true
            theme: Theme::Light,               // default: Dark
            gui_theme: Theme::Light,           // default: Dark
            gui_transparency: 0.7,             // default: 1.0
            color_offset_percent: 42,          // default: 0
            font_name: "TestFont".to_string(), // default: ""
            font_size: 18.0,                   // default: 14.0
            web_font_size_phone: 12.0,         // default: 10.0
            web_font_size_tablet: 16.0,        // default: 14.0
            web_font_size_desktop: 20.0,       // default: 18.0
            web_secure: true,                  // default: false
            http_enabled: true,                // default: false
            http_port: 8080,                   // default: 9000
            ws_enabled: true,                  // default: false
            ws_port: 8081,                     // default: 9001
            websocket_password: "testpass".to_string(),     // default: ""
            websocket_allow_list: "192.168.1.1".to_string(), // default: ""
            websocket_whitelisted_host: Some("10.0.0.1".to_string()), // default: None (not persisted to .clay.dat)
            websocket_cert_file: "/tmp/cert.pem".to_string(), // default: ""
            websocket_key_file: "/tmp/key.pem".to_string(),   // default: ""
            websocket_auth_keys: vec!["key1".to_string(), "key2".to_string()], // default: []
            actions: vec![
                {
                    let mut a = Action::new();
                    a.name = "test_action".to_string();
                    a.pattern = "^test.*pattern$".to_string();
                    a.command = "/echo matched".to_string();
                    a.world = "testworld".to_string();
                    a.match_type = crate::actions::MatchType::Wildcard;
                    a.enabled = false;
                    a.startup = true;
                    a
                },
            ],
            tls_proxy_enabled: true,           // default: false
            dictionary_path: "/custom/dict".to_string(), // default: ""
            editor_side: EditorSide::Right,    // default: Left
        }
    }

    /// Helper: set ALL WorldSettings fields to non-default values.
    fn make_non_default_world_settings() -> WorldSettings {
        WorldSettings {
            world_type: WorldType::Mud,
            hostname: "mud.example.com".to_string(),
            port: "4000".to_string(),
            user: "testuser".to_string(),
            password: "testpassword".to_string(),
            use_ssl: true,                             // default: false
            log_enabled: true,                         // default: false
            encoding: Encoding::Latin1,                // default: Utf8
            auto_connect_type: AutoConnectType::Prompt, // default: Connect
            keep_alive_type: KeepAliveType::Custom,    // default: Nop
            keep_alive_cmd: "keepalive_cmd".to_string(), // default: ""
            slack_token: "slack_tok".to_string(),
            slack_channel: "slack_chan".to_string(),
            slack_workspace: "slack_ws".to_string(),
            discord_token: "disc_tok".to_string(),
            discord_guild: "disc_guild".to_string(),
            discord_channel: "disc_chan".to_string(),
            discord_dm_user: "disc_dm".to_string(),
            notes: "test notes\nline two".to_string(),
            gmcp_packages: "Custom.Package 1".to_string(), // default: "Client.Media 1"
        }
    }

    /// Assert all Settings fields match between two instances.
    /// Explicitly checks every field — if a new field is added, this must be updated.
    fn assert_settings_match(a: &Settings, b: &Settings, context: &str) {
        assert_eq!(a.more_mode_enabled, b.more_mode_enabled, "{context}: more_mode_enabled");
        assert_eq!(a.spell_check_enabled, b.spell_check_enabled, "{context}: spell_check_enabled");
        assert_eq!(a.temp_convert_enabled, b.temp_convert_enabled, "{context}: temp_convert_enabled");
        assert_eq!(a.world_switch_mode.name(), b.world_switch_mode.name(), "{context}: world_switch_mode");
        assert_eq!(a.debug_enabled, b.debug_enabled, "{context}: debug_enabled");
        assert_eq!(a.ansi_music_enabled, b.ansi_music_enabled, "{context}: ansi_music_enabled");
        assert_eq!(a.theme.name(), b.theme.name(), "{context}: theme");
        assert_eq!(a.gui_theme.name(), b.gui_theme.name(), "{context}: gui_theme");
        assert_eq!(a.gui_transparency, b.gui_transparency, "{context}: gui_transparency");
        assert_eq!(a.color_offset_percent, b.color_offset_percent, "{context}: color_offset_percent");
        assert_eq!(a.font_name, b.font_name, "{context}: font_name");
        assert_eq!(a.font_size, b.font_size, "{context}: font_size");
        assert_eq!(a.web_font_size_phone, b.web_font_size_phone, "{context}: web_font_size_phone");
        assert_eq!(a.web_font_size_tablet, b.web_font_size_tablet, "{context}: web_font_size_tablet");
        assert_eq!(a.web_font_size_desktop, b.web_font_size_desktop, "{context}: web_font_size_desktop");
        assert_eq!(a.web_secure, b.web_secure, "{context}: web_secure");
        assert_eq!(a.http_enabled, b.http_enabled, "{context}: http_enabled");
        assert_eq!(a.http_port, b.http_port, "{context}: http_port");
        assert_eq!(a.ws_enabled, b.ws_enabled, "{context}: ws_enabled");
        assert_eq!(a.ws_port, b.ws_port, "{context}: ws_port");
        assert_eq!(a.websocket_password, b.websocket_password, "{context}: websocket_password");
        assert_eq!(a.websocket_allow_list, b.websocket_allow_list, "{context}: websocket_allow_list");
        // websocket_whitelisted_host is not persisted to .clay.dat (runtime state)
        assert_eq!(a.websocket_cert_file, b.websocket_cert_file, "{context}: websocket_cert_file");
        assert_eq!(a.websocket_key_file, b.websocket_key_file, "{context}: websocket_key_file");
        assert_eq!(a.websocket_auth_keys, b.websocket_auth_keys, "{context}: websocket_auth_keys");
        assert_eq!(a.actions.len(), b.actions.len(), "{context}: actions.len()");
        for (i, (aa, bb)) in a.actions.iter().zip(b.actions.iter()).enumerate() {
            assert_eq!(aa.name, bb.name, "{context}: action[{i}].name");
            assert_eq!(aa.pattern, bb.pattern, "{context}: action[{i}].pattern");
            assert_eq!(aa.command, bb.command, "{context}: action[{i}].command");
            assert_eq!(aa.world, bb.world, "{context}: action[{i}].world");
            assert_eq!(aa.match_type, bb.match_type, "{context}: action[{i}].match_type");
            assert_eq!(aa.enabled, bb.enabled, "{context}: action[{i}].enabled");
            assert_eq!(aa.startup, bb.startup, "{context}: action[{i}].startup");
        }
        assert_eq!(a.tls_proxy_enabled, b.tls_proxy_enabled, "{context}: tls_proxy_enabled");
        assert_eq!(a.dictionary_path, b.dictionary_path, "{context}: dictionary_path");
        assert_eq!(a.editor_side.name(), b.editor_side.name(), "{context}: editor_side");
    }

    /// Assert all WorldSettings fields match between two instances.
    fn assert_world_settings_match(a: &WorldSettings, b: &WorldSettings, context: &str) {
        assert_eq!(a.world_type.name(), b.world_type.name(), "{context}: world_type");
        assert_eq!(a.hostname, b.hostname, "{context}: hostname");
        assert_eq!(a.port, b.port, "{context}: port");
        assert_eq!(a.user, b.user, "{context}: user");
        assert_eq!(a.password, b.password, "{context}: password");
        assert_eq!(a.use_ssl, b.use_ssl, "{context}: use_ssl");
        assert_eq!(a.log_enabled, b.log_enabled, "{context}: log_enabled");
        assert_eq!(a.encoding.name(), b.encoding.name(), "{context}: encoding");
        assert_eq!(a.auto_connect_type.name(), b.auto_connect_type.name(), "{context}: auto_connect_type");
        assert_eq!(a.keep_alive_type.name(), b.keep_alive_type.name(), "{context}: keep_alive_type");
        assert_eq!(a.keep_alive_cmd, b.keep_alive_cmd, "{context}: keep_alive_cmd");
        assert_eq!(a.slack_token, b.slack_token, "{context}: slack_token");
        assert_eq!(a.slack_channel, b.slack_channel, "{context}: slack_channel");
        assert_eq!(a.slack_workspace, b.slack_workspace, "{context}: slack_workspace");
        assert_eq!(a.discord_token, b.discord_token, "{context}: discord_token");
        assert_eq!(a.discord_guild, b.discord_guild, "{context}: discord_guild");
        assert_eq!(a.discord_channel, b.discord_channel, "{context}: discord_channel");
        assert_eq!(a.discord_dm_user, b.discord_dm_user, "{context}: discord_dm_user");
        assert_eq!(a.notes, b.notes, "{context}: notes");
        assert_eq!(a.gmcp_packages, b.gmcp_packages, "{context}: gmcp_packages");
    }

    #[test]
    fn test_settings_save_load_roundtrip() {
        let tmp = std::env::temp_dir().join("clay_test_settings_roundtrip.dat");
        // Ensure clean state
        let _ = std::fs::remove_file(&tmp);

        // Create app with non-default settings and a world
        let mut app = App::new();
        app.settings = make_non_default_settings();
        app.input_height = 7; // non-default (default: 3)

        // Add a world with non-default settings
        let mut world = World::new("testworld");
        world.settings = make_non_default_world_settings();
        app.worlds.push(world);

        // Save
        save_settings_to_path(&app, &tmp).expect("save_settings_to_path failed");

        // Load into fresh app
        let mut loaded_app = App::new();
        loaded_app.worlds.clear(); // load will create worlds
        load_settings_from_path(&mut loaded_app, &tmp).expect("load_settings_from_path failed");

        // Verify all global settings survived roundtrip
        assert_settings_match(&app.settings, &loaded_app.settings, "save/load roundtrip");

        // Verify input_height survived
        assert_eq!(loaded_app.input_height, 7, "input_height");

        // Verify world settings survived roundtrip
        assert_eq!(loaded_app.worlds.len(), 1, "world count");
        assert_eq!(loaded_app.worlds[0].name, "testworld", "world name");
        assert_world_settings_match(
            &app.worlds[0].settings,
            &loaded_app.worlds[0].settings,
            "world settings roundtrip",
        );

        // Cleanup
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_settings_non_default_detection() {
        // Verify that make_non_default_settings actually differs from defaults
        // for every field that is persisted. This catches the case where
        // make_non_default_settings uses a value that happens to equal the default.
        let non_default = make_non_default_settings();
        let default = Settings::default();

        assert_ne!(non_default.more_mode_enabled, default.more_mode_enabled, "more_mode_enabled should differ");
        assert_ne!(non_default.spell_check_enabled, default.spell_check_enabled, "spell_check_enabled should differ");
        assert_ne!(non_default.temp_convert_enabled, default.temp_convert_enabled, "temp_convert_enabled should differ");
        assert_ne!(non_default.world_switch_mode.name(), default.world_switch_mode.name(), "world_switch_mode should differ");
        assert_ne!(non_default.debug_enabled, default.debug_enabled, "debug_enabled should differ");
        assert_ne!(non_default.ansi_music_enabled, default.ansi_music_enabled, "ansi_music_enabled should differ");
        assert_ne!(non_default.theme.name(), default.theme.name(), "theme should differ");
        assert_ne!(non_default.gui_theme.name(), default.gui_theme.name(), "gui_theme should differ");
        assert_ne!(non_default.gui_transparency, default.gui_transparency, "gui_transparency should differ");
        assert_ne!(non_default.color_offset_percent, default.color_offset_percent, "color_offset_percent should differ");
        assert_ne!(non_default.font_name, default.font_name, "font_name should differ");
        assert_ne!(non_default.font_size, default.font_size, "font_size should differ");
        assert_ne!(non_default.web_font_size_phone, default.web_font_size_phone, "web_font_size_phone should differ");
        assert_ne!(non_default.web_font_size_tablet, default.web_font_size_tablet, "web_font_size_tablet should differ");
        assert_ne!(non_default.web_font_size_desktop, default.web_font_size_desktop, "web_font_size_desktop should differ");
        assert_ne!(non_default.web_secure, default.web_secure, "web_secure should differ");
        assert_ne!(non_default.http_enabled, default.http_enabled, "http_enabled should differ");
        assert_ne!(non_default.http_port, default.http_port, "http_port should differ");
        assert_ne!(non_default.ws_enabled, default.ws_enabled, "ws_enabled should differ");
        assert_ne!(non_default.ws_port, default.ws_port, "ws_port should differ");
        assert_ne!(non_default.websocket_password, default.websocket_password, "websocket_password should differ");
        assert_ne!(non_default.websocket_allow_list, default.websocket_allow_list, "websocket_allow_list should differ");
        assert_ne!(non_default.websocket_cert_file, default.websocket_cert_file, "websocket_cert_file should differ");
        assert_ne!(non_default.websocket_key_file, default.websocket_key_file, "websocket_key_file should differ");
        assert!(!non_default.websocket_auth_keys.is_empty(), "websocket_auth_keys should be non-empty");
        assert!(!non_default.actions.is_empty(), "actions should be non-empty");
        assert_ne!(non_default.tls_proxy_enabled, default.tls_proxy_enabled, "tls_proxy_enabled should differ");
        assert_ne!(non_default.dictionary_path, default.dictionary_path, "dictionary_path should differ");
        assert_ne!(non_default.editor_side.name(), default.editor_side.name(), "editor_side should differ");
    }

    #[test]
    fn test_world_settings_non_default_detection() {
        let non_default = make_non_default_world_settings();
        let default = WorldSettings::default();

        assert_ne!(non_default.hostname, default.hostname, "hostname should differ");
        assert_ne!(non_default.port, default.port, "port should differ");
        assert_ne!(non_default.user, default.user, "user should differ");
        assert_ne!(non_default.password, default.password, "password should differ");
        assert_ne!(non_default.use_ssl, default.use_ssl, "use_ssl should differ");
        assert_ne!(non_default.log_enabled, default.log_enabled, "log_enabled should differ");
        assert_ne!(non_default.encoding.name(), default.encoding.name(), "encoding should differ");
        assert_ne!(non_default.auto_connect_type.name(), default.auto_connect_type.name(), "auto_connect_type should differ");
        assert_ne!(non_default.keep_alive_type.name(), default.keep_alive_type.name(), "keep_alive_type should differ");
        assert_ne!(non_default.keep_alive_cmd, default.keep_alive_cmd, "keep_alive_cmd should differ");
        assert_ne!(non_default.slack_token, default.slack_token, "slack_token should differ");
        assert_ne!(non_default.slack_channel, default.slack_channel, "slack_channel should differ");
        assert_ne!(non_default.slack_workspace, default.slack_workspace, "slack_workspace should differ");
        assert_ne!(non_default.discord_token, default.discord_token, "discord_token should differ");
        assert_ne!(non_default.discord_guild, default.discord_guild, "discord_guild should differ");
        assert_ne!(non_default.discord_channel, default.discord_channel, "discord_channel should differ");
        assert_ne!(non_default.discord_dm_user, default.discord_dm_user, "discord_dm_user should differ");
        assert_ne!(non_default.notes, default.notes, "notes should differ");
        assert_ne!(non_default.gmcp_packages, default.gmcp_packages, "gmcp_packages should differ");
    }
}
