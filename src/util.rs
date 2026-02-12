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

/// Normalize a prompt string: strip CR/LF, strip trailing spaces, add exactly one space
pub fn normalize_prompt(text: &str) -> String {
    let clean = text.replace('\r', "").replace('\n', " ");
    format!("{} ", clean.trim())
}

/// Convert a color name to ANSI background color code
/// Supports named colors, xterm 256-color codes, and RGB values
/// Empty string returns a default highlight color (dark cyan background)
pub fn color_name_to_ansi_bg(color: &str) -> String {
    let color_lower = color.to_lowercase();
    let color_lower = color_lower.trim();

    // Empty color means use default highlight
    if color_lower.is_empty() {
        return "\x1b[48;5;23m".to_string(); // Dark cyan background
    }

    // Named colors (using darker/muted versions for backgrounds)
    match color_lower {
        "red" => "\x1b[48;5;52m".to_string(),      // Dark red
        "green" => "\x1b[48;5;22m".to_string(),    // Dark green
        "blue" => "\x1b[48;5;17m".to_string(),     // Dark blue
        "yellow" => "\x1b[48;5;58m".to_string(),   // Dark yellow/olive
        "cyan" => "\x1b[48;5;23m".to_string(),     // Dark cyan
        "magenta" | "purple" => "\x1b[48;5;53m".to_string(), // Dark magenta
        "orange" => "\x1b[48;5;94m".to_string(),   // Dark orange
        "pink" => "\x1b[48;5;125m".to_string(),    // Dark pink
        "white" => "\x1b[48;5;250m".to_string(),   // Light gray (for contrast)
        "black" => "\x1b[48;5;234m".to_string(),   // Very dark gray
        "gray" | "grey" => "\x1b[48;5;240m".to_string(), // Medium gray
        _ => {
            // Try parsing as xterm 256 color number
            if let Ok(num) = color_lower.parse::<u8>() {
                return format!("\x1b[48;5;{}m", num);
            }
            // Try parsing as RGB (format: r,g,b or r;g;b with values 0-255)
            let parts: Vec<&str> = if color_lower.contains(',') {
                color_lower.split(',').collect()
            } else if color_lower.contains(';') {
                color_lower.split(';').collect()
            } else {
                vec![]
            };
            if parts.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    parts[0].trim().parse::<u8>(),
                    parts[1].trim().parse::<u8>(),
                    parts[2].trim().parse::<u8>(),
                ) {
                    return format!("\x1b[48;2;{};{};{}m", r, g, b);
                }
            }
            // Default fallback
            "\x1b[48;5;23m".to_string() // Dark cyan
        }
    }
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

// ============================================================================
// Cross-Platform Local Time
// ============================================================================

/// Portable local time representation
pub struct LocalTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
    pub weekday: i32, // 0=Sunday
}

/// Convert epoch seconds to local time (Unix implementation)
#[cfg(unix)]
#[allow(deprecated)]
pub fn local_time_from_epoch(epoch_secs: i64) -> LocalTime {
    let time_t = epoch_secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&time_t, &mut tm);
    }
    LocalTime {
        year: tm.tm_year + 1900,
        month: tm.tm_mon + 1,
        day: tm.tm_mday,
        hour: tm.tm_hour,
        minute: tm.tm_min,
        second: tm.tm_sec,
        weekday: tm.tm_wday,
    }
}

/// Convert epoch seconds to local time (Windows implementation)
#[cfg(windows)]
pub fn local_time_from_epoch(epoch_secs: i64) -> LocalTime {
    // Windows FILETIME epoch is 1601-01-01, Unix epoch is 1970-01-01
    // Difference: 11644473600 seconds
    const UNIX_TO_FILETIME_OFFSET: i64 = 11_644_473_600;
    // FILETIME is in 100-nanosecond intervals
    let filetime_val = (epoch_secs + UNIX_TO_FILETIME_OFFSET) * 10_000_000;

    #[repr(C)]
    struct FILETIME {
        dw_low_date_time: u32,
        dw_high_date_time: u32,
    }

    #[repr(C)]
    struct SYSTEMTIME {
        w_year: u16,
        w_month: u16,
        w_day_of_week: u16,
        w_day: u16,
        w_hour: u16,
        w_minute: u16,
        w_second: u16,
        w_milliseconds: u16,
    }

    extern "system" {
        fn FileTimeToSystemTime(ft: *const FILETIME, st: *mut SYSTEMTIME) -> i32;
        fn SystemTimeToTzSpecificLocalTime(
            tz: *const std::ffi::c_void,
            utc: *const SYSTEMTIME,
            local: *mut SYSTEMTIME,
        ) -> i32;
    }

    let ft = FILETIME {
        dw_low_date_time: filetime_val as u32,
        dw_high_date_time: (filetime_val >> 32) as u32,
    };

    let mut utc_st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local_st: SYSTEMTIME = unsafe { std::mem::zeroed() };

    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc_st) == 0 {
            return LocalTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, weekday: 4 };
        }
        if SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc_st, &mut local_st) == 0 {
            return LocalTime { year: 1970, month: 1, day: 1, hour: 0, minute: 0, second: 0, weekday: 4 };
        }
    }

    LocalTime {
        year: local_st.w_year as i32,
        month: local_st.w_month as i32,
        day: local_st.w_day as i32,
        hour: local_st.w_hour as i32,
        minute: local_st.w_minute as i32,
        second: local_st.w_second as i32,
        weekday: local_st.w_day_of_week as i32,
    }
}

/// Get the current local time
pub fn local_time_now() -> LocalTime {
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    local_time_from_epoch(epoch_secs)
}

/// Get the current time in 12-hour format (H:MM)
pub fn get_current_time_12hr() -> String {
    let lt = local_time_now();

    let hours_24 = lt.hour as u32;
    let minutes = lt.minute as u32;

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
        let mut end = max_len - 3;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
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

    let lt = local_time_from_epoch(timestamp);

    let year = lt.year;
    let month = lt.month;
    let day = lt.day;
    let hour = lt.hour;
    let minute = lt.minute;
    let second = lt.second;
    let weekday = lt.weekday;

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

/// Direction for world switching
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SwitchDirection {
    Next,
    Previous,
}

/// Get the next or previous world index to switch to
/// Returns None if no world to switch to, or Some(index) of the target world
pub fn calculate_world_switch(
    worlds: &[WorldSwitchInfo],
    current_index: usize,
    world_switch_mode: WorldSwitchMode,
    direction: SwitchDirection,
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
            let target_pos = match direction {
                SwitchDirection::Next => (pos + 1) % sorted.len(),
                SwitchDirection::Previous => if pos > 0 { pos - 1 } else { sorted.len() - 1 },
            };
            if sorted[target_pos] != current_index {
                Some(sorted[target_pos])
            } else {
                None
            }
        }
        None => {
            // Current world isn't in cycleable list, go to first or last cycleable world
            match direction {
                SwitchDirection::Next => Some(sorted[0]),
                SwitchDirection::Previous => Some(sorted[sorted.len() - 1]),
            }
        }
    }
}

/// Get the next world index to switch to (convenience wrapper)
pub fn calculate_next_world(
    worlds: &[WorldSwitchInfo],
    current_index: usize,
    world_switch_mode: WorldSwitchMode,
) -> Option<usize> {
    calculate_world_switch(worlds, current_index, world_switch_mode, SwitchDirection::Next)
}

/// Get the previous world index to switch to (convenience wrapper)
pub fn calculate_prev_world(
    worlds: &[WorldSwitchInfo],
    current_index: usize,
    world_switch_mode: WorldSwitchMode,
) -> Option<usize> {
    calculate_world_switch(worlds, current_index, world_switch_mode, SwitchDirection::Previous)
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
    pub buffer_size: usize,
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
        last: String,        // recv/send combined
        ka: String,          // lastAK/nextAK combined
        buffer: String,
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
        let last = format!("{}/{}", last_recv, last_send);
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
        let ka = format!("{}/{}", last_ak, next_ak);
        let buffer = world.buffer_size.to_string();
        FormattedWorld {
            ssh,
            current_marker,
            name: world.name.clone(),
            unseen,
            unseen_raw,
            last,
            ka,
            buffer,
        }
    }).collect();

    // Calculate dynamic column widths (minimum is header width)
    let name_width = formatted.iter().map(|w| w.name.len()).max().unwrap_or(5).max(5);
    let unseen_width = formatted.iter().map(|w| w.unseen_raw.len()).max().unwrap_or(6).max(6);
    let last_width = formatted.iter().map(|w| w.last.len()).max().unwrap_or(4).max(4);
    let ka_width = formatted.iter().map(|w| w.ka.len()).max().unwrap_or(2).max(2);
    let buffer_width = formatted.iter().map(|w| w.buffer.len()).max().unwrap_or(6).max(6);

    let mut lines = Vec::new();

    // Header line with dynamic widths
    lines.push(format!(
        "  SSH  {:name_w$}  {:>unseen_w$}  {:>last_w$}  {:>ka_w$}  {:>buf_w$}",
        "World", "Unseen", "Last", "KA", "Buffer",
        name_w = name_width,
        unseen_w = unseen_width,
        last_w = last_width,
        ka_w = ka_width,
        buf_w = buffer_width
    ));

    for world in &formatted {
        lines.push(format!(
            "{} {}  {:name_w$}  {:>unseen_w$}  {:>last_w$}  {:>ka_w$}  {:>buf_w$}",
            world.current_marker, world.ssh, world.name, world.unseen,
            world.last, world.ka, world.buffer,
            name_w = name_width,
            unseen_w = unseen_width + (world.unseen.len() - world.unseen_raw.len()),  // Account for color codes
            last_w = last_width,
            ka_w = ka_width,
            buf_w = buffer_width
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
