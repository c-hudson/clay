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

/// Legacy encryption key — only used for decrypting old .clay.dat files. Decrypt-only:
/// `encrypt_password` never writes with this key, so no new data is ever protected by
/// it (C5, security remediation) — it exists purely so pre-migration blobs still
/// decrypt once, at which point they're re-encrypted under the per-machine
/// `machine_key()` on the next save.
const LEGACY_ENCRYPTION_KEY: &[u8; 32] = b"nonsupersecretpassword#\0\0\0\0\0\0\0\0\0";

/// Per-machine encryption key, loaded from ~/.clay/secure.key (generated on first run)
static MACHINE_KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

fn machine_key() -> &'static [u8; 32] {
    MACHINE_KEY.get_or_init(|| {
        load_or_generate_key()
    })
}

fn key_file_path() -> PathBuf {
    crate::clay_config_path("secure.key")
}

fn load_or_generate_key() -> [u8; 32] {
    let path = key_file_path();

    // Try to read existing key
    if let Ok(data) = std::fs::read(&path) {
        if data.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&data);
            return key;
        }
    }

    // Generate new random key
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).expect("Failed to generate random key");

    // Write with restrictive (owner-only) permissions — B2 (security remediation) also
    // hardens the previously-unrestricted Windows branch via `restrict_to_owner_windows`
    // (currently best-effort/no-op there; see util.rs for the TODO).
    let _ = crate::util::write_secret_file(&path, &key);

    key
}

/// Encrypt a password using AES-256-GCM with per-machine key and random nonce
pub fn encrypt_password(password: &str) -> String {
    if password.is_empty() {
        return String::new();
    }

    let cipher = Aes256Gcm::new(machine_key().into());

    // Generate random 12-byte nonce
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).expect("Failed to generate random nonce");
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

/// Decrypt a password. Tries machine key first, falls back to legacy key for migration.
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

    // Try machine key first
    let cipher = Aes256Gcm::new(machine_key().into());
    if let Ok(plaintext) = cipher.decrypt(nonce, ciphertext) {
        return String::from_utf8_lossy(&plaintext).to_string();
    }

    // Fall back to legacy key (old .clay.dat files). This key is fixed/public (it ships
    // in the binary) and exists ONLY to let old blobs migrate — new data is always
    // written with `encrypt_password`, which only ever uses the per-machine
    // `machine_key()` (C5, security remediation). Log (always-on, no secret content)
    // whenever this fires so stale legacy-key blobs are noticed; they get re-encrypted
    // under the machine key the next time settings are saved.
    let legacy_cipher = Aes256Gcm::new(LEGACY_ENCRYPTION_KEY.into());
    if let Ok(plaintext) = legacy_cipher.decrypt(nonce, ciphertext) {
        debug_log(true, "decrypt_password: decrypted a stale blob using LEGACY_ENCRYPTION_KEY — will be re-encrypted under the machine key on next save");
        return String::from_utf8_lossy(&plaintext).to_string();
    }

    // Both failed — return as-is
    stored.to_string()
}

// ---------------------------------------------------------------------------
// Trust-on-first-use (TOFU) TLS certificate pin store — `~/.clay/known_hosts.dat`
//
// Maps `host:port` -> hex-encoded SHA-256 fingerprint of the end-entity
// certificate DER last seen (and trusted) for that host. Used by
// `platform::danger::TofuVerifier` (rustls) and the native-tls MUD path to
// detect when a server's certificate changes between connects.
// ---------------------------------------------------------------------------

fn known_hosts_path() -> PathBuf {
    crate::clay_config_path("known_hosts.dat")
}

static KNOWN_HOSTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn known_hosts() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    KNOWN_HOSTS.get_or_init(|| std::sync::Mutex::new(load_known_hosts_from(&known_hosts_path())))
}

fn load_known_hosts_from(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((host, fp)) = line.split_once('=') {
                map.insert(host.trim().to_string(), fp.trim().to_lowercase());
            }
        }
    }
    map
}

fn save_known_hosts_to(path: &std::path::Path, map: &std::collections::HashMap<String, String>) {
    let mut contents = String::from(
        "# Clay TLS certificate pins (trust-on-first-use)\n\
         # host:port=sha256(end-entity cert DER) hex — do not edit by hand\n",
    );
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        contents.push_str(k);
        contents.push('=');
        contents.push_str(&map[k]);
        contents.push('\n');
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
        {
            let _ = f.write_all(contents.as_bytes());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(path, contents.as_bytes());
    }
}

/// Look up the pinned fingerprint for a `host:port`, if one has been recorded.
pub fn get_pin(host_port: &str) -> Option<String> {
    known_hosts().lock().ok()?.get(host_port).cloned()
}

/// Pin a fingerprint for `host_port` only if no pin exists yet (first-connect,
/// silent). Returns `true` if a new pin was written.
pub fn add_pin(host_port: &str, fingerprint: &str) -> bool {
    let Ok(mut map) = known_hosts().lock() else { return false };
    if map.contains_key(host_port) {
        return false;
    }
    map.insert(host_port.to_string(), fingerprint.to_lowercase());
    save_known_hosts_to(&known_hosts_path(), &map);
    true
}

/// Overwrite the pin for `host_port` (user explicitly chose to trust a new
/// certificate after a mismatch warning).
pub fn replace_pin(host_port: &str, fingerprint: &str) {
    let Ok(mut map) = known_hosts().lock() else { return };
    map.insert(host_port.to_string(), fingerprint.to_lowercase());
    save_known_hosts_to(&known_hosts_path(), &map);
}

/// Remove a pin (e.g. so the next connect re-pins silently). Mostly useful for tests.
#[cfg(test)]
pub fn remove_pin(host_port: &str) {
    let Ok(mut map) = known_hosts().lock() else { return };
    map.remove(host_port);
    save_known_hosts_to(&known_hosts_path(), &map);
}

pub fn save_settings(app: &App) -> io::Result<()> {
    save_settings_with_source(app, "local")
}

/// Save settings, tagging the debug-mode audit log entry (if enabled) with `source` —
/// the origin of this save, e.g. "web", "gui", "console", "android" for a remote
/// client's UpdateGlobalSettings push (see the handlers in main.rs/daemon.rs), or
/// "local" for TUI/lifecycle saves. Only master client saves.
pub fn save_settings_with_source(app: &App, source: &str) -> io::Result<()> {
    // Only master client should save settings
    if !app.is_master {
        return Ok(());
    }
    save_settings_to_path_with_source(app, &get_settings_path(), source)
}

/// Save settings to a specific path (used by tests)
pub fn save_settings_to_path(app: &App, path: &std::path::Path) -> io::Result<()> {
    save_settings_to_path_with_source(app, path, "local")
}

/// Save settings to a specific path, tagging the debug-mode audit log (if enabled) with
/// `source` — see `save_settings_with_source`.
pub fn save_settings_to_path_with_source(app: &App, path: &std::path::Path, source: &str) -> io::Result<()> {
    // Snapshot the previous [global] section (debug mode only) so we can log exactly
    // which keys changed after the full-file rewrite below. Settings loss has bitten
    // silently before (a remote client pushing a stale/default snapshot clobbers globals
    // wholesale) — this audit trail lets a future incident be diagnosed after the fact.
    let debug_on = is_debug_enabled();
    let old_global = if debug_on { read_global_section(path) } else { Default::default() };

    // B2 (security remediation): settings.dat holds encrypted-at-rest passwords/tokens —
    // create it owner-only (0600 on Unix) instead of default (often world-readable) perms.
    let mut file = crate::util::secure_create_file(path)?;
    write_settings_dat(app, &mut file, false)?;
    drop(file);

    // Debug-gated audit trail: log which [global] keys changed, old -> new, along with
    // the source of the write and a backtrace of the code path that triggered it. Only
    // active when debug mode is on (see CLAUDE.md debug-logging conventions), so this
    // costs nothing normally but is available to diagnose a future settings-loss report.
    if debug_on {
        let new_global = read_global_section(path);
        append_settings_audit_log(source, &old_global, &new_global);
    }

    Ok(())
}

/// Serializes the current settings to `settings.dat` text (same format as the file on
/// disk) with every secret — world passwords, Slack/Discord tokens, the WS password and
/// auth key — written **in plaintext** rather than encrypted-at-rest. Used only by the
/// `/import` export path (`RequestSettingsExport` handler): the sender decrypts, the
/// wire carries plaintext to an authenticated peer (same trust level `InitialState`
/// already grants — see plan `i-d-like-to-make-snuggly-rain.md`), and the importer
/// re-encrypts under its own local `machine_key()` before ever touching disk.
pub fn serialize_settings_for_export(app: &App) -> String {
    let mut buf: Vec<u8> = Vec::new();
    write_settings_dat(app, &mut buf, true).expect("writing to an in-memory Vec<u8> cannot fail");
    String::from_utf8(buf).expect("settings.dat content is always valid UTF-8")
}

/// Writes the settings.dat body (global settings, worlds, actions, TF globals) to `w`.
/// When `plaintext_secrets` is true, passwords/tokens are written in cleartext instead of
/// their encrypted-at-rest form — see `serialize_settings_for_export`. Shared so the two
/// callers can never drift apart on which fields are considered secret.
fn write_settings_dat(app: &App, w: &mut impl IoWrite, plaintext_secrets: bool) -> io::Result<()> {
    let secret = |s: &str| -> String {
        if plaintext_secrets { s.to_string() } else { encrypt_password(s) }
    };
    let file = w;

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
    writeln!(file, "web_font_weight={}", app.settings.web_font_weight)?;
    writeln!(file, "web_font_line_height={}", app.settings.web_font_line_height)?;
    writeln!(file, "web_font_letter_spacing={}", app.settings.web_font_letter_spacing)?;
    writeln!(file, "web_font_word_spacing={}", app.settings.web_font_word_spacing)?;
    writeln!(file, "web_secure={}", app.settings.web_secure)?;
    writeln!(file, "http_enabled={}", app.settings.http_enabled)?;
    writeln!(file, "http_port={}", app.settings.http_port)?;
    // Written unconditionally (even when empty): key-absent (old settings file) means
    // default "clay"; present-but-empty means legacy mode (UI served at "/").
    writeln!(file, "web_path={}", app.settings.web_path)?;
    if !app.settings.websocket_password.is_empty() {
        writeln!(file, "websocket_password={}", secret(&app.settings.websocket_password))?;
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
    // Save single device auth key (encrypted, with timestamp)
    if let Some(ref ak) = app.settings.websocket_auth_key {
        writeln!(file, "websocket_auth_key={}|{}", secret(&ak.key), ak.created_at)?;
    }
    writeln!(file, "tls_proxy_enabled={}", app.settings.tls_proxy_enabled)?;
    if !app.settings.dictionary_path.is_empty() {
        writeln!(file, "dictionary_path={}", app.settings.dictionary_path)?;
    }
    writeln!(file, "editor_side={}", app.settings.editor_side.name())?;
    writeln!(file, "mouse_enabled={}", app.settings.mouse_enabled)?;
    writeln!(file, "zwj_enabled={}", app.settings.zwj_enabled)?;
    writeln!(file, "new_line_indicator={}", app.settings.new_line_indicator)?;
    writeln!(file, "tts_mode={}", app.settings.tts_mode.name())?;
    writeln!(file, "tts_speak_mode={}", app.settings.tts_speak_mode.name())?;
    writeln!(file, "scrollback_enabled={}", app.settings.scrollback_enabled)?;
    writeln!(file, "url_shortener={}", app.settings.url_shortener_service.name())?;

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
        writeln!(file, "password={}", secret(&world.settings.password))?;
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
        let ar = world.settings.auto_reconnect_display();
        if ar != "0" {
            writeln!(file, "auto_reconnect_secs={}", ar)?;
        }
        if world.settings.log_enabled {
            writeln!(file, "log_enabled=true")?;
        }
        // Slack settings
        if !world.settings.slack_token.is_empty() {
            writeln!(file, "slack_token={}", secret(&world.settings.slack_token))?;
        }
        if !world.settings.slack_channel.is_empty() {
            writeln!(file, "slack_channel={}", world.settings.slack_channel)?;
        }
        if !world.settings.slack_workspace.is_empty() {
            writeln!(file, "slack_workspace={}", world.settings.slack_workspace)?;
        }
        // Discord settings
        if !world.settings.discord_token.is_empty() {
            writeln!(file, "discord_token={}", secret(&world.settings.discord_token))?;
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
        // Save action-level match type (only when not the default Regexp)
        if action.match_type != MatchType::Regexp {
            writeln!(file, "match_type={}", action.match_type.as_str().to_lowercase())?;
        }
        // Save patterns as pattern.N.text (no per-pattern type; type is action-level)
        for (i, mp) in action.patterns.iter().enumerate() {
            if mp.pattern.is_empty() { continue; }
            writeln!(file, "pattern.{}.text={}", i, mp.pattern.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
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

    // Note: bans are in-memory only and not persisted

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

/// Parse just the `[global]` section of a settings.dat-style file into a key->value map
/// of raw strings, for the debug-mode audit-log diff in `save_settings_to_path_with_source`.
/// Returns an empty map if the file doesn't exist or has no `[global]` section.
fn read_global_section(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return map,
    };
    let reader = io::BufReader::new(file);
    let mut in_global = false;
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_global = trimmed == "[global]";
            continue;
        }
        if !in_global || trimmed.is_empty() {
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            map.insert(trimmed[..eq].to_string(), trimmed[eq + 1..].to_string());
        }
    }
    map
}

/// Append a diff of changed `[global]` keys to `~/.clay/settings-audit.log`, tagged with
/// the save's source (web/gui/console/android/local) and a captured backtrace so a future
/// settings-loss report can be traced to the exact code path that wrote the bad values.
/// Only called when debug mode is on (see caller). Encrypted values (e.g.
/// websocket_password) are stored/logged in their encrypted-at-rest form, same as on disk.
fn append_settings_audit_log(
    source: &str,
    old: &std::collections::HashMap<String, String>,
    new: &std::collections::HashMap<String, String>,
) {
    append_settings_audit_log_to_path(&clay_config_path("settings-audit.log"), source, old, new)
}

/// Same as `append_settings_audit_log`, but to an explicit path (used for tests so they
/// don't write into the user's real `~/.clay/` directory).
fn append_settings_audit_log_to_path(
    log_path: &std::path::Path,
    source: &str,
    old: &std::collections::HashMap<String, String>,
    new: &std::collections::HashMap<String, String>,
) {
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    let changes: Vec<String> = keys.into_iter()
        .filter_map(|key| {
            let old_val = old.get(key).map(|s| s.as_str()).unwrap_or("<unset>");
            let new_val = new.get(key).map(|s| s.as_str()).unwrap_or("<unset>");
            if old_val == new_val {
                None
            } else {
                Some(format!("    {}: {} -> {}", key, old_val, new_val))
            }
        })
        .collect();
    if changes.is_empty() {
        return;
    }

    let lt = local_time_now();
    let timestamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        lt.year, lt.month, lt.day, lt.hour, lt.minute, lt.second
    );
    let backtrace = std::backtrace::Backtrace::force_capture();

    // B2 (security remediation): the audit log records old/new setting values (including
    // encrypted-at-rest secrets in their encrypted form) plus a backtrace — owner-only perms.
    if let Ok(mut file) = crate::util::secure_append_file(log_path) {
        let _ = writeln!(file, "[{}] source={} changed global settings:", timestamp, source);
        for change in &changes {
            let _ = writeln!(file, "{}", change);
        }
        let _ = writeln!(file, "  backtrace:\n{}", backtrace);
        let _ = writeln!(file);
    }
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
    let content = std::fs::read_to_string(path)?;
    load_settings_from_str(app, &content);
    Ok(())
}

/// Parses settings.dat-format text and merges it into `app` **in place**: a `[global]` key
/// present in `content` overwrites `app.settings`'s matching field; a `[world:name]`/
/// `[action:name]` section is matched by name (found via `find_or_create_world`/by-name
/// lookup) and overwrites that entry's fields, creating a new one if the name doesn't
/// already exist. Anything `content` doesn't mention is left exactly as `app` already had
/// it. This "merge into existing state" behavior — rather than resetting `app` first — is
/// what `load_settings_from_path` has always done (needed for `/reload`), and turns out to
/// be exactly the remote-wins-on-conflict / keep-local-only-entries semantics `/import`'s
/// `merge_settings_dat` needs (plan `i-d-like-to-make-snuggly-rain.md`): `decrypt_password`
/// treats an already-plaintext value (no `ENC:` prefix, which is what
/// `serialize_settings_for_export` emits) as a no-op passthrough, so merging a remote
/// export's plaintext secrets into `app`'s always-in-memory-plaintext settings needs no new
/// crypto here — the next `save_settings` re-encrypts everything under the local machine key.
/// One asymmetry worth knowing: a key `content` omits entirely (e.g. `write_settings_dat`
/// skips `websocket_password=` when it's empty) leaves `app`'s existing value alone rather
/// than clearing it — an absent remote value can't "win" a conflict it never entered.
pub fn load_settings_from_str(app: &mut App, content: &str) {
    let mut current_world: Option<String> = None;
    let mut current_action: Option<usize> = None;
    let mut in_banned_hosts = false;
    let mut in_tf_globals = false;

    for line in content.lines() {
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

            // Bans are in-memory only — skip any [banned_hosts] entries in old files
            if in_banned_hosts {
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
                        _ if key.starts_with("pattern.") => {
                            let parts: Vec<&str> = key.splitn(3, '.').collect();
                            if parts.len() == 3 {
                                if let Ok(idx) = parts[1].parse::<usize>() {
                                    while action.patterns.len() <= idx {
                                        action.patterns.push(MatchPattern::default());
                                    }
                                    match parts[2] {
                                        // Back-compat: old files may have per-pattern type; fold into
                                        // action-level match_type (any wildcard makes the whole action wildcard)
                                        "type" => {
                                            let mt = MatchType::parse(value);
                                            if mt == MatchType::Wildcard {
                                                action.match_type = MatchType::Wildcard;
                                            }
                                        }
                                        "text" => action.patterns[idx].pattern = unescape_action_value(value),
                                        _ => {}
                                    }
                                }
                            }
                        }
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
                    "web_font_weight" => {
                        if let Ok(w) = value.parse::<u16>() {
                            app.settings.web_font_weight = w.clamp(1, 900);
                        }
                    }
                    "web_font_line_height" => {
                        if let Ok(v) = value.parse::<f32>() {
                            app.settings.web_font_line_height = v.clamp(0.5, 3.0);
                        }
                    }
                    "web_font_letter_spacing" => {
                        if let Ok(v) = value.parse::<f32>() {
                            app.settings.web_font_letter_spacing = v.clamp(-5.0, 10.0);
                        }
                    }
                    "web_font_word_spacing" => {
                        if let Ok(v) = value.parse::<f32>() {
                            app.settings.web_font_word_spacing = v.clamp(-5.0, 20.0);
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
                    // Legacy: ws_enabled/ws_port/websocket_enabled/websocket_port — silently ignored
                    "ws_enabled" | "websocket_enabled" => {}
                    "ws_port" | "websocket_port" => {}
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
                        // Load single device auth key (format: ENC:...|timestamp or legacy ENC:...)
                        // If multiple lines found, ignore all (migration: startup will generate fresh)
                        if app.settings.websocket_auth_key.is_some() {
                            // Multiple keys found — clear and let startup generate a fresh one
                            app.settings.websocket_auth_key = None;
                        } else {
                            let (enc_part, timestamp) = if let Some(pipe_pos) = value.rfind('|') {
                                let ts_str = &value[pipe_pos + 1..];
                                if let Ok(ts) = ts_str.parse::<u64>() {
                                    (&value[..pipe_pos], ts)
                                } else {
                                    (value, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                                }
                            } else {
                                (value, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                            };
                            let key = decrypt_password(enc_part);
                            if !key.is_empty() {
                                app.settings.websocket_auth_key = Some(crate::AuthKey { key, created_at: timestamp });
                            }
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
                    "web_path" => {
                        // Key present (even empty) overrides the default "clay" — empty
                        // means legacy mode (UI served at "/").
                        app.settings.web_path = sanitize_web_path(value);
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
                    // Legacy: ws_nonsecure_enabled/ws_nonsecure_port — silently ignored
                    "ws_nonsecure_enabled" | "ws_nonsecure_port" => {}
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
                    "mouse_enabled" => {
                        app.settings.mouse_enabled = value == "true";
                    }
                    "zwj_enabled" => {
                        app.settings.zwj_enabled = value == "true";
                    }
                    "new_line_indicator" => {
                        app.settings.new_line_indicator = value == "true";
                    }
                    "tts_mode" => {
                        app.settings.tts_mode = crate::tts::TtsMode::from_name(value);
                    }
                    "tts_speak_mode" => {
                        app.settings.tts_speak_mode = crate::tts::TtsSpeakMode::from_name(value);
                    }
                    "tts_enabled" => {
                        // Legacy: convert bool to TtsMode
                        if value == "true" {
                            app.settings.tts_mode = crate::tts::TtsMode::Local;
                        }
                    }
                    "scrollback_enabled" => {
                        app.settings.scrollback_enabled = value == "true";
                    }
                    "url_shortener" => {
                        app.settings.url_shortener_service = crate::encoding::UrlShortener::from_name(value);
                    }
                    "arrow_up_down_mode" | "shift_arrow_up_down_mode" => {
                        // Legacy: silently ignore (now handled by keybindings system)
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
                        "auto_reconnect_secs" => {
                            let (secs, on_web) = crate::WorldSettings::parse_auto_reconnect(value);
                            world.settings.auto_reconnect_secs = secs;
                            world.settings.auto_reconnect_on_web = on_web;
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

    *app.ws_auth_key_shared.write().unwrap() = app.settings.websocket_auth_key.as_ref().map(|ak| ak.key.clone());
}

/// Merges a remote Clay instance's exported settings.dat text into `app` — remote-wins on
/// every key it mentions, local-only worlds/actions/global keys are left untouched. Thin
/// wrapper over `load_settings_from_str`; see its doc comment for exactly how the merge
/// and re-encryption work. Part of the `/import` merge step (plan
/// `i-d-like-to-make-snuggly-rain.md`) — the caller must still call `save_settings` (or
/// `save_settings_to_path`) afterward to persist the merged, re-encrypted result.
pub fn merge_settings_dat(app: &mut App, remote_settings_dat: &str) {
    load_settings_from_str(app, remote_settings_dat);
}

/// Merges a remote instance's exported theme.dat text into `app.theme_file`. Thin wrapper
/// over `ThemeFile::merge_remote` — see its doc comment. Part of the `/import` merge step
/// (plan `i-d-like-to-make-snuggly-rain.md`); no secrets involved, so no re-encryption step.
pub fn merge_theme_dat(app: &mut App, remote_theme_dat: &str) {
    app.theme_file.merge_remote(remote_theme_dat);
}

/// Merges a remote instance's exported keybindings.dat text into `app.keybindings`. Thin
/// wrapper over `KeyBindings::merge_remote` — see its doc comment. Part of the `/import`
/// merge step (plan `i-d-like-to-make-snuggly-rain.md`); no secrets involved, so no
/// re-encryption step.
pub fn merge_keybindings_dat(app: &mut App, remote_keybindings_dat: &str) {
    app.keybindings.merge_remote(remote_keybindings_dat);
}

/// Load settings for multiuser mode from ~/.clay/multiuser.dat
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

            // Bans are in-memory only — skip any [banned_hosts] entries in old files
            if in_banned_hosts {
                continue;
            }

            // User settings
            if let Some(ref user_name) = current_user {
                if let Some(user) = app.users.iter_mut().find(|u| &u.name == user_name) {
                    match key {
                        "password" => user.password = decrypt_password(value),
                        // C1 (security remediation): set by ChangePassword when it
                        // stores an already-hashed value in `password` instead of a
                        // plaintext one — see the `User::password_is_hash` doc comment.
                        "password_is_hash" => user.password_is_hash = value == "true",
                        _ => {}
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
                        // Action-level match type (also handles legacy single-pattern files)
                        "match_type" => action.match_type = MatchType::parse(value),
                        "pattern" => action.pattern = unescape_action_value(value),
                        "command" => action.command = unescape_action_value(value),
                        "enabled" => action.enabled = value != "false",
                        "startup" => action.startup = value == "true",
                        _ if key.starts_with("pattern.") => {
                            // Multi-pattern keys: "pattern.N.text" (and legacy "pattern.N.type")
                            let parts: Vec<&str> = key.splitn(3, '.').collect();
                            if parts.len() == 3 {
                                if let Ok(idx) = parts[1].parse::<usize>() {
                                    while action.patterns.len() <= idx {
                                        action.patterns.push(MatchPattern::default());
                                    }
                                    match parts[2] {
                                        // Back-compat: fold any per-pattern wildcard into action-level type
                                        "type" => {
                                            let mt = MatchType::parse(value);
                                            if mt == MatchType::Wildcard {
                                                action.match_type = MatchType::Wildcard;
                                            }
                                        }
                                        "text" => action.patterns[idx].pattern = unescape_action_value(value),
                                        _ => {}
                                    }
                                }
                            }
                        }
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
                        "auto_reconnect_secs" => {
                            let (secs, on_web) = crate::WorldSettings::parse_auto_reconnect(value);
                            world.settings.auto_reconnect_secs = secs;
                            world.settings.auto_reconnect_on_web = on_web;
                        }
                        _ => {}
                    }
                }
            }
            // Global settings
            else {
                match key {
                    // Legacy: ws_enabled/ws_port — silently ignored
                    "ws_enabled" | "ws_port" => {}
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
                    "web_path" => app.settings.web_path = sanitize_web_path(value),
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Save settings for multiuser mode to ~/.clay/multiuser.dat
pub fn save_multiuser_settings(app: &App) -> io::Result<()> {
    let path = get_multiuser_settings_path();
    // B2 (security remediation): holds encrypted user/world passwords — owner-only perms.
    let mut file = crate::util::secure_create_file(&path)?;

    // [global] section
    writeln!(file, "[global]")?;
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
    // Written unconditionally (even when empty): key-absent (old settings file) means
    // default "clay"; present-but-empty means legacy mode (UI served at "/").
    writeln!(file, "web_path={}", app.settings.web_path)?;

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
        // Only written when true (keeps old-format files/diffs minimal); absent means
        // false, matching User::new's default. See User::password_is_hash doc comment.
        if user.password_is_hash {
            writeln!(file, "password_is_hash=true")?;
        }
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
            let ar = world.settings.auto_reconnect_display();
            if ar != "0" {
                writeln!(file, "auto_reconnect_secs={}", ar)?;
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
            // Save action-level match type (only when not the default Regexp)
            if action.match_type != MatchType::Regexp {
                writeln!(file, "match_type={}", action.match_type.as_str().to_lowercase())?;
            }
            // Save patterns as pattern.N.text (no per-pattern type; type is action-level)
            for (i, mp) in action.patterns.iter().enumerate() {
                if mp.pattern.is_empty() { continue; }
                writeln!(file, "pattern.{}.text={}", i, mp.pattern.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
            }
            let escaped_command = action.command
                .replace('\\', "\\\\")
                .replace('=', "\\e")
                .replace('\n', "\\n");
            writeln!(file, "command={}", escaped_command)?;
            if !action.enabled {
                writeln!(file, "enabled=false")?;
            }
            if action.startup {
                writeln!(file, "startup=true")?;
            }
        }
    }

    // Note: bans are in-memory only and not persisted

    Ok(())
}

#[cfg(not(target_os = "android"))]
pub fn save_reload_state(app: &App) -> io::Result<()> {
    let path = get_reload_state_path();
    // B2 (security remediation): reload state holds world passwords/tokens and the
    // encrypted auth key — create it owner-only (0600 on Unix), like settings.dat.
    let mut file = crate::util::secure_create_file(&path)?;

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
    writeln!(file, "web_font_weight={}", app.settings.web_font_weight)?;
    writeln!(file, "web_font_line_height={}", app.settings.web_font_line_height)?;
    writeln!(file, "web_font_letter_spacing={}", app.settings.web_font_letter_spacing)?;
    writeln!(file, "web_font_word_spacing={}", app.settings.web_font_word_spacing)?;
    writeln!(file, "web_secure={}", app.settings.web_secure)?;
    writeln!(file, "http_enabled={}", app.settings.http_enabled)?;
    writeln!(file, "http_port={}", app.settings.http_port)?;
    // Written unconditionally (even when empty): key-absent (old settings file) means
    // default "clay"; present-but-empty means legacy mode (UI served at "/").
    writeln!(file, "web_path={}", app.settings.web_path)?;
    if !app.settings.websocket_password.is_empty() {
        writeln!(file, "websocket_password={}", encrypt_password(&app.settings.websocket_password))?;
    }
    if let Some(ref ak) = app.settings.websocket_auth_key {
        writeln!(file, "websocket_auth_key={}|{}", encrypt_password(&ak.key), ak.created_at)?;
    }
    if !app.settings.websocket_allow_list.is_empty() {
        writeln!(file, "websocket_allow_list={}", app.settings.websocket_allow_list)?;
    }
    // whitelisted_host is runtime-only state, not persisted across reloads
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
    writeln!(file, "mouse_enabled={}", app.settings.mouse_enabled)?;
    writeln!(file, "zwj_enabled={}", app.settings.zwj_enabled)?;
    writeln!(file, "new_line_indicator={}", app.settings.new_line_indicator)?;
    writeln!(file, "tts_mode={}", app.settings.tts_mode.name())?;
    writeln!(file, "tts_speak_mode={}", app.settings.tts_speak_mode.name())?;
    writeln!(file, "scrollback_enabled={}", app.settings.scrollback_enabled)?;
    writeln!(file, "url_shortener={}", app.settings.url_shortener_service.name())?;

    // Save watchdog state
    writeln!(file, "watchdog_enabled={}", app.tf_engine.watchdog_enabled)?;
    writeln!(file, "watchdog_n1={}", app.tf_engine.watchdog_n1)?;
    writeln!(file, "watchdog_n2={}", app.tf_engine.watchdog_n2)?;
    let mut wd_overrides: Vec<(&String, &tf::WatchdogConfig)> = app.tf_engine.watchdog_overrides.iter().collect();
    wd_overrides.sort_by_key(|(k, _)| *k);
    writeln!(file, "watchdog_override_count={}", wd_overrides.len())?;
    for (i, (world, cfg)) in wd_overrides.iter().enumerate() {
        let escaped_world = world.replace('\\', "\\\\").replace('=', "\\e");
        let status = if cfg.enabled { "on" } else { "off" };
        writeln!(file, "watchdog_override_{}={}|{}|{}|{}", i, escaped_world, status, cfg.n1, cfg.n2)?;
    }

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
        writeln!(file, "visual_line_offset={}", world.visual_line_offset)?;
        writeln!(file, "is_tls={}", world.is_tls)?;
        writeln!(file, "was_connected={}", world.was_connected)?;
        writeln!(file, "showing_splash={}", world.showing_splash)?;
        writeln!(file, "telnet_mode={}", world.telnet_mode)?;
        if let Some(enc) = world.negotiated_encoding {
            writeln!(file, "negotiated_encoding={}", enc.iana_name())?;
        }
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
        let ar = world.settings.auto_reconnect_display();
        if ar != "0" {
            writeln!(file, "auto_reconnect_secs={}", ar)?;
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
    // Flags: s = from_server, g = gagged, n = marked_new (omit if false)
    for (idx, world) in app.worlds.iter().enumerate() {
        writeln!(file)?;
        writeln!(file, "[output:{}]", idx)?;
        for line in &world.output_lines {
            let ts_secs = line.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut flags = String::new();
            if line.from_server { flags.push('s'); }
            if line.gagged { flags.push('g'); }
            if line.marked_new { flags.push('n'); }
            let escaped = line.text.replace('\\', "\\\\").replace('\n', "\\n");
            writeln!(file, "{}|{}|{}|{}", ts_secs, flags, line.seq, escaped)?;
        }
        writeln!(file, "[pending:{}]", idx)?;
        for line in &world.pending_lines {
            let ts_secs = line.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let mut flags = String::new();
            if line.from_server { flags.push('s'); }
            if line.gagged { flags.push('g'); }
            if line.marked_new { flags.push('n'); }
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
        // Save action-level match type (only when not the default Regexp)
        if action.match_type != MatchType::Regexp {
            writeln!(file, "match_type={}", action.match_type.as_str().to_lowercase())?;
        }
        // Save patterns as pattern.N.text (no per-pattern type; type is action-level)
        for (i, mp) in action.patterns.iter().enumerate() {
            if mp.pattern.is_empty() { continue; }
            writeln!(file, "pattern.{}.text={}", i, mp.pattern.replace('\\', "\\\\").replace('=', "\\e").replace('\n', "\\n"))?;
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
    debug_log(is_debug_enabled(), "LOAD_STATE: Starting load_reload_state");
    let path = get_reload_state_path();
    if !path.exists() {
        debug_log(is_debug_enabled(), "LOAD_STATE: No state file found");
        return Ok(false);
    }

    debug_log(is_debug_enabled(), &format!("LOAD_STATE: Reading state file: {:?}", path));
    let content = std::fs::read_to_string(&path)?;
    let lines: Vec<&str> = content.lines().collect();
    debug_log(is_debug_enabled(), &format!("LOAD_STATE: State file has {} lines", lines.len()));

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
        visual_line_offset: usize,
        is_tls: bool,
        was_connected: bool,
        showing_splash: bool,
        telnet_mode: bool,
        negotiated_encoding: Option<Encoding>,
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
    // Newest format: timestamp_secs|flags|seq|text (flags: s=from_server, g=gagged, n=marked_new)
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
                    let marked_new = flags.contains('n');
                    return OutputLine {
                        text,
                        timestamp,
                        from_server,
                        gagged,
                        seq,
                        highlight_color: None,
                        marked_new,
                        from_archive: false,
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
                        marked_new: false,
                        from_archive: false,
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
                        marked_new: false,
                        from_archive: false,
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
                        visual_line_offset: 0,
                        is_tls: false,
                        was_connected: false,
                        showing_splash: false,
                        telnet_mode: false,
                        negotiated_encoding: None,
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
                    "web_font_weight" => {
                        if let Ok(w) = value.parse::<u16>() {
                            app.settings.web_font_weight = w.clamp(1, 900);
                        }
                    }
                    "web_font_line_height" => {
                        if let Ok(v) = value.parse::<f32>() {
                            app.settings.web_font_line_height = v.clamp(0.5, 3.0);
                        }
                    }
                    "web_font_letter_spacing" => {
                        if let Ok(v) = value.parse::<f32>() {
                            app.settings.web_font_letter_spacing = v.clamp(-5.0, 10.0);
                        }
                    }
                    "web_font_word_spacing" => {
                        if let Ok(v) = value.parse::<f32>() {
                            app.settings.web_font_word_spacing = v.clamp(-5.0, 20.0);
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
                    // Legacy: ws_enabled/ws_port/websocket_enabled/websocket_port — silently ignored
                    "ws_enabled" | "websocket_enabled" => {}
                    "ws_port" | "websocket_port" => {}
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
                        // Legacy: ignored, whitelisted_host is now runtime-only state
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
                    "web_path" => {
                        app.settings.web_path = sanitize_web_path(value);
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
                    // Legacy: ws_nonsecure_enabled/ws_nonsecure_port — silently ignored
                    "ws_nonsecure_enabled" | "ws_nonsecure_port" => {}
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
                    "mouse_enabled" => {
                        app.settings.mouse_enabled = value == "true";
                    }
                    "zwj_enabled" => {
                        app.settings.zwj_enabled = value == "true";
                    }
                    "new_line_indicator" => {
                        app.settings.new_line_indicator = value == "true";
                    }
                    "tts_mode" => {
                        app.settings.tts_mode = crate::tts::TtsMode::from_name(value);
                    }
                    "tts_speak_mode" => {
                        app.settings.tts_speak_mode = crate::tts::TtsSpeakMode::from_name(value);
                    }
                    "tts_enabled" => {
                        // Legacy: convert bool to TtsMode
                        if value == "true" {
                            app.settings.tts_mode = crate::tts::TtsMode::Local;
                        }
                    }
                    "scrollback_enabled" => {
                        app.settings.scrollback_enabled = value == "true";
                    }
                    "url_shortener" => {
                        app.settings.url_shortener_service = crate::encoding::UrlShortener::from_name(value);
                    }
                    "arrow_up_down_mode" | "shift_arrow_up_down_mode" => {
                        // Legacy: silently ignore (now handled by keybindings system)
                    }
                    "history_count" | "world_count" | "watchdog_override_count" => {
                        // These are informational, not needed for parsing
                    }
                    "watchdog_enabled" => {
                        app.tf_engine.watchdog_enabled = value == "true";
                    }
                    "watchdog_n1" => {
                        if let Ok(n) = value.parse::<usize>() { app.tf_engine.watchdog_n1 = n; }
                    }
                    "watchdog_n2" => {
                        if let Ok(n) = value.parse::<usize>() { app.tf_engine.watchdog_n2 = n; }
                    }
                    k if k.starts_with("history_") => {
                        app.input.history.push(unescape_string(value));
                    }
                    k if k.starts_with("watchdog_override_") => {
                        // value: escaped_worldname|on|n1|n2
                        let parts: Vec<&str> = value.splitn(4, '|').collect();
                        if parts.len() == 4 {
                            let world = unescape_string(parts[0]);
                            let enabled = parts[1] == "on";
                            if let (Ok(n1), Ok(n2)) = (parts[2].parse::<usize>(), parts[3].parse::<usize>()) {
                                app.tf_engine.watchdog_overrides.insert(world, tf::WatchdogConfig { enabled, n1, n2 });
                            }
                        }
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
                            "visual_line_offset" => tw.visual_line_offset = value.parse().unwrap_or(0),
                            "is_tls" => tw.is_tls = value == "true",
                            "was_connected" => tw.was_connected = value == "true",
                            "showing_splash" => tw.showing_splash = value == "true",
                            "telnet_mode" => tw.telnet_mode = value == "true",
                            "negotiated_encoding" => tw.negotiated_encoding = Encoding::from_iana_name(value),
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
                            "auto_reconnect_secs" => {
                                let (secs, on_web) = crate::WorldSettings::parse_auto_reconnect(value);
                                tw.settings.auto_reconnect_secs = secs;
                                tw.settings.auto_reconnect_on_web = on_web;
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
                            // Action-level match type (also handles legacy single-pattern files)
                            "match_type" => action.match_type = MatchType::parse(value),
                            "pattern" => action.pattern = unescape_action_value(value),
                            "command" => action.command = unescape_action_value(value),
                            "enabled" => action.enabled = value != "false",
                            "startup" => action.startup = value == "true",
                            _ if key.starts_with("pattern.") => {
                                // Multi-pattern keys: "pattern.N.text" (and legacy "pattern.N.type")
                                let parts: Vec<&str> = key.splitn(3, '.').collect();
                                if parts.len() == 3 {
                                    if let Ok(idx) = parts[1].parse::<usize>() {
                                        while action.patterns.len() <= idx {
                                            action.patterns.push(MatchPattern::default());
                                        }
                                        match parts[2] {
                                            // Back-compat: fold any per-pattern wildcard into action-level type
                                            "type" => {
                                                let mt = MatchType::parse(value);
                                                if mt == MatchType::Wildcard {
                                                    action.match_type = MatchType::Wildcard;
                                                }
                                            }
                                            "text" => action.patterns[idx].pattern = unescape_action_value(value),
                                            _ => {}
                                        }
                                    }
                                }
                            }
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
        world.first_marked_new_index = world.output_lines.iter().position(|l| l.marked_new);
        world.scroll_offset = tw.scroll_offset;
        world.connected = tw.connected;
        world.unseen_lines = tw.unseen_lines;
        world.paused = tw.paused;
        world.pending_lines = tw.pending_lines;
        world.lines_since_pause = tw.lines_since_pause;
        world.visual_line_offset = tw.visual_line_offset;
        world.is_tls = tw.is_tls;
        world.was_connected = tw.was_connected;
        world.showing_splash = tw.showing_splash;
        world.telnet_mode = tw.telnet_mode;
        world.negotiated_encoding = tw.negotiated_encoding;
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

    // Load auth key from ~/.clay/settings.dat (it's not in the reload state file)
    let settings_path = get_settings_path();
    if settings_path.exists() {
        if let Ok(settings_content) = std::fs::read_to_string(&settings_path) {
            let mut found_count = 0u32;
            for line in settings_content.lines() {
                let trimmed = line.trim();
                if let Some(value) = trimmed.strip_prefix("websocket_auth_key=") {
                    found_count += 1;
                    if found_count > 1 {
                        // Multiple keys found — clear and let startup generate a fresh one
                        app.settings.websocket_auth_key = None;
                        break;
                    }
                    let (enc_part, timestamp) = if let Some(pipe_pos) = value.rfind('|') {
                        let ts_str = &value[pipe_pos + 1..];
                        if let Ok(ts) = ts_str.parse::<u64>() {
                            (&value[..pipe_pos], ts)
                        } else {
                            (value, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                        }
                    } else {
                        (value, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
                    };
                    let key = decrypt_password(enc_part);
                    if !key.is_empty() {
                        app.settings.websocket_auth_key = Some(crate::AuthKey { key, created_at: timestamp });
                    }
                }
            }
        }
    }

    *app.ws_auth_key_shared.write().unwrap() = app.settings.websocket_auth_key.as_ref().map(|ak| ak.key.clone());

    // Clean up the reload state file and env var
    let _ = std::fs::remove_file(&path);
    std::env::remove_var("CLAY_RELOAD_PID");

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
            web_font_weight: 200,              // default: 400
            web_secure: true,                  // default: false
            http_enabled: true,                // default: false
            http_port: 8080,                   // default: 9000
            web_path: "stealth".to_string(),   // default: "clay"
            websocket_password: "testpass".to_string(),     // default: ""
            websocket_allow_list: "192.168.1.1".to_string(), // default: ""
            websocket_whitelisted_host: Some("10.0.0.1".to_string()), // default: None (not persisted to .clay.dat)
            websocket_cert_file: "/tmp/cert.pem".to_string(), // default: ""
            websocket_key_file: "/tmp/key.pem".to_string(),   // default: ""
            websocket_auth_key: {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                Some(crate::AuthKey { key: "key1".to_string(), created_at: now })
            }, // default: None
            actions: vec![
                {
                    let mut a = Action::new();
                    a.name = "test_action".to_string();
                    a.match_type = crate::actions::MatchType::Wildcard;
                    a.patterns = vec![
                        MatchPattern {
                            pattern: "^test.*pattern$".to_string(),
                            compiled_regex: None,
                        },
                    ];
                    a.command = "/echo matched".to_string();
                    a.world = "testworld".to_string();
                    a.enabled = false;
                    a.startup = true;
                    a
                },
            ],
            tls_proxy_enabled: true,           // default: false
            dictionary_path: "/custom/dict".to_string(), // default: ""
            editor_side: EditorSide::Right,    // default: Left
            mouse_enabled: false,              // default: true
            zwj_enabled: true,                 // default: false
            new_line_indicator: true,             // default: false
            tts_mode: crate::tts::TtsMode::Edge,  // default: Off
            tts_speak_mode: crate::tts::TtsSpeakMode::All, // default: All
            tts_muted: true,                   // default: false
            web_font_letter_spacing: 1.5,      // default: 0.0
            web_font_line_height: 1.8,         // default: 1.2
            web_font_word_spacing: 2.0,        // default: 0.0
            scrollback_enabled: true,          // default: false
            url_shortener_service: crate::encoding::UrlShortener::TinyUrl, // default: IsGd
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
            auto_reconnect_secs: 30,                       // default: 0
            auto_reconnect_on_web: true,                   // default: false
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
        assert_eq!(a.web_font_weight, b.web_font_weight, "{context}: web_font_weight");
        assert_eq!(a.web_secure, b.web_secure, "{context}: web_secure");
        assert_eq!(a.http_enabled, b.http_enabled, "{context}: http_enabled");
        assert_eq!(a.http_port, b.http_port, "{context}: http_port");
        assert_eq!(a.web_path, b.web_path, "{context}: web_path");
        assert_eq!(a.websocket_password, b.websocket_password, "{context}: websocket_password");
        assert_eq!(a.websocket_allow_list, b.websocket_allow_list, "{context}: websocket_allow_list");
        // websocket_whitelisted_host is not persisted to .clay.dat (runtime state)
        assert_eq!(a.websocket_cert_file, b.websocket_cert_file, "{context}: websocket_cert_file");
        assert_eq!(a.websocket_key_file, b.websocket_key_file, "{context}: websocket_key_file");
        assert_eq!(a.websocket_auth_key.is_some(), b.websocket_auth_key.is_some(), "{context}: websocket_auth_key.is_some()");
        if let (Some(ak_a), Some(ak_b)) = (&a.websocket_auth_key, &b.websocket_auth_key) {
            assert_eq!(ak_a.key, ak_b.key, "{context}: websocket_auth_key.key");
        }
        assert_eq!(a.actions.len(), b.actions.len(), "{context}: actions.len()");
        for (i, (aa, bb)) in a.actions.iter().zip(b.actions.iter()).enumerate() {
            assert_eq!(aa.name, bb.name, "{context}: action[{i}].name");
            assert_eq!(aa.match_type, bb.match_type, "{context}: action[{i}].match_type");
            assert_eq!(aa.patterns.len(), bb.patterns.len(), "{context}: action[{i}].patterns.len()");
            for (j, (pa, pb)) in aa.patterns.iter().zip(bb.patterns.iter()).enumerate() {
                assert_eq!(pa.pattern, pb.pattern, "{context}: action[{i}].patterns[{j}].pattern");
            }
            assert_eq!(aa.command, bb.command, "{context}: action[{i}].command");
            assert_eq!(aa.world, bb.world, "{context}: action[{i}].world");
            assert_eq!(aa.enabled, bb.enabled, "{context}: action[{i}].enabled");
            assert_eq!(aa.startup, bb.startup, "{context}: action[{i}].startup");
        }
        assert_eq!(a.tls_proxy_enabled, b.tls_proxy_enabled, "{context}: tls_proxy_enabled");
        assert_eq!(a.dictionary_path, b.dictionary_path, "{context}: dictionary_path");
        assert_eq!(a.editor_side.name(), b.editor_side.name(), "{context}: editor_side");
        assert_eq!(a.mouse_enabled, b.mouse_enabled, "{context}: mouse_enabled");
        assert_eq!(a.zwj_enabled, b.zwj_enabled, "{context}: zwj_enabled");
        assert_eq!(a.new_line_indicator, b.new_line_indicator, "{context}: new_line_indicator");
        assert_eq!(a.tts_mode, b.tts_mode, "{context}: tts_mode");
        assert_eq!(a.scrollback_enabled, b.scrollback_enabled, "{context}: scrollback_enabled");
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
        assert_eq!(a.auto_reconnect_secs, b.auto_reconnect_secs, "{context}: auto_reconnect_secs");
        assert_eq!(a.auto_reconnect_on_web, b.auto_reconnect_on_web, "{context}: auto_reconnect_on_web");
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
    fn test_serialize_settings_for_export_plaintext_secrets() {
        let mut app = App::new();
        app.settings = make_non_default_settings();
        let mut world = World::new("testworld");
        world.settings = make_non_default_world_settings();
        app.worlds.push(world);

        let exported = serialize_settings_for_export(&app);

        // Every secret must appear in plaintext in the export...
        assert!(exported.contains("websocket_password=testpass\n"),
            "export should contain plaintext ws password:\n{exported}");
        assert!(exported.contains("websocket_auth_key=key1|"),
            "export should contain plaintext auth key:\n{exported}");
        assert!(exported.contains("password=testpassword\n"),
            "export should contain plaintext world password:\n{exported}");
        assert!(exported.contains("slack_token=slack_tok\n"),
            "export should contain plaintext slack token:\n{exported}");
        assert!(exported.contains("discord_token=disc_tok\n"),
            "export should contain plaintext discord token:\n{exported}");

        // ...but the real on-disk save (sharing the same write_settings_dat helper) must
        // still encrypt at rest - this is the regression check that the refactor didn't
        // accidentally flip plaintext_secrets for the real save path.
        let tmp = std::env::temp_dir().join("clay_test_export_vs_disk.dat");
        let _ = std::fs::remove_file(&tmp);
        save_settings_to_path(&app, &tmp).expect("save_settings_to_path failed");
        let on_disk = std::fs::read_to_string(&tmp).expect("read saved settings.dat");
        let _ = std::fs::remove_file(&tmp);

        assert!(!on_disk.contains("websocket_password=testpass\n"),
            "on-disk save must not contain the plaintext ws password:\n{on_disk}");
        assert!(!on_disk.contains("password=testpassword\n"),
            "on-disk save must not contain the plaintext world password:\n{on_disk}");

        // Sanity: the encrypted-at-rest value on disk still decrypts back to the original,
        // i.e. export plaintext and on-disk ciphertext are two views of the same secret.
        let stored_line = on_disk.lines().find(|l| l.starts_with("password="))
            .expect("password line present in saved file");
        let stored = stored_line.trim_start_matches("password=");
        assert_eq!(decrypt_password(stored), "testpassword");
    }

    #[test]
    fn test_import_merge_remote_wins_and_reencrypts() {
        // "Instance A" — the export source (what /import connects out to).
        let mut app_a = App::new();
        app_a.settings.websocket_password = "a_ws_pass".to_string();

        let mut shared_a = World::new("shared_world");
        shared_a.settings.hostname = "a-host.example.com".to_string();
        shared_a.settings.password = "a_world_pass".to_string();
        app_a.worlds.push(shared_a);

        let mut remote_only = World::new("remote_only_world");
        remote_only.settings.hostname = "remote-only.example.com".to_string();
        remote_only.settings.password = "remote_only_pass".to_string();
        app_a.worlds.push(remote_only);

        app_a.theme_file.set_theme("a_theme", theme::ThemeColors::dark_default());
        app_a.keybindings.set_binding("F5", "remote_action");

        // "Instance B" — the local instance running /import, with its own pre-existing state.
        let mut app_b = App::new();
        app_b.settings.websocket_password = "b_ws_pass".to_string();

        let mut shared_b = World::new("shared_world");
        shared_b.settings.hostname = "b-host.example.com".to_string();
        shared_b.settings.password = "b_world_pass".to_string();
        app_b.worlds.push(shared_b);

        let mut local_only = World::new("local_only_world");
        local_only.settings.hostname = "local-only.example.com".to_string();
        local_only.settings.password = "local_only_pass".to_string();
        app_b.worlds.push(local_only);

        app_b.theme_file.set_theme("b_theme", theme::ThemeColors::light_default());
        app_b.keybindings.set_binding("F6", "local_action");

        // Export A, exactly as the RequestSettingsExport handler would build it, and merge
        // into B, exactly as the /import driver will (plan step 6).
        let settings_dat = serialize_settings_for_export(&app_a);
        let theme_dat = app_a.theme_file.generate_file_content();
        let keybindings_dat = app_a.keybindings.to_dat_string();
        merge_settings_dat(&mut app_b, &settings_dat);
        merge_theme_dat(&mut app_b, &theme_dat);
        merge_keybindings_dat(&mut app_b, &keybindings_dat);

        // Remote wins on the conflicting global key.
        assert_eq!(app_b.settings.websocket_password, "a_ws_pass");

        // Remote wins on the conflicting world name; remote-only world is added.
        assert_eq!(app_b.worlds.len(), 3, "shared + remote_only + local_only");
        let shared = app_b.worlds.iter().find(|w| w.name == "shared_world").expect("shared_world");
        assert_eq!(shared.settings.hostname, "a-host.example.com");
        assert_eq!(shared.settings.password, "a_world_pass");
        let remote_only = app_b.worlds.iter().find(|w| w.name == "remote_only_world").expect("remote_only_world");
        assert_eq!(remote_only.settings.password, "remote_only_pass");

        // B's local-only world survives untouched.
        let local_only = app_b.worlds.iter().find(|w| w.name == "local_only_world").expect("local_only_world");
        assert_eq!(local_only.settings.hostname, "local-only.example.com");
        assert_eq!(local_only.settings.password, "local_only_pass");

        // Remote theme/keybinding are added; B's local-only theme/keybinding survive.
        assert!(app_b.theme_file.themes.contains_key("a_theme"));
        assert!(app_b.theme_file.themes.contains_key("b_theme"));
        assert_eq!(app_b.keybindings.get_action("F5"), Some("remote_action"));
        assert_eq!(app_b.keybindings.get_action("F6"), Some("local_action"));

        // Re-encryption: saving B to disk now must store every password/token encrypted at
        // rest (never the plaintext that arrived over the wire), and it must decrypt back to
        // the original. (A real cross-machine /import re-encrypts under a *different*
        // secure.key than the sender used — this process only has one machine_key() to work
        // with, but that's exactly why the mechanism matters: the merge step never touches
        // encrypt/decrypt at all, so the same code path is exercised regardless of whose key
        // it is — see load_settings_from_str's doc comment.)
        let tmp = std::env::temp_dir().join("clay_test_import_merge_reencrypt.dat");
        let _ = std::fs::remove_file(&tmp);
        save_settings_to_path(&app_b, &tmp).expect("save_settings_to_path failed");
        let on_disk = std::fs::read_to_string(&tmp).expect("read saved settings.dat");
        let _ = std::fs::remove_file(&tmp);

        assert!(!on_disk.contains("a_world_pass"), "plaintext password must not reach disk:\n{on_disk}");
        assert!(!on_disk.contains("a_ws_pass"), "plaintext ws password must not reach disk:\n{on_disk}");

        let ws_line = on_disk.lines().find(|l| l.starts_with("websocket_password=")).expect("websocket_password line");
        let ws_stored = ws_line.trim_start_matches("websocket_password=");
        assert!(ws_stored.starts_with("ENC:"), "websocket_password should be encrypted at rest: {ws_stored}");
        assert_eq!(decrypt_password(ws_stored), "a_ws_pass");

        let password_lines: Vec<&str> = on_disk.lines().filter(|l| l.starts_with("password=")).collect();
        assert_eq!(password_lines.len(), 3, "shared_world + remote_only_world + local_only_world:\n{on_disk}");
        for line in password_lines {
            let stored = line.trim_start_matches("password=");
            assert!(stored.starts_with("ENC:"), "world password should be encrypted at rest: {stored}");
        }
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
        assert_ne!(non_default.web_font_weight, default.web_font_weight, "web_font_weight should differ");
        assert_ne!(non_default.web_secure, default.web_secure, "web_secure should differ");
        assert_ne!(non_default.http_enabled, default.http_enabled, "http_enabled should differ");
        assert_ne!(non_default.http_port, default.http_port, "http_port should differ");
        assert_ne!(non_default.web_path, default.web_path, "web_path should differ");
        assert_ne!(non_default.websocket_password, default.websocket_password, "websocket_password should differ");
        assert_ne!(non_default.websocket_allow_list, default.websocket_allow_list, "websocket_allow_list should differ");
        assert_ne!(non_default.websocket_cert_file, default.websocket_cert_file, "websocket_cert_file should differ");
        assert_ne!(non_default.websocket_key_file, default.websocket_key_file, "websocket_key_file should differ");
        assert!(non_default.websocket_auth_key.is_some(), "websocket_auth_key should be Some");
        assert!(!non_default.actions.is_empty(), "actions should be non-empty");
        assert_ne!(non_default.tls_proxy_enabled, default.tls_proxy_enabled, "tls_proxy_enabled should differ");
        assert_ne!(non_default.dictionary_path, default.dictionary_path, "dictionary_path should differ");
        assert_ne!(non_default.editor_side.name(), default.editor_side.name(), "editor_side should differ");
        assert_ne!(non_default.mouse_enabled, default.mouse_enabled, "mouse_enabled should differ");
        assert_ne!(non_default.zwj_enabled, default.zwj_enabled, "zwj_enabled should differ");
        assert_ne!(non_default.new_line_indicator, default.new_line_indicator, "new_line_indicator should differ");
        assert_ne!(non_default.tts_mode, default.tts_mode, "tts_mode should differ");
        assert_ne!(non_default.scrollback_enabled, default.scrollback_enabled, "scrollback_enabled should differ");
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
        assert_ne!(non_default.auto_reconnect_secs, default.auto_reconnect_secs, "auto_reconnect_secs should differ");
        assert_ne!(non_default.auto_reconnect_on_web, default.auto_reconnect_on_web, "auto_reconnect_on_web should differ");
    }

    #[test]
    fn test_read_global_section_parses_only_global_keys() {
        let tmp = std::env::temp_dir().join("clay_test_read_global_section.dat");
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(
            &tmp,
            "[global]\nscrollback_enabled=true\ndebug_enabled=false\n\n[world:test]\nhostname=example.com\n",
        ).unwrap();

        let global = read_global_section(&tmp);
        assert_eq!(global.get("scrollback_enabled").map(String::as_str), Some("true"));
        assert_eq!(global.get("debug_enabled").map(String::as_str), Some("false"));
        assert!(global.get("hostname").is_none(), "world-section keys must not leak into the global map");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_read_global_section_missing_file_returns_empty() {
        let tmp = std::env::temp_dir().join("clay_test_read_global_section_missing.dat");
        let _ = std::fs::remove_file(&tmp);
        assert!(read_global_section(&tmp).is_empty());
    }

    #[test]
    fn test_append_settings_audit_log_records_changed_keys_and_source() {
        let tmp = std::env::temp_dir().join("clay_test_settings_audit.log");
        let _ = std::fs::remove_file(&tmp);

        let mut old = std::collections::HashMap::new();
        old.insert("scrollback_enabled".to_string(), "true".to_string());
        old.insert("debug_enabled".to_string(), "true".to_string());
        let mut new = std::collections::HashMap::new();
        new.insert("scrollback_enabled".to_string(), "false".to_string());
        new.insert("debug_enabled".to_string(), "true".to_string()); // unchanged

        append_settings_audit_log_to_path(&tmp, "android", &old, &new);

        let contents = std::fs::read_to_string(&tmp).expect("audit log should have been written");
        assert!(contents.contains("source=android"), "should record the source client type");
        assert!(contents.contains("scrollback_enabled: true -> false"), "should record the changed key's old/new values");
        assert!(!contents.contains("debug_enabled: true -> true"), "unchanged keys should not be logged");
        assert!(contents.to_lowercase().contains("backtrace"), "should include a backtrace");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_append_settings_audit_log_no_changes_writes_nothing() {
        let tmp = std::env::temp_dir().join("clay_test_settings_audit_no_change.log");
        let _ = std::fs::remove_file(&tmp);

        let mut old = std::collections::HashMap::new();
        old.insert("debug_enabled".to_string(), "true".to_string());
        let new = old.clone();

        append_settings_audit_log_to_path(&tmp, "local", &old, &new);

        assert!(!tmp.exists(), "no file should be created when nothing changed");
    }

    // -----------------------------------------------------------------
    // TOFU known_hosts pin store
    // -----------------------------------------------------------------

    #[test]
    fn test_known_hosts_load_save_roundtrip_via_path() {
        let tmp = std::env::temp_dir().join("clay_test_known_hosts_roundtrip.dat");
        let _ = std::fs::remove_file(&tmp);

        let mut map = std::collections::HashMap::new();
        map.insert("example.com:9000".to_string(), "aa".repeat(32));
        map.insert("other.host:443".to_string(), "bb".repeat(32));
        save_known_hosts_to(&tmp, &map);

        let loaded = load_known_hosts_from(&tmp);
        assert_eq!(loaded.get("example.com:9000"), Some(&"aa".repeat(32)));
        assert_eq!(loaded.get("other.host:443"), Some(&"bb".repeat(32)));
        assert_eq!(loaded.len(), 2);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    #[cfg(unix)]
    fn test_known_hosts_file_permissions_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join("clay_test_known_hosts_perms.dat");
        let _ = std::fs::remove_file(&tmp);

        let mut map = std::collections::HashMap::new();
        map.insert("host:1".to_string(), "cc".repeat(32));
        save_known_hosts_to(&tmp, &map);

        let mode = std::fs::metadata(&tmp).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "known_hosts.dat must be created 0600");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_known_hosts_ignores_comments_and_blank_lines() {
        let tmp = std::env::temp_dir().join("clay_test_known_hosts_comments.dat");
        std::fs::write(&tmp, "# comment\n\nhost:1=deadbeef\n   \n# another\nhost:2=cafef00d\n").unwrap();

        let loaded = load_known_hosts_from(&tmp);
        assert_eq!(loaded.get("host:1"), Some(&"deadbeef".to_string()));
        assert_eq!(loaded.get("host:2"), Some(&"cafef00d".to_string()));
        assert_eq!(loaded.len(), 2);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_pin_lifecycle_add_get_replace() {
        // Use a unique host key so this doesn't collide with any real pin the
        // developer running these tests might already have.
        let host = "clay-test-pin-lifecycle.invalid:9999";
        remove_pin(host);

        // No pin yet
        assert_eq!(get_pin(host), None);

        // First add pins silently
        assert!(add_pin(host, "1111"));
        assert_eq!(get_pin(host), Some("1111".to_string()));

        // Adding again with a different fingerprint does NOT overwrite (add_pin
        // is first-connect-only; a real mismatch must go through replace_pin)
        assert!(!add_pin(host, "2222"));
        assert_eq!(get_pin(host), Some("1111".to_string()));

        // replace_pin overwrites unconditionally (the "trust new certificate" action)
        replace_pin(host, "3333");
        assert_eq!(get_pin(host), Some("3333".to_string()));

        remove_pin(host);
        assert_eq!(get_pin(host), None);
    }
}
