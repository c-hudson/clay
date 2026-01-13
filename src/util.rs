use ansi_to_tui::IntoText;
use unicode_width::UnicodeWidthStr;

/// Get the binary name of the current executable
pub fn get_binary_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "client".to_string())
}

/// Strip ANSI escape codes from a string
pub fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c == '[' {
                // CSI sequence - skip until terminator
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_alphabetic() || next == '~' {
                        break;
                    }
                }
            }
            in_escape = false;
            continue;
        }
        result.push(c);
    }
    result
}

/// Calculate the number of visual lines a string takes when wrapped to width
pub fn visual_line_count(line: &str, width: usize) -> usize {
    if width == 0 {
        return 1;
    }
    match line.as_bytes().into_text() {
        Ok(text) => {
            let mut total = 0;
            for l in text.lines {
                let line_width: usize = l.spans.iter().map(|s| s.content.width()).sum();
                if line_width == 0 {
                    total += 1;
                } else {
                    total += line_width.div_ceil(width);
                }
            }
            total.max(1)
        }
        Err(_) => {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width)
            }
        }
    }
}

/// Get the current time in 12-hour format (H:MM)
pub fn get_current_time_12hr() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as libc::time_t;

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&now, &mut tm);
    }

    let hours_24 = tm.tm_hour as u32;
    let minutes = tm.tm_min as u32;

    let hours_12 = if hours_24 == 0 {
        12
    } else if hours_24 <= 12 {
        hours_24
    } else {
        hours_24 - 12
    };

    format!("{}:{:02}", hours_12, minutes)
}

/// Strip MUD tags from a line (tags like [channel:] or [channel(player)])
pub fn strip_mud_tag(text: &str) -> String {
    // First, find any leading whitespace
    let trimmed = text.trim_start();
    let leading_ws_len = text.len() - trimmed.len();
    let leading_ws = &text[..leading_ws_len];

    // Check if line starts with [ (possibly after ANSI codes)
    let mut chars = trimmed.chars().peekable();
    let mut ansi_prefix = String::new();
    let mut in_ansi = false;

    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            ansi_prefix.push(c);
            in_ansi = true;
        } else if in_ansi {
            ansi_prefix.push(c);
            if c.is_ascii_alphabetic() {
                in_ansi = false;
            }
        } else if c == '[' {
            // Found the start of a potential tag
            let rest: String = chars.collect();
            if let Some(end) = rest.find(']') {
                let tag = &rest[..end];
                if tag.contains(':') || tag.contains('(') {
                    // It's a MUD tag, skip it
                    let after_tag = &rest[end + 1..];
                    let after_tag = after_tag.strip_prefix(' ').unwrap_or(after_tag);
                    return format!("{}{}{}", leading_ws, ansi_prefix, after_tag);
                }
            }
            return text.to_string();
        } else {
            // Not a tag start, return original
            return text.to_string();
        }
    }
    text.to_string()
}

/// Truncate a string to max_len, adding "..." if truncated
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}

// ============================================================================
// Shared World Switching Logic
// ============================================================================

use crate::encoding::WorldSwitchMode;

/// Information about a world needed for switching calculations
pub struct WorldSwitchInfo {
    pub name: String,
    pub connected: bool,
    pub unseen_lines: usize,
}

/// Determine if a world should be included in the cycle list
/// (connected OR has unseen output)
pub fn world_should_cycle(info: &WorldSwitchInfo) -> bool {
    info.connected || info.unseen_lines > 0
}

/// Check if a world has pending/unseen output
pub fn world_has_pending(info: &WorldSwitchInfo) -> bool {
    info.unseen_lines > 0
}

/// Get the next world index to switch to
/// Returns None if no world to switch to, or Some(index) of the next world
pub fn calculate_next_world(
    worlds: &[WorldSwitchInfo],
    current_index: usize,
    world_switch_mode: WorldSwitchMode,
) -> Option<usize> {
    // Get list of worlds that should be included in cycling
    let cycleable: Vec<usize> = worlds.iter()
        .enumerate()
        .filter(|(_, w)| world_should_cycle(w))
        .map(|(i, _)| i)
        .collect();

    if cycleable.is_empty() {
        return None;
    }

    // If world switching is set to "Unseen First", check for OTHER worlds with unseen output first
    if world_switch_mode == WorldSwitchMode::UnseenFirst {
        let mut unseen_worlds: Vec<usize> = cycleable.iter()
            .filter(|&&i| i != current_index && world_has_pending(&worlds[i]))
            .copied()
            .collect();

        if !unseen_worlds.is_empty() {
            // Sort alphabetically and go to first
            unseen_worlds.sort_by(|&a, &b| {
                worlds[a].name.to_lowercase().cmp(&worlds[b].name.to_lowercase())
            });
            return Some(unseen_worlds[0]);
        }
    }

    // Fall back to alphabetical cycling
    let mut sorted = cycleable.clone();
    sorted.sort_by(|&a, &b| {
        worlds[a].name.to_lowercase().cmp(&worlds[b].name.to_lowercase())
    });

    // Find current position in the sorted cycleable list
    let current_pos = sorted.iter().position(|&i| i == current_index);

    match current_pos {
        Some(pos) => {
            let next_pos = (pos + 1) % sorted.len();
            if sorted[next_pos] != current_index {
                Some(sorted[next_pos])
            } else {
                None
            }
        }
        None => {
            // Current world isn't in cycleable list, go to first cycleable world
            Some(sorted[0])
        }
    }
}

/// Get the previous world index to switch to
/// Returns None if no world to switch to, or Some(index) of the previous world
pub fn calculate_prev_world(
    worlds: &[WorldSwitchInfo],
    current_index: usize,
    world_switch_mode: WorldSwitchMode,
) -> Option<usize> {
    // Get list of worlds that should be included in cycling
    let cycleable: Vec<usize> = worlds.iter()
        .enumerate()
        .filter(|(_, w)| world_should_cycle(w))
        .map(|(i, _)| i)
        .collect();

    if cycleable.is_empty() {
        return None;
    }

    // If world switching is set to "Unseen First", check for OTHER worlds with unseen output first
    if world_switch_mode == WorldSwitchMode::UnseenFirst {
        let mut unseen_worlds: Vec<usize> = cycleable.iter()
            .filter(|&&i| i != current_index && world_has_pending(&worlds[i]))
            .copied()
            .collect();

        if !unseen_worlds.is_empty() {
            // Sort alphabetically and go to first (same behavior as next when unseen first is on)
            unseen_worlds.sort_by(|&a, &b| {
                worlds[a].name.to_lowercase().cmp(&worlds[b].name.to_lowercase())
            });
            return Some(unseen_worlds[0]);
        }
    }

    // Fall back to alphabetical cycling
    let mut sorted = cycleable.clone();
    sorted.sort_by(|&a, &b| {
        worlds[a].name.to_lowercase().cmp(&worlds[b].name.to_lowercase())
    });

    // Find current position in the sorted cycleable list
    let current_pos = sorted.iter().position(|&i| i == current_index);

    match current_pos {
        Some(pos) => {
            let prev_pos = if pos > 0 { pos - 1 } else { sorted.len() - 1 };
            if sorted[prev_pos] != current_index {
                Some(sorted[prev_pos])
            } else {
                None
            }
        }
        None => {
            // Current world isn't in cycleable list, go to last cycleable world
            Some(sorted[sorted.len() - 1])
        }
    }
}

// ============================================================================
// Connected Worlds List Formatting (/l command)
// ============================================================================

/// Format a duration as a human-readable string
/// - Under 60 minutes: Xm (e.g., 12m, 45m)
/// - 1-24 hours: X.Xh (e.g., 2.5h, 12.3h)
/// - Over 24 hours: X.Xd (e.g., 1.2d, 3.5d)
pub fn format_duration_short(secs: u64) -> String {
    let minutes = secs / 60;
    let hours = secs as f64 / 3600.0;
    let days = secs as f64 / 86400.0;

    if minutes < 60 {
        format!("{}m", minutes)
    } else if hours < 24.0 {
        format!("{:.1}h", hours)
    } else {
        format!("{:.1}d", days)
    }
}

/// Information about a world needed for the /l command output
pub struct WorldListInfo {
    pub name: String,
    pub connected: bool,
    pub is_current: bool,
    pub unseen_lines: usize,
    pub last_send_secs: Option<u64>,
    pub last_recv_secs: Option<u64>,
    pub last_nop_secs: Option<u64>,
    pub next_nop_secs: Option<u64>,
}

/// Format the connected worlds list for /l command output
/// Returns a string with ANSI color codes for terminal display
/// Only shows connected worlds
pub fn format_worlds_list(worlds: &[WorldListInfo]) -> String {
    const KEEPALIVE_SECS: u64 = 5 * 60;

    // ANSI color codes
    const GRAY: &str = "\x1b[90m";
    const YELLOW: &str = "\x1b[33m";
    const CYAN: &str = "\x1b[36m";
    const RESET: &str = "\x1b[0m";

    // Filter to connected worlds only
    let connected_worlds: Vec<&WorldListInfo> = worlds.iter()
        .filter(|w| w.connected)
        .collect();

    if connected_worlds.is_empty() {
        return "No worlds connected.".to_string();
    }

    let mut lines = Vec::new();

    // Header line
    lines.push(format!(
        "{:20} {:>6}  {:>8}  {:>8}  {:>8}  {:>8}",
        "World", "Unseen", "LastSend", "LastRecv", "LastNOP", "NextNOP"
    ));

    for world in connected_worlds {
        // Current world marker
        let current_marker = if world.is_current {
            format!("{}*{}", CYAN, RESET)
        } else {
            " ".to_string()
        };

        // Unseen count: yellow if > 0, gray dash otherwise
        let unseen = if world.unseen_lines > 0 {
            format!("{}{:>6}{}", YELLOW, world.unseen_lines, RESET)
        } else {
            format!("{}{:>6}{}", GRAY, "—", RESET)
        };

        // Time columns
        let last_send = world.last_send_secs
            .map(format_duration_short)
            .unwrap_or_else(|| "—".to_string());
        let last_recv = world.last_recv_secs
            .map(format_duration_short)
            .unwrap_or_else(|| "—".to_string());
        let last_nop = world.last_nop_secs
            .map(format_duration_short)
            .unwrap_or_else(|| "—".to_string());

        // NextNOP: calculate based on last activity
        let last_activity = match (world.last_send_secs, world.last_recv_secs) {
            (Some(s), Some(r)) => Some(s.min(r)),
            (Some(s), None) => Some(s),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        };
        let next_nop = match last_activity {
            Some(elapsed) => {
                let remaining = KEEPALIVE_SECS.saturating_sub(elapsed);
                format_duration_short(remaining)
            }
            None => "—".to_string(),
        };

        // Truncate world name to 18 chars to fit with marker
        let name_display = if world.name.len() > 18 {
            format!("{}...", &world.name[..15])
        } else {
            world.name.clone()
        };

        lines.push(format!(
            "{} {:18} {}  {:>8}  {:>8}  {:>8}  {:>8}",
            current_marker, name_display, unseen, last_send, last_recv, last_nop, next_nop
        ));
    }

    lines.join("\n")
}
