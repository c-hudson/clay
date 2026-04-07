// Module declarations
pub mod encoding;
pub mod telnet;
pub mod spell;
pub mod input;
pub mod util;
pub mod websocket;
pub mod ansi_music;
pub mod tf;
pub mod popup;
pub mod actions;
pub mod http;
pub mod persistence;
pub mod daemon;
pub mod theme;
pub mod keybindings;
pub mod input_handler;
pub mod rendering;
pub mod audio;
pub mod platform;
pub mod commands;
pub mod remote_client;
pub mod tts;
#[cfg(feature = "webview-gui")]
pub mod webview_gui;
pub mod testserver;
#[cfg(test)]
pub mod testharness;

// Version information
const VERSION: &str = "1.0.1-beta";
const BUILD_HASH: &str = env!("BUILD_HASH");
const BUILD_DATE: &str = env!("BUILD_DATE");

// Custom config file path (set via --conf=<path> argument)
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
static CUSTOM_CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Global debug flag — set from settings, checked by debug_log and file writes
pub(crate) static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Tracks whether the startup header has been written to clay.debug.log
static DEBUG_LOG_HEADER_WRITTEN: AtomicBool = AtomicBool::new(false);
/// Tracks whether the startup header has been written to clay.output.debug
static OUTPUT_DEBUG_HEADER_WRITTEN: AtomicBool = AtomicBool::new(false);
/// Startup time stored as Unix timestamp (seconds since epoch)
static STARTUP_TIME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Flag set by IPC handler to request reload — checked by headless event loop
pub static GUI_RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);
/// Flag set by HTTP server after successful bind — checked by GUI readiness wait
pub static GUI_HTTP_READY: AtomicBool = AtomicBool::new(false);

/// Check if debug logging is enabled
pub fn is_debug_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

/// Set a custom config file path (call early in main before loading settings)
pub fn set_custom_config_path(path: PathBuf) {
    let _ = CUSTOM_CONFIG_PATH.set(path);
}

/// Get the custom config path if one was set
pub fn get_custom_config_path() -> Option<&'static PathBuf> {
    CUSTOM_CONFIG_PATH.get()
}

/// Get the full version string including build hash
pub fn get_version_string() -> String {
    format!("Clay v{} (build {}-{})", VERSION, BUILD_DATE, BUILD_HASH)
}

// Re-export commonly used types from modules
pub use encoding::{Encoding, Theme, WorldSwitchMode, convert_discord_emojis, convert_discord_emojis_with_links, colorize_square_emojis, is_visually_empty, is_ansi_only_line, has_background_color, strip_non_sgr_sequences, wrap_urls_with_osc8};
pub use telnet::{
    WriteCommand, StreamReader, StreamWriter, AutoConnectType, KeepAliveType,
    process_telnet, find_safe_split_point, build_naws_subnegotiation, build_ttype_response, TelnetResult,
    build_gmcp_message, build_msdp_request, build_msdp_set,
    build_charset_accepted, build_charset_rejected,
    TELNET_IAC, TELNET_NOP, TELNET_GA, TELNET_OPT_NAWS, TELNET_OPT_CHARSET,
};
pub use spell::{SpellChecker, SpellState};
pub use input::{InputArea, display_width, display_width_chars, chars_for_display_width};
pub use util::{get_binary_name, strip_ansi_codes, visual_line_count, get_current_time_12hr, strip_mud_tag, truncate_str, convert_temperatures, parse_discord_timestamps, local_time_from_epoch, local_time_now, color_name_to_ansi_bg};
pub use websocket::{
    WsMessage, WorldStateMsg, WorldSettingsMsg, GlobalSettingsMsg, TimestampedLine,
    WsClientInfo, WebSocketServer,
    hash_password, hash_with_challenge, is_ip_in_allow_list,
};
pub use actions::{
    Action, MatchType, ActionTriggerResult,
    split_action_commands, substitute_action_args, substitute_pattern_captures,
    wildcard_to_regex, execute_recall, check_action_triggers,
    compile_action_patterns, line_matches_compiled_patterns,
    compile_all_action_regexes,
};
pub use http::{HttpsServer, HttpServer, BanList, start_https_server, start_http_server, log_http_404, log_ws_auth, log_ban};
pub use persistence::{
    encrypt_password, decrypt_password,
    save_settings, load_settings,
    load_multiuser_settings, save_multiuser_settings,
    load_reload_state,
    unescape_string,
};
#[cfg(not(target_os = "android"))]
pub use persistence::save_reload_state;
pub use daemon::{
    run_daemon_server, run_multiuser_server,
    generate_splash_strings,
};
use input_handler::*;
use rendering::*;
use commands::*;
use platform::*;

use std::io::{self, stdout, Write as IoWrite};
#[cfg(unix)]
use std::os::unix::io::{FromRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::FromRawSocket;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Platform-specific socket descriptor type for hot reload preservation
#[cfg(unix)]
pub(crate) type SocketFd = i32;  // same as RawFd
#[cfg(not(unix))]
pub(crate) type SocketFd = i64;
use bytes::BytesMut;
use crossterm::{
    cursor,
    event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind, MouseButton, EnableMouseCapture, DisableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, Clear, ClearType},
};
use futures::StreamExt;
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
};
#[cfg(all(unix, not(target_os = "android")))]
use tokio::signal::unix::{signal, SignalKind};
use std::sync::Arc;
// flate2 used in reader tasks via flate2::Decompress::new()

// ============================================================================
// Web Settings Popup (/web command)
// ============================================================================


pub struct ConfirmDialog {
    visible: bool,
    message: String,
    yes_selected: bool,
    action: ConfirmAction,
}

#[derive(Clone, Copy, PartialEq)]
enum ConfirmAction {
    None,
}

impl ConfirmDialog {
    fn new() -> Self {
        Self {
            visible: false,
            message: String::new(),
            yes_selected: false,
            action: ConfirmAction::None,
        }
    }

    fn close(&mut self) {
        self.visible = false;
        self.action = ConfirmAction::None;
    }
}

pub struct FilterPopup {
    visible: bool,
    filter_text: String,
    cursor: usize,
    filtered_indices: Vec<usize>,  // Indices of matching lines in output_lines
    scroll_offset: usize,          // Scroll position within filtered results
}

impl FilterPopup {
    fn new() -> Self {
        Self {
            visible: false,
            filter_text: String::new(),
            cursor: 0,
            filtered_indices: Vec::new(),
            scroll_offset: 0,
        }
    }

    fn open(&mut self) {
        self.visible = true;
        self.filter_text.clear();
        self.cursor = 0;
        self.filtered_indices.clear();
        self.scroll_offset = 0;
    }

    fn close(&mut self) {
        self.visible = false;
        self.filter_text.clear();
        self.filtered_indices.clear();
        self.scroll_offset = 0;
    }

    fn update_filter(&mut self, output_lines: &[OutputLine]) {
        if self.filter_text.is_empty() {
            self.filtered_indices = (0..output_lines.len()).collect();
        } else {
            // Check if pattern has wildcards
            let has_wildcards = self.filter_text.contains('*') || self.filter_text.contains('?');

            if has_wildcards {
                // Use wildcard matching with regex
                if let Some(regex) = filter_wildcard_to_regex(&self.filter_text) {
                    self.filtered_indices = output_lines
                        .iter()
                        .enumerate()
                        .filter(|(_, line)| {
                            let plain = strip_ansi_codes(&line.text);
                            regex.is_match(&plain)
                        })
                        .map(|(i, _)| i)
                        .collect();
                } else {
                    // Invalid regex, show no matches
                    self.filtered_indices.clear();
                }
            } else {
                // Simple substring matching (case-insensitive)
                let filter_lower = self.filter_text.to_lowercase();
                self.filtered_indices = output_lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| {
                        let plain = strip_ansi_codes(&line.text);
                        plain.to_lowercase().contains(&filter_lower)
                    })
                    .map(|(i, _)| i)
                    .collect();
            }
        }
        // Reset scroll to end (most recent matches)
        self.scroll_offset = self.filtered_indices.len().saturating_sub(1);
    }
}

/// Which side of the screen the editor appears on
#[derive(Clone, Copy, PartialEq, Default)]
pub enum EditorSide {
    #[default]
    Left,
    Right,
}

impl EditorSide {
    pub fn name(&self) -> &'static str {
        match self {
            EditorSide::Left => "left",
            EditorSide::Right => "right",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "right" => EditorSide::Right,
            _ => EditorSide::Left,
        }
    }
}

/// Which element has focus when editor is open
#[derive(Clone, Copy, PartialEq, Default)]
pub enum EditorFocus {
    #[default]
    Editor,
    Input,
}

/// State for the split-screen text editor
pub struct EditorState {
    /// Whether the editor is currently visible
    pub visible: bool,
    /// Which element has focus (editor or input)
    pub focus: EditorFocus,
    /// The text buffer being edited
    pub buffer: String,
    /// Cursor position in the buffer (character index)
    pub cursor_position: usize,
    /// Current line number (0-indexed)
    pub cursor_line: usize,
    /// Current column number (0-indexed)
    pub cursor_col: usize,
    /// Scroll offset for viewing (line index)
    pub scroll_offset: usize,
    /// Original content (for detecting changes)
    pub original_content: String,
    /// Whether the buffer has been modified
    pub dirty: bool,
    /// File path if editing an external file
    pub file_path: Option<PathBuf>,
    /// World index if editing world notes
    pub world_index: Option<usize>,
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            visible: false,
            focus: EditorFocus::Editor,
            buffer: String::new(),
            cursor_position: 0,
            cursor_line: 0,
            cursor_col: 0,
            scroll_offset: 0,
            original_content: String::new(),
            dirty: false,
            file_path: None,
            world_index: None,
        }
    }

    /// Open editor for world notes
    pub fn open_notes(&mut self, world_index: usize, content: &str) {
        self.visible = true;
        self.focus = EditorFocus::Editor;
        self.buffer = content.to_string();
        self.cursor_position = content.len();
        self.original_content = content.to_string();
        self.dirty = false;
        self.file_path = None;
        self.world_index = Some(world_index);
        self.update_cursor_position();
    }

    /// Open editor for an external file
    pub fn open_file(&mut self, path: PathBuf, content: &str) {
        self.visible = true;
        self.focus = EditorFocus::Editor;
        self.buffer = content.to_string();
        self.cursor_position = content.len();
        self.original_content = content.to_string();
        self.dirty = false;
        self.file_path = Some(path);
        self.world_index = None;
        self.update_cursor_position();
    }

    /// Close the editor
    pub fn close(&mut self) {
        self.visible = false;
        self.buffer.clear();
        self.original_content.clear();
        self.file_path = None;
        self.world_index = None;
        self.cursor_position = 0;
        self.cursor_line = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.dirty = false;
    }

    /// Toggle focus between editor and input
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            EditorFocus::Editor => EditorFocus::Input,
            EditorFocus::Input => EditorFocus::Editor,
        };
    }

    /// Get the title for the editor panel
    pub fn title(&self, world_name: Option<&str>) -> String {
        if let Some(ref path) = self.file_path {
            let filename = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            if self.dirty {
                format!("Edit: {} [modified]", filename)
            } else {
                format!("Edit: {}", filename)
            }
        } else if let Some(name) = world_name {
            if self.dirty {
                format!("Notes: {} [modified]", name)
            } else {
                format!("Notes: {}", name)
            }
        } else {
            "Editor".to_string()
        }
    }

    /// Update cursor_line and cursor_col from cursor_position
    fn update_cursor_position(&mut self) {
        let mut line = 0;
        let mut col = 0;
        for (i, c) in self.buffer.chars().enumerate() {
            if i == self.cursor_position {
                break;
            }
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        self.cursor_line = line;
        self.cursor_col = col;
    }

    /// Get lines of the buffer
    pub fn lines(&self) -> Vec<&str> {
        self.buffer.split('\n').collect()
    }

    /// Get total line count (logical lines)
    pub fn line_count(&self) -> usize {
        self.buffer.split('\n').count()
    }

    /// Get total visual line count (with wrapping at given width)
    pub fn visual_line_count(&self, width: usize) -> usize {
        if width == 0 {
            return self.line_count();
        }
        let mut count = 0;
        for line in self.buffer.split('\n') {
            let len = line.chars().count();
            if len == 0 {
                count += 1;
            } else {
                count += len.div_ceil(width); // Ceiling division
            }
        }
        count
    }

    /// Get the visual line index where the cursor is (with wrapping at given width)
    pub fn cursor_visual_line(&self, width: usize) -> usize {
        if width == 0 {
            return self.cursor_line;
        }
        let mut visual_line = 0;
        for (line_idx, line) in self.buffer.split('\n').enumerate() {
            let len = line.chars().count();
            if line_idx == self.cursor_line {
                // Cursor is on this logical line
                // Which visual line within this logical line?
                let visual_offset = self.cursor_col / width;
                return visual_line + visual_offset;
            }
            // Count visual lines for this logical line
            if len == 0 {
                visual_line += 1;
            } else {
                visual_line += len.div_ceil(width);
            }
        }
        visual_line
    }

    /// Move cursor up one visual (wrapped) line
    pub fn cursor_up(&mut self, width: usize) {
        if width == 0 {
            // Fallback to logical line movement
            if self.cursor_line == 0 { return; }
            let lines = self.lines();
            let target_line = self.cursor_line - 1;
            let target_col = self.cursor_col.min(lines[target_line].chars().count());
            let mut pos = 0;
            for (i, line) in lines.iter().enumerate() {
                if i == target_line { pos += target_col; break; }
                pos += line.chars().count() + 1;
            }
            self.cursor_position = pos;
            self.update_cursor_position();
            return;
        }

        let visual_col_in_visual_row = self.cursor_col % width;
        let visual_row_in_line = self.cursor_col / width;

        if visual_row_in_line > 0 {
            // Move up within the same logical line
            let new_col = self.cursor_col - width;
            // cursor_position moves back by width characters
            self.cursor_position -= width;
            self.cursor_col = new_col;
        } else {
            // Move to previous logical line's last visual row
            if self.cursor_line == 0 { return; }
            let lines = self.lines();
            let prev_line = self.cursor_line - 1;
            let prev_len = lines[prev_line].chars().count();
            let prev_last_visual_row = if prev_len == 0 { 0 } else { (prev_len - 1) / width };
            let target_col_in_prev = (prev_last_visual_row * width) + visual_col_in_visual_row;
            let target_col = target_col_in_prev.min(prev_len);

            let mut pos = 0;
            for (i, line) in lines.iter().enumerate() {
                if i == prev_line { pos += target_col; break; }
                pos += line.chars().count() + 1;
            }
            self.cursor_position = pos;
            self.update_cursor_position();
        }
    }

    /// Move cursor down one visual (wrapped) line
    pub fn cursor_down(&mut self, width: usize) {
        if width == 0 {
            // Fallback to logical line movement
            let lines = self.lines();
            if self.cursor_line >= lines.len() - 1 { return; }
            let target_line = self.cursor_line + 1;
            let target_col = self.cursor_col.min(lines[target_line].chars().count());
            let mut pos = 0;
            for (i, line) in lines.iter().enumerate() {
                if i == target_line { pos += target_col; break; }
                pos += line.chars().count() + 1;
            }
            self.cursor_position = pos;
            self.update_cursor_position();
            return;
        }

        let lines = self.lines();
        let cur_len = lines[self.cursor_line].chars().count();
        let visual_col_in_visual_row = self.cursor_col % width;
        let visual_row_in_line = self.cursor_col / width;
        let last_visual_row = if cur_len == 0 { 0 } else { (cur_len - 1) / width };

        if visual_row_in_line < last_visual_row {
            // Move down within the same logical line
            let new_col = (self.cursor_col + width).min(cur_len);
            let advance = new_col - self.cursor_col;
            self.cursor_position += advance;
            self.cursor_col = new_col;
        } else {
            // Move to next logical line's first visual row
            if self.cursor_line >= lines.len() - 1 { return; }
            let next_line = self.cursor_line + 1;
            let next_len = lines[next_line].chars().count();
            let target_col = visual_col_in_visual_row.min(next_len);

            let mut pos = 0;
            for (i, line) in lines.iter().enumerate() {
                if i == next_line { pos += target_col; break; }
                pos += line.chars().count() + 1;
            }
            self.cursor_position = pos;
            self.update_cursor_position();
        }
    }

    /// Move cursor left one character
    pub fn cursor_left(&mut self) {
        if self.cursor_position > 0 {
            // Move back by one character
            let chars: Vec<char> = self.buffer.chars().collect();
            self.cursor_position = chars[..self.cursor_position].len() - 1;
            // Actually need character count up to new position
            self.cursor_position = self.cursor_position.min(chars.len());
            self.update_cursor_position();
        }
    }

    /// Move cursor right one character
    pub fn cursor_right(&mut self) {
        let char_count = self.buffer.chars().count();
        if self.cursor_position < char_count {
            self.cursor_position += 1;
            self.update_cursor_position();
        }
    }

    /// Move cursor to start of current line
    pub fn cursor_home(&mut self) {
        let lines = self.lines();
        let mut pos = 0;
        for (i, line) in lines.iter().enumerate() {
            if i == self.cursor_line {
                break;
            }
            pos += line.chars().count() + 1;
        }
        self.cursor_position = pos;
        self.update_cursor_position();
    }

    /// Move cursor to end of current line
    pub fn cursor_end(&mut self) {
        let lines = self.lines();
        let mut pos = 0;
        for (i, line) in lines.iter().enumerate() {
            if i == self.cursor_line {
                pos += line.chars().count();
                break;
            }
            pos += line.chars().count() + 1;
        }
        self.cursor_position = pos;
        self.update_cursor_position();
    }

    /// Insert a character at cursor position
    pub fn insert_char(&mut self, c: char) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let pos = self.cursor_position.min(chars.len());
        let before: String = chars[..pos].iter().collect();
        let after: String = chars[pos..].iter().collect();
        self.buffer = format!("{}{}{}", before, c, after);
        self.cursor_position = pos + 1;
        self.dirty = self.buffer != self.original_content;
        self.update_cursor_position();
    }

    /// Delete character before cursor (backspace)
    pub fn delete_backward(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        let chars: Vec<char> = self.buffer.chars().collect();
        let pos = self.cursor_position.min(chars.len());
        let before: String = chars[..pos - 1].iter().collect();
        let after: String = chars[pos..].iter().collect();
        self.buffer = format!("{}{}", before, after);
        self.cursor_position = pos - 1;
        self.dirty = self.buffer != self.original_content;
        self.update_cursor_position();
    }

    /// Delete character after cursor (delete)
    pub fn delete_forward(&mut self) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let pos = self.cursor_position.min(chars.len());
        if pos >= chars.len() {
            return;
        }
        let before: String = chars[..pos].iter().collect();
        let after: String = chars[pos + 1..].iter().collect();
        self.buffer = format!("{}{}", before, after);
        self.dirty = self.buffer != self.original_content;
        self.update_cursor_position();
    }

    /// Scroll up one page (using visual lines with wrapping)
    pub fn page_up(&mut self, visible_lines: usize, _width: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(visible_lines.saturating_sub(1));
    }

    /// Scroll down one page (using visual lines with wrapping)
    pub fn page_down(&mut self, visible_lines: usize, width: usize) {
        let max_scroll = self.visual_line_count(width).saturating_sub(visible_lines);
        self.scroll_offset = (self.scroll_offset + visible_lines.saturating_sub(1)).min(max_scroll);
    }

    /// Ensure cursor is visible (adjust scroll based on visual lines with wrapping)
    pub fn ensure_cursor_visible(&mut self, visible_lines: usize, width: usize) {
        let cursor_visual = self.cursor_visual_line(width);
        if cursor_visual < self.scroll_offset {
            self.scroll_offset = cursor_visual;
        } else if cursor_visual >= self.scroll_offset + visible_lines {
            self.scroll_offset = cursor_visual - visible_lines + 1;
        }
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply TF attributes to text for /substitute command.
/// Attribute string format: C<color> for foreground, B for bold, etc.
/// Examples: "Cred" = red, "Cbold" = bold, "Cgreen" = green
fn apply_tf_attrs(text: &str, attrs: &str) -> String {
    let mut result = String::new();
    let mut has_attr = false;

    // Parse attributes - Cred, Cgreen, Cbold, etc.
    let attrs_lower = attrs.to_lowercase();
    if attrs_lower.contains("cred") || attrs_lower.contains("red") {
        result.push_str("\x1b[1;31m");  // Bold red
        has_attr = true;
    } else if attrs_lower.contains("cgreen") || attrs_lower.contains("green") {
        result.push_str("\x1b[1;32m");  // Bold green
        has_attr = true;
    } else if attrs_lower.contains("cyellow") || attrs_lower.contains("yellow") {
        result.push_str("\x1b[1;33m");  // Bold yellow
        has_attr = true;
    } else if attrs_lower.contains("cblue") || attrs_lower.contains("blue") {
        result.push_str("\x1b[1;34m");  // Bold blue
        has_attr = true;
    } else if attrs_lower.contains("cmagenta") || attrs_lower.contains("magenta") {
        result.push_str("\x1b[1;35m");  // Bold magenta
        has_attr = true;
    } else if attrs_lower.contains("ccyan") || attrs_lower.contains("cyan") {
        result.push_str("\x1b[1;36m");  // Bold cyan
        has_attr = true;
    } else if attrs_lower.contains("cwhite") || attrs_lower.contains("white") {
        result.push_str("\x1b[1;37m");  // Bold white
        has_attr = true;
    } else if attrs_lower.contains("cbold") || attrs_lower.contains("bold") {
        result.push_str("\x1b[1m");  // Bold
        has_attr = true;
    }

    result.push_str(text);

    if has_attr {
        result.push_str("\x1b[0m");  // Reset
    }

    result
}

/// Result of running both Clay action triggers and TF triggers on a line.
struct TriggerProcessingResult {
    /// Whether the line should be gagged (suppressed from display)
    pub is_gagged: bool,
    /// Commands to send to the MUD server
    pub send_commands: Vec<String>,
    /// Clay commands to execute (e.g. /connect, /worlds)
    pub clay_commands: Vec<String>,
    /// Messages to display locally (from /echo, substitution, etc.)
    pub messages: Vec<String>,
    /// Highlight color from action triggers
    pub highlight_color: Option<String>,
}

/// Parsed BAMF portal information
struct BamfPortal {
    name: String,
    host: String,
    port: String,
}

/// Parse a BAMF portal line: #### Please reconnect to name@addr (host) port NNN ####
fn parse_bamf_portal(line: &str) -> Option<BamfPortal> {
    let stripped = util::strip_ansi_codes(line);
    let trimmed = stripped.trim();
    if !trimmed.starts_with("####") || !trimmed.ends_with("####") {
        return None;
    }
    // Extract inner text between #### markers
    let inner = trimmed.trim_start_matches('#').trim_end_matches('#').trim();
    // Expected: "Please reconnect to Name@addr (hostname) port NNN"
    let inner_lower = inner.to_lowercase();
    if !inner_lower.starts_with("please reconnect to ") {
        return None;
    }
    let rest = &inner[20..]; // after "Please reconnect to "
    // Parse: Name@addr (hostname) port NNN
    // Find the port number at the end
    let parts: Vec<&str> = rest.rsplitn(2, "port").collect();
    if parts.len() != 2 {
        return None;
    }
    let port = parts[0].trim().trim_end_matches('#').trim();
    let before_port = parts[1].trim();
    // before_port: "Name@addr (hostname)" or "Name@addr"
    // Extract name from Name@addr
    let at_pos = before_port.find('@')?;
    let name = before_port[..at_pos].trim();
    // Extract hostname - prefer the (hostname) if present, otherwise use addr
    let after_at = &before_port[at_pos + 1..];
    let host = if let Some(paren_start) = after_at.find('(') {
        if let Some(paren_end) = after_at.find(')') {
            after_at[paren_start + 1..paren_end].trim()
        } else {
            after_at.split_whitespace().next().unwrap_or(after_at).trim()
        }
    } else {
        after_at.split_whitespace().next().unwrap_or(after_at).trim()
    };
    if name.is_empty() || host.is_empty() || port.is_empty() {
        return None;
    }
    Some(BamfPortal {
        name: name.to_string(),
        host: host.to_string(),
        port: port.to_string(),
    })
}

/// Run both Clay action triggers and TF triggers on a line.
/// Returns combined results from both trigger systems.
fn process_triggers(
    line: &str,
    world_name: &str,
    actions: &[actions::Action],
    tf_engine: &mut tf::TfEngine,
) -> TriggerProcessingResult {
    let mut result = TriggerProcessingResult {
        is_gagged: false,
        send_commands: Vec::new(),
        clay_commands: Vec::new(),
        messages: Vec::new(),
        highlight_color: None,
    };

    // Check Clay action triggers
    if let Some(action_result) = check_action_triggers(line, world_name, actions) {
        result.send_commands.extend(action_result.commands);
        result.is_gagged = action_result.should_gag;
        result.highlight_color = action_result.highlight_color;
    }

    // Check TF triggers
    let tf_result = tf::bridge::process_line(tf_engine, line, Some(world_name));
    result.send_commands.extend(tf_result.send_commands);
    result.clay_commands.extend(tf_result.clay_commands);
    result.messages.extend(tf_result.messages);
    result.is_gagged = result.is_gagged || tf_result.should_gag;

    // Handle substitution: gag original, output substitute with attributes
    if let Some((sub_text, sub_attrs)) = tf_result.substitution {
        result.is_gagged = true;
        let sub_with_attrs = if sub_attrs.contains('C') || sub_attrs.contains('B') {
            apply_tf_attrs(&sub_text, &sub_attrs)
        } else {
            sub_text
        };
        result.messages.push(sub_with_attrs);
    }

    result
}

/// Strip ANSI escape sequences from a line for watchdog/watchname comparison
fn strip_ansi_for_watchdog(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    let mut in_csi = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
            in_csi = false;
        } else if in_escape && !in_csi {
            if c == '[' {
                in_csi = true;
            } else {
                in_escape = false;
            }
        } else if in_csi {
            if ('@'..='~').contains(&c) {
                in_escape = false;
                in_csi = false;
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Convert a wildcard filter pattern to regex for F4 filter popup.
/// Always uses "contains" semantics - patterns match anywhere in the line.
/// Supports \* and \? to match literal asterisk and question mark.
/// Examples:
///   "*foo*" matches any line containing "foo"
///   "foo*" matches any line containing "foo" followed by anything
///   "hel?o" matches any line containing "hello", "helao", etc.
///   "what\?" matches any line containing "what?"
fn filter_wildcard_to_regex(pattern: &str) -> Option<regex::Regex> {
    let mut regex = String::with_capacity(pattern.len() * 2 + 4);

    // No anchoring - always "contains" semantics for filter

    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Check for escape sequences
                match chars.peek() {
                    Some('*') | Some('?') | Some('\\') => {
                        // Escaped wildcard or backslash - treat as literal
                        let escaped = chars.next().unwrap();
                        regex.push('\\');
                        regex.push(escaped);
                    }
                    _ => {
                        // Lone backslash - escape it for regex
                        regex.push_str("\\\\");
                    }
                }
            }
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            // Escape regex special characters
            '.' | '+' | '^' | '$' | '|' | '(' | ')' | '[' | ']' | '{' | '}' => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
    }

    regex::RegexBuilder::new(&regex)
        .case_insensitive(true)
        .build()
        .ok()
}

/// Run a command with a timeout. Returns None if the command fails or times out.
fn run_command_with_timeout(cmd: &str, args: &[&str], timeout_secs: u64) -> Option<String> {
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let output = child.wait_with_output().ok()?;
                return String::from_utf8(output.stdout).ok();
            }
            Ok(Some(_)) => return None, // Exited with error
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

/// Get all non-loopback IPv4 addresses on this machine.
fn get_local_ip_addresses() -> Vec<String> {
    let mut addrs = Vec::new();

    // Use socket trick: connect UDP to a public IP (doesn't send anything)
    // to discover the default local IP
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(local_addr) = socket.local_addr() {
                let ip = local_addr.ip().to_string();
                if ip != "0.0.0.0" && !addrs.contains(&ip) {
                    addrs.push(ip);
                }
            }
        }
    }

    // On Linux, use hostname -I to get all local IPs
    #[cfg(target_os = "linux")]
    {
        if let Some(output) = run_command_with_timeout("hostname", &["-I"], 2) {
            let ips = output;
            for ip in ips.split_whitespace() {
                let ip = ip.trim().to_string();
                if !ip.is_empty() && ip != "127.0.0.1" && !addrs.contains(&ip) {
                    addrs.push(ip);
                }
            }
        }
    }

    addrs
}

/// Generate a self-signed TLS certificate and key, saving to the given paths.
/// Uses rcgen for pure-Rust X.509 cert generation (no external tools needed).
fn generate_self_signed_cert(cert_path: &std::path::Path, key_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use rcgen::{CertificateParams, KeyPair};
    use std::io::Write;

    let mut san_names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];

    // Add all local IP addresses so the cert works across the LAN
    if let Some(name) = run_command_with_timeout("hostname", &[], 2) {
        let name = name.trim().to_string();
        if !name.is_empty() && !san_names.contains(&name) {
            san_names.push(name);
        }
    }
    for iface in get_local_ip_addresses() {
        if !san_names.contains(&iface) {
            san_names.push(iface);
        }
    }

    let params = CertificateParams::new(san_names.clone())?;

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    let mut cert_file = std::fs::File::create(cert_path)?;
    cert_file.write_all(cert.pem().as_bytes())?;

    let mut key_file = std::fs::File::create(key_path)?;
    key_file.write_all(key_pair.serialize_pem().as_bytes())?;

    // Save the SAN list so we can detect IP changes on next startup
    let ips_path = cert_path.with_extension("ips");
    let mut sorted = san_names;
    sorted.sort();
    let _ = std::fs::write(&ips_path, sorted.join("\n"));

    Ok(())
}

/// Check if a Clay-generated cert needs regeneration due to IP changes.
/// Returns true if the cert should be regenerated.
fn cert_needs_regeneration(cert_path: &std::path::Path) -> bool {
    let ips_path = cert_path.with_extension("ips");

    // No IPs file means not a Clay-generated cert — don't touch it
    if !ips_path.exists() {
        return false;
    }

    // Read saved IPs
    let saved = match std::fs::read_to_string(&ips_path) {
        Ok(s) => s,
        Err(_) => return true, // Can't read — regenerate
    };
    let mut saved_ips: Vec<String> = saved.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    saved_ips.sort();

    // Build current IPs list (same logic as generate_self_signed_cert)
    let mut current_ips = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ];
    if let Ok(hostname) = std::process::Command::new("hostname").output() {
        let name = String::from_utf8_lossy(&hostname.stdout).trim().to_string();
        if !name.is_empty() && !current_ips.contains(&name) {
            current_ips.push(name);
        }
    }
    for ip in get_local_ip_addresses() {
        if !current_ips.contains(&ip) {
            current_ips.push(ip);
        }
    }
    current_ips.sort();

    saved_ips != current_ips
}

pub fn get_home_dir() -> String {
    home::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string())
}

/// Returns a dot-prefixed filename on Unix, plain filename on Windows.
/// e.g. clay_filename("clay.dat") → ".clay.dat" on Unix, "clay.dat" on Windows.
pub fn clay_filename(name: &str) -> String {
    #[cfg(unix)]
    { format!(".{}", name) }
    #[cfg(not(unix))]
    { name.to_string() }
}

/// Generate a WAV file from ANSI music notes (square wave, matching web client's oscillator)
fn generate_wav_from_notes(notes: &[crate::ansi_music::MusicNote]) -> Vec<u8> {
    let sample_rate: u32 = 22050;
    let mut samples: Vec<i16> = Vec::new();

    for note in notes {
        let num_samples = (sample_rate as f64 * note.duration_ms as f64 / 1000.0) as usize;
        if note.frequency <= 0.0 {
            // Rest/silence
            samples.extend(std::iter::repeat(0i16).take(num_samples));
        } else {
            let period = sample_rate as f64 / note.frequency as f64;
            for i in 0..num_samples {
                // Square wave: +amplitude for first half of period, -amplitude for second half
                let phase = (i as f64 % period) / period;
                let sample = if phase < 0.5 { 8000i16 } else { -8000i16 };
                samples.push(sample);
            }
        }
    }

    // Build WAV header + data
    let data_size = (samples.len() * 2) as u32;
    let file_size = 36 + data_size;
    let mut wav = Vec::with_capacity(44 + data_size as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    wav.extend_from_slice(&1u16.to_le_bytes());  // PCM format
    wav.extend_from_slice(&1u16.to_le_bytes());  // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes());  // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for s in &samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

/// Generate the Super Mario Bros theme (~10 seconds) for /testmusic
fn generate_test_music_notes() -> Vec<crate::ansi_music::MusicNote> {
    use crate::ansi_music::MusicNote;
    // Note frequencies (A4=440Hz standard tuning)
    const E5: f32 = 659.25; const D5: f32 = 587.33; const C5: f32 = 523.25;
    const B4: f32 = 493.88; const BB4: f32 = 466.16; const A4: f32 = 440.00;
    const G4: f32 = 392.00; const FS4: f32 = 369.99; const F4: f32 = 349.23;
    const E4: f32 = 329.63;
    const F5: f32 = 698.46; const G5: f32 = 783.99; const A5: f32 = 880.00;
    const REST: f32 = 0.0;
    // Tempo ~200 BPM: eighth=150ms, quarter=300ms, dotted-quarter=450ms
    let n = |freq: f32, ms: u32| MusicNote { frequency: freq, duration_ms: ms };
    vec![
        // Bar 1
        n(E5,150), n(E5,150), n(REST,150), n(E5,150), n(REST,150), n(C5,150), n(E5,300),
        // Bar 2
        n(G5,300), n(REST,300), n(G4,300), n(REST,300),
        // Bar 3
        n(C5,450), n(G4,150), n(REST,300), n(E4,300),
        // Bar 4
        n(REST,150), n(A4,300), n(B4,300), n(BB4,150), n(A4,300),
        // Bar 5 (triplet feel on first 3 notes)
        n(G4,200), n(E5,200), n(G5,200), n(A5,300), n(F5,150), n(G5,150),
        // Bar 6
        n(REST,150), n(E5,300), n(C5,150), n(D5,150), n(B4,300),
        // Bar 7 (second phrase)
        n(C5,450), n(G4,150), n(REST,300), n(E4,300),
        // Bar 8
        n(REST,150), n(A4,300), n(B4,300), n(BB4,150), n(A4,300),
        // Bar 9
        n(G4,200), n(E5,200), n(G5,200), n(A5,300), n(F5,150), n(G5,150),
        // Bar 10
        n(REST,150), n(E5,300), n(C5,150), n(D5,150), n(B4,300),
        // Bar 11 (descending run ending)
        n(REST,300), n(G5,150), n(FS4,150), n(F4,150), n(D5,300), n(E5,300),
        // Bar 12
        n(REST,150), n(G4,150), n(A4,150), n(C5,300), n(REST,150), n(A4,150), n(C5,150), n(D5,150),
    ]
}


/// An authentication key with creation timestamp
#[derive(Clone, Debug)]
pub struct AuthKey {
    pub key: String,
    pub created_at: u64,  // Unix timestamp
}

impl AuthKey {
    fn new(key: String) -> Self {
        Self {
            key,
            created_at: std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

#[derive(Clone)]
pub struct Settings {
    pub more_mode_enabled: bool,
    spell_check_enabled: bool,
    temp_convert_enabled: bool,  // Temperature conversion (e.g., 32F -> 32F (0C))
    world_switch_mode: WorldSwitchMode,
    debug_enabled: bool,    // Debug logging to clay.debug.log
    ansi_music_enabled: bool, // Enable ANSI music playback (web/GUI only)
    theme: Theme,           // Console theme
    gui_theme: Theme,       // GUI theme (separate from console)
    gui_transparency: f32,  // GUI window transparency (0.0-1.0)
    // Color contrast adjustment for web/GUI (0 = disabled, 1-100 = adjustment percentage)
    color_offset_percent: u8,
    // Remote GUI font settings
    font_name: String,
    font_size: f32,
    // Web interface font sizes (separate settings for phone/tablet/desktop)
    web_font_size_phone: f32,
    web_font_size_tablet: f32,
    web_font_size_desktop: f32,
    web_font_weight: u16,
    web_font_line_height: f32,     // Line height multiplier (default 1.2)
    web_font_letter_spacing: f32,  // Letter spacing in px (default 0)
    web_font_word_spacing: f32,    // Word spacing in px (default 0)
    // Web server settings (consolidated)
    web_secure: bool,              // Protocol: true=Secure (https/wss), false=Non-Secure (http/ws)
    http_enabled: bool,            // Enable HTTP/HTTPS web server (name depends on web_secure)
    http_port: u16,                // Port for HTTP/HTTPS web interface
    websocket_password: String,
    websocket_allow_list: String,  // CSV list of hosts that can be whitelisted
    websocket_whitelisted_host: Option<String>,  // Currently whitelisted host (authenticated from allow list)
    websocket_cert_file: String,   // Path to TLS certificate file (PEM) - only used when web_secure=true
    websocket_key_file: String,    // Path to TLS private key file (PEM) - only used when web_secure=true
    // Single persistent auth key for passwordless device authentication
    websocket_auth_key: Option<AuthKey>,
    // User-defined actions/triggers
    actions: Vec<Action>,
    // TLS proxy for connection preservation over hot reload
    tls_proxy_enabled: bool,
    // Custom dictionary path for spell checking (empty = use system defaults)
    dictionary_path: String,
    // Editor side for split-screen editor (left or right)
    editor_side: EditorSide,
    // Mouse click support for console popups
    mouse_enabled: bool,
    // ZWJ emoji sequence handling (true = pass through, false = strip ZWJ + colored square)
    zwj_enabled: bool,
    // New line indicator - prefix unseen/pending lines with green "+"
    pub new_line_indicator: bool,
    /// Text-to-speech mode: Off, Local (espeak/say), or Edge (MS neural TTS)
    pub tts_mode: tts::TtsMode,
    pub tts_speak_mode: tts::TtsSpeakMode,
    pub tts_muted: bool,  // Runtime-only, toggled by F9
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            more_mode_enabled: true,
            spell_check_enabled: true,
            temp_convert_enabled: false,  // Disabled by default
            world_switch_mode: WorldSwitchMode::UnseenFirst,
            debug_enabled: false,
            ansi_music_enabled: true,  // ANSI music enabled by default
            theme: Theme::Dark,
            gui_theme: Theme::Dark,
            gui_transparency: 1.0,
            color_offset_percent: 0,   // 0 = disabled, 1-100 = adjustment percentage
            font_name: String::new(),  // Empty means use system default
            font_size: 14.0,
            // Web interface font sizes (per device type)
            web_font_size_phone: 10.0,   // Default for phones
            web_font_size_tablet: 14.0,  // Default for tablets
            web_font_size_desktop: 18.0, // Default for desktop
            web_font_weight: 400,      // Default font weight (normal)
            web_font_line_height: 1.2,
            web_font_letter_spacing: 0.0,
            web_font_word_spacing: 0.0,
            web_secure: false,         // Default to non-secure
            http_enabled: false,
            http_port: 9000,
            websocket_password: String::new(),
            websocket_allow_list: String::new(),
            websocket_whitelisted_host: None,
            websocket_cert_file: String::new(),
            websocket_key_file: String::new(),
            websocket_auth_key: None,
            actions: Vec::new(),
            tls_proxy_enabled: false,
            dictionary_path: String::new(),
            editor_side: EditorSide::Left,
            mouse_enabled: true,
            zwj_enabled: false,
            new_line_indicator: false,
            tts_mode: tts::TtsMode::Off,
            tts_speak_mode: tts::TtsSpeakMode::All,
            tts_muted: false,
        }
    }
}

/// Type of world connection
#[derive(Clone, Debug, PartialEq, Default)]
pub enum WorldType {
    #[default]
    Mud,
    Slack,
    Discord,
}

impl WorldType {
    fn name(&self) -> &'static str {
        match self {
            WorldType::Mud => "mud",
            WorldType::Slack => "slack",
            WorldType::Discord => "discord",
        }
    }

    fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "slack" => WorldType::Slack,
            "discord" => WorldType::Discord,
            _ => WorldType::Mud,
        }
    }

}

#[derive(Clone)]
pub struct WorldSettings {
    pub world_type: WorldType,
    // MUD settings
    pub hostname: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub use_ssl: bool,
    pub log_enabled: bool,
    pub encoding: Encoding,
    pub auto_connect_type: AutoConnectType,
    pub keep_alive_type: KeepAliveType,
    pub keep_alive_cmd: String,
    // Slack settings
    slack_token: String,
    slack_channel: String,
    slack_workspace: String,
    // Discord settings
    discord_token: String,
    discord_guild: String,
    discord_channel: String,
    discord_dm_user: String, // User ID for DM (creates DM channel on connect)
    // User notes (stored per-world, edited with /edit command)
    pub notes: String,
    // GMCP packages to request (comma-separated, e.g. "Client.Media 1, Char.Vitals 1")
    pub gmcp_packages: String,
    // Auto-reconnect delay in seconds (0 = disabled)
    pub auto_reconnect_secs: u32,
    // Auto-reconnect when a web/Android client connects
    pub auto_reconnect_on_web: bool,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            world_type: WorldType::Mud,
            hostname: String::new(),
            port: String::new(),
            user: String::new(),
            password: String::new(),
            use_ssl: false,
            log_enabled: false,
            encoding: Encoding::Utf8,
            auto_connect_type: AutoConnectType::Connect,
            keep_alive_type: KeepAliveType::Nop,
            keep_alive_cmd: String::new(),
            slack_token: String::new(),
            slack_channel: String::new(),
            slack_workspace: String::new(),
            discord_token: String::new(),
            discord_guild: String::new(),
            discord_channel: String::new(),
            discord_dm_user: String::new(),
            notes: String::new(),
            gmcp_packages: "Client.Media 1".to_string(),
            auto_reconnect_secs: 0,
            auto_reconnect_on_web: false,
        }
    }
}

impl WorldSettings {
    /// Check if this world has enough settings to attempt a connection
    fn has_connection_settings(&self) -> bool {
        match self.world_type {
            WorldType::Mud => !self.hostname.is_empty() && !self.port.is_empty(),
            WorldType::Slack => !self.slack_token.is_empty(),
            WorldType::Discord => !self.discord_token.is_empty(),
        }
    }

    /// Parse an auto-reconnect string like "30", "web", "web,30", "30,web" into (secs, on_web).
    /// Spaces are stripped. Unrecognized tokens are ignored.
    fn parse_auto_reconnect(s: &str) -> (u32, bool) {
        let mut secs = 0u32;
        let mut on_web = false;
        for part in s.split(',') {
            let part = part.trim();
            if part.eq_ignore_ascii_case("web") {
                on_web = true;
            } else if let Ok(n) = part.parse::<u32>() {
                secs = n;
            }
        }
        (secs, on_web)
    }

    /// Reconstruct the display string from the two internal fields.
    fn auto_reconnect_display(&self) -> String {
        match (self.auto_reconnect_on_web, self.auto_reconnect_secs) {
            (true, n) if n > 0 => format!("web,{}", n),
            (true, _) => "web".to_string(),
            (false, n) if n > 0 => n.to_string(),
            _ => "0".to_string(),
        }
    }
}


/// User account for multiuser mode
#[derive(Clone, Debug)]
pub struct User {
    pub name: String,
    pub password: String,  // Stored encrypted in file, decrypted in memory
}

impl User {
    fn new(name: &str, password: &str) -> Self {
        Self {
            name: name.to_string(),
            password: password.to_string(),
        }
    }
}

/// Tracks a WebSocket client's view state for synchronized more-mode
#[derive(Clone, Debug)]
pub struct ClientViewState {
    /// Which world the client is viewing
    pub world_index: usize,
    /// Number of visible output lines in the client's display
    pub visible_lines: usize,
    /// Number of visible columns in the client's output area (0 = not reported)
    pub visible_columns: usize,
    /// Client's output area dimensions (width, height) for NAWS
    pub dimensions: Option<(u16, u16)>,
}

// ============================================================================
// Shared Command Parsing
// ============================================================================

/// Parsed command representation - shared across console, GUI, and web interfaces
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::enum_variant_names)]
pub enum Command {
    /// /help [topic] - show help popup (or topic-specific help)
    Help,
    /// /help <topic> - show help for a specific topic
    HelpTopic { topic: String },
    /// /version - show version info
    Version,
    /// /quit - exit application
    Quit,
    /// /reload - hot reload binary
    Reload,
    /// /update - check for and install updates from GitHub
    Update { force: bool },
    /// /setup - show global settings popup
    Setup,
    /// /web - show web settings popup
    Web,
    /// /actions [world] - show actions popup, optionally filtered by world
    Actions { world: Option<String> },
    /// /connections or /l - show connected worlds list
    WorldsList,
    /// /worlds (no args) - show world selector
    WorldSelector,
    /// /worlds -e [name] - edit world settings
    WorldEdit { name: Option<String> },
    /// /worlds -l <name> - connect without auto-login
    WorldConnectNoLogin { name: String },
    /// /worlds <name> - switch to or connect to named world
    WorldSwitch { name: String },
    /// /connect [host port [ssl]] - connect to server (internal use by buttons/TF)
    Connect { host: Option<String>, port: Option<String>, ssl: bool },
    /// /disconnect or /dc - disconnect current world
    Disconnect,
    /// /flush - clear output buffer for current world
    Flush,
    /// /menu - show menu popup to select windows/popups
    Menu,
    /// /font - show font settings popup (web/GUI only)
    Font,
    /// /send [-W] [-w<world>] [-n] <text> - send text
    Send { text: String, all_worlds: bool, target_world: Option<String>, no_newline: bool },
    /// /remote - list remotely connected clients
    Remote,
    /// /remote --kill <id> - disconnect a remote client
    RemoteKill { client_id: u64 },
    /// /ban - show banned hosts
    BanList,
    /// /unban <host> - remove ban for host
    Unban { host: String },
    /// /testmusic - play a test ANSI music sequence
    TestMusic,
    /// /dump - dump all scrollback buffers to ~/.clay.dmp.log
    Dump,
    /// /notify <message> - send notification to mobile clients
    Notify { message: String },
    /// /addworld - add or update a world definition
    AddWorld {
        name: String,
        host: Option<String>,
        port: Option<String>,
        user: Option<String>,
        password: Option<String>,
        use_ssl: bool,
    },
    /// /edit [filename] - open split-screen editor for world notes or file
    Edit { filename: Option<String> },
    /// /edit -l - open notes list popup
    EditList,
    /// /tag - toggle MUD tag display (same as F2)
    Tag,
    /// /dict <word> - look up word definition
    Dict { word: String },
    /// /dict usage error
    DictUsage,
    /// /urban <word> - look up Urban Dictionary definition
    Urban { word: String },
    /// /urban usage error
    UrbanUsage,
    /// /translate <lang> <text> - translate text
    Translate { lang: String, text: String },
    /// /translate usage error
    TranslateUsage,
    /// /url <url> - shorten URL via is.gd
    TinyUrl { url: String },
    /// /url usage error
    TinyUrlUsage,
    /// /say <text> - speak text aloud via TTS
    Say { text: String },
    /// /window [world] - open a new GUI/web window, optionally locked to a world
    Window { world: Option<String> },
    /// /<action_name> [args] - execute action
    ActionCommand { name: String, args: String },
    /// Not a command (regular text to send to MUD)
    NotACommand { text: String },
    /// Unknown/invalid command
    Unknown { cmd: String },
}

/// Parse a command string into a Command enum
pub fn parse_command(input: &str) -> Command {
    let trimmed = input.trim();

    // Not a command if doesn't start with /
    if !trimmed.starts_with('/') {
        return Command::NotACommand { text: trimmed.to_string() };
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return Command::NotACommand { text: trimmed.to_string() };
    }

    let cmd = parts[0].to_lowercase();
    let args = &parts[1..];

    match cmd.as_str() {
        "/help" => {
            if !args.is_empty() {
                Command::HelpTopic { topic: args[0].to_lowercase() }
            } else {
                Command::Help
            }
        }
        "/version" => Command::Version,
        "/quit" => Command::Quit,
        "/reload" => Command::Reload,
        "/update" => {
            let force = args.first().map(|a| *a == "-f" || *a == "--force").unwrap_or(false);
            Command::Update { force }
        }
        "/setup" => Command::Setup,
        "/web" => Command::Web,
        "/actions" => {
            let world = if args.is_empty() {
                None
            } else {
                Some(args.join(" "))
            };
            Command::Actions { world }
        }
        "/connections" | "/l" => Command::WorldsList,
        "/worlds" | "/world" => parse_world_command(args),
        "/__connect" => parse_connect_command(args),  // Internal use only (Connect buttons)
        "/disconnect" | "/dc" => Command::Disconnect,
        "/flush" => Command::Flush,
        "/menu" => Command::Menu,
        "/font" => Command::Font,
        "/send" => parse_send_command(args, trimmed),
        "/remote" => {
            if args.len() >= 2 && args[0] == "--kill" {
                if let Ok(id) = args[1].parse::<u64>() {
                    Command::RemoteKill { client_id: id }
                } else {
                    Command::Unknown { cmd: trimmed.to_string() }
                }
            } else {
                Command::Remote
            }
        }
        "/ban" => Command::BanList,
        "/unban" => {
            if args.is_empty() {
                Command::Unknown { cmd: trimmed.to_string() }
            } else {
                Command::Unban { host: args[0].to_string() }
            }
        }
        "/testmusic" => Command::TestMusic,
        "/dump" => Command::Dump,
        "/notify" => {
            if args.is_empty() {
                Command::Unknown { cmd: trimmed.to_string() }
            } else {
                Command::Notify { message: args.join(" ") }
            }
        }
        "/addworld" => parse_addworld_command(args),
        "/note" => {
            if args.first() == Some(&"-l") {
                Command::EditList
            } else if args.is_empty() {
                Command::Edit { filename: None }
            } else {
                Command::Edit { filename: Some(args.join(" ")) }
            }
        }
        "/tag" | "/tags" => Command::Tag,
        "/dict" => {
            if !args.is_empty() {
                Command::Dict { word: args.join(" ") }
            } else {
                Command::DictUsage
            }
        }
        "/urban" => {
            if !args.is_empty() {
                Command::Urban { word: args.join(" ") }
            } else {
                Command::UrbanUsage
            }
        }
        "/translate" | "/tr" => {
            if args.len() >= 2 {
                Command::Translate {
                    lang: args[0].to_string(),
                    text: args[1..].join(" "),
                }
            } else {
                Command::TranslateUsage
            }
        }
        "/url" => {
            if !args.is_empty() {
                Command::TinyUrl { url: args[0].to_string() }
            } else {
                Command::TinyUrlUsage
            }
        }
        "/say" => {
            if !args.is_empty() {
                // Preserve the original text after /say (not split by whitespace)
                let text = trimmed.split_once(char::is_whitespace).map(|x| x.1).unwrap_or("").trim();
                Command::Say { text: text.to_string() }
            } else {
                Command::Unknown { cmd: trimmed.to_string() }
            }
        }
        "/window" => {
            if args.is_empty() {
                Command::Window { world: None }
            } else {
                Command::Window { world: Some(args.join(" ")) }
            }
        }
        _ => {
            // Check if it's an action command (starts with / but not a known command)
            let action_name = cmd.trim_start_matches('/');
            if !action_name.is_empty() {
                Command::ActionCommand {
                    name: action_name.to_string(),
                    args: args.join(" "),
                }
            } else {
                Command::Unknown { cmd: trimmed.to_string() }
            }
        }
    }
}

/// Parse /worlds command with its various forms
fn parse_world_command(args: &[&str]) -> Command {
    if args.is_empty() {
        return Command::WorldSelector;
    }

    match args[0] {
        "-e" => {
            // /worlds -e [name] - edit world
            let name = if args.len() > 1 {
                Some(args[1..].join(" "))
            } else {
                None
            };
            Command::WorldEdit { name }
        }
        "-l" => {
            // /worlds -l <name> - connect without auto-login
            if args.len() > 1 {
                Command::WorldConnectNoLogin { name: args[1..].join(" ") }
            } else {
                Command::Unknown { cmd: "/worlds -l".to_string() }
            }
        }
        _ => {
            // /worlds <name> - switch to or connect to named world
            Command::WorldSwitch { name: args.join(" ") }
        }
    }
}

/// Parse /connect command
fn parse_connect_command(args: &[&str]) -> Command {
    if args.is_empty() {
        return Command::Connect { host: None, port: None, ssl: false };
    }

    let host = Some(args[0].to_string());
    let port = args.get(1).map(|s| s.to_string());
    let ssl = args.get(2).map(|s| s.to_lowercase() == "ssl").unwrap_or(false);

    Command::Connect { host, port, ssl }
}

/// Parse /addworld command (TF-compatible world creation)
///
/// Formats:
///   /addworld [-xe] [-Ttype] name [char pass] host port
///   /addworld [-Ttype] name
///
/// Options:
///   -x  Use SSL/TLS
///   -e  Echo (ignored)
///   -Ttype  World type (ignored, defaults to MUD)
///   -p  No proxy (ignored)
fn parse_addworld_command(args: &[&str]) -> Command {
    if args.is_empty() {
        return Command::Unknown { cmd: "/addworld".to_string() };
    }

    let mut use_ssl = false;
    let mut remaining_args: Vec<&str> = Vec::new();

    // Parse options
    for arg in args {
        if let Some(flags) = arg.strip_prefix('-') {
            // Parse option flags
            for c in flags.chars() {
                match c {
                    'x' => use_ssl = true,
                    'e' | 'p' => {} // Ignored: echo, proxy
                    'T' => break,   // -Ttype: skip the rest of this arg
                    _ => {}
                }
            }
        } else {
            remaining_args.push(arg);
        }
    }

    // Remaining args interpretation:
    // 1 arg:  name (connectionless world)
    // 3 args: name host port
    // 5 args: name char pass host port

    match remaining_args.len() {
        0 => Command::Unknown { cmd: "/addworld".to_string() },
        1 => {
            // Just name - connectionless world
            Command::AddWorld {
                name: remaining_args[0].to_string(),
                host: None,
                port: None,
                user: None,
                password: None,
                use_ssl,
            }
        }
        2 => {
            // name host (assume default port or error)
            Command::AddWorld {
                name: remaining_args[0].to_string(),
                host: Some(remaining_args[1].to_string()),
                port: None,
                user: None,
                password: None,
                use_ssl,
            }
        }
        3 => {
            // name host port
            Command::AddWorld {
                name: remaining_args[0].to_string(),
                host: Some(remaining_args[1].to_string()),
                port: Some(remaining_args[2].to_string()),
                user: None,
                password: None,
                use_ssl,
            }
        }
        4 => {
            // Could be: name char host port (missing pass) or name char pass host (missing port)
            // Assume: name char host port (user without password)
            Command::AddWorld {
                name: remaining_args[0].to_string(),
                host: Some(remaining_args[2].to_string()),
                port: Some(remaining_args[3].to_string()),
                user: Some(remaining_args[1].to_string()),
                password: None,
                use_ssl,
            }
        }
        _ => {
            // 5+ args: name char pass host port [file]
            Command::AddWorld {
                name: remaining_args[0].to_string(),
                host: Some(remaining_args[3].to_string()),
                port: Some(remaining_args[4].to_string()),
                user: Some(remaining_args[1].to_string()),
                password: Some(remaining_args[2].to_string()),
                use_ssl,
            }
        }
    }
}

/// Parse /send command with flags
fn parse_send_command(args: &[&str], full_cmd: &str) -> Command {
    let mut all_worlds = false;
    let mut target_world: Option<String> = None;
    let mut no_newline = false;
    let mut text_start = 0;

    for (i, arg) in args.iter().enumerate() {
        if *arg == "-W" {
            all_worlds = true;
            text_start = i + 1;
        } else if let Some(world) = arg.strip_prefix("-w") {
            target_world = Some(world.to_string());
            text_start = i + 1;
        } else if *arg == "-n" {
            no_newline = true;
            text_start = i + 1;
        } else {
            break;
        }
    }

    // Get the text after flags - use original string to preserve spacing
    let text = if text_start < args.len() {
        // Find position of first non-flag argument in original string
        let mut pos = 0;
        let mut found_flags = 0;
        for c in full_cmd.chars() {
            if found_flags > text_start { // +1 for /send itself
                break;
            }
            if c.is_whitespace() && pos > 0 {
                // Check if we just finished a word
                let prev_nonws = full_cmd[..pos].chars().rev().find(|c| !c.is_whitespace());
                if prev_nonws.is_some() {
                    found_flags += 1;
                }
            }
            pos += c.len_utf8();
        }
        // Skip whitespace after last flag
        while pos < full_cmd.len() && full_cmd[pos..].starts_with(char::is_whitespace) {
            pos += 1;
        }
        full_cmd[pos..].to_string()
    } else {
        String::new()
    };

    Command::Send { text, all_worlds, target_world, no_newline }
}

pub(crate) fn get_reload_state_path() -> PathBuf {
    let home = get_home_dir();
    // Use PID from env (set by old process during reload) or current PID
    let pid = std::env::var("CLAY_RELOAD_PID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(std::process::id());
    PathBuf::from(home).join(clay_filename(&format!("clay.reload.{}", pid)))
}

/// Get current time as seconds since Unix epoch (for WebSocket timestamps)
pub fn current_timestamp_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Format a Duration for display (short format)
pub fn format_duration_short(d: std::time::Duration) -> String {
    let total_secs = d.as_secs_f64();
    if total_secs < 60.0 {
        if total_secs == total_secs.floor() {
            format!("{}s", total_secs as u64)
        } else {
            format!("{:.1}s", total_secs)
        }
    } else if total_secs < 3600.0 {
        let mins = (total_secs / 60.0) as u64;
        let secs = (total_secs % 60.0) as u64;
        format!("{}m{}s", mins, secs)
    } else {
        let hours = (total_secs / 3600.0) as u64;
        let mins = ((total_secs % 3600.0) / 60.0) as u64;
        format!("{}h{}m", hours, mins)
    }
}

/// Output line with timestamp for F2/show tags feature
#[derive(Clone)]
pub struct OutputLine {
    pub text: String,
    pub timestamp: SystemTime,
    pub from_server: bool,  // true if from MUD server, false if client-generated
    pub gagged: bool,       // true if line was gagged by an action (only shown with F2)
    pub seq: u64,           // Unique sequential number within the world (for debugging out-of-order issues)
    pub highlight_color: Option<String>, // Optional highlight color from /highlight action command
    pub marked_new: bool,   // true if line arrived while user wasn't viewing (unseen/pending)
}

/// Maximum characters per output line (prevents performance issues with extremely long lines)
const MAX_LINE_LENGTH: usize = 10_000;

impl OutputLine {
    /// Truncate text if it exceeds MAX_LINE_LENGTH to prevent performance issues
    fn truncate_if_needed(text: String) -> String {
        if text.len() > MAX_LINE_LENGTH {
            // Find a safe truncation point (don't split UTF-8 or ANSI sequences)
            let mut truncate_at = MAX_LINE_LENGTH;
            // Walk back to find a safe character boundary
            while truncate_at > 0 && !text.is_char_boundary(truncate_at) {
                truncate_at -= 1;
            }
            // Also avoid truncating in the middle of an ANSI escape sequence
            // Look for incomplete escape: \x1b[ without terminating letter
            let prefix = &text[..truncate_at];
            if let Some(last_esc) = prefix.rfind('\x1b') {
                // Check if there's a terminating letter after the escape
                let after_esc = &prefix[last_esc..];
                let has_terminator = after_esc.chars().skip(1).any(|c| c.is_ascii_alphabetic());
                if !has_terminator {
                    // Truncate before the incomplete escape sequence
                    truncate_at = last_esc;
                }
            }
            let mut result = text[..truncate_at].to_string();
            result.push_str("\x1b[0m\x1b[33m... [truncated]\x1b[0m");
            result
        } else {
            text
        }
    }

    pub fn new(text: String, seq: u64) -> Self {
        Self {
            text: Self::truncate_if_needed(text),
            timestamp: SystemTime::now(),
            from_server: true,  // Default to server output
            gagged: false,
            seq,
            highlight_color: None,
            marked_new: false,
        }
    }

    pub fn new_client(text: String, seq: u64) -> Self {
        Self {
            text: Self::truncate_if_needed(text),
            timestamp: SystemTime::now(),
            from_server: false,
            gagged: false,
            seq,
            highlight_color: None,
            marked_new: false,
        }
    }

    fn new_gagged(text: String, seq: u64) -> Self {
        Self {
            text: Self::truncate_if_needed(text),
            timestamp: SystemTime::now(),
            from_server: true,
            gagged: true,
            seq,
            highlight_color: None,
            marked_new: false,
        }
    }

    fn new_with_timestamp(text: String, timestamp: SystemTime, seq: u64) -> Self {
        Self { text: Self::truncate_if_needed(text), timestamp, from_server: true, gagged: false, seq, highlight_color: None, marked_new: false }
    }

    /// Format timestamp using a pre-computed "now" value for batch rendering
    fn format_timestamp_with_now(&self, _now: &CachedNow) -> String {
        let ts_secs = self.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        let lt = local_time_from_epoch(ts_secs);

        // Always show month/day with time
        format!("{:02}/{:02} {:02}:{:02}>", lt.month, lt.day, lt.hour, lt.minute)
    }
}

/// Cached "now" time for batch timestamp formatting
/// Computing localtime_r once per frame instead of once per line
pub(crate) struct CachedNow;

impl CachedNow {
    pub(crate) fn new() -> Self {
        Self
    }
}

pub struct World {
    pub name: String,
    pub output_lines: Vec<OutputLine>,
    pub scroll_offset: usize,
    pub connected: bool,
    pub command_tx: Option<mpsc::Sender<WriteCommand>>,
    pub unseen_lines: usize,
    pub paused: bool,
    pub pending_lines: Vec<OutputLine>,
    pub pending_count: usize, // For remote client mode: daemon's pending line count (not in pending_lines)
    pub lines_since_pause: usize,
    pub settings: WorldSettings,
    log_handle: Option<std::sync::Arc<std::sync::Mutex<std::fs::File>>>,
    log_date: Option<String>,    // Current log file date (MMDDYY) for day rollover detection
    #[cfg(unix)]
    socket_fd: Option<RawFd>,    // Store fd for hot reload (plain TCP only)
    #[cfg(not(unix))]
    socket_fd: Option<i64>,      // Socket handle on Windows, placeholder on other platforms
    #[cfg(unix)]
    proxy_socket_fd: Option<RawFd>, // Store Unix socket fd to TLS proxy for hot reload
    #[cfg(not(unix))]
    proxy_socket_fd: Option<i64>,   // Placeholder on non-Unix (never used)
    is_tls: bool,                // Track if using TLS
    telnet_mode: bool,           // True if telnet negotiation detected
    pub negotiated_encoding: Option<Encoding>, // Encoding negotiated via TELNET CHARSET (RFC 2066)
    pub prompt: String,              // Current prompt detected via telnet GA
    pub prompt_count: usize,         // Number of prompts received since connect (for auto-login)
    last_send_time: Option<std::time::Instant>, // For keepalive timing
    last_receive_time: Option<std::time::Instant>, // Last time server data was received
    last_nop_time: Option<std::time::Instant>,     // Last time NOP keepalive was sent
    last_user_command_time: Option<std::time::Instant>, // Last time user sent a command
    pub partial_line: String,        // Buffer for incomplete lines (no trailing newline)
    pub partial_in_pending: bool,    // True if partial_line is in pending_lines (vs output_lines)
    trigger_partial_line: String, // Buffer for incomplete lines for action trigger checking
    just_filtered_idler: bool,   // True if we just filtered an idler message (for filtering trailing newline)
    wont_echo_time: Option<std::time::Instant>, // When WONT ECHO was seen (for timeout-based prompt detection)
    uses_wont_echo_prompt: bool, // True if this world uses WONT ECHO for prompts (auto-detected)
    pub is_initial_world: bool,      // True for the auto-created world before first connection
    pub was_connected: bool,         // True if world has ever been connected (for world cycling)
    pub skip_auto_login: bool,       // True to skip auto-login on next connect (for /worlds -l)
    pub showing_splash: bool,        // True when showing startup splash (for centering)
    needs_redraw: bool,          // True when terminal needs full redraw (after splash clear)
    pending_since: Option<std::time::Instant>, // When pending output first appeared (for Alt-w)
    pub first_unseen_at: Option<std::time::Instant>, // When unseen output first arrived (for Unseen First switching)
    last_pending_broadcast: Option<std::time::Instant>, // Last time pending count was broadcast (for 2s timer)
    last_pending_count_broadcast: usize, // Last pending count that was broadcast (to detect changes)
    owner: Option<String>,       // Username who owns this world (multiuser mode)
    proxy_pid: Option<u32>,      // PID of TLS proxy process (if using TLS proxy)
    proxy_socket_path: Option<std::path::PathBuf>, // Unix socket path for TLS proxy
    naws_enabled: bool,          // True if NAWS telnet option was negotiated
    naws_sent_size: Option<(u16, u16)>, // Last sent window size (width, height) to avoid duplicates
    pub next_seq: u64,               // Next sequence number for output lines (for debugging)
    reader_name: Option<String>, // Name used by active reader task (for lookup after rename)
    pub gmcp_enabled: bool,          // True if GMCP was negotiated with the server
    pub msdp_enabled: bool,          // True if MSDP was negotiated with the server
    pub gmcp_supported_packages: Vec<String>, // GMCP packages we told the server we support
    pub msdp_variables: std::collections::HashMap<String, String>, // MSDP var -> JSON value
    pub gmcp_data: std::collections::HashMap<String, String>, // GMCP package -> last JSON data
    pub mcmp_default_url: String,    // MCMP default URL from Client.Media.Default
    pub active_media: std::collections::HashMap<String, String>, // key -> Client.Media.Play JSON (for restart on world switch)
    pub gmcp_user_enabled: bool,     // True if user has enabled GMCP processing (F9 toggle)
    pub tts_speaker_whitelist: std::collections::HashSet<String>, // Per-world TTS speaker whitelist
    pub connection_id: u64,          // Incremented on each connection, used to ignore stale disconnect events
    pub max_received_seq: u64,       // Highest seq received from server (remote console dedup)
    pub first_marked_new_index: Option<usize>, // Index of first marked_new line in output_lines (for fast clear)
    pub visual_line_offset: usize, // When > 0, show only first N visual lines of scroll_offset line (partial display for more-mode)
    pub watchdog_history: std::collections::VecDeque<String>,  // Rolling window of recent lines (stripped) for /watchdog
    pub watchname_history: std::collections::VecDeque<String>, // Rolling window of first-words for /watchname
    fansi_detect_until: Option<std::time::Instant>,  // FANSI client detection window (2s after connect)
    fansi_login_pending: Option<String>,             // Deferred login command for FANSI worlds
    pub reconnect_at: Option<std::time::Instant>,   // When to auto-reconnect (None = no reconnect scheduled)
}

impl World {
    pub fn new(name: &str) -> Self {
        Self::new_with_splash(name, false)
    }

    pub fn new_with_splash(name: &str, show_splash: bool) -> Self {
        let output_lines = if show_splash {
            Self::generate_splash_lines()
        } else {
            Vec::new()
        };
        let scroll_offset = output_lines.len().saturating_sub(1);
        Self {
            name: name.to_string(),
            output_lines,
            scroll_offset,
            connected: false,
            command_tx: None,
            unseen_lines: 0,
            paused: false,
            pending_lines: Vec::new(),
            pending_count: 0,
            lines_since_pause: 0,
            settings: WorldSettings::default(),
            log_handle: None,
            log_date: None,
            socket_fd: None,
            proxy_socket_fd: None,
            is_tls: false,
            telnet_mode: false,
            negotiated_encoding: None,
            prompt: String::new(),
            prompt_count: 0,
            last_send_time: None,
            last_receive_time: None,
            last_nop_time: None,
            last_user_command_time: None,
            partial_line: String::new(),
            partial_in_pending: false,
            trigger_partial_line: String::new(),
            just_filtered_idler: false,
            wont_echo_time: None,
            uses_wont_echo_prompt: false,
            is_initial_world: false,
            was_connected: false,
            skip_auto_login: false,
            showing_splash: show_splash,
            needs_redraw: false,
            pending_since: None,
            first_unseen_at: None,
            last_pending_broadcast: None,
            last_pending_count_broadcast: 0,
            owner: None,
            proxy_pid: None,
            proxy_socket_path: None,
            naws_enabled: false,
            naws_sent_size: None,
            next_seq: 0,
            reader_name: None,
            gmcp_enabled: false,
            msdp_enabled: false,
            gmcp_supported_packages: Vec::new(),
            msdp_variables: std::collections::HashMap::new(),
            gmcp_data: std::collections::HashMap::new(),
            mcmp_default_url: String::new(),
            active_media: std::collections::HashMap::new(),
            gmcp_user_enabled: false,
            tts_speaker_whitelist: std::collections::HashSet::new(),
            connection_id: 0,
            max_received_seq: 0,
            first_marked_new_index: None,
            visual_line_offset: 0,
            watchdog_history: std::collections::VecDeque::new(),
            watchname_history: std::collections::VecDeque::new(),
            fansi_detect_until: None,
            fansi_login_pending: None,
            reconnect_at: None,
        }
    }

    /// Get the current date as MMDDYY string for log file naming
    fn get_current_date_string() -> String {
        let lt = local_time_now();
        format!("{:02}{:02}{:02}", lt.month, lt.day, lt.year % 100)
    }

    /// Get the path to the logs directory, creating it if needed
    fn get_logs_dir() -> std::path::PathBuf {
        let home = get_home_dir();
        let logs_dir = std::path::PathBuf::from(home).join(clay_filename("clay")).join("logs");
        if !logs_dir.exists() {
            let _ = std::fs::create_dir_all(&logs_dir);
        }
        logs_dir
    }

    /// Get the full path to this world's log file for the current date
    fn get_log_path(&self) -> std::path::PathBuf {
        let date_str = Self::get_current_date_string();
        // Sanitize world name for use in filename (replace invalid chars with _)
        let safe_name: String = self.name.chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        Self::get_logs_dir().join(format!("{}.{}.log", safe_name, date_str))
    }

    /// Open the log file for this world (creates logs directory if needed)
    fn open_log_file(&mut self) -> bool {
        if !self.settings.log_enabled {
            return false;
        }

        let date_str = Self::get_current_date_string();
        let log_path = self.get_log_path();

        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(file) => {
                self.log_handle = Some(std::sync::Arc::new(std::sync::Mutex::new(file)));
                self.log_date = Some(date_str);
                true
            }
            Err(_) => false,
        }
    }

    /// Close the log file
    fn close_log_file(&mut self) {
        self.log_handle = None;
        self.log_date = None;
    }

    /// Clear connection state when disconnecting
    /// Optionally removes the proxy socket file and clears the prompt
    fn clear_connection_state(&mut self, remove_socket: bool, clear_prompt: bool) {
        if remove_socket {
            if let Some(ref socket_path) = self.proxy_socket_path {
                let _ = std::fs::remove_file(socket_path);
            }
        }
        self.proxy_pid = None;
        self.proxy_socket_path = None;
        self.proxy_socket_fd = None;
        self.command_tx = None;
        self.connected = false;
        self.socket_fd = None;
        self.telnet_mode = false;
        self.negotiated_encoding = None;
        self.naws_enabled = false;
        self.naws_sent_size = None;
        self.reader_name = None;
        // Reset skip_auto_login so next fresh connection triggers auto-login
        self.skip_auto_login = false;
        self.fansi_detect_until = None;
        self.fansi_login_pending = None;
        // Clear timing fields so /connections doesn't show stale times
        self.last_send_time = None;
        self.last_receive_time = None;
        self.last_nop_time = None;
        self.last_user_command_time = None;
        // Clear active media tracking (processes already killed by stop_world_media)
        self.active_media.clear();
        if clear_prompt {
            self.close_log_file();
            self.prompt.clear();
        }
    }

    /// Return the effective encoding for this world: negotiated charset if available, otherwise configured encoding.
    pub fn effective_encoding(&self) -> Encoding {
        self.negotiated_encoding.unwrap_or(self.settings.encoding)
    }

    /// Write a line to the log file with timestamp prefix
    /// Handles day rollover (opens new file if date changed)
    fn write_log_line(&mut self, line: &str) {
        if !self.settings.log_enabled {
            return;
        }

        // Check for day rollover
        let current_date = Self::get_current_date_string();
        if self.log_date.as_ref() != Some(&current_date) {
            // Date changed, close old file and open new one
            self.close_log_file();
            if !self.open_log_file() {
                return;
            }
        }

        if let Some(ref handle) = self.log_handle {
            if let Ok(mut file) = handle.lock() {
                let lt = local_time_now();

                // Format: [HH:MM:SS] line
                let _ = writeln!(file, "[{:02}:{:02}:{:02}] {}",
                    lt.hour, lt.minute, lt.second, line);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_output(
        &mut self,
        text: &str,
        is_current: bool,
        settings: &Settings,
        output_height: u16,
        output_width: u16,
        clear_splash: bool,
        from_server: bool,
    ) {

        // Handle splash mode transitions
        if self.showing_splash && clear_splash {
            // MUD server data: clear splash mode AND clear buffer
            self.showing_splash = false;
            self.needs_redraw = true; // Signal terminal needs full redraw
            self.output_lines.clear();
            self.scroll_offset = 0;
            self.first_marked_new_index = None;
        }
        let max_lines = (output_height as usize).saturating_sub(2);

        // For non-current worlds that aren't paused: recalculate lines_since_pause
        // based on actual visible content at the bottom of output_lines.
        // Without this, lines_since_pause carries over from when the user was viewing
        // the world, causing immediate pause when new output arrives.
        if !is_current && !self.paused && self.lines_since_pause >= max_lines && max_lines > 0 {
            let width = (output_width as usize).max(1);
            let mut visible = 0;
            for line in self.output_lines.iter().rev() {
                let vl = wrap_ansi_line(&line.text, width).len().max(1);
                if visible + vl > max_lines {
                    break;
                }
                visible += vl;
            }
            self.lines_since_pause = visible;
        }

        // If we have a partial line from before, combine it with new text
        let had_partial = !self.partial_line.is_empty();
        let partial_was_in_pending = self.partial_in_pending;
        let combined = if had_partial {
            let mut s = std::mem::take(&mut self.partial_line);
            s.push_str(text);
            self.partial_in_pending = false;
            s
        } else {
            text.to_string()
        };

        // Parse Discord timestamps (e.g., <t:1234567890:f>)
        let combined = parse_discord_timestamps(&combined);

        // Check if text ends with newline (all lines complete) or not (last line is partial)
        let ends_with_newline = combined.ends_with('\n');

        // Collect lines
        let lines: Vec<&str> = combined.lines().collect();
        if lines.is_empty() {
            return;
        }

        // If we had a partial line, update it in the correct list
        let start_idx = if had_partial {
            let completed_line = lines[0];
            // Check if the completed line should be filtered
            let should_filter = (completed_line.contains("###_idler_message_") && completed_line.contains("_###"))
                || is_visually_empty(completed_line);

            if should_filter {
                // Remove the partial line instead of updating it
                if partial_was_in_pending {
                    self.pending_lines.pop();
                } else {
                    self.output_lines.pop();
                    // Invalidate tracked index if we popped the last marked_new line
                    if let Some(idx) = self.first_marked_new_index {
                        if self.output_lines.len() <= idx {
                            self.first_marked_new_index = None;
                        }
                    }
                }
            } else {
                // Update the partial line with combined content
                if partial_was_in_pending {
                    if let Some(last) = self.pending_lines.last_mut() {
                        last.text = completed_line.to_string();
                    }
                } else if let Some(last) = self.output_lines.last_mut() {
                    last.text = completed_line.to_string();
                }
            }

            // If the combined text is still partial (no trailing newline) and there's
            // only one line, we need to preserve the partial tracking. The for loop below
            // won't run (skip(1) with 1 line), so partial_line must be restored here.
            if !ends_with_newline && lines.len() == 1 && !should_filter {
                self.partial_line = completed_line.to_string();
                self.partial_in_pending = partial_was_in_pending;
            }

            1 // Skip first line since we handled it
        } else {
            0
        };

        // Process remaining lines
        let line_count = lines.len();
        for (i, line) in lines.iter().enumerate().skip(start_idx) {
            let is_last = i == line_count - 1;
            let is_partial = is_last && !ends_with_newline;

            // Filter out keep-alive idler message lines (only for Custom/Generic keep-alive types)
            let uses_idler_keepalive = matches!(
                self.settings.keep_alive_type,
                KeepAliveType::Custom | KeepAliveType::Generic
            );
            if uses_idler_keepalive && line.contains("###_idler_message_") && line.contains("_###") {
                continue;
            }

            // Write to log file if enabled (only for complete lines)
            if !is_partial {
                self.write_log_line(line);
            }

            // Use wrap_ansi_line for accurate visual line count — visual_line_count uses
            // character-based division which undercounts when word wrapping pushes words
            // to the next line, causing more-mode to pause too late.
            let visual_lines = wrap_ansi_line(line, output_width as usize).len().max(1);

            // Track if this line goes to pending (for partial tracking)
            let goes_to_pending = self.paused && settings.more_mode_enabled;
            // Use projected line count (current + this line's visual lines) for pause trigger
            let triggers_pause = !goes_to_pending
                && settings.more_mode_enabled
                && (self.lines_since_pause + visual_lines) > max_lines;

            // Create OutputLine with appropriate from_server flag
            let seq = self.next_seq;
            self.next_seq += 1;
            let mut new_line = if from_server {
                OutputLine::new(line.to_string(), seq)
            } else {
                OutputLine::new_client(line.to_string(), seq)
            };

            if goes_to_pending {
                // Track when pending output first appeared
                if self.pending_lines.is_empty() {
                    self.pending_since = Some(std::time::Instant::now());
                    // Also track first unseen timestamp
                    if self.first_unseen_at.is_none() {
                        self.first_unseen_at = Some(std::time::Instant::now());
                    }
                }

                new_line.marked_new = !is_current;
                self.pending_lines.push(new_line);
                if !is_current { self.unseen_lines += 1; }
                if is_partial {
                    self.partial_line = line.to_string();
                    self.partial_in_pending = true;
                }
            } else if triggers_pause {
                // Line fits on screen (single-line or small overflow) — add to output then pause
                new_line.marked_new = !is_current;
                if !is_current && self.first_marked_new_index.is_none() {
                    self.first_marked_new_index = Some(self.output_lines.len());
                }
                self.output_lines.push(new_line);
                self.lines_since_pause += visual_lines;
                if !is_current {
                    if self.unseen_lines == 0 && self.first_unseen_at.is_none() {
                        self.first_unseen_at = Some(std::time::Instant::now());
                    }
                    self.unseen_lines += 1;
                }
                self.scroll_to_bottom();
                // If this line overflows the remaining budget, show only what fits
                let prior_visual = self.lines_since_pause.saturating_sub(visual_lines);
                let remaining_budget = max_lines.saturating_sub(prior_visual);
                if visual_lines > remaining_budget && remaining_budget > 0 {
                    self.visual_line_offset = remaining_budget;
                }
                self.paused = true;
                if is_partial {
                    self.partial_line = line.to_string();
                    self.partial_in_pending = false;
                }
            } else {
                new_line.marked_new = !is_current;
                if !is_current && self.first_marked_new_index.is_none() {
                    self.first_marked_new_index = Some(self.output_lines.len());
                }
                self.output_lines.push(new_line);
                self.lines_since_pause += visual_lines;
                if !is_current {
                    // Track when first unseen output arrived
                    if self.unseen_lines == 0 && self.first_unseen_at.is_none() {
                        self.first_unseen_at = Some(std::time::Instant::now());
                    }
                    self.unseen_lines += 1;
                }
                if is_partial {
                    self.partial_line = line.to_string();
                    self.partial_in_pending = false;
                }
            }
        }
        // If more mode is off, always unpause, release pending, and scroll to bottom
        if !settings.more_mode_enabled {
            self.paused = false;
            // Release any pending lines immediately
            if !self.pending_lines.is_empty() {
                if self.first_marked_new_index.is_none() {
                    // Check if any pending line has marked_new before bulk append
                    if self.pending_lines.iter().any(|l| l.marked_new) {
                        self.first_marked_new_index = Some(self.output_lines.len());
                    }
                }
                self.output_lines.append(&mut self.pending_lines);
            }
        }
        // Always scroll to bottom unless paused (and more mode is on)
        if !self.paused {
            self.scroll_to_bottom();
        }
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.output_lines.len().saturating_sub(1);
        self.visual_line_offset = 0;
    }

    pub fn mark_seen(&mut self) {
        self.unseen_lines = 0;
        self.first_unseen_at = None;
        // Note: marked_new indicators are NOT cleared here. They persist while
        // viewing the world and are only cleared when switching AWAY from it
        // (via clear_new_line_indicators in switch_world).
    }

    /// Clear new line indicator flags on output lines.
    /// Called when switching away from a world or on Ctrl+L.
    /// Only iterates from the first marked_new line to avoid O(n) scan of large buffers.
    pub fn clear_new_line_indicators(&mut self) {
        if let Some(first) = self.first_marked_new_index {
            for line in &mut self.output_lines[first..] {
                line.marked_new = false;
            }
            self.first_marked_new_index = None;
        }
    }

    /// Returns true if this world has activity (unseen lines or pending output)
    fn has_activity(&self) -> bool {
        self.unseen_lines > 0 || !self.pending_lines.is_empty()
    }

    /// Release pending lines, counting by VISUAL lines (wrapped line count) to fill
    /// approximately one screenful. Always releases at least one logical line.
    pub fn release_pending(&mut self, visual_budget: usize, output_width: usize) {
        // Reset visual_line_offset so truncated wrapped lines from the pause trigger
        // are fully visible after releasing.
        self.visual_line_offset = 0;
        if self.pending_lines.is_empty() {
            self.paused = false;
            self.lines_since_pause = 0;
            return;
        }
        let width = output_width.max(1);
        let mut visual_total = 0;
        let mut logical_count = 0;

        for line in &self.pending_lines {
            let vl = visual_line_count(&line.text, width);
            // If adding this line would exceed budget AND we already have at least one,
            // stop before adding it. Always release at least 1 line.
            if visual_total > 0 && visual_total + vl > visual_budget {
                break;
            }
            visual_total += vl;
            logical_count += 1;
            if visual_total >= visual_budget {
                break;
            }
        }

        // Always release at least 1 line
        if logical_count == 0 {
            logical_count = 1;
        }

        let to_release: Vec<OutputLine> = self
            .pending_lines
            .drain(..logical_count.min(self.pending_lines.len()))
            .collect();
        for line in to_release {
            if line.marked_new && self.first_marked_new_index.is_none() {
                self.first_marked_new_index = Some(self.output_lines.len());
            }
            self.output_lines.push(line);
        }
        if self.pending_lines.is_empty() {
            self.paused = false;
            self.pending_since = None;

            self.lines_since_pause = 0;
            // If partial was in pending, it's now in output
            if self.partial_in_pending {
                self.partial_in_pending = false;
            }
        } else {
            // Reset counter for next batch

            self.lines_since_pause = 0;
        }
        self.scroll_to_bottom();
    }

    pub fn release_all_pending(&mut self) {
        if self.first_marked_new_index.is_none() && self.pending_lines.iter().any(|l| l.marked_new) {
            self.first_marked_new_index = Some(self.output_lines.len());
        }
        self.output_lines.append(&mut self.pending_lines);
        self.paused = false;
        self.lines_since_pause = 0;
        self.pending_since = None; // Clear pending timestamp
        self.visual_line_offset = 0;
        // If partial was in pending, it's now in output
        if self.partial_in_pending {
            self.partial_in_pending = false;
        }
        self.scroll_to_bottom();
    }

    /// Filter output to hide client-generated lines (mark them as gagged instead of removing).
    /// They become visible again with F2 (show_tags mode).
    fn filter_to_server_output(&mut self) {
        for line in &mut self.output_lines {
            if !line.from_server {
                line.gagged = true;
            }
        }
        for line in &mut self.pending_lines {
            if !line.from_server {
                line.gagged = true;
            }
        }
        // Clear new line indicators (Ctrl+L clears them)
        // Note: retain() invalidates first_marked_new_index, so reset and scan
        self.first_marked_new_index = None;
        for line in &mut self.output_lines {
            line.marked_new = false;
        }
        for line in &mut self.pending_lines {
            line.marked_new = false;
        }
        self.visual_line_offset = 0;
        // Adjust scroll offset if it's now past the end
        if self.scroll_offset > 0 && self.scroll_offset >= self.output_lines.len() {
            self.scroll_offset = self.output_lines.len().saturating_sub(1);
        }
    }

    fn is_at_bottom(&self) -> bool {
        self.scroll_offset >= self.output_lines.len().saturating_sub(1)
    }

    fn lines_from_bottom(&self, show_tags: bool) -> usize {
        if self.scroll_offset >= self.output_lines.len().saturating_sub(1) {
            return 0;
        }
        if show_tags {
            // All lines visible when show_tags is on
            self.output_lines.len().saturating_sub(1).saturating_sub(self.scroll_offset)
        } else {
            // Count only non-gagged lines after scroll_offset
            self.output_lines[(self.scroll_offset + 1)..]
                .iter()
                .filter(|l| !l.gagged)
                .count()
        }
    }

    fn generate_splash_lines() -> Vec<OutputLine> {
        // Splash content without centering - will be centered at render time
        // Dog art:
        //           (\/\__o
        //   __      `-/ `_/
        //  `--\______/  |
        //     /        /
        //  -`/_------'\_.
        let now = SystemTime::now();
        // Splash lines use seq 0-11 (will be cleared when real MUD data arrives)
        vec![
            OutputLine::new_with_timestamp("".to_string(), now, 0),
            OutputLine::new_with_timestamp("\x1b[38;5;180m          (\\/\\__o     \x1b[38;5;209m ██████╗██╗      █████╗ ██╗   ██╗\x1b[0m".to_string(), now, 1),
            OutputLine::new_with_timestamp("\x1b[38;5;180m  __      `-/ `_/     \x1b[38;5;208m██╔════╝██║     ██╔══██╗╚██╗ ██╔╝\x1b[0m".to_string(), now, 2),
            OutputLine::new_with_timestamp("\x1b[38;5;180m `--\\______/  |       \x1b[38;5;215m██║     ██║     ███████║ ╚████╔╝ \x1b[0m".to_string(), now, 3),
            OutputLine::new_with_timestamp("\x1b[38;5;180m    /        /        \x1b[38;5;216m██║     ██║     ██╔══██║  ╚██╔╝  \x1b[0m".to_string(), now, 4),
            OutputLine::new_with_timestamp("\x1b[38;5;180m -`/_------'\\_.       \x1b[38;5;217m╚██████╗███████╗██║  ██║   ██║   \x1b[0m".to_string(), now, 5),
            OutputLine::new_with_timestamp("\x1b[38;5;218m                       ╚═════╝╚══════╝╚═╝  ╚═╝   ╚═╝   \x1b[0m".to_string(), now, 6),
            OutputLine::new_with_timestamp("".to_string(), now, 7),
            OutputLine::new_with_timestamp("\x1b[38;5;213m✨ A 90dies mud client written today ✨\x1b[0m".to_string(), now, 8),
            OutputLine::new_with_timestamp("".to_string(), now, 9),
            OutputLine::new_with_timestamp("\x1b[38;5;244m/help for how to use clay\x1b[0m".to_string(), now, 10),
            OutputLine::new_with_timestamp("".to_string(), now, 11),
        ]
    }
}

pub struct App {
    pub worlds: Vec<World>,
    pub current_world_index: usize,
    pub previous_world_index: Option<usize>, // For Alt+w fallback when no unseen/pending
    pub input: InputArea,
    pub input_height: u16,
    pub output_height: u16,
    pub output_width: u16,
    pub spell_checker: SpellChecker,
    pub spell_state: SpellState,
    pub last_input_was_delete: bool, // Track if last input action was backspace/delete (for spell check)
    pub skip_temp_conversion: Option<String>, // Temperature to skip re-converting (after user undid conversion)
    pub cached_misspelled: Vec<(usize, usize)>, // Cached misspelled word ranges (char positions)
    pub suggestion_message: Option<String>,
    pub settings: Settings,
    pub confirm_dialog: ConfirmDialog,
    pub filter_popup: FilterPopup,
    /// Split-screen text editor for notes and files
    pub editor: EditorState,
    /// New unified popup manager (gradual migration from old popup types)
    pub popup_manager: popup::PopupManager,
    pub last_ctrl_c: Option<std::time::Instant>,
    pub last_escape: Option<std::time::Instant>, // For Escape+key sequences (Alt emulation)
    pub literal_next: bool, // When true, next keypress inserts literally (Ctrl+V)
    pub show_tags: bool, // F2 toggles - false = hide tags (default), true = show tags
    pub highlight_actions: bool, // F8 toggles - highlight lines matching action patterns
    // WebSocket server (ws:// or wss:// depending on web_secure setting)
    pub ws_server: Option<WebSocketServer>,
    // HTTP web interface server (no TLS)
    pub http_server: Option<HttpServer>,
    // HTTPS web interface server
    #[cfg(feature = "native-tls-backend")]
    pub https_server: Option<HttpsServer>,
    #[cfg(feature = "rustls-backend")]
    pub https_server: Option<HttpsServer>,
    // Track if popup was visible last frame (for terminal clear on transition)
    pub popup_was_visible: bool,
    /// Cache of each WS client's view state (for activity indicator and more-mode)
    /// Maps client_id -> ClientViewState (world_index + visible_lines)
    pub ws_client_worlds: std::collections::HashMap<u64, ClientViewState>,
    /// True if this is the master client (runs WS server or WS disabled).
    /// Only master should save settings or initiate connections.
    pub is_master: bool,
    /// Set to true after a web client authenticates, to trigger reconnects in the event loop.
    pub web_reconnect_needed: bool,
    /// Set to true when http_port/http_enabled/web_secure changes, to restart the HTTP server.
    pub web_restart_needed: bool,
    /// True if this session started from a hot reload (suppress server startup messages)
    pub is_reload: bool,
    /// True if the output area needs to be redrawn (optimization to avoid unnecessary redraws)
    pub needs_output_redraw: bool,
    /// True if terminal needs full clear (for Ctrl+L redraw in --console mode)
    pub needs_terminal_clear: bool,
    /// True if mouse capture is currently active in the terminal
    pub mouse_capture_active: bool,
    /// True if running in multiuser mode (--multiuser flag)
    pub multiuser_mode: bool,
    /// User accounts (multiuser mode only)
    pub users: Vec<User>,
    /// Ban list for HTTP/WebSocket security
    pub ban_list: BanList,
    /// Per-user connections in multiuser mode: (world_index, username) -> UserConnection
    pub user_connections: std::collections::HashMap<(usize, String), UserConnection>,
    /// TinyFugue scripting engine
    pub tf_engine: tf::TfEngine,
    /// Loaded theme colors from ~/.clay.theme.dat
    pub theme_file: theme::ThemeFile,
    /// Configurable keyboard bindings (TF defaults + user customizations from ~/.clay.key.dat)
    pub keybindings: keybindings::KeyBindings,
    /// Remote client mode: WebSocket transmitter for sending commands to server
    pub ws_client_tx: Option<mpsc::UnboundedSender<WsMessage>>,
    /// Remote client mode: pending /update request (Some(force_flag))
    pub pending_update: Option<bool>,
    /// Remote client mode: pending /reload request (re-exec local binary)
    pub pending_reload: bool,
    /// Activity count from server (used in remote client mode, i.e. --console)
    pub server_activity_count: usize,
    /// Remote client backfill: queue of (world_index, total_output_lines) to backfill
    pub backfill_queue: Vec<(usize, usize)>,
    /// Remote client backfill: next request to send (world_index, before_seq)
    pub backfill_next: Option<(usize, Option<u64>)>,
    /// Master GUI mode: channel to send messages to the embedded GUI
    pub gui_tx: Option<mpsc::UnboundedSender<WsMessage>>,
    /// Master GUI mode: callback to wake the GUI for immediate repaint
    pub gui_repaint: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// Audio backend for console playback (native rodio or external mpv/ffplay)
    pub audio_backend: audio::AudioBackend,
    /// Directory for cached media files
    pub media_cache_dir: PathBuf,
    /// Running media playback handles keyed by identifier (for Stop) - value is (world_idx, handle)
    pub media_processes: std::collections::HashMap<String, (usize, audio::PlayHandle)>,
    /// Current music process key (only one music track at a time) - (world_idx, key)
    pub media_music_key: Option<(usize, String)>,
    /// Event channel sender for async media events
    pub event_tx: Option<mpsc::Sender<AppEvent>>,
    /// Currently playing ANSI music handle
    /// Tracked so we can stop it before starting a new sequence
    pub ansi_music_handle: Option<audio::PlayHandle>,
    /// Counter for unique ANSI music WAV filenames (external player only)
    pub ansi_music_counter: u32,
    /// Shared set of client IDs that have responded to the current /remote PingCheck
    pub remote_ping_responses: Option<std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u64>>>>,
    /// Nonce for current /remote ping check (to ignore stale PongCheck responses)
    pub remote_ping_nonce: u64,
    /// Text-to-speech backend (console: espeak/say subprocess, web: ServerSpeak WS message)
    pub tts_backend: tts::TtsBackend,
    /// Test-only: log of all messages passed to ws_broadcast() and ws_broadcast_to_world()
    #[cfg(test)]
    pub ws_broadcast_log: std::sync::Arc<std::sync::Mutex<Vec<WsMessage>>>,
}

/// Result of handling a WS client message that may require async follow-up.
enum WsAsyncAction {
    /// Message was fully handled synchronously.
    Done,
    /// Need to run /connect for world_index. prev_index is the world to restore after.
    /// broadcast = true means broadcast WorldConnected on success.
    Connect { world_index: usize, prev_index: usize, broadcast: bool },
    /// Need to run /disconnect for world_index. prev_index is the world to restore after.
    Disconnect { world_index: usize, prev_index: usize },
    /// Need to trigger hot reload (exec_reload).
    Reload,
}

impl App {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            worlds: Vec::new(),
            current_world_index: 0,
            previous_world_index: None,
            input: InputArea::new(3),
            input_height: 3,
            output_height: 20, // Will be updated by ui()
            output_width: 80,  // Will be updated by ui()
            spell_checker: SpellChecker::new(""),
            spell_state: SpellState::new(),
            last_input_was_delete: false,
            skip_temp_conversion: None,
            cached_misspelled: Vec::new(),
            suggestion_message: None,
            settings: Settings::default(),
            confirm_dialog: ConfirmDialog::new(),
            filter_popup: FilterPopup::new(),
            editor: EditorState::new(),
            popup_manager: popup::PopupManager::new(),
            last_ctrl_c: None,
            last_escape: None,
            literal_next: false,
            show_tags: false, // Default: hide tags
            highlight_actions: false, // Default: don't highlight action matches
            ws_server: None,
            http_server: None,
            #[cfg(feature = "native-tls-backend")]
            https_server: None,
            #[cfg(feature = "rustls-backend")]
            https_server: None,
            popup_was_visible: false,
            ws_client_worlds: std::collections::HashMap::new(),
            is_master: true, // Console app is always master (remote GUI is separate execution path)
            web_reconnect_needed: false,
            web_restart_needed: false,
            is_reload: false, // Set to true in run_app if started from hot reload
            needs_output_redraw: true, // Start with true to ensure initial render
            needs_terminal_clear: false, // Set to true by Ctrl+L in --console mode
            mouse_capture_active: false, // Toggled dynamically when popups open/close
            multiuser_mode: false, // Set to true in main if started with --multiuser
            users: Vec::new(),
            ban_list: BanList::new(),
            user_connections: std::collections::HashMap::new(),
            tf_engine: tf::TfEngine::new(),
            theme_file: theme::ThemeFile::with_defaults(),
            keybindings: keybindings::KeyBindings::tf_defaults(),
            ws_client_tx: None, // Set when running as remote client (--console mode)
            pending_update: None,
            pending_reload: false,
            server_activity_count: 0, // Activity count from server (remote client mode)
            backfill_queue: Vec::new(),
            backfill_next: None,
            gui_tx: None, // Set when running in master GUI mode (--gui)
            gui_repaint: None,
            audio_backend: audio::AudioBackend::None, // Lazy init on first use
            media_cache_dir: {
                let home = get_home_dir();
                PathBuf::from(home).join(clay_filename("clay")).join("media")
            },
            media_processes: std::collections::HashMap::new(),
            media_music_key: None,
            event_tx: None,
            ansi_music_handle: None,
            ansi_music_counter: 0,
            remote_ping_responses: None,
            remote_ping_nonce: 0,
            tts_backend: tts::init_tts(),
            #[cfg(test)]
            ws_broadcast_log: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
        // Note: No initial world created here - it will be created after persistence::load_settings()
        // if no worlds are configured
    }

    /// Get theme colors for the current console theme
    pub fn theme_colors(&self) -> &theme::ThemeColors {
        self.theme_file.get(self.settings.theme.name())
    }

    /// Get theme colors for the current GUI theme
    pub fn gui_theme_colors(&self) -> &theme::ThemeColors {
        self.theme_file.get(self.settings.gui_theme.name())
    }

    /// Build a GlobalSettingsMsg from current app state
    pub fn build_global_settings_msg(&self) -> GlobalSettingsMsg {
        GlobalSettingsMsg {
            more_mode_enabled: self.settings.more_mode_enabled,
            spell_check_enabled: self.settings.spell_check_enabled,
            temp_convert_enabled: self.settings.temp_convert_enabled,
            world_switch_mode: self.settings.world_switch_mode.name().to_string(),
            debug_enabled: self.settings.debug_enabled,
            show_tags: self.show_tags,
            ansi_music_enabled: self.settings.ansi_music_enabled,
            console_theme: self.settings.theme.name().to_string(),
            gui_theme: self.settings.gui_theme.name().to_string(),
            gui_transparency: self.settings.gui_transparency,
            color_offset_percent: self.settings.color_offset_percent,
            input_height: self.input_height,
            font_name: self.settings.font_name.clone(),
            font_size: self.settings.font_size,
            web_font_size_phone: self.settings.web_font_size_phone,
            web_font_size_tablet: self.settings.web_font_size_tablet,
            web_font_size_desktop: self.settings.web_font_size_desktop,
            web_font_weight: self.settings.web_font_weight,
            web_font_line_height: self.settings.web_font_line_height,
            web_font_letter_spacing: self.settings.web_font_letter_spacing,
            web_font_word_spacing: self.settings.web_font_word_spacing,
            ws_allow_list: self.settings.websocket_allow_list.clone(),
            web_secure: self.settings.web_secure,
            http_enabled: self.settings.http_enabled,
            http_port: self.settings.http_port,
            ws_enabled: false,  // Legacy
            ws_port: 0,        // Legacy
            ws_cert_file: self.settings.websocket_cert_file.clone(),
            ws_key_file: self.settings.websocket_key_file.clone(),
            tls_configured: !self.settings.websocket_cert_file.is_empty() && !self.settings.websocket_key_file.is_empty(),
            tls_proxy_enabled: self.settings.tls_proxy_enabled,
            dictionary_path: self.settings.dictionary_path.clone(),
            mouse_enabled: self.settings.mouse_enabled,
            zwj_enabled: self.settings.zwj_enabled,
            new_line_indicator: self.settings.new_line_indicator,
            tts_mode: self.settings.tts_mode.name().to_string(),
            tts_speak_mode: self.settings.tts_speak_mode.name().to_string(),
            theme_colors_json: self.gui_theme_colors().to_json(),
            keybindings_json: self.keybindings.to_json(),
            auth_key: self.settings.websocket_auth_key.as_ref().map(|ak| ak.key.clone()).unwrap_or_default(),
            ws_password: self.settings.websocket_password.clone(),
        }
    }

    /// Apply all global settings from a GlobalSettingsMsg to local state.
    /// Used by remote clients to sync settings from the master on connect and on updates.
    fn apply_global_settings(&mut self, settings: &GlobalSettingsMsg) {
        self.settings.more_mode_enabled = settings.more_mode_enabled;
        self.settings.spell_check_enabled = settings.spell_check_enabled;
        self.settings.temp_convert_enabled = settings.temp_convert_enabled;
        self.settings.world_switch_mode = WorldSwitchMode::from_name(&settings.world_switch_mode);
        self.show_tags = settings.show_tags;
        self.settings.debug_enabled = settings.debug_enabled;
        DEBUG_ENABLED.store(settings.debug_enabled, Ordering::Relaxed);
        self.settings.ansi_music_enabled = settings.ansi_music_enabled;
        self.settings.theme = Theme::from_name(&settings.console_theme);
        self.settings.gui_theme = Theme::from_name(&settings.gui_theme);
        self.settings.gui_transparency = settings.gui_transparency;
        self.settings.color_offset_percent = settings.color_offset_percent;
        self.input_height = settings.input_height;
        self.input.visible_height = self.input_height;
        self.settings.font_name = settings.font_name.clone();
        self.settings.font_size = settings.font_size;
        self.settings.web_font_size_phone = settings.web_font_size_phone;
        self.settings.web_font_size_tablet = settings.web_font_size_tablet;
        self.settings.web_font_size_desktop = settings.web_font_size_desktop;
        self.settings.web_font_weight = settings.web_font_weight;
        self.settings.web_font_line_height = settings.web_font_line_height;
        self.settings.web_font_letter_spacing = settings.web_font_letter_spacing;
        self.settings.web_font_word_spacing = settings.web_font_word_spacing;
        self.settings.websocket_allow_list = settings.ws_allow_list.clone();
        self.settings.web_secure = settings.web_secure;
        self.settings.http_enabled = settings.http_enabled;
        self.settings.http_port = settings.http_port;
        // ws_enabled and ws_port are legacy — ignored
        // Only update cert/key if non-empty (server sends empty to avoid leaking paths)
        if !settings.ws_cert_file.is_empty() {
            self.settings.websocket_cert_file = settings.ws_cert_file.clone();
        }
        if !settings.ws_key_file.is_empty() {
            self.settings.websocket_key_file = settings.ws_key_file.clone();
        }
        self.settings.tls_proxy_enabled = settings.tls_proxy_enabled;
        self.settings.dictionary_path = settings.dictionary_path.clone();
        self.settings.mouse_enabled = settings.mouse_enabled;
        self.settings.zwj_enabled = settings.zwj_enabled;
        self.settings.new_line_indicator = settings.new_line_indicator;
        let old_tts_mode = self.settings.tts_mode;
        self.settings.tts_mode = tts::TtsMode::from_name(&settings.tts_mode);
        self.settings.tts_speak_mode = tts::TtsSpeakMode::from_name(&settings.tts_speak_mode);
        // Auto-unmute when TTS is enabled
        if old_tts_mode == tts::TtsMode::Off && self.settings.tts_mode != tts::TtsMode::Off {
            self.settings.tts_muted = false;
        }
        // Sync keybindings from master
        if !settings.keybindings_json.is_empty() {
            self.keybindings = keybindings::KeyBindings::from_json(&settings.keybindings_json);
        }
    }

    /// Ensure there's at least one world (creates initial world if needed)
    /// Also adds splash screen to current world if it has no output
    fn ensure_has_world(&mut self) {
        if self.worlds.is_empty() {
            let mut initial_world = World::new_with_splash(&get_binary_name(), true);
            initial_world.is_initial_world = true;
            self.worlds.push(initial_world);
        } else {
            // Add splash to current world if it has no output yet
            let current = &mut self.worlds[self.current_world_index];
            if current.output_lines.is_empty() && !current.connected {
                current.output_lines = World::generate_splash_lines();
                current.showing_splash = true;
                current.scroll_offset = current.output_lines.len().saturating_sub(1);
            }
        }
    }

    pub fn current_world(&self) -> &World {
        // Safety: clamp index to valid range to prevent panic
        let idx = if self.worlds.is_empty() {
            0  // Will panic below, but ensure_has_world() should prevent this
        } else {
            self.current_world_index.min(self.worlds.len() - 1)
        };
        &self.worlds[idx]
    }

    pub fn current_world_mut(&mut self) -> &mut World {
        // Safety: clamp index to valid range to prevent panic
        let idx = if self.worlds.is_empty() {
            0  // Will panic below, but ensure_has_world() should prevent this
        } else {
            self.current_world_index.min(self.worlds.len() - 1)
        };
        &mut self.worlds[idx]
    }

    /// Sync world info to TfEngine for TF functions (fg_world, world_info, nactive)
    fn sync_tf_world_info(&mut self) {
        // Set current world name
        self.tf_engine.current_world = if !self.worlds.is_empty() {
            Some(self.worlds[self.current_world_index].name.clone())
        } else {
            None
        };

        // Build world info cache
        let now = std::time::Instant::now();
        self.tf_engine.world_info_cache.clear();
        for world in &self.worlds {
            self.tf_engine.world_info_cache.push(tf::WorldInfoCache {
                name: world.name.clone(),
                host: world.settings.hostname.clone(),
                port: world.settings.port.clone(),
                user: world.settings.user.clone(),
                password: world.settings.password.clone(),
                is_connected: world.connected,
                use_ssl: world.settings.use_ssl,
                unseen_lines: world.unseen_lines,
                last_receive_secs_ago: world.last_receive_time.map(|t| now.duration_since(t).as_secs() as i64),
                last_send_secs_ago: world.last_send_time.map(|t| now.duration_since(t).as_secs() as i64),
            });
        }

        // Sync keyboard buffer state
        self.tf_engine.keyboard_state = tf::KeyboardBufferState {
            buffer: self.input.buffer.clone(),
            cursor_position: self.input.cursor_position,
        };
    }

    /// Process pending keyboard operations from TF functions
    fn process_pending_keyboard_ops(&mut self) {
        let ops: Vec<tf::PendingKeyboardOp> = self.tf_engine.pending_keyboard_ops.drain(..).collect();
        for op in ops {
            match op {
                tf::PendingKeyboardOp::Goto(pos) => {
                    let max_pos = self.input.buffer.chars().count();
                    self.input.cursor_position = pos.min(max_pos);
                }
                tf::PendingKeyboardOp::Delete(count) => {
                    if count > 0 {
                        // Delete forward
                        let chars: Vec<char> = self.input.buffer.chars().collect();
                        let pos = self.input.cursor_position;
                        let end = (pos + count as usize).min(chars.len());
                        let new_buffer: String = chars[..pos].iter().chain(chars[end..].iter()).collect();
                        self.input.buffer = new_buffer;
                    } else if count < 0 {
                        // Delete backward
                        let chars: Vec<char> = self.input.buffer.chars().collect();
                        let pos = self.input.cursor_position;
                        let start = pos.saturating_sub((-count) as usize);
                        let new_buffer: String = chars[..start].iter().chain(chars[pos..].iter()).collect();
                        self.input.buffer = new_buffer;
                        self.input.cursor_position = start;
                    }
                }
                tf::PendingKeyboardOp::WordLeft => {
                    let chars: Vec<char> = self.input.buffer.chars().collect();
                    let mut pos = self.input.cursor_position;
                    // Skip whitespace
                    while pos > 0 && !chars[pos - 1].is_alphanumeric() {
                        pos -= 1;
                    }
                    // Skip word
                    while pos > 0 && chars[pos - 1].is_alphanumeric() {
                        pos -= 1;
                    }
                    self.input.cursor_position = pos;
                }
                tf::PendingKeyboardOp::WordRight => {
                    let chars: Vec<char> = self.input.buffer.chars().collect();
                    let mut pos = self.input.cursor_position;
                    // Skip word
                    while pos < chars.len() && chars[pos].is_alphanumeric() {
                        pos += 1;
                    }
                    // Skip whitespace
                    while pos < chars.len() && !chars[pos].is_alphanumeric() {
                        pos += 1;
                    }
                    self.input.cursor_position = pos;
                }
                tf::PendingKeyboardOp::Insert(text) => {
                    let chars: Vec<char> = self.input.buffer.chars().collect();
                    let pos = self.input.cursor_position.min(chars.len());
                    let before: String = chars[..pos].iter().collect();
                    let after: String = chars[pos..].iter().collect();
                    self.input.buffer = format!("{}{}{}", before, text, after);
                    self.input.cursor_position = pos + text.chars().count();
                }
            }
        }
    }

    /// Check if a new-style popup is currently visible
    fn has_new_popup(&self) -> bool {
        self.popup_manager.current().map(|s| s.visible).unwrap_or(false)
    }

    /// Open the help popup using the new unified popup system
    fn open_help_popup_new(&mut self) {
        use popup::definitions::help::{create_help_popup, HELP_FIELD_CONTENT};
        self.popup_manager.open(create_help_popup());
        // Select the content field so arrow keys can scroll
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_field(HELP_FIELD_CONTENT);
        }
    }

    /// Close the current new-style popup
    fn close_new_popup(&mut self) {
        self.popup_manager.close();
    }

    /// Open the menu popup using the new unified popup system
    fn open_menu_popup_new(&mut self) {
        use popup::definitions::menu::create_menu_popup;
        self.popup_manager.open(create_menu_popup());
    }

    /// Open a confirm dialog for deleting a world
    fn open_delete_world_confirm(&mut self, world_name: &str, world_index: usize) {
        use popup::definitions::confirm::{create_delete_world_dialog, CONFIRM_BTN_NO};
        let mut def = create_delete_world_dialog(world_name);
        // Store the world index in custom_data for retrieval on confirm
        def.custom_data.insert("world_index".to_string(), world_index.to_string());
        self.popup_manager.open(def);
        // Select No by default for safety
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_button(CONFIRM_BTN_NO);
        }
    }

    /// Open the world selector popup using the new unified popup system
    fn open_world_selector_new(&mut self) {
        use popup::definitions::world_selector::{create_world_selector_popup, WorldInfo, SELECTOR_FIELD_LIST};

        let worlds: Vec<WorldInfo> = self.worlds.iter().enumerate().map(|(i, w)| {
            WorldInfo {
                name: w.name.clone(),
                hostname: w.settings.hostname.clone(),
                port: w.settings.port.to_string(),
                user: w.settings.user.clone(),
                is_connected: w.connected,
                is_current: i == self.current_world_index,
            }
        }).collect();

        let visible_height = 10.min(worlds.len().max(3));
        let def = create_world_selector_popup(&worlds, visible_height);
        self.popup_manager.open(def);

        // Select current world in the list
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_field(SELECTOR_FIELD_LIST);
            // Set selection to current world
            if let Some(field) = state.field_mut(SELECTOR_FIELD_LIST) {
                if let popup::FieldKind::List { selected_index, .. } = &mut field.kind {
                    *selected_index = self.current_world_index;
                }
            }
        }
    }

    /// Open the notes list popup showing worlds with notes
    fn open_notes_list_popup(&mut self) {
        use popup::definitions::notes_list::{create_notes_list_popup, NoteInfo, NOTES_FIELD_LIST};

        let notes: Vec<NoteInfo> = self.worlds.iter().enumerate()
            .filter(|(_, w)| !w.settings.notes.trim().is_empty())
            .map(|(i, w)| {
                let first_line = w.settings.notes.lines().next().unwrap_or("");
                let preview = if first_line.len() > 50 {
                    format!("{}...", &first_line[..47])
                } else {
                    first_line.to_string()
                };
                NoteInfo {
                    world_name: w.name.clone(),
                    preview,
                    is_current: i == self.current_world_index,
                }
            })
            .collect();

        if notes.is_empty() {
            self.add_output("No worlds have notes. Use /edit to create notes for the current world.");
            return;
        }

        let visible_height = 10.min(notes.len().max(3));
        let def = create_notes_list_popup(&notes, visible_height);
        self.popup_manager.open(def);

        // Select current world in the list if it has notes
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_field(NOTES_FIELD_LIST);
            if let Some(current_idx) = notes.iter().position(|n| n.is_current) {
                if let Some(field) = state.field_mut(NOTES_FIELD_LIST) {
                    if let popup::FieldKind::List { selected_index, .. } = &mut field.kind {
                        *selected_index = current_idx;
                    }
                }
            }
        }
    }

    /// Open the new setup popup for global settings
    fn open_setup_popup_new(&mut self) {
        use popup::definitions::setup::{create_setup_popup, SETUP_FIELD_MORE_MODE};

        let world_switching = match self.settings.world_switch_mode {
            WorldSwitchMode::UnseenFirst => "unseen_first",
            WorldSwitchMode::Alphabetical => "alphabetical",
        };

        let def = create_setup_popup(
            self.settings.more_mode_enabled,
            self.settings.spell_check_enabled,
            self.settings.temp_convert_enabled,
            world_switching,
            self.settings.debug_enabled,
            self.input_height as i64,
            self.settings.gui_theme.name(),
            self.settings.tls_proxy_enabled,
            &self.settings.dictionary_path,
            self.settings.editor_side.name(),
            self.settings.mouse_enabled,
            self.settings.zwj_enabled,
            self.settings.ansi_music_enabled,
            self.settings.new_line_indicator,
            self.settings.tts_mode.name(),
            self.settings.tts_speak_mode.name(),
        );
        self.popup_manager.open(def);

        // Select first field
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_field(SETUP_FIELD_MORE_MODE);
        }
    }

    /// Open the new web settings popup
    fn open_web_popup_new(&mut self) {
        use popup::definitions::web::{create_web_popup, WEB_FIELD_PROTOCOL};

        let auth_key_str = self.settings.websocket_auth_key
            .as_ref()
            .map(|ak| ak.key.clone())
            .unwrap_or_default();
        let def = create_web_popup(
            self.settings.web_secure,
            self.settings.http_enabled,
            &self.settings.http_port.to_string(),
            &self.settings.websocket_password,
            &self.settings.websocket_allow_list,
            &self.settings.websocket_cert_file,
            &self.settings.websocket_key_file,
            &auth_key_str,
        );
        self.popup_manager.open(def);

        // Select first field
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_field(WEB_FIELD_PROTOCOL);
        }
    }

    /// Open the world editor popup for a specific world
    fn open_world_editor_popup_new(&mut self, world_index: usize) {
        use popup::definitions::world_editor::{
            create_world_editor_popup, WorldSettings as PopupWorldSettings, WORLD_FIELD_NAME,
        };

        let world = &self.worlds[world_index];

        // Map auto_connect type to lowercase value for popup
        let auto_connect = match world.settings.auto_connect_type {
            AutoConnectType::Connect => "connect",
            AutoConnectType::Prompt => "prompt",
            AutoConnectType::MooPrompt => "moo_prompt",
            AutoConnectType::NoLogin => "none",
        };

        // Map keep_alive type to lowercase value for popup
        let keep_alive = match world.settings.keep_alive_type {
            KeepAliveType::None => "none",
            KeepAliveType::Nop => "nop",
            KeepAliveType::Custom => "custom",
            KeepAliveType::Generic => "generic",
        };

        let settings = PopupWorldSettings {
            name: world.name.clone(),
            world_type: world.settings.world_type.name().to_string(),
            hostname: world.settings.hostname.clone(),
            port: world.settings.port.clone(),
            user: world.settings.user.clone(),
            password: world.settings.password.clone(),
            use_ssl: world.settings.use_ssl,
            log_enabled: world.settings.log_enabled,
            encoding: world.settings.encoding.name().to_string(),
            auto_connect: auto_connect.to_string(),
            keep_alive: keep_alive.to_string(),
            keep_alive_cmd: world.settings.keep_alive_cmd.clone(),
            gmcp_packages: world.settings.gmcp_packages.clone(),
            auto_reconnect_secs: world.settings.auto_reconnect_display(),
            slack_token: world.settings.slack_token.clone(),
            slack_channel: world.settings.slack_channel.clone(),
            slack_workspace: world.settings.slack_workspace.clone(),
            discord_token: world.settings.discord_token.clone(),
            discord_guild: world.settings.discord_guild.clone(),
            discord_channel: world.settings.discord_channel.clone(),
            discord_dm_user: world.settings.discord_dm_user.clone(),
        };

        let def = create_world_editor_popup(&settings);
        self.popup_manager.open(def);

        // Store world index in popup custom state and select first field
        if let Some(state) = self.popup_manager.current_mut() {
            state.set_custom("world_index", world_index.to_string());
            state.select_field(WORLD_FIELD_NAME);
            // Auto-start editing on the name field
            state.start_edit();
        }
    }

    /// Open the actions list popup
    fn open_actions_list_popup(&mut self) {
        self.open_actions_list_popup_with_filter("");
    }

    /// Open the actions list popup with optional world filter
    fn open_actions_list_popup_with_filter(&mut self, world_filter: &str) {
        use popup::definitions::actions::ACTIONS_FIELD_LIST;

        // Build items with indices for the popup
        let mut items: Vec<popup::ListItem> = self.settings.actions.iter().enumerate()
            .filter(|(_, a)| {
                if world_filter.is_empty() {
                    true
                } else {
                    a.world.to_lowercase().contains(&world_filter.to_lowercase())
                }
            })
            .map(|(idx, a)| {
                #[cfg(not(windows))]
                let status = if a.enabled { "[✓]" } else { "[ ]" };
                #[cfg(windows)]
                let status = if a.enabled { "[x]" } else { "[ ]" };
                let world_part = if a.world.is_empty() {
                    String::new()
                } else {
                    format!("({})", a.world)
                };
                let pattern_preview = if a.pattern.len() > 30 {
                    format!("{}...", &a.pattern[..27])
                } else {
                    a.pattern.clone()
                };
                popup::ListItem {
                    id: idx.to_string(),  // Store original index as ID
                    columns: vec![
                        format!("{} {}", status, a.name),
                        world_part,
                        pattern_preview,
                    ],
                    style: popup::ListItemStyle {
                        is_disabled: !a.enabled,
                        ..Default::default()
                    },
                }
            })
            .collect();

        // Sort alphabetically by action name (case-insensitive)
        items.sort_by(|a, b| {
            // Extract name from first column: "[x] name" or "[ ] name"
            let name_a = a.columns[0].get(4..).unwrap_or("").to_lowercase();
            let name_b = b.columns[0].get(4..).unwrap_or("").to_lowercase();
            name_a.cmp(&name_b)
        });

        let visible_height = 10.min(items.len().max(3));

        // Create the popup with the filtered actions
        let def = popup::PopupDefinition::new(popup::PopupId("actions_list"), "Actions")
            .with_field(popup::Field::new(
                popup::definitions::actions::ACTIONS_FIELD_FILTER,
                "Filter",
                popup::FieldKind::text_with_placeholder("", "Type to filter..."),
            ))
            .with_field(popup::Field::new(
                ACTIONS_FIELD_LIST,
                "",
                popup::FieldKind::list(items, visible_height),
            ))
            .with_button(popup::Button::new(popup::definitions::actions::ACTIONS_BTN_DELETE, "Delete").danger().with_shortcut('D').left_align())
            .with_button(popup::Button::new(popup::definitions::actions::ACTIONS_BTN_ADD, "Add").with_shortcut('A'))
            .with_button(popup::Button::new(popup::definitions::actions::ACTIONS_BTN_EDIT, "Edit").with_shortcut('E'))
            .with_button(popup::Button::new(popup::definitions::actions::ACTIONS_BTN_CANCEL, "Ok").primary().with_shortcut('O'))
            .with_layout(popup::PopupLayout {
                label_width: 8,
                min_width: 70,
                max_width_percent: 85,
                center_horizontal: true,
                center_vertical: true,
                modal: true,
                buttons_right_align: false,
                blank_line_before_list: false,
                tab_buttons_only: false,
            anchor_bottom_left: false,
            anchor_x: 0,
            });

        self.popup_manager.open(def);

        // Select the list field
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_field(ACTIONS_FIELD_LIST);
        }
    }

    /// Open the action editor popup
    fn open_action_editor_popup(&mut self, editing_index: Option<usize>) {
        use popup::definitions::actions::{
            create_action_editor_popup, ActionSettings,
            EDITOR_FIELD_NAME,
        };

        let settings = if let Some(idx) = editing_index {
            if idx < self.settings.actions.len() {
                let action = &self.settings.actions[idx];
                ActionSettings {
                    name: action.name.clone(),
                    world: action.world.clone(),
                    match_type: action.match_type.as_str().to_string(),
                    pattern: action.pattern.clone(),
                    command: action.command.clone(),
                    enabled: action.enabled,
                    startup: action.startup,
                }
            } else {
                ActionSettings::default()
            }
        } else {
            ActionSettings::default()
        };

        let is_new = editing_index.is_none();
        let def = create_action_editor_popup(&settings, is_new);

        self.popup_manager.open(def);

        // Store editing index in custom state
        if let Some(state) = self.popup_manager.current_mut() {
            if let Some(idx) = editing_index {
                state.set_custom("editing_index", idx.to_string());
            }
            state.select_field(EDITOR_FIELD_NAME);
        }
    }

    /// Open delete action confirmation dialog
    fn open_delete_action_confirm(&mut self, name: &str, index: usize) {
        use popup::definitions::confirm::create_delete_action_dialog;
        let mut def = create_delete_action_dialog(name);
        def.custom_data.insert("action_index".to_string(), index.to_string());
        self.popup_manager.push(def);
    }

    /// Handle incoming WebSocket message when running as remote client
    fn handle_remote_ws_message(&mut self, msg: WsMessage) {
        match msg {
            WsMessage::ServerData { world_index, data, from_server, seq: msg_seq, marked_new, flush, .. } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    // Flush: clear output buffer before appending new lines
                    // (e.g., splash screen cleared — combined with data to avoid race condition)
                    if flush {
                        world.output_lines.clear();
                        world.first_marked_new_index = None;
                        world.pending_lines.clear();
                        world.showing_splash = false;
                        world.paused = false;
                        world.lines_since_pause = 0;
                        world.scroll_offset = 0;
                        self.needs_output_redraw = true;
                    }
                    // Dedup: skip ServerData that has already been received (e.g., after resync)
                    if msg_seq > 0 && msg_seq <= world.max_received_seq {
                        // Log duplicate to debug file
                        if is_debug_enabled() {
                            output_debug_log(&format!("DUPLICATE [console] in '{}': line_seq={}, max_seq={}, text={:?}",
                                world.name, msg_seq, world.max_received_seq,
                                data.chars().take(200).collect::<String>()));
                        }
                        // Report back to server
                        if let Some(ref tx) = self.ws_client_tx {
                            let _ = tx.send(WsMessage::ReportDuplicate {
                                world_index,
                                line_seq: msg_seq,
                                max_seq: world.max_received_seq,
                                line_text: data.chars().take(200).collect(),
                                source: "console".to_string(),
                            });
                        }
                    } else {
                        // Check if user was at bottom before adding lines
                        let was_at_bottom = world.is_at_bottom();

                        let line_count = data.lines().count();
                        for (i, line) in data.lines().enumerate() {
                            // Preserve from_server flag for Ctrl+L filtering
                            let seq = world.next_seq;
                            world.next_seq += 1;
                            let mut output_line = if from_server {
                                OutputLine::new(line.to_string(), seq)
                            } else {
                                OutputLine::new_client(line.to_string(), seq)
                            };
                            output_line.marked_new = marked_new;
                            if marked_new && world.first_marked_new_index.is_none() {
                                world.first_marked_new_index = Some(world.output_lines.len());
                            }
                            world.output_lines.push(output_line);
                            // Track max received seq from server
                            if msg_seq > 0 {
                                world.max_received_seq = msg_seq + i as u64;
                            }
                        }

                        // Keep scroll at bottom if user was viewing latest output
                        if was_at_bottom {
                            world.scroll_offset = world.output_lines.len().saturating_sub(1);
                        }

                        if world_index != self.current_world_index {
                            world.unseen_lines += line_count;
                        }
                        self.needs_output_redraw = true;
                    }
                }
            }
            WsMessage::WorldConnected { world_index, .. } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.connected = true;
                    world.was_connected = true;
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::WorldDisconnected { world_index } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.connected = false;
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::WorldSwitched { new_index } => {
                self.current_world_index = new_index;
                self.needs_output_redraw = true;
            }
            WsMessage::PromptUpdate { world_index, prompt } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.prompt = prompt;
                }
            }
            WsMessage::UnseenCleared { world_index } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.unseen_lines = 0;
                }
            }
            WsMessage::UnseenUpdate { world_index, count } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.unseen_lines = count;
                }
            }
            WsMessage::PendingLinesUpdate { world_index, count } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    // Track pending count from daemon (no actual lines stored client-side)
                    world.pending_count = count;
                    world.paused = count > 0;
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::PendingReleased { world_index, count: _ } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    // Pending lines released - daemon will send ServerData and PendingLinesUpdate
                    // Just ensure we scroll to bottom to show new content
                    world.scroll_to_bottom();
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::WorldStateResponse { world_index, pending_count, prompt, scroll_offset, recent_lines } => {
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.scroll_offset = scroll_offset;
                    world.prompt = prompt;
                    world.pending_count = pending_count;
                    world.paused = pending_count > 0;
                    // Append recent lines, skipping any already received (dedup)
                    for tl in recent_lines {
                        if tl.seq > 0 && tl.seq <= world.max_received_seq {
                            continue; // Skip duplicate line
                        }
                        let seq = world.next_seq;
                        world.next_seq += 1;
                        world.output_lines.push(OutputLine {
                            text: tl.text,
                            timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(tl.ts),
                            from_server: true,
                            gagged: tl.gagged,
                            seq,
                            highlight_color: tl.highlight_color,
                            marked_new: tl.marked_new,
                        });
                        if tl.seq > 0 {
                            world.max_received_seq = tl.seq;
                        }
                    }
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::WorldFlushed { world_index } => {
                // World's output buffer was cleared (e.g., splash screen replaced with MUD data)
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.output_lines.clear();
                    world.first_marked_new_index = None;
                    world.pending_count = 0;
                    world.paused = false;
                    world.scroll_offset = 0;
                    world.partial_line.clear();
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::CalculatedWorld { index: Some(idx) } => {
                // Server calculated next/prev world for us - switch to it
                if idx < self.worlds.len() {
                    self.current_world_index = idx;
                    // Clear unseen for the world we're switching to
                    if let Some(world) = self.worlds.get_mut(idx) {
                        world.unseen_lines = 0;
                    }
                    self.needs_output_redraw = true;
                    // Send MarkWorldSeen to notify server
                    if let Some(ref tx) = self.ws_client_tx {
                        let _ = tx.send(WsMessage::MarkWorldSeen { world_index: idx });
                    }
                }
            }
            WsMessage::CalculatedWorld { index: None } => {}
            WsMessage::ActivityUpdate { count } => {
                // Server's activity count changed - update our copy
                self.server_activity_count = count;
                self.needs_output_redraw = true;
            }
            WsMessage::ShowTagsChanged { show_tags } => {
                // Server toggled show_tags (F2 or /tag command)
                self.show_tags = show_tags;
                self.needs_output_redraw = true;
            }
            WsMessage::GmcpUserToggled { world_index, enabled } => {
                if world_index < self.worlds.len() {
                    self.worlds[world_index].gmcp_user_enabled = enabled;
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::SetInputBuffer { text, cursor_start } => {
                self.input.buffer = text;
                self.input.cursor_position = if cursor_start { 0 } else { self.input.buffer.len() };
                self.needs_output_redraw = true;
            }
            WsMessage::ConnectionsListResponse { lines } => {
                // Display the connections list from server
                let was_at_bottom = self.current_world().is_at_bottom();
                for line in lines {
                    let seq = self.current_world().next_seq;
                    let world = self.current_world_mut();
                    world.next_seq = seq + 1;
                    world.output_lines.push(OutputLine::new_client(line, seq));
                }
                if was_at_bottom {
                    self.current_world_mut().scroll_to_bottom();
                }
                self.needs_output_redraw = true;
            }
            WsMessage::ExecuteLocalCommand { command } => {
                // Server wants us to execute a command locally (from action)
                let parsed = parse_command(&command);
                match parsed {
                    Command::WorldSelector => {
                        self.open_world_selector_new();
                    }
                    Command::WorldsList => {
                        // Request connections list from server
                        if let Some(ref tx) = self.ws_client_tx {
                            let _ = tx.send(WsMessage::RequestConnectionsList);
                        }
                    }
                    Command::Help => {
                        self.open_help_popup_new();
                    }
                    Command::HelpTopic { ref topic } => {
                        use popup::definitions::help::{get_topic_help, create_topic_help_popup, HELP_FIELD_CONTENT};
                        if let Some(lines) = get_topic_help(topic) {
                            self.popup_manager.open(create_topic_help_popup(lines));
                            if let Some(state) = self.popup_manager.current_mut() {
                                state.select_field(HELP_FIELD_CONTENT);
                            }
                        }
                    }
                    Command::Setup => {
                        self.open_setup_popup_new();
                    }
                    Command::Web => {
                        self.open_web_popup_new();
                    }
                    Command::Actions { world } => {
                        if let Some(world_name) = world {
                            self.open_actions_list_popup_with_filter(&world_name);
                        } else {
                            self.open_actions_list_popup();
                        }
                    }
                    Command::Menu => {
                        self.open_menu_popup_new();
                    }
                    Command::Font => {
                        // Font popup is handled by each client type locally
                        // Remote GUI opens its own font popup
                    }
                    Command::Update { force } => {
                        // Signal to the outer run_console_client loop to handle locally
                        self.pending_update = Some(force);
                    }
                    Command::Quit => {
                        // Handled by the outer run_console_client loop
                    }
                    _ => {
                        // Unknown local command - ignore or log
                    }
                }
            }
            WsMessage::WorldSwitchResult { world_index, world_name: _, pending_count, paused } => {
                // Response to CycleWorld - update local world index and state
                if world_index < self.worlds.len() {
                    self.current_world_index = world_index;
                    if let Some(world) = self.worlds.get_mut(world_index) {
                        world.pending_count = pending_count;
                        world.paused = paused;
                        world.unseen_lines = 0;
                    }
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::OutputLines { world_index, lines, is_initial: _ } => {
                // Batch of output lines from server
                if let Some(world) = self.worlds.get_mut(world_index) {
                    let was_at_bottom = world.is_at_bottom();
                    for line in lines {
                        let output_line = OutputLine {
                            text: line.text,
                            timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(line.ts),
                            from_server: line.from_server,
                            gagged: line.gagged,
                            seq: line.seq,
                            highlight_color: line.highlight_color,
                            marked_new: line.marked_new,
                        };
                        world.output_lines.push(output_line);
                        if line.seq >= world.next_seq {
                            world.next_seq = line.seq + 1;
                        }
                    }
                    if was_at_bottom {
                        world.scroll_to_bottom();
                    }
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::PendingCountUpdate { world_index, count } => {
                // Periodic pending count update from server
                if let Some(world) = self.worlds.get_mut(world_index) {
                    world.pending_count = count;
                    world.paused = count > 0;
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::ScrollbackLines { world_index, lines, backfill_complete } => {
                // Response to RequestScrollback - prepend lines to output
                if let Some(world) = self.worlds.get_mut(world_index) {
                    // Insert at the beginning of output_lines
                    let new_lines: Vec<OutputLine> = lines.into_iter().map(|line| {
                        OutputLine {
                            text: line.text,
                            timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(line.ts),
                            from_server: line.from_server,
                            gagged: line.gagged,
                            seq: line.seq,
                            highlight_color: line.highlight_color,
                            marked_new: line.marked_new,
                        }
                    }).collect();
                    let prepended_count = new_lines.len();
                    let mut combined = new_lines;
                    combined.append(&mut world.output_lines);
                    world.output_lines = combined;
                    // Adjust scroll_offset to keep viewing the same content
                    world.scroll_offset += prepended_count;
                    self.needs_output_redraw = true;
                }
                // Track backfill progress for remote console
                if !backfill_complete {
                    // More lines available - request next chunk
                    if let Some(world) = self.worlds.get(world_index) {
                        let oldest_seq = world.output_lines.first().map(|l| l.seq);
                        self.backfill_next = Some((world_index, oldest_seq));
                    }
                } else {
                    // This world is done - advance to next world in queue
                    self.backfill_advance_to_next();
                }
            }
            WsMessage::GlobalSettingsUpdated { settings, input_height: _ } => {
                // Master or another client updated global settings - sync our local copy
                self.apply_global_settings(&settings);
                self.needs_output_redraw = true;
            }
            WsMessage::PingCheck { nonce } => {
                // Server liveness check for /remote command - respond immediately
                if let Some(ref tx) = self.ws_client_tx {
                    let _ = tx.send(WsMessage::PongCheck { nonce });
                }
            }
            _ => {}
        }
    }

    /// Initialize App state from InitialState message (remote client mode)
    fn init_from_initial_state(
        &mut self,
        worlds: Vec<WorldStateMsg>,
        current_world_index: usize,
        settings: GlobalSettingsMsg,
        splash_lines: Vec<String>,
    ) {
        self.worlds = worlds.into_iter().map(|w| {
            let mut world = World::new(&w.name);
            world.connected = w.connected;
            world.was_connected = w.was_connected;
            // Set proxy_pid sentinel if server reports proxy (actual PID not needed for display)
            world.proxy_pid = if w.is_proxy { Some(0) } else { None };
            let output_lines_count = w.output_lines_ts.len();
            world.output_lines = w.output_lines_ts.into_iter().map(|tl| {
                OutputLine {
                    text: tl.text,
                    timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(tl.ts),
                    from_server: tl.from_server,
                    gagged: tl.gagged,
                    seq: tl.seq,
                    highlight_color: tl.highlight_color,
                    marked_new: tl.marked_new,
                }
            }).collect();
            // Update next_seq to continue from highest seq in output_lines
            world.next_seq = world.output_lines.iter().map(|l| l.seq).max().unwrap_or(0).saturating_add(1);
            // Set max_received_seq for dedup (highest seq we've received from server)
            world.max_received_seq = world.next_seq.saturating_sub(1);
            world.unseen_lines = w.unseen_lines;
            // Use server's scroll_offset, or set to end of buffer if showing splash
            world.scroll_offset = if w.showing_splash {
                output_lines_count.saturating_sub(1)
            } else {
                w.scroll_offset
            };
            world.pending_lines = w.pending_lines_ts.into_iter().map(|tl| {
                OutputLine {
                    text: tl.text,
                    timestamp: std::time::UNIX_EPOCH + std::time::Duration::from_secs(tl.ts),
                    from_server: tl.from_server,
                    gagged: tl.gagged,
                    seq: tl.seq,
                    highlight_color: tl.highlight_color,
                    marked_new: tl.marked_new,
                }
            }).collect();
            // Update next_seq if pending_lines have higher seq values
            if let Some(max_pending_seq) = world.pending_lines.iter().map(|l| l.seq).max() {
                world.next_seq = world.next_seq.max(max_pending_seq.saturating_add(1));
            }
            world.paused = w.paused;
            world.prompt = w.prompt;
            world.showing_splash = w.showing_splash;
            world.gmcp_user_enabled = w.gmcp_user_enabled;
            world.settings = WorldSettings {
                hostname: w.settings.hostname,
                port: w.settings.port,
                user: w.settings.user,
                password: String::new(), // Don't receive passwords from server
                use_ssl: w.settings.use_ssl,
                log_enabled: w.settings.log_enabled,
                encoding: Encoding::from_name(&w.settings.encoding),
                auto_connect_type: AutoConnectType::from_name(&w.settings.auto_connect_type),
                keep_alive_type: KeepAliveType::from_name(&w.settings.keep_alive_type),
                keep_alive_cmd: w.settings.keep_alive_cmd,
                ..WorldSettings::default()
            };
            world
        }).collect();
        self.current_world_index = current_world_index;
        self.apply_global_settings(&settings);
        self.is_master = false; // Remote client is never master

        // If current world has no output, add splash screen
        if !splash_lines.is_empty() {
            if let Some(world) = self.worlds.get_mut(current_world_index) {
                if world.output_lines.is_empty() && !world.connected {
                    world.output_lines = splash_lines.into_iter()
                        .enumerate()
                        .map(|(i, line)| {
                            let seq = world.next_seq + i as u64;
                            OutputLine::new(line, seq)
                        })
                        .collect();
                    world.next_seq += world.output_lines.len() as u64;
                    world.showing_splash = true;
                    world.scroll_offset = world.output_lines.len().saturating_sub(1);
                }
            }
        }
    }

    /// Advance backfill to the next world in the queue.
    /// Sets backfill_next so the main loop can send the request.
    fn backfill_advance_to_next(&mut self) {
        while let Some((world_idx, _total)) = self.backfill_queue.first().copied() {
            self.backfill_queue.remove(0);
            if let Some(world) = self.worlds.get(world_idx) {
                let oldest_seq = world.output_lines.first().map(|l| l.seq);
                if oldest_seq.is_some() {
                    self.backfill_next = Some((world_idx, oldest_seq));
                    return;
                }
            }
        }
        // Queue empty - backfill complete
        self.backfill_next = None;
    }

    /// Initialize backfill queue from InitialState worlds data.
    /// Called after init_from_initial_state() with the original WorldStateMsg data.
    fn init_backfill(&mut self, world_totals: &[(usize, usize)]) {
        self.backfill_queue.clear();
        self.backfill_next = None;
        // Current world first for best UX, then others
        let mut queue: Vec<(usize, usize)> = Vec::new();
        for &(idx, total) in world_totals {
            let received = self.worlds.get(idx).map(|w| w.output_lines.len()).unwrap_or(0);
            if total > received {
                if idx == self.current_world_index {
                    queue.insert(0, (idx, total));
                } else {
                    queue.push((idx, total));
                }
            }
        }
        self.backfill_queue = queue;
    }

    /// Find world index by name (case-insensitive), also checks reader_name for renamed worlds
    pub fn find_world_index(&self, name: &str) -> Option<usize> {
        self.worlds.iter().position(|w| {
            w.name.eq_ignore_ascii_case(name) ||
            w.reader_name.as_ref().is_some_and(|rn| rn.eq_ignore_ascii_case(name))
        })
    }

    pub fn switch_world(&mut self, index: usize) {
        if index < self.worlds.len() && index != self.current_world_index {
            // Stop media for old world (audio only plays for current world)
            self.stop_world_media(self.current_world_index);
            // Reset lines_since_pause for the old world if more-mode hasn't triggered
            let old_index = self.current_world_index;
            if self.worlds[old_index].pending_lines.is_empty() {
                self.worlds[old_index].lines_since_pause = 0;
            }
            // Clear new line indicators on the world we're leaving
            self.worlds[old_index].clear_new_line_indicators();
            // Track previous world for Alt+w fallback
            self.previous_world_index = Some(self.current_world_index);
            self.current_world_index = index;
            // Note: mark_seen() is NOT called here - lines are only marked seen when displayed
            // Mark output for redraw since we switched worlds
            self.needs_output_redraw = true;
            // Broadcast activity count since switching worlds changes which world is "current"
            // and activity_count() excludes the current world
            self.broadcast_activity();
            // Restart active media for the new world
            self.restart_world_media(index);
        }
    }

    /// Switch to a world with activity (Alt-w)
    /// Priority: 1) oldest pending output, 2) any unseen output, 3) previous world
    /// Returns true if switched, false if nowhere to switch
    fn switch_to_oldest_pending(&mut self) -> bool {
        // First, check for worlds with pending output (paused lines)
        let mut oldest_idx: Option<usize> = None;
        let mut oldest_time: Option<std::time::Instant> = None;

        for (idx, world) in self.worlds.iter().enumerate() {
            // Skip current world and worlds without pending output
            if idx == self.current_world_index || world.pending_lines.is_empty() {
                continue;
            }
            if let Some(pending_time) = world.pending_since {
                if oldest_time.is_none() || pending_time < oldest_time.unwrap() {
                    oldest_time = Some(pending_time);
                    oldest_idx = Some(idx);
                }
            }
        }

        if let Some(idx) = oldest_idx {
            self.switch_world(idx);
            return true;
        }

        // Second, check for worlds with unseen output (activity indicator)
        for (idx, world) in self.worlds.iter().enumerate() {
            if idx != self.current_world_index && world.unseen_lines > 0 {
                self.switch_world(idx);
                return true;
            }
        }

        // Third, fall back to previous world if it exists and is different
        if let Some(prev_idx) = self.previous_world_index {
            if prev_idx < self.worlds.len() && prev_idx != self.current_world_index {
                self.switch_world(prev_idx);
                return true;
            }
        }

        false
    }

    /// Calculate minimum output dimensions across all connected instances (console + web clients)
    /// Returns (width, height) or None if no instances are connected
    fn get_minimum_dimensions(&self) -> Option<(u16, u16)> {
        let mut min_width: Option<u16> = None;
        let mut min_height: Option<u16> = None;

        // Console dimensions (always present)
        if self.output_width > 0 && self.output_height > 0 {
            min_width = Some(self.output_width);
            min_height = Some(self.output_height);
        }

        // WebSocket client dimensions
        for state in self.ws_client_worlds.values() {
            if let Some((w, h)) = state.dimensions {
                if w > 0 && h > 0 {
                    min_width = Some(min_width.map_or(w, |mw| mw.min(w)));
                    min_height = Some(min_height.map_or(h, |mh| mh.min(h)));
                }
            }
        }

        match (min_width, min_height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        }
    }

    /// Send NAWS subnegotiation to a world if dimensions changed
    /// Returns true if NAWS was sent
    fn send_naws_if_changed(&mut self, world_index: usize) -> bool {
        if world_index >= self.worlds.len() {
            return false;
        }

        let world = &self.worlds[world_index];
        if !world.naws_enabled || !world.connected {
            return false;
        }

        if let Some((width, height)) = self.get_minimum_dimensions() {
            // Check if dimensions changed
            if self.worlds[world_index].naws_sent_size != Some((width, height)) {
                // Send NAWS subnegotiation
                if let Some(ref tx) = self.worlds[world_index].command_tx {
                    let naws_msg = build_naws_subnegotiation(width, height);
                    let _ = tx.try_send(WriteCommand::Raw(naws_msg));
                    self.worlds[world_index].naws_sent_size = Some((width, height));
                    return true;
                }
            }
        }
        false
    }

    /// Send NAWS updates to all connected worlds that have NAWS enabled
    fn send_naws_to_all_worlds(&mut self) {
        let world_count = self.worlds.len();
        for idx in 0..world_count {
            self.send_naws_if_changed(idx);
        }
    }

    fn next_world(&mut self) {
        // Build world info for shared function
        let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
            .map(|w| crate::util::WorldSwitchInfo {
                name: w.name.clone(),
                connected: w.connected,
                unseen_lines: w.unseen_lines,
                pending_lines: w.pending_lines.len(),
                first_unseen_at: w.first_unseen_at,
            })
            .collect();

        if let Some(next_idx) = crate::util::calculate_next_world(
            &world_info,
            self.current_world_index,
            self.settings.world_switch_mode,
        ) {
            self.switch_world(next_idx);
        }
    }

    fn prev_world(&mut self) {
        // Build world info for shared function
        let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
            .map(|w| crate::util::WorldSwitchInfo {
                name: w.name.clone(),
                connected: w.connected,
                unseen_lines: w.unseen_lines,
                pending_lines: w.pending_lines.len(),
                first_unseen_at: w.first_unseen_at,
            })
            .collect();

        if let Some(prev_idx) = crate::util::calculate_prev_world(
            &world_info,
            self.current_world_index,
            self.settings.world_switch_mode,
        ) {
            self.switch_world(prev_idx);
        }
    }

    fn find_world(&self, name: &str) -> Option<usize> {
        self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name))
    }

    fn find_or_create_world(&mut self, name: &str) -> usize {
        if let Some(idx) = self.find_world(name) {
            idx
        } else {
            self.worlds.push(World::new(name));
            self.worlds.len() - 1
        }
    }

    /// Calculate the next world index from a given starting point (without switching)
    fn calculate_next_world_from(&self, from_index: usize) -> Option<usize> {
        let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
            .map(|w| crate::util::WorldSwitchInfo {
                name: w.name.clone(),
                connected: w.connected,
                unseen_lines: w.unseen_lines,
                pending_lines: w.pending_lines.len(),
                first_unseen_at: w.first_unseen_at,
            })
            .collect();
        crate::util::calculate_next_world(&world_info, from_index, self.settings.world_switch_mode)
    }

    /// Calculate the previous world index from a given starting point (without switching)
    fn calculate_prev_world_from(&self, from_index: usize) -> Option<usize> {
        let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
            .map(|w| crate::util::WorldSwitchInfo {
                name: w.name.clone(),
                connected: w.connected,
                unseen_lines: w.unseen_lines,
                pending_lines: w.pending_lines.len(),
                first_unseen_at: w.first_unseen_at,
            })
            .collect();
        crate::util::calculate_prev_world(&world_info, from_index, self.settings.world_switch_mode)
    }

    /// Activity count excluding a specific world (for per-client counts)
    fn activity_count_excluding(&self, exclude_world: Option<usize>) -> usize {
        self.worlds
            .iter()
            .enumerate()
            .filter(|(i, w)| Some(*i) != exclude_world && w.has_activity())
            .count()
    }

    pub fn activity_count(&self) -> usize {
        self.activity_count_excluding(Some(self.current_world_index))
    }

    /// Send per-client activity counts to all WebSocket clients
    /// Each client may be viewing a different world, so each gets a personalized count
    pub(crate) fn broadcast_activity(&self) {
        #[cfg(test)]
        {
            // Log console's activity count for test assertions
            if let Ok(mut log) = self.ws_broadcast_log.lock() {
                log.push(WsMessage::ActivityUpdate {
                    count: self.activity_count(),
                });
            }
        }
        if let Some(ref server) = self.ws_server {
            if let Ok(clients_guard) = server.clients.try_read() {
                for client in clients_guard.values() {
                    if client.authenticated {
                        let count = self.activity_count_excluding(client.current_world);
                        let _ = client.tx.send(WsMessage::ActivityUpdate { count });
                    }
                }
            }
        }
    }

    /// Discard the initial "fake" world if it exists and is not connected.
    /// Called after first successful connection to a real world.
    fn discard_initial_world(&mut self) {
        // Find index of initial world that's not connected
        if let Some(idx) = self.worlds.iter().position(|w| w.is_initial_world && !w.connected) {
            // Don't discard if it's the only world or if it's the current world
            if self.worlds.len() > 1 && idx != self.current_world_index {
                self.worlds.remove(idx);
                // Adjust current_world_index if needed
                if self.current_world_index > idx {
                    self.current_world_index -= 1;
                }
                // Adjust previous_world_index if needed
                if let Some(prev) = self.previous_world_index {
                    if prev >= self.worlds.len() {
                        self.previous_world_index = Some(self.worlds.len().saturating_sub(1));
                    } else if prev > idx {
                        self.previous_world_index = Some(prev - 1);
                    }
                }
            }
        }
    }

    /// Ensure audio backend is initialized (lazy init on first use)
    fn ensure_audio(&mut self) {
        if matches!(self.audio_backend, audio::AudioBackend::None) {
            self.audio_backend = audio::init_audio();
        }
    }

    fn add_output(&mut self, text: &str) {
        let is_current = true;
        let settings = self.settings.clone();
        let output_height = self.output_height;
        let console_width = self.output_width;
        let world_idx = self.current_world_index;
        let output_width = self.min_viewer_width(world_idx)
            .map(|w| console_width.min(w as u16))
            .unwrap_or(console_width);
        // Ensure client-generated messages are complete lines (end with newline)
        let text_with_newline = if text.ends_with('\n') || text.is_empty() {
            text.to_string()
        } else {
            format!("{}\n", text)
        };
        self.current_world_mut()
            .add_output(&text_with_newline, is_current, &settings, output_height, output_width, false, false);

        // Broadcast to WebSocket clients viewing this world
        self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
            world_index: world_idx,
            data: text_with_newline,
            is_viewed: true,
            ts: current_timestamp_secs(),
            from_server: false,  // Client-generated message
            seq: 0,
            marked_new: false,
            flush: false, gagged: false,
        });

        // Mark output for redraw since we added content
        self.needs_output_redraw = true;
    }

    /// Add TF command output (does NOT get % prefix, but is client-generated so Ctrl+L filters it)
    fn add_tf_output(&mut self, text: &str) {
        let is_current = true;
        let settings = self.settings.clone();
        let output_height = self.output_height;
        let console_width = self.output_width;
        let world_idx = self.current_world_index;
        let output_width = self.min_viewer_width(world_idx)
            .map(|w| console_width.min(w as u16))
            .unwrap_or(console_width);
        let text_with_newline = if text.ends_with('\n') || text.is_empty() {
            text.to_string()
        } else {
            format!("{}\n", text)
        };
        self.current_world_mut()
            .add_output(&text_with_newline, is_current, &settings, output_height, output_width, false, false);
        self.needs_output_redraw = true;
    }

    /// Add output to a specific world by index (for background connection events)
    /// Also broadcasts to WebSocket clients viewing the world
    fn add_output_to_world(&mut self, world_idx: usize, text: &str) {
        if world_idx >= self.worlds.len() {
            return;
        }
        let is_current = world_idx == self.current_world_index || self.ws_client_viewing(world_idx);
        let settings = self.settings.clone();
        let output_height = self.output_height;
        let console_width = self.output_width;
        let output_width = self.min_viewer_width(world_idx)
            .map(|w| console_width.min(w as u16))
            .unwrap_or(console_width);
        let text_with_newline = if text.ends_with('\n') || text.is_empty() {
            text.to_string()
        } else {
            format!("{}\n", text)
        };
        self.worlds[world_idx]
            .add_output(&text_with_newline, is_current, &settings, output_height, output_width, false, false);

        // Broadcast to WebSocket clients viewing this world
        self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
            world_index: world_idx,
            data: text_with_newline,
            is_viewed: is_current,
            ts: current_timestamp_secs(),
            from_server: false,  // Client-generated message
            seq: 0,
            marked_new: false,
            flush: false, gagged: false,
        });

        if world_idx == self.current_world_index {
            self.needs_output_redraw = true;
        }
    }

    /// Handle GMCP Client.Media.* messages (MCMP protocol)
    /// When play_audio is false, only tracks state in active_media without spawning processes
    fn handle_gmcp_media(&mut self, world_idx: usize, package: &str, json_data: &str, play_audio: bool) {
        match package {
            "Client.Media.Default" => {
                // Store default URL for resolving relative media paths
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_data) {
                    if let Some(url) = parsed.get("url").and_then(|v| v.as_str()) {
                        self.worlds[world_idx].mcmp_default_url = url.to_string();
                    }
                }
            }
            "Client.Media.Play" | "Client.Media.Stop" | "Client.Media.Load" => {
                let action = package.rsplit('.').next().unwrap_or("Play").to_string();
                let default_url = self.worlds[world_idx].mcmp_default_url.clone();
                // Parse JSON unconditionally - we always track state in active_media
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_data) {
                    let name = parsed.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let url = parsed.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let media_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("sound");
                    let key = parsed.get("key").and_then(|v| v.as_str()).unwrap_or(name);
                    let volume = parsed.get("volume").and_then(|v| v.as_i64()).unwrap_or(50);
                    let loops = parsed.get("loops").and_then(|v| v.as_i64()).unwrap_or(1);
                    let cont = parsed.get("continue").and_then(|v| v.as_bool()).unwrap_or(false);

                    match action.as_str() {
                        "Play" => {
                            // Resolve full URL
                            let full_url = if !url.is_empty() {
                                format!("{}{}", url, name)
                            } else if !default_url.is_empty() {
                                format!("{}{}", default_url, name)
                            } else if !name.is_empty() {
                                name.to_string()
                            } else {
                                return;
                            };

                            // Always track looping media for restart on world switch/F9 toggle
                            if loops == -1 || loops > 1 {
                                self.worlds[world_idx].active_media.insert(key.to_string(), json_data.to_string());
                            }

                            // Only play audio when play_audio is true; lazy-init backend
                            if play_audio {
                                self.ensure_audio();
                                // For music type: stop previous music (unless continue)
                                if media_type == "music" {
                                    if cont
                                        && self.media_music_key.as_ref().map(|(_, k)| k.as_str()) == Some(key) {
                                        return;
                                    }
                                    if let Some((_, prev_key)) = self.media_music_key.take() {
                                        if let Some((_, mut handle)) = self.media_processes.remove(&prev_key) {
                                            handle.kill();
                                        }
                                    }
                                }

                                let cache_dir = self.media_cache_dir.clone();
                                let key_owned = key.to_string();
                                let vol = volume;
                                let loop_count = loops;
                                let is_music = media_type == "music";
                                let event_tx = self.event_tx.clone();

                                std::thread::spawn(move || {
                                    if let Some(cache_path) = audio::download_to_cache(&full_url, &cache_dir) {
                                        if let Some(tx) = event_tx {
                                            let _ = tx.blocking_send(AppEvent::MediaFileReady(
                                                world_idx, key_owned, cache_path, vol, loop_count, is_music,
                                            ));
                                        }
                                    }
                                });
                            }
                        }
                        "Stop" => {
                            // Always update tracking state and stop playback
                            if !key.is_empty() {
                                if let Some((_, mut handle)) = self.media_processes.remove(key) {
                                    handle.kill();
                                }
                                self.worlds[world_idx].active_media.remove(key);
                                if self.media_music_key.as_ref().map(|(_, k)| k.as_str()) == Some(key) {
                                    self.media_music_key = None;
                                }
                            } else if media_type == "music" {
                                if let Some((_, mk)) = self.media_music_key.take() {
                                    self.worlds[world_idx].active_media.remove(&mk);
                                    if let Some((_, mut handle)) = self.media_processes.remove(&mk) {
                                        handle.kill();
                                    }
                                }
                            }
                        }
                        "Load" => {
                            // Pre-cache only, no playback
                            let full_url = if !url.is_empty() {
                                format!("{}{}", url, name)
                            } else if !default_url.is_empty() {
                                format!("{}{}", default_url, name)
                            } else {
                                return;
                            };
                            let cache_dir = self.media_cache_dir.clone();
                            std::thread::spawn(move || {
                                let _ = audio::download_to_cache(&full_url, &cache_dir);
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {} // Other GMCP packages handled by hook system
        }
    }

    /// Stop all media processes for a specific world
    fn stop_world_media(&mut self, world_idx: usize) {
        // Stop media playback belonging to this world
        let keys_to_remove: Vec<String> = self.media_processes.iter()
            .filter(|(_, (idx, _))| *idx == world_idx)
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys_to_remove {
            if let Some((_, mut handle)) = self.media_processes.remove(&key) {
                handle.kill();
            }
        }
        // Clear music key if it belongs to this world
        if let Some((idx, _)) = &self.media_music_key {
            if *idx == world_idx {
                self.media_music_key = None;
            }
        }
    }

    /// Restart active media for a world (called when switching back to it)
    fn restart_world_media(&mut self, world_idx: usize) {
        if !self.worlds[world_idx].gmcp_user_enabled {
            return;
        }
        let default_url = self.worlds[world_idx].mcmp_default_url.clone();
        let plays: Vec<(String, String)> = self.worlds[world_idx].active_media.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (_key, json_data) in plays {
            // Restart console media (ffplay/mpv) - play_audio=true since restart implies enabled
            self.handle_gmcp_media(world_idx, "Client.Media.Play", &json_data, true);
            // Broadcast to web/GUI clients
            self.ws_broadcast_to_world(world_idx, WsMessage::McmpMedia {
                world_index: world_idx,
                action: "Play".to_string(),
                data: json_data,
                default_url: default_url.clone(),
            });
        }
    }

    /// Play ANSI music notes on the console.
    /// Stops any currently playing ANSI music first to prevent overlapping playback.
    fn play_ansi_music_console(&mut self, notes: &[crate::ansi_music::MusicNote]) {
        // Stop any currently playing ANSI music
        if let Some(mut handle) = self.ansi_music_handle.take() {
            handle.stop();
        }

        let wav_data = generate_wav_from_notes(notes);
        self.ansi_music_counter = self.ansi_music_counter.wrapping_add(1);
        self.ensure_audio();

        if let Some(handle) = audio::play_wav_memory(
            &self.audio_backend,
            wav_data,
            &self.media_cache_dir,
            self.ansi_music_counter,
        ) {
            self.ansi_music_handle = Some(handle);
        }
    }

    /// Broadcast a message to all authenticated WebSocket clients and the embedded GUI (if any)
    pub(crate) fn ws_broadcast(&self, msg: WsMessage) {
        #[cfg(test)]
        {
            if let Ok(mut log) = self.ws_broadcast_log.lock() {
                log.push(msg.clone());
            }
        }
        // Send to embedded GUI channel (master GUI mode)
        if let Some(ref tx) = self.gui_tx {
            let _ = tx.send(msg.clone());
            // Wake the GUI for immediate repaint
            if let Some(ref repaint) = self.gui_repaint {
                repaint();
            }
        }
        // Broadcast to WebSocket server
        if let Some(ref server) = self.ws_server {
            // Use synchronous try_read to avoid spawning a task
            if let Ok(clients_guard) = server.clients.try_read() {
                let mut sent_count = 0;
                for client in clients_guard.values() {
                    if client.authenticated {
                        let _ = client.tx.send(msg.clone());
                        sent_count += 1;
                    }
                }
                // Log if we didn't send to anyone (for debugging)
                if is_debug_enabled() && sent_count == 0 && !clients_guard.is_empty() {
                    output_debug_log(&format!("ws_broadcast: {} clients connected, 0 authenticated", clients_guard.len()));
                }
            } else {
                // Lock contention - fall back to async broadcast
                let clients = server.clients.clone();
                tokio::spawn(async move {
                    let clients_guard = clients.read().await;
                    for client in clients_guard.values() {
                        if client.authenticated {
                            let _ = client.tx.send(msg.clone());
                        }
                    }
                });
            }
        }
    }

    /// Send a message to a specific WebSocket client (or embedded GUI if client_id == 0)
    fn ws_send_to_client(&self, client_id: u64, msg: WsMessage) {
        // client_id 0 is the embedded GUI in master GUI mode
        if client_id == 0 {
            if let Some(ref tx) = self.gui_tx {
                let _ = tx.send(msg);
                if let Some(ref repaint) = self.gui_repaint {
                    repaint();
                }
            }
            return;
        }
        if let Some(ref server) = self.ws_server {
            let clients = server.clients.clone();
            tokio::spawn(async move {
                let clients_guard = clients.read().await;
                if let Some(client) = clients_guard.get(&client_id) {
                    let _ = client.tx.send(msg);
                }
            });
        }
    }

    /// Send active media for a world to a specific client (for world switch restart)
    fn ws_send_active_media_to_client(&self, client_id: u64, world_idx: usize) {
        if world_idx >= self.worlds.len() || !self.worlds[world_idx].gmcp_user_enabled {
            return;
        }
        let default_url = self.worlds[world_idx].mcmp_default_url.clone();
        for json_data in self.worlds[world_idx].active_media.values() {
            self.ws_send_to_client(client_id, WsMessage::McmpMedia {
                world_index: world_idx,
                action: "Play".to_string(),
                data: json_data.clone(),
                default_url: default_url.clone(),
            });
        }
    }

    /// Send InitialState to a client AND mark it as having received InitialState.
    /// Uses try_write first for immediate marking (avoids race where broadcasts
    /// are dropped before the spawned task runs). Falls back to spawned task
    /// if the lock is contended.
    fn ws_send_initial_state_and_mark(&self, client_id: u64, msg: WsMessage) {
        // client_id 0 is the embedded GUI - send directly, no tracking needed
        if client_id == 0 {
            if let Some(ref tx) = self.gui_tx {
                let _ = tx.send(msg);
                if let Some(ref repaint) = self.gui_repaint {
                    repaint();
                }
            }
            return;
        }
        if let Some(ref server) = self.ws_server {
            // Try synchronous write first — this avoids the race where broadcasts
            // are skipped because received_initial_state hasn't been set yet
            if let Ok(mut clients_guard) = server.clients.try_write() {
                if let Some(client) = clients_guard.get_mut(&client_id) {
                    let _ = client.tx.send(msg);
                    client.received_initial_state = true;
                }
            } else {
                // Lock contended — fall back to spawned task
                let clients = server.clients.clone();
                tokio::spawn(async move {
                    let mut clients_guard = clients.write().await;
                    if let Some(client) = clients_guard.get_mut(&client_id) {
                        let _ = client.tx.send(msg);
                        client.received_initial_state = true;
                    }
                });
            }
        }
    }

    /// Set the client type for a connected WebSocket client
    fn ws_set_client_type(&self, client_id: u64, client_type: websocket::RemoteClientType) {
        if client_id == 0 {
            return;  // Embedded GUI doesn't need this
        }
        if let Some(ref server) = self.ws_server {
            server.set_client_type(client_id, client_type);
        }
    }

    /// Set the current world being viewed by a WebSocket client
    fn ws_set_client_world(&self, client_id: u64, world_index: Option<usize>) {
        if client_id == 0 {
            return;  // Embedded GUI tracks its own world
        }
        if let Some(ref server) = self.ws_server {
            server.set_client_world(client_id, world_index);
        }
    }

    /// Set the authenticated status for a WebSocket client
    fn ws_set_client_authenticated(&self, client_id: u64, authenticated: bool) {
        if let Some(ref server) = self.ws_server {
            server.set_client_authenticated(client_id, authenticated);
        }
    }

    /// Get the client type for a connected WebSocket client
    fn ws_get_client_type(&self, client_id: u64) -> Option<websocket::RemoteClientType> {
        if client_id == 0 {
            return Some(websocket::RemoteClientType::RemoteGUI);  // Embedded GUI
        }
        if let Some(ref server) = self.ws_server {
            server.get_client_type(client_id)
        } else {
            None
        }
    }

    /// Broadcast a message only to clients viewing a specific world
    fn ws_broadcast_to_world(&self, world_index: usize, msg: WsMessage) {
        #[cfg(test)]
        {
            if let Ok(mut log) = self.ws_broadcast_log.lock() {
                log.push(msg.clone());
            }
        }
        // Send to embedded GUI if it's viewing this world
        if let Some(ref tx) = self.gui_tx {
            // For embedded GUI, always send (it tracks its own current world)
            let _ = tx.send(msg.clone());
        }
        // Broadcast to WebSocket clients viewing this world
        if let Some(ref server) = self.ws_server {
            server.broadcast_to_world_viewers(world_index, msg);
        }
    }

    /// Check if any WS client is currently viewing a specific world
    fn ws_client_viewing(&self, world_index: usize) -> bool {
        self.ws_client_worlds.values().any(|v| v.world_index == world_index)
    }

    /// Get the minimum visible lines among all viewers of a world (for more-mode threshold)
    /// Returns None if no WS clients are viewing the world (use console output_height)
    fn min_viewer_lines(&self, world_index: usize) -> Option<usize> {
        let ws_min = self.ws_client_worlds
            .values()
            .filter(|v| v.world_index == world_index && v.visible_lines > 0)
            .map(|v| v.visible_lines)
            .min();
        ws_min
    }

    /// Get the minimum visible columns among WS clients viewing a world (for wrap width)
    /// Returns None if no WS clients have reported their width
    fn min_viewer_width(&self, world_index: usize) -> Option<usize> {
        self.ws_client_worlds
            .values()
            .filter(|v| v.world_index == world_index && v.visible_columns > 0)
            .map(|v| v.visible_columns)
            .min()
    }

    /// Process incoming server data - shared logic for both console and daemon modes
    /// Returns commands that need to be executed (for trigger processing)
    pub fn process_server_data(
        &mut self,
        world_idx: usize,
        bytes: &[u8],
        console_height: u16,
        console_width: u16,
        is_daemon_mode: bool,
    ) -> Vec<String> {
        self.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());

        // FANSI client detection: check for "Detecting client..." within 2s window
        if let Some(deadline) = self.worlds[world_idx].fansi_detect_until {
            if std::time::Instant::now() < deadline {
                let peek = self.worlds[world_idx].effective_encoding().decode(bytes);
                for line in peek.lines() {
                    let stripped = crate::util::strip_ansi_codes(line).trim().to_string();
                    if stripped == "Detecting client..." {
                        if let Some(tx) = &self.worlds[world_idx].command_tx {
                            let _ = tx.try_send(WriteCommand::Text("CLIENT WEBCLIENT2".to_string()));
                            let _ = tx.try_send(WriteCommand::Text(String::new()));
                        }
                        // Delay login by 2s to let the MUD process the client response
                        self.worlds[world_idx].fansi_detect_until = Some(std::time::Instant::now() + Duration::from_secs(2));
                        break;
                    }
                }
            } else {
                // Detection window expired — send deferred login now
                if let Some(login_cmd) = self.worlds[world_idx].fansi_login_pending.take() {
                    if let Some(tx) = &self.worlds[world_idx].command_tx {
                        let _ = tx.try_send(WriteCommand::Text(login_cmd));
                    }
                }
                self.worlds[world_idx].fansi_detect_until = None;
            }
        }

        // Consider "current" if console OR any web/GUI client is viewing this world
        let is_current = world_idx == self.current_world_index || self.ws_client_viewing(world_idx);
        let decoded_data = self.worlds[world_idx].effective_encoding().decode(bytes);

        // Extract ANSI music sequences FIRST, before any other processing
        let (data, music_sequences) = if self.settings.ansi_music_enabled {
            ansi_music::extract_music(&decoded_data)
        } else {
            (decoded_data, Vec::new())
        };

        // Broadcast music to WebSocket clients (web/GUI play audio)
        for notes in &music_sequences {
            self.ws_broadcast(WsMessage::AnsiMusic {
                world_index: world_idx,
                notes: notes.clone(),
            });
        }

        // Console ANSI music playback via system player
        // Concatenate all sequences into one to avoid rapid kill/restart
        if !music_sequences.is_empty() {
            let all_notes: Vec<crate::ansi_music::MusicNote> = music_sequences.iter().flatten().cloned().collect();
            self.play_ansi_music_console(&all_notes);
        }

        let world_name_for_triggers = self.worlds[world_idx].name.clone();
        let actions = self.settings.actions.clone();

        // Combine with any partial line from previous data chunk
        let had_trigger_partial = !self.worlds[world_idx].trigger_partial_line.is_empty();
        let combined_data = if had_trigger_partial {
            let mut s = std::mem::take(&mut self.worlds[world_idx].trigger_partial_line);
            s.push_str(&data);
            s
        } else {
            data.clone()
        };

        // Process action triggers on complete lines
        // Track lines with gagged flag and highlight color: (line, is_gagged, highlight_color)
        let mut processed_lines: Vec<(&str, bool, Option<String>)> = Vec::new();
        let mut commands_to_execute: Vec<String> = Vec::new();
        let mut tf_commands_to_execute: Vec<String> = Vec::new();
        let mut tf_messages: Vec<String> = Vec::new();
        let ends_with_newline = combined_data.ends_with('\n');
        let lines: Vec<&str> = combined_data.lines().collect();
        let line_count = lines.len();
        let mut has_partial = false;
        // Use persistent flag to track idler filtering across TCP packets
        let mut just_filtered_idler = self.worlds[world_idx].just_filtered_idler;


        for (i, line) in lines.iter().enumerate() {
            let is_last = i == line_count - 1;
            let is_partial = is_last && !ends_with_newline;

            // Filter out keep-alive idler message lines (only for Custom/Generic keep-alive types)
            let uses_idler_keepalive = matches!(
                self.worlds[world_idx].settings.keep_alive_type,
                KeepAliveType::Custom | KeepAliveType::Generic
            );
            if uses_idler_keepalive && line.contains("###_idler_message_") && line.contains("_###") {
                just_filtered_idler = true;
                continue;
            }

            // Filter blank lines that immediately follow an idler message
            if just_filtered_idler && is_visually_empty(line) {
                just_filtered_idler = false; // Reset after filtering the blank
                continue;
            }
            just_filtered_idler = false;

            // Check triggers on complete lines only
            if is_partial {
                // Store partial line for next chunk - don't process yet
                self.worlds[world_idx].trigger_partial_line = line.to_string();
                has_partial = true;
            } else {
                // Watchdog/watchname spam detection (before triggers)
                let mut watchdog_gagged = false;
                let stripped = strip_ansi_for_watchdog(line);
                if self.tf_engine.watchdog_enabled {
                    let n1 = self.tf_engine.watchdog_n1;
                    let n2 = self.tf_engine.watchdog_n2;
                    let count = self.worlds[world_idx].watchdog_history.iter()
                        .filter(|h| *h == &stripped)
                        .count();
                    if count >= n1 {
                        watchdog_gagged = true;
                    }
                    self.worlds[world_idx].watchdog_history.push_back(stripped.clone());
                    while self.worlds[world_idx].watchdog_history.len() > n2 {
                        self.worlds[world_idx].watchdog_history.pop_front();
                    }
                }
                if self.tf_engine.watchname_enabled {
                    let n1 = self.tf_engine.watchname_n1;
                    let n2 = self.tf_engine.watchname_n2;
                    let first_word = stripped.split_whitespace().next().unwrap_or("").to_string();
                    if !first_word.is_empty() {
                        let count = self.worlds[world_idx].watchname_history.iter()
                            .filter(|h| *h == &first_word)
                            .count();
                        if count >= n1 {
                            watchdog_gagged = true;
                        }
                    }
                    self.worlds[world_idx].watchname_history.push_back(first_word);
                    while self.worlds[world_idx].watchname_history.len() > n2 {
                        self.worlds[world_idx].watchname_history.pop_front();
                    }
                }

                let tr = process_triggers(line, &world_name_for_triggers, &actions, &mut self.tf_engine);
                commands_to_execute.extend(tr.send_commands);
                tf_commands_to_execute.extend(tr.clay_commands);
                tf_messages.extend(tr.messages);
                processed_lines.push((line, tr.is_gagged || watchdog_gagged, tr.highlight_color));
            }
        }

        // BAMF portal detection: #### Please reconnect to name@addr (host) port NNN ####
        let bamf_val = self.tf_engine.get_var("bamf").map(|v| v.to_string_value()).unwrap_or_default();
        if bamf_val == "1" || bamf_val == "old" {
            for (line, _, _) in &processed_lines {
                if let Some(portal) = parse_bamf_portal(line) {
                    let world_name = self.worlds[world_idx].name.clone();
                    let user = self.worlds[world_idx].settings.user.clone();
                    let password = self.worlds[world_idx].settings.password.clone();
                    self.add_tf_output(&format!("Portal detected: {} ({}:{})", portal.name, portal.host, portal.port));
                    if bamf_val == "1" {
                        // Disconnect current world first
                        commands_to_execute.push(format!("/disconnect {}", world_name));
                    }
                    // Create/update the world and connect
                    commands_to_execute.push(format!("/addworld {} {} {}", portal.name, portal.host, portal.port));
                    commands_to_execute.push(format!("/worlds {}", portal.name));
                    // If login flag is set, auto-login with current world's credentials
                    let login_val = self.tf_engine.get_var("login").map(|v| v.to_string_value()).unwrap_or_default();
                    if (login_val == "1" || login_val == "on") && !user.is_empty() && !password.is_empty() {
                        commands_to_execute.push(format!("/send connect {} {}", user, password));
                    }
                    break; // Only handle first portal per chunk
                }
            }
        }

        // Save the idler filter state for next packet
        self.worlds[world_idx].just_filtered_idler = just_filtered_idler;

        // If we have a partial line and world uses WONT ECHO prompts, start timeout
        if has_partial && self.worlds[world_idx].prompt.is_empty()
            && self.worlds[world_idx].uses_wont_echo_prompt {
            self.worlds[world_idx].wont_echo_time = Some(std::time::Instant::now());
        }

        // Separate gagged and non-gagged lines, tracking highlight colors
        let non_gagged_lines: Vec<(&str, Option<String>)> = processed_lines.iter()
            .filter(|(_, gagged, _)| !gagged)
            .map(|(line, _, highlight)| (*line, highlight.clone()))
            .collect();
        let gagged_lines: Vec<(&str, Option<String>)> = processed_lines.iter()
            .filter(|(_, gagged, _)| *gagged)
            .map(|(line, _, highlight)| (*line, highlight.clone()))
            .collect();
        // Create a map of line content to highlight color for non-gagged lines
        let highlight_map: std::collections::HashMap<String, Option<String>> = non_gagged_lines.iter()
            .filter(|(_, hl)| hl.is_some())
            .map(|(line, hl)| (line.to_string(), hl.clone()))
            .collect();

        // Rebuild data for non-gagged lines
        // Add trailing newline if original ended with newline OR if we have a partial
        // (because a partial means there was a newline before it that we need to preserve)
        let filtered_data = if non_gagged_lines.is_empty() {
            String::new()
        } else {
            let lines_only: Vec<&str> = non_gagged_lines.iter().map(|(line, _)| *line).collect();
            let mut result = lines_only.join("\n");
            if ends_with_newline || has_partial {
                result.push('\n');
            }
            result
        };

        // Add non-gagged output to world
        if !filtered_data.is_empty() {
            let settings = self.settings.clone();

            // Calculate minimum visible lines among all viewers for synchronized more-mode
            // Console counts as a viewer if it's viewing this world (unless daemon mode)
            let console_viewing = !is_daemon_mode && world_idx == self.current_world_index;
            let ws_min = self.min_viewer_lines(world_idx);
            let output_height = match (console_viewing, ws_min) {
                (true, Some(ws)) => console_height.min(ws as u16),
                (true, None) => console_height,
                (false, Some(ws)) => ws as u16,
                (false, None) => {
                    if is_daemon_mode {
                        // Daemon mode with no viewers - use min of all WS clients or default
                        self.ws_client_worlds.values()
                            .map(|s| s.visible_lines)
                            .filter(|&v| v > 0)
                            .min()
                            .unwrap_or(24) as u16
                    } else {
                        // Non-current world in console mode: use console height
                        // so more-mode triggers at the right point for when user switches
                        console_height
                    }
                }
            };

            // Calculate minimum visible columns among all viewers for wrap width
            let ws_min_width = self.min_viewer_width(world_idx);
            let output_width = match (console_viewing, ws_min_width) {
                (true, Some(ws_w)) => console_width.min(ws_w as u16),
                (true, None) => console_width,
                (false, Some(ws_w)) => ws_w as u16,
                (false, None) => {
                    if is_daemon_mode {
                        self.ws_client_worlds.values()
                            .map(|s| s.visible_columns)
                            .filter(|&v| v > 0)
                            .min()
                            .unwrap_or(200) as u16
                    } else {
                        console_width
                    }
                }
            };

            // Track pending count before add_output for synchronized more-mode
            let pending_before = self.worlds[world_idx].pending_lines.len();
            let output_before = self.worlds[world_idx].output_lines.len();
            let was_showing_splash = self.worlds[world_idx].showing_splash;

            // Track partial line state before add_output:
            // If there's a partial line in output_lines, we shouldn't have broadcast it yet
            // (we exclude partials from broadcasts). When it's completed, we'll include it.
            let had_partial_in_output = !self.worlds[world_idx].partial_line.is_empty()
                && !self.worlds[world_idx].partial_in_pending;

            self.worlds[world_idx].add_output(&filtered_data, is_current, &settings, output_height, output_width, true, true);

            // Check if splash was cleared (output_lines was reset)
            let splash_was_cleared = was_showing_splash && !self.worlds[world_idx].showing_splash;

            // Check partial state after add_output
            let has_partial_in_output = !self.worlds[world_idx].partial_line.is_empty()
                && !self.worlds[world_idx].partial_in_pending;

            // Calculate what went where
            let pending_after = self.worlds[world_idx].pending_lines.len();
            let output_after = self.worlds[world_idx].output_lines.len();


            // Calculate broadcast boundaries, accounting for partial lines:
            // - Partial lines are NOT broadcast (clients only get complete lines)
            // - When a partial is completed (updated in-place), include it in the broadcast
            // This fixes the bug where partial line updates were never sent to remote clients
            let (lines_to_output, skip_count) = if splash_was_cleared {
                // Broadcast all output lines since buffer was cleared
                (output_after, 0)
            } else {
                let mut skip = output_before;
                let mut count = output_after.saturating_sub(output_before);

                // If we had a partial in output_lines before, it wasn't broadcast yet.
                // Now that add_output may have completed it (or added new lines after it),
                // include it in this broadcast by moving skip back one position.
                if had_partial_in_output && skip > 0 {
                    skip -= 1;
                    count = output_after.saturating_sub(skip);
                }

                // If there's currently a partial line at the end of output_lines,
                // exclude it from this broadcast (it'll be sent when completed)
                if has_partial_in_output && count > 0 {
                    count -= 1;
                }

                (count, skip)
            };
            let lines_to_pending = pending_after.saturating_sub(pending_before);

            // Apply highlight colors to newly added lines
            if !highlight_map.is_empty() {
                // Apply to output_lines
                for line in self.worlds[world_idx].output_lines.iter_mut().skip(output_before) {
                    let plain_text = strip_ansi_codes(&line.text);
                    if let Some(hl) = highlight_map.get(&plain_text) {
                        line.highlight_color = hl.clone();
                    }
                }
                // Apply to pending_lines
                for line in self.worlds[world_idx].pending_lines.iter_mut().skip(pending_before) {
                    let plain_text = strip_ansi_codes(&line.text);
                    if let Some(hl) = highlight_map.get(&plain_text) {
                        line.highlight_color = hl.clone();
                    }
                }
            }

            // Mark output for redraw whenever output changes
            self.needs_output_redraw = true;

            // For synchronized more-mode: only broadcast lines that went to output_lines
            // Lines that went to pending_lines will be broadcast when released
            // Only send to clients viewing this world (Phase 2 output routing)
            if lines_to_output > 0 {
                // Get the seq of the first line being broadcast (for client-side dedup)
                let first_seq = self.worlds[world_idx]
                    .output_lines
                    .get(skip_count)
                    .map(|l| l.seq)
                    .unwrap_or(0);

                // Get only the lines that went to output_lines
                let output_lines_to_broadcast: Vec<String> = self.worlds[world_idx]
                    .output_lines
                    .iter()
                    .skip(skip_count)
                    .take(lines_to_output)
                    .map(|line| line.text.replace('\r', ""))
                    .collect();
                let ws_data = output_lines_to_broadcast.join("\n") + "\n";

                // Route output only to clients viewing this world
                // If splash was cleared, set flush flag so client clears buffer atomically
                // before appending new lines (avoids race with separate WorldFlushed message)
                self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                    world_index: world_idx,
                    data: ws_data,
                    is_viewed: true,  // Clients viewing this world consider it "viewed"
                    ts: current_timestamp_secs(),
                    from_server: true,
                    seq: first_seq,
                    marked_new: !is_current,
                    flush: splash_was_cleared, gagged: false,
                });
            } else if splash_was_cleared {
                // Splash cleared but no output lines to send — still need to flush client buffers
                self.ws_broadcast(WsMessage::WorldFlushed { world_index: world_idx });
            }

            // Broadcast pending count update if it changed (for synchronized more-mode indicator)
            // Use filtered broadcast to skip clients that received pending in InitialState
            if lines_to_pending > 0 || pending_after != pending_before {
                self.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: pending_after });
            }

            // Broadcast updated unseen count so all clients stay in sync
            let unseen_count = self.worlds[world_idx].unseen_lines;
            if unseen_count > 0 {
                self.ws_broadcast(WsMessage::UnseenUpdate {
                    world_index: world_idx,
                    count: unseen_count,
                });
            }

            // Broadcast activity count to keep all clients in sync
            self.broadcast_activity();

            // Text-to-speech: speak non-gagged MUD output when TTS is enabled and not muted
            // Only speak output from the currently visible world
            if self.settings.tts_mode != tts::TtsMode::Off && !self.settings.tts_muted
                && (world_idx == self.current_world_index || self.ws_client_viewing(world_idx))
            {
                // Filter lines based on speak mode
                let lines_to_speak: Vec<&str> = if self.settings.tts_speak_mode == tts::TtsSpeakMode::Limit {
                    non_gagged_lines.iter()
                        .filter(|(line, _)| {
                            tts::should_speak(line, &mut self.worlds[world_idx].tts_speaker_whitelist)
                        })
                        .map(|(line, _)| *line)
                        .collect()
                } else {
                    non_gagged_lines.iter()
                        .map(|(line, _)| *line)
                        .collect()
                };
                // Strip ANSI codes and MUD tags — only speak what's displayed on screen
                let speak_text = lines_to_speak.iter()
                    .map(|line| {
                        let stripped = strip_ansi_codes(line);
                        crate::util::strip_mud_tag(&stripped)
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let clean_text = speak_text.trim();
                if !clean_text.is_empty() {
                    // Console: speak via local TTS subprocess (queued — waits for previous to finish)
                    tts::speak(&self.tts_backend, clean_text, self.settings.tts_mode);
                    // Web/GUI: broadcast ServerSpeak to WebSocket clients
                    self.ws_broadcast(WsMessage::ServerSpeak {
                        text: clean_text.to_string(),
                        world_index: world_idx,
                    });
                }
            }
        }

        // Add gagged lines to output (they'll only show with F2)
        // Also broadcast to WebSocket clients so F2 works on all interfaces
        let was_at_bottom = self.worlds[world_idx].is_at_bottom();
        for (line, highlight) in gagged_lines {
            let seq = self.worlds[world_idx].next_seq;
            self.worlds[world_idx].next_seq += 1;
            let mut output_line = OutputLine::new_gagged(line.to_string(), seq);
            output_line.highlight_color = highlight;
            self.worlds[world_idx].output_lines.push(output_line);
            // Broadcast gagged line to WebSocket clients
            let ws_data = line.to_string().replace('\r', "") + "\n";
            self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                world_index: world_idx,
                data: ws_data,
                is_viewed: is_current,
                ts: current_timestamp_secs(),
                from_server: true,
                seq,
                marked_new: !is_current,
                flush: false,
                gagged: true,
            });
        }
        // Keep scroll at bottom if we were already there, even when paused.
        // Without this, gagged lines shift output_lines.len() ahead of scroll_offset,
        // causing is_at_bottom() to return false and Tab to scroll instead of releasing.
        // Preserve visual_line_offset since gagged lines don't affect visible display.
        if was_at_bottom {
            let saved_vlo = self.worlds[world_idx].visual_line_offset;
            self.worlds[world_idx].scroll_to_bottom();
            self.worlds[world_idx].visual_line_offset = saved_vlo;
        }

        // Display TF trigger messages (from /echo commands)
        for msg in tf_messages {
            self.add_tf_output(&msg);
        }

        // Merge TF commands into commands_to_execute
        commands_to_execute.extend(tf_commands_to_execute);
        commands_to_execute
    }

    // ========================================================================
    // Extracted event handlers (shared across headless, console, and batch drain loops)
    // ========================================================================

    /// Handle Disconnected event for a world.
    fn handle_disconnected(&mut self, world_idx: usize) {
        // Fire TF DISCONNECT hook before cleaning up
        let hook_result = tf::bridge::fire_event(&mut self.tf_engine, tf::TfHookEvent::Disconnect);
        for cmd in hook_result.clay_commands {
            let _ = self.tf_engine.execute(&cmd);
        }

        // Push prompt to output before clearing
        if !self.worlds[world_idx].prompt.is_empty() {
            let prompt_text = self.worlds[world_idx].prompt.trim().to_string();
            let seq = self.worlds[world_idx].next_seq;
            self.worlds[world_idx].next_seq += 1;
            self.worlds[world_idx].output_lines.push(OutputLine::new(prompt_text, seq));
        }
        self.worlds[world_idx].clear_connection_state(true, true);
        // Show disconnection message
        let seq = self.worlds[world_idx].next_seq;
        self.worlds[world_idx].next_seq += 1;
        let disconnect_msg = OutputLine::new_client("Disconnected.".to_string(), seq);
        self.worlds[world_idx].output_lines.push(disconnect_msg.clone());
        self.worlds[world_idx].scroll_to_bottom();

        // If this is not the current world, increment unseen_lines for activity indicator
        if world_idx != self.current_world_index {
            if self.worlds[world_idx].unseen_lines == 0 {
                self.worlds[world_idx].first_unseen_at = Some(std::time::Instant::now());
            }
            self.worlds[world_idx].unseen_lines += 1;
        }

        // Mark output for redraw if this is the current world
        if world_idx == self.current_world_index {
            self.needs_output_redraw = true;
        }

        // Broadcast disconnect message to WebSocket clients viewing this world
        self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
            world_index: world_idx,
            data: "Disconnected.\n".to_string(),
            is_viewed: true,
            ts: disconnect_msg.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            from_server: false,
            seq,
            marked_new: false,
            flush: false, gagged: false,
        });
        self.ws_broadcast(WsMessage::WorldDisconnected { world_index: world_idx });

        // Schedule auto-reconnect if configured and world was previously connected
        let secs = self.worlds[world_idx].settings.auto_reconnect_secs;
        if secs > 0 && self.worlds[world_idx].was_connected {
            self.worlds[world_idx].reconnect_at = Some(
                std::time::Instant::now() + std::time::Duration::from_secs(secs as u64)
            );
            let seq = self.worlds[world_idx].next_seq;
            self.worlds[world_idx].next_seq += 1;
            let msg = OutputLine::new_client(format!("Reconnecting in {} seconds...", secs), seq);
            self.worlds[world_idx].output_lines.push(msg.clone());
            self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                world_index: world_idx,
                data: format!("Reconnecting in {} seconds...\n", secs),
                is_viewed: true,
                ts: msg.timestamp.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                from_server: false,
                seq,
                marked_new: false,
                flush: false, gagged: false,
            });
        }
    }

    /// Returns the earliest scheduled reconnect time across all worlds, if any.
    fn next_reconnect_instant(&self) -> Option<std::time::Instant> {
        self.worlds.iter().filter_map(|w| w.reconnect_at).min()
    }

    /// Schedule immediate reconnection for all worlds with auto_reconnect_on_web enabled
    /// that are currently disconnected and have been connected before.
    /// Returns true if any world was scheduled (caller should re-arm the reconnect timer).
    fn trigger_web_reconnects(&mut self) -> bool {
        let mut triggered = false;
        for i in 0..self.worlds.len() {
            if self.worlds[i].settings.auto_reconnect_on_web
                && !self.worlds[i].connected
                && self.worlds[i].settings.has_connection_settings()
                && self.worlds[i].reconnect_at.is_none()
            {
                self.worlds[i].reconnect_at = Some(std::time::Instant::now());
                self.add_output_to_world(i, "Web client connected - reconnecting...");
                triggered = true;
            }
        }
        triggered
    }

    /// Handle TelnetDetected event.
    fn handle_telnet_detected(&mut self, world_idx: usize) {
        if !self.worlds[world_idx].telnet_mode {
            self.worlds[world_idx].telnet_mode = true;
        }
    }

    /// Handle WontEchoSeen event.
    fn handle_wont_echo_seen(&mut self, world_idx: usize) {
        if !self.worlds[world_idx].uses_wont_echo_prompt {
            self.worlds[world_idx].uses_wont_echo_prompt = true;
        }
    }

    /// Handle NawsRequested event.
    fn handle_naws_requested(&mut self, world_idx: usize) {
        self.worlds[world_idx].naws_enabled = true;
        self.send_naws_if_changed(world_idx);
    }

    /// Handle TtypeRequested event.
    fn handle_ttype_requested(&mut self, world_idx: usize) {
        let term_type = std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string());
        if let Some(ref tx) = self.worlds[world_idx].command_tx {
            let ttype_response = build_ttype_response(&term_type);
            let _ = tx.try_send(WriteCommand::Raw(ttype_response));
        }
    }

    /// Handle CharsetRequested event (RFC 2066 TELNET CHARSET).
    /// Selects the best charset from the offered list that Clay supports.
    fn handle_charset_requested(&mut self, world_idx: usize, charsets: &[String]) {
        // Priority order: UTF-8 > Latin1 > Fansi
        // First pass: look for UTF-8 (highest capability)
        // Second pass: accept first supported charset from offered list
        let mut best: Option<(Encoding, &str)> = None;
        for name in charsets {
            if let Some(enc) = Encoding::from_iana_name(name) {
                match enc {
                    Encoding::Utf8 => {
                        // UTF-8 is always preferred — accept immediately
                        best = Some((enc, "UTF-8"));
                        break;
                    }
                    _ => {
                        if best.is_none() {
                            best = Some((enc, match enc {
                                Encoding::Latin1 => "ISO-8859-1",
                                Encoding::Fansi => "IBM437",
                                Encoding::Utf8 => unreachable!(),
                            }));
                        }
                    }
                }
            }
        }

        if let Some(ref tx) = self.worlds[world_idx].command_tx {
            if let Some((enc, iana_name)) = best {
                let response = build_charset_accepted(iana_name);
                let _ = tx.try_send(WriteCommand::Raw(response));
                self.worlds[world_idx].negotiated_encoding = Some(enc);
            } else {
                let response = build_charset_rejected();
                let _ = tx.try_send(WriteCommand::Raw(response));
            }
        }
    }

    /// Handle Prompt event.
    fn handle_prompt(&mut self, world_idx: usize, prompt_bytes: &[u8]) {
        self.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());
        let encoding = self.worlds[world_idx].effective_encoding();
        let prompt_text = encoding.decode(prompt_bytes);
        let prompt_normalized = crate::util::normalize_prompt(&prompt_text);

        // If world is not connected, display prompt as output instead of input area
        if !self.worlds[world_idx].connected {
            let seq = self.worlds[world_idx].next_seq;
            self.worlds[world_idx].next_seq += 1;
            self.worlds[world_idx].output_lines.push(OutputLine::new(prompt_normalized.trim().to_string(), seq));
            self.worlds[world_idx].scroll_to_bottom();
            self.worlds[world_idx].prompt.clear();
            if world_idx == self.current_world_index {
                self.needs_output_redraw = true;
            }
            return;
        }

        self.worlds[world_idx].prompt = prompt_normalized.clone();
        self.ws_broadcast(WsMessage::PromptUpdate {
            world_index: world_idx,
            prompt: prompt_normalized,
        });

        let world = &mut self.worlds[world_idx];
        world.prompt_count += 1;

        // Skip auto-login if flag is set (from /worlds -l)
        if world.skip_auto_login {
            return;
        }

        let auto_type = world.settings.auto_connect_type;
        let user = world.settings.user.clone();
        let password = world.settings.password.clone();
        let prompt_num = world.prompt_count;

        if !user.is_empty() && !password.is_empty() {
            let cmd_to_send = match auto_type {
                AutoConnectType::Prompt => {
                    match prompt_num {
                        1 if !user.is_empty() => Some(user),
                        2 if !password.is_empty() => Some(password),
                        _ => None,
                    }
                }
                AutoConnectType::MooPrompt => {
                    match prompt_num {
                        1 if !user.is_empty() => Some(user.clone()),
                        2 if !password.is_empty() => Some(password),
                        3 if !user.is_empty() => Some(user),
                        _ => None,
                    }
                }
                AutoConnectType::Connect | AutoConnectType::NoLogin => None,
            };

            if let Some(cmd) = cmd_to_send {
                if let Some(tx) = &world.command_tx {
                    let _ = tx.try_send(WriteCommand::Text(cmd));
                    world.last_send_time = Some(std::time::Instant::now());
                    // Clear prompt since we auto-answered it
                    world.prompt.clear();
                }
            }
        }
    }

    /// Handle GmcpNegotiated event.
    fn handle_gmcp_negotiated(&mut self, world_idx: usize) {
        self.worlds[world_idx].gmcp_enabled = true;
        let packages_str = self.worlds[world_idx].settings.gmcp_packages.clone();
        let packages: Vec<String> = packages_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        self.worlds[world_idx].gmcp_supported_packages = packages.clone();
        if let Some(ref tx) = self.worlds[world_idx].command_tx {
            let hello = build_gmcp_message("Core.Hello", &format!(
                "{{\"client\":\"Clay\",\"version\":\"{}\"}}",
                VERSION
            ));
            let _ = tx.try_send(WriteCommand::Raw(hello));
            let json_list: Vec<String> = packages.iter()
                .map(|p| format!("\"{}\"", p))
                .collect();
            let supports = build_gmcp_message(
                "Core.Supports.Set",
                &format!("[{}]", json_list.join(",")),
            );
            let _ = tx.try_send(WriteCommand::Raw(supports));
        }
    }

    /// Handle GmcpReceived event.
    fn handle_gmcp_received(&mut self, world_idx: usize, package: &str, json_data: &str) {
        // Always store GMCP data
        self.worlds[world_idx].gmcp_data.insert(package.to_string(), json_data.to_string());
        // Always store Client.Media.Default URL
        if package == "Client.Media.Default" {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_data) {
                if let Some(url) = parsed.get("url").and_then(|v| v.as_str()) {
                    self.worlds[world_idx].mcmp_default_url = url.to_string();
                }
            }
        }
        // Always broadcast to remote clients
        self.ws_broadcast(WsMessage::GmcpData {
            world_index: world_idx,
            package: package.to_string(),
            data: json_data.to_string(),
        });
        if package.starts_with("Client.Media.") {
            let action = package.rsplit('.').next().unwrap_or("Play").to_string();
            let default_url = self.worlds[world_idx].mcmp_default_url.clone();
            self.ws_broadcast(WsMessage::McmpMedia {
                world_index: world_idx,
                action,
                data: json_data.to_string(),
                default_url,
            });
        }
        // Always track media state; only play audio when enabled + current world
        if package.starts_with("Client.Media.") {
            let play_audio = self.worlds[world_idx].gmcp_user_enabled
                && world_idx == self.current_world_index;
            self.handle_gmcp_media(world_idx, package, json_data, play_audio);
        }
        // Gate TF hooks on gmcp_user_enabled
        if self.worlds[world_idx].gmcp_user_enabled {
            self.tf_engine.set_global("gmcp_package", crate::tf::TfValue::String(package.to_string()));
            self.tf_engine.set_global("gmcp_data", crate::tf::TfValue::String(json_data.to_string()));
            let results = crate::tf::hooks::fire_hook(&mut self.tf_engine, crate::tf::TfHookEvent::Gmcp);
            for r in results {
                if let crate::tf::TfCommandResult::SendToMud(text) = r {
                    if let Some(world) = self.worlds.get(world_idx) {
                        if let Some(ref tx) = world.command_tx {
                            let _ = tx.try_send(WriteCommand::Text(text));
                        }
                    }
                }
            }
        }
    }

    /// Handle MsdpReceived event.
    fn handle_msdp_received(&mut self, world_idx: usize, variable: &str, value_json: &str) {
        self.worlds[world_idx].msdp_variables.insert(variable.to_string(), value_json.to_string());
        // Fire TF MSDP hook
        self.tf_engine.set_global("msdp_var", crate::tf::TfValue::String(variable.to_string()));
        self.tf_engine.set_global("msdp_val", crate::tf::TfValue::String(value_json.to_string()));
        let results = crate::tf::hooks::fire_hook(&mut self.tf_engine, crate::tf::TfHookEvent::Msdp);
        for r in results {
            if let crate::tf::TfCommandResult::SendToMud(text) = r {
                if let Some(world) = self.worlds.get(world_idx) {
                    if let Some(ref tx) = world.command_tx {
                        let _ = tx.try_send(WriteCommand::Text(text));
                    }
                }
            }
        }
        // Broadcast to WebSocket clients
        self.ws_broadcast(WsMessage::MsdpData {
            world_index: world_idx,
            variable: variable.to_string(),
            value: value_json.to_string(),
        });
    }

    /// Handle WsClientDisconnected event.
    fn handle_ws_client_disconnected(&mut self, client_id: u64) {
        // Check if this client had NAWS dimensions, and recalculate if needed
        if let Some(state) = self.ws_client_worlds.get(&client_id) {
            if state.dimensions.is_some() {
                // Client had dimensions - need to recalculate NAWS after removal
                self.ws_client_worlds.remove(&client_id);
                // Recalculate NAWS for all worlds that might be affected
                for i in 0..self.worlds.len() {
                    if self.worlds[i].naws_enabled && self.worlds[i].connected {
                        self.send_naws_if_changed(i);
                    }
                }
                return;
            }
        }
        self.ws_client_worlds.remove(&client_id);
    }

    /// Handle WsAuthKeyValidation event.
    fn handle_ws_auth_key_validation(&mut self, client_id: u64, msg: WsMessage, client_ip: &str, challenge: &str) {
        if let WsMessage::AuthRequest { auth_key: Some(key), current_world, challenge_response: uses_challenge, .. } = msg {
            let has_key = self.settings.websocket_auth_key.is_some();

            crate::http::log_remote_event("WS-KEY", client_ip,
                &format!("challenge={}, has_stored_key={}", uses_challenge, has_key));

            // If challenge_response, client sent SHA256(auth_key + challenge)
            // We compute SHA256(stored_key + challenge) and compare
            let is_valid = if let Some(ref ak) = self.settings.websocket_auth_key {
                if uses_challenge {
                    hash_with_challenge(&ak.key, challenge) == key
                } else {
                    ak.key == key
                }
            } else {
                false
            };
            if is_valid {
                crate::http::log_remote_event("WS-KEY-OK", client_ip, "accepted");
                self.ws_set_client_authenticated(client_id, true);
                self.ws_send_to_client(client_id, WsMessage::AuthResponse {
                    success: true,
                    error: None,
                    username: None,
                    multiuser_mode: false,
                });
                let initial_state = self.build_initial_state();
                self.ws_send_initial_state_and_mark(client_id, initial_state);
                let world_idx = current_world
                    .filter(|&w| w < self.worlds.len())
                    .unwrap_or(self.current_world_index);
                self.ws_set_client_world(client_id, Some(world_idx));
                self.ws_client_worlds.insert(client_id, ClientViewState {
                    world_index: world_idx,
                    visible_lines: 0,
                    visible_columns: 0,
                    dimensions: None,
                });
                // Signal event loop to trigger web reconnects
                self.web_reconnect_needed = true;
            } else {
                crate::http::log_remote_event("WS-KEY-REJECT", client_ip, "no matching key");
                self.ban_list.record_violation(client_ip, "WebSocket: failed auth key");
                self.ws_send_to_client(client_id, WsMessage::AuthResponse {
                    success: false,
                    error: Some("Invalid auth key".to_string()),
                    username: None,
                    multiuser_mode: false,
                });
            }
        }
    }

    /// Handle WsKeyRequest event — generate a new single auth key (replaces any existing).
    fn handle_ws_key_request(&mut self, _client_id: u64) {
        let key = App::generate_auth_key();
        self.settings.websocket_auth_key = Some(AuthKey::new(key.clone()));
        let _ = persistence::save_settings(self);
        // Broadcast to ALL clients so every web UI updates its displayed key
        self.ws_broadcast(WsMessage::KeyGenerated { auth_key: key });
    }

    /// Handle WsKeyRevoke event — clear the single auth key.
    fn handle_ws_key_revoke(&mut self, _key: &str) {
        self.settings.websocket_auth_key = None;
        let _ = persistence::save_settings(self);
    }

    /// Generate a new auth key string.
    fn generate_auth_key() -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes());
        hasher.update(std::process::id().to_le_bytes());
        let mut random_bytes = [0u8; 16];
        let _ = getrandom::getrandom(&mut random_bytes);
        hasher.update(random_bytes);
        let key = hex::encode(hasher.finalize());
        let bt = std::backtrace::Backtrace::force_capture();
        let now = util::local_time_now();
        debug_log(true, &format!(
            "AUTH KEY GENERATED {:04}-{:02}-{:02} {:02}:{:02}:{:02}\n{}",
            now.year, now.month, now.day, now.hour, now.minute, now.second, bt
        ));
        key
    }

    /// Handle initial WsClientMessage (AuthRequest) after authentication.
    fn handle_ws_auth_initial_state(&mut self, client_id: u64, current_world: Option<usize>) {
        // Debug: log current world state for reload message diagnosis
        let cw = self.current_world_index;
        if cw < self.worlds.len() {
            let world = &self.worlds[cw];
            debug_log(true, &format!(
                "AUTH_INITIAL_STATE: current_world={} '{}' showing_splash={} output_lines={} is_reload={}",
                cw, world.name, world.showing_splash, world.output_lines.len(), self.is_reload
            ));
            // Log last 3 lines of output to verify reload message is present
            let start = world.output_lines.len().saturating_sub(3);
            for (i, line) in world.output_lines[start..].iter().enumerate() {
                debug_log(true, &format!(
                    "AUTH_INITIAL_STATE: output_line[{}] from_server={} text='{}'",
                    start + i, line.from_server, line.text.trim()
                ));
            }
        }
        let initial_state = self.build_initial_state();
        self.ws_send_initial_state_and_mark(client_id, initial_state);
        let world_idx = current_world
            .filter(|&w| w < self.worlds.len())
            .unwrap_or(self.current_world_index);
        self.ws_set_client_world(client_id, Some(world_idx));
        self.ws_client_worlds.insert(client_id, ClientViewState {
            world_index: world_idx,
            visible_lines: 0,
            visible_columns: 0,
            dimensions: None,
        });
        // Broadcast activity count to new client
        self.broadcast_activity();
        // Signal event loop to trigger web reconnects
        self.web_reconnect_needed = true;
    }

    /// Handle ConnectionSuccess event (used in headless and console modes).
    fn handle_connection_success(&mut self, world_name: &str, cmd_tx: mpsc::Sender<WriteCommand>, socket_fd: Option<SocketFd>, is_tls: bool) {
        if let Some(world_idx) = self.find_world_index(world_name) {
            self.worlds[world_idx].connected = true;
            self.worlds[world_idx].was_connected = true;
            self.worlds[world_idx].prompt_count = 0;
            let now = std::time::Instant::now();
            self.worlds[world_idx].last_send_time = Some(now);
            self.worlds[world_idx].last_receive_time = Some(now);
            self.worlds[world_idx].last_user_command_time = Some(now);
            self.worlds[world_idx].last_nop_time = None;
            self.worlds[world_idx].is_initial_world = false;
            self.worlds[world_idx].command_tx = Some(cmd_tx.clone());
            #[cfg(any(unix, windows))]
            { self.worlds[world_idx].socket_fd = socket_fd; }
            self.worlds[world_idx].is_tls = is_tls;

            // Discard any unused initial world (may shift indices)
            self.discard_initial_world();
            // Re-find world index since discard may have shifted the vec
            let world_idx = match self.find_world_index(world_name) {
                Some(idx) => idx,
                None => return,
            };

            // Open log file if enabled
            if self.worlds[world_idx].settings.log_enabled {
                if self.worlds[world_idx].open_log_file() {
                    let log_path = self.worlds[world_idx].get_log_path();
                    self.add_output_to_world(world_idx, &format!("Logging to: {}", log_path.display()));
                } else {
                    self.add_output_to_world(world_idx, "Warning: Could not open log file");
                }
            }

            // Fire TF CONNECT hook
            let hook_result = tf::bridge::fire_event(&mut self.tf_engine, tf::TfHookEvent::Connect);
            for cmd in hook_result.send_commands {
                let _ = cmd_tx.try_send(WriteCommand::Text(cmd));
            }
            for cmd in hook_result.clay_commands {
                let _ = self.tf_engine.execute(&cmd);
            }

            // Send auto-login if configured
            let skip_login = self.worlds[world_idx].skip_auto_login;
            let user = self.worlds[world_idx].settings.user.clone();
            let password = self.worlds[world_idx].settings.password.clone();
            let auto_connect_type = self.worlds[world_idx].settings.auto_connect_type;
            // Only clear skip flag here for Connect type; Prompt/MooPrompt check it later in handle_prompt
            if auto_connect_type == AutoConnectType::Connect {
                self.worlds[world_idx].skip_auto_login = false;
            }
            // FANSI worlds: always set up client detection window
            if self.worlds[world_idx].settings.encoding == Encoding::Fansi {
                self.worlds[world_idx].fansi_detect_until = Some(std::time::Instant::now() + Duration::from_secs(2));
                if !skip_login && !user.is_empty() && !password.is_empty() && auto_connect_type == AutoConnectType::Connect {
                    let connect_cmd = format!("connect {} {}", user, password);
                    self.worlds[world_idx].fansi_login_pending = Some(connect_cmd);
                }
            } else if !skip_login && !user.is_empty() && !password.is_empty() && auto_connect_type == AutoConnectType::Connect {
                let connect_cmd = format!("connect {} {}", user, password);
                let _ = cmd_tx.try_send(WriteCommand::Text(connect_cmd));
            }

            // Broadcast connection status
            self.ws_broadcast(WsMessage::WorldConnected { world_index: world_idx, name: self.worlds[world_idx].name.clone() });
        }
    }

    /// Handle WsMessage::SendCommand - processes the command and returns a WsAsyncAction
    /// if an async operation (connect/disconnect) is needed.
    #[allow(clippy::too_many_lines)]
    fn handle_ws_send_command(&mut self, client_id: u64, world_index: usize, command: &str, event_tx: &mpsc::Sender<AppEvent>) -> WsAsyncAction {
        // Use shared command parsing
        let parsed = parse_command(command);

        match parsed {
            // Commands handled locally on server
            Command::ActionCommand { name, args } => {
                // Execute action if it exists
                if let Some(action) = self.settings.actions.iter().find(|a| a.name.eq_ignore_ascii_case(&name)) {
                    // Skip disabled actions
                    if !action.enabled {
                        self.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: format!("\u{2728} Action '{}' is disabled.", name),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0,
                            marked_new: false,
                            flush: false, gagged: false,
                        });
                    } else {
                        let commands = split_action_commands(&action.command);
                        let mut sent_to_server = false;
                        for cmd in commands {
                            // Substitute $1-$9 and $* with arguments
                            let cmd = substitute_action_args(&cmd, &args);

                            if cmd.eq_ignore_ascii_case("/gag") || cmd.to_lowercase().starts_with("/gag ") {
                                continue;
                            }
                            // Unified command system - route through TF parser
                            if cmd.starts_with('/') {
                                self.sync_tf_world_info();
                                match self.tf_engine.execute(&cmd) {
                                    tf::TfCommandResult::Success(Some(msg)) => {
                                        self.ws_broadcast(WsMessage::ServerData {
                                            world_index,
                                            data: msg,
                                            is_viewed: false,
                                            ts: current_timestamp_secs(),
                                            from_server: false,
                                            seq: 0,
                                            marked_new: false,
                                            flush: false, gagged: false,
                                        });
                                    }
                                    tf::TfCommandResult::Success(None) => {}
                                    tf::TfCommandResult::Error(err) => {
                                        self.ws_broadcast(WsMessage::ServerData {
                                            world_index,
                                            data: format!("Error: {}", err),
                                            is_viewed: false,
                                            ts: current_timestamp_secs(),
                                            from_server: false,
                                            seq: 0,
                                            marked_new: false,
                                            flush: false, gagged: false,
                                        });
                                    }
                                    tf::TfCommandResult::SendToMud(text) => {
                                        if world_index < self.worlds.len() {
                                            if let Some(tx) = &self.worlds[world_index].command_tx {
                                                let _ = tx.try_send(WriteCommand::Text(text));
                                                sent_to_server = true;
                                            }
                                        }
                                    }
                                    tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                        self.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: clay_cmd });
                                    }
                                    tf::TfCommandResult::Recall(opts) => {
                                        if world_index < self.worlds.len() {
                                            let output_lines = self.worlds[world_index].output_lines.clone();
                                            let (matches, header) = execute_recall(&opts, &output_lines);
                                            let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                                            let ts = current_timestamp_secs();

                                            if !opts.quiet {
                                                if let Some(h) = header {
                                                    self.ws_broadcast(WsMessage::ServerData { world_index, data: h, is_viewed: false, ts , from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                                }
                                            }
                                            if matches.is_empty() {
                                                self.ws_broadcast(WsMessage::ServerData { world_index, data: format!("\u{2728} No matches for '{}'", pattern_str), is_viewed: false, ts, from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                            } else {
                                                for m in matches {
                                                    self.ws_broadcast(WsMessage::ServerData { world_index, data: m, is_viewed: false, ts , from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                                }
                                            }
                                            if !opts.quiet {
                                                self.ws_broadcast(WsMessage::ServerData { world_index, data: "================= Recall end =================".to_string(), is_viewed: false, ts , from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                            }
                                        }
                                    }
                                    tf::TfCommandResult::RepeatProcess(process) => {
                                        self.tf_engine.processes.push(process);
                                    }
                                    tf::TfCommandResult::Quote { mut lines, disposition, world, delay_secs, recall_opts } => {
                                        self.handle_ws_quote_result(world_index, &mut lines, disposition, &world, delay_secs, recall_opts);
                                        if disposition == tf::QuoteDisposition::Send && !lines.is_empty() {
                                            sent_to_server = true;
                                        }
                                    }
                                    _ => {}
                                }
                            } else if world_index < self.worlds.len() {
                                // Plain text - send to MUD server
                                if let Some(tx) = &self.worlds[world_index].command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(cmd));
                                    sent_to_server = true;
                                }
                            }
                        }
                        if sent_to_server {
                            self.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                        }
                    }
                } else {
                    // No matching action - try TF engine (handles /recall, /set, /echo, etc.)
                    self.sync_tf_world_info();
                    match self.tf_engine.execute(command) {
                        tf::TfCommandResult::Success(Some(msg)) => {
                            self.ws_broadcast(WsMessage::ServerData {
                                world_index, data: msg, is_viewed: false,
                                ts: current_timestamp_secs(), from_server: false,
                                seq: 0,
                                marked_new: false,
                                flush: false, gagged: false,
                            });
                        }
                        tf::TfCommandResult::Success(None) => {}
                        tf::TfCommandResult::Error(err) => {
                            self.ws_broadcast(WsMessage::ServerData {
                                world_index, data: format!("Error: {}", err), is_viewed: false,
                                ts: current_timestamp_secs(), from_server: false,
                                seq: 0,
                                marked_new: false,
                                flush: false, gagged: false,
                            });
                        }
                        tf::TfCommandResult::SendToMud(text) => {
                            if world_index < self.worlds.len() {
                                if let Some(tx) = &self.worlds[world_index].command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(text));
                                    self.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                }
                            }
                        }
                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                            self.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: clay_cmd });
                        }
                        tf::TfCommandResult::Recall(opts) => {
                            if world_index < self.worlds.len() {
                                let output_lines = self.worlds[world_index].output_lines.clone();
                                let (matches, header) = execute_recall(&opts, &output_lines);
                                let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                                let ts = current_timestamp_secs();
                                if !opts.quiet {
                                    if let Some(h) = header {
                                        self.ws_broadcast(WsMessage::ServerData { world_index, data: h, is_viewed: false, ts, from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                    }
                                }
                                if matches.is_empty() {
                                    self.ws_broadcast(WsMessage::ServerData { world_index, data: format!("\u{2728} No matches for '{}'", pattern_str), is_viewed: false, ts, from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                } else {
                                    for m in matches {
                                        self.ws_broadcast(WsMessage::ServerData { world_index, data: m, is_viewed: false, ts, from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                    }
                                }
                                if !opts.quiet {
                                    self.ws_broadcast(WsMessage::ServerData { world_index, data: "================= Recall end =================".to_string(), is_viewed: false, ts, from_server: false, seq: 0, marked_new: false, flush: false, gagged: false });
                                }
                            }
                        }
                        tf::TfCommandResult::RepeatProcess(process) => {
                            self.tf_engine.processes.push(process);
                        }
                        tf::TfCommandResult::Quote { mut lines, disposition, world, delay_secs, recall_opts } => {
                            self.handle_ws_quote_result(world_index, &mut lines, disposition, &world, delay_secs, recall_opts);
                        }
                        _ => {
                            self.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: format!("Unknown command: /{}", name),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0,
                                marked_new: false,
                                flush: false, gagged: false,
                            });
                        }
                    }
                }
            }
            Command::NotACommand { text } => {
                // Regular text - send to MUD
                if world_index < self.worlds.len() {
                    if let Some(tx) = &self.worlds[world_index].command_tx {
                        if tx.try_send(WriteCommand::Text(text)).is_ok() {
                            self.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                            self.worlds[world_index].prompt.clear();
                        }
                    }
                }
            }
            Command::Edit { .. } | Command::EditList => {
                // Edit command is handled locally on the client, not on server
                // Send back to client for local execution
                self.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.to_string() });
            }
            Command::Tag => {
                // Toggle show_tags setting (same as F2)
                self.show_tags = !self.show_tags;
                self.ws_broadcast(WsMessage::ShowTagsChanged { show_tags: self.show_tags });
            }
            Command::Unknown { cmd } => {
                self.ws_broadcast(WsMessage::ServerData {
                    world_index,
                    data: format!("Unknown command: {}", cmd),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::Send { text, all_worlds, target_world, no_newline } => {
                // Handle /send command
                // Helper to create the write command
                let make_write_cmd = |t: &str| -> WriteCommand {
                    if no_newline {
                        WriteCommand::Raw(t.as_bytes().to_vec())
                    } else {
                        WriteCommand::Text(t.to_string())
                    }
                };

                if all_worlds {
                    // Send to all connected worlds
                    for world in self.worlds.iter_mut() {
                        if world.connected {
                            if let Some(tx) = &world.command_tx {
                                let _ = tx.try_send(make_write_cmd(&text));
                                world.last_send_time = Some(std::time::Instant::now());
                            }
                        }
                    }
                } else if let Some(ref target) = target_world {
                    // Send to specific world by name
                    if let Some(world) = self.worlds.iter_mut().find(|w| w.name.eq_ignore_ascii_case(target)) {
                        if world.connected {
                            if let Some(tx) = &world.command_tx {
                                let _ = tx.try_send(make_write_cmd(&text));
                                world.last_send_time = Some(std::time::Instant::now());
                            }
                        } else {
                            self.ws_broadcast(WsMessage::ServerData {
                                world_index,
                                data: format!("World '{}' is not connected.", target),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0,
                                marked_new: false,
                                flush: false, gagged: false,
                            });
                        }
                    } else {
                        self.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: format!("Unknown world: {}", target),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0,
                            marked_new: false,
                            flush: false, gagged: false,
                        });
                    }
                } else {
                    // Send to current world (the one this command came from)
                    if world_index < self.worlds.len() {
                        if let Some(tx) = &self.worlds[world_index].command_tx {
                            let _ = tx.try_send(make_write_cmd(&text));
                            self.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                        }
                    }
                }
            }
            Command::Disconnect => {
                // Disconnect the specified world
                if world_index < self.worlds.len() && self.worlds[world_index].connected {
                    // Kill proxy process if one exists
                    #[cfg(unix)]
                    if let Some(proxy_pid) = self.worlds[world_index].proxy_pid {
                        unsafe { libc::kill(proxy_pid as libc::pid_t, libc::SIGTERM); }
                    }
                    self.worlds[world_index].clear_connection_state(true, true);
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: "Disconnected.".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
                    });
                    self.ws_broadcast(WsMessage::WorldDisconnected { world_index });
                } else {
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: "Not connected.".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
                    });
                }
            }
            Command::Flush => {
                // Clear output buffer for this world
                if world_index < self.worlds.len() {
                    let line_count = self.worlds[world_index].output_lines.len();
                    self.worlds[world_index].output_lines.clear();
                    self.worlds[world_index].first_marked_new_index = None;
                    self.worlds[world_index].pending_lines.clear();
                    self.worlds[world_index].scroll_offset = 0;
                    self.worlds[world_index].lines_since_pause = 0;
                    self.worlds[world_index].paused = false;
                    self.ws_broadcast(WsMessage::WorldFlushed { world_index });
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: format!("Flushed {} lines from output buffer.", line_count),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
                    });
                }
            }
            Command::Remote => {
                spawn_remote_ping_check(self, event_tx.clone(), client_id, world_index);
            }
            Command::RemoteKill { client_id: kill_id } => {
                if let Some(ref ws_server) = self.ws_server {
                    let msg = if let Ok(clients) = ws_server.clients.try_read() {
                        if let Some(client) = clients.get(&kill_id) {
                            let ip = client.ip_address.clone();
                            drop(clients);
                            if let Ok(mut clients_mut) = ws_server.clients.try_write() {
                                clients_mut.remove(&kill_id);
                                format!("Disconnected remote client {} ({})", kill_id, ip)
                            } else {
                                "Could not acquire write lock (busy).".to_string()
                            }
                        } else {
                            format!("No client with ID {}.", kill_id)
                        }
                    } else {
                        "Could not read client list (busy).".to_string()
                    };
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index, data: msg, is_viewed: false,
                        ts: current_timestamp_secs(), from_server: false,
                        seq: 0, marked_new: false, flush: false, gagged: false,
                    });
                }
            }
            Command::BanList => {
                // Send current ban list
                let bans = self.ban_list.get_ban_info();
                if bans.is_empty() {
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: "No hosts are currently banned.".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
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
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: output,
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
                    });
                }
                self.ws_send_to_client(client_id, WsMessage::BanListResponse { bans });
            }
            Command::Unban { host } => {
                if self.ban_list.remove_ban(&host) {
                    // Save settings to persist the change
                    let _ = persistence::save_settings(self);
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: format!("Removed ban for: {}", host),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
                    });
                    // Broadcast updated ban list
                    self.ws_broadcast(WsMessage::BanListResponse { bans: self.ban_list.get_ban_info() });
                    self.ws_send_to_client(client_id, WsMessage::UnbanResult { success: true, host, error: None });
                } else {
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index,
                        data: format!("No ban found for: {}", host),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
                    });
                    self.ws_send_to_client(client_id, WsMessage::UnbanResult { success: false, host, error: Some("No ban found".to_string()) });
                }
            }
            Command::TestMusic => {
                let test_notes = generate_test_music_notes();
                // Play only on the client that invoked the command
                self.ws_send_to_client(client_id, WsMessage::AnsiMusic {
                    world_index,
                    notes: test_notes,
                });
                self.ws_send_to_client(client_id, WsMessage::ServerData {
                    world_index,
                    data: "Playing test music (Super Mario Bros)...".to_string(),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::Notify { message } => {
                // Send notification to mobile clients
                let title = if world_index < self.worlds.len() {
                    self.worlds[world_index].name.clone()
                } else {
                    "Clay".to_string()
                };
                self.ws_broadcast(WsMessage::Notification {
                    title,
                    message: message.clone(),
                });
                self.ws_broadcast(WsMessage::ServerData {
                    world_index,
                    data: format!("Notification sent: {}", message),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::Say { text } => {
                // Speak text via TTS (console subprocess + broadcast to web clients)
                tts::speak(&self.tts_backend, &text, self.settings.tts_mode);
                let clean_text = strip_ansi_codes(&text);
                self.ws_broadcast(WsMessage::ServerSpeak {
                    text: clean_text.clone(),
                    world_index,
                });
                self.ws_broadcast(WsMessage::ServerData {
                    world_index,
                    data: format!("TTS: {}", text),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::Dump => {
                // Dump comprehensive debug state to ~/.clay.dmp.log
                use std::io::Write;
                let ts = current_timestamp_secs();

                let home = get_home_dir();
                let dump_path = format!("{}/{}", home, clay_filename("clay.dmp.log"));

                match std::fs::File::create(&dump_path) {
                    Ok(mut file) => {
                        let _ = writeln!(file, "=== CLAY DEBUG DUMP ===");
                        let _ = writeln!(file, "Version: {} (build {}-{})", VERSION, BUILD_DATE, BUILD_HASH);
                        let lt = local_time_from_epoch(ts as i64);
                        let _ = writeln!(file, "Timestamp: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                            lt.year, lt.month, lt.day, lt.hour, lt.minute, lt.second);
                        let _ = writeln!(file, "Mode: daemon/ws");
                        let _ = writeln!(file);

                        // Global state
                        let _ = writeln!(file, "=== GLOBAL STATE ===");
                        let _ = writeln!(file, "output_height: {}", self.output_height);
                        let _ = writeln!(file, "output_width: {}", self.output_width);
                        let _ = writeln!(file, "current_world_index: {}", self.current_world_index);
                        let _ = writeln!(file, "worlds_count: {}", self.worlds.len());
                        let _ = writeln!(file, "more_mode_enabled: {}", self.settings.more_mode_enabled);
                        let _ = writeln!(file, "show_tags: {}", self.show_tags);
                        let _ = writeln!(file, "new_line_indicator: {}", self.settings.new_line_indicator);
                        let _ = writeln!(file);

                        // WS client info
                        let _ = writeln!(file, "=== WS CLIENTS ===");
                        let _ = writeln!(file, "ws_client_worlds count: {}", self.ws_client_worlds.len());
                        for (cid, cv) in &self.ws_client_worlds {
                            let _ = writeln!(file, "  client {}: world_index={}, visible_lines={}, visible_columns={}, dimensions={:?}",
                                cid, cv.world_index, cv.visible_lines, cv.visible_columns, cv.dimensions);
                        }
                        let _ = writeln!(file);

                        // Per-world state
                        for (wi, world) in self.worlds.iter().enumerate() {
                            let is_current = wi == self.current_world_index;
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

                            let ws_min_lines = self.min_viewer_lines(wi);
                            let ws_min_width = self.min_viewer_width(wi);
                            let _ = writeln!(file, "min_viewer_lines: {:?}", ws_min_lines);
                            let _ = writeln!(file, "min_viewer_width: {:?}", ws_min_width);

                            let console_viewing = wi == self.current_world_index;
                            let eff_height = match (console_viewing, ws_min_lines) {
                                (true, Some(ws)) => (self.output_height).min(ws as u16),
                                (true, None) => self.output_height,
                                (false, Some(ws)) => ws as u16,
                                (false, None) => self.output_height,
                            };
                            let eff_width = match (console_viewing, ws_min_width) {
                                (true, Some(ws_w)) => (self.output_width).min(ws_w as u16),
                                (true, None) => self.output_width,
                                (false, Some(ws_w)) => ws_w as u16,
                                (false, None) => self.output_width,
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

                        // Don't broadcast — /dump must be passive and not disturb more-mode state
                    }
                    Err(_e) => {}
                }
            }
            Command::Window { ref world } => {
                // Auto-connect if the world exists but isn't connected
                if let Some(ref name) = world {
                    if let Some(idx) = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                        if !self.worlds[idx].connected && self.worlds[idx].settings.has_connection_settings() {
                            let prev_index = self.current_world_index;
                            self.current_world_index = idx;
                            // Return Connect action — caller will handle the async connect
                            self.ws_send_to_client(client_id, WsMessage::OpenWindow { world: world.clone() });
                            return WsAsyncAction::Connect { world_index: idx, prev_index, broadcast: true };
                        }
                    }
                }
                // Send OpenWindow message to requesting client
                self.ws_send_to_client(client_id, WsMessage::OpenWindow { world: world.clone() });
            }
            Command::Quit => {
                // Tell the requesting client to close (GUI window closes)
                // Server keeps running — /quit only affects the remote client
                self.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: "/quit".to_string() });
            }
            Command::Update { .. } => {
                // Bounce back to client — /update should run locally on the client's machine
                self.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.to_string() });
            }
            Command::Reload => {
                return WsAsyncAction::Reload;
            }
            Command::Help => {
                self.handle_ws_help_via_tf(client_id, world_index, "/help");
            }
            // UI popup commands - send back to client for local handling
            Command::Menu | Command::Font | Command::Setup | Command::Web | Command::Actions { .. } |
            Command::WorldsList | Command::WorldSelector | Command::WorldEdit { .. } => {
                self.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.to_string() });
            }
            Command::Version => {
                self.ws_send_to_client(client_id, WsMessage::ServerData {
                    world_index,
                    data: get_version_string(),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            // AddWorld - add or update world definition
            Command::AddWorld { name, host, port, user, password, use_ssl } => {
                let existing_idx = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(&name));

                let world_idx = if let Some(idx) = existing_idx {
                    idx
                } else {
                    let new_world = World::new(&name);
                    self.worlds.push(new_world);
                    self.worlds.len() - 1
                };

                if let Some(h) = host {
                    self.worlds[world_idx].settings.hostname = h;
                }
                if let Some(p) = port {
                    self.worlds[world_idx].settings.port = p;
                }
                if let Some(u) = user {
                    self.worlds[world_idx].settings.user = u;
                }
                if let Some(p) = password {
                    self.worlds[world_idx].settings.password = p;
                }
                self.worlds[world_idx].settings.use_ssl = use_ssl;

                let _ = persistence::save_settings(self);

                let action = if existing_idx.is_some() { "Updated" } else { "Added" };
                let host_info = if !self.worlds[world_idx].settings.hostname.is_empty() {
                    format!(" ({}:{}{})",
                        self.worlds[world_idx].settings.hostname,
                        self.worlds[world_idx].settings.port,
                        if use_ssl { " SSL" } else { "" })
                } else {
                    " (connectionless)".to_string()
                };
                self.ws_broadcast(WsMessage::ServerData {
                    world_index,
                    data: format!("{} world '{}'{}.", action, name, host_info),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            // Connect command - needs async follow-up
            Command::Connect { .. } => {
                if world_index < self.worlds.len() && !self.worlds[world_index].connected {
                    if self.worlds[world_index].settings.has_connection_settings() {
                        let prev_index = self.current_world_index;
                        self.current_world_index = world_index;
                        return WsAsyncAction::Connect { world_index, prev_index, broadcast: false };
                    } else {
                        self.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: "No connection settings configured for this world.".to_string(),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0,
                            marked_new: false,
                            flush: false, gagged: false,
                        });
                    }
                }
            }
            // WorldSwitch and WorldConnectNoLogin need proper handling
            Command::WorldSwitch { ref name } | Command::WorldConnectNoLogin { ref name } => {
                if let Some(idx) = self.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                    // Switch only the requesting client's world, not the console
                    let prev = self.ws_client_worlds.get(&client_id);
                    let dimensions = prev.and_then(|s| s.dimensions);
                    let visible_lines = prev.map(|v| v.visible_lines).unwrap_or(0);
                    let visible_columns = prev.map(|v| v.visible_columns).unwrap_or(0);
                    self.ws_client_worlds.insert(client_id, ClientViewState { world_index: idx, visible_lines, visible_columns, dimensions });
                    self.ws_set_client_world(client_id, Some(idx));
                    self.ws_send_to_client(client_id, WsMessage::WorldSwitched { new_index: idx });
                    // Also send ExecuteLocalCommand so web clients can switch their local view
                    self.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.to_string() });
                    // Connect if not connected and has settings
                    if !self.worlds[idx].connected
                        && self.worlds[idx].settings.has_connection_settings()
                    {
                        // For WorldConnectNoLogin, set skip flag
                        if matches!(parsed, Command::WorldConnectNoLogin { .. }) {
                            self.worlds[idx].skip_auto_login = true;
                        }
                        let prev_index = self.current_world_index;
                        self.current_world_index = idx;
                        return WsAsyncAction::Connect { world_index: idx, prev_index, broadcast: false };
                    }
                } else {
                    self.ws_send_to_client(client_id, WsMessage::ServerData {
                        world_index,
                        data: format!("World '{}' not found.", name),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0,
                        marked_new: false,
                        flush: false, gagged: false,
                    });
                }
            }
            Command::Dict { .. } | Command::Urban { .. } | Command::Translate { .. } | Command::TinyUrl { .. } => {
                spawn_api_lookup(event_tx.clone(), client_id, world_index, parsed);
            }
            Command::DictUsage => {
                self.ws_send_to_client(client_id, WsMessage::ServerData {
                    world_index,
                    data: "Usage: /dict <word>".to_string(),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::UrbanUsage => {
                self.ws_send_to_client(client_id, WsMessage::ServerData {
                    world_index,
                    data: "Usage: /urban <word>".to_string(),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::TranslateUsage => {
                self.ws_send_to_client(client_id, WsMessage::ServerData {
                    world_index,
                    data: "Usage: /translate <lang> <text>".to_string(),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::TinyUrlUsage => {
                self.ws_send_to_client(client_id, WsMessage::ServerData {
                    world_index,
                    data: "Usage: /url <url>".to_string(),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0,
                    marked_new: false,
                    flush: false, gagged: false,
                });
            }
            Command::HelpTopic { ref topic } => {
                // Try Clay help first, then TF help via engine
                use popup::definitions::help::get_topic_help;
                if let Some(lines) = get_topic_help(topic) {
                    let help_text = lines.join("\n");
                    for line in help_text.lines() {
                        self.ws_send_to_client(client_id, WsMessage::ServerData {
                            world_index, data: line.to_string(), is_viewed: false,
                            ts: current_timestamp_secs(), from_server: false, seq: 0,
                            marked_new: false, flush: false, gagged: false,
                        });
                    }
                } else {
                    self.handle_ws_help_via_tf(client_id, world_index, &format!("/help {}", topic));
                }
            }
        }
        WsAsyncAction::Done
    }

    /// Send TF help output to a WebSocket client
    fn handle_ws_help_via_tf(&mut self, client_id: u64, world_index: usize, cmd: &str) {
        self.sync_tf_world_info();
        let help_text = match self.tf_engine.execute(cmd) {
            tf::TfCommandResult::Success(Some(msg)) => msg,
            _ => "No help available. Try /help commands".to_string(),
        };
        for line in help_text.lines() {
            self.ws_send_to_client(client_id, WsMessage::ServerData {
                world_index, data: line.to_string(), is_viewed: false,
                ts: current_timestamp_secs(), from_server: false, seq: 0,
                marked_new: false, flush: false, gagged: false,
            });
        }
    }

    /// Handle TfCommandResult::Quote from a WebSocket client command.
    /// Mirrors the console Quote handling but uses synchronous ws_broadcast/try_send.
    fn handle_ws_quote_result(
        &mut self,
        world_index: usize,
        lines: &mut Vec<String>,
        disposition: tf::QuoteDisposition,
        world: &Option<String>,
        delay_secs: f64,
        recall_opts: Option<(tf::RecallOptions, String)>,
    ) {
        // If this is a /quote with backtick /recall, execute the recall now
        if let Some((opts, recall_prefix)) = recall_opts {
            if world_index < self.worlds.len() {
                let output_lines = self.worlds[world_index].output_lines.clone();
                let (matches, _header) = execute_recall(&opts, &output_lines);
                *lines = matches.iter()
                    .map(|line| format!("{}{}", recall_prefix, line))
                    .collect();
                if lines.is_empty() {
                    let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                    self.ws_broadcast(WsMessage::ServerData {
                        world_index, data: format!("(no recall matches for '{}')", pattern_str),
                        is_viewed: false, ts: current_timestamp_secs(), from_server: false, seq: 0, marked_new: false,
 flush: false, gagged: false,
                    });
                }
            }
        }

        // Determine target world index
        let target_idx = if let Some(ref world_name) = world {
            self.worlds.iter().position(|w| w.name == *world_name).unwrap_or(world_index)
        } else {
            world_index
        };
        let target_world_name = if target_idx < self.worlds.len() {
            Some(self.worlds[target_idx].name.clone())
        } else {
            None
        };

        if delay_secs > 0.0 && lines.len() > 1 {
            // Schedule as processes with delays
            let delay = std::time::Duration::from_secs_f64(delay_secs);
            let now = std::time::Instant::now();
            for (i, line) in lines.drain(..).enumerate() {
                let cmd = match disposition {
                    tf::QuoteDisposition::Send => line,
                    tf::QuoteDisposition::Echo => format!("/echo {}", line),
                    tf::QuoteDisposition::Exec => line,
                };
                let id = self.tf_engine.next_process_id;
                self.tf_engine.next_process_id += 1;
                let process = tf::TfProcess {
                    id,
                    command: cmd,
                    interval: delay,
                    count: Some(1),
                    remaining: Some(1),
                    next_run: now + delay * i as u32,
                    world: target_world_name.clone(),
                    synchronous: false,
                    on_prompt: false,
                    priority: 0,
                };
                self.tf_engine.processes.push(process);
            }
        } else {
            // Send immediately (no delay or single line)
            for line in lines.drain(..) {
                match disposition {
                    tf::QuoteDisposition::Send => {
                        if target_idx < self.worlds.len() {
                            if self.worlds[target_idx].connected {
                                if let Some(tx) = &self.worlds[target_idx].command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(line));
                                    self.worlds[target_idx].last_send_time = Some(std::time::Instant::now());
                                }
                            } else {
                                self.ws_broadcast(WsMessage::ServerData {
                                    world_index, data: "Not connected".to_string(),
                                    is_viewed: false, ts: current_timestamp_secs(), from_server: false, seq: 0, marked_new: false,
 flush: false, gagged: false,
                                });
                                break;
                            }
                        }
                    }
                    tf::QuoteDisposition::Echo => {
                        self.ws_broadcast(WsMessage::ServerData {
                            world_index, data: line,
                            is_viewed: false, ts: current_timestamp_secs(), from_server: false, seq: 0, marked_new: false,
 flush: false, gagged: false,
                        });
                    }
                    tf::QuoteDisposition::Exec => {
                        // Execute each line as a TF command
                        let result = self.tf_engine.execute(&line);
                        match result {
                            tf::TfCommandResult::SendToMud(text) => {
                                if target_idx < self.worlds.len() {
                                    if let Some(tx) = &self.worlds[target_idx].command_tx {
                                        let _ = tx.try_send(WriteCommand::Text(text));
                                        self.worlds[target_idx].last_send_time = Some(std::time::Instant::now());
                                    }
                                }
                            }
                            tf::TfCommandResult::Success(Some(msg)) => {
                                self.ws_broadcast(WsMessage::ServerData {
                                    world_index, data: msg,
                                    is_viewed: false, ts: current_timestamp_secs(), from_server: false, seq: 0, marked_new: false,
 flush: false, gagged: false,
                                });
                            }
                            tf::TfCommandResult::Error(err) => {
                                self.ws_broadcast(WsMessage::ServerData {
                                    world_index, data: format!("Error: {}", err),
                                    is_viewed: false, ts: current_timestamp_secs(), from_server: false, seq: 0, marked_new: false,
 flush: false, gagged: false,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Handle a full WsMessage from a client. Returns WsAsyncAction for operations
    /// that require async follow-up (connect/disconnect).
    #[allow(clippy::too_many_lines)]
    fn handle_ws_client_msg(&mut self, client_id: u64, msg: WsMessage, event_tx: &mpsc::Sender<AppEvent>) -> WsAsyncAction {
        let auth_current_world = if let WsMessage::AuthRequest { ref current_world, .. } = msg {
            *current_world
        } else {
            None
        };
        match msg {
            WsMessage::AuthRequest { .. } => {
                self.handle_ws_auth_initial_state(client_id, auth_current_world);
            }
            WsMessage::SendCommand { world_index, command } => {
                // Reset more-mode counter when ANY client sends a command
                if world_index < self.worlds.len() {
                    self.worlds[world_index].lines_since_pause = 0;
                    self.worlds[world_index].last_user_command_time = Some(std::time::Instant::now());
                    // Clear paused flag if no pending lines (matches console Enter behavior)
                    if self.worlds[world_index].pending_lines.is_empty() {
                        self.worlds[world_index].paused = false;
                    }
                    // Update client's viewing world to ensure they receive output
                    // (fixes race condition where client sends command before UpdateViewState)
                    let prev = self.ws_client_worlds.get(&client_id);
                    let dimensions = prev.and_then(|s| s.dimensions);
                    let visible_lines = prev.map(|v| v.visible_lines).unwrap_or(0);
                    let visible_columns = prev.map(|v| v.visible_columns).unwrap_or(0);
                    self.ws_client_worlds.insert(client_id, ClientViewState { world_index, visible_lines, visible_columns, dimensions });
                    self.ws_set_client_world(client_id, Some(world_index));
                }

                return self.handle_ws_send_command(client_id, world_index, &command, event_tx);
            }
            WsMessage::SwitchWorld { world_index } => {
                // Switch only the requesting client's world, not the console
                if world_index < self.worlds.len() {
                    let prev = self.ws_client_worlds.get(&client_id);
                    let dimensions = prev.and_then(|s| s.dimensions);
                    let visible_lines = prev.map(|v| v.visible_lines).unwrap_or(0);
                    let visible_columns = prev.map(|v| v.visible_columns).unwrap_or(0);
                    self.ws_client_worlds.insert(client_id, ClientViewState { world_index, visible_lines, visible_columns, dimensions });
                    self.ws_set_client_world(client_id, Some(world_index));
                    self.ws_send_to_client(client_id, WsMessage::WorldSwitched { new_index: world_index });
                    // Send active media for the new world
                    self.ws_send_active_media_to_client(client_id, world_index);
                }
            }
            WsMessage::ConnectWorld { world_index } => {
                // Trigger connection for specified world
                if world_index < self.worlds.len() && !self.worlds[world_index].connected {
                    // Check if world has connection settings
                    if !self.worlds[world_index].settings.has_connection_settings() {
                        self.ws_broadcast(WsMessage::ServerData {
                            world_index,
                            data: "No connection settings configured for this world.".to_string(),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0,
                            marked_new: false,
                            flush: false, gagged: false,
                        });
                    } else {
                        // Save current world index, switch to target, connect, then restore
                        let prev_index = self.current_world_index;
                        self.current_world_index = world_index;
                        return WsAsyncAction::Connect { world_index, prev_index, broadcast: true };
                    }
                }
            }
            WsMessage::DisconnectWorld { world_index } => {
                // Disconnect specified world
                if world_index < self.worlds.len() && self.worlds[world_index].connected {
                    let prev_index = self.current_world_index;
                    self.current_world_index = world_index;
                    return WsAsyncAction::Disconnect { world_index, prev_index };
                }
            }
            WsMessage::CreateWorld { name } => {
                // Create new world and broadcast to all clients
                let new_world = World::new(&name);
                self.worlds.push(new_world);
                let idx = self.worlds.len() - 1;
                let world = &self.worlds[idx];
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
                        password: {
                            let p = persistence::decrypt_password(&world.settings.password);
                            if p.starts_with("ENC:") { String::new() } else { p }
                        },
                        has_password: !world.settings.password.is_empty(),
                        use_ssl: world.settings.use_ssl,
                        log_enabled: world.settings.log_enabled,
                        encoding: world.settings.encoding.name().to_string(),
                        auto_connect_type: world.settings.auto_connect_type.name().to_string(),
                        keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                        keep_alive_cmd: world.settings.keep_alive_cmd.clone(),
                        gmcp_packages: world.settings.gmcp_packages.clone(),
                        auto_reconnect_secs: world.settings.auto_reconnect_display(),
                    },
                    last_send_secs: None,
                    last_recv_secs: None,
                    last_nop_secs: None,
                    keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                    showing_splash: world.showing_splash,
                    was_connected: false,
                    is_proxy: false,
                    gmcp_user_enabled: world.gmcp_user_enabled,
                    total_output_lines: 0,
                    pending_count: 0,
                };
                self.ws_broadcast(WsMessage::WorldAdded { world: Box::new(world_state) });
                let _ = persistence::save_settings(self);
                // Send the new world's index back to the requesting client
                self.ws_send_to_client(client_id, WsMessage::WorldCreated { world_index: idx });
            }
            WsMessage::DeleteWorld { world_index } => {
                // Delete specified world (if not the last one)
                if self.worlds.len() > 1 && world_index < self.worlds.len() {
                    let deleted_name = self.worlds[world_index].name.clone();
                    self.worlds.remove(world_index);
                    // Adjust current_world_index if needed
                    if self.current_world_index >= self.worlds.len() {
                        self.current_world_index = self.worlds.len().saturating_sub(1);
                    } else if self.current_world_index > world_index {
                        self.current_world_index -= 1;
                    }
                    // Adjust previous_world_index if needed
                    if let Some(prev) = self.previous_world_index {
                        if prev >= self.worlds.len() {
                            self.previous_world_index = Some(self.worlds.len().saturating_sub(1));
                        } else if prev > world_index {
                            self.previous_world_index = Some(prev - 1);
                        }
                    }
                    self.add_output(&format!("World '{}' deleted.\n", deleted_name));
                    // Broadcast WorldRemoved to all clients
                    self.ws_broadcast(WsMessage::WorldRemoved { world_index });
                    let _ = persistence::save_settings(self);
                }
            }
            WsMessage::MarkWorldSeen { world_index } => {
                // A remote client has viewed this world - update their current_world
                if world_index < self.worlds.len() {
                    // Check if client is switching to a different world
                    let old_world_idx = self.ws_client_worlds.get(&client_id).map(|s| s.world_index);
                    let switched = old_world_idx.map(|old| old != world_index).unwrap_or(true);
                    // When switching away from the old world, clear its new line indicators
                    // and reset lines_since_pause if more-mode hasn't triggered
                    if let Some(old_idx) = old_world_idx {
                        if old_idx != world_index && old_idx < self.worlds.len() {
                            self.worlds[old_idx].clear_new_line_indicators();
                            if self.worlds[old_idx].pending_lines.is_empty() {
                                self.worlds[old_idx].lines_since_pause = 0;
                            }
                        }
                    }
                    // Track which world this client is viewing (sync cache)
                    let prev = self.ws_client_worlds.get(&client_id);
                    let visible_lines = prev.map(|v| v.visible_lines).unwrap_or(0);
                    let visible_columns = prev.map(|v| v.visible_columns).unwrap_or(0);
                    let dimensions = prev.and_then(|s| s.dimensions);
                    self.ws_client_worlds.insert(client_id, ClientViewState { world_index, visible_lines, visible_columns, dimensions });
                    // Update client's world in WebSocket server (async state)
                    self.ws_set_client_world(client_id, Some(world_index));

                    self.worlds[world_index].mark_seen();
                    // Broadcast to all clients so they update their UI
                    self.ws_broadcast(WsMessage::UnseenCleared { world_index });
                    // Broadcast activity count since a world was just marked as seen
                    self.broadcast_activity();
                    // Trigger console redraw to update activity indicator
                    self.needs_output_redraw = true;
                    if switched {
                        self.ws_send_active_media_to_client(client_id, world_index);
                    }
                }
            }
            WsMessage::UpdateViewState { world_index, visible_lines, visible_columns } => {
                // A remote client is reporting its view state (for more-mode threshold calculation)
                if world_index < self.worlds.len() {
                    // Preserve existing dimensions when updating view state
                    let dimensions = self.ws_client_worlds.get(&client_id).and_then(|s| s.dimensions);
                    let vc = visible_columns.unwrap_or_else(|| self.ws_client_worlds.get(&client_id).map(|v| v.visible_columns).unwrap_or(0));
                    self.ws_client_worlds.insert(client_id, ClientViewState { world_index, visible_lines, visible_columns: vc, dimensions });
                    // Update client's world in WebSocket server so broadcast_to_world_viewers works
                    self.ws_set_client_world(client_id, Some(world_index));
                }
            }
            WsMessage::UpdateDimensions { width, height } => {
                // A remote client is reporting its output dimensions (for NAWS)
                if let Some(state) = self.ws_client_worlds.get_mut(&client_id) {
                    let old_dims = state.dimensions;
                    state.dimensions = Some((width, height));
                    // If dimensions changed, send NAWS updates to all worlds
                    if old_dims != Some((width, height)) {
                        self.send_naws_to_all_worlds();
                    }
                }
            }
            WsMessage::ReleasePending { world_index, count } => {
                // A remote client is releasing pending lines - sync across all interfaces
                if world_index < self.worlds.len() {
                    let pending_count = self.worlds[world_index].pending_lines.len();
                    if pending_count > 0 {
                        // Get client's output width for visual line calculation
                        let client_width = self.ws_client_worlds.get(&client_id)
                            .map(|s| s.visible_columns)
                            .filter(|&w| w > 0)
                            .unwrap_or(self.output_width as usize);

                        // count == 0 means release all; otherwise treat count as visual budget
                        let visual_budget = if count == 0 { usize::MAX } else { count };

                        // Pre-calculate logical lines to release (same logic as release_pending)
                        let width = client_width.max(1);
                        let mut visual_total = 0;
                        let mut logical_count = 0;
                        for line in &self.worlds[world_index].pending_lines {
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
                        // For release-all, cap at pending_count
                        let to_release = logical_count.min(pending_count);

                        // Get text and marked_new flag of lines that will be released
                        let has_marked_new = self.worlds[world_index].pending_lines.iter()
                            .take(to_release).any(|l| l.marked_new);
                        let lines_to_broadcast: Vec<String> = self.worlds[world_index]
                            .pending_lines
                            .iter()
                            .take(to_release)
                            .map(|line| line.text.replace('\r', ""))
                            .collect();

                        // Release the lines on the server
                        self.worlds[world_index].release_pending(visual_budget, client_width);

                        // Broadcast the released lines to clients viewing this world
                        if !lines_to_broadcast.is_empty() {
                            let ws_data = lines_to_broadcast.join("\n") + "\n";
                            self.ws_broadcast_to_world(world_index, WsMessage::ServerData {
                                world_index,
                                data: ws_data,
                                is_viewed: true,
                                ts: current_timestamp_secs(),
                                from_server: true,
                                // Use seq 0 to bypass client-side dedup check. Released pending
                                // lines have old seqs that may be lower than _max_seq if new data
                                // arrived after reconnect, causing false duplicate detection.
                                seq: 0,
                                marked_new: has_marked_new,
                                flush: false, gagged: false,
                            });
                        }

                        // Broadcast to all clients so they know how many were released
                        self.ws_broadcast(WsMessage::PendingReleased { world_index, count: to_release });
                        let new_count = self.worlds[world_index].pending_lines.len();
                        self.ws_broadcast(WsMessage::PendingLinesUpdate { world_index, count: new_count });

                        // Broadcast activity count since pending lines changed
                        self.broadcast_activity();

                        // Update console display
                        if world_index == self.current_world_index {
                            self.needs_output_redraw = true;
                        }
                    } else {
                        // Client has stale pending_count - sync them with the actual state
                        self.ws_broadcast(WsMessage::PendingLinesUpdate { world_index, count: 0 });
                        self.broadcast_activity();
                    }
                }
            }
            WsMessage::UpdateWorldSettings { world_index, name, hostname, port, user, password, use_ssl, log_enabled, encoding, auto_login, keep_alive_type, keep_alive_cmd, gmcp_packages, auto_reconnect_secs } => {
                // Update world settings from remote client
                if world_index < self.worlds.len() {
                    self.worlds[world_index].name = name.clone();
                    self.worlds[world_index].settings.hostname = hostname.clone();
                    self.worlds[world_index].settings.port = port.clone();
                    self.worlds[world_index].settings.user = user.clone();
                    // Only update password if client sent a non-empty plaintext value
                    if !password.is_empty() && !password.starts_with("ENC:") {
                        self.worlds[world_index].settings.password = password.clone();
                    }
                    self.worlds[world_index].settings.use_ssl = use_ssl;
                    self.worlds[world_index].settings.log_enabled = log_enabled;
                    self.worlds[world_index].settings.encoding = match encoding.as_str() {
                        "latin1" => Encoding::Latin1,
                        "fansi" => Encoding::Fansi,
                        _ => Encoding::Utf8,
                    };
                    self.worlds[world_index].settings.auto_connect_type = AutoConnectType::from_name(&auto_login);
                    self.worlds[world_index].settings.keep_alive_type = KeepAliveType::from_name(&keep_alive_type);
                    self.worlds[world_index].settings.keep_alive_cmd = keep_alive_cmd.clone();
                    self.worlds[world_index].settings.gmcp_packages = gmcp_packages.clone();
                    let (ar_secs, ar_on_web) = WorldSettings::parse_auto_reconnect(&auto_reconnect_secs);
                    self.worlds[world_index].settings.auto_reconnect_secs = ar_secs;
                    self.worlds[world_index].settings.auto_reconnect_on_web = ar_on_web;
                    // Save settings to persist changes
                    let _ = persistence::save_settings(self);
                    // Build settings message for broadcast (don't leak password)
                    let settings_msg = WorldSettingsMsg {
                        hostname,
                        port,
                        user,
                        has_password: !password.is_empty(),
                        password: String::new(),
                        use_ssl,
                        log_enabled,
                        encoding,
                        auto_connect_type: auto_login,
                        keep_alive_type,
                        keep_alive_cmd,
                        gmcp_packages,
                        auto_reconnect_secs,
                    };
                    // Broadcast update to all clients
                    self.ws_broadcast(WsMessage::WorldSettingsUpdated {
                        world_index,
                        settings: settings_msg,
                        name,
                    });
                }
            }
            WsMessage::UpdateGlobalSettings { more_mode_enabled, spell_check_enabled, temp_convert_enabled, world_switch_mode, show_tags, debug_enabled, ansi_music_enabled, console_theme, gui_theme, gui_transparency, color_offset_percent, input_height, font_name, font_size, web_font_size_phone, web_font_size_tablet, web_font_size_desktop, web_font_weight, web_font_line_height, web_font_letter_spacing, web_font_word_spacing, ws_allow_list, web_secure, http_enabled, http_port, ws_enabled: _, ws_port: _, ws_cert_file, ws_key_file, ws_password, tls_proxy_enabled, dictionary_path, mouse_enabled, zwj_enabled, new_line_indicator, tts_mode, tts_speak_mode } => {
                // Update global settings from remote client
                self.settings.more_mode_enabled = more_mode_enabled;
                self.settings.spell_check_enabled = spell_check_enabled;
                self.settings.temp_convert_enabled = temp_convert_enabled;
                self.settings.world_switch_mode = WorldSwitchMode::from_name(&world_switch_mode);
                self.show_tags = show_tags;
                self.settings.debug_enabled = debug_enabled;
                DEBUG_ENABLED.store(debug_enabled, Ordering::Relaxed);
                self.settings.ansi_music_enabled = ansi_music_enabled;
                // Console theme affects the TUI on the server
                self.settings.theme = Theme::from_name(&console_theme);
                // GUI theme is stored for sending back to GUI clients
                self.settings.gui_theme = Theme::from_name(&gui_theme);
                self.settings.gui_transparency = gui_transparency.clamp(0.3, 1.0);
                self.settings.color_offset_percent = color_offset_percent.min(100);
                self.input_height = input_height.clamp(1, 15);
                self.input.visible_height = self.input_height;
                self.settings.font_name = font_name;
                self.settings.font_size = font_size.clamp(8.0, 48.0);
                self.settings.web_font_size_phone = web_font_size_phone.clamp(8.0, 48.0);
                self.settings.web_font_size_tablet = web_font_size_tablet.clamp(8.0, 48.0);
                self.settings.web_font_size_desktop = web_font_size_desktop.clamp(8.0, 48.0);
                self.settings.web_font_weight = web_font_weight.clamp(1, 900);
                self.settings.web_font_line_height = web_font_line_height.clamp(0.5, 3.0);
                self.settings.web_font_letter_spacing = web_font_letter_spacing.clamp(-5.0, 10.0);
                self.settings.web_font_word_spacing = web_font_word_spacing.clamp(-5.0, 20.0);
                self.settings.websocket_allow_list = ws_allow_list.clone();
                // Update the running WebSocket server's allow list
                if let Some(ref server) = self.ws_server {
                    server.update_allow_list(&ws_allow_list);
                }
                // Update password (may be empty string to clear it)
                self.settings.websocket_password = ws_password.clone();
                if let Some(ref server) = self.ws_server {
                    server.update_password(&ws_password);
                }
                // Update web settings — flag a restart if server-affecting settings changed
                let web_changed = self.settings.web_secure != web_secure
                    || self.settings.http_enabled != http_enabled
                    || self.settings.http_port != http_port;
                self.settings.web_secure = web_secure;
                self.settings.http_enabled = http_enabled;
                self.settings.http_port = http_port;
                if web_changed { self.web_restart_needed = true; }
                // ws_enabled and ws_port are legacy — ignored
                // Only update cert/key if non-empty (remote clients send empty for security)
                if !ws_cert_file.is_empty() {
                    self.settings.websocket_cert_file = ws_cert_file;
                }
                if !ws_key_file.is_empty() {
                    self.settings.websocket_key_file = ws_key_file;
                }
                self.settings.tls_proxy_enabled = tls_proxy_enabled;
                self.settings.mouse_enabled = mouse_enabled;
                self.settings.zwj_enabled = zwj_enabled;
                self.settings.new_line_indicator = new_line_indicator;
                self.settings.tts_mode = tts::TtsMode::from_name(&tts_mode);
                self.settings.tts_speak_mode = tts::TtsSpeakMode::from_name(&tts_speak_mode);
                if self.settings.dictionary_path != dictionary_path {
                    self.settings.dictionary_path = dictionary_path;
                    self.spell_checker = SpellChecker::new(&self.settings.dictionary_path);
                }
                // Save settings to persist changes
                let _ = persistence::save_settings(self);
                // Build settings message for broadcast (uses build_global_settings_msg to avoid leaking sensitive data)
                let settings_msg = self.build_global_settings_msg();
                // Broadcast update to all clients
                self.ws_broadcast(WsMessage::GlobalSettingsUpdated {
                    settings: settings_msg,
                    input_height: self.input_height,
                });
            }
            WsMessage::UpdateActions { actions } => {
                // Update actions from remote client
                self.settings.actions = actions.clone();
                compile_all_action_regexes(&mut self.settings.actions);
                // Save settings to persist changes
                let _ = persistence::save_settings(self);
                // Broadcast update to all clients
                self.ws_broadcast(WsMessage::ActionsUpdated {
                    actions,
                });
            }
            WsMessage::CalculateNextWorld { current_index } => {
                // Calculate next world using shared logic
                let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
                    .map(|w| crate::util::WorldSwitchInfo {
                        name: w.name.clone(),
                        connected: w.connected,
                        unseen_lines: w.unseen_lines,
                        pending_lines: w.pending_lines.len(),
                        first_unseen_at: w.first_unseen_at,
                    })
                    .collect();
                let next_idx = crate::util::calculate_next_world(
                    &world_info,
                    current_index,
                    self.settings.world_switch_mode,
                );
                self.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: next_idx });
            }
            WsMessage::CalculatePrevWorld { current_index } => {
                // Calculate prev world using shared logic
                let world_info: Vec<crate::util::WorldSwitchInfo> = self.worlds.iter()
                    .map(|w| crate::util::WorldSwitchInfo {
                        name: w.name.clone(),
                        connected: w.connected,
                        unseen_lines: w.unseen_lines,
                        pending_lines: w.pending_lines.len(),
                        first_unseen_at: w.first_unseen_at,
                    })
                    .collect();
                let prev_idx = crate::util::calculate_prev_world(
                    &world_info,
                    current_index,
                    self.settings.world_switch_mode,
                );
                self.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: prev_idx });
            }
            WsMessage::CalculateOldestPending { current_index } => {
                // Find world with oldest pending output (for Escape+w)
                // Priority: 1) oldest pending, 2) any unseen, 3) previous world
                let mut oldest_idx: Option<usize> = None;
                let mut oldest_time: Option<std::time::Instant> = None;

                // Check for worlds with pending output
                for (idx, world) in self.worlds.iter().enumerate() {
                    if idx == current_index || world.pending_lines.is_empty() {
                        continue;
                    }
                    if let Some(pending_time) = world.pending_since {
                        if oldest_time.is_none() || pending_time < oldest_time.unwrap() {
                            oldest_time = Some(pending_time);
                            oldest_idx = Some(idx);
                        }
                    }
                }

                // If no pending, check for unseen output
                if oldest_idx.is_none() {
                    for (idx, world) in self.worlds.iter().enumerate() {
                        if idx != current_index && world.unseen_lines > 0 {
                            oldest_idx = Some(idx);
                            break;
                        }
                    }
                }

                // If still none, use previous world
                if oldest_idx.is_none() {
                    if let Some(prev_idx) = self.previous_world_index {
                        if prev_idx != current_index && prev_idx < self.worlds.len() {
                            oldest_idx = Some(prev_idx);
                        }
                    }
                }

                self.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: oldest_idx });
            }
            WsMessage::RequestState => {
                // Client requested full state resync - send initial state
                let initial_state = self.build_initial_state();
                self.ws_send_initial_state_and_mark(client_id, initial_state);
                // Set client's initial world so broadcast_to_world_viewers works immediately
                self.ws_set_client_world(client_id, Some(self.current_world_index));
                // Also send current activity count
                self.ws_send_to_client(client_id, WsMessage::ActivityUpdate {
                    count: self.activity_count(),
                });
            }
            WsMessage::RequestWorldState { world_index } => {
                // Client switched to a world and needs current state
                if world_index < self.worlds.len() {
                    let world = &self.worlds[world_index];
                    // Build recent lines from output_lines (last 100 lines for context)
                    let recent_lines: Vec<TimestampedLine> = world.output_lines
                        .iter()
                        .rev()
                        .take(100)
                        .map(|line| TimestampedLine {
                            text: line.text.clone(),
                            ts: line.timestamp.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                            gagged: line.gagged,
                            from_server: line.from_server,
                            seq: line.seq,
                            highlight_color: line.highlight_color.clone(),
                            marked_new: line.marked_new,
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();

                    let pending_count = world.pending_lines.len();

                    self.ws_send_to_client(client_id, WsMessage::WorldStateResponse {
                        world_index,
                        pending_count,
                        prompt: world.prompt.clone(),
                        scroll_offset: world.scroll_offset,
                        recent_lines,
                    });
                }
            }
            WsMessage::BanListRequest => {
                // Send current ban list to client
                let bans = self.ban_list.get_ban_info();
                self.ws_send_to_client(client_id, WsMessage::BanListResponse { bans });
            }
            WsMessage::UnbanRequest { host } => {
                if self.ban_list.remove_ban(&host) {
                    // Save settings to persist the change
                    let _ = persistence::save_settings(self);
                    // Broadcast updated ban list to all clients
                    self.ws_broadcast(WsMessage::BanListResponse { bans: self.ban_list.get_ban_info() });
                    self.ws_send_to_client(client_id, WsMessage::UnbanResult { success: true, host, error: None });
                } else {
                    self.ws_send_to_client(client_id, WsMessage::UnbanResult { success: false, host, error: Some("No ban found".to_string()) });
                }
            }
            // Theme editor messages
            WsMessage::RequestThemeEditorState => {
                let themes_json = self.theme_file.to_json_all();
                let theme_names: Vec<String> = self.theme_file.themes.keys().cloned().collect();
                let active_theme = self.settings.gui_theme.name().to_string();
                self.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                    themes_json,
                    theme_names,
                    active_theme,
                });
            }
            WsMessage::UpdateThemeColors { theme_name, colors_json } => {
                let base = if theme_name == "light" {
                    theme::ThemeColors::light_default()
                } else {
                    theme::ThemeColors::dark_default()
                };
                let colors = theme::ThemeColors::from_json(&colors_json, &base);
                self.theme_file.set_theme(&theme_name, colors);
                // If updated theme is the active GUI theme, broadcast CSS vars update
                if theme_name == self.settings.gui_theme.name() {
                    let css_vars = self.gui_theme_colors().to_css_vars();
                    let colors_json = self.gui_theme_colors().to_json();
                    self.ws_broadcast(WsMessage::ThemeCssVarsUpdated {
                        css_vars,
                        colors_json: colors_json.clone(),
                    });
                    // Also broadcast GlobalSettingsUpdated for GUI clients
                    let settings_msg = self.build_global_settings_msg();
                    self.ws_broadcast(WsMessage::GlobalSettingsUpdated {
                        settings: settings_msg,
                        input_height: self.input_height,
                    });
                }
            }
            WsMessage::AddTheme { name, copy_from } => {
                let base_colors = self.theme_file.get(&copy_from).clone();
                self.theme_file.set_theme(&name, base_colors);
                // Send updated state back to editor
                let themes_json = self.theme_file.to_json_all();
                let theme_names: Vec<String> = self.theme_file.themes.keys().cloned().collect();
                let active_theme = self.settings.gui_theme.name().to_string();
                self.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                    themes_json,
                    theme_names,
                    active_theme,
                });
            }
            WsMessage::DeleteTheme { name } => {
                self.theme_file.remove_theme(&name);
                // Send updated state back to editor
                let themes_json = self.theme_file.to_json_all();
                let theme_names: Vec<String> = self.theme_file.themes.keys().cloned().collect();
                let active_theme = self.settings.gui_theme.name().to_string();
                self.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                    themes_json,
                    theme_names,
                    active_theme,
                });
            }
            WsMessage::SaveThemeFile => {
                let content = self.theme_file.generate_file_content();
                let path = std::path::Path::new(&get_home_dir()).join(clay_filename("clay.theme.dat"));
                match std::fs::write(&path, &content) {
                    Ok(_) => {
                        self.ws_send_to_client(client_id, WsMessage::ThemeFileSaved { success: true, error: None });
                    }
                    Err(e) => {
                        self.ws_send_to_client(client_id, WsMessage::ThemeFileSaved { success: false, error: Some(e.to_string()) });
                    }
                }
            }
            WsMessage::RequestKeybindEditorState => {
                let bindings_json = self.keybindings.to_json();
                let defaults_json = keybindings::KeyBindings::tf_defaults().to_json();
                let actions_json = keybindings::KeyBindings::actions_json();
                self.ws_send_to_client(client_id, WsMessage::KeybindEditorState {
                    bindings_json,
                    defaults_json,
                    actions_json,
                });
            }
            WsMessage::UpdateKeybindEditorBindings { bindings_json } => {
                self.keybindings = keybindings::KeyBindings::from_json(&bindings_json);
                // Broadcast to all clients so web/GUI clients update live
                self.ws_broadcast(WsMessage::KeybindingsUpdated {
                    bindings_json: self.keybindings.to_json(),
                });
            }
            WsMessage::SaveKeybindFile => {
                let path = std::path::Path::new(&get_home_dir()).join(clay_filename("clay.key.dat"));
                match self.keybindings.save(&path) {
                    Ok(_) => {
                        self.ws_send_to_client(client_id, WsMessage::KeybindFileSaved { success: true, error: None });
                    }
                    Err(e) => {
                        self.ws_send_to_client(client_id, WsMessage::KeybindFileSaved { success: false, error: Some(e.to_string()) });
                    }
                }
            }
            WsMessage::ResetKeybindDefaults => {
                self.keybindings = keybindings::KeyBindings::tf_defaults();
                let bindings_json = self.keybindings.to_json();
                let defaults_json = keybindings::KeyBindings::tf_defaults().to_json();
                let actions_json = keybindings::KeyBindings::actions_json();
                self.ws_send_to_client(client_id, WsMessage::KeybindEditorState {
                    bindings_json,
                    defaults_json,
                    actions_json,
                });
                self.ws_broadcast(WsMessage::KeybindingsUpdated {
                    bindings_json: self.keybindings.to_json(),
                });
            }
            WsMessage::RequestConnectionsList => {
                // Generate connections list using same format as master console
                let current_idx = self.current_world_index;
                const KEEPALIVE_SECS: u64 = 5 * 60;
                let worlds_info: Vec<util::WorldListInfo> = self.worlds.iter().enumerate().map(|(idx, world)| {
                    let now = std::time::Instant::now();
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
                let lines: Vec<String> = output.lines().map(|s| s.to_string()).collect();
                self.ws_send_to_client(client_id, WsMessage::ConnectionsListResponse { lines });
            }
            WsMessage::ReportSeqMismatch { world_index, expected_seq_gt, actual_seq, line_text, source } => {
                if is_debug_enabled() {
                    let world_name = self.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?");
                    output_debug_log(&format!("SEQ MISMATCH [{}] in '{}': expected seq>{}, got seq={}, text={:?}",
                        source, world_name, expected_seq_gt, actual_seq,
                        line_text.chars().take(80).collect::<String>()));
                }
            }
            WsMessage::ReportDuplicate { world_index, line_seq, max_seq, line_text, source } => {
                if is_debug_enabled() {
                    let world_name = self.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?");
                    output_debug_log(&format!("DUPLICATE [{}] in '{}': line_seq={}, max_seq={}, text={:?}",
                        source, world_name, line_seq, max_seq,
                        line_text.chars().take(200).collect::<String>()));
                }
            }
            WsMessage::ClientTypeDeclaration { client_type } => {
                // Update client type in WebSocket server
                self.ws_set_client_type(client_id, client_type);
            }
            WsMessage::CycleWorld { direction } => {
                // Client requests to cycle to next/previous world
                let current = self.ws_client_worlds.get(&client_id)
                    .map(|s| s.world_index)
                    .unwrap_or(self.current_world_index);

                let new_index = if direction == "up" {
                    self.calculate_prev_world_from(current)
                } else {
                    self.calculate_next_world_from(current)
                };

                if let Some(idx) = new_index {
                    // Update client's view state (sync state)
                    let prev = self.ws_client_worlds.get(&client_id);
                    let visible_lines = prev.map(|s| s.visible_lines).unwrap_or(24);
                    let visible_columns = prev.map(|s| s.visible_columns).unwrap_or(0);
                    let dimensions = prev.and_then(|s| s.dimensions);
                    self.ws_client_worlds.insert(client_id, ClientViewState {
                        world_index: idx,
                        visible_lines,
                        visible_columns,
                        dimensions,
                    });
                    // Update client's world in WebSocket server (async state)
                    self.ws_set_client_world(client_id, Some(idx));

                    // Send world switch result with state
                    if idx < self.worlds.len() {
                        let pending_count = self.worlds[idx].pending_lines.len();
                        let paused = self.worlds[idx].paused;
                        let world_name = self.worlds[idx].name.clone();

                        self.ws_send_to_client(client_id, WsMessage::WorldSwitchResult {
                            world_index: idx,
                            world_name,
                            pending_count,
                            paused,
                        });

                        // Send initial output lines based on client type
                        let client_type = self.ws_get_client_type(client_id);
                        let world = &self.worlds[idx];
                        let total_lines = world.output_lines.len();

                        let lines_to_send = match client_type {
                            Some(websocket::RemoteClientType::RemoteConsole) => {
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
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs())
                                        .unwrap_or(0);
                                    TimestampedLine {
                                        text: line.text.clone(),
                                        ts,
                                        gagged: line.gagged,
                                        from_server: line.from_server,
                                        seq: line.seq,
                                        highlight_color: line.highlight_color.clone(),
                                        marked_new: line.marked_new,
                                    }
                                })
                                .collect();

                            self.ws_send_to_client(client_id, WsMessage::OutputLines {
                                world_index: idx,
                                lines,
                                is_initial: true,
                            });
                        }

                        // Also mark world as seen if it had unseen output
                        if self.worlds[idx].unseen_lines > 0 {
                            self.worlds[idx].unseen_lines = 0;
                            self.worlds[idx].first_unseen_at = None;
                            self.ws_broadcast(WsMessage::UnseenCleared { world_index: idx });
                            self.broadcast_activity();
                        }
                    }
                }
            }
            WsMessage::RequestScrollback { world_index, count, before_seq } => {
                // Console client requests scrollback from master
                if world_index < self.worlds.len() {
                    let world = &self.worlds[world_index];

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
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                TimestampedLine {
                                    text: line.text.clone(),
                                    ts,
                                    gagged: line.gagged,
                                    from_server: line.from_server,
                                    seq: line.seq,
                                    highlight_color: line.highlight_color.clone(),
                                    marked_new: line.marked_new,
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
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                TimestampedLine {
                                    text: line.text.clone(),
                                    ts,
                                    gagged: line.gagged,
                                    from_server: line.from_server,
                                    seq: line.seq,
                                    highlight_color: line.highlight_color.clone(),
                                    marked_new: line.marked_new,
                                }
                            })
                            .collect()
                    };

                    let backfill_complete = lines.len() < count;
                    self.ws_send_to_client(client_id, WsMessage::ScrollbackLines {
                        world_index,
                        lines,
                        backfill_complete,
                    });
                }
            }
            WsMessage::ToggleWorldGmcp { world_index } => {
                if world_index < self.worlds.len() {
                    self.worlds[world_index].gmcp_user_enabled = !self.worlds[world_index].gmcp_user_enabled;
                    // Broadcast toggle state first so clients update before receiving media
                    self.ws_broadcast(WsMessage::GmcpUserToggled {
                        world_index,
                        enabled: self.worlds[world_index].gmcp_user_enabled,
                    });
                    if self.worlds[world_index].gmcp_user_enabled {
                        self.restart_world_media(world_index);
                    } else {
                        self.stop_world_media(world_index);
                    }
                    self.needs_output_redraw = true;
                }
            }
            WsMessage::SendGmcp { world_index, package, data } => {
                if world_index < self.worlds.len() {
                    if let Some(ref tx) = self.worlds[world_index].command_tx {
                        let msg = build_gmcp_message(&package, &data);
                        let _ = tx.try_send(WriteCommand::Raw(msg));
                    }
                }
            }
            WsMessage::SendMsdp { world_index, variable, value } => {
                if world_index < self.worlds.len() {
                    if let Some(ref tx) = self.worlds[world_index].command_tx {
                        let msg = build_msdp_set(&variable, &value);
                        let _ = tx.try_send(WriteCommand::Raw(msg));
                    }
                }
            }
            WsMessage::PongCheck { nonce } => {
                // Client responded to a /remote liveness check
                if nonce == self.remote_ping_nonce {
                    if let Some(ref responses) = self.remote_ping_responses {
                        if let Ok(mut set) = responses.lock() {
                            set.insert(client_id);
                        }
                    }
                }
            }
            _ => {
                // Other message types handled elsewhere or ignored
            }
        }
        WsAsyncAction::Done
    }

    /// Build initial state message for a newly authenticated client.
    /// Only sends output_lines (not pending_lines) - clients see the More indicator
    /// and release pending via PgDn/Tab, avoiding duplicate line bugs.
    fn build_initial_state(&self) -> WsMessage {
        // Send only the most recent lines in InitialState for fast initial load.
        // Clients backfill remaining history via RequestScrollback after rendering.
        const MAX_INITIAL_LINES: usize = 100;

        let worlds: Vec<WorldStateMsg> = self.worlds.iter().enumerate().map(|(idx, world)| {
            // Create timestamped versions (add sparkle prefix for client-generated messages)
            // Only include output_lines - pending_lines stay on the server and are
            // released via PgDn/Tab, then broadcast to clients normally.
            // Exclude trailing partial line (text without trailing newline, e.g., prompt).
            // It will be included in the next broadcast when completed by subsequent data.
            let has_trailing_partial = !world.partial_line.is_empty() && !world.partial_in_pending;
            let total_lines = if has_trailing_partial {
                world.output_lines.len().saturating_sub(1)
            } else {
                world.output_lines.len()
            };
            // Find skip index so that at least MAX_INITIAL_LINES *visible* (non-gagged) lines
            // are included. Walk backwards counting visible lines.
            let skip = {
                let mut visible_count = 0;
                let mut start = total_lines;
                for i in (0..total_lines).rev() {
                    start = i;
                    if !world.output_lines[i].gagged {
                        visible_count += 1;
                        if visible_count >= MAX_INITIAL_LINES {
                            break;
                        }
                    }
                }
                start
            };
            let output_lines_ts: Vec<TimestampedLine> = world.output_lines.iter()
                .skip(skip)
                .take(total_lines - skip)
                .map(|s| {
                    let text = s.text.replace('\r', "");
                    let text = if !s.from_server {
                        format!("✨ {}", text)
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
                        marked_new: s.marked_new,
                    }
                })
                .collect();
            let output_len = output_lines_ts.len();
            let pending_lines_ts: Vec<TimestampedLine> = Vec::new();
            WorldStateMsg {
                index: idx,
                name: world.name.clone(),
                connected: world.connected,
                // Legacy output_lines left empty - all clients prefer output_lines_ts
                output_lines: Vec::new(),
                pending_lines: Vec::new(),
                output_lines_ts,
                pending_lines_ts,
                prompt: world.prompt.replace('\r', ""),
                // Set scroll_offset to end of output lines so client starts at bottom
                scroll_offset: output_len.saturating_sub(1),
                // Report server's paused state so client shows More indicator
                paused: world.paused,
                unseen_lines: world.unseen_lines,
                settings: WorldSettingsMsg {
                    hostname: world.settings.hostname.clone(),
                    port: world.settings.port.clone(),
                    user: world.settings.user.clone(),
                    password: {
                        let p = persistence::decrypt_password(&world.settings.password);
                        if p.starts_with("ENC:") { String::new() } else { p }
                    },
                    has_password: !world.settings.password.is_empty(),
                    use_ssl: world.settings.use_ssl,
                    log_enabled: world.settings.log_enabled,
                    encoding: world.settings.encoding.name().to_string(),
                    auto_connect_type: world.settings.auto_connect_type.name().to_string(),
                    keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                    keep_alive_cmd: world.settings.keep_alive_cmd.clone(),
                    gmcp_packages: world.settings.gmcp_packages.clone(),
                    auto_reconnect_secs: world.settings.auto_reconnect_display(),
                },
                last_send_secs: world.last_send_time.map(|t| t.elapsed().as_secs()),
                last_recv_secs: world.last_receive_time.map(|t| t.elapsed().as_secs()),
                last_nop_secs: world.last_nop_time.map(|t| t.elapsed().as_secs()),
                keep_alive_type: world.settings.keep_alive_type.name().to_string(),
                showing_splash: world.showing_splash,
                was_connected: world.was_connected,
                is_proxy: world.proxy_pid.is_some(),
                gmcp_user_enabled: world.gmcp_user_enabled,
                total_output_lines: world.output_lines.len(),
                pending_count: world.pending_lines.len(),
            }
        }).collect();

        let settings = self.build_global_settings_msg();

        WsMessage::InitialState {
            worlds,
            settings,
            current_world_index: self.current_world_index,
            actions: self.settings.actions.clone(),
            splash_lines: generate_splash_strings(),
        }
    }

    fn increase_input_height(&mut self) {
        if self.input_height < 15 {
            self.input_height += 1;
            self.input.visible_height = self.input_height;
        }
    }

    fn decrease_input_height(&mut self) {
        if self.input_height > 1 {
            self.input_height -= 1;
            self.input.visible_height = self.input_height;
            self.input.adjust_viewport();
        }
    }

    fn handle_spell_check(&mut self) {
        if !self.spell_state.showing_suggestions {
            if let Some((start, end, word)) = self.input.current_word() {
                if !self.spell_checker.is_valid(&word) {
                    let mut suggestions = self.spell_checker.suggestions(&word, 6);
                    if !suggestions.is_empty() {
                        // Store original word and add it to the end for cycling
                        self.spell_state.original_word = word.clone();
                        suggestions.push(word);  // Add original word at the end

                        // Output suggestions to the output area (excluding the original word)
                        let display_suggestions: Vec<_> = suggestions[..suggestions.len()-1].to_vec();
                        self.add_output(&format!(
                            "Suggestions for '{}': {}",
                            self.spell_state.original_word,
                            display_suggestions.join(", ")
                        ));

                        self.spell_state.suggestions = suggestions;
                        self.spell_state.suggestion_index = 0;
                        self.spell_state.word_start = start;
                        self.spell_state.word_end = end;
                        self.spell_state.showing_suggestions = true;
                        self.suggestion_message = Some(format!(
                            "Press Ctrl+Q to cycle: {}",
                            self.spell_state.suggestions[0]
                        ));
                    }
                    // If no suggestions found or word is correctly spelled, do nothing
                }
                // If word is spelled correctly, do nothing
            }
            // If no word at cursor, do nothing
        } else if !self.spell_state.suggestions.is_empty() {
            // Cycle through suggestions (including original word at the end)
            let replacement = self.spell_state.suggestions[self.spell_state.suggestion_index].clone();
            self.input.replace_word(
                self.spell_state.word_start,
                self.spell_state.word_end,
                &replacement,
            );
            self.spell_state.word_end = self.spell_state.word_start + replacement.chars().count();
            self.spell_state.suggestion_index =
                (self.spell_state.suggestion_index + 1) % self.spell_state.suggestions.len();

            let next_word = &self.spell_state.suggestions[self.spell_state.suggestion_index];
            if next_word == &self.spell_state.original_word {
                self.suggestion_message = Some(format!(
                    "Applied '{}'. Next: '{}' (original)",
                    replacement, next_word
                ));
            } else {
                self.suggestion_message = Some(format!(
                    "Applied '{}'. Next: '{}'",
                    replacement, next_word
                ));
            }
        }
    }

    fn check_word_ended(&mut self) {
        if self.spell_state.showing_suggestions {
            // Convert byte cursor to character position for comparison
            let cursor_char_pos = self.input.buffer[..self.input.cursor_position].chars().count();
            let chars: Vec<char> = self.input.buffer.chars().collect();

            // Allow cursor to be one position past the word if there's a non-word character there
            // This handles "thiss |" where user typed a space after the misspelled word
            let effective_word_end = if self.spell_state.word_end < chars.len()
                && !chars[self.spell_state.word_end].is_alphabetic()
            {
                self.spell_state.word_end + 1
            } else {
                self.spell_state.word_end
            };

            if cursor_char_pos < self.spell_state.word_start || cursor_char_pos > effective_word_end {
                self.spell_state.reset();
                self.suggestion_message = None;
            }
        }
    }

    /// Check for temperature patterns and convert them when followed by a separator.
    /// Patterns: 32F, 32f, 100C, 100c, 32°F, 32.5F, -10C, etc.
    /// When detected, inserts conversion in parentheses: "32F " -> "32F (0C) "
    fn check_temp_conversion(&mut self) {
        // Only convert temperatures when enabled
        if !self.settings.temp_convert_enabled {
            return;
        }

        // Don't convert when user is deleting (backspace/delete) - allows undoing conversion
        if self.last_input_was_delete {
            return;
        }

        let chars: Vec<char> = self.input.buffer.chars().collect();
        if chars.is_empty() {
            return;
        }

        // Only check when cursor is at the end (just typed a character)
        let cursor_char_pos = self.input.buffer[..self.input.cursor_position].chars().count();
        if cursor_char_pos != chars.len() {
            return;
        }

        // Check if we just typed a separator after a temperature
        let last_char = chars[chars.len() - 1];
        if !last_char.is_whitespace() && !matches!(last_char, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}') {
            return;
        }

        // Look backwards for a temperature pattern before the separator
        // Pattern: optional minus, digits, optional decimal+digits, optional °, F or C
        let end = chars.len() - 1; // Position of the separator
        if end == 0 {
            return;
        }

        // Find the F/C unit character
        let unit_pos = end - 1;
        let unit_char = chars[unit_pos].to_ascii_uppercase();
        if unit_char != 'F' && unit_char != 'C' {
            return;
        }

        // Check for optional degree symbol before the unit
        let mut num_end = unit_pos;
        if num_end > 0 && chars[num_end - 1] == '°' {
            num_end -= 1;
        }

        // Find the start of the number (digits, optional decimal, optional leading minus)
        let mut num_start = num_end;
        let mut found_digit = false;
        let mut found_decimal = false;

        while num_start > 0 {
            let c = chars[num_start - 1];
            if c.is_ascii_digit() {
                found_digit = true;
                num_start -= 1;
            } else if c == '.' && !found_decimal {
                found_decimal = true;
                num_start -= 1;
            } else if c == '-' {
                // Allow minus at the very start of the number
                num_start -= 1;
                break;
            } else {
                break;
            }
        }

        // Check we have at least one digit
        if !found_digit {
            return;
        }

        // Make sure the character before the number isn't part of the "word"
        // (e.g., "abc32F" shouldn't trigger, but "test 32F" should)
        if num_start > 0 {
            let prev_char = chars[num_start - 1];
            if prev_char.is_alphanumeric() || prev_char == '_' {
                return;
            }
        }

        // Build the full temperature string (e.g., "21F", "-5.5°C")
        let temp_str: String = chars[num_start..=unit_pos].iter().collect();

        // Check if this temperature was already converted and undone - skip if so
        if let Some(ref skip) = self.skip_temp_conversion {
            if skip == &temp_str {
                return;
            }
        }

        // Parse the number
        let num_str: String = chars[num_start..num_end].iter().collect();
        let temp: f64 = match num_str.parse() {
            Ok(t) => t,
            Err(_) => return,
        };

        // Convert temperature
        let (converted, converted_unit) = if unit_char == 'F' {
            // Fahrenheit to Celsius: (F - 32) * 5/9
            ((temp - 32.0) * 5.0 / 9.0, 'C')
        } else {
            // Celsius to Fahrenheit: C * 9/5 + 32
            (temp * 9.0 / 5.0 + 32.0, 'F')
        };

        // Format the conversion - use integer if whole number, else one decimal
        // No space before the parenthesis - the separator the user typed goes after
        let converted_str = if (converted - converted.round()).abs() < 0.05 {
            format!("({:.0}{})", converted, converted_unit)
        } else {
            format!("({:.1}{})", converted, converted_unit)
        };

        // Remember this temperature so we don't re-convert if user undoes it
        self.skip_temp_conversion = Some(temp_str);

        // Insert the conversion before the separator
        // Build new buffer: [before separator] + conversion + [separator]
        let before_sep: String = chars[..end].iter().collect();
        let sep: String = chars[end..].iter().collect();
        self.input.buffer = format!("{}{}{}", before_sep, converted_str, sep);

        // Move cursor to after the conversion and separator
        self.input.cursor_position = self.input.buffer.len();
    }

    fn find_misspelled_words(&mut self) -> Vec<(usize, usize)> {
        let mut misspelled = Vec::new();
        let chars: Vec<char> = self.input.buffer.chars().collect();
        let mut i = 0;

        // Helper to check if a character at position is part of a word
        // (alphabetic, or apostrophe between alphabetic characters)
        let is_word_char = |pos: usize| -> bool {
            if pos >= chars.len() {
                return false;
            }
            let c = chars[pos];
            if c.is_alphabetic() {
                return true;
            }
            // Include apostrophe if between alphabetic characters (contractions)
            if c == '\'' {
                let has_alpha_before = pos > 0 && chars[pos - 1].is_alphabetic();
                let has_alpha_after = pos + 1 < chars.len() && chars[pos + 1].is_alphabetic();
                return has_alpha_before && has_alpha_after;
            }
            false
        };

        // Convert byte cursor to character position
        let cursor_char_pos = self.input.buffer[..self.input.cursor_position].chars().count();
        let cached = &self.cached_misspelled;

        // Helper to check if a word overlaps with any cached misspelled range
        let is_cached_misspelled = |start: usize, end: usize| -> bool {
            cached.iter().any(|(cs, ce)| start < *ce && end > *cs)
        };

        // Helper to check if followed by separator
        let has_separator = |end_pos: usize| -> bool {
            if end_pos >= chars.len() {
                return false;
            }
            let next_char = chars[end_pos];
            next_char.is_whitespace() || matches!(next_char, '.' | ',' | '!' | '?' | ';' | ':' | ')' | ']' | '}' | '"' | '%' | '@' | '#' | '$' | '^' | '&' | '*' | '(' | '[' | '{')
        };

        while i < chars.len() {
            // Skip non-word characters
            while i < chars.len() && !chars[i].is_alphabetic() {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }

            let start = i;
            // Continue while we have word characters (including internal apostrophes)
            while i < chars.len() && is_word_char(i) {
                i += 1;
            }
            let end = i;

            let word: String = chars[start..end].iter().collect();
            // Don't check if cursor is inside the word (actively typing)
            let cursor_in_word = cursor_char_pos >= start && cursor_char_pos < end;

            if cursor_in_word {
                // Cursor inside word - don't flag
                continue;
            }

            let at_end_of_input = end >= chars.len();
            let cursor_at_word_end = cursor_char_pos == end;

            if at_end_of_input && cursor_at_word_end {
                // Word at end of input with cursor right at the end
                // Use cached state - if word overlaps with cached misspelled, keep it flagged
                // This keeps words flagged while typing/backspacing until completed again
                if is_cached_misspelled(start, end) {
                    misspelled.push((start, end));
                }
                // If not in cache, don't flag - user is typing a fresh word
            } else if at_end_of_input {
                // Word at end of input but cursor moved away - check spelling
                if !self.spell_checker.is_valid(&word) {
                    misspelled.push((start, end));
                }
            } else if has_separator(end) {
                // Word followed by separator - check spelling
                if !self.spell_checker.is_valid(&word) {
                    misspelled.push((start, end));
                }
            }
            // else: word not followed by separator and not at end - don't check
        }

        // Update cache with current result
        self.cached_misspelled = misspelled.clone();
        misspelled
    }

    fn scroll_output_up(&mut self) {
        let more_mode = self.settings.more_mode_enabled;
        let target_visual_lines = (self.output_height as usize).saturating_sub(2).max(1);
        let visible_height = (self.output_height as usize).max(1);
        let width = (self.output_width as usize).max(1);
        let world = self.current_world_mut();
        world.visual_line_offset = 0;

        // Calculate the minimum scroll_offset where line 0 is at the top
        // This is where all content from line 0 to scroll_offset fits in visible_height
        let mut min_offset = 0usize;
        let mut visual_lines = 0usize;
        for (idx, line) in world.output_lines.iter().enumerate() {
            visual_lines += visual_line_count(&line.text, width);
            if visual_lines >= visible_height {
                min_offset = idx;
                break;
            }
            min_offset = idx;
        }

        // If already at or past the minimum, don't scroll further
        if world.scroll_offset <= min_offset {
            // Still enable pause mode if more_mode is on
            if more_mode && !world.paused {
                world.paused = true;
            }
            return;
        }

        // Count lines being scrolled off (from scroll_offset going backwards)
        // These are the lines that will disappear from the bottom
        let mut visual_lines_moved = 0;
        let mut new_offset = world.scroll_offset;

        while visual_lines_moved < target_visual_lines {
            visual_lines_moved += visual_line_count(&world.output_lines[new_offset].text, width);
            if new_offset == 0 {
                break;
            }
            new_offset -= 1;
        }

        // Clamp to minimum offset
        world.scroll_offset = new_offset.max(min_offset);
        if more_mode && !world.paused {
            world.paused = true;
        }
        // Mark output for redraw
        self.needs_output_redraw = true;
    }

    fn scroll_output_up_by(&mut self, lines: usize) {
        let more_mode = self.settings.more_mode_enabled;
        let visible_height = (self.output_height as usize).max(1);
        let width = (self.output_width as usize).max(1);
        let world = self.current_world_mut();
        world.visual_line_offset = 0;

        let mut min_offset = 0usize;
        let mut visual_lines = 0usize;
        for (idx, line) in world.output_lines.iter().enumerate() {
            visual_lines += visual_line_count(&line.text, width);
            if visual_lines >= visible_height {
                min_offset = idx;
                break;
            }
            min_offset = idx;
        }

        if world.scroll_offset <= min_offset {
            if more_mode && !world.paused {
                world.paused = true;
            }
            return;
        }

        let mut visual_lines_moved = 0;
        let mut new_offset = world.scroll_offset;
        while visual_lines_moved < lines {
            visual_lines_moved += visual_line_count(&world.output_lines[new_offset].text, width);
            if new_offset == 0 {
                break;
            }
            new_offset -= 1;
        }

        world.scroll_offset = new_offset.max(min_offset);
        if more_mode && !world.paused {
            world.paused = true;
        }
        self.needs_output_redraw = true;
    }

    fn scroll_output_down(&mut self) {
        let target_visual_lines = (self.output_height as usize).saturating_sub(2).max(1);
        let width = (self.output_width as usize).max(1);
        let world = self.current_world_mut();
        world.visual_line_offset = 0;
        let max_scroll = world.output_lines.len().saturating_sub(1);

        if world.scroll_offset >= max_scroll {
            return; // Already at bottom
        }

        // Count lines being scrolled in (from scroll_offset+1 going forwards)
        // These are the lines that will appear at the bottom
        let mut visual_lines_moved = 0;
        let mut new_offset = world.scroll_offset + 1;

        while new_offset <= max_scroll && visual_lines_moved < target_visual_lines {
            visual_lines_moved += visual_line_count(&world.output_lines[new_offset].text, width);
            new_offset += 1;
        }

        // new_offset is one past the last line counted, so subtract 1
        world.scroll_offset = (new_offset - 1).min(max_scroll);

        // Mark output for redraw
        self.needs_output_redraw = true;
    }

    /// Release one screenful of pending lines and broadcast to WebSocket clients.
    /// Used by both Tab and PgDn when at the bottom and paused.
    pub(crate) fn release_pending_screenful(&mut self) {
        let mut visual_budget = (self.output_height as usize).saturating_sub(2);
        let output_width = self.output_width as usize;
        let world_idx = self.current_world_index;
        let width = output_width.max(1);

        // If we have a partially-shown line, first reveal more of it
        if self.worlds[world_idx].visual_line_offset > 0 {
            // Find the actual partially-shown line (walk back past gagged lines)
            let scroll_idx = self.worlds[world_idx].scroll_offset;
            let mut partial_idx = scroll_idx;
            while partial_idx > 0 && partial_idx < self.worlds[world_idx].output_lines.len()
                && self.worlds[world_idx].output_lines[partial_idx].gagged {
                partial_idx -= 1;
            }
            if partial_idx < self.worlds[world_idx].output_lines.len() {
                let total_vl = wrap_ansi_line(&self.worlds[world_idx].output_lines[partial_idx].text, width).len().max(1);
                let remaining = total_vl.saturating_sub(self.worlds[world_idx].visual_line_offset);
                if remaining > visual_budget {
                    // More of this line than fits on screen — advance offset, done
                    self.worlds[world_idx].visual_line_offset += visual_budget;
                    self.needs_output_redraw = true;
                    return;
                }
                // Remaining fits — clear partial, reduce budget for pending release
                self.worlds[world_idx].visual_line_offset = 0;
                visual_budget = visual_budget.saturating_sub(remaining);
                if visual_budget == 0 {
                    self.needs_output_redraw = true;
                    return;
                }
            } else {
                self.worlds[world_idx].visual_line_offset = 0;
            }
        }

        // Pre-calculate how many logical lines fit in the visual budget
        // (mirrors the logic in release_pending so we can collect lines for broadcasting)
        let mut visual_total = 0;
        let mut logical_count = 0;
        for line in &self.worlds[world_idx].pending_lines {
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
        if logical_count == 0 && !self.worlds[world_idx].pending_lines.is_empty() {
            logical_count = 1;
        }

        // Collect per-line text and marked_new for broadcasting
        let lines_with_flags: Vec<(String, bool)> = self.worlds[world_idx]
            .pending_lines
            .iter()
            .take(logical_count)
            .map(|line| (line.text.replace('\r', ""), line.marked_new))
            .collect();

        let pending_before = self.worlds[world_idx].pending_lines.len();
        self.current_world_mut().release_pending(visual_budget, output_width);
        let released = pending_before - self.worlds[world_idx].pending_lines.len();

        // Broadcast released lines to clients viewing this world.
        // Group consecutive lines by marked_new so each gets the correct indicator.
        if !lines_with_flags.is_empty() {
            let ts = current_timestamp_secs();
            let mut batch: Vec<&str> = Vec::new();
            let mut batch_marked_new = lines_with_flags[0].1;
            for (text, marked_new) in &lines_with_flags {
                if *marked_new != batch_marked_new && !batch.is_empty() {
                    let ws_data = batch.join("\n") + "\n";
                    self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                        world_index: world_idx,
                        data: ws_data,
                        is_viewed: true,
                        ts,
                        from_server: true,
                        seq: 0,
                        marked_new: batch_marked_new,
                        flush: false, gagged: false,
                    });
                    batch.clear();
                    batch_marked_new = *marked_new;
                }
                batch.push(text);
            }
            if !batch.is_empty() {
                let ws_data = batch.join("\n") + "\n";
                self.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                    world_index: world_idx,
                    data: ws_data,
                    is_viewed: true,
                    ts,
                    from_server: true,
                    seq: 0,
                    marked_new: batch_marked_new,
                    flush: false, gagged: false,
                });
            }
        }

        // Mark output for redraw so released lines are rendered
        self.needs_output_redraw = true;
        self.needs_terminal_clear = true;

        // Broadcast release event so other clients sync
        self.ws_broadcast(WsMessage::PendingReleased { world_index: world_idx, count: released });
        let pending = self.worlds[world_idx].pending_lines.len();
        self.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: pending });
        // Broadcast activity count since pending lines changed
        self.broadcast_activity();
        self.needs_output_redraw = true;
    }
}

pub enum AppEvent {
    ServerData(String, Vec<u8>),  // world_name, raw bytes
    Disconnected(String, u64),     // world_name, connection_id
    TelnetDetected(String),       // world_name - telnet negotiation detected
    Prompt(String, Vec<u8>),      // world_name, prompt bytes (from telnet GA)
    WontEchoSeen(String),         // world_name - IAC WONT ECHO detected (for timeout-based prompts)
    NawsRequested(String),        // world_name - server sent DO NAWS (we should send window size)
    TtypeRequested(String),       // world_name - server sent SB TTYPE SEND (we should send terminal type)
    CharsetRequested(String, Vec<String>), // world_name, offered charsets - server sent CHARSET REQUEST (RFC 2066)
    SystemMessage(String),       // message to display in current world's output
    Sigusr1Received,             // SIGUSR1 received - trigger hot reload (not available on Android)
    // Background connection events
    ConnectionSuccess(String, mpsc::Sender<WriteCommand>, Option<SocketFd>, bool),  // world_name, cmd_tx, socket_fd, is_tls
    ConnectionFailed(String, String),  // world_name, error_message
    // WebSocket events
    WsClientConnected(u64),                    // client_id
    WsClientDisconnected(u64),                 // client_id
    WsClientMessage(u64, Box<WsMessage>),      // client_id, message
    WsAuthKeyValidation(u64, Box<WsMessage>, String, String),   // client_id, AuthRequest with auth_key, client_ip, challenge
    WsKeyRequest(u64),                         // client_id - generate and send new auth key
    WsKeyRevoke(u64, String),                  // client_id, auth_key to revoke
    // Multiuser mode events (include username for per-user connection isolation)
    ConnectWorldRequest(usize, String),  // world_index, requesting username
    MultiuserServerData(usize, String, Vec<u8>),  // world_index, username, raw bytes
    MultiuserDisconnected(usize, String),         // world_index, username
    MultiuserTelnetDetected(usize, String),       // world_index, username
    MultiuserPrompt(usize, String, Vec<u8>),      // world_index, username, prompt bytes
    // Slack/Discord events
    SlackMessage(String, String), // world_name, formatted message
    DiscordMessage(String, String), // world_name, formatted message
    // GMCP/MSDP events
    GmcpNegotiated(String),                   // world_name
    MsdpNegotiated(String),                   // world_name
    GmcpReceived(String, String, String),     // world_name, package, json_data
    MsdpReceived(String, String, String),     // world_name, variable, value_json
    // Media file downloaded and ready to play
    MediaFileReady(usize, String, std::path::PathBuf, i64, i64, bool),  // world_idx, key, path, volume, loops, is_music
    // API lookup result (dict/urban/translate) from spawned task
    ApiLookupResult(u64, usize, Result<String, String>, bool),  // client_id, world_index, Ok(input_text) or Err(error), cursor_start
    // /remote ping check result (after 2s timeout)
    RemoteListResult(u64, usize, Vec<String>),  // requesting_client_id (0 = console), world_index, output lines
    // Result from background update check/download
    UpdateResult(Result<UpdateSuccess, String>),
}

/// Successful update download ready to install
pub struct UpdateSuccess {
    pub version: String,
    pub temp_path: std::path::PathBuf,
}

/// Per-user connection state for multiuser mode
/// Each user has their own independent connection to each world
#[derive(Clone)]
pub struct UserConnection {
    pub connected: bool,
    pub command_tx: Option<mpsc::Sender<WriteCommand>>,
    output_lines: Vec<OutputLine>,
    pending_lines: Vec<OutputLine>,
    pub scroll_offset: usize,
    pub unseen_lines: usize,
    pub paused: bool,
    pub lines_since_pause: usize,
    pub telnet_mode: bool,
    pub prompt: String,
    pub prompt_count: usize,
    pub last_send_time: Option<std::time::Instant>,
    pub last_receive_time: Option<std::time::Instant>,
    pub partial_line: String,
    pub partial_in_pending: bool,
}

impl Default for UserConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl UserConnection {
    pub fn new() -> Self {
        Self {
            connected: false,
            command_tx: None,
            output_lines: Vec::new(),
            pending_lines: Vec::new(),
            scroll_offset: 0,
            unseen_lines: 0,
            paused: false,
            lines_since_pause: 0,
            telnet_mode: false,
            prompt: String::new(),
            prompt_count: 0,
            last_send_time: None,
            last_receive_time: None,
            partial_line: String::new(),
            partial_in_pending: false,
        }
    }
}

pub fn get_settings_path() -> PathBuf {
    // Use custom config path if set via --conf=<path>
    if let Some(custom_path) = get_custom_config_path() {
        return custom_path.clone();
    }
    let home = get_home_dir();
    PathBuf::from(home).join(clay_filename("clay.dat"))
}

pub fn get_multiuser_settings_path() -> PathBuf {
    let home = get_home_dir();
    PathBuf::from(home).join(clay_filename("clay.multiuser.dat"))
}

fn get_debug_log_path() -> PathBuf {
    let home = get_home_dir();
    PathBuf::from(home).join("clay.debug.log")
}

/// Write a session startup header to a debug log file.
/// Called once per file per session (startup/reload).
fn write_debug_header(file: &mut std::fs::File) {
    use std::io::Write;
    let startup_secs = STARTUP_TIME.load(Ordering::Relaxed);
    let lt = local_time_from_epoch(startup_secs as i64);
    let startup_ts = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        lt.year, lt.month, lt.day,
        lt.hour, lt.minute, lt.second
    );
    let _ = writeln!(file);
    let _ = writeln!(file, "=== {} — started {} ===", get_version_string(), startup_ts);
}

/// Write a debug message to clay.debug.log if debug is enabled
fn debug_log(debug_enabled: bool, message: &str) {
    if !debug_enabled {
        return;
    }
    use std::io::Write;
    let path = get_debug_log_path();
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            // Write session header on first log entry
            if !DEBUG_LOG_HEADER_WRITTEN.swap(true, Ordering::Relaxed) {
                write_debug_header(&mut file);
            }
            let lt = local_time_now();
            let timestamp = format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                lt.year, lt.month, lt.day,
                lt.hour, lt.minute, lt.second
            );
            let _ = writeln!(file, "[{}] {}", timestamp, message);
        }
        Err(e) => {
            eprintln!("Failed to open debug log {:?}: {}", path, e);
        }
    }
}

/// Write a debug message to clay.output.debug (output/seq debugging)
pub(crate) fn output_debug_log(message: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("clay.output.debug")
    {
        // Write session header on first log entry
        if !OUTPUT_DEBUG_HEADER_WRITTEN.swap(true, Ordering::Relaxed) {
            write_debug_header(&mut f);
        }
        let _ = writeln!(f, "{}", message);
    }
}

/// Load theme file from ~/.clay.theme.dat into app.theme_file
/// If the file doesn't exist, generates a default one and loads defaults
fn load_theme_file(app: &mut App) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let theme_path = std::path::Path::new(&home).join(clay_filename("clay.theme.dat"));
    // Migrate old ~/clay.theme.dat → ~/.clay.theme.dat
    if !theme_path.exists() {
        let old_path = std::path::Path::new(&home).join("clay.theme.dat");
        if old_path.exists() {
            let _ = std::fs::rename(&old_path, &theme_path);
        }
    }
    if !theme_path.exists() {
        // Generate default theme file
        let content = theme::ThemeFile::generate_default_file();
        let _ = std::fs::write(&theme_path, content);
    }
    app.theme_file = theme::ThemeFile::load(&theme_path);
}





/// Result from handling a new popup key
pub(crate) enum NewPopupAction {
    None,
    /// Execute a command (from menu selection)
    Command(String),
    /// Confirm action with custom_data from the popup
    Confirm(std::collections::HashMap<String, String>),
    /// Confirm dialog cancelled (No/Esc) with custom_data from the popup
    ConfirmCancelled(std::collections::HashMap<String, String>),
    /// World selector action
    WorldSelector(WorldSelectorAction),
    /// World selector filter changed - need to update the list
    WorldSelectorFilter,
    /// Setup (global settings) saved
    SetupSaved(SetupSettings),
    /// Web settings saved
    WebSaved(WebSettings),
    /// Connections popup closed (just close, no action needed)
    ConnectionsClose,
    /// Actions list action (Add, Edit, Delete)
    ActionsList(ActionsListAction),
    /// Actions list filter changed
    ActionsListFilter,
    /// Action editor saved
    ActionEditorSave { action: Action, editing_index: Option<usize> },
    /// Action editor delete requested
    ActionEditorDelete { editing_index: usize },
    /// World editor saved
    WorldEditorSaved(Box<WorldEditorSettings>),
    /// World editor delete requested
    WorldEditorDelete(usize),
    /// World editor connect requested
    WorldEditorConnect(usize),
    /// Notes list action
    NotesList(NotesListAction),
}

/// Settings from the setup popup
pub(crate) struct SetupSettings {
    pub(crate) more_mode: bool,
    pub(crate) spell_check: bool,
    pub(crate) temp_convert: bool,
    pub(crate) world_switching: String,
    pub(crate) debug: bool,
    pub(crate) input_height: i64,
    pub(crate) gui_theme: String,
    pub(crate) tls_proxy: bool,
    pub(crate) dictionary_path: String,
    pub(crate) editor_side: String,
    pub(crate) mouse_enabled: bool,
    pub(crate) zwj_enabled: bool,
    pub(crate) ansi_music: bool,
    pub(crate) new_line_indicator: bool,
    pub(crate) tts_mode: String,
    pub(crate) tts_speak_mode: String,
}

/// Settings from the web popup
pub(crate) struct WebSettings {
    pub(crate) web_secure: bool,
    pub(crate) http_enabled: bool,
    pub(crate) http_port: String,
    pub(crate) ws_password: String,
    pub(crate) ws_allow_list: String,
    pub(crate) ws_cert_file: String,
    pub(crate) ws_key_file: String,
    pub(crate) auth_key: String,
}

/// Apply web settings to app and save to disk
pub(crate) fn apply_web_settings(app: &mut App, settings: &WebSettings) {
    let port_changed = app.settings.http_port != settings.http_port.parse().unwrap_or(9000);
    let secure_changed = app.settings.web_secure != settings.web_secure;
    let http_changed = app.settings.http_enabled != settings.http_enabled;
    let cert_changed = app.settings.websocket_cert_file != settings.ws_cert_file
        || app.settings.websocket_key_file != settings.ws_key_file;

    app.settings.web_secure = settings.web_secure;
    app.settings.http_enabled = settings.http_enabled;
    app.settings.http_port = settings.http_port.parse().unwrap_or(9000);
    app.settings.websocket_password = settings.ws_password.clone();
    app.settings.websocket_allow_list = settings.ws_allow_list.clone();
    app.settings.websocket_cert_file = settings.ws_cert_file.clone();
    app.settings.websocket_key_file = settings.ws_key_file.clone();

    // Update auth key if changed
    if !settings.auth_key.is_empty() {
        let current_key = app.settings.websocket_auth_key.as_ref().map(|ak| ak.key.as_str()).unwrap_or("");
        if settings.auth_key != current_key {
            app.settings.websocket_auth_key = Some(AuthKey::new(settings.auth_key.clone()));
        }
    }

    // Update the running server's allow list and password immediately (no reload needed)
    if let Some(ref server) = app.ws_server {
        server.update_allow_list(&settings.ws_allow_list);
        if !settings.ws_password.is_empty() {
            server.update_password(&settings.ws_password);
        }
    }

    let _ = persistence::save_settings(app);

    if port_changed || secure_changed || http_changed || cert_changed {
        app.web_restart_needed = true;
        app.add_output("Web settings saved. Restarting web server...");
    } else {
        app.add_output("Web settings saved.");
    }
}

/// Extract WebSettings from a confirm dialog's custom_data
pub(crate) fn web_settings_from_custom_data(data: &std::collections::HashMap<String, String>) -> WebSettings {
    WebSettings {
        web_secure: data.get("web_secure").map(|v| v == "true").unwrap_or(false),
        http_enabled: data.get("http_enabled").map(|v| v == "true").unwrap_or(false),
        http_port: data.get("http_port").cloned().unwrap_or_else(|| "9000".to_string()),
        ws_password: data.get("ws_password").cloned().unwrap_or_default(),
        ws_allow_list: data.get("ws_allow_list").cloned().unwrap_or_default(),
        ws_cert_file: data.get("ws_cert_file").cloned().unwrap_or_default(),
        ws_key_file: data.get("ws_key_file").cloned().unwrap_or_default(),
        auth_key: data.get("auth_key").cloned().unwrap_or_default(),
    }
}


/// Settings from the world editor popup
pub(crate) struct WorldEditorSettings {
    pub(crate) world_index: usize,  // Which world is being edited
    pub(crate) name: String,
    pub(crate) world_type: String,
    // MUD fields
    pub(crate) hostname: String,
    pub(crate) port: String,
    pub(crate) user: String,
    pub(crate) password: String,
    pub(crate) use_ssl: bool,
    pub(crate) log_enabled: bool,
    pub(crate) encoding: String,
    pub(crate) auto_connect: String,
    pub(crate) keep_alive: String,
    pub(crate) keep_alive_cmd: String,
    pub(crate) gmcp_packages: String,
    pub(crate) auto_reconnect_secs: String,
    // Slack fields
    pub(crate) slack_token: String,
    pub(crate) slack_channel: String,
    pub(crate) slack_workspace: String,
    // Discord fields
    pub(crate) discord_token: String,
    pub(crate) discord_guild: String,
    pub(crate) discord_channel: String,
    pub(crate) discord_dm_user: String,
}

/// Actions from the world selector popup
pub(crate) enum WorldSelectorAction {
    Connect(String),      // Connect to world by name
    Edit(String),         // Edit world by name
    Delete(String),       // Delete world by name
    Add,                  // Add new world
}

/// Actions from the actions list popup
pub(crate) enum ActionsListAction {
    Add,                  // Add new action
    Edit(usize),          // Edit action at index
    Delete(usize),        // Delete action at index
    Toggle(usize),        // Toggle enable/disable action at index
}

/// Actions from the notes list popup
pub(crate) enum NotesListAction {
    Open(String),         // Open notes for world by name
}

/// Handle input for new unified popup system
pub(crate) fn handle_new_popup_key(app: &mut App, key: KeyEvent) -> NewPopupAction {
    use crossterm::event::KeyCode::*;
    use popup::definitions::help::HELP_BTN_OK;
    use popup::definitions::confirm::{CONFIRM_BTN_YES, CONFIRM_BTN_NO};
    use popup::ElementSelection;
    use popup::definitions::world_selector::{
        SELECTOR_FIELD_FILTER,
        SELECTOR_BTN_ADD, SELECTOR_BTN_EDIT, SELECTOR_BTN_DELETE,
        SELECTOR_BTN_CONNECT, SELECTOR_BTN_CANCEL,
    };
    use popup::definitions::setup::{
        SETUP_FIELD_MORE_MODE, SETUP_FIELD_SPELL_CHECK, SETUP_FIELD_TEMP_CONVERT,
        SETUP_FIELD_WORLD_SWITCHING, SETUP_FIELD_DEBUG,
        SETUP_FIELD_INPUT_HEIGHT, SETUP_FIELD_GUI_THEME, SETUP_FIELD_TLS_PROXY,
        SETUP_FIELD_DICTIONARY, SETUP_FIELD_EDITOR_SIDE, SETUP_FIELD_MOUSE, SETUP_FIELD_ZWJ, SETUP_FIELD_ANSI_MUSIC,
        SETUP_FIELD_NEW_LINE_INDICATOR, SETUP_FIELD_TTS, SETUP_FIELD_TTS_SPEAK_MODE,
        SETUP_BTN_SAVE, SETUP_BTN_CANCEL,
    };
    use popup::definitions::web::{
        WEB_FIELD_PROTOCOL, WEB_FIELD_HTTP_ENABLED, WEB_FIELD_HTTP_PORT,
        WEB_FIELD_WS_PASSWORD, WEB_FIELD_AUTH_KEY,
        WEB_FIELD_WS_ALLOW_LIST, WEB_FIELD_WS_CERT_FILE, WEB_FIELD_WS_KEY_FILE,
        WEB_BTN_SAVE, WEB_BTN_CANCEL, WEB_BTN_REGEN_KEY, WEB_BTN_COPY_KEY,
        update_tls_visibility,
    };
    use popup::definitions::actions::{
        ACTIONS_FIELD_FILTER, ACTIONS_FIELD_LIST,
        ACTIONS_BTN_ADD, ACTIONS_BTN_EDIT, ACTIONS_BTN_DELETE, ACTIONS_BTN_CANCEL,
        EDITOR_FIELD_NAME, EDITOR_FIELD_WORLD, EDITOR_FIELD_MATCH_TYPE,
        EDITOR_FIELD_PATTERN, EDITOR_FIELD_COMMAND, EDITOR_FIELD_ENABLED, EDITOR_FIELD_STARTUP,
        EDITOR_BTN_SAVE, EDITOR_BTN_CANCEL, EDITOR_BTN_DELETE,
    };
    use popup::definitions::world_editor::{
        WORLD_FIELD_NAME, WORLD_FIELD_TYPE, WORLD_FIELD_HOSTNAME, WORLD_FIELD_PORT,
        WORLD_FIELD_USER, WORLD_FIELD_PASSWORD, WORLD_FIELD_USE_SSL, WORLD_FIELD_LOG_ENABLED,
        WORLD_FIELD_ENCODING, WORLD_FIELD_AUTO_CONNECT, WORLD_FIELD_KEEP_ALIVE, WORLD_FIELD_KEEP_ALIVE_CMD,
        WORLD_FIELD_GMCP_PACKAGES, WORLD_FIELD_AUTO_RECONNECT,
        WORLD_FIELD_SLACK_TOKEN, WORLD_FIELD_SLACK_CHANNEL, WORLD_FIELD_SLACK_WORKSPACE,
        WORLD_FIELD_DISCORD_TOKEN, WORLD_FIELD_DISCORD_GUILD, WORLD_FIELD_DISCORD_CHANNEL, WORLD_FIELD_DISCORD_DM_USER,
        WORLD_BTN_SAVE, WORLD_BTN_CANCEL, WORLD_BTN_DELETE, WORLD_BTN_CONNECT,
        WorldType as PopupWorldType, update_field_visibility,
    };

    // Clear mouse highlight on any keyboard interaction
    if let Some(state) = app.popup_manager.current_mut() {
        state.highlight = None;
    }

    // Generic help button handler: '?' shortcut or Enter/Space on help button
    {
        let should_open_help = if let Some(state) = app.popup_manager.current() {
            !state.definition.help_lines.is_empty()
                && ((matches!(key.code, crossterm::event::KeyCode::Char('?')) && !state.editing)
                    || (matches!(key.code, crossterm::event::KeyCode::Enter | crossterm::event::KeyCode::Char(' '))
                        && state.is_button_focused(popup::POPUP_BTN_HELP)))
        } else {
            false
        };
        if should_open_help {
            let help_lines = app.popup_manager.current().unwrap().definition.help_lines.clone();
            app.popup_manager.push(popup::definitions::help::create_topic_help_popup(help_lines));
            return NewPopupAction::None;
        }
    }

    let popup_id = app.popup_manager.current().map(|s| s.definition.id.clone());
    let is_menu = popup_id == Some(popup::PopupId("menu"));
    let is_confirm = app.popup_manager.current().map(|s| {
        s.definition.buttons.iter().any(|b| b.id == popup::definitions::confirm::CONFIRM_BTN_YES)
            && s.definition.buttons.iter().any(|b| b.id == popup::definitions::confirm::CONFIRM_BTN_NO)
    }).unwrap_or(false);
    let is_world_selector = popup_id == Some(popup::PopupId("world_selector"));
    let is_setup = popup_id == Some(popup::PopupId("setup"));
    let is_web = popup_id == Some(popup::PopupId("web"));
    let is_connections = popup_id == Some(popup::PopupId("connections"));
    let is_actions_list = popup_id == Some(popup::PopupId("actions_list"));
    let is_action_editor = popup_id == Some(popup::PopupId("action_editor"));
    let is_world_editor = popup_id == Some(popup::PopupId("world_editor"));
    let is_notes_list = popup_id == Some(popup::PopupId("notes_list"));

    if let Some(state) = app.popup_manager.current_mut() {
        // World selector has special handling
        if is_world_selector {
            // Get selected world name before any state mutations
            let get_selected = || state.get_selected_list_item().map(|item| item.id.clone());

            // Check if we're editing the filter field
            let is_editing_filter = state.editing && state.is_field_selected(SELECTOR_FIELD_FILTER);

            // When editing filter, handle text input
            if is_editing_filter {
                match key.code {
                    Esc => {
                        state.commit_edit();
                        // Apply filter after editing
                        return NewPopupAction::WorldSelectorFilter;
                    }
                    Tab => {
                        // Tab exits filter field and goes to buttons
                        state.commit_edit();
                        state.cycle_field_buttons();
                        return NewPopupAction::WorldSelectorFilter;
                    }
                    Enter => {
                        state.commit_edit();
                        // Apply filter and stay in popup
                        return NewPopupAction::WorldSelectorFilter;
                    }
                    Backspace => {
                        state.backspace();
                        return NewPopupAction::WorldSelectorFilter;
                    }
                    Delete => {
                        state.delete_char();
                        return NewPopupAction::WorldSelectorFilter;
                    }
                    Left => {
                        state.cursor_left();
                    }
                    Right => {
                        state.cursor_right();
                    }
                    Home => {
                        state.cursor_home();
                    }
                    End => {
                        state.cursor_end();
                    }
                    Char(c) => {
                        // Insert character into filter
                        state.insert_char(c);
                        return NewPopupAction::WorldSelectorFilter;
                    }
                    _ => {}
                }
                return NewPopupAction::None;
            }

            // Not editing - handle normal navigation and shortcuts
            match key.code {
                Esc => {
                    app.popup_manager.close();
                }
                Enter => {
                    if state.is_on_button() {
                        if state.is_button_focused(SELECTOR_BTN_CONNECT) {
                            if let Some(name) = get_selected() {
                                app.popup_manager.close();
                                return NewPopupAction::WorldSelector(WorldSelectorAction::Connect(name));
                            }
                        } else if state.is_button_focused(SELECTOR_BTN_EDIT) {
                            if let Some(name) = get_selected() {
                                app.popup_manager.close();
                                return NewPopupAction::WorldSelector(WorldSelectorAction::Edit(name));
                            }
                        } else if state.is_button_focused(SELECTOR_BTN_DELETE) {
                            if let Some(name) = get_selected() {
                                app.popup_manager.close();
                                return NewPopupAction::WorldSelector(WorldSelectorAction::Delete(name));
                            }
                        } else if state.is_button_focused(SELECTOR_BTN_ADD) {
                            app.popup_manager.close();
                            return NewPopupAction::WorldSelector(WorldSelectorAction::Add);
                        } else if state.is_button_focused(SELECTOR_BTN_CANCEL) {
                            app.popup_manager.close();
                        }
                    } else if state.is_field_selected(SELECTOR_FIELD_FILTER) {
                        // Start editing filter field
                        state.start_edit();
                    } else {
                        // On list - connect to selected world
                        if let Some(name) = get_selected() {
                            app.popup_manager.close();
                            return NewPopupAction::WorldSelector(WorldSelectorAction::Connect(name));
                        }
                    }
                }
                Up => {
                    state.list_select_up();
                }
                Down => {
                    state.list_select_down();
                }
                Tab => {
                    // Cycle between filter field and buttons
                    state.cycle_field_buttons();
                }
                BackTab => {
                    // Cycle backwards
                    state.cycle_field_buttons();
                }
                Char(c) => {
                    // Check if filter field is selected - if so, start editing and add char
                    if state.is_field_selected(SELECTOR_FIELD_FILTER) {
                        state.start_edit();
                        state.insert_char(c);
                        return NewPopupAction::WorldSelectorFilter;
                    }
                    // Otherwise handle as shortcuts
                    match c {
                        'f' | 'F' => {
                            // Select filter field and start editing
                            state.select_field(SELECTOR_FIELD_FILTER);
                            state.start_edit();
                        }
                        'a' | 'A' => {
                            app.popup_manager.close();
                            return NewPopupAction::WorldSelector(WorldSelectorAction::Add);
                        }
                        'e' | 'E' => {
                            if let Some(name) = get_selected() {
                                app.popup_manager.close();
                                return NewPopupAction::WorldSelector(WorldSelectorAction::Edit(name));
                            }
                        }
                        'd' | 'D' => {
                            if let Some(name) = get_selected() {
                                app.popup_manager.close();
                                return NewPopupAction::WorldSelector(WorldSelectorAction::Delete(name));
                            }
                        }
                        'o' | 'O' => {
                            if let Some(name) = get_selected() {
                                app.popup_manager.close();
                                return NewPopupAction::WorldSelector(WorldSelectorAction::Connect(name));
                            }
                        }
                        'c' | 'C' => {
                            app.popup_manager.close();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        // Setup popup handling
        if is_setup {
            // Helper to extract settings before closing
            let extract_settings = || -> SetupSettings {
                SetupSettings {
                    more_mode: state.get_bool(SETUP_FIELD_MORE_MODE).unwrap_or(true),
                    spell_check: state.get_bool(SETUP_FIELD_SPELL_CHECK).unwrap_or(true),
                    temp_convert: state.get_bool(SETUP_FIELD_TEMP_CONVERT).unwrap_or(false),
                    world_switching: state.get_selected(SETUP_FIELD_WORLD_SWITCHING)
                        .unwrap_or("unseen_first").to_string(),
                    debug: state.get_bool(SETUP_FIELD_DEBUG).unwrap_or(false),
                    input_height: state.get_number(SETUP_FIELD_INPUT_HEIGHT).unwrap_or(3),
                    gui_theme: state.get_selected(SETUP_FIELD_GUI_THEME)
                        .unwrap_or("dark").to_string(),
                    tls_proxy: state.get_bool(SETUP_FIELD_TLS_PROXY).unwrap_or(false),
                    dictionary_path: state.get_text(SETUP_FIELD_DICTIONARY)
                        .unwrap_or("").to_string(),
                    editor_side: state.get_selected(SETUP_FIELD_EDITOR_SIDE)
                        .unwrap_or("left").to_string(),
                    mouse_enabled: state.get_bool(SETUP_FIELD_MOUSE).unwrap_or(true),
                    zwj_enabled: state.get_bool(SETUP_FIELD_ZWJ).unwrap_or(false),
                    ansi_music: state.get_bool(SETUP_FIELD_ANSI_MUSIC).unwrap_or(true),
                    new_line_indicator: state.get_bool(SETUP_FIELD_NEW_LINE_INDICATOR).unwrap_or(false),
                    tts_mode: state.get_selected(SETUP_FIELD_TTS).unwrap_or("off").to_string(),
                    tts_speak_mode: state.get_selected(SETUP_FIELD_TTS_SPEAK_MODE).unwrap_or("all").to_string(),
                }
            };

            // Check if current field is a text field
            let is_text_field = state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false);

            match key.code {
                Esc => {
                    if state.editing {
                        state.cancel_edit();
                    } else {
                        app.popup_manager.close();
                    }
                }
                Enter => {
                    if state.editing {
                        state.commit_edit();
                    } else if state.is_on_button() {
                        if state.is_button_focused(SETUP_BTN_SAVE) {
                            let settings = extract_settings();
                            app.popup_manager.close();
                            return NewPopupAction::SetupSaved(settings);
                        } else if state.is_button_focused(SETUP_BTN_CANCEL) {
                            app.popup_manager.close();
                        }
                    } else {
                        // Toggle current field or start editing text field
                        if is_text_field {
                            state.start_edit();
                        } else {
                            state.toggle_current();
                        }
                    }
                }
                Char(' ') => {
                    if state.editing {
                        state.insert_char(' ');
                    } else if state.is_on_button() {
                        if state.is_button_focused(SETUP_BTN_SAVE) {
                            let settings = extract_settings();
                            app.popup_manager.close();
                            return NewPopupAction::SetupSaved(settings);
                        } else if state.is_button_focused(SETUP_BTN_CANCEL) {
                            app.popup_manager.close();
                        }
                    } else {
                        // Toggle current field
                        state.toggle_current();
                    }
                }
                Up => {
                    // Commit any current edit before moving
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        // Go back to last field from button row
                        state.select_last_field();
                    } else {
                        state.prev_field();
                    }
                    // Auto-start editing if new field is text
                    if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                Down => {
                    // Commit any current edit before moving
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        // Go back to last field from button row
                        state.select_last_field();
                    } else {
                        // Move to next field (don't go to buttons)
                        state.next_field();
                    }
                    // Auto-start editing if new field is text
                    if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                Left => {
                    if is_text_field {
                        // Text field: move cursor (start editing if needed)
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_left();
                    } else {
                        // Decrease number or cycle select
                        state.decrease_current();
                    }
                }
                Right => {
                    if is_text_field {
                        // Text field: move cursor (start editing if needed)
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_right();
                    } else {
                        // Increase number or cycle select
                        state.increase_current();
                    }
                }
                Tab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        state.next_button();
                    } else {
                        state.select_first_button();
                    }
                }
                BackTab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        state.prev_button();
                    } else {
                        state.select_last_field();
                        // Auto-start editing if new field is text
                        if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                            state.start_edit();
                        }
                    }
                }
                Backspace => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.backspace();
                    }
                }
                Delete => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.delete_char();
                    }
                }
                Home => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_home();
                    }
                }
                End => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_end();
                    }
                }
                Char('s') | Char('S') if !state.editing && !is_text_field => {
                    let settings = extract_settings();
                    app.popup_manager.close();
                    return NewPopupAction::SetupSaved(settings);
                }
                Char('c') | Char('C') if !state.editing && !is_text_field => {
                    app.popup_manager.close();
                }
                Char(c) => {
                    if state.editing {
                        state.insert_char(c);
                    } else if is_text_field {
                        // Start editing and insert character
                        state.start_edit();
                        state.insert_char(c);
                    }
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        // Web popup handling
        if is_web {
            // Helper to extract settings before closing
            let extract_settings = || -> WebSettings {
                WebSettings {
                    web_secure: state.get_selected(WEB_FIELD_PROTOCOL) == Some("secure"),
                    http_enabled: state.get_bool(WEB_FIELD_HTTP_ENABLED).unwrap_or(false),
                    http_port: state.get_text(WEB_FIELD_HTTP_PORT).unwrap_or("9000").to_string(),
                    ws_password: state.get_text(WEB_FIELD_WS_PASSWORD).unwrap_or("").to_string(),
                    ws_allow_list: state.get_text(WEB_FIELD_WS_ALLOW_LIST).unwrap_or("").to_string(),
                    ws_cert_file: state.get_text(WEB_FIELD_WS_CERT_FILE).unwrap_or("").to_string(),
                    ws_key_file: state.get_text(WEB_FIELD_WS_KEY_FILE).unwrap_or("").to_string(),
                    auth_key: state.get_text(WEB_FIELD_AUTH_KEY).unwrap_or("").to_string(),
                }
            };

            // Check if current field is a text field
            let is_text_field = state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false);

            match key.code {
                Esc => {
                    if state.editing {
                        state.cancel_edit();
                    } else {
                        app.popup_manager.close();
                    }
                }
                Enter => {
                    if state.editing {
                        state.commit_edit();
                    } else if state.is_on_button() {
                        if state.is_button_focused(WEB_BTN_SAVE) {
                            let settings = extract_settings();
                            app.popup_manager.close();
                            return NewPopupAction::WebSaved(settings);
                        } else if state.is_button_focused(WEB_BTN_REGEN_KEY) {
                            // Generate a new auth key and update the field
                            let new_key = App::generate_auth_key();
                            if let Some(field) = state.definition.fields.iter_mut()
                                .find(|f| f.id == WEB_FIELD_AUTH_KEY)
                            {
                                if let popup::FieldKind::Text { ref mut value, .. } = field.kind {
                                    *value = new_key;
                                }
                            }
                        } else if state.is_button_focused(WEB_BTN_COPY_KEY) {
                            // Copy auth key to clipboard via OSC 52
                            if let Some(key_text) = state.get_text(WEB_FIELD_AUTH_KEY) {
                                if !key_text.is_empty() {
                                    use std::io::Write;
                                    let encoded = base64::Engine::encode(
                                        &base64::engine::general_purpose::STANDARD,
                                        key_text.as_bytes(),
                                    );
                                    let osc52 = format!("\x1b]52;c;{}\x07", encoded);
                                    let _ = std::io::stdout().write_all(osc52.as_bytes());
                                    let _ = std::io::stdout().flush();
                                    // Show feedback (auto-clears after 10s)
                                    state.error = Some("Auth key copied to clipboard".to_string());
                                    state.error_at = Some(std::time::Instant::now());
                                }
                            }
                        } else if state.is_button_focused(WEB_BTN_CANCEL) {
                            app.popup_manager.close();
                        }
                    } else if is_text_field {
                        state.start_edit();
                    } else {
                        // Toggle current field
                        state.toggle_current();
                        // Update TLS visibility when protocol changes
                        if let ElementSelection::Field(id) = &state.selected {
                            if *id == WEB_FIELD_PROTOCOL {
                                update_tls_visibility(state);
                            }
                        }
                    }
                }
                Char(' ') => {
                    if !state.editing {
                        // Toggle for non-text fields
                        state.toggle_current();
                        // Update TLS visibility when protocol changes
                        if let ElementSelection::Field(id) = &state.selected {
                            if *id == WEB_FIELD_PROTOCOL {
                                update_tls_visibility(state);
                            }
                        }
                    } else {
                        state.insert_char(' ');
                    }
                }
                Up => {
                    // Commit any current edit before moving
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        // Go back to last field from button row
                        state.select_last_field();
                    } else {
                        state.prev_field();
                    }
                    // Auto-start editing if new field is text
                    if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                Down => {
                    // Commit any current edit before moving
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        // Go back to last field from button row
                        state.select_last_field();
                    } else {
                        // Move to next field (don't go to buttons)
                        state.next_field();
                    }
                    // Auto-start editing if new field is text
                    if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                Left => {
                    if is_text_field {
                        // Text field: move cursor (start editing if needed)
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_left();
                    } else {
                        state.decrease_current();
                        // Update TLS visibility when protocol changes
                        if let ElementSelection::Field(id) = &state.selected {
                            if *id == WEB_FIELD_PROTOCOL {
                                update_tls_visibility(state);
                            }
                        }
                    }
                }
                Right => {
                    if is_text_field {
                        // Text field: move cursor (start editing if needed)
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_right();
                    } else {
                        state.increase_current();
                        // Update TLS visibility when protocol changes
                        if let ElementSelection::Field(id) = &state.selected {
                            if *id == WEB_FIELD_PROTOCOL {
                                update_tls_visibility(state);
                            }
                        }
                    }
                }
                Tab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        state.next_button();
                    } else {
                        state.select_first_button();
                    }
                }
                BackTab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        state.prev_button();
                    } else {
                        state.select_last_field();
                        // Auto-start editing if new field is text
                        if state.selected_field().map(|f| f.kind.is_text()).unwrap_or(false) {
                            state.start_edit();
                        }
                    }
                }
                Backspace => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.backspace();
                    }
                }
                Delete => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.delete_char();
                    }
                }
                Home => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_home();
                    }
                }
                End => {
                    if is_text_field {
                        if !state.editing {
                            state.start_edit();
                        }
                        state.cursor_end();
                    }
                }
                Char('s') | Char('S') if !state.editing && !is_text_field => {
                    let settings = extract_settings();
                    app.popup_manager.close();
                    return NewPopupAction::WebSaved(settings);
                }
                Char('c') | Char('C') if !state.editing && !is_text_field => {
                    app.popup_manager.close();
                }
                Char(c) => {
                    if state.editing {
                        state.insert_char(c);
                    } else if is_text_field {
                        // Start editing and insert character
                        state.start_edit();
                        state.insert_char(c);
                    }
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        // Notes list popup handling
        if is_notes_list {
            use popup::definitions::notes_list::{NOTES_BTN_CANCEL};

            let get_selected_name = || state.get_selected_list_item().map(|item| item.id.clone());

            match key.code {
                Esc => {
                    app.popup_manager.close();
                    return NewPopupAction::None;
                }
                Enter => {
                    // If on a button, check which one
                    if state.is_button_focused(NOTES_BTN_CANCEL) {
                        app.popup_manager.close();
                        return NewPopupAction::None;
                    }
                    // Open button or list item - open notes
                    if let Some(name) = get_selected_name() {
                        app.popup_manager.close();
                        return NewPopupAction::NotesList(NotesListAction::Open(name));
                    }
                }
                Up => {
                    state.list_select_up();
                }
                Down => {
                    state.list_select_down();
                }
                Tab | BackTab => {
                    state.cycle_field_buttons();
                }
                Char('o') | Char('O') => {
                    if let Some(name) = get_selected_name() {
                        app.popup_manager.close();
                        return NewPopupAction::NotesList(NotesListAction::Open(name));
                    }
                }
                Char('c') | Char('C') => {
                    app.popup_manager.close();
                    return NewPopupAction::None;
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        // Connections popup handling (simple - just close on any key)
        if is_connections {
            match key.code {
                Esc | Enter | Char(' ') => {
                    app.popup_manager.close();
                    return NewPopupAction::ConnectionsClose;
                }
                Char('o') | Char('O') => {
                    // OK shortcut
                    app.popup_manager.close();
                    return NewPopupAction::ConnectionsClose;
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        // Actions list popup handling
        if is_actions_list {
            // Get selected action index before any state mutations
            let get_selected_index = || {
                if let Some(field) = state.field(ACTIONS_FIELD_LIST) {
                    if let popup::FieldKind::List { items, selected_index, .. } = &field.kind {
                        if !items.is_empty() && *selected_index < items.len() {
                            // Parse the action index from the item id (which is just the index as string)
                            return items.get(*selected_index).and_then(|item| item.id.parse::<usize>().ok());
                        }
                    }
                }
                None
            };

            // Check if we're editing the filter field
            let is_editing_filter = state.editing && state.is_field_selected(ACTIONS_FIELD_FILTER);

            // When editing filter, handle text input
            if is_editing_filter {
                match key.code {
                    Esc => {
                        state.commit_edit();
                        return NewPopupAction::ActionsListFilter;
                    }
                    Tab => {
                        state.commit_edit();
                        state.cycle_field_buttons();
                        return NewPopupAction::ActionsListFilter;
                    }
                    Enter => {
                        state.commit_edit();
                        return NewPopupAction::ActionsListFilter;
                    }
                    Backspace => {
                        state.backspace();
                        return NewPopupAction::ActionsListFilter;
                    }
                    Delete => {
                        state.delete_char();
                        return NewPopupAction::ActionsListFilter;
                    }
                    Left => {
                        state.cursor_left();
                    }
                    Right => {
                        state.cursor_right();
                    }
                    Home => {
                        state.cursor_home();
                    }
                    End => {
                        state.cursor_end();
                    }
                    Char(c) => {
                        state.insert_char(c);
                        return NewPopupAction::ActionsListFilter;
                    }
                    _ => {}
                }
                return NewPopupAction::None;
            }

            // Not editing - handle normal navigation and shortcuts
            match key.code {
                Esc => {
                    app.popup_manager.close();
                }
                Enter => {
                    if state.is_on_button() {
                        if state.is_button_focused(ACTIONS_BTN_ADD) {
                            app.popup_manager.close();
                            return NewPopupAction::ActionsList(ActionsListAction::Add);
                        } else if state.is_button_focused(ACTIONS_BTN_EDIT) {
                            if let Some(idx) = get_selected_index() {
                                app.popup_manager.close();
                                return NewPopupAction::ActionsList(ActionsListAction::Edit(idx));
                            }
                        } else if state.is_button_focused(ACTIONS_BTN_DELETE) {
                            if let Some(idx) = get_selected_index() {
                                app.popup_manager.close();
                                return NewPopupAction::ActionsList(ActionsListAction::Delete(idx));
                            }
                        } else if state.is_button_focused(ACTIONS_BTN_CANCEL) {
                            app.popup_manager.close();
                        }
                    } else if state.is_field_selected(ACTIONS_FIELD_FILTER) {
                        // Start editing filter field
                        state.start_edit();
                    } else {
                        // On list - edit selected action
                        if let Some(idx) = get_selected_index() {
                            app.popup_manager.close();
                            return NewPopupAction::ActionsList(ActionsListAction::Edit(idx));
                        }
                    }
                }
                Up => {
                    state.list_select_up();
                }
                Down => {
                    state.list_select_down();
                }
                Tab => {
                    state.cycle_field_buttons();
                }
                BackTab => {
                    state.cycle_field_buttons();
                }
                Char(c) => {
                    // Check if filter field is selected - start editing
                    if state.is_field_selected(ACTIONS_FIELD_FILTER) {
                        state.start_edit();
                        state.insert_char(c);
                        return NewPopupAction::ActionsListFilter;
                    }
                    // Otherwise handle shortcuts
                    match c {
                        ' ' => {
                            // Space toggles enable/disable for selected action
                            if let Some(idx) = get_selected_index() {
                                // Don't close popup - stay on list for more toggles
                                return NewPopupAction::ActionsList(ActionsListAction::Toggle(idx));
                            }
                        }
                        'f' | 'F' | '/' => {
                            state.select_field(ACTIONS_FIELD_FILTER);
                            state.start_edit();
                        }
                        'a' | 'A' => {
                            app.popup_manager.close();
                            return NewPopupAction::ActionsList(ActionsListAction::Add);
                        }
                        'e' | 'E' => {
                            if let Some(idx) = get_selected_index() {
                                app.popup_manager.close();
                                return NewPopupAction::ActionsList(ActionsListAction::Edit(idx));
                            }
                        }
                        'd' | 'D' => {
                            if let Some(idx) = get_selected_index() {
                                app.popup_manager.close();
                                return NewPopupAction::ActionsList(ActionsListAction::Delete(idx));
                            }
                        }
                        'c' | 'C' | 'o' | 'O' => {
                            app.popup_manager.close();
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        // Action editor popup handling
        if is_action_editor {
            let is_text_field = state.selected_field()
                .map(|f| f.kind.is_text_editable())
                .unwrap_or(false);
            let is_multiline = state.selected_field()
                .map(|f| matches!(&f.kind, popup::FieldKind::MultilineText { .. }))
                .unwrap_or(false);
            let is_toggle = state.selected_field()
                .map(|f| matches!(&f.kind, popup::FieldKind::Toggle { .. }))
                .unwrap_or(false);
            let is_select = state.selected_field()
                .map(|f| matches!(&f.kind, popup::FieldKind::Select { .. }))
                .unwrap_or(false);

            match key.code {
                Esc => {
                    if state.editing {
                        state.cancel_edit();
                    } else {
                        app.popup_manager.close();
                    }
                }
                Enter => {
                    if state.editing && is_multiline {
                        // In multiline field, Enter inserts a newline
                        state.insert_newline();
                        state.ensure_multiline_cursor_visible();
                    } else if state.editing {
                        state.commit_edit();
                        state.next_field();
                    } else if state.is_on_button() {
                        if state.is_button_focused(EDITOR_BTN_SAVE) {
                            // Extract action data and save
                            let name = state.get_text(EDITOR_FIELD_NAME).unwrap_or("").to_string();
                            let world = state.get_text(EDITOR_FIELD_WORLD).unwrap_or("").to_string();
                            let match_type_str = state.get_selected(EDITOR_FIELD_MATCH_TYPE).unwrap_or("regexp");
                            let pattern = state.get_text(EDITOR_FIELD_PATTERN).unwrap_or("").to_string();
                            let command = state.get_text(EDITOR_FIELD_COMMAND).unwrap_or("").to_string();
                            let enabled = state.get_bool(EDITOR_FIELD_ENABLED).unwrap_or(true);
                            let startup = state.get_bool(EDITOR_FIELD_STARTUP).unwrap_or(false);
                            let editing_index = state.get_custom("editing_index").and_then(|s| s.parse::<usize>().ok());

                            let match_type = if match_type_str == "wildcard" {
                                MatchType::Wildcard
                            } else {
                                MatchType::Regexp
                            };

                            let action = Action {
                                name,
                                world,
                                match_type,
                                pattern,
                                command,
                                owner: None,
                                enabled,
                                startup,
                                compiled_regex: None,
                            };

                            app.popup_manager.close();
                            return NewPopupAction::ActionEditorSave { action, editing_index };
                        } else if state.is_button_focused(EDITOR_BTN_CANCEL) {
                            app.popup_manager.close();
                        } else if state.is_button_focused(EDITOR_BTN_DELETE) {
                            let editing_index = state.get_custom("editing_index").and_then(|s| s.parse::<usize>().ok());
                            if let Some(idx) = editing_index {
                                app.popup_manager.close();
                                return NewPopupAction::ActionEditorDelete { editing_index: idx };
                            }
                        }
                    } else if is_toggle || is_select {
                        state.toggle_current();
                    } else if is_text_field {
                        state.start_edit();
                    }
                }
                Tab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    state.cycle_field_buttons();
                }
                BackTab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    // Go to previous element
                    if !state.prev_field() {
                        state.select_last_field();
                    }
                }
                Up => {
                    if state.editing && is_multiline {
                        // In multiline field, Up moves cursor up
                        state.cursor_up();
                        state.ensure_multiline_cursor_visible();
                    } else {
                        if state.editing {
                            state.commit_edit();
                        }
                        state.prev_field();
                    }
                }
                Down => {
                    if state.editing && is_multiline {
                        // In multiline field, Down moves cursor down
                        state.cursor_down();
                        state.ensure_multiline_cursor_visible();
                    } else {
                        if state.editing {
                            state.commit_edit();
                        }
                        if !state.next_field() {
                            state.select_first_button();
                        }
                    }
                }
                Left => {
                    if state.editing {
                        state.cursor_left();
                        if is_multiline {
                            state.ensure_multiline_cursor_visible();
                        }
                    } else if is_select || is_toggle {
                        state.decrease_current();
                    } else if state.is_on_button() {
                        state.prev_button();
                    }
                }
                Right => {
                    if state.editing {
                        state.cursor_right();
                        if is_multiline {
                            state.ensure_multiline_cursor_visible();
                        }
                    } else if is_select || is_toggle {
                        state.increase_current();
                    } else if state.is_on_button() {
                        state.next_button();
                    }
                }
                Backspace => {
                    if state.editing {
                        state.backspace();
                        if is_multiline {
                            state.ensure_multiline_cursor_visible();
                        }
                    }
                }
                Delete => {
                    if state.editing {
                        state.delete_char();
                    }
                }
                Home => {
                    if state.editing {
                        state.cursor_home();
                        if is_multiline {
                            state.ensure_multiline_cursor_visible();
                        }
                    }
                }
                End => {
                    if state.editing {
                        state.cursor_end();
                        if is_multiline {
                            state.ensure_multiline_cursor_visible();
                        }
                    }
                }
                Char('s') | Char('S') if !state.editing && !is_text_field => {
                    // Save shortcut (only when not on a text field)
                    state.select_button(EDITOR_BTN_SAVE);
                }
                Char('c') | Char('C') if !state.editing && !is_text_field => {
                    app.popup_manager.close();
                }
                Char('d') | Char('D') if !state.editing && !is_text_field => {
                    // Delete shortcut
                    state.select_button(EDITOR_BTN_DELETE);
                }
                Char(' ') if !state.editing && (is_toggle || is_select) => {
                    // Space toggles current toggle/select field
                    state.toggle_current();
                }
                Char(c) => {
                    if state.editing {
                        state.insert_char(c);
                        if is_multiline {
                            state.ensure_multiline_cursor_visible();
                        }
                    } else if is_text_field {
                        state.start_edit();
                        state.insert_char(c);
                        if is_multiline {
                            state.ensure_multiline_cursor_visible();
                        }
                    }
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        // World editor popup handling
        if is_world_editor {
            // Get world index from custom state
            let world_index: usize = state.get_custom("world_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            // Helper to extract settings before closing
            let extract_settings = || -> WorldEditorSettings {
                WorldEditorSettings {
                    world_index,
                    name: state.get_text(WORLD_FIELD_NAME).unwrap_or("").to_string(),
                    world_type: state.get_selected(WORLD_FIELD_TYPE).unwrap_or("mud").to_string(),
                    hostname: state.get_text(WORLD_FIELD_HOSTNAME).unwrap_or("").to_string(),
                    port: state.get_text(WORLD_FIELD_PORT).unwrap_or("").to_string(),
                    user: state.get_text(WORLD_FIELD_USER).unwrap_or("").to_string(),
                    password: state.get_text(WORLD_FIELD_PASSWORD).unwrap_or("").to_string(),
                    use_ssl: state.get_bool(WORLD_FIELD_USE_SSL).unwrap_or(false),
                    log_enabled: state.get_bool(WORLD_FIELD_LOG_ENABLED).unwrap_or(false),
                    encoding: state.get_selected(WORLD_FIELD_ENCODING).unwrap_or("utf8").to_string(),
                    auto_connect: state.get_selected(WORLD_FIELD_AUTO_CONNECT).unwrap_or("connect").to_string(),
                    keep_alive: state.get_selected(WORLD_FIELD_KEEP_ALIVE).unwrap_or("nop").to_string(),
                    keep_alive_cmd: state.get_text(WORLD_FIELD_KEEP_ALIVE_CMD).unwrap_or("").to_string(),
                    gmcp_packages: state.get_text(WORLD_FIELD_GMCP_PACKAGES).unwrap_or("Client.Media 1").to_string(),
                    auto_reconnect_secs: state.get_text(WORLD_FIELD_AUTO_RECONNECT).unwrap_or("0").to_string(),
                    slack_token: state.get_text(WORLD_FIELD_SLACK_TOKEN).unwrap_or("").to_string(),
                    slack_channel: state.get_text(WORLD_FIELD_SLACK_CHANNEL).unwrap_or("").to_string(),
                    slack_workspace: state.get_text(WORLD_FIELD_SLACK_WORKSPACE).unwrap_or("").to_string(),
                    discord_token: state.get_text(WORLD_FIELD_DISCORD_TOKEN).unwrap_or("").to_string(),
                    discord_guild: state.get_text(WORLD_FIELD_DISCORD_GUILD).unwrap_or("").to_string(),
                    discord_channel: state.get_text(WORLD_FIELD_DISCORD_CHANNEL).unwrap_or("").to_string(),
                    discord_dm_user: state.get_text(WORLD_FIELD_DISCORD_DM_USER).unwrap_or("").to_string(),
                }
            };

            // Check field type
            let is_text_field = state.selected_field()
                .map(|f| f.kind.is_text_editable())
                .unwrap_or(false);
            let is_toggle = state.selected_field()
                .map(|f| matches!(&f.kind, popup::FieldKind::Toggle { .. }))
                .unwrap_or(false);
            let is_select = state.selected_field()
                .map(|f| matches!(&f.kind, popup::FieldKind::Select { .. }))
                .unwrap_or(false);

            // Macro to update visibility when type or keep_alive changes
            // (We can't use a closure because of borrow conflicts)
            macro_rules! update_visibility {
                ($state:expr) => {{
                    // Extract values first to avoid borrow conflicts
                    let world_type_str = $state.get_selected(WORLD_FIELD_TYPE).unwrap_or("mud").to_string();
                    let keep_alive_str = $state.get_selected(WORLD_FIELD_KEEP_ALIVE).unwrap_or("nop").to_string();
                    update_field_visibility(
                        &mut $state.definition,
                        PopupWorldType::parse(&world_type_str),
                        keep_alive_str == "custom",
                    );
                }};
            }

            match key.code {
                Esc => {
                    if state.editing {
                        state.cancel_edit();
                    } else {
                        app.popup_manager.close();
                    }
                }
                Enter => {
                    if state.editing {
                        state.commit_edit();
                        state.next_field();
                        // Auto-start editing on text/password fields
                        if state.selected_field().map(|f| f.kind.is_text_editable()).unwrap_or(false) {
                            state.start_edit();
                        }
                    } else if state.is_on_button() {
                        if state.is_button_focused(WORLD_BTN_SAVE) {
                            let settings = extract_settings();
                            app.popup_manager.close();
                            return NewPopupAction::WorldEditorSaved(Box::new(settings));
                        } else if state.is_button_focused(WORLD_BTN_CANCEL) {
                            app.popup_manager.close();
                        } else if state.is_button_focused(WORLD_BTN_DELETE) {
                            app.popup_manager.close();
                            return NewPopupAction::WorldEditorDelete(world_index);
                        } else if state.is_button_focused(WORLD_BTN_CONNECT) {
                            let _settings = extract_settings();
                            app.popup_manager.close();
                            // First save, then connect
                            return NewPopupAction::WorldEditorConnect(world_index);
                        }
                    } else if is_toggle {
                        state.toggle_current();
                    } else if is_select {
                        state.toggle_current();
                        update_visibility!(state);
                    } else if is_text_field {
                        state.start_edit();
                    }
                }
                Char(' ') => {
                    if state.editing {
                        state.insert_char(' ');
                    } else if is_toggle {
                        state.toggle_current();
                    } else if is_select {
                        state.toggle_current();
                        update_visibility!(state);
                    }
                }
                Tab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    state.cycle_field_buttons();
                    // Auto-start editing on text/password fields
                    if state.selected_field().map(|f| f.kind.is_text_editable()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                BackTab => {
                    if state.editing {
                        state.commit_edit();
                    }
                    // Go to previous element
                    if !state.prev_field() {
                        state.select_last_field();
                    }
                    // Auto-start editing on text/password fields
                    if state.selected_field().map(|f| f.kind.is_text_editable()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                Up => {
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        state.select_last_field();
                    } else {
                        state.prev_field();
                    }
                    // Auto-start editing on text/password fields
                    if state.selected_field().map(|f| f.kind.is_text_editable()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                Down => {
                    if state.editing {
                        state.commit_edit();
                    }
                    if state.is_on_button() {
                        state.select_last_field();
                    } else {
                        // Only move between fields, never to buttons
                        state.next_field();
                    }
                    // Auto-start editing on text/password fields
                    if state.selected_field().map(|f| f.kind.is_text_editable()).unwrap_or(false) {
                        state.start_edit();
                    }
                }
                Left => {
                    if state.editing {
                        state.cursor_left();
                    } else if is_select || is_toggle {
                        state.decrease_current();
                        update_visibility!(state);
                    } else if state.is_on_button() {
                        state.prev_button();
                    }
                }
                Right => {
                    if state.editing {
                        state.cursor_right();
                    } else if is_select || is_toggle {
                        state.increase_current();
                        update_visibility!(state);
                    } else if state.is_on_button() {
                        state.next_button();
                    }
                }
                Backspace => {
                    if state.editing {
                        state.backspace();
                    }
                }
                Delete => {
                    if state.editing {
                        state.delete_char();
                    }
                }
                Home => {
                    if state.editing {
                        state.cursor_home();
                    }
                }
                End => {
                    if state.editing {
                        state.cursor_end();
                    }
                }
                Char('s') | Char('S') if !state.editing && !is_text_field => {
                    let settings = extract_settings();
                    app.popup_manager.close();
                    return NewPopupAction::WorldEditorSaved(Box::new(settings));
                }
                Char('c') | Char('C') if !state.editing && !is_text_field => {
                    app.popup_manager.close();
                }
                Char('d') | Char('D') if !state.editing && !is_text_field => {
                    app.popup_manager.close();
                    return NewPopupAction::WorldEditorDelete(world_index);
                }
                Char('o') | Char('O') if !state.editing && !is_text_field => {
                    let _settings = extract_settings();
                    app.popup_manager.close();
                    return NewPopupAction::WorldEditorConnect(world_index);
                }
                Char(c) => {
                    if state.editing {
                        state.insert_char(c);
                    } else if is_text_field {
                        state.start_edit();
                        state.insert_char(c);
                    }
                }
                _ => {}
            }
            return NewPopupAction::None;
        }

        match key.code {
            Esc => {
                if is_confirm {
                    let data = state.definition.custom_data.clone();
                    app.popup_manager.close();
                    return NewPopupAction::ConfirmCancelled(data);
                }
                app.popup_manager.close();
            }
            Enter => {
                // Check popup type
                if is_menu {
                    // For menu popup, get selected command and close
                    if let Some(item) = state.get_selected_list_item() {
                        let cmd = item.id.clone();
                        app.popup_manager.close();
                        return NewPopupAction::Command(cmd);
                    }
                } else if is_confirm {
                    // For confirm dialog, check which button is selected
                    if state.is_button_focused(CONFIRM_BTN_YES) {
                        let data = state.definition.custom_data.clone();
                        app.popup_manager.close();
                        return NewPopupAction::Confirm(data);
                    }
                    // No button - cancel
                    let data = state.definition.custom_data.clone();
                    app.popup_manager.close();
                    return NewPopupAction::ConfirmCancelled(data);
                }
                // For other popups, Enter closes
                app.popup_manager.close();
            }
            Up | Down => {
                if is_menu {
                    if matches!(key.code, Up) {
                        state.list_select_up();
                    } else {
                        state.list_select_down();
                    }
                } else if is_confirm {
                    // Toggle between Yes and No
                    if state.is_button_focused(CONFIRM_BTN_YES) {
                        state.select_button(CONFIRM_BTN_NO);
                    } else {
                        state.select_button(CONFIRM_BTN_YES);
                    }
                } else {
                    // Scrollable content (help popup) - scroll regardless of button focus
                    if matches!(key.code, Up) {
                        state.scroll_content_up(1);
                    } else {
                        state.scroll_content_down(1);
                    }
                }
            }
            Left | Right | Tab => {
                if is_confirm {
                    // Toggle between Yes and No
                    if state.is_button_focused(CONFIRM_BTN_YES) {
                        state.select_button(CONFIRM_BTN_NO);
                    } else {
                        state.select_button(CONFIRM_BTN_YES);
                    }
                } else if matches!(key.code, Tab) {
                    // Tab selects the first button (e.g., Ok in help popup)
                    state.select_first_button();
                }
            }
            PageUp => {
                state.scroll_content_up(5);
            }
            PageDown => {
                state.scroll_content_down(5);
            }
            Char('y') | Char('Y') => {
                if is_confirm {
                    let data = state.definition.custom_data.clone();
                    app.popup_manager.close();
                    return NewPopupAction::Confirm(data);
                }
            }
            Char('n') | Char('N') => {
                if is_confirm {
                    let data = state.definition.custom_data.clone();
                    app.popup_manager.close();
                    return NewPopupAction::ConfirmCancelled(data);
                }
            }
            Char('o') | Char('O') => {
                // Shortcut for OK button (help popup)
                state.select_button(HELP_BTN_OK);
            }
            Char('c') | Char('C') => {
                // Cancel shortcut
                if is_confirm {
                    let data = state.definition.custom_data.clone();
                    app.popup_manager.close();
                    return NewPopupAction::ConfirmCancelled(data);
                }
                app.popup_manager.close();
            }
            _ => {}
        }
    }
    NewPopupAction::None
}


#[tokio::main]
async fn main() -> io::Result<()> {

    // On Windows, ensure Winsock is initialized before any socket operations.
    // This is needed because inherited socket handles from a parent process
    // (during hot reload) require WSAStartup in the new process.
    #[cfg(windows)]
    {
        extern "system" {
            fn WSAStartup(wVersionRequired: u16, lpWSAData: *mut [u8; 408]) -> i32;
        }
        let mut wsa_data = [0u8; 408];
        unsafe { WSAStartup(0x0202, &mut wsa_data); }
    }

    // =========================================================================
    // CLI argument parsing - single structured pass with validation
    // =========================================================================
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut show_version = false;
    let mut show_help = false;
    let mut conf_path: Option<String> = None;
    let mut daemon_mode = false;
    let mut multiuser_mode = false;
    let mut is_reload_arg = false;
    let mut is_crash_arg = false;
    #[allow(unused_variables)]
    let mut tls_proxy_config: Option<String> = None;
    let mut console_arg: Option<Option<String>> = None;  // None=not set, Some(None)=bare, Some(Some(addr))=with addr
    let mut gui_arg: Option<Option<String>> = None;
    let mut grep_arg: Option<String> = None;
    let mut grep_extra_args: Vec<String> = Vec::new();

    #[allow(unused_assignments)]
    {
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if let Some(stripped) = arg.strip_prefix("--grep=") {
                let mut addr = stripped.to_string();
                // Default to port 9000 if no port specified
                if !addr.contains(':') {
                    addr.push_str(":9000");
                }
                grep_arg = Some(addr);
                // Collect all remaining args for grep
                grep_extra_args = args[i + 1..].to_vec();
                break;
            }
            match arg.as_str() {
                "-v" | "--version" => show_version = true,
                "-h" | "--help" => show_help = true,
                "-D" => daemon_mode = true,
                "--multiuser" => multiuser_mode = true,
                "--reload" => is_reload_arg = true,
                "--crash" => is_crash_arg = true,
                "--console" => console_arg = Some(None),
                "--gui" => gui_arg = Some(None),
                _ if arg.starts_with("--conf=") => conf_path = Some(arg[7..].to_string()),
                _ if arg.starts_with("--console=") => console_arg = Some(Some(arg[10..].to_string())),
                _ if arg.starts_with("--gui=") => gui_arg = Some(Some(arg[6..].to_string())),
                _ if arg.starts_with("--tls-proxy=") => tls_proxy_config = Some(arg[12..].to_string()),
                _ => {
                    eprintln!("Error: Unknown option '{}'. Use -h for help.", arg);
                    std::process::exit(1);
                }
            }
            i += 1;
        }
    }

    // Handle -h / --help
    if show_help {
        println!("Clay MUD Client v{}", VERSION);
        println!();
        println!("Usage: clay [OPTIONS]");
        println!();
        println!("Options:");
        println!("    --console            Run in console (TUI) mode");
        println!("    --console=host[:port] Connect to a Clay server via console (default port: 9000)");
        println!("    --gui                Run in GUI (webview) mode");
        println!("    --gui=host[:port]    Connect to a Clay server via GUI (default port: 9000)");
        println!("    -D                   Run as headless daemon server");
        println!("    --multiuser          Run as multiuser server");
        println!("    --conf=<path>        Use custom config file (default: ~/.clay.dat)");
        println!("    --grep=host[:port] <pattern>  Search world output (default port: 9000)");
        println!("      -w <world>              Limit to specific world");
        println!("      --regexp                Use regex (default: glob with * and ? wildcards)");
        println!("      --noesc                 Strip ANSI color codes from output");
        println!("      -f                      Follow mode (match new output, runs until Ctrl+C)");
        println!("      Password via CLAY_PASSWORD environment variable");
        println!("    -v, --version        Show version and build information");
        println!("    -h, --help           Show this help message");
        println!();
        println!("Defaults:");
        println!("    Windows/macOS: --gui");
        println!("    Linux/Termux:  --console");
        return Ok(());
    }

    // Handle -v / --version with feature info
    if show_version {
        let mut features = Vec::new();
        if cfg!(feature = "rustls-backend") { features.push("rustls"); }
        if cfg!(feature = "native-tls-backend") { features.push("native-tls"); }
        if cfg!(feature = "webview-gui") { features.push("webview-gui"); }
        let features_str = if features.is_empty() {
            "none".to_string()
        } else {
            features.join(", ")
        };
        println!("Clay v{} (build {}-{})", VERSION, BUILD_DATE, BUILD_HASH);
        println!("Features: {}", features_str);
        return Ok(());
    }

    // Set custom config path if specified
    if let Some(ref path) = conf_path {
        set_custom_config_path(PathBuf::from(path));
    }

    // Record startup time and reset debug log header flags for this session
    STARTUP_TIME.store(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        Ordering::Relaxed,
    );
    DEBUG_LOG_HEADER_WRITTEN.store(false, Ordering::Relaxed);
    OUTPUT_DEBUG_HEADER_WRITTEN.store(false, Ordering::Relaxed);

    // Always log startup (not gated by debug flag) for reload/crash diagnostics
    debug_log(true, &format!("STARTUP: {} (reload={}, crash={}, gui={:?})", get_version_string(), is_reload_arg, is_crash_arg, gui_arg));

    // Parse --grep extra args
    let mut grep_world_filter: Option<String> = None;
    let mut grep_use_regex = false;
    let mut grep_strip_ansi = false;
    let mut grep_follow_mode = false;
    let mut grep_pattern: Option<String> = None;
    if grep_arg.is_some() {
        let mut gi = 0;
        while gi < grep_extra_args.len() {
            match grep_extra_args[gi].as_str() {
                "-w" => {
                    gi += 1;
                    if gi >= grep_extra_args.len() {
                        eprintln!("Error: -w requires a world name argument.");
                        std::process::exit(1);
                    }
                    grep_world_filter = Some(grep_extra_args[gi].clone());
                }
                "--regexp" => grep_use_regex = true,
                "--noesc" => grep_strip_ansi = true,
                "-f" => grep_follow_mode = true,
                other if other.starts_with('-') => {
                    eprintln!("Error: Unknown grep option '{}'. Use -h for help.", other);
                    std::process::exit(1);
                }
                _ => {
                    if grep_pattern.is_none() {
                        grep_pattern = Some(grep_extra_args[gi].clone());
                    } else {
                        eprintln!("Error: Unexpected argument '{}'. Pattern already set to '{}'.", grep_extra_args[gi], grep_pattern.as_deref().unwrap_or(""));
                        std::process::exit(1);
                    }
                }
            }
            gi += 1;
        }
        if grep_pattern.is_none() {
            eprintln!("Error: Missing search pattern. Usage: clay --grep=host:port [OPTIONS] <pattern>");
            std::process::exit(1);
        }
    }

    // Validate mutual exclusivity
    if console_arg.is_some() && gui_arg.is_some() {
        eprintln!("Error: --console and --gui are mutually exclusive.");
        std::process::exit(1);
    }
    if (daemon_mode || multiuser_mode) && (console_arg.is_some() || gui_arg.is_some()) {
        eprintln!("Error: -D/--multiuser cannot be combined with --console/--gui.");
        std::process::exit(1);
    }
    if grep_arg.is_some() && (console_arg.is_some() || gui_arg.is_some() || daemon_mode || multiuser_mode) {
        eprintln!("Error: --grep cannot be combined with --console/--gui/-D/--multiuser.");
        std::process::exit(1);
    }

    // Handle --tls-proxy (internal, used when spawning TLS proxy processes)
    #[cfg(all(unix, not(target_os = "android")))]
    if let Some(ref config_path) = tls_proxy_config {
        if let Ok(contents) = std::fs::read_to_string(config_path) {
            let lines: Vec<&str> = contents.lines().collect();
            if lines.len() >= 2 {
                let host_port: Vec<&str> = lines[0].splitn(2, ':').collect();
                if host_port.len() == 2 {
                    let host = host_port[0];
                    let port = host_port[1];
                    let socket_path = PathBuf::from(lines[1]);
                    let _ = std::fs::remove_file(config_path);
                    run_tls_proxy_async(host, port, &socket_path).await;
                }
            }
        }
        return Ok(());
    }

    // Handle --grep mode (before multiuser/daemon since it's a client mode)
    if let Some(ref addr) = grep_arg {
        return remote_client::run_grep_client(
            addr,
            grep_pattern.as_ref().unwrap(),
            grep_world_filter.as_deref(),
            grep_use_regex,
            grep_strip_ansi,
            grep_follow_mode,
        ).await;
    }

    // Handle --multiuser mode
    if multiuser_mode {
        return run_multiuser_server().await;
    }

    // Handle -D (daemon mode)
    if daemon_mode {
        return run_daemon_server().await;
    }

    // Determine interface mode: GUI vs Console
    let has_gui_flag = gui_arg.is_some();
    let (use_gui, remote_addr) = if let Some(addr_opt) = gui_arg {
        (true, addr_opt)
    } else if let Some(addr_opt) = console_arg {
        (false, addr_opt)
    } else {
        // Default: Mac/Windows -> GUI (if feature available), others -> Console
        let default_gui = (cfg!(target_os = "macos") || cfg!(windows))
            && cfg!(feature = "webview-gui");
        (default_gui, None)
    };

    // Fall back to console if webview-gui feature not available
    let use_gui = use_gui && cfg!(feature = "webview-gui");
    if has_gui_flag && !use_gui {
        #[cfg(not(feature = "webview-gui"))]
        {
            eprintln!("Warning: --gui requires the 'webview-gui' feature. Falling back to console.");
            eprintln!("Rebuild with: cargo build --features webview-gui");
        }
    }

    // On Windows, detach from the console window when running in GUI mode
    #[cfg(windows)]
    if use_gui {
        extern "system" {
            fn FreeConsole() -> i32;
        }
        unsafe { FreeConsole(); }
    }

    // Dispatch based on (interface, connection_mode)
    match (use_gui, remote_addr) {
        // Remote GUI: connect to a running Clay instance via WebSocket
        (true, Some(ref addr)) => {
            #[cfg(feature = "webview-gui")]
            {
                return webview_gui::run_remote_webgui(addr);
            }
            #[cfg(not(feature = "webview-gui"))]
            {
                let _ = addr;
                unreachable!("use_gui should be false when webview-gui feature is not available");
            }
        }
        // Master GUI: run App in-process with webview GUI
        (true, None) => {
            #[cfg(feature = "webview-gui")]
            {
                return webview_gui::run_master_webgui();
            }
            #[cfg(not(feature = "webview-gui"))]
            {
                unreachable!("use_gui should be false when webview-gui feature is not available");
            }
        }
        // Remote console: connect to a running Clay instance via WebSocket
        (false, Some(ref addr)) => {
            return remote_client::run_console_client(addr).await;
        }
        // Master console: default TUI mode (falls through to existing code below)
        (false, None) => {}
    }

    // Set up signal handlers for crash debugging (not available on Android or Windows)
    #[cfg(all(unix, not(target_os = "android")))]
    unsafe {
        extern "C" fn sigfpe_handler(_: libc::c_int) {
            // Restore terminal before printing
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            debug_log(is_debug_enabled(), "CRASH: SIGFPE (Floating Point Exception)");
            eprintln!("\n\n=== SIGFPE (Floating Point Exception) detected! ===");
            eprintln!("This is typically caused by division by zero.");
            eprintln!("Please report this bug with the steps to reproduce.");

            // Try to print a backtrace
            eprintln!("\nBacktrace:");
            let bt = std::backtrace::Backtrace::force_capture();
            let bt_str = format!("{}", bt);
            debug_log(is_debug_enabled(), &format!("BACKTRACE:\n{}", bt_str));
            eprintln!("{}", bt);

            std::process::exit(136);  // 128 + 8 (SIGFPE)
        }
        libc::signal(libc::SIGFPE, sigfpe_handler as *const () as libc::sighandler_t);

        extern "C" fn sigsegv_handler(_: libc::c_int) {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            debug_log(is_debug_enabled(), "CRASH: SIGSEGV (Segmentation Fault)");
            eprintln!("\n\n=== SIGSEGV (Segmentation Fault) detected! ===");
            let bt = std::backtrace::Backtrace::force_capture();
            let bt_str = format!("{}", bt);
            debug_log(is_debug_enabled(), &format!("BACKTRACE:\n{}", bt_str));
            eprintln!("{}", bt);
            std::process::exit(139);  // 128 + 11 (SIGSEGV)
        }
        libc::signal(libc::SIGSEGV, sigsegv_handler as *const () as libc::sighandler_t);

        extern "C" fn sigbus_handler(_: libc::c_int) {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            debug_log(is_debug_enabled(), "CRASH: SIGBUS (Bus Error)");
            eprintln!("\n\n=== SIGBUS (Bus Error) detected! ===");
            let bt = std::backtrace::Backtrace::force_capture();
            let bt_str = format!("{}", bt);
            debug_log(is_debug_enabled(), &format!("BACKTRACE:\n{}", bt_str));
            eprintln!("{}", bt);
            std::process::exit(135);  // 128 + 7 (SIGBUS)
        }
        libc::signal(libc::SIGBUS, sigbus_handler as *const () as libc::sighandler_t);

        extern "C" fn sigabrt_handler(_: libc::c_int) {
            let _ = disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            debug_log(is_debug_enabled(), "CRASH: SIGABRT (Abort)");
            eprintln!("\n\n=== SIGABRT (Abort) detected! ===");
            let bt = std::backtrace::Backtrace::force_capture();
            let bt_str = format!("{}", bt);
            debug_log(is_debug_enabled(), &format!("BACKTRACE:\n{}", bt_str));
            eprintln!("{}", bt);
            std::process::exit(134);  // 128 + 6 (SIGABRT)
        }
        libc::signal(libc::SIGABRT, sigabrt_handler as *const () as libc::sighandler_t);
    }

    // Set up crash handler for automatic recovery (not available on Android)
    #[cfg(not(target_os = "android"))]
    setup_crash_handler();

    enable_raw_mode()?;
    let mut stdout = stdout();
    // Use explicit cursor positioning and clearing for Windows 11 compatibility
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )?;

    // On Windows reload: signal the old process that we've taken over the console.
    // This lets the old process exit without the shell reclaiming the terminal.
    #[cfg(windows)]
    if let Ok(event_name) = std::env::var("CLAY_RELOAD_SYNC_EVENT") {
        extern "system" {
            fn OpenEventA(dwDesiredAccess: u32, bInheritHandle: i32, lpName: *const u8) -> isize;
            fn SetEvent(hEvent: isize) -> i32;
            fn CloseHandle(hObject: isize) -> i32;
        }
        const EVENT_MODIFY_STATE: u32 = 0x0002;
        let name = format!("{}\0", event_name);
        let handle = unsafe { OpenEventA(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if handle != 0 {
            unsafe { SetEvent(handle); }
            unsafe { CloseHandle(handle); }
        }
        std::env::remove_var("CLAY_RELOAD_SYNC_EVENT");
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_app(&mut terminal).await;

    disable_raw_mode()?;
    let _ = execute!(terminal.backend_mut(), DisableMouseCapture);
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = result {
        eprintln!("Error: {err}");
    }

    Ok(())
}

/// Restart the HTTP/HTTPS web server after settings change (port, enabled, or secure mode).
/// Shuts down the current server and starts a new one on the configured port.
pub async fn restart_http_server(app: &mut App, event_tx: mpsc::Sender<AppEvent>) {
    // Collect everything we need before any mutable borrows
    let http_enabled = app.settings.http_enabled;
    let http_port = app.settings.http_port;
    let web_secure = app.settings.web_secure;
    let cert_file = app.settings.websocket_cert_file.clone();
    let key_file = app.settings.websocket_key_file.clone();
    let ban_list = app.ban_list.clone();
    let theme_css = app.gui_theme_colors().to_css_vars();
    let ws_state = app.ws_server.as_ref().map(|server| {
        std::sync::Arc::new(server.connection_state(event_tx.clone()))
    });

    // Shut down existing HTTP server
    if let Some(ref mut server) = app.http_server {
        if let Some(tx) = server.shutdown_tx.take() { let _ = tx.send(()); }
    }
    app.http_server = None;

    // Shut down existing HTTPS server
    #[cfg(any(feature = "native-tls-backend", feature = "rustls-backend"))]
    {
        if let Some(ref mut server) = app.https_server {
            if let Some(tx) = server.shutdown_tx.take() { let _ = tx.send(()); }
        }
        app.https_server = None;
    }

    if !http_enabled {
        return;
    }

    if web_secure {
        #[cfg(any(feature = "native-tls-backend", feature = "rustls-backend"))]
        {
            let mut https_server = HttpsServer::new(http_port);
            match start_https_server(&mut https_server, &cert_file, &key_file, ws_state, ban_list, theme_css).await {
                Ok(()) => {
                    app.add_output(&format!("HTTPS web interface restarted on port {http_port}"));
                    app.https_server = Some(https_server);
                }
                Err(e) => {
                    app.add_output(&format!("Failed to start HTTPS server on port {http_port}: {e}"));
                }
            }
        }
    } else {
        let mut http_server = HttpServer::new(http_port);
        match start_http_server(&mut http_server, ws_state, ban_list, theme_css, None).await {
            Ok(()) => {
                app.add_output(&format!("HTTP web interface restarted on port {http_port}"));
                app.http_server = Some(http_server);
            }
            Err(e) => {
                app.add_output(&format!("Failed to start HTTP server on port {http_port}: {e}"));
            }
        }
    }
}

/// Run the App headlessly (no terminal UI) for master GUI mode.
/// The App communicates with the embedded GUI via channels.
pub async fn run_app_headless(
    gui_tx: mpsc::UnboundedSender<WsMessage>,
    mut gui_rx: mpsc::UnboundedReceiver<WsMessage>,
    ws_override: Option<String>,  // password for auto-started WS (GUI master mode)
    gui_repaint: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
) -> io::Result<()> {
    let mut app = App::new();
    app.gui_tx = Some(gui_tx.clone());
    let gui_repaint_clone = gui_repaint.clone();
    app.gui_repaint = gui_repaint;

    // Check if we're in reload mode (via --reload command line argument)
    let is_reload = std::env::args().any(|a| a == "--reload");
    app.is_reload = is_reload;
    let is_crash = std::env::args().any(|a| a == "--crash");

    // Initialize crash count from environment variable
    let crash_count = get_crash_count();
    CRASH_COUNT.store(crash_count, Ordering::SeqCst);

    let should_load_state = is_reload || is_crash;

    if should_load_state {
        debug_log(true, "HEADLESS STARTUP: Loading reload state...");
        match persistence::load_reload_state(&mut app) {
            Ok(true) => {
                debug_log(true, "HEADLESS STARTUP: Reload state loaded successfully");
                for (idx, world) in app.worlds.iter().enumerate() {
                    debug_log(true, &format!(
                        "HEADLESS RELOAD STATE: World[{}] '{}' showing_splash={} output_lines={} connected={}",
                        idx, world.name, world.showing_splash, world.output_lines.len(), world.connected
                    ));
                }
            }
            Ok(false) => {
                debug_log(true, "HEADLESS STARTUP: No reload state found");
                if let Err(e) = persistence::load_settings(&mut app) {
                    eprintln!("Warning: Could not load settings: {}", e);
                }
            }
            Err(e) => {
                debug_log(true, &format!("HEADLESS STARTUP: Failed to load reload state: {}", e));
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

    // Sync global debug flag from loaded settings
    DEBUG_ENABLED.store(app.settings.debug_enabled, Ordering::Relaxed);

    // Pre-compile action regexes after loading settings or reload state
    compile_all_action_regexes(&mut app.settings.actions);

    // Load theme file (~/.clay.theme.dat)
    load_theme_file(&mut app);

    // Load keyboard bindings (~/.clay.key.dat)
    {
        let home = get_home_dir();
        let key_path = std::path::Path::new(&home).join(clay_filename("clay.key.dat"));
        app.keybindings = keybindings::KeyBindings::load(&key_path);
    }

    app.ensure_has_world();

    // Re-create spell checker with custom dictionary path if configured
    if !app.settings.dictionary_path.is_empty() {
        app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
    }

    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(100);
    app.event_tx = Some(event_tx.clone());

    // Reconstruct connections from saved fds if in reload/crash mode
    #[cfg(any(unix, windows))]
    if should_load_state {
        // Log world state for debugging (always-on)
        for (idx, world) in app.worlds.iter().enumerate() {
            if world.connected || world.socket_fd.is_some() {
                debug_log(true, &format!(
                    "HEADLESS RELOAD: World[{}] '{}' connected={} is_tls={} socket_fd={:?} proxy_pid={:?}",
                    idx, world.name, world.connected, world.is_tls, world.socket_fd, world.proxy_pid
                ));
            }
        }
        debug_log(true, "HEADLESS STARTUP: Reconstructing connections...");
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
            if world_idx != app.current_world_index {
                if app.worlds[world_idx].unseen_lines == 0 {
                    app.worlds[world_idx].first_unseen_at = Some(std::time::Instant::now());
                }
                app.worlds[world_idx].unseen_lines += 1;
            }
        }

        // Second pass: reconstruct plain TCP connections
        for world_idx in 0..app.worlds.len() {
            let world = &app.worlds[world_idx];
            if world.connected && world.socket_fd.is_some() && !world.is_tls {
                let fd = world.socket_fd.unwrap();
                debug_log(true, &format!("HEADLESS RELOAD: Reconstructing plain TCP for '{}' fd={}", world.name, fd));
                #[cfg(unix)]
                let tcp_stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
                #[cfg(windows)]
                let tcp_stream = unsafe { std::net::TcpStream::from_raw_socket(fd as u64) };
                tcp_stream.set_nonblocking(true)?;
                let tcp_stream = TcpStream::from_std(tcp_stream)?;
                let (r, w) = tcp_stream.into_split();
                let mut read_half = StreamReader::Plain(r);
                let mut write_half = StreamWriter::Plain(w);
                let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
                app.worlds[world_idx].skip_auto_login = true;
                app.worlds[world_idx].open_log_file();
                let _telnet_tx = cmd_tx;
                let world_name = app.worlds[world_idx].name.clone();
                app.worlds[world_idx].connection_id += 1;
                app.worlds[world_idx].reader_name = Some(world_name.clone());

                // Spawn reader task
                let reader_tx = event_tx.clone();
                let reader_world_name = world_name.clone();
                let reader_conn_id = app.worlds[world_idx].connection_id;
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    loop {
                        match read_half.read(&mut buf).await {
                            Ok(0) => {
                                let _ = reader_tx.send(AppEvent::Disconnected(reader_world_name, reader_conn_id)).await;
                                break;
                            }
                            Ok(n) => {
                                let data = buf[..n].to_vec();
                                let _ = reader_tx.send(AppEvent::ServerData(reader_world_name.clone(), data)).await;
                            }
                            Err(_) => {
                                let _ = reader_tx.send(AppEvent::Disconnected(reader_world_name, reader_conn_id)).await;
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
                                let data = format!("{}\r\n", text);
                                if write_half.write_all(data.as_bytes()).await.is_err() {
                                    break;
                                }
                            }
                            WriteCommand::Raw(data) => {
                                if write_half.write_all(&data).await.is_err() {
                                    break;
                                }
                            }
                            WriteCommand::Shutdown => break,
                        }
                    }
                });
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
                                let mut read_half = StreamReader::Proxy(r);
                                let mut write_half = StreamWriter::Proxy(w);
                                let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                                app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
                                app.worlds[world_idx].skip_auto_login = true;
                                app.worlds[world_idx].open_log_file();
                                let world_name = app.worlds[world_idx].name.clone();
                                app.worlds[world_idx].connection_id += 1;
                                app.worlds[world_idx].reader_name = Some(world_name.clone());

                                let reader_tx = event_tx.clone();
                                let reader_world_name = world_name.clone();
                                let reader_conn_id = app.worlds[world_idx].connection_id;
                                tokio::spawn(async move {
                                    let mut buf = vec![0u8; 4096];
                                    loop {
                                        match read_half.read(&mut buf).await {
                                            Ok(0) => {
                                                let _ = reader_tx.send(AppEvent::Disconnected(reader_world_name, reader_conn_id)).await;
                                                break;
                                            }
                                            Ok(n) => {
                                                let data = buf[..n].to_vec();
                                                let _ = reader_tx.send(AppEvent::ServerData(reader_world_name.clone(), data)).await;
                                            }
                                            Err(_) => {
                                                let _ = reader_tx.send(AppEvent::Disconnected(reader_world_name, reader_conn_id)).await;
                                                break;
                                            }
                                        }
                                    }
                                });

                                tokio::spawn(async move {
                                    while let Some(cmd) = cmd_rx.recv().await {
                                        match cmd {
                                            WriteCommand::Text(text) => {
                                                let data = format!("{}\r\n", text);
                                                if write_half.write_all(data.as_bytes()).await.is_err() {
                                                    break;
                                                }
                                            }
                                            WriteCommand::Raw(data) => {
                                                if write_half.write_all(&data).await.is_err() {
                                                    break;
                                                }
                                            }
                                            WriteCommand::Shutdown => break,
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                debug_log(is_debug_enabled(), &format!("HEADLESS: Failed to reconnect proxy for {}: {}", app.worlds[world_idx].name, e));
                                app.worlds[world_idx].clear_connection_state(false, false);
                            }
                        }
                    }
                }
            }
        }

        // Cleanup: mark disconnected worlds that claim to be connected but have no command channel
        for world in &mut app.worlds {
            if world.connected && world.command_tx.is_none() {
                debug_log(true, &format!(
                    "HEADLESS RELOAD CLEANUP: World '{}' connected but no command_tx — marking disconnected (socket_fd={:?})",
                    world.name, world.socket_fd
                ));
                world.connected = false;
                world.socket_fd = None;
                world.pending_lines.clear();
                world.paused = false;
                let seq = world.next_seq;
                world.next_seq += 1;
                world.output_lines.push(OutputLine::new(
                    "Connection was not restored during reload. Use /worlds to reconnect.".to_string(), seq
                ));
            }
        }

        // Clear splash screen so the reload message is visible (splash hides output_lines)
        app.current_world_mut().showing_splash = false;
        let reload_msg = if is_crash {
            "Crash recovery complete."
        } else {
            "Hot reload complete."
        };
        app.add_output(reload_msg);
    }

    // In GUI master mode, enable the web interface and set the auto-generated password
    // Force plain HTTP — WebView connects via ws:// on localhost, doesn't need TLS
    if let Some(ref ws_password) = ws_override {
        app.settings.http_enabled = true;
        app.settings.web_secure = false;
        app.settings.websocket_password = ws_password.clone();
    }

    // Create WebSocket server state (for client management, no standalone listener)
    let ws_state = if !app.settings.websocket_password.is_empty() {
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

    // Auto-generate self-signed certs if secure mode enabled but no cert files
    if app.settings.web_secure
        && (app.settings.websocket_cert_file.is_empty() || app.settings.websocket_key_file.is_empty())
    {
        let home = get_home_dir();
        let cert_path = std::path::PathBuf::from(&home).join(clay_filename("clay.cert.pem"));
        let key_path = std::path::PathBuf::from(&home).join(clay_filename("clay.key.pem"));

        let needs_gen = !cert_path.exists() || !key_path.exists();
        let needs_regen = !needs_gen && cert_needs_regeneration(&cert_path);
        if needs_gen || needs_regen {
            if needs_regen {
                app.add_output("\u{2728} IP address changed, regenerating TLS certificate...");
            }
            match generate_self_signed_cert(&cert_path, &key_path) {
                Ok(()) => {
                    app.add_output("\u{2728} Generated self-signed TLS certificate.");
                }
                Err(e) => {
                    app.add_output(&format!("\u{2728} Failed to generate TLS certificate: {}", e));
                    app.add_output("\u{2728} Falling back to non-secure HTTP.");
                }
            }
        }

        if cert_path.exists() && key_path.exists() {
            app.settings.websocket_cert_file = cert_path.to_string_lossy().to_string();
            app.settings.websocket_key_file = key_path.to_string_lossy().to_string();
            let _ = persistence::save_settings(&app);
        }
    }

    // Start unified HTTP+WS server if enabled
    if app.settings.http_enabled {
        if app.settings.web_secure
            && !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty()
        {
            #[cfg(feature = "native-tls-backend")]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
                    ws_state.clone(),
                    app.ban_list.clone(),
                    app.gui_theme_colors().to_css_vars(),
                ).await {
                    Ok(()) => {
                        if !app.is_reload {
                            app.add_output(&format!("HTTPS web interface started on port {}", app.settings.http_port));
                        }
                        app.https_server = Some(https_server);
                    }
                    Err(e) => {
                        app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                    }
                }
            }
            #[cfg(feature = "rustls-backend")]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
                    ws_state.clone(),
                    app.ban_list.clone(),
                    app.gui_theme_colors().to_css_vars(),
                ).await {
                    Ok(()) => {
                        if !app.is_reload {
                            app.add_output(&format!("HTTPS web interface started on port {}", app.settings.http_port));
                        }
                        app.https_server = Some(https_server);
                    }
                    Err(e) => {
                        app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                    }
                }
            }
        } else {
            // Check for inherited HTTP listener handle (from GUI reload)
            let inherited_handle = std::env::var("CLAY_HTTP_LISTENER").ok()
                .and_then(|s| s.parse::<u64>().ok());
            if inherited_handle.is_some() {
                std::env::remove_var("CLAY_HTTP_LISTENER");
            }
            let mut http_server = HttpServer::new(app.settings.http_port);
            match start_http_server(
                &mut http_server,
                ws_state.clone(),
                app.ban_list.clone(),
                app.gui_theme_colors().to_css_vars(),
                inherited_handle,
            ).await {
                Ok(()) => {
                    if !app.is_reload {
                        app.add_output(&format!("HTTP web interface started on port {}", app.settings.http_port));
                    }
                    app.http_server = Some(http_server);
                }
                Err(e) => {
                    app.add_output(&format!("Warning: Failed to start HTTP server: {}", e));
                }
            }
        }
    }

    // Re-set gui_tx and repaint callback after potential reload state load (reload clears them)
    app.gui_tx = Some(gui_tx);
    app.gui_repaint = gui_repaint_clone;

    // Send initial state to the GUI
    let initial_state = app.build_initial_state();
    app.ws_broadcast(initial_state);

    // Keepalive interval
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5 * 60);
    let mut keepalive_interval = tokio::time::interval(Duration::from_secs(60));
    keepalive_interval.tick().await;

    // Conditional timers: sleep far-future when inactive, reset to short interval when needed.
    const FAR_FUTURE: Duration = Duration::from_secs(86400);

    // Prompt timeout detection — only active when a world has wont_echo_time set
    let prompt_check_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(prompt_check_sleep);

    // TF repeat process ticks — only active when processes exist
    let process_tick_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(process_tick_sleep);

    // Pending count broadcast — only active when worlds have pending lines
    let pending_update_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(pending_update_sleep);

    // Auto-reconnect timer — fires when a world is due for reconnection
    let reconnect_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(reconnect_sleep);

    // GUI reload check — polls atomic flag set by IPC handler (100ms interval)
    let mut gui_reload_check = tokio::time::interval(Duration::from_millis(100));
    gui_reload_check.tick().await; // consume first immediate tick

    // SIGUSR1 handler for hot reload
    #[cfg(all(unix, not(target_os = "android")))]
    {
        let sigusr1_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut sigusr1 = match signal(SignalKind::user_defined1()) {
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

    // Run startup actions (including on reload/crash recovery)
    // Note: In headless mode, we only support TF commands (#...) for startup actions
    // since handle_command isn't compatible with spawned tasks
    {
        let startup_actions: Vec<String> = app.settings.actions.iter()
            .filter(|a| a.startup && a.enabled)
            .map(|a| a.command.clone())
            .collect();
        for cmd_str in startup_actions {
            // Split by semicolons and execute each command
            for single_cmd in cmd_str.split(';') {
                let single_cmd = single_cmd.trim();
                if single_cmd.is_empty() { continue; }
                if single_cmd.starts_with('/') {
                    // Unified command system - route through TF parser
                    app.sync_tf_world_info();
                    let _ = app.tf_engine.execute(single_cmd);
                } else {
                    // Plain text can't be run at startup in headless mode
                    debug_log(is_debug_enabled(), &format!("[Startup] Skipped command (headless): {}", single_cmd));
                }
            }
        }
    }

    debug_log(true, "HEADLESS: Entering main event loop");

    // Main event loop
    loop {
        #[cfg(all(unix, not(target_os = "android")))]
        reap_zombie_children();

        tokio::select! {
            // App events (server data, disconnects, WS client messages)
            Some(event) = event_rx.recv() => {
                match event {
                    AppEvent::ServerData(ref world_name, ref bytes) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            // Use large width in daemon mode so text is not pre-wrapped —
                            // GUI/web clients handle wrapping via CSS
                            let commands = app.process_server_data(
                                world_idx, bytes, 24, 1000, true,
                            );
                            // Activate prompt check if server data set wont_echo_time
                            if app.worlds[world_idx].wont_echo_time.is_some() {
                                prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                            }
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
                                            match parsed {
                                                Command::Send { text, target_world, .. } => {
                                                    let target_idx = if let Some(ref w) = target_world {
                                                        app.find_world_index(w)
                                                    } else { Some(world_idx) };
                                                    if let Some(idx) = target_idx {
                                                        if let Some(tx) = &app.worlds[idx].command_tx {
                                                            let _ = tx.send(WriteCommand::Text(text)).await;
                                                        }
                                                    }
                                                }
                                                Command::Notify { message } => {
                                                    let title = if world_idx < app.worlds.len() {
                                                        app.worlds[world_idx].name.clone()
                                                    } else {
                                                        "Clay".to_string()
                                                    };
                                                    app.ws_broadcast(WsMessage::Notification {
                                                        title,
                                                        message: message.clone(),
                                                    });
                                                }
                                                Command::Say { text } => {
                                                    tts::speak(&app.tts_backend, &text, app.settings.tts_mode);
                                                    let clean_text = strip_ansi_codes(&text);
                                                    app.ws_broadcast(WsMessage::ServerSpeak {
                                                        text: clean_text,
                                                        world_index: world_idx,
                                                    });
                                                }
                                                _ => {}
                                            }
                                        }
                                        tf::TfCommandResult::RepeatProcess(process) => {
                                            app.tf_engine.processes.push(process);
                                        }
                                        _ => {}
                                    }
                                } else if let Some(tx) = &app.worlds[world_idx].command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(cmd));
                                }
                            }
                            app.current_world_index = saved_current_world;
                        }
                    }
                    AppEvent::Disconnected(ref world_name, conn_id) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            if conn_id != app.worlds[world_idx].connection_id { continue; }
                            app.handle_disconnected(world_idx);
                            if let Some(next) = app.next_reconnect_instant() {
                                let dur = next.saturating_duration_since(std::time::Instant::now());
                                reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                            }
                        }
                    }
                    AppEvent::WsClientMessage(client_id, msg) => {
                        if let WsMessage::AuthRequest { current_world, .. } = &*msg {
                            app.handle_ws_auth_initial_state(client_id, *current_world);
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
                            daemon::handle_daemon_ws_message(&mut app, client_id, *msg, &event_tx).await;
                            if app.web_restart_needed {
                                app.web_restart_needed = false;
                                restart_http_server(&mut app, event_tx.clone()).await;
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
                    AppEvent::SystemMessage(msg) => {
                        // Add to current world's output and broadcast
                        let seq = app.current_world().next_seq;
                        let world_idx = app.current_world_index;
                        app.worlds[world_idx].next_seq += 1;
                        app.worlds[world_idx].output_lines.push(OutputLine::new_client(msg.clone(), seq));
                        app.ws_broadcast(WsMessage::ServerData {
                            world_index: world_idx,
                            data: format!("{}\n", msg),
                            is_viewed: true,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0,
                            marked_new: false,
                            flush: false, gagged: false,
                        });
                    }
                    AppEvent::Sigusr1Received => {
                        #[cfg(not(target_os = "android"))]
                        {
                            // Always log reload trigger (not gated by debug flag)
                            debug_log(true, "HEADLESS: Reload triggered, calling exec_reload");
                            for (idx, world) in app.worlds.iter().enumerate() {
                                debug_log(true, &format!(
                                    "HEADLESS PRE-RELOAD: World[{}] '{}' showing_splash={} output_lines={} connected={}",
                                    idx, world.name, world.showing_splash, world.output_lines.len(), world.connected
                                ));
                            }
                            app.ws_broadcast(WsMessage::ServerReloading);
                            exec_reload(&mut app)?;
                            return Ok(());
                        }
                    }
                    AppEvent::ConnectionSuccess(world_name, cmd_tx, socket_fd, is_tls) => {
                        app.handle_connection_success(&world_name, cmd_tx, socket_fd, is_tls);
                    }
                    AppEvent::ConnectionFailed(world_name, error) => {
                        if let Some(world_idx) = app.find_world_index(&world_name) {
                            app.add_output_to_world(world_idx, &format!("Connection failed: {}", error));
                        }
                    }
                    AppEvent::TelnetDetected(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_telnet_detected(world_idx);
                        }
                    }
                    AppEvent::WontEchoSeen(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_wont_echo_seen(world_idx);
                        }
                    }
                    AppEvent::NawsRequested(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_naws_requested(world_idx);
                        }
                    }
                    AppEvent::TtypeRequested(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_ttype_requested(world_idx);
                        }
                    }
                    AppEvent::GmcpNegotiated(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_gmcp_negotiated(world_idx);
                        }
                    }
                    AppEvent::MsdpNegotiated(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.worlds[world_idx].msdp_enabled = true;
                        }
                    }
                    AppEvent::GmcpReceived(ref world_name, ref package, ref json_data) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_gmcp_received(world_idx, package, json_data);
                        }
                    }
                    AppEvent::MsdpReceived(ref world_name, ref variable, ref value_json) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_msdp_received(world_idx, variable, value_json);
                        }
                    }
                    AppEvent::MediaFileReady(world_idx, key, path, volume, loops, is_music) => {
                        app.ensure_audio();
                        if let Some(handle) = audio::play_file(&app.audio_backend, &path, volume, loops) {
                            if is_music {
                                app.media_music_key = Some((world_idx, key.clone()));
                            }
                            app.media_processes.insert(key, (world_idx, handle));
                        }
                    }
                    AppEvent::ApiLookupResult(client_id, world_index, result, cursor_start) => {
                        match result {
                            Ok(text) => app.ws_send_to_client(client_id, WsMessage::SetInputBuffer { text, cursor_start }),
                            Err(e) => app.ws_send_to_client(client_id, WsMessage::ServerData {
                                world_index,
                                data: e,
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0,
                                marked_new: false,
                                flush: false, gagged: false,
                            }),
                        }
                    }
                    AppEvent::RemoteListResult(requesting_client_id, world_index, lines) => {
                        app.remote_ping_responses = None;
                        if requesting_client_id == 0 {
                            for line in &lines {
                                app.add_output(line);
                            }
                        } else {
                            for line in &lines {
                                app.ws_send_to_client(requesting_client_id, WsMessage::ServerData {
                                    world_index,
                                    data: line.clone(),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0,
                                    marked_new: false,
                                    flush: false, gagged: false,
                                });
                            }
                        }
                    }
                    AppEvent::UpdateResult(result) => {
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
                                                // rename may fail cross-device; fallback to copy
                                                match std::fs::copy(&success.temp_path, &exe_path) {
                                                    Ok(_) => {
                                                        let _ = std::fs::remove_file(&success.temp_path);
                                                        app.add_output(&format!("Updated to Clay v{} — reloading...", success.version));
                                                        app.ws_broadcast(WsMessage::ServerReloading);
                                                        exec_reload(&mut app)?;
                                                        return Ok(());
                                                    }
                                                    Err(e2) => {
                                                        app.add_output(&format!("Failed to install update: {} (rename: {})", e2, e));
                                                        let _ = std::fs::remove_file(&success.temp_path);
                                                    }
                                                }
                                            } else {
                                                app.add_output(&format!("Updated to Clay v{} — reloading...", success.version));
                                                app.ws_broadcast(WsMessage::ServerReloading);
                                                exec_reload(&mut app)?;
                                                return Ok(());
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
                    }
                    AppEvent::Prompt(ref world_name, ref prompt_bytes) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_prompt(world_idx, prompt_bytes);
                        }
                    }
                    _ => {}
                }
            }

            // GUI messages (same format as WsMessage - reuse daemon handler)
            Some(msg) = gui_rx.recv() => {
                // Use client_id 0 for the embedded GUI (not a real WS client)
                daemon::handle_daemon_ws_message(&mut app, 0, msg, &event_tx).await;
            }

            // Keepalive timer
            _ = keepalive_interval.tick() => {
                for world in &mut app.worlds {
                    if world.connected {
                        let last_activity = match (world.last_send_time, world.last_receive_time) {
                            (Some(s), Some(r)) => Some(s.max(r)),
                            (Some(s), None) => Some(s),
                            (None, Some(r)) => Some(r),
                            (None, None) => None,
                        };
                        let should_send = match last_activity {
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
                                        let idler_tag = format!("###_idler_message_{}_###", rand_num);
                                        let cmd = world.settings.keep_alive_cmd
                                            .replace("##rand##", &idler_tag);
                                        let _ = tx.try_send(WriteCommand::Text(cmd));
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
                for world in &mut app.worlds {
                    if world.connected {
                        if let Some(proxy_pid) = world.proxy_pid {
                            if !is_process_alive(proxy_pid) {
                                world.clear_connection_state(false, false);
                                let seq = world.next_seq;
                                world.next_seq += 1;
                                world.output_lines.push(OutputLine::new("TLS proxy terminated. Connection lost.".to_string(), seq));
                            }
                        }
                    }
                }
            }

            // Prompt timeout check — only fires when a world has wont_echo_time set
            _ = &mut prompt_check_sleep => {
                let now = std::time::Instant::now();
                for world in &mut app.worlds {
                    if let Some(wont_echo_time) = world.wont_echo_time {
                        if now.duration_since(wont_echo_time) >= Duration::from_millis(150) {
                            if !world.trigger_partial_line.is_empty() && world.prompt.is_empty() {
                                let prompt_text = std::mem::take(&mut world.trigger_partial_line);
                                let prompt_clean = prompt_text.replace('\r', "").replace('\n', " ");
                                let normalized = format!("{} ", prompt_clean.trim());

                                if !world.connected {
                                    let seq = world.next_seq;
                                    world.next_seq += 1;
                                    world.output_lines.push(OutputLine::new(normalized.trim().to_string(), seq));
                                    world.wont_echo_time = None;
                                    continue;
                                }

                                world.prompt = normalized;
                                world.prompt_count += 1;

                                // Handle auto-login
                                if !world.skip_auto_login {
                                    let auto_type = world.settings.auto_connect_type;
                                    let user = world.settings.user.clone();
                                    let password = world.settings.password.clone();
                                    let prompt_num = world.prompt_count;

                                    if !user.is_empty() && !password.is_empty() {
                                        let cmd_to_send = match auto_type {
                                            AutoConnectType::Prompt => {
                                                match prompt_num {
                                                    1 => Some(user),
                                                    2 => Some(password),
                                                    _ => None,
                                                }
                                            }
                                            AutoConnectType::MooPrompt => {
                                                match prompt_num {
                                                    1 => Some(user.clone()),
                                                    2 => Some(password),
                                                    3 => Some(user),
                                                    _ => None,
                                                }
                                            }
                                            AutoConnectType::Connect | AutoConnectType::NoLogin => None,
                                        };

                                        if let Some(cmd) = cmd_to_send {
                                            world.prompt.clear();
                                            if let Some(tx) = &world.command_tx {
                                                let _ = tx.try_send(WriteCommand::Text(cmd));
                                            }
                                        }
                                    }
                                }
                            }
                            world.wont_echo_time = None;
                        }
                    }
                }
                // Check for expired FANSI detection windows
                let now_fansi = std::time::Instant::now();
                for world in &mut app.worlds {
                    if let Some(deadline) = world.fansi_detect_until {
                        if now_fansi >= deadline {
                            if let Some(login_cmd) = world.fansi_login_pending.take() {
                                if let Some(tx) = &world.command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(login_cmd));
                                }
                            }
                            world.fansi_detect_until = None;
                        }
                    }
                }
                // Re-arm: check again in 150ms if any world still needs it
                let any_pending = app.worlds.iter().any(|w| w.wont_echo_time.is_some() || w.fansi_detect_until.is_some());
                if any_pending {
                    prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                } else {
                    prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }

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
                        // Sync world info before executing process command
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
                                let seq = app.worlds[world_idx].next_seq;
                                app.worlds[world_idx].next_seq += 1;
                                app.worlds[world_idx].output_lines.push(OutputLine::new_client(msg.clone(), seq));
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: format!("{}\n", msg),
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0,
                                    marked_new: false,
                                    flush: false, gagged: false,
                                });
                            }
                            tf::TfCommandResult::Error(err) => {
                                let seq = app.worlds[world_idx].next_seq;
                                app.worlds[world_idx].next_seq += 1;
                                let err_msg = format!("Error: {}", err);
                                app.worlds[world_idx].output_lines.push(OutputLine::new_client(err_msg.clone(), seq));
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: format!("{}\n", err_msg),
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0,
                                    marked_new: false,
                                    flush: false, gagged: false,
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
                // Re-arm: tick again in 1s if processes remain
                if !app.tf_engine.processes.is_empty() {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(1));
                } else {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }

            // GUI reload check — polls atomic flag set by WebView IPC handler
            _ = gui_reload_check.tick() => {
                if GUI_RELOAD_REQUESTED.swap(false, Ordering::SeqCst) {
                    #[cfg(not(target_os = "android"))]
                    {
                        debug_log(true, "HEADLESS: GUI reload flag detected, calling exec_reload");
                        for (idx, world) in app.worlds.iter().enumerate() {
                            debug_log(true, &format!(
                                "HEADLESS GUI-RELOAD: World[{}] '{}' showing_splash={} output_lines={} connected={}",
                                idx, world.name, world.showing_splash, world.output_lines.len(), world.connected
                            ));
                        }
                        app.ws_broadcast(WsMessage::ServerReloading);
                        exec_reload(&mut app)?;
                        return Ok(());
                    }
                }
            }

            // Pending count update — only fires when worlds have pending lines
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

            // Auto-reconnect timer
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
                            app.add_output_to_world(idx, &format!("Connecting to {}:{}{}...", settings.hostname, settings.port, ssl_msg));
                            // Pass skip_auto_login=true to connect_daemon_world so it doesn't
                            // send auto-login; handle_connection_success handles that instead.
                            match daemon::connect_daemon_world(
                                idx, world_name.clone(), &settings, event_tx.clone(), connection_id, true
                            ).await {
                                Some((cmd_tx, socket_fd, is_tls)) => {
                                    app.handle_connection_success(&world_name, cmd_tx, socket_fd, is_tls);
                                    if let Some(new_idx) = app.find_world_index(&world_name) {
                                        app.add_output_to_world(new_idx, "Connected!");
                                    }
                                }
                                None => {
                                    if let Some(current_idx) = app.find_world_index(&world_name) {
                                        let secs = app.worlds[current_idx].settings.auto_reconnect_secs;
                                        if secs > 0 {
                                            app.worlds[current_idx].reconnect_at = Some(
                                                std::time::Instant::now() + std::time::Duration::from_secs(secs as u64)
                                            );
                                            app.add_output_to_world(current_idx, &format!("Connection failed. Reconnecting in {} seconds...", secs));
                                        } else {
                                            app.add_output_to_world(current_idx, "Connection failed.");
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
        }

        // Activate process tick sleep if processes were added during this iteration
        if !app.tf_engine.processes.is_empty()
            && process_tick_sleep.deadline() > tokio::time::Instant::now() + Duration::from_secs(2)
        {
            process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(1));
        }
    }
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut app = App::new();

    // Check if we're in reload mode (via --reload command line argument)
    let is_reload = std::env::args().any(|a| a == "--reload");
    app.is_reload = is_reload; // Suppress server startup messages during reload
    // Check if we're recovering from a crash (via --crash command line argument)
    let is_crash = std::env::args().any(|a| a == "--crash");

    // Initialize crash count from environment variable
    let crash_count = get_crash_count();
    CRASH_COUNT.store(crash_count, Ordering::SeqCst);

    // Collect any startup messages to display after ensuring we have a world
    let mut startup_messages: Vec<String> = Vec::new();

    // Crash recovery also loads state like reload
    let should_load_state = is_reload || is_crash;

    if should_load_state {
        // Load the reload state
        debug_log(is_debug_enabled(), "STARTUP: Loading reload state...");
        match persistence::load_reload_state(&mut app) {
            Ok(true) => {
                debug_log(is_debug_enabled(), "STARTUP: Reload state loaded successfully");
                // Silent on success - only show errors
            }
            Ok(false) => {
                debug_log(is_debug_enabled(), "STARTUP: No reload state found");
                startup_messages.push("Warning: No reload state found, starting fresh.".to_string());
                if let Err(e) = persistence::load_settings(&mut app) {
                    startup_messages.push(format!("Warning: Could not load settings: {}", e));
                }
            }
            Err(e) => {
                debug_log(is_debug_enabled(), &format!("STARTUP: Failed to load reload state: {}", e));
                startup_messages.push(format!("Warning: Failed to load reload state: {}", e));
                if let Err(e) = persistence::load_settings(&mut app) {
                    startup_messages.push(format!("Warning: Could not load settings: {}", e));
                }
            }
        }
    } else {
        // Normal startup - load settings from file
        if let Err(e) = persistence::load_settings(&mut app) {
            startup_messages.push(format!("Warning: Could not load settings: {}", e));
        }
        // On fresh start, clear all runtime state that was persisted for reload
        // These values are meaningless without active connections
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

    // Sync global debug flag from loaded settings
    DEBUG_ENABLED.store(app.settings.debug_enabled, Ordering::Relaxed);

    // Pre-compile action regexes after loading settings or reload state
    compile_all_action_regexes(&mut app.settings.actions);

    // Load theme file (~/.clay.theme.dat)
    load_theme_file(&mut app);

    // Load keyboard bindings (~/.clay.key.dat)
    {
        let home = get_home_dir();
        let key_path = std::path::Path::new(&home).join(clay_filename("clay.key.dat"));
        app.keybindings = keybindings::KeyBindings::load(&key_path);
    }

    // Ensure we have at least one world (creates initial world only if no worlds loaded)
    debug_log(is_debug_enabled(), "STARTUP: Ensuring has world...");
    app.ensure_has_world();
    debug_log(is_debug_enabled(), &format!("STARTUP: Have {} worlds", app.worlds.len()));

    // Re-create spell checker with custom dictionary path if configured
    if !app.settings.dictionary_path.is_empty() {
        app.spell_checker = SpellChecker::new(&app.settings.dictionary_path);
    }

    // Note: pending_lines and paused state are preserved across reload
    // so that more-mode continues seamlessly. Disconnected worlds have their
    // pending_lines cleared in the cleanup pass below (line ~17205).

    // Now display any startup messages
    debug_log(is_debug_enabled(), "STARTUP: Displaying startup messages...");
    for msg in startup_messages {
        app.add_output(&msg);
    }

    debug_log(is_debug_enabled(), "STARTUP: Creating event channel...");
    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(100);
    app.event_tx = Some(event_tx.clone());

    // If in reload or crash recovery mode, reconstruct connections from saved fds
    #[cfg(any(unix, windows))]
    if should_load_state {
        debug_log(is_debug_enabled(), "STARTUP: Reconstructing connections...");
        // First pass: identify TLS worlds WITHOUT proxy that need to be disconnected
        // TLS worlds WITH proxy will be reconnected via Unix socket
        let mut tls_disconnect_worlds: Vec<usize> = Vec::new();
        for (world_idx, world) in app.worlds.iter().enumerate() {
            if world.connected && world.is_tls && world.proxy_pid.is_none() {
                tls_disconnect_worlds.push(world_idx);
            }
        }

        // Disconnect TLS worlds without proxy
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
            // If not the current world, set unseen_lines for activity indicator
            if world_idx != app.current_world_index {
                if app.worlds[world_idx].unseen_lines == 0 {
                    app.worlds[world_idx].first_unseen_at = Some(std::time::Instant::now());
                }
                app.worlds[world_idx].unseen_lines += 1;
            }
        }

        // Second pass: reconstruct plain TCP connections
        for world_idx in 0..app.worlds.len() {
            let world = &app.worlds[world_idx];
            if world.connected && world.socket_fd.is_some() && !world.is_tls {
                let fd = world.socket_fd.unwrap();

                // Reconstruct TcpStream from the raw fd/handle
                #[cfg(unix)]
                let tcp_stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
                #[cfg(windows)]
                let tcp_stream = unsafe { std::net::TcpStream::from_raw_socket(fd as u64) };
                tcp_stream.set_nonblocking(true)?;
                let tcp_stream = TcpStream::from_std(tcp_stream)?;

                let (r, w) = tcp_stream.into_split();
                let mut read_half = StreamReader::Plain(r);
                let mut write_half = StreamWriter::Plain(w);

                let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
                // Skip auto-login for restored connections (only fresh connects should auto-login)
                app.worlds[world_idx].skip_auto_login = true;

                // Re-open log file if enabled
                app.worlds[world_idx].open_log_file();

                // Clone tx for use in reader (for telnet responses)
                let telnet_tx = cmd_tx;

                // Capture world name for the reader task (stable across world deletions)
                let world_name = app.worlds[world_idx].name.clone();
                app.worlds[world_idx].connection_id += 1;
                app.worlds[world_idx].reader_name = Some(world_name.clone());
                let reader_conn_id = app.worlds[world_idx].connection_id;

                // Spawn reader task
                let event_tx_read = event_tx.clone();
                tokio::spawn(async move {
                    let mut buffer = BytesMut::with_capacity(10240);
                    buffer.resize(10240, 0);
                    let mut line_buffer: Vec<u8> = Vec::new();
                    let mut mccp2: Option<flate2::Decompress> = None;

                    loop {
                        match read_half.read(&mut buffer).await {
                            Ok(0) => {
                                // Send any remaining buffered data
                                if !line_buffer.is_empty() {
                                    let result = process_telnet(&line_buffer);
                                    if !result.responses.is_empty() {
                                        let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                    }
                                    if result.telnet_detected {
                                        let _ = event_tx_read.send(AppEvent::TelnetDetected(world_name.clone())).await;
                                    }
                                    if result.gmcp_negotiated {
                                        let _ = event_tx_read.send(AppEvent::GmcpNegotiated(world_name.clone())).await;
                                    }
                                    if result.msdp_negotiated {
                                        let _ = event_tx_read.send(AppEvent::MsdpNegotiated(world_name.clone())).await;
                                    }
                                    for (pkg, json) in &result.gmcp_data {
                                        let _ = event_tx_read.send(AppEvent::GmcpReceived(world_name.clone(), pkg.clone(), json.clone())).await;
                                    }
                                    for (var, val) in &result.msdp_data {
                                        let _ = event_tx_read.send(AppEvent::MsdpReceived(world_name.clone(), var.clone(), val.clone())).await;
                                    }
                                    // Send prompt FIRST for immediate auto-login response
                                    if let Some(prompt_bytes) = result.prompt {
                                        let _ = event_tx_read.send(AppEvent::Prompt(world_name.clone(), prompt_bytes)).await;
                                    }
                                    // Send remaining data
                                    if !result.cleaned.is_empty() {
                                        let _ = event_tx_read.send(AppEvent::ServerData(world_name.clone(), result.cleaned)).await;
                                    }
                                }
                                let _ = event_tx_read
                                    .send(AppEvent::ServerData(
                                        world_name.clone(),
                                        "Connection closed by server.\n".as_bytes().to_vec(),
                                    ))
                                    .await;
                                let _ = event_tx_read.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
                                break;
                            }
                            Ok(n) => {
                                // MCCP2: decompress if active, otherwise append raw
                                if let Some(ref mut decomp) = mccp2 {
                                    let decompressed = telnet::mccp2_decompress(decomp, &buffer[..n]);
                                    line_buffer.extend_from_slice(&decompressed);
                                } else {
                                    line_buffer.extend_from_slice(&buffer[..n]);
                                }

                                // Find safe split point (complete lines with complete ANSI sequences)
                                let split_at = find_safe_split_point(&line_buffer);

                                // Send data immediately - either up to split point, or all if no incomplete sequences
                                let to_send: Vec<u8> = if split_at > 0 {
                                    line_buffer.drain(..split_at).collect()
                                } else if !line_buffer.is_empty() {
                                    // No safe split point but we have data - send it anyway
                                    std::mem::take(&mut line_buffer)
                                } else {
                                    Vec::new()
                                };

                                if !to_send.is_empty() {
                                    // Process telnet sequences
                                    let result = process_telnet(&to_send);

                                    // Send telnet responses if any
                                    if !result.responses.is_empty() {
                                        let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                    }

                                    // MCCP2: if activated, decompress remaining data
                                    if result.mccp2_activated {
                                        let mut decomp = flate2::Decompress::new(true);
                                        // Decompress tail of current chunk (after IAC SB MCCP2 IAC SE)
                                        if result.mccp2_offset < to_send.len() {
                                            let tail = telnet::mccp2_decompress(&mut decomp, &to_send[result.mccp2_offset..]);
                                            // Prepend decompressed tail to line_buffer
                                            let mut new_buf = tail;
                                            new_buf.append(&mut line_buffer);
                                            line_buffer = new_buf;
                                        }
                                        // Also decompress any remaining line_buffer content (was raw compressed)
                                        if !line_buffer.is_empty() && result.mccp2_offset >= to_send.len() {
                                            let remaining = std::mem::take(&mut line_buffer);
                                            line_buffer = telnet::mccp2_decompress(&mut decomp, &remaining);
                                        }
                                        mccp2 = Some(decomp);
                                    }

                                    // Notify if telnet detected
                                    if result.telnet_detected {
                                        let _ = event_tx_read
                                            .send(AppEvent::TelnetDetected(world_name.clone()))
                                            .await;
                                    }

                                    // Notify if NAWS was requested (server sent DO NAWS)
                                    if result.naws_requested {
                                        let _ = event_tx_read
                                            .send(AppEvent::NawsRequested(world_name.clone()))
                                            .await;
                                    }

                                    // Notify if TTYPE was requested (server sent SB TTYPE SEND)
                                    if result.ttype_requested {
                                        let _ = event_tx_read
                                            .send(AppEvent::TtypeRequested(world_name.clone()))
                                            .await;
                                    }

                                    // Notify if CHARSET was requested (RFC 2066)
                                    if let Some(ref charsets) = result.charset_request {
                                        let _ = event_tx_read.send(AppEvent::CharsetRequested(world_name.clone(), charsets.clone())).await;
                                    }

                                    // Notify GMCP/MSDP negotiation and data
                                    if result.gmcp_negotiated {
                                        let _ = event_tx_read.send(AppEvent::GmcpNegotiated(world_name.clone())).await;
                                    }
                                    if result.msdp_negotiated {
                                        let _ = event_tx_read.send(AppEvent::MsdpNegotiated(world_name.clone())).await;
                                    }
                                    for (pkg, json) in &result.gmcp_data {
                                        let _ = event_tx_read.send(AppEvent::GmcpReceived(world_name.clone(), pkg.clone(), json.clone())).await;
                                    }
                                    for (var, val) in &result.msdp_data {
                                        let _ = event_tx_read.send(AppEvent::MsdpReceived(world_name.clone(), var.clone(), val.clone())).await;
                                    }

                                    // Send prompt FIRST if detected via telnet GA/EOR
                                    if let Some(prompt_bytes) = result.prompt {
                                        let _ = event_tx_read
                                            .send(AppEvent::Prompt(world_name.clone(), prompt_bytes))
                                            .await;
                                    }

                                    // Send cleaned data to main loop
                                    if !result.cleaned.is_empty()
                                        && event_tx_read
                                            .send(AppEvent::ServerData(world_name.clone(), result.cleaned))
                                            .await
                                            .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                let msg = format!("Read error: {}", e);
                                let _ = event_tx_read
                                    .send(AppEvent::ServerData(world_name.clone(), msg.into_bytes()))
                                    .await;
                                let _ = event_tx_read.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
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
            }
        }

        // Third pass: restore TLS proxy connections (Unix only - TLS Proxy uses Unix sockets)
        // The proxy is designed to accept reconnections - when exec() happens, the old
        // client connection closes and the proxy waits for a new connection on its listener.
        // We reconnect via the Unix socket path rather than trying to preserve FDs.
        #[cfg(all(unix, not(target_os = "android")))]
        for world_idx in 0..app.worlds.len() {
            let world = &app.worlds[world_idx];
            if world.connected && world.is_tls && world.proxy_pid.is_some() {
                let proxy_pid = world.proxy_pid.unwrap();
                let socket_path = world.proxy_socket_path.clone();

                // Check if proxy is still alive
                let proxy_alive = is_process_alive(proxy_pid);

                if !proxy_alive {
                    // Proxy died, mark disconnected
                    app.worlds[world_idx].clear_connection_state(false, false);
                    let seq = app.worlds[world_idx].next_seq;
                    app.worlds[world_idx].next_seq += 1;
                    app.worlds[world_idx].output_lines.push(OutputLine::new(
                        "TLS proxy terminated during reload. Use /worlds to reconnect.".to_string(), seq
                    ));
                    continue;
                }

                // Reconnect to proxy via Unix socket with retry logic
                // The proxy accepts reconnections after the old client disconnects
                let unix_stream: Option<tokio::net::UnixStream> = if let Some(ref path) = socket_path {
                    if path.exists() {
                        let mut result = None;
                        // More attempts with longer delays to give proxy time to accept
                        for attempt in 0..20 {
                            match tokio::net::UnixStream::connect(path).await {
                                Ok(stream) => {
                                    result = Some(stream);
                                    break;
                                }
                                Err(_) => {
                                    if attempt < 19 {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                                    }
                                }
                            }
                        }
                        result
                    } else {
                        None
                    }
                } else {
                    None
                };

                match unix_stream {
                    Some(unix_stream) => {
                        // Store the new FD for future reloads
                        #[cfg(unix)]
                        {
                            use std::os::unix::io::AsRawFd;
                            let new_fd = unix_stream.as_raw_fd();
                            app.worlds[world_idx].proxy_socket_fd = Some(new_fd);
                        }
                        let (r, w) = unix_stream.into_split();
                        let mut read_half = StreamReader::Proxy(r);
                        let mut write_half = StreamWriter::Proxy(w);

                        let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                        app.worlds[world_idx].command_tx = Some(cmd_tx.clone());
                        app.worlds[world_idx].skip_auto_login = true;

                        // Re-open log file if enabled
                        app.worlds[world_idx].open_log_file();

                        let world_name = app.worlds[world_idx].name.clone();
                        app.worlds[world_idx].connection_id += 1;
                        app.worlds[world_idx].reader_name = Some(world_name.clone());
                        let reader_conn_id = app.worlds[world_idx].connection_id;

                        // Spawn reader task with telnet processing
                        let event_tx_read = event_tx.clone();
                        let telnet_tx = cmd_tx.clone();
                        tokio::spawn(async move {
                            let mut buf = [0u8; 4096];
                            let mut line_buffer = Vec::new();
                            let mut mccp2: Option<flate2::Decompress> = None;
                            loop {
                                match tokio::io::AsyncReadExt::read(&mut read_half, &mut buf).await {
                                    Ok(0) => {
                                        if !line_buffer.is_empty() {
                                            let result = process_telnet(&line_buffer);
                                            if !result.responses.is_empty() {
                                                let _ = telnet_tx.send(WriteCommand::Raw(result.responses)).await;
                                            }
                                            if result.telnet_detected {
                                                let _ = event_tx_read.send(AppEvent::TelnetDetected(world_name.clone())).await;
                                            }
                                            if result.gmcp_negotiated {
                                                let _ = event_tx_read.send(AppEvent::GmcpNegotiated(world_name.clone())).await;
                                            }
                                            if result.msdp_negotiated {
                                                let _ = event_tx_read.send(AppEvent::MsdpNegotiated(world_name.clone())).await;
                                            }
                                            for (pkg, json) in &result.gmcp_data {
                                                let _ = event_tx_read.send(AppEvent::GmcpReceived(world_name.clone(), pkg.clone(), json.clone())).await;
                                            }
                                            for (var, val) in &result.msdp_data {
                                                let _ = event_tx_read.send(AppEvent::MsdpReceived(world_name.clone(), var.clone(), val.clone())).await;
                                            }
                                            if let Some(prompt_bytes) = result.prompt {
                                                let _ = event_tx_read.send(AppEvent::Prompt(world_name.clone(), prompt_bytes)).await;
                                            }
                                            if !result.cleaned.is_empty() {
                                                let _ = event_tx_read.send(AppEvent::ServerData(world_name.clone(), result.cleaned)).await;
                                            }
                                        }
                                        let _ = event_tx_read.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
                                        break;
                                    }
                                    Ok(n) => {
                                        if let Some(ref mut decomp) = mccp2 {
                                            let decompressed = telnet::mccp2_decompress(decomp, &buf[..n]);
                                            line_buffer.extend_from_slice(&decompressed);
                                        } else {
                                            line_buffer.extend_from_slice(&buf[..n]);
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
                                                let _ = event_tx_read.send(AppEvent::TelnetDetected(world_name.clone())).await;
                                            }
                                            if result.naws_requested {
                                                let _ = event_tx_read.send(AppEvent::NawsRequested(world_name.clone())).await;
                                            }
                                            if result.ttype_requested {
                                                let _ = event_tx_read.send(AppEvent::TtypeRequested(world_name.clone())).await;
                                            }
                                            if let Some(ref charsets) = result.charset_request {
                                                let _ = event_tx_read.send(AppEvent::CharsetRequested(world_name.clone(), charsets.clone())).await;
                                            }
                                            if result.gmcp_negotiated {
                                                let _ = event_tx_read.send(AppEvent::GmcpNegotiated(world_name.clone())).await;
                                            }
                                            if result.msdp_negotiated {
                                                let _ = event_tx_read.send(AppEvent::MsdpNegotiated(world_name.clone())).await;
                                            }
                                            for (pkg, json) in &result.gmcp_data {
                                                let _ = event_tx_read.send(AppEvent::GmcpReceived(world_name.clone(), pkg.clone(), json.clone())).await;
                                            }
                                            for (var, val) in &result.msdp_data {
                                                let _ = event_tx_read.send(AppEvent::MsdpReceived(world_name.clone(), var.clone(), val.clone())).await;
                                            }
                                            if let Some(prompt_bytes) = result.prompt {
                                                let _ = event_tx_read.send(AppEvent::Prompt(world_name.clone(), prompt_bytes)).await;
                                            }
                                            if !result.cleaned.is_empty() {
                                                let _ = event_tx_read.send(AppEvent::ServerData(world_name.clone(), result.cleaned)).await;
                                            }
                                        }
                                    }
                                    Err(_) => {
                                        let _ = event_tx_read.send(AppEvent::Disconnected(world_name.clone(), reader_conn_id)).await;
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
                    }
                    None => {
                        // Failed to reconnect
                        app.worlds[world_idx].clear_connection_state(false, false);
                        let seq = app.worlds[world_idx].next_seq;
                        app.worlds[world_idx].next_seq += 1;
                        app.worlds[world_idx].output_lines.push(OutputLine::new(
                            "Failed to reconnect to TLS proxy. Use /worlds to reconnect.".to_string(), seq
                        ));
                    }
                }
            }
        }

        // Final cleanup pass: mark any world as disconnected if it claims to be connected
        // but has no command channel (meaning the connection wasn't successfully reconstructed)
        for world in &mut app.worlds {
            if world.connected && world.command_tx.is_none() {
                world.connected = false;
                world.socket_fd = None;
                let seq = world.next_seq;
                world.next_seq += 1;
                world.output_lines
                    .push(OutputLine::new("Connection was not restored during reload. Use /worlds to reconnect.".to_string(), seq));
            }

            // For ALL worlds: flush pending_lines to output_lines so content isn't lost,
            // then clear more-mode state. This prevents stale activity indicators after reload.
            if !world.pending_lines.is_empty() {
                world.output_lines.append(&mut world.pending_lines);
                // Update scroll_offset to include the newly appended lines
                world.scroll_offset = world.output_lines.len().saturating_sub(1);
            } else if world.scroll_offset >= world.output_lines.len() {
                // Only adjust if scroll_offset is past the end (shouldn't happen, but be safe)
                world.scroll_offset = world.output_lines.len().saturating_sub(1);
            }
            world.pending_since = None;
            world.paused = false;

            // Clear unseen_lines for disconnected worlds only
            // (connected worlds may have received new output during reload)
            if !world.connected {
                world.unseen_lines = 0;
                world.first_unseen_at = None;
            }
        }

        debug_log(is_debug_enabled(), "STARTUP: Connection cleanup done, sending keepalives...");

        // Send immediate keepalive for all reconnected worlds since we don't know how long they were idle
        for world in &mut app.worlds {
            if world.connected {
                if let Some(tx) = &world.command_tx {
                    let now = std::time::Instant::now();

                    // Initialize timing fields for /connections display after reload
                    world.last_receive_time = Some(now);
                    world.last_user_command_time = Some(now);
                    match world.settings.keep_alive_type {
                        KeepAliveType::None => {
                            world.last_send_time = Some(now);
                        }
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
                            let idler_tag = format!("###_idler_message_{}_###", rand_num);
                            let cmd = world.settings.keep_alive_cmd
                                .replace("##rand##", &idler_tag);
                            let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                            world.last_send_time = Some(now);
                            world.last_nop_time = Some(now);
                        }
                        KeepAliveType::Generic => {
                            let rand_num = (std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos() % 1000 + 1) as u32;
                            let cmd = format!("help commands ###_idler_message_{}_###", rand_num);
                            let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                            world.last_send_time = Some(now);
                            world.last_nop_time = Some(now);
                        }
                    }
                }
            }
        }

        // Clear splash screen so the reload message is visible (splash hides output_lines)
        app.current_world_mut().showing_splash = false;
        let reload_msg = if is_crash {
            "Crash recovery complete."
        } else {
            "Hot reload complete."
        };
        app.add_output(reload_msg);
    }

    debug_log(is_debug_enabled(), "STARTUP: Keepalives sent, starting servers...");

    // Create WebSocket server state (for client management, no standalone listener)
    let ws_state = if !app.settings.websocket_password.is_empty() {
        let server = WebSocketServer::new(
            &app.settings.websocket_password,
            app.settings.http_port,
            &app.settings.websocket_allow_list,
            app.settings.websocket_whitelisted_host.clone(),
            app.multiuser_mode,
            app.ban_list.clone(),
        );
        let state = Arc::new(server.connection_state(event_tx.clone()));
        app.ws_server = Some(server);
        Some(state)
    } else {
        None
    };

    // Start unified HTTP+WS server if enabled
    if app.settings.http_enabled {
        // Auto-generate self-signed certs if secure mode enabled but no cert files
        if app.settings.web_secure
            && (app.settings.websocket_cert_file.is_empty() || app.settings.websocket_key_file.is_empty())
        {
            let home = get_home_dir();
            let cert_path = std::path::PathBuf::from(&home).join(clay_filename("clay.cert.pem"));
            let key_path = std::path::PathBuf::from(&home).join(clay_filename("clay.key.pem"));

            let needs_gen = !cert_path.exists() || !key_path.exists();
            let needs_regen = !needs_gen && cert_needs_regeneration(&cert_path);
            if needs_gen || needs_regen {
                if needs_regen {
                    app.add_output("\u{2728} IP address changed, regenerating TLS certificate...");
                }
                match generate_self_signed_cert(&cert_path, &key_path) {
                    Ok(()) => {
                        app.add_output("\u{2728} Generated self-signed TLS certificate.");
                    }
                    Err(e) => {
                        app.add_output(&format!("\u{2728} Failed to generate TLS certificate: {}", e));
                        app.add_output("\u{2728} Falling back to non-secure HTTP.");
                    }
                }
            }

            if cert_path.exists() && key_path.exists() {
                app.settings.websocket_cert_file = cert_path.to_string_lossy().to_string();
                app.settings.websocket_key_file = key_path.to_string_lossy().to_string();
                let _ = persistence::save_settings(&app);
            }
        }
        if app.settings.web_secure
            && !app.settings.websocket_cert_file.is_empty()
            && !app.settings.websocket_key_file.is_empty()
        {
            // Start HTTPS+WSS server (secure mode)
            #[cfg(feature = "native-tls-backend")]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
                    ws_state.clone(),
                    app.ban_list.clone(),
                    app.gui_theme_colors().to_css_vars(),
                ).await {
                    Ok(()) => {
                        if !app.is_reload {
                            app.add_output(&format!("HTTPS web interface started on port {}", app.settings.http_port));
                        }
                        app.https_server = Some(https_server);
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                            app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                        }
                    }
                }
            }
            #[cfg(feature = "rustls-backend")]
            {
                let mut https_server = HttpsServer::new(app.settings.http_port);
                match start_https_server(
                    &mut https_server,
                    &app.settings.websocket_cert_file,
                    &app.settings.websocket_key_file,
                    ws_state.clone(),
                    app.ban_list.clone(),
                    app.gui_theme_colors().to_css_vars(),
                ).await {
                    Ok(()) => {
                        if !app.is_reload {
                            app.add_output(&format!("HTTPS web interface started on port {}", app.settings.http_port));
                        }
                        app.https_server = Some(https_server);
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                            app.add_output(&format!("Warning: Failed to start HTTPS server: {}", e));
                        }
                    }
                }
            }
        } else {
            // Start HTTP+WS server (non-secure mode)
            let mut http_server = HttpServer::new(app.settings.http_port);
            match start_http_server(
                &mut http_server,
                ws_state.clone(),
                app.ban_list.clone(),
                app.gui_theme_colors().to_css_vars(),
                None,
            ).await {
                Ok(()) => {
                    if !app.is_reload {
                        app.add_output(&format!("HTTP web interface started on port {}", app.settings.http_port));
                    }
                    app.http_server = Some(http_server);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if !err_str.contains("Address in use") && !err_str.contains("address already in use") {
                        app.add_output(&format!("Warning: Failed to start HTTP server: {}", e));
                    }
                }
            }
        }
    }

    // Keepalive: send NOP every 5 minutes if telnet mode and idle
    const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5 * 60);

    // Use async event stream instead of polling to reduce CPU usage
    let mut event_stream = EventStream::new();

    // Spawn SIGUSR1 handler task for hot reload (not available on Android or Windows)
    #[cfg(all(unix, not(target_os = "android")))]
    {
        let sigusr1_tx = event_tx.clone();
        tokio::spawn(async move {
            let mut sigusr1 = match signal(SignalKind::user_defined1()) {
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

    // Reset terminal state after reload — clear stale mouse capture, etc.
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    app.mouse_capture_active = false;

    // Initial draw - after reload, we need to force a complete redraw
    // because the terminal state may be inconsistent
    terminal.clear()?;
    // Force ratatui to redraw everything by resizing (clears internal buffer cache)
    terminal.resize(terminal.size()?)?;
    terminal.draw(|f| ui(f, &mut app))?;
    // Render output with crossterm (needed after reload when ratatui early-returns)
    render_output_crossterm(&app);
    // Flush stdout to ensure everything is displayed
    std::io::Write::flush(&mut std::io::stdout())?;

    // Create a persistent interval for periodic tasks (clock updates, keepalive checks, error timeouts)
    let mut keepalive_interval = tokio::time::interval(Duration::from_secs(15));
    // Skip the first tick which fires immediately
    keepalive_interval.tick().await;

    // Conditional timers: sleep far-future when inactive, reset to short interval when needed.
    // This eliminates ~8 wakeups/sec when idle (was: 6.7 + 1.0 + 0.5).
    const FAR_FUTURE: Duration = Duration::from_secs(86400);

    // Prompt timeout detection — only active when a world has wont_echo_time set
    let prompt_check_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(prompt_check_sleep);

    // TF repeat process ticks — only active when processes exist
    let process_tick_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(process_tick_sleep);

    // Pending count broadcast — only active when worlds have pending lines
    let pending_update_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(pending_update_sleep);

    // Auto-reconnect timer — fires when a world is due for reconnection
    let reconnect_sleep = tokio::time::sleep(FAR_FUTURE);
    tokio::pin!(reconnect_sleep);

    // Set the app pointer for crash recovery
    // SAFETY: app lives for the duration of this function and the pointer is only used
    // in the panic hook which only runs while this function is on the stack
    set_app_ptr(&mut app as *mut App);

    // Track if we've cleared the crash count after successful user input
    let mut crash_count_cleared = false;

    // Run startup actions (including on reload/crash recovery)
    {
        let startup_actions: Vec<String> = app.settings.actions.iter()
            .filter(|a| a.startup && a.enabled)
            .map(|a| a.command.clone())
            .collect();
        for cmd_str in startup_actions {
            // Split by semicolons and execute each command
            for single_cmd in cmd_str.split(';') {
                let single_cmd = single_cmd.trim();
                if single_cmd.is_empty() { continue; }
                if single_cmd.starts_with('/') {
                    // Unified command system - route through TF parser
                    app.sync_tf_world_info();
                    match app.tf_engine.execute(single_cmd) {
                        tf::TfCommandResult::Success(Some(msg)) => {
                            app.add_output(&msg);
                        }
                        tf::TfCommandResult::Error(err) => {
                            app.add_output(&format!("Error: {}", err));
                        }
                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                            handle_command(&clay_cmd, &mut app, event_tx.clone()).await;
                        }
                        tf::TfCommandResult::RepeatProcess(process) => {
                            app.tf_engine.processes.push(process);
                        }
                        _ => {}
                    }
                } else {
                    // Text to send to server - but we're not connected yet
                    // Just add it to output as a note
                    app.add_output(&format!("[Startup] Would send: {}", single_cmd));
                }
            }
        }
    }

    debug_log(is_debug_enabled(), "STARTUP: Entering main event loop");

    // Counter for debugging first few loop iterations
    let mut loop_count: u64 = 0;

    loop {
        loop_count += 1;
        // Log first 5 iterations to debug early crashes
        if loop_count <= 5 {
            debug_log(is_debug_enabled(), &format!("LOOP: iteration {}", loop_count));
        }

        // Track whether this iteration changed visible state requiring a redraw
        let mut needs_draw = false;

        // Reap any zombie child processes (TLS proxies that have exited)
        // This is a fast non-blocking call that prevents defunct processes from accumulating
        #[cfg(all(unix, not(target_os = "android")))]
        reap_zombie_children();

        // Use tokio::select! to efficiently wait for events without busy-polling
        tokio::select! {
            // Terminal events (keyboard and mouse input)
            maybe_event = event_stream.next() => {
                needs_draw = true;
                if let Some(Ok(event)) = maybe_event {
                // Handle mouse events
                if let Event::Mouse(mouse) = event {
                    if !app.settings.mouse_enabled {
                        continue;
                    }
                    if app.has_new_popup() {
                        // Popup mode: existing popup mouse handling
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                // Try highlight first (content area click)
                                if !handle_popup_mouse_highlight_start(&mut app, mouse.column, mouse.row) {
                                    // Not a content area click - try button/field click
                                    let button_clicked = handle_popup_mouse_click(&mut app, mouse.column, mouse.row);
                                    if button_clicked {
                                        let enter_key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
                                        let action = handle_key_event(enter_key, &mut app);
                                        if let KeyAction::SendCommand(cmd) = action {
                                            if handle_command(&cmd, &mut app, event_tx.clone()).await {
                                                return Ok(());
                                            }
                                        }
                                    }
                                }
                                // Don't skip draw — button clicks may open new popups
                                // whose hit_areas need to be populated by the render pass
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                handle_popup_mouse_highlight_drag(&mut app, mouse.column, mouse.row);
                                continue;
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                handle_popup_mouse_highlight_end(&mut app);
                                continue;
                            }
                            MouseEventKind::ScrollUp => {
                                if let Some(state) = app.popup_manager.current_mut() {
                                    state.mouse_scroll_up();
                                }
                                let tc = app.settings.theme;
                                if let Some(state) = app.popup_manager.current() {
                                    popup::console_renderer::render_popup_content_direct(state, &tc);
                                }
                                continue;
                            }
                            MouseEventKind::ScrollDown => {
                                if let Some(state) = app.popup_manager.current_mut() {
                                    state.mouse_scroll_down();
                                }
                                let tc = app.settings.theme;
                                if let Some(state) = app.popup_manager.current() {
                                    popup::console_renderer::render_popup_content_direct(state, &tc);
                                }
                                continue;
                            }
                            _ => { continue; }
                        }
                    } else {
                        continue;
                    }
                }
                if let Event::Paste(ref text) = event {
                    // Bracketed paste: entire pasted text arrives as one event.
                    // Insert all characters directly into the input buffer.
                    for c in text.chars() {
                        if c == '\n' || c == '\r' {
                            // Pastes may contain newlines — insert literal newline
                            app.input.insert_char('\n');
                        } else if !c.is_control() {
                            app.input.insert_char(c);
                        }
                    }
                    app.last_input_was_delete = false;
                    // Fall through to the draw at the end of the loop
                } else if let Event::Key(key) = event {
                    if key.kind != KeyEventKind::Press { continue; }
                    match handle_key_event(key, &mut app) {
                        KeyAction::Quit => return Ok(()),
                        KeyAction::Redraw => {
                            // Filter output to only show server data (remove client-generated lines)
                            app.current_world_mut().filter_to_server_output();
                            // Full terminal reset: unconditionally tear down and re-setup
                            // Always disable mouse capture to clear any stuck state
                            let _ = execute!(std::io::stdout(), DisableMouseCapture);
                            app.mouse_capture_active = false;
                            let _ = execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
                            disable_raw_mode()?;
                            execute!(std::io::stdout(), LeaveAlternateScreen)?;
                            enable_raw_mode()?;
                            execute!(
                                std::io::stdout(),
                                EnterAlternateScreen,
                                crossterm::event::EnableBracketedPaste,
                                Clear(ClearType::All),
                                cursor::MoveTo(0, 0)
                            )?;
                            // Re-enable mouse capture only if a popup is currently visible
                            if app.settings.mouse_enabled && app.has_new_popup() {
                                let _ = execute!(std::io::stdout(), EnableMouseCapture);
                                app.mouse_capture_active = true;
                            }
                            terminal.clear()?;
                            app.needs_output_redraw = true;
                        }
                        KeyAction::Connect => {
                            if !app.current_world().connected
                                && handle_command("/__connect", &mut app, event_tx.clone()).await
                            {
                                return Ok(());
                            }
                        }
                        KeyAction::Reload => {
                            if handle_command("/reload", &mut app, event_tx.clone()).await {
                                return Ok(());
                            }
                        }
                        KeyAction::Suspend => {
                            // Process suspension not available on Android or Windows
                            #[cfg(all(unix, not(target_os = "android")))]
                            {
                                // Restore terminal to normal mode before suspending
                                if app.mouse_capture_active {
                                    let _ = execute!(std::io::stdout(), DisableMouseCapture);
                                }
                                let _ = execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
                                disable_raw_mode()?;
                                execute!(std::io::stdout(), LeaveAlternateScreen)?;

                                // Send SIGTSTP to self to suspend
                                unsafe {
                                    libc::kill(libc::getpid(), libc::SIGTSTP);
                                }

                                // When we resume (after fg), re-enter raw mode and redraw
                                enable_raw_mode()?;
                                execute!(std::io::stdout(), EnterAlternateScreen, crossterm::event::EnableBracketedPaste)?;
                                if app.mouse_capture_active {
                                    let _ = execute!(std::io::stdout(), EnableMouseCapture);
                                }
                                terminal.clear()?;
                                app.needs_output_redraw = true;
                            }
                            #[cfg(any(target_os = "android", not(unix)))]
                            {
                                app.add_output("Process suspension (Ctrl+Z) is not available on this platform.");
                            }
                        }
                        KeyAction::SendCommand(cmd) => {
                            // Clear splash on first user input (same as server data)
                            if app.current_world().showing_splash {
                                let world = app.current_world_mut();
                                world.showing_splash = false;
                                world.needs_redraw = true;
                                world.output_lines.clear();
                                world.first_marked_new_index = None;
                                world.scroll_offset = 0;
                                terminal.clear()?;
                                terminal.resize(terminal.size()?)?;
                            }
                            // Clear crash count after first successful user input
                            // This indicates the client is stable and running normally
                            if !crash_count_cleared {
                                clear_crash_count();
                                crash_count_cleared = true;
                            }

                            app.spell_state.reset();
                            app.suggestion_message = None;

                            // Check for TF-style /N pattern shorthand (e.g., /10 *combat*)
                            if cmd.starts_with('/') && cmd.len() > 1 {
                                let rest = &cmd[1..];
                                // Check if it starts with a number followed by space and pattern
                                if let Some(space_pos) = rest.find(char::is_whitespace) {
                                    let num_part = &rest[..space_pos];
                                    if num_part.chars().all(|c| c.is_ascii_digit()) && !num_part.is_empty() {
                                        let pattern = rest[space_pos..].trim();
                                        if !pattern.is_empty() {
                                            // Convert to /recall /N pattern - sync world info first
                                            app.sync_tf_world_info();
                                            let recall_cmd = format!("/recall /{} {}", num_part, pattern);
                                            match app.tf_engine.execute(&recall_cmd) {
                                                tf::TfCommandResult::Recall(opts) => {
                                                    let output_lines = app.current_world().output_lines.clone();
                                                    let (matches, header) = execute_recall(&opts, &output_lines);
                                                    let pattern_str = opts.pattern.as_deref().unwrap_or("*");

                                                    if !opts.quiet {
                                                        if let Some(h) = header {
                                                            app.add_tf_output(&h);
                                                        }
                                                    }
                                                    if matches.is_empty() {
                                                        app.add_tf_output(&format!("No matches for '{}'", pattern_str));
                                                    } else {
                                                        for m in &matches {
                                                            app.add_tf_output(m);
                                                        }
                                                    }
                                                    if !opts.quiet {
                                                        app.add_tf_output("================= Recall end =================");
                                                    }
                                                }
                                                tf::TfCommandResult::Error(err) => {
                                                    app.add_tf_output(&format!("Error: {}", err));
                                                }
                                                _ => {}
                                            }
                                            continue;
                                        }
                                    }
                                }
                            }

                            if cmd.starts_with('/') {
                                // Unified command system - route through TF parser first
                                // TF parser will return ClayCommand for Clay-specific commands
                                app.sync_tf_world_info();
                                match app.tf_engine.execute(&cmd) {
                                    tf::TfCommandResult::Success(Some(msg)) => {
                                        app.add_tf_output(&msg);
                                    }
                                    tf::TfCommandResult::Success(None) => {
                                        // Silent success
                                    }
                                    tf::TfCommandResult::Error(err) => {
                                        app.add_tf_output(&format!("Error: {}", err));
                                    }
                                    tf::TfCommandResult::SendToMud(text) => {
                                        if app.current_world().connected {
                                            if let Some(tx) = &app.current_world().command_tx {
                                                if tx.send(WriteCommand::Text(text)).await.is_err() {
                                                    app.add_tf_output("Failed to send command");
                                                } else {
                                                    let now = std::time::Instant::now();
                                                    app.current_world_mut().last_send_time = Some(now);
                                                    app.current_world_mut().last_user_command_time = Some(now);
                                                    app.current_world_mut().prompt.clear();
                                                    // Reset more-mode counter after successfully sending
                                                    app.current_world_mut().lines_since_pause = 0;
                                                }
                                            }
                                        } else {
                                            app.add_output("Not connected. Use /worlds to connect.");
                                        }
                                    }
                                    tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                        if handle_command(&clay_cmd, &mut app, event_tx.clone()).await {
                                            return Ok(());
                                        }
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
                                        let id = process.id;
                                        let interval = format_duration_short(process.interval);
                                        let count_str = process.count.map_or("infinite".to_string(), |c| c.to_string());
                                        let cmd = process.command.clone();
                                        app.tf_engine.processes.push(process);
                                        app.add_tf_output(&format!("% Process {} started: {} every {} ({} times)", id, cmd, interval, count_str));
                                    }
                                    tf::TfCommandResult::Quote { mut lines, disposition, world, delay_secs, recall_opts } => {
                                        // If this is a /quote with backtick /recall, execute the recall now
                                        if let Some((opts, recall_prefix)) = recall_opts {
                                            let output_lines = app.current_world().output_lines.clone();
                                            let (matches, _header) = execute_recall(&opts, &output_lines);
                                            lines = matches.iter()
                                                .map(|line| format!("{}{}", recall_prefix, line))
                                                .collect();
                                            if lines.is_empty() {
                                                let pattern_str = opts.pattern.as_deref().unwrap_or("*");
                                                app.add_tf_output(&format!("(no recall matches for '{}')", pattern_str));
                                            }
                                        }

                                        // Determine target world
                                        let target_world = if let Some(ref world_name) = world {
                                            Some(world_name.clone())
                                        } else {
                                            Some(app.current_world().name.clone())
                                        };
                                        let target_idx = if let Some(ref world_name) = world {
                                            app.worlds.iter().position(|w| w.name == *world_name)
                                        } else {
                                            Some(app.current_world_index)
                                        };

                                        if delay_secs > 0.0 && lines.len() > 1 {
                                            // Schedule as processes with delays
                                            let delay = std::time::Duration::from_secs_f64(delay_secs);
                                            let now = std::time::Instant::now();
                                            for (i, line) in lines.into_iter().enumerate() {
                                                let cmd = match disposition {
                                                    tf::QuoteDisposition::Send => line,
                                                    tf::QuoteDisposition::Echo => format!("/echo {}", line),
                                                    tf::QuoteDisposition::Exec => line,
                                                };
                                                let id = app.tf_engine.next_process_id;
                                                app.tf_engine.next_process_id += 1;
                                                let process = tf::TfProcess {
                                                    id,
                                                    command: cmd,
                                                    interval: delay,
                                                    count: Some(1),
                                                    remaining: Some(1),
                                                    next_run: now + delay * i as u32,
                                                    world: target_world.clone(),
                                                    synchronous: false,
                                                    on_prompt: false,
                                                    priority: 0,
                                                };
                                                app.tf_engine.processes.push(process);
                                            }
                                        } else {
                                            // Send immediately (no delay or single line)
                                            for line in lines {
                                                match disposition {
                                                    tf::QuoteDisposition::Send => {
                                                        if let Some(idx) = target_idx {
                                                            if app.worlds[idx].connected {
                                                                if let Some(tx) = &app.worlds[idx].command_tx {
                                                                    let _ = tx.send(WriteCommand::Text(line)).await;
                                                                }
                                                            } else {
                                                                app.add_output("Not connected");
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    tf::QuoteDisposition::Echo => {
                                                        app.add_output(&line);
                                                    }
                                                    tf::QuoteDisposition::Exec => {
                                                        // Execute each line as a TF command
                                                        let result = app.tf_engine.execute(&line);
                                                        match result {
                                                            tf::TfCommandResult::SendToMud(text) => {
                                                                if let Some(idx) = target_idx {
                                                                    if let Some(tx) = &app.worlds[idx].command_tx {
                                                                        let _ = tx.send(WriteCommand::Text(text)).await;
                                                                    }
                                                                }
                                                            }
                                                            tf::TfCommandResult::Success(Some(msg)) => {
                                                                app.add_output(&msg);
                                                            }
                                                            tf::TfCommandResult::Error(err) => {
                                                                app.add_output(&format!("Error: {}", err));
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    tf::TfCommandResult::NotTfCommand => {
                                        // Shouldn't happen since we checked for /
                                        app.add_output("Internal error: not a command");
                                    }
                                    tf::TfCommandResult::UnknownCommand(cmd_name) => {
                                        // Show the command with its original prefix
                                        let prefix = if cmd_name.starts_with("//") { "" } else { "/" };
                                        app.add_output(&format!("Unknown command: {}{}", prefix, cmd_name));
                                    }
                                    tf::TfCommandResult::ExitLoad => {
                                        // ExitLoad from command line means nothing (not in a file load)
                                        // This shouldn't normally happen since cmd_exit checks loading_files
                                    }
                                    tf::TfCommandResult::Return(_) => {
                                        // Return outside of macro execution - no-op
                                    }
                                }
                                // Process any pending world operations from TF functions like addworld()
                                process_pending_world_ops(&mut app);
                                // Process any pending commands from TF macro execution
                                process_pending_tf_commands(&mut app);
                                // Process any pending keyboard operations from TF functions like kbgoto()
                                app.process_pending_keyboard_ops();
                            } else if app.current_world().connected {
                                if let Some(tx) = &app.current_world().command_tx {
                                    if tx.send(WriteCommand::Text(cmd)).await.is_err() {
                                        app.add_output("Failed to send command");
                                    } else {
                                        let now = std::time::Instant::now();
                                        app.current_world_mut().last_send_time = Some(now);
                                        app.current_world_mut().last_user_command_time = Some(now);
                                        app.current_world_mut().prompt.clear();
                                        // Reset more-mode counter after successfully sending command
                                        // This ensures the counter is 0 when the server response arrives
                                        app.current_world_mut().lines_since_pause = 0;
                                    }
                                }
                            } else {
                                app.add_output("Not connected. Use /worlds to connect.");
                            }
                        }
                        KeyAction::SwitchedWorld(_world_index) => {
                            // UnseenCleared is now broadcast by switch_world() itself
                        }
                        KeyAction::None => {}
                    }
                    if app.web_restart_needed {
                        app.web_restart_needed = false;
                        restart_http_server(&mut app, event_tx.clone()).await;
                    }
                    app.check_word_ended();
                    app.check_temp_conversion();

                }
                } // close if let Some(Ok(event))
            }

            // Server events (data from MUD connections)
            Some(event) = event_rx.recv() => {
                needs_draw = true;
                match event {
                    AppEvent::ServerData(ref world_name, bytes) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            // Use shared server data processing
                            let console_height = app.output_height;
                            let console_width = app.output_width;
                            let commands = app.process_server_data(
                                world_idx,
                                &bytes,
                                console_height,
                                console_width,
                                false, // not daemon mode
                            );

                            // Activate prompt check if server data set wont_echo_time
                            if app.worlds[world_idx].wont_echo_time.is_some() {
                                prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                            }
                            // Activate timer for FANSI login timeout
                            if app.worlds[world_idx].fansi_detect_until.is_some() {
                                prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                            }
                            // Activate pending update if lines were paused
                            if !app.worlds[world_idx].pending_lines.is_empty() {
                                pending_update_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(2));
                            }

                            // Check if terminal needs full redraw (after splash clear)
                            if app.worlds[world_idx].needs_redraw {
                                app.worlds[world_idx].needs_redraw = false;
                                terminal.clear()?;
                            }

                            // Execute any triggered commands
                            // Temporarily set current_world to the triggering world so /send
                            // without -w sends to the world that triggered the action
                            let saved_current_world = app.current_world_index;
                            app.current_world_index = world_idx;
                            for cmd in commands {
                                if cmd.starts_with('/') {
                                    // Unified command system - route through TF parser
                                    app.sync_tf_world_info();
                                    match app.tf_engine.execute(&cmd) {
                                        tf::TfCommandResult::SendToMud(text) => {
                                            if let Some(tx) = &app.worlds[world_idx].command_tx {
                                                let _ = tx.try_send(WriteCommand::Text(text));
                                            }
                                        }
                                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                            handle_command(&clay_cmd, &mut app, event_tx.clone()).await;
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
                            if conn_id != app.worlds[world_idx].connection_id { continue; }
                            app.handle_disconnected(world_idx);
                            if let Some(next) = app.next_reconnect_instant() {
                                let dur = next.saturating_duration_since(std::time::Instant::now());
                                reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                            }
                        }
                    }
                    AppEvent::TelnetDetected(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_telnet_detected(world_idx);
                        }
                    }
                    AppEvent::WontEchoSeen(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_wont_echo_seen(world_idx);
                        }
                    }
                    AppEvent::NawsRequested(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_naws_requested(world_idx);
                        }
                    }
                    AppEvent::TtypeRequested(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_ttype_requested(world_idx);
                        }
                    }
                    AppEvent::CharsetRequested(ref world_name, ref charsets) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_charset_requested(world_idx, charsets);
                        }
                    }
                    AppEvent::GmcpNegotiated(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_gmcp_negotiated(world_idx);
                        }
                    }
                    AppEvent::MsdpNegotiated(ref world_name) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.worlds[world_idx].msdp_enabled = true;
                        }
                    }
                    AppEvent::GmcpReceived(ref world_name, ref package, ref json_data) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_gmcp_received(world_idx, package, json_data);
                        }
                    }
                    AppEvent::MsdpReceived(ref world_name, ref variable, ref value_json) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_msdp_received(world_idx, variable, value_json);
                        }
                    }
                    AppEvent::Prompt(ref world_name, prompt_bytes) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.handle_prompt(world_idx, &prompt_bytes);
                        }
                    }
                    AppEvent::SystemMessage(message) => {
                        // Display system message in current world's output
                        app.add_output(&message);
                    }
                    AppEvent::Sigusr1Received => {
                        // SIGUSR1 received - trigger hot reload (only on non-Android)
                        debug_log(is_debug_enabled(), "LOOP: Received SIGUSR1 via event channel");
                        app.add_output("Received SIGUSR1, performing hot reload...");
                        if handle_command("/reload", &mut app, event_tx.clone()).await {
                            return Ok(());
                        }
                    }
                    // Multiuser events are only used in multiuser mode, ignore in normal mode
                    AppEvent::ConnectWorldRequest(_, _) => {}
                    AppEvent::MultiuserServerData(_, _, _) => {}
                    AppEvent::MultiuserDisconnected(_, _) => {}
                    AppEvent::MultiuserTelnetDetected(_, _) => {}
                    AppEvent::MultiuserPrompt(_, _, _) => {}
                    // Slack/Discord events
                    AppEvent::SlackMessage(ref world_name, message) | AppEvent::DiscordMessage(ref world_name, message) => {
                        if let Some(world_idx) = app.find_world_index(world_name) {
                            app.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());
                            let is_current = world_idx == app.current_world_index || app.ws_client_viewing(world_idx);
                            let world_name_for_triggers = world_name.clone();
                            let actions = app.settings.actions.clone();

                            // Check action and TF triggers on the message
                            let tr = process_triggers(&message, &world_name_for_triggers, &actions, &mut app.tf_engine);
                            let commands_to_execute = tr.send_commands;
                            let tf_commands_to_execute = tr.clay_commands;
                            for msg in &tr.messages {
                                app.add_tf_output(msg);
                            }

                            let data = format!("{}\n", message);

                            if tr.is_gagged {
                                // Add as gagged line (only visible with F2)
                                let seq = app.worlds[world_idx].next_seq;
                                app.worlds[world_idx].next_seq += 1;
                                app.worlds[world_idx].output_lines.push(OutputLine::new_gagged(message.clone(), seq));
                                if !app.worlds[world_idx].paused {
                                    app.worlds[world_idx].scroll_to_bottom();
                                }
                                // Broadcast gagged line to WebSocket clients
                                let is_current = world_idx == app.current_world_index;
                                let ws_data = message.replace('\r', "") + "\n";
                                app.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: ws_data,
                                    is_viewed: is_current,
                                    ts: current_timestamp_secs(),
                                    from_server: true,
                                    seq,
                                    marked_new: !is_current,
                                    flush: false, gagged: true,
                                });
                            } else {
                                // Add non-gagged output normally
                                let settings = app.settings.clone();
                                let console_height = app.output_height;
                                let console_width = app.output_width;

                                // Calculate minimum visible lines among all viewers for synchronized more-mode
                                let console_viewing = world_idx == app.current_world_index;
                                let ws_min = app.min_viewer_lines(world_idx);
                                let output_height = match (console_viewing, ws_min) {
                                    (true, Some(ws)) => console_height.min(ws as u16),
                                    (true, None) => console_height,
                                    (false, Some(ws)) => ws as u16,
                                    (false, None) => console_height,
                                };

                                // Calculate minimum visible columns among all viewers for wrap width
                                let ws_min_width = app.min_viewer_width(world_idx);
                                let output_width = match (console_viewing, ws_min_width) {
                                    (true, Some(ws_w)) => console_width.min(ws_w as u16),
                                    (true, None) => console_width,
                                    (false, Some(ws_w)) => ws_w as u16,
                                    (false, None) => console_width,
                                };

                                // Track pending count before add_output for synchronized more-mode
                                let pending_before = app.worlds[world_idx].pending_lines.len();
                                let output_before = app.worlds[world_idx].output_lines.len();

                                app.worlds[world_idx].add_output(&data, is_current, &settings, output_height, output_width, true, true);

                                // Calculate what went where
                                let pending_after = app.worlds[world_idx].pending_lines.len();
                                let output_after = app.worlds[world_idx].output_lines.len();
                                let lines_to_output = output_after.saturating_sub(output_before);
                                let lines_to_pending = pending_after.saturating_sub(pending_before);

                                // For synchronized more-mode: only broadcast lines that went to output_lines
                                if lines_to_output > 0 {
                                    let output_lines_to_broadcast: Vec<String> = app.worlds[world_idx]
                                        .output_lines
                                        .iter()
                                        .skip(output_before)
                                        .take(lines_to_output)
                                        .map(|line| line.text.replace('\r', ""))
                                        .collect();
                                    let ws_data = output_lines_to_broadcast.join("\n") + "\n";
                                    app.ws_broadcast(WsMessage::ServerData {
                                        world_index: world_idx,
                                        data: ws_data,
                                        is_viewed: is_current,
                                        ts: current_timestamp_secs(),
                                        from_server: false,
                                        seq: 0,
                                        marked_new: false,
                                        flush: false, gagged: false,
                                    });
                                }

                                // Broadcast pending count update if it changed
                                // Use filtered broadcast to skip clients that received pending in InitialState
                                if lines_to_pending > 0 || pending_after != pending_before {
                                    app.ws_broadcast(WsMessage::PendingLinesUpdate { world_index: world_idx, count: pending_after });
                                }

                                // Broadcast updated unseen count so all clients stay in sync
                                let unseen_count = app.worlds[world_idx].unseen_lines;
                                if unseen_count > 0 {
                                    app.ws_broadcast(WsMessage::UnseenUpdate {
                                        world_index: world_idx,
                                        count: unseen_count,
                                    });
                                }

                                // Broadcast activity count to keep all clients in sync
                                app.broadcast_activity();
                            }

                            // Mark output for redraw if this is the current world
                            if world_idx == app.current_world_index {
                                app.needs_output_redraw = true;
                            }

                            // Execute any triggered commands
                            // Temporarily set current_world to the triggering world so /send
                            // without -w sends to the world that triggered the action
                            let saved_current_world = app.current_world_index;
                            app.current_world_index = world_idx;
                            for cmd in commands_to_execute {
                                if cmd.starts_with('/') {
                                    // Unified command system - route through TF parser
                                    app.sync_tf_world_info();
                                    match app.tf_engine.execute(&cmd) {
                                        tf::TfCommandResult::SendToMud(text) => {
                                            if let Some(tx) = &app.worlds[world_idx].command_tx {
                                                let _ = tx.try_send(WriteCommand::Text(text));
                                            }
                                        }
                                        tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                            handle_command(&clay_cmd, &mut app, event_tx.clone()).await;
                                        }
                                        tf::TfCommandResult::RepeatProcess(process) => {
                                            app.tf_engine.processes.push(process);
                                        }
                                        _ => {}
                                    }
                                } else if let Some(tx) = &app.worlds[world_idx].command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(cmd));
                                }
                            }
                            app.current_world_index = saved_current_world;
                            // Execute TF-generated Clay commands
                            for cmd in tf_commands_to_execute {
                                let _ = app.tf_engine.execute(&cmd);
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
                    AppEvent::WsClientMessage(client_id, msg) => {
                        let op = app.handle_ws_client_msg(client_id, *msg, &event_tx);
                        if app.web_reconnect_needed {
                            app.web_reconnect_needed = false;
                            if app.trigger_web_reconnects() {
                                if let Some(next) = app.next_reconnect_instant() {
                                    let dur = next.saturating_duration_since(std::time::Instant::now());
                                    reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                                }
                            }
                        }
                        if app.web_restart_needed {
                            app.web_restart_needed = false;
                            restart_http_server(&mut app, event_tx.clone()).await;
                        }
                        match op {
                            WsAsyncAction::Connect { world_index, prev_index, broadcast } => {
                                if handle_command("/__connect", &mut app, event_tx.clone()).await {
                                    return Ok(());
                                }
                                if broadcast && world_index < app.worlds.len() && app.worlds[world_index].connected {
                                    let name = app.worlds[world_index].name.clone();
                                    app.ws_broadcast(WsMessage::WorldConnected { world_index, name });
                                }
                                if prev_index != world_index {
                                    app.current_world_index = prev_index;
                                }
                            }
                            WsAsyncAction::Disconnect { world_index, prev_index } => {
                                let _ = world_index;
                                if handle_command("/disconnect", &mut app, event_tx.clone()).await {
                                    return Ok(());
                                }
                                app.current_world_index = prev_index;
                            }
                            WsAsyncAction::Reload => {
                                #[cfg(not(target_os = "android"))]
                                {
                                    debug_log(is_debug_enabled(), "HEADLESS: WS client requested reload");
                                    app.ws_broadcast(WsMessage::ServerReloading);
                                    exec_reload(&mut app)?;
                                    return Ok(());
                                }
                            }
                            WsAsyncAction::Done => {}
                        }
                    }
                    AppEvent::ConnectionSuccess(world_name, cmd_tx, socket_fd, is_tls) => {
                        app.handle_connection_success(&world_name, cmd_tx, socket_fd, is_tls);
                    }
                    AppEvent::ConnectionFailed(world_name, error) => {
                        if let Some(world_idx) = app.find_world_index(&world_name) {
                            app.add_output_to_world(world_idx, &format!("Connection failed: {}", error));
                        }
                    }
                    AppEvent::MediaFileReady(world_idx, key, path, volume, loops, is_music) => {
                        app.ensure_audio();
                        if let Some(handle) = audio::play_file(&app.audio_backend, &path, volume, loops) {
                            if is_music {
                                app.media_music_key = Some((world_idx, key.clone()));
                            }
                            app.media_processes.insert(key, (world_idx, handle));
                        }
                    }
                    AppEvent::ApiLookupResult(client_id, world_index, result, cursor_start) => {
                        match result {
                            Ok(text) => app.ws_send_to_client(client_id, WsMessage::SetInputBuffer { text, cursor_start }),
                            Err(e) => app.ws_send_to_client(client_id, WsMessage::ServerData {
                                world_index,
                                data: e,
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0,
                                marked_new: false,
                                flush: false, gagged: false,
                            }),
                        }
                    }
                    AppEvent::RemoteListResult(requesting_client_id, world_index, lines) => {
                        app.remote_ping_responses = None;
                        if requesting_client_id == 0 {
                            for line in &lines {
                                app.add_output(line);
                            }
                        } else {
                            for line in &lines {
                                app.ws_send_to_client(requesting_client_id, WsMessage::ServerData {
                                    world_index,
                                    data: line.clone(),
                                    is_viewed: false,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0,
                                    marked_new: false,
                                    flush: false, gagged: false,
                                });
                            }
                        }
                    }
                    AppEvent::UpdateResult(result) => {
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
                                                        app.add_output(&format!("Updated to Clay v{} — reloading...", success.version));
                                                        app.ws_broadcast(WsMessage::ServerReloading);
                                                        let _ = crossterm::terminal::disable_raw_mode();
                                                        let _ = crossterm::execute!(
                                                            std::io::stdout(),
                                                            crossterm::terminal::LeaveAlternateScreen
                                                        );
                                                        exec_reload(&mut app)?;
                                                        return Ok(());
                                                    }
                                                    Err(e2) => {
                                                        app.add_output(&format!("Failed to install update: {} (rename: {})", e2, e));
                                                        let _ = std::fs::remove_file(&success.temp_path);
                                                    }
                                                }
                                            } else {
                                                app.add_output(&format!("Updated to Clay v{} — reloading...", success.version));
                                                app.ws_broadcast(WsMessage::ServerReloading);
                                                let _ = crossterm::terminal::disable_raw_mode();
                                                let _ = crossterm::execute!(
                                                    std::io::stdout(),
                                                    crossterm::terminal::LeaveAlternateScreen
                                                );
                                                exec_reload(&mut app)?;
                                                return Ok(());
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
                    }
                }
            }

            // Periodic timer for clock updates and keepalive checks (once per minute)
            _ = keepalive_interval.tick() => {
                needs_draw = true; // Clock display updates every minute

                // Clear popup error messages after timeout
                if let Some(state) = app.popup_manager.current_mut() {
                    if let Some(error_time) = state.error_at {
                        if error_time.elapsed() >= std::time::Duration::from_secs(10) {
                            state.error = None;
                            state.error_at = None;
                        }
                    }
                }

                // Check keepalive for all connected worlds (send NOP if no activity in 5 min)
                for world in &mut app.worlds {
                    if world.connected {
                        // Check last activity time (either send or receive)
                        let last_activity = match (world.last_send_time, world.last_receive_time) {
                            (Some(s), Some(r)) => Some(s.max(r)),
                            (Some(s), None) => Some(s),
                            (None, Some(r)) => Some(r),
                            (None, None) => None,
                        };
                        let should_send = match last_activity {
                            Some(t) => t.elapsed() >= KEEPALIVE_INTERVAL,
                            None => true,
                        };
                        if should_send {
                            if let Some(tx) = &world.command_tx {
                                let now = std::time::Instant::now();

                                // Send keepalive based on type
                                match world.settings.keep_alive_type {
                                    KeepAliveType::None => {
                                        // Don't update times - nothing was sent
                                    }
                                    KeepAliveType::Nop => {
                                        let nop = vec![TELNET_IAC, TELNET_NOP];
                                        let _ = tx.try_send(WriteCommand::Raw(nop));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Custom => {
                                        // Generate random number 1-1000
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let idler_tag = format!("###_idler_message_{}_###", rand_num);
                                        let cmd = world.settings.keep_alive_cmd
                                            .replace("##rand##", &idler_tag);
                                        let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                    KeepAliveType::Generic => {
                                        // Generate random number 1-1000
                                        let rand_num = (std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_nanos() % 1000 + 1) as u32;
                                        let cmd = format!("help commands ###_idler_message_{}_###", rand_num);
                                        let _ = tx.try_send(WriteCommand::Text(cmd.clone()));
                                        world.last_send_time = Some(now);
                                        world.last_nop_time = Some(now);
                                    }
                                }
                            }
                        }
                    }
                }

                // Check proxy health for TLS proxy connections
                for world in &mut app.worlds {
                    if world.connected {
                        if let Some(proxy_pid) = world.proxy_pid {
                            if !is_process_alive(proxy_pid) {
                                // Proxy died - mark world as disconnected
                                world.clear_connection_state(false, false);
                                let seq = world.next_seq;
                                world.next_seq += 1;
                                world.output_lines.push(OutputLine::new("TLS proxy terminated. Connection lost.".to_string(), seq));
                            }
                        }
                    }
                }

                // Redraw to update the clock display in separator bar
            }

            // Prompt timeout check — only fires when a world has wont_echo_time set
            _ = &mut prompt_check_sleep => {
                let now = std::time::Instant::now();
                for world in &mut app.worlds {
                    // Check if there's a partial line waiting to become a prompt
                    if let Some(wont_echo_time) = world.wont_echo_time {
                        // If 150ms+ has passed since partial line was seen
                        if now.duration_since(wont_echo_time) >= Duration::from_millis(150) {
                            // Check if there's a partial line to extract as prompt
                            if !world.trigger_partial_line.is_empty() && world.prompt.is_empty() {
                                needs_draw = true; // Prompt extracted — redraw input area
                                // Extract partial line as prompt
                                let prompt_text = std::mem::take(&mut world.trigger_partial_line);
                                let normalized = crate::util::normalize_prompt(&prompt_text);

                                // If world is not connected, display prompt as output instead
                                if !world.connected {
                                    let seq = world.next_seq;
                                    world.next_seq += 1;
                                    world.output_lines.push(OutputLine::new(normalized.trim().to_string(), seq));
                                    world.wont_echo_time = None;
                                    continue;
                                }

                                world.prompt = normalized;
                                world.prompt_count += 1;

                                // Handle auto-login (same logic as AppEvent::Prompt handler)
                                if !world.skip_auto_login {
                                    let auto_type = world.settings.auto_connect_type;
                                    let user = world.settings.user.clone();
                                    let password = world.settings.password.clone();
                                    let prompt_num = world.prompt_count;

                                    if !user.is_empty() && !password.is_empty() {
                                        let cmd_to_send = match auto_type {
                                            AutoConnectType::Prompt => {
                                                match prompt_num {
                                                    1 if !user.is_empty() => Some(user),
                                                    2 if !password.is_empty() => Some(password),
                                                    _ => None,
                                                }
                                            }
                                            AutoConnectType::MooPrompt => {
                                                match prompt_num {
                                                    1 if !user.is_empty() => Some(user.clone()),
                                                    2 if !password.is_empty() => Some(password),
                                                    3 if !user.is_empty() => Some(user),
                                                    _ => None,
                                                }
                                            }
                                            AutoConnectType::Connect | AutoConnectType::NoLogin => None,
                                        };

                                        if let Some(cmd) = cmd_to_send {
                                            world.prompt.clear(); // Clear prompt since we're auto-responding
                                            if let Some(tx) = &world.command_tx {
                                                let _ = tx.try_send(WriteCommand::Text(cmd));
                                            }
                                        }
                                    }
                                }
                            }
                            world.wont_echo_time = None;
                        }
                    }
                }
                // Check for expired FANSI detection windows
                let now_fansi = std::time::Instant::now();
                for world in &mut app.worlds {
                    if let Some(deadline) = world.fansi_detect_until {
                        if now_fansi >= deadline {
                            if let Some(login_cmd) = world.fansi_login_pending.take() {
                                if let Some(tx) = &world.command_tx {
                                    let _ = tx.try_send(WriteCommand::Text(login_cmd));
                                }
                            }
                            world.fansi_detect_until = None;
                        }
                    }
                }
                // Re-arm: check again in 150ms if any world still needs it
                let any_pending_prompt = app.worlds.iter().any(|w| w.wont_echo_time.is_some() || w.fansi_detect_until.is_some());
                if any_pending_prompt {
                    prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                } else {
                    prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }

            // TF repeat process tick — only fires when processes exist
            _ = &mut process_tick_sleep => {
                needs_draw = true; // Process may produce output
                let now = std::time::Instant::now();
                let mut to_remove = vec![];
                let process_count = app.tf_engine.processes.len();
                for i in 0..process_count {
                    if app.tf_engine.processes[i].on_prompt { continue; }
                    if app.tf_engine.processes[i].next_run <= now {
                        let cmd = app.tf_engine.processes[i].command.clone();
                        let process_world = app.tf_engine.processes[i].world.clone();
                        // Sync world info before executing process command
                        app.sync_tf_world_info();
                        let result = app.tf_engine.execute(&cmd);
                        // Determine target world for this process
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
                                let seq = app.worlds[world_idx].next_seq;
                                app.worlds[world_idx].next_seq += 1;
                                app.worlds[world_idx].output_lines.push(OutputLine::new_client(msg.clone(), seq));
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: msg,
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0,
                                    marked_new: false,
                                    flush: false, gagged: false,
                                });
                            }
                            tf::TfCommandResult::Error(err) => {
                                let err_msg = format!("Error: {}", err);
                                let seq = app.worlds[world_idx].next_seq;
                                app.worlds[world_idx].next_seq += 1;
                                app.worlds[world_idx].output_lines.push(OutputLine::new_client(err_msg.clone(), seq));
                                app.ws_broadcast(WsMessage::ServerData {
                                    world_index: world_idx,
                                    data: err_msg,
                                    is_viewed: true,
                                    ts: current_timestamp_secs(),
                                    from_server: false,
                                    seq: 0,
                                    marked_new: false,
                                    flush: false, gagged: false,
                                });
                            }
                            tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                if handle_command(&clay_cmd, &mut app, event_tx.clone()).await {
                                    return Ok(());
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
                // Re-arm: tick again in 1s if processes remain
                if !app.tf_engine.processes.is_empty() {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(1));
                } else {
                    process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }

            // Pending count update — only fires when worlds have pending lines
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
                let any_pending_lines = app.worlds.iter().any(|w| !w.pending_lines.is_empty());
                if any_pending_lines {
                    pending_update_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(2));
                } else {
                    pending_update_sleep.as_mut().reset(tokio::time::Instant::now() + FAR_FUTURE);
                }
            }

            // Auto-reconnect timer
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
                            app.add_output_to_world(idx, &format!("Connecting to {}:{}{}...", settings.hostname, settings.port, ssl_msg));
                            // Pass skip_auto_login=true to connect_daemon_world so it doesn't
                            // send auto-login; handle_connection_success handles that instead.
                            match daemon::connect_daemon_world(
                                idx, world_name.clone(), &settings, event_tx.clone(), connection_id, true
                            ).await {
                                Some((cmd_tx, socket_fd, is_tls)) => {
                                    app.handle_connection_success(&world_name, cmd_tx, socket_fd, is_tls);
                                    if let Some(new_idx) = app.find_world_index(&world_name) {
                                        app.add_output_to_world(new_idx, "Connected!");
                                    }
                                }
                                None => {
                                    if let Some(current_idx) = app.find_world_index(&world_name) {
                                        let secs = app.worlds[current_idx].settings.auto_reconnect_secs;
                                        if secs > 0 {
                                            app.worlds[current_idx].reconnect_at = Some(
                                                std::time::Instant::now() + std::time::Duration::from_secs(secs as u64)
                                            );
                                            app.add_output_to_world(current_idx, &format!("Connection failed. Reconnecting in {} seconds...", secs));
                                        } else {
                                            app.add_output_to_world(current_idx, "Connection failed.");
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
        }

        // Process additional queued events with a time budget for UI responsiveness.
        // When a burst of data arrives (e.g., 11K lines), this processes events for up to
        // 16ms, then breaks to draw and handle keyboard input. Remaining events stay in
        // the channel and are picked up next iteration (event_rx.recv() returns immediately).
        let drain_deadline = std::time::Instant::now() + Duration::from_millis(16);
        while let Ok(event) = event_rx.try_recv() {
            needs_draw = true;
            match event {
                AppEvent::ServerData(ref world_name, bytes) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        // Use shared server data processing
                        let console_height = app.output_height;
                        let console_width = app.output_width;
                        let commands = app.process_server_data(
                            world_idx,
                            &bytes,
                            console_height,
                            console_width,
                            false, // not daemon mode
                        );

                        // Activate prompt check if server data set wont_echo_time
                        if app.worlds[world_idx].wont_echo_time.is_some() {
                            prompt_check_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(150));
                        }
                        // Activate pending update if lines were paused
                        if !app.worlds[world_idx].pending_lines.is_empty() {
                            pending_update_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(2));
                        }

                        // Check if terminal needs full redraw (after splash clear)
                        if app.worlds[world_idx].needs_redraw {
                            app.worlds[world_idx].needs_redraw = false;
                            let _ = terminal.clear();
                        }

                        // Execute any triggered commands
                        let saved_current_world = app.current_world_index;
                        app.current_world_index = world_idx;
                        for cmd in commands {
                            if cmd.starts_with('/') {
                                app.sync_tf_world_info();
                                match app.tf_engine.execute(&cmd) {
                                    tf::TfCommandResult::SendToMud(text) => {
                                        if let Some(tx) = &app.worlds[world_idx].command_tx {
                                            let _ = tx.try_send(WriteCommand::Text(text));
                                        }
                                    }
                                    tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                        handle_command(&clay_cmd, &mut app, event_tx.clone()).await;
                                    }
                                    tf::TfCommandResult::RepeatProcess(process) => {
                                        app.tf_engine.processes.push(process);
                                    }
                                    _ => {}
                                }
                            } else if let Some(tx) = &app.worlds[world_idx].command_tx {
                                let _ = tx.try_send(WriteCommand::Text(cmd));
                            }
                        }
                        app.current_world_index = saved_current_world;
                    }
                }
                AppEvent::Disconnected(ref world_name, conn_id) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        if conn_id != app.worlds[world_idx].connection_id { continue; }
                        app.handle_disconnected(world_idx);
                        if let Some(next) = app.next_reconnect_instant() {
                            let dur = next.saturating_duration_since(std::time::Instant::now());
                            if reconnect_sleep.deadline() > tokio::time::Instant::now() + dur {
                                reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                            }
                        }
                    }
                }
                AppEvent::TelnetDetected(ref world_name) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_telnet_detected(world_idx);
                    }
                }
                AppEvent::WontEchoSeen(ref world_name) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_wont_echo_seen(world_idx);
                    }
                }
                AppEvent::NawsRequested(ref world_name) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_naws_requested(world_idx);
                    }
                }
                AppEvent::TtypeRequested(ref world_name) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_ttype_requested(world_idx);
                    }
                }
                AppEvent::CharsetRequested(ref world_name, ref charsets) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_charset_requested(world_idx, charsets);
                    }
                }
                AppEvent::Prompt(ref world_name, prompt_bytes) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_prompt(world_idx, &prompt_bytes);
                    }
                }
                AppEvent::SystemMessage(message) => {
                    app.add_output(&message);
                }
                // Multiuser events are only used in multiuser mode, ignore in normal mode
                AppEvent::ConnectWorldRequest(_, _) => {}
                AppEvent::MultiuserServerData(_, _, _) => {}
                AppEvent::MultiuserDisconnected(_, _) => {}
                AppEvent::MultiuserTelnetDetected(_, _) => {}
                AppEvent::MultiuserPrompt(_, _, _) => {}
                AppEvent::Sigusr1Received => {}
                AppEvent::UpdateResult(result) => {
                    match result {
                        Ok(_success) => {
                            app.add_output("Update downloaded but reload is not supported in daemon mode.");
                            let _ = std::fs::remove_file(&_success.temp_path);
                        }
                        Err(e) => { app.add_output(&e); }
                    }
                }
                AppEvent::GmcpNegotiated(ref world_name) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_gmcp_negotiated(world_idx);
                    }
                }
                AppEvent::MsdpNegotiated(ref world_name) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.worlds[world_idx].msdp_enabled = true;
                    }
                }
                AppEvent::GmcpReceived(ref world_name, ref package, ref json_data) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_gmcp_received(world_idx, package, json_data);
                    }
                }
                AppEvent::MsdpReceived(ref world_name, ref variable, ref value_json) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.handle_msdp_received(world_idx, variable, value_json);
                    }
                }
                // Slack/Discord events
                AppEvent::SlackMessage(ref world_name, message) | AppEvent::DiscordMessage(ref world_name, message) => {
                    if let Some(world_idx) = app.find_world_index(world_name) {
                        app.worlds[world_idx].last_receive_time = Some(std::time::Instant::now());
                        let is_current = world_idx == app.current_world_index || app.ws_client_viewing(world_idx);
                        let world_name_for_triggers = world_name.clone();
                        let actions = app.settings.actions.clone();

                        // Check action and TF triggers on the message
                        let tr = process_triggers(&message, &world_name_for_triggers, &actions, &mut app.tf_engine);
                        let commands_to_execute = tr.send_commands;
                        let tf_commands_to_execute = tr.clay_commands;
                        for msg in &tr.messages {
                            app.add_tf_output(msg);
                        }

                        let data = format!("{}\n", message);

                        if tr.is_gagged {
                            // Add as gagged line (only visible with F2)
                            let seq = app.worlds[world_idx].next_seq;
                            app.worlds[world_idx].next_seq += 1;
                            app.worlds[world_idx].output_lines.push(OutputLine::new_gagged(message.clone(), seq));
                            if !app.worlds[world_idx].paused {
                                app.worlds[world_idx].scroll_to_bottom();
                            }
                            // Broadcast gagged line to WebSocket clients
                            let is_current = world_idx == app.current_world_index;
                            let ws_data = message.replace('\r', "") + "\n";
                            app.ws_broadcast_to_world(world_idx, WsMessage::ServerData {
                                world_index: world_idx,
                                data: ws_data,
                                is_viewed: is_current,
                                ts: current_timestamp_secs(),
                                from_server: true,
                                seq,
                                marked_new: !is_current,
                                flush: false, gagged: true,
                            });
                        } else {
                            // Add non-gagged output normally
                            let settings = app.settings.clone();
                            let output_height = app.output_height;
                            let console_width = app.output_width;
                            let ws_min_width = app.min_viewer_width(world_idx);
                            let console_viewing = world_idx == app.current_world_index;
                            let output_width = match (console_viewing, ws_min_width) {
                                (true, Some(ws_w)) => console_width.min(ws_w as u16),
                                (true, None) => console_width,
                                (false, Some(ws_w)) => ws_w as u16,
                                (false, None) => console_width,
                            };
                            app.worlds[world_idx].add_output(&data, is_current, &settings, output_height, output_width, true, true);
                            // Broadcast to WebSocket clients (only non-gagged)
                            app.ws_broadcast(WsMessage::ServerData {
                                world_index: world_idx,
                                data,
                                is_viewed: is_current,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0,
                                marked_new: false,
                                flush: false, gagged: false,
                            });
                        }

                        // Mark output for redraw if this is the current world
                        if world_idx == app.current_world_index {
                            app.needs_output_redraw = true;
                        }

                        // Execute any triggered commands
                        // Temporarily set current_world to the triggering world so /send
                        // without -w sends to the world that triggered the action
                        let saved_current_world = app.current_world_index;
                        app.current_world_index = world_idx;
                        for cmd in commands_to_execute {
                            if cmd.starts_with('/') {
                                // Unified command system - route through TF parser
                                app.sync_tf_world_info();
                                match app.tf_engine.execute(&cmd) {
                                    tf::TfCommandResult::SendToMud(text) => {
                                        if let Some(tx) = &app.worlds[world_idx].command_tx {
                                            let _ = tx.try_send(WriteCommand::Text(text));
                                        }
                                    }
                                    tf::TfCommandResult::ClayCommand(clay_cmd) => {
                                        handle_command(&clay_cmd, &mut app, event_tx.clone()).await;
                                    }
                                    tf::TfCommandResult::RepeatProcess(process) => {
                                        app.tf_engine.processes.push(process);
                                    }
                                    _ => {}
                                }
                            } else if let Some(tx) = &app.worlds[world_idx].command_tx {
                                let _ = tx.try_send(WriteCommand::Text(cmd));
                            }
                        }
                        app.current_world_index = saved_current_world;
                        // Execute TF-generated Clay commands
                        for cmd in tf_commands_to_execute {
                            let _ = app.tf_engine.execute(&cmd);
                        }
                    }
                }
                AppEvent::WsClientConnected(_) => {}
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
                AppEvent::WsClientMessage(client_id, msg) => {
                    let op = app.handle_ws_client_msg(client_id, *msg, &event_tx);
                    if app.web_reconnect_needed {
                        app.web_reconnect_needed = false;
                        if app.trigger_web_reconnects() {
                            if let Some(next) = app.next_reconnect_instant() {
                                let dur = next.saturating_duration_since(std::time::Instant::now());
                                reconnect_sleep.as_mut().reset(tokio::time::Instant::now() + dur);
                            }
                        }
                    }
                    if app.web_restart_needed {
                        app.web_restart_needed = false;
                        restart_http_server(&mut app, event_tx.clone()).await;
                    }
                    match op {
                        WsAsyncAction::Connect { world_index, prev_index, broadcast } => {
                            if handle_command("/__connect", &mut app, event_tx.clone()).await {
                                return Ok(());
                            }
                            if broadcast && world_index < app.worlds.len() && app.worlds[world_index].connected {
                                let name = app.worlds[world_index].name.clone();
                                app.ws_broadcast(WsMessage::WorldConnected { world_index, name });
                            }
                            if prev_index != world_index {
                                app.current_world_index = prev_index;
                            }
                        }
                        WsAsyncAction::Disconnect { world_index, prev_index } => {
                            let _ = world_index;
                            if handle_command("/disconnect", &mut app, event_tx.clone()).await {
                                return Ok(());
                            }
                            app.current_world_index = prev_index;
                        }
                        WsAsyncAction::Reload => {
                            debug_log(is_debug_enabled(), "CONSOLE: WS client requested reload");
                            app.add_output("Received reload request, performing hot reload...");
                            if handle_command("/reload", &mut app, event_tx.clone()).await {
                                return Ok(());
                            }
                        }
                        WsAsyncAction::Done => {}
                    }
                }
                AppEvent::ConnectionSuccess(world_name, cmd_tx, socket_fd, is_tls) => {
                    app.handle_connection_success(&world_name, cmd_tx, socket_fd, is_tls);
                    // Batch drain shows "Connected!" message (primary select loop doesn't)
                    if let Some(world_idx) = app.find_world_index(&world_name) {
                        app.add_output_to_world(world_idx, "Connected!");
                    }
                }
                // Background connection failed
                AppEvent::ConnectionFailed(world_name, error) => {
                    if let Some(world_idx) = app.find_world_index(&world_name) {
                        app.add_output_to_world(world_idx, &format!("Connection failed: {}", error));
                    }
                }
                AppEvent::MediaFileReady(world_idx, key, path, volume, loops, is_music) => {
                    app.ensure_audio();
                    if let Some(handle) = audio::play_file(&app.audio_backend, &path, volume, loops) {
                        if is_music {
                            app.media_music_key = Some((world_idx, key.clone()));
                        }
                        app.media_processes.insert(key, (world_idx, handle));
                    }
                }
                AppEvent::ApiLookupResult(client_id, world_index, result, cursor_start) => {
                    match result {
                        Ok(text) => app.ws_send_to_client(client_id, WsMessage::SetInputBuffer { text, cursor_start }),
                        Err(e) => app.ws_send_to_client(client_id, WsMessage::ServerData {
                            world_index,
                            data: e,
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0,
                            marked_new: false,
                            flush: false, gagged: false,
                        }),
                    }
                }
                AppEvent::RemoteListResult(requesting_client_id, world_index, lines) => {
                    app.remote_ping_responses = None;
                    for line in &lines {
                        app.ws_send_to_client(requesting_client_id, WsMessage::ServerData {
                            world_index,
                            data: line.clone(),
                            is_viewed: false,
                            ts: current_timestamp_secs(),
                            from_server: false,
                            seq: 0,
                            marked_new: false,
                            flush: false, gagged: false,
                        });
                    }
                }
            }
            // Time budget exceeded — break to draw and handle keyboard input.
            // Remaining events stay in the channel for next iteration.
            if std::time::Instant::now() >= drain_deadline {
                break;
            }
        }

        // Now draw with the most up-to-date state
        // Activate process tick sleep if processes were added during this iteration
        if !app.tf_engine.processes.is_empty() {
            // Only reset if the sleep is far-future (avoid resetting an already-short timer)
            if process_tick_sleep.deadline() > tokio::time::Instant::now() + Duration::from_secs(2) {
                process_tick_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(1));
            }
        }

        // Skip drawing when no visible state changed (e.g., pending update WS broadcast)
        if needs_draw {
            // Check if any popup is now visible
            let any_popup_visible = app.confirm_dialog.visible
                || app.filter_popup.visible
                || app.has_new_popup();

            // When transitioning to popup: terminal.clear() resets ratatui's buffers so
            // the diff will write ALL popup cells (full coverage, no bleed-through).
            // Ratatui handles both output and popup rendering when popup is visible.
            if any_popup_visible && !app.popup_was_visible {
                terminal.clear()?;
            }

            // Detect popup visibility change before updating
            let popup_visibility_changed = any_popup_visible != app.popup_was_visible;
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

            // Handle terminal clear request (e.g., after closing editor)
            if app.needs_terminal_clear {
                execute!(
                    std::io::stdout(),
                    crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
                )?;
                terminal.clear()?;
                app.needs_terminal_clear = false;
            }

            // Use ratatui for everything, but render output area with raw crossterm
            // after the ratatui draw (ratatui's Paragraph has rendering bugs)
            terminal.draw(|f| ui(f, &mut app))?;

            // Render output area with crossterm only when needed (optimization)
            // Also redraw when popup visibility changes (including popup open/close)
            // When popup is visible, render_output_crossterm is a no-op (ratatui handles it)
            if app.needs_output_redraw || popup_visibility_changed {
                render_output_crossterm(&app);
                app.needs_output_redraw = false;
                // Mark current world as seen since its output was just displayed
                let has_unseen = app.current_world().unseen_lines > 0;
                let has_pending = !app.current_world().pending_lines.is_empty();
                if has_unseen {
                    app.current_world_mut().mark_seen();
                    // Broadcast to WebSocket clients
                    app.ws_broadcast(WsMessage::UnseenCleared { world_index: app.current_world_index });
                    // Broadcast activity count since a world was just marked as seen
                    app.broadcast_activity();
                }
                // If more mode is disabled but world has orphaned pending_lines, release them
                if has_pending && !app.settings.more_mode_enabled {
                    let world = app.current_world_mut();
                    world.output_lines.append(&mut world.pending_lines);
                    world.pending_since = None;
                    world.paused = false;
                    world.scroll_to_bottom();
                }
            }
        }
    }
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
