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

/// Parse Discord timestamps in text and convert them to human-readable format
/// Discord format: <t:TIMESTAMP:FORMAT> where FORMAT is t, T, d, D, f, F, s, S, or R
/// - t: short time (10:17 PM)
/// - T: long time (10:17:46 PM)
/// - d: short date (01/17/2026)
/// - D: long date (January 17, 2026)
/// - f: short date/time (January 17, 2026 10:17 PM)
/// - F: long date/time (Friday, January 17, 2026 10:17 PM)
/// - s: short date + time (01/16/2026, 10:27 PM)
/// - S: short date + time with seconds (01/16/2026, 10:27:33 PM)
/// - R: relative time (in 2 hours, 3 days ago)
pub fn parse_discord_timestamps(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find("<t:") {
        // Add text before the timestamp
        result.push_str(&remaining[..start]);

        // Find the end of the timestamp
        let after_start = &remaining[start + 3..];
        if let Some(end) = after_start.find('>') {
            let inner = &after_start[..end];

            // Parse timestamp and optional format
            let (timestamp_str, format) = if let Some(colon_pos) = inner.rfind(':') {
                (&inner[..colon_pos], &inner[colon_pos + 1..])
            } else {
                (inner, "f") // Default format
            };

            if let Ok(timestamp) = timestamp_str.parse::<i64>() {
                // Convert timestamp to formatted string
                let formatted = format_discord_timestamp(timestamp, format);
                result.push_str(&formatted);
                remaining = &after_start[end + 1..];
                continue;
            }
        }

        // Couldn't parse, keep the original text
        result.push_str("<t:");
        remaining = after_start;
    }

    // Add any remaining text
    result.push_str(remaining);
    result
}

/// Format a Unix timestamp according to Discord format specifier
fn format_discord_timestamp(timestamp: i64, format: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Convert timestamp to tm struct using libc
    let time_t = timestamp as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&time_t, &mut tm);
    }

    let year = tm.tm_year + 1900;
    let month = tm.tm_mon + 1;
    let day = tm.tm_mday;
    let hour = tm.tm_hour;
    let minute = tm.tm_min;
    let second = tm.tm_sec;
    let weekday = tm.tm_wday;

    let month_names = ["January", "February", "March", "April", "May", "June",
                       "July", "August", "September", "October", "November", "December"];
    let weekday_names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

    let month_name = month_names.get((month - 1) as usize).unwrap_or(&"???");
    let weekday_name = weekday_names.get(weekday as usize).unwrap_or(&"???");

    // 12-hour format
    let (hour_12, am_pm) = if hour == 0 {
        (12, "AM")
    } else if hour < 12 {
        (hour, "AM")
    } else if hour == 12 {
        (12, "PM")
    } else {
        (hour - 12, "PM")
    };

    match format {
        "t" => format!("{}:{:02} {}", hour_12, minute, am_pm),
        "T" => format!("{}:{:02}:{:02} {}", hour_12, minute, second, am_pm),
        "d" => format!("{:02}/{:02}/{}", month, day, year),
        "D" => format!("{} {}, {}", month_name, day, year),
        "f" => format!("{} {}, {} {}:{:02} {}", month_name, day, year, hour_12, minute, am_pm),
        "F" => format!("{}, {} {}, {} {}:{:02} {}", weekday_name, month_name, day, year, hour_12, minute, am_pm),
        "s" => format!("{:02}/{:02}/{}, {}:{:02} {}", month, day, year, hour_12, minute, am_pm),
        "S" => format!("{:02}/{:02}/{}, {}:{:02}:{:02} {}", month, day, year, hour_12, minute, second, am_pm),
        "R" => {
            // Relative time
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let diff = timestamp - now;

            if diff.abs() < 60 {
                "just now".to_string()
            } else if diff.abs() < 3600 {
                let mins = diff.abs() / 60;
                if diff > 0 {
                    format!("in {} minute{}", mins, if mins == 1 { "" } else { "s" })
                } else {
                    format!("{} minute{} ago", mins, if mins == 1 { "" } else { "s" })
                }
            } else if diff.abs() < 86400 {
                let hours = diff.abs() / 3600;
                if diff > 0 {
                    format!("in {} hour{}", hours, if hours == 1 { "" } else { "s" })
                } else {
                    format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
                }
            } else {
                let days = diff.abs() / 86400;
                if diff > 0 {
                    format!("in {} day{}", days, if days == 1 { "" } else { "s" })
                } else {
                    format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
                }
            }
        }
        _ => format!("{} {}, {} {}:{:02} {}", month_name, day, year, hour_12, minute, am_pm), // Default to 'f'
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
    pub pending_lines: usize,
    pub first_unseen_at: Option<std::time::Instant>,
}

/// Determine if a world should be included in the cycle list
/// (connected OR has unseen output OR has pending lines from more-mode)
pub fn world_should_cycle(info: &WorldSwitchInfo) -> bool {
    info.connected || info.unseen_lines > 0 || info.pending_lines > 0
}

/// Check if a world has pending/unseen output (including more-mode pending lines)
pub fn world_has_pending(info: &WorldSwitchInfo) -> bool {
    info.unseen_lines > 0 || info.pending_lines > 0
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
            // Sort by first_unseen_at (oldest first), then alphabetically as tiebreaker
            unseen_worlds.sort_by(|&a, &b| {
                match (worlds[a].first_unseen_at, worlds[b].first_unseen_at) {
                    (Some(time_a), Some(time_b)) => time_a.cmp(&time_b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => worlds[a].name.to_lowercase().cmp(&worlds[b].name.to_lowercase()),
                }
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
            // Sort by first_unseen_at (oldest first), then alphabetically as tiebreaker
            unseen_worlds.sort_by(|&a, &b| {
                match (worlds[a].first_unseen_at, worlds[b].first_unseen_at) {
                    (Some(time_a), Some(time_b)) => time_a.cmp(&time_b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => worlds[a].name.to_lowercase().cmp(&worlds[b].name.to_lowercase()),
                }
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
    pub is_ssl: bool,
    pub is_proxy: bool,
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

    // Build formatted data for each world first to calculate column widths
    struct FormattedWorld {
        ssh: String,
        current_marker: String,
        name: String,
        unseen: String,
        unseen_raw: String,  // Without color codes for width calculation
        last_send: String,
        last_recv: String,
        last_ak: String,
        next_ak: String,
    }

    let formatted: Vec<FormattedWorld> = connected_worlds.iter().map(|world| {
        let ssh = if world.is_ssl {
            if world.is_proxy { "PRX" } else { "SSH" }
        } else {
            "   "
        }.to_string();
        let current_marker = if world.is_current {
            format!("{}*{}", CYAN, RESET)
        } else {
            " ".to_string()
        };
        let unseen_raw = if world.unseen_lines > 0 {
            world.unseen_lines.to_string()
        } else {
            "—".to_string()
        };
        let unseen = if world.unseen_lines > 0 {
            format!("{}{}{}", YELLOW, world.unseen_lines, RESET)
        } else {
            format!("{}—{}", GRAY, RESET)
        };
        let last_send = world.last_send_secs
            .map(format_duration_short)
            .unwrap_or_else(|| "—".to_string());
        let last_recv = world.last_recv_secs
            .map(format_duration_short)
            .unwrap_or_else(|| "—".to_string());
        let last_ak = world.last_nop_secs
            .map(format_duration_short)
            .unwrap_or_else(|| "—".to_string());
        let last_activity = match (world.last_send_secs, world.last_recv_secs) {
            (Some(s), Some(r)) => Some(s.min(r)),
            (Some(s), None) => Some(s),
            (None, Some(r)) => Some(r),
            (None, None) => None,
        };
        let next_ak = match last_activity {
            Some(elapsed) => {
                let remaining = KEEPALIVE_SECS.saturating_sub(elapsed);
                format_duration_short(remaining)
            }
            None => "—".to_string(),
        };
        FormattedWorld {
            ssh,
            current_marker,
            name: world.name.clone(),
            unseen,
            unseen_raw,
            last_send,
            last_recv,
            last_ak,
            next_ak,
        }
    }).collect();

    // Calculate dynamic column widths (minimum is header width)
    let name_width = formatted.iter().map(|w| w.name.len()).max().unwrap_or(5).max(5);
    let unseen_width = formatted.iter().map(|w| w.unseen_raw.len()).max().unwrap_or(6).max(6);
    let send_width = formatted.iter().map(|w| w.last_send.len()).max().unwrap_or(8).max(8);
    let recv_width = formatted.iter().map(|w| w.last_recv.len()).max().unwrap_or(8).max(8);
    let last_ak_width = formatted.iter().map(|w| w.last_ak.len()).max().unwrap_or(6).max(6);
    let next_ak_width = formatted.iter().map(|w| w.next_ak.len()).max().unwrap_or(6).max(6);

    let mut lines = Vec::new();

    // Header line with dynamic widths
    lines.push(format!(
        "  SSH  {:name_w$}  {:>unseen_w$}  {:>send_w$}  {:>recv_w$}  {:>last_ak_w$}  {:>next_ak_w$}",
        "World", "Unseen", "LastSend", "LastRecv", "LastAK", "NextAK",
        name_w = name_width,
        unseen_w = unseen_width,
        send_w = send_width,
        recv_w = recv_width,
        last_ak_w = last_ak_width,
        next_ak_w = next_ak_width
    ));

    for world in &formatted {
        lines.push(format!(
            "{} {}  {:name_w$}  {:>unseen_w$}  {:>send_w$}  {:>recv_w$}  {:>last_ak_w$}  {:>next_ak_w$}",
            world.current_marker, world.ssh, world.name, world.unseen,
            world.last_send, world.last_recv, world.last_ak, world.next_ak,
            name_w = name_width,
            unseen_w = unseen_width + (world.unseen.len() - world.unseen_raw.len()),  // Account for color codes
            send_w = send_width,
            recv_w = recv_width,
            last_ak_w = last_ak_width,
            next_ak_w = next_ak_width
        ));
    }

    lines.join("\n")
}

/// Convert temperatures in text: "32C" -> "32C (90F)", "100F" -> "100F (38C)"
/// Matches numbers followed by C or F (with optional space), followed by delimiter or end
/// Uses fast character scanning instead of regex for performance
pub fn convert_temperatures(text: &str) -> String {
    let bytes = text.as_bytes();

    // Quick scan: does this line have any potential temperature patterns?
    // Look for C or F that could follow a digit
    let mut has_potential = false;
    for i in 1..bytes.len() {
        let c = bytes[i];
        if c == b'C' || c == b'c' || c == b'F' || c == b'f' {
            // Check if preceded by digit or space+digit
            let prev = bytes[i - 1];
            if prev.is_ascii_digit() || (prev == b' ' && i >= 2 && bytes[i - 2].is_ascii_digit()) {
                has_potential = true;
                break;
            }
        }
    }

    if !has_potential {
        return text.to_string();
    }

    // Full parse - we have at least one potential match
    let mut result = String::with_capacity(text.len() + 32);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Look for start of a number (digit or minus followed by digit)
        let num_start = i;
        let mut num_end;

        // Check for optional minus sign
        if chars[i] == '-' {
            if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                num_end = i + 1;
            } else {
                result.push(chars[i]);
                i += 1;
                continue;
            }
        } else if chars[i].is_ascii_digit() {
            num_end = i;
        } else {
            result.push(chars[i]);
            i += 1;
            continue;
        }

        // Consume digits
        while num_end < chars.len() && chars[num_end].is_ascii_digit() {
            num_end += 1;
        }

        // Check for decimal part
        if num_end < chars.len() && chars[num_end] == '.' {
            let dot_pos = num_end;
            num_end += 1;
            if num_end < chars.len() && chars[num_end].is_ascii_digit() {
                while num_end < chars.len() && chars[num_end].is_ascii_digit() {
                    num_end += 1;
                }
            } else {
                // Not a decimal, just a period - backtrack
                num_end = dot_pos;
            }
        }

        // Check for optional space then C/F
        let mut unit_pos = num_end;
        let has_space = unit_pos < chars.len() && chars[unit_pos] == ' ';
        if has_space {
            unit_pos += 1;
        }

        // Check for C or F
        if unit_pos < chars.len() {
            let unit_char = chars[unit_pos];
            if unit_char == 'C' || unit_char == 'c' || unit_char == 'F' || unit_char == 'f' {
                // Check delimiter after unit (space, period, comma, quotes, etc.)
                let after_unit = unit_pos + 1;
                let is_valid_end = after_unit >= chars.len() ||
                    matches!(chars[after_unit], ' ' | '.' | ',' | ';' | ':' | '!' | '?' | ']' | ')' | '\n' | '\r' | '"' | '\'');

                if is_valid_end {
                    // Valid temperature - convert it
                    let num_str: String = chars[num_start..num_end].iter().collect();
                    if let Ok(num) = num_str.parse::<f64>() {
                        let (converted, new_unit) = if unit_char == 'C' || unit_char == 'c' {
                            (((num * 9.0 / 5.0) + 32.0).round() as i64, 'F')
                        } else {
                            (((num - 32.0) * 5.0 / 9.0).round() as i64, 'C')
                        };

                        result.push_str(&num_str);
                        if has_space {
                            result.push(' ');
                        }
                        result.push(unit_char);
                        result.push_str(&format!(" ({}{})", converted, new_unit));
                        i = after_unit;
                        continue;
                    }
                }
            }
        }

        // Not a valid temperature pattern - output the number as-is
        for c in &chars[num_start..num_end] {
            result.push(*c);
        }
        i = num_end;
    }

    result
}
