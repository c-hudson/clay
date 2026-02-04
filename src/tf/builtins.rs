//! Additional builtin commands for TinyFugue compatibility.
//!
//! Implements:
//! - Output commands: #beep, #gag, #ungag, #recall, #quote
//! - File operations: #load, #save, #log
//! - Miscellaneous: #time, #sh, #lcd

use std::fs;
use std::path::Path;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};
use super::{TfEngine, TfProcess, TfCommandResult, RecallOptions, RecallSource, RecallRange, RecallMatchStyle};

/// #beep - Sound the terminal bell
pub fn cmd_beep() -> TfCommandResult {
    // Return a special message that the main app can interpret
    TfCommandResult::Success(Some("\x07".to_string()))
}

/// #time [format] - Display current time
pub fn cmd_time(args: &str) -> TfCommandResult {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let format = args.trim();

    if format.is_empty() {
        // Default format: human readable
        let secs = now % 60;
        let mins = (now / 60) % 60;
        let hours = (now / 3600) % 24;
        TfCommandResult::Success(Some(format!("{:02}:{:02}:{:02}", hours, mins, secs)))
    } else if format == "%s" || format == "epoch" {
        // Unix timestamp
        TfCommandResult::Success(Some(now.to_string()))
    } else {
        // For now, just return the timestamp
        // Full strftime support could be added later
        TfCommandResult::Success(Some(now.to_string()))
    }
}

/// #lcd [directory] - Change local directory
pub fn cmd_lcd(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let dir = args.trim();

    if dir.is_empty() {
        // Show current directory
        if let Some(ref cd) = engine.current_dir {
            return TfCommandResult::Success(Some(cd.clone()));
        }
        if let Ok(cwd) = std::env::current_dir() {
            return TfCommandResult::Success(Some(cwd.display().to_string()));
        }
        return TfCommandResult::Success(Some(".".to_string()));
    }

    // Expand ~ to home directory
    let expanded = if dir.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy();
            if dir == "~" {
                home_str.to_string()
            } else if let Some(rest) = dir.strip_prefix("~/") {
                format!("{}/{}", home_str, rest)
            } else {
                dir.to_string()
            }
        } else {
            dir.to_string()
        }
    } else {
        dir.to_string()
    };

    // Verify directory exists
    let path = Path::new(&expanded);
    if path.is_dir() {
        engine.current_dir = Some(expanded.clone());
        TfCommandResult::Success(Some(format!("Changed to {}", expanded)))
    } else {
        TfCommandResult::Error(format!("Directory not found: {}", expanded))
    }
}

/// #sh command - Execute shell command
pub fn cmd_sh(args: &str) -> TfCommandResult {
    let cmd = args.trim();

    if cmd.is_empty() {
        return TfCommandResult::Error("Usage: #sh command".to_string());
    }

    // Execute command and capture output
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&stderr);
            }

            if result.is_empty() {
                TfCommandResult::Success(None)
            } else {
                TfCommandResult::Success(Some(result.trim_end().to_string()))
            }
        }
        Err(e) => TfCommandResult::Error(format!("Failed to execute: {}", e)),
    }
}

/// #quote [options] [prefix] source [suffix] - Generate text from file, command, or literal
///
/// Sources:
///   '"file"     - Read lines from a file
///   `"command"  - Read output from internal Clay/TF command
///   !"command"  - Read output from shell command
///   text        - Send literal text (no special prefix)
///
/// Options:
///   -dsend      - Send each line to MUD (default when no prefix)
///   -decho      - Echo each line locally
///   -dexec      - Execute each line as TF command
///   -wworld     - Send to specified world
///   -S          - Synchronous mode (wait for completion)
///
/// Examples:
///   #quote hello world           - Send "hello world" to MUD
///   #quote '"/etc/motd"          - Send each line of /etc/motd to MUD
///   #quote say '"/tmp/lines.txt" - Send "say <line>" for each line
///   #quote think `"/version"     - Send "think <version>" to MUD
///   #quote !"ls -la"             - Send output of shell ls command
///   #quote -decho '"config.txt"  - Display file contents locally
pub fn cmd_quote(engine: &mut super::TfEngine, args: &str) -> TfCommandResult {
    use super::QuoteDisposition;
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    if args.is_empty() {
        return TfCommandResult::Error("Usage: #quote [options] [prefix] source [suffix]".to_string());
    }

    let mut input = args.trim();
    let mut disposition = QuoteDisposition::Send;
    let mut world: Option<String> = None;
    let mut _synchronous = false;
    let mut _on_prompt = false;  // -P flag: run on prompt (not yet implemented)
    let mut delay_secs: f64 = 0.0;  // Timing between lines

    // Helper to parse time string: "seconds", "min:sec", or "hour:min:sec"
    fn parse_time_spec(s: &str) -> Option<f64> {
        if s == "S" {
            return Some(0.0);  // Synchronous = no delay
        }
        if s == "P" {
            return None;  // Prompt-based, handled separately
        }
        let parts: Vec<&str> = s.split(':').collect();
        match parts.len() {
            1 => parts[0].parse::<f64>().ok(),
            2 => {
                // Could be hours:minutes or minutes:seconds
                // TF treats it as hours:minutes, but we'll be flexible
                let a: f64 = parts[0].parse().ok()?;
                let b: f64 = parts[1].parse().ok()?;
                Some(a * 60.0 + b)  // Treat as minutes:seconds for practical use
            }
            3 => {
                let hours: f64 = parts[0].parse().ok()?;
                let mins: f64 = parts[1].parse().ok()?;
                let secs: f64 = parts[2].parse().ok()?;
                Some(hours * 3600.0 + mins * 60.0 + secs)
            }
            _ => None,
        }
    }

    // Check if string looks like a time spec (digits, colons, dots, or S/P)
    fn is_time_spec(s: &str) -> bool {
        if s == "S" || s == "P" {
            return true;
        }
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.')
    }

    // Parse options
    while input.starts_with('-') {
        if let Some(space_pos) = input.find(|c: char| c.is_whitespace()) {
            let opt = &input[..space_pos];
            input = input[space_pos..].trim_start();

            if let Some(disp_str) = opt.strip_prefix("-d") {
                disposition = match disp_str {
                    "send" => QuoteDisposition::Send,
                    "echo" => QuoteDisposition::Echo,
                    "exec" => QuoteDisposition::Exec,
                    _ => return TfCommandResult::Error(format!("Unknown disposition: {}. Use send, echo, or exec.", disp_str)),
                };
            } else if opt.starts_with("-w") {
                world = Some(opt[2..].to_string());
            } else if opt == "-S" {
                _synchronous = true;
            } else if opt == "-P" {
                _on_prompt = true;
            } else if opt.len() >= 2 && is_time_spec(&opt[1..]) {
                // Timing option: -0, -1, -0.5, -1:30, -1:30:00, etc.
                let time_str = &opt[1..];
                if time_str == "P" {
                    _on_prompt = true;
                } else if let Some(secs) = parse_time_spec(time_str) {
                    delay_secs = secs;
                    if time_str == "S" {
                        _synchronous = true;
                    }
                } else {
                    return TfCommandResult::Error(format!("Invalid timing option: {}", opt));
                }
            } else {
                return TfCommandResult::Error(format!("Unknown option: {}", opt));
            }
        } else {
            // Option at end with no more args - check if it's a valid option
            if input.starts_with("-d") || input.starts_with("-w") || input == "-S" || input == "-P" {
                return TfCommandResult::Error("No source specified after options".to_string());
            }
            // Check for timing option at end
            if input.len() >= 2 && is_time_spec(&input[1..]) {
                return TfCommandResult::Error("No source specified after options".to_string());
            }
            // Not an option - break to process as source
            break;
        }
    }

    // Find the source specifier: ' for file, ` or ! for shell, # for TF command
    // Format: [prefix] source [suffix]
    // source is: '"file"suffix or 'file suffix or `"cmd"suffix or !cmd suffix

    let (prefix, source_pos) = if let Some(pos) = input.find(['\'', '`', '!', '#']) {
        // Check if the # is actually a TF command source or just part of text
        let char_at_pos = input.chars().nth(pos).unwrap();
        if char_at_pos == '#' {
            // Only treat as source if followed by " (for #"command" syntax)
            let after_hash = &input[pos + 1..];
            if after_hash.starts_with('"') {
                // Keep trailing space in prefix (user controls spacing)
                (&input[..pos], Some(pos))
            } else {
                // No special source, treat entire input as literal text
                ("", None)
            }
        } else {
            // Keep trailing space in prefix (user controls spacing)
            (&input[..pos], Some(pos))
        }
    } else {
        // No special source character, treat entire input as literal text
        ("", None)
    };

    // If no source specifier found, send the text literally
    let source_start = match source_pos {
        Some(pos) => pos,
        None => {
            return TfCommandResult::Quote {
                lines: vec![input.to_string()],
                disposition,
                world,
                delay_secs,
            };
        }
    };

    let source_char = input.chars().nth(source_start).unwrap();
    let after_source_char = &input[source_start + 1..];

    // Parse the source: could be quoted ("...") or unquoted (word)
    let (source_value, suffix) = if after_source_char.starts_with('"') {
        // Quoted source: find closing quote
        let content_start = 1; // Skip opening quote
        let mut end = content_start;
        let chars: Vec<char> = after_source_char.chars().collect();
        let mut source_content = String::new();

        while end < chars.len() {
            if chars[end] == '\\' && end + 1 < chars.len() {
                // Escape sequence
                source_content.push(chars[end + 1]);
                end += 2;
            } else if chars[end] == '"' {
                // End of quoted string
                break;
            } else {
                source_content.push(chars[end]);
                end += 1;
            }
        }

        // Calculate byte position for suffix
        let byte_end = after_source_char
            .char_indices()
            .nth(end + 1)
            .map(|(i, _)| i)
            .unwrap_or(after_source_char.len());
        let suffix = after_source_char[byte_end..].trim();

        (source_content, suffix)
    } else {
        // Unquoted source: read until whitespace
        if let Some(space_pos) = after_source_char.find(char::is_whitespace) {
            let source = after_source_char[..space_pos].to_string();
            let suffix = after_source_char[space_pos..].trim();
            (source, suffix)
        } else {
            (after_source_char.to_string(), "")
        }
    };

    // Read lines from the source
    let lines: Vec<String> = match source_char {
        '\'' => {
            // File source - expand ~ to home directory
            let path = if source_value.starts_with("~/") {
                if let Some(home) = home::home_dir() {
                    home.join(&source_value[2..]).to_string_lossy().into_owned()
                } else {
                    source_value.clone()
                }
            } else if source_value == "~" {
                home::home_dir()
                    .map(|h| h.to_string_lossy().into_owned())
                    .unwrap_or_else(|| source_value.clone())
            } else {
                source_value.clone()
            };
            match std::fs::File::open(&path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    reader.lines()
                        .filter_map(|l| l.ok())
                        .map(|line| format!("{}{}{}", prefix, line, suffix))
                        .collect()
                }
                Err(e) => return TfCommandResult::Error(format!("Cannot open file '{}': {}", path, e)),
            }
        }
        '`' => {
            // Internal command source (Clay/TF command)
            let result = super::parser::execute_command(engine, &source_value);
            match result {
                TfCommandResult::Success(Some(msg)) => {
                    msg.lines()
                        .map(|line| format!("{}{}{}", prefix, line, suffix))
                        .collect()
                }
                TfCommandResult::Success(None) => {
                    vec![]
                }
                TfCommandResult::Error(e) => {
                    return TfCommandResult::Error(format!("Command '{}' failed: {}", source_value, e));
                }
                _ => {
                    // Other result types (SendToMud, ClayCommand, etc.) don't produce capturable output
                    vec![]
                }
            }
        }
        '!' => {
            // Shell command source
            match Command::new("sh")
                .arg("-c")
                .arg(&source_value)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
            {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    stdout
                        .lines()
                        .map(|line| format!("{}{}{}", prefix, line, suffix))
                        .collect()
                }
                Err(e) => return TfCommandResult::Error(format!("Cannot execute shell command '{}': {}", source_value, e)),
            }
        }
        '#' => {
            // Alternative syntax for internal commands (same as backtick)
            let result = super::parser::execute_command(engine, &source_value);
            match result {
                TfCommandResult::Success(Some(msg)) => {
                    msg.lines()
                        .map(|line| format!("{}{}{}", prefix, line, suffix))
                        .collect()
                }
                TfCommandResult::Success(None) => {
                    vec![]
                }
                TfCommandResult::Error(e) => {
                    return TfCommandResult::Error(format!("Command '{}' failed: {}", source_value, e));
                }
                _ => {
                    vec![]
                }
            }
        }
        _ => unreachable!(),
    };

    if lines.is_empty() {
        return TfCommandResult::Success(Some("(no output)".to_string()));
    }

    TfCommandResult::Quote {
        lines,
        disposition,
        world,
        delay_secs,
    }
}

/// #recall [-<count>] <pattern> - Search output history
/// Examples:
///   #recall *combat*     - Show all lines matching *combat*
///   #recall -10 *combat* - Show last 10 lines matching *combat*
/// Parse a time string like "1:30" or "1:30:45" into seconds
fn parse_time_to_seconds(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        2 => {
            // hours:minutes
            let hours: f64 = parts[0].parse().ok()?;
            let minutes: f64 = parts[1].parse().ok()?;
            Some(hours * 3600.0 + minutes * 60.0)
        }
        3 => {
            // hours:minutes:seconds
            let hours: f64 = parts[0].parse().ok()?;
            let minutes: f64 = parts[1].parse().ok()?;
            let seconds: f64 = parts[2].parse().ok()?;
            Some(hours * 3600.0 + minutes * 60.0 + seconds)
        }
        _ => None,
    }
}

/// Check if a string looks like a time format (contains colon with digits)
fn looks_like_time(s: &str) -> bool {
    s.contains(':') && s.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.')
}

pub fn cmd_recall(args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Success(Some(
            "Usage: /recall [-wworld] [-ligv] [-t[format]] [-aattrs] [-mstyle] [-An] [-Bn] [-Cn] [#]range [pattern]".to_string()
        ));
    }

    let mut opts = RecallOptions::default();
    let mut remaining = args;
    let mut _saw_hash = false;

    // Parse options (start with -)
    while !remaining.is_empty() {
        let trimmed = remaining.trim_start();
        if trimmed.is_empty() {
            break;
        }

        // Check for # (show line numbers) - must be last option before range
        if trimmed.starts_with('#') && !trimmed.starts_with("#recall") {
            _saw_hash = true;
            opts.show_line_numbers = true;
            remaining = &trimmed[1..];
            break; // # must be last option
        }

        // Check for options starting with -
        if !trimmed.starts_with('-') {
            remaining = trimmed;
            break;
        }

        // Find end of this option (space or end)
        let opt_end = trimmed[1..].find(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(trimmed.len());
        let opt = &trimmed[..opt_end];
        remaining = &trimmed[opt_end..];

        // Parse the option
        let opt_chars: Vec<char> = opt[1..].chars().collect();
        if opt_chars.is_empty() {
            // Just "-" alone, this is the start of range like "- -4"
            remaining = trimmed;
            break;
        }

        let mut i = 0;
        while i < opt_chars.len() {
            match opt_chars[i] {
                'w' => {
                    // -w or -wworld
                    if i + 1 < opt_chars.len() {
                        let world: String = opt_chars[i+1..].iter().collect();
                        opts.source = RecallSource::World(world);
                        i = opt_chars.len();
                    } else {
                        opts.source = RecallSource::CurrentWorld;
                        i += 1;
                    }
                }
                'l' => {
                    opts.source = RecallSource::Local;
                    i += 1;
                }
                'g' => {
                    opts.source = RecallSource::Global;
                    i += 1;
                }
                'i' => {
                    opts.source = RecallSource::Input;
                    i += 1;
                }
                'v' => {
                    opts.inverse_match = true;
                    i += 1;
                }
                'q' => {
                    opts.quiet = true;
                    i += 1;
                }
                't' => {
                    opts.show_timestamps = true;
                    // Check for optional format
                    if i + 1 < opt_chars.len() {
                        let fmt: String = opt_chars[i+1..].iter().collect();
                        opts.timestamp_format = Some(fmt);
                        i = opt_chars.len();
                    } else {
                        i += 1;
                    }
                }
                'a' => {
                    // -aattrs - for now just support -ag (show gagged)
                    if i + 1 < opt_chars.len() && opt_chars[i+1] == 'g' {
                        opts.show_gagged = true;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                'm' => {
                    // -mstyle
                    if i + 1 < opt_chars.len() {
                        let style: String = opt_chars[i+1..].iter().collect();
                        opts.match_style = match style.to_lowercase().as_str() {
                            "simple" => RecallMatchStyle::Simple,
                            "glob" => RecallMatchStyle::Glob,
                            "regexp" | "regex" => RecallMatchStyle::Regexp,
                            _ => RecallMatchStyle::Glob,
                        };
                        i = opt_chars.len();
                    } else {
                        i += 1;
                    }
                }
                'A' => {
                    // -An context after
                    let num: String = opt_chars[i+1..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num.parse::<usize>() {
                        opts.context_after = n;
                        i += 1 + num.len();
                    } else {
                        i += 1;
                    }
                }
                'B' => {
                    // -Bn context before
                    let num: String = opt_chars[i+1..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num.parse::<usize>() {
                        opts.context_before = n;
                        i += 1 + num.len();
                    } else {
                        i += 1;
                    }
                }
                'C' => {
                    // -Cn context both
                    let num: String = opt_chars[i+1..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num.parse::<usize>() {
                        opts.context_before = n;
                        opts.context_after = n;
                        i += 1 + num.len();
                    } else {
                        i += 1;
                    }
                }
                _ => {
                    // Unknown option or might be a negative range
                    // Check if rest looks like a number (negative range like -4)
                    let rest: String = opt_chars[i..].iter().collect();
                    if rest.chars().all(|c| c.is_ascii_digit()) {
                        // This is a negative range, put it back
                        remaining = trimmed;
                        break;
                    }
                    i += 1;
                }
            }
        }
    }

    // Parse range and pattern
    let remaining = remaining.trim();

    if remaining.is_empty() {
        // No range or pattern, recall all
        opts.range = RecallRange::All;
        return TfCommandResult::Recall(opts);
    }

    // Find where range ends and pattern begins
    // Range formats: /x, x, x-y, -y, x-, or time formats
    let mut range_end = 0;
    let chars: Vec<char> = remaining.chars().collect();

    if chars.first() == Some(&'/') {
        // /x format - last x matching lines
        let num_str: String = chars[1..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num_str.parse::<usize>() {
            opts.range = RecallRange::LastMatching(n);
            range_end = 1 + num_str.len();
        }
    } else if chars.first() == Some(&'-') && chars.len() > 1 {
        // Could be: - -y (with space) or just part of options we already parsed
        // Look for the number after the dash
        let rest: String = chars[1..].iter().collect();
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == ':' || *c == '.').collect();
        if !num_str.is_empty() {
            if looks_like_time(&num_str) {
                if let Some(secs) = parse_time_to_seconds(&num_str) {
                    opts.range = RecallRange::TimePeriod(secs);
                    range_end = 1 + num_str.len();
                }
            } else if let Ok(n) = num_str.parse::<usize>() {
                opts.range = RecallRange::Previous(n);
                range_end = 1 + num_str.len();
            }
        }
    } else {
        // Parse as: x, x-y, x-, or time
        let range_str: String = chars.iter().take_while(|c|
            c.is_ascii_digit() || **c == '-' || **c == ':' || **c == '.'
        ).collect();

        if !range_str.is_empty() {
            range_end = range_str.len();

            if range_str.contains('-') && !range_str.starts_with('-') {
                // x-y or x- format
                let parts: Vec<&str> = range_str.splitn(2, '-').collect();
                if parts.len() == 2 {
                    if parts[1].is_empty() {
                        // x- format (after x)
                        if looks_like_time(parts[0]) {
                            if let Some(secs) = parse_time_to_seconds(parts[0]) {
                                opts.range = RecallRange::TimeRange(secs, 0.0);
                            }
                        } else if let Ok(x) = parts[0].parse::<usize>() {
                            opts.range = RecallRange::After(x);
                        }
                    } else {
                        // x-y format
                        if looks_like_time(parts[0]) && looks_like_time(parts[1]) {
                            if let (Some(start), Some(end)) = (parse_time_to_seconds(parts[0]), parse_time_to_seconds(parts[1])) {
                                opts.range = RecallRange::TimeRange(start, end);
                            }
                        } else if let (Ok(x), Ok(y)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
                            opts.range = RecallRange::Range(x, y);
                        }
                    }
                }
            } else if looks_like_time(&range_str) {
                // Time period
                if let Some(secs) = parse_time_to_seconds(&range_str) {
                    opts.range = RecallRange::TimePeriod(secs);
                }
            } else if let Ok(n) = range_str.parse::<usize>() {
                // Plain number - last n lines
                opts.range = RecallRange::Last(n);
            }
        }
    }

    // Everything after range is the pattern
    let pattern = remaining[range_end..].trim();
    if !pattern.is_empty() {
        opts.pattern = Some(pattern.to_string());
    }

    TfCommandResult::Recall(opts)
}

/// #gag pattern - Add a gag pattern (suppress matching output)
/// Note: Returns a message for main.rs integration
pub fn cmd_gag(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: #gag pattern".to_string());
    }

    // Create a macro with gag attribute
    let gag_name = format!("__gag_{}", engine.next_macro_sequence);
    let macro_def = super::TfMacro {
        name: gag_name,
        body: String::new(),
        trigger: Some(super::TfTrigger {
            pattern: pattern.to_string(),
            match_mode: super::TfMatchMode::Glob,
            compiled: regex::Regex::new(&super::macros::glob_to_regex(pattern)).ok(),
        }),
        attributes: super::TfAttributes {
            gag: true,
            ..Default::default()
        },
        ..Default::default()
    };

    engine.add_macro(macro_def);
    TfCommandResult::Success(Some(format!("Gagging '{}'", pattern)))
}

/// #ungag pattern - Remove a gag pattern
pub fn cmd_ungag(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: #ungag pattern".to_string());
    }

    let before = engine.macros.len();
    engine.macros.retain(|m| {
        if let Some(ref trigger) = m.trigger {
            !(m.attributes.gag && trigger.pattern == pattern)
        } else {
            true
        }
    });

    let removed = before - engine.macros.len();
    if removed > 0 {
        TfCommandResult::Success(Some(format!("Removed {} gag(s)", removed)))
    } else {
        TfCommandResult::Error(format!("Gag pattern '{}' not found", pattern))
    }
}

/// Expand ~ and search TFPATH/TFLIBDIR for a file
fn resolve_file_path(engine: &TfEngine, filename: &str) -> Option<String> {
    // Expand ~ to home directory
    let expanded = if filename.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy();
            if filename == "~" {
                home_str.to_string()
            } else if let Some(rest) = filename.strip_prefix("~/") {
                format!("{}/{}", home_str, rest)
            } else {
                filename.to_string()
            }
        } else {
            filename.to_string()
        }
    } else {
        filename.to_string()
    };

    // If absolute path, just check if it exists
    if expanded.starts_with('/') {
        let path = Path::new(&expanded);
        if path.exists() {
            return Some(expanded);
        }
        return None;
    }

    // Search order for relative paths:
    // 1. Current directory (from #lcd or actual cwd)
    // 2. Directories in TFPATH
    // 3. TFLIBDIR

    let search_dirs: Vec<String> = {
        let mut dirs = Vec::new();

        // Current directory
        if let Some(ref cd) = engine.current_dir {
            dirs.push(cd.clone());
        } else if let Ok(cwd) = std::env::current_dir() {
            dirs.push(cwd.display().to_string());
        }

        // TFPATH (colon-separated list of directories)
        if let Ok(tfpath) = std::env::var("TFPATH") {
            for dir in tfpath.split(':') {
                if !dir.is_empty() {
                    dirs.push(dir.to_string());
                }
            }
        }

        // TFLIBDIR (fallback if TFPATH not set)
        if let Ok(tflibdir) = std::env::var("TFLIBDIR") {
            if !tflibdir.is_empty() {
                dirs.push(tflibdir);
            }
        }

        dirs
    };

    // Search each directory
    for dir in search_dirs {
        let full_path = format!("{}/{}", dir, expanded);
        if Path::new(&full_path).exists() {
            return Some(full_path);
        }
    }

    None
}

/// Internal load implementation used by both #load and #require
fn load_file_internal(engine: &mut TfEngine, filename: &str, quiet: bool) -> TfCommandResult {
    // Resolve the file path
    let resolved = match resolve_file_path(engine, filename) {
        Some(p) => p,
        None => return TfCommandResult::Error(format!("Cannot find file: {}", filename)),
    };

    // Open the file
    let file = match fs::File::open(&resolved) {
        Ok(f) => f,
        Err(e) => return TfCommandResult::Error(format!("Cannot open '{}': {}", resolved, e)),
    };

    // Track that we're loading this file (for nested loads)
    engine.loading_files.push(resolved.clone());

    // Show loading message unless quiet
    let mut results = Vec::new();
    if !quiet {
        results.push(TfCommandResult::Success(Some(format!("Loading commands from {}", resolved))));
    }

    let reader = BufReader::new(file);
    let mut line_num = 0;
    let mut continued_line = String::new();
    let mut exit_early = false;

    for line in reader.lines() {
        line_num += 1;
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                results.push(TfCommandResult::Error(format!("Line {}: {}", line_num, e)));
                continue;
            }
        };

        // Strip leading whitespace
        let trimmed = line.trim_start();

        // Check if this is a comment line (starts with ; or is just # or # followed by space)
        let is_comment = trimmed.starts_with(';')
            || trimmed == "#"
            || trimmed.starts_with("# ");

        // If this is a comment line, skip it entirely (even during line continuation)
        // The continuation just continues to the next non-comment line
        if is_comment {
            // If the comment ends with \, it's still a continuation but we skip the comment content
            if trimmed.ends_with('\\') && !trimmed.ends_with("%\\") {
                // Don't append the comment, but continue looking for more lines
                continue;
            }
            // Regular comment - just skip
            continue;
        }

        // Handle line continuation
        if trimmed.ends_with('\\') && !trimmed.ends_with("%\\") {
            // Line continues - append without the backslash
            continued_line.push_str(&trimmed[..trimmed.len() - 1]);
            continue;
        }

        // Build the complete line
        let complete_line = if !continued_line.is_empty() {
            let mut full = std::mem::take(&mut continued_line);
            full.push_str(trimmed);
            // Replace %\ with just \ (escaped backslash for line continuation)
            // Note: %; is NOT replaced here - it's handled during macro execution
            full.replace("%\\", "\\")
        } else {
            trimmed.replace("%\\", "\\")
        };

        let trimmed = complete_line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Execute the line
        let result = if trimmed.starts_with('#') || trimmed.starts_with('/') {
            super::parser::execute_command(engine, trimmed)
        } else {
            // Non-command lines are sent to the MUD in TF, but we ignore them in Clay
            continue;
        };

        match &result {
            TfCommandResult::Error(e) => {
                results.push(TfCommandResult::Error(format!("{}:{}: {}", resolved, line_num, e)));
            }
            TfCommandResult::ExitLoad => {
                // #exit was called - stop loading this file
                exit_early = true;
                break;
            }
            _ => results.push(result),
        }
    }

    // Remove this file from the loading stack
    engine.loading_files.pop();

    // Fire LOAD hook (even for early exit)
    let hook_results = super::hooks::fire_hook(engine, super::TfHookEvent::Load);
    results.extend(hook_results);

    // Collect errors for detailed output
    let errors: Vec<String> = results.iter()
        .filter_map(|r| match r {
            TfCommandResult::Error(e) => Some(e.clone()),
            _ => None,
        })
        .collect();

    if !errors.is_empty() {
        // Build multi-line output with summary and error details
        let mut output = format!("Loaded '{}' with {} error(s)", resolved, errors.len());
        for error in &errors {
            output.push_str(&format!("\n   {}", error));
        }
        TfCommandResult::Error(output)
    } else if exit_early {
        // Success with early exit - no output (silent)
        TfCommandResult::Success(None)
    } else {
        // Success - no completion output (silent like TF)
        TfCommandResult::Success(None)
    }
}

/// #load [-q] filename - Load and execute a TF script file
///
/// Options:
///   -q  Quiet mode - don't echo "Loading commands from..." message
///
/// The file may contain TF commands starting with # or /.
/// Blank lines and lines beginning with ';' or single '#' are ignored.
/// Lines ending in '\' continue on the next line (use %\ for literal backslash).
/// Use #exit to abort loading early.
pub fn cmd_load(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: #load [-q] filename".to_string());
    }

    // Parse options
    let mut quiet = false;
    let mut filename = args;

    if args.starts_with("-q") {
        quiet = true;
        filename = args[2..].trim_start();
        if filename.is_empty() {
            return TfCommandResult::Error("Usage: #load [-q] filename".to_string());
        }
    }

    load_file_internal(engine, filename, quiet)
}

/// #require [-q] filename - Load file only if not already loaded via #loaded
///
/// Same as #load, but if the file has already registered a token via #loaded,
/// the file will not be read again (but the LOAD hook is still called).
pub fn cmd_require(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: #require [-q] filename".to_string());
    }

    // Parse options
    let mut quiet = false;
    let mut filename = args;

    if args.starts_with("-q") {
        quiet = true;
        filename = args[2..].trim_start();
        if filename.is_empty() {
            return TfCommandResult::Error("Usage: #require [-q] filename".to_string());
        }
    }

    // Note: We don't check loaded_tokens here - that's done by #loaded inside the file.
    // #require just calls #load; the difference is that files designed for #require
    // will have #loaded as their first command, which will abort if already loaded.
    load_file_internal(engine, filename, quiet)
}

/// #loaded token - Mark this file as loaded (for use with #require)
///
/// Should be the first command in a file designed for #require.
/// If the token has already been registered, aborts the file load and returns success.
/// Token should be unique (file's full path is recommended).
pub fn cmd_loaded(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let token = args.trim();

    if token.is_empty() {
        return TfCommandResult::Error("Usage: #loaded token".to_string());
    }

    // Check if already loaded
    if engine.loaded_tokens.contains(token) {
        // Already loaded - abort this file load
        return TfCommandResult::ExitLoad;
    }

    // Register the token
    engine.loaded_tokens.insert(token.to_string());
    TfCommandResult::Success(None)
}

/// #exit - Abort loading the current file early
///
/// When called during #load or #require, stops reading the current file.
/// When called outside of file loading, this is equivalent to /quit.
pub fn cmd_exit(engine: &TfEngine) -> TfCommandResult {
    if engine.loading_files.is_empty() {
        // Not loading a file - quit the application
        TfCommandResult::ClayCommand("/quit".to_string())
    } else {
        // Loading a file - abort early
        TfCommandResult::ExitLoad
    }
}

/// #save filename - Save macros to a file
pub fn cmd_save(engine: &TfEngine, args: &str) -> TfCommandResult {
    let filename = args.trim();

    if filename.is_empty() {
        return TfCommandResult::Error("Usage: #save filename".to_string());
    }

    // Expand ~ to home directory
    let expanded = if filename.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy();
            if filename == "~" {
                return TfCommandResult::Error("Cannot save to home directory".to_string());
            } else if let Some(rest) = filename.strip_prefix("~/") {
                format!("{}/{}", home_str, rest)
            } else {
                filename.to_string()
            }
        } else {
            filename.to_string()
        }
    } else {
        filename.to_string()
    };

    let mut output = String::new();

    // Save global variables
    output.push_str(";; TinyFugue script generated by Clay\n");
    output.push_str(";; Variables\n");
    for (name, value) in &engine.global_vars {
        output.push_str(&format!("#set {} {}\n", name, value.to_string_value()));
    }

    // Save macros
    output.push_str("\n;; Macros\n");
    for macro_def in &engine.macros {
        // Skip internal macros
        if macro_def.name.starts_with("__") {
            continue;
        }

        let mut def_line = String::from("#def ");

        // Add flags
        if let Some(ref trigger) = macro_def.trigger {
            if !trigger.pattern.is_empty() {
                def_line.push_str(&format!("-t\"{}\" ", trigger.pattern));
                if trigger.match_mode != super::TfMatchMode::Glob {
                    def_line.push_str(&format!("-m{:?} ", trigger.match_mode).to_lowercase());
                }
            }
        }

        if macro_def.priority != 0 {
            def_line.push_str(&format!("-p{} ", macro_def.priority));
        }
        if macro_def.fall_through {
            def_line.push_str("-F ");
        }
        if let Some(n) = macro_def.one_shot {
            if n == 1 {
                def_line.push_str("-1 ");
            } else {
                def_line.push_str(&format!("-n{} ", n));
            }
        }
        if let Some(ref hook) = macro_def.hook {
            def_line.push_str(&format!("-h{:?} ", hook));
        }
        if let Some(ref keys) = macro_def.keybinding {
            def_line.push_str(&format!("-b\"{}\" ", keys));
        }
        if let Some(ref world) = macro_def.world {
            def_line.push_str(&format!("-w\"{}\" ", world));
        }
        if let Some(ref cond) = macro_def.condition {
            def_line.push_str(&format!("-E\"{}\" ", cond));
        }
        if let Some(prob) = macro_def.probability {
            def_line.push_str(&format!("-c{} ", prob));
        }

        // Add attributes
        let mut attrs: Vec<String> = Vec::new();
        if macro_def.attributes.gag { attrs.push("gag".to_string()); }
        if macro_def.attributes.bold { attrs.push("bold".to_string()); }
        if macro_def.attributes.underline { attrs.push("underline".to_string()); }
        if macro_def.attributes.reverse { attrs.push("reverse".to_string()); }
        if macro_def.attributes.flash { attrs.push("flash".to_string()); }
        if macro_def.attributes.dim { attrs.push("dim".to_string()); }
        if macro_def.attributes.bell { attrs.push("bell".to_string()); }
        if let Some(ref color) = macro_def.attributes.hilite {
            attrs.push(format!("hilite:{}", color));
        }
        if !attrs.is_empty() {
            def_line.push_str(&format!("-a{} ", attrs.join(",")));
        }

        def_line.push_str(&format!("{} = {}\n", macro_def.name, macro_def.body));
        output.push_str(&def_line);
    }

    // Save keybindings
    output.push_str("\n;; Keybindings\n");
    for (key, cmd) in &engine.keybindings {
        output.push_str(&format!("#bind {} = {}\n", key, cmd));
    }

    // Write to file
    match fs::write(&expanded, output) {
        Ok(()) => TfCommandResult::Success(Some(format!("Saved to '{}'", expanded))),
        Err(e) => TfCommandResult::Error(format!("Cannot write '{}': {}", expanded, e)),
    }
}

/// #log [filename] - Toggle logging to file
/// Note: Actual logging needs main.rs integration, this just returns a message
pub fn cmd_log(args: &str) -> TfCommandResult {
    let filename = args.trim();

    if filename.is_empty() {
        TfCommandResult::Success(Some("Usage: #log filename - Toggle logging (requires main.rs integration)".to_string()))
    } else {
        TfCommandResult::Success(Some(format!("Log '{}' - requires main.rs integration", filename)))
    }
}

/// Parse a TF time string into a Duration
/// Formats: "S" (seconds, supports decimals), "M:S", "H:M:S"
/// Leading '-' is stripped (TF convention for repeat intervals)
pub fn parse_tf_time(s: &str) -> Option<Duration> {
    let s = s.strip_prefix('-').unwrap_or(s);
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        1 => {
            // Just seconds (supports decimals)
            let secs: f64 = parts[0].parse().ok()?;
            if secs < 0.0 { return None; }
            Some(Duration::from_secs_f64(secs))
        }
        2 => {
            // M:S
            let mins: f64 = parts[0].parse().ok()?;
            let secs: f64 = parts[1].parse().ok()?;
            if mins < 0.0 || secs < 0.0 { return None; }
            Some(Duration::from_secs_f64(mins * 60.0 + secs))
        }
        3 => {
            // H:M:S
            let hours: f64 = parts[0].parse().ok()?;
            let mins: f64 = parts[1].parse().ok()?;
            let secs: f64 = parts[2].parse().ok()?;
            if hours < 0.0 || mins < 0.0 || secs < 0.0 { return None; }
            Some(Duration::from_secs_f64(hours * 3600.0 + mins * 60.0 + secs))
        }
        _ => None,
    }
}

/// #repeat [-w[world]] [-n] {[-time]|-S|-P} count command
pub fn cmd_repeat(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error(
            "Usage: #repeat [-w[world]] [-n] {[-time]|-S|-P} count command".to_string()
        );
    }

    let mut world: Option<String> = None;
    let mut no_initial_delay = false;
    let mut synchronous = false;
    let mut on_prompt = false;
    let mut interval: Option<Duration> = None;
    let mut priority: i32 = 0;
    let mut remaining = args;

    // Parse flags
    loop {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        if remaining.starts_with("-w") {
            // -w or -wworld
            let rest = &remaining[2..];
            if rest.starts_with(char::is_whitespace) || rest.is_empty() {
                // -w with no world name — current world
                world = Some(String::new());
                remaining = rest.trim_start();
            } else {
                // -wworld
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                world = Some(rest[..end].to_string());
                remaining = &rest[end..];
            }
            continue;
        }

        if remaining.starts_with("-n") && (remaining.len() == 2 || remaining[2..].starts_with(char::is_whitespace)) {
            no_initial_delay = true;
            remaining = &remaining[2..];
            continue;
        }

        if remaining.starts_with("-S") && (remaining.len() == 2 || remaining[2..].starts_with(char::is_whitespace)) {
            synchronous = true;
            remaining = &remaining[2..];
            continue;
        }

        if remaining.starts_with("-P") && (remaining.len() == 2 || remaining[2..].starts_with(char::is_whitespace)) {
            on_prompt = true;
            remaining = &remaining[2..];
            continue;
        }

        // Check for -p priority
        if remaining.starts_with("-p") {
            let rest = &remaining[2..];
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            if let Ok(p) = rest[..end].parse::<i32>() {
                priority = p;
                remaining = &rest[end..];
                continue;
            }
        }

        // Check for -time (e.g. -30, -0:30, -1:0:0)
        if remaining.starts_with('-') {
            let rest = &remaining[1..];
            let time_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let time_str = &rest[..time_end];
            // Must start with a digit to be a time value (not another flag)
            if time_str.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                if let Some(dur) = parse_tf_time(time_str) {
                    interval = Some(dur);
                    remaining = &rest[time_end..];
                    continue;
                }
            }
        }

        break;
    }

    remaining = remaining.trim_start();

    // Parse count: integer or "i" for infinite
    let count_end = remaining.find(char::is_whitespace).unwrap_or(remaining.len());
    let count_str = &remaining[..count_end];
    let count: Option<u32> = if count_str.eq_ignore_ascii_case("i") {
        None // infinite
    } else if let Ok(n) = count_str.parse::<u32>() {
        if n == 0 {
            return TfCommandResult::Error("#repeat: count must be > 0".to_string());
        }
        Some(n)
    } else {
        return TfCommandResult::Error(format!("#repeat: invalid count '{}'", count_str));
    };
    remaining = remaining[count_end..].trim_start();

    // Parse command (rest of args)
    let command = remaining.to_string();
    if command.is_empty() {
        return TfCommandResult::Error("#repeat: no command specified".to_string());
    }

    // Synchronous mode: execute all iterations immediately
    if synchronous {
        let iterations = count.unwrap_or(1);
        let mut last_result = TfCommandResult::Success(None);
        for _ in 0..iterations {
            last_result = engine.execute(&command);
        }
        return last_result;
    }

    // Need an interval for async mode
    let interval = interval.unwrap_or(Duration::from_secs(1));

    // Create process
    let id = engine.next_process_id;
    engine.next_process_id += 1;

    // Always run first iteration immediately, then wait interval between subsequent runs
    // The -n flag is now a no-op (kept for backwards compatibility)
    let _ = no_initial_delay;
    let next_run = Instant::now();

    let process = TfProcess {
        id,
        command,
        interval,
        count,
        remaining: count,
        next_run,
        world,
        synchronous: false,
        on_prompt,
        priority,
    };

    TfCommandResult::RepeatProcess(process)
}

/// #ps - List background processes
pub fn cmd_ps(engine: &TfEngine) -> TfCommandResult {
    if engine.processes.is_empty() {
        return TfCommandResult::Success(Some("No background processes.".to_string()));
    }

    let mut lines = vec![format!("{:<6} {:<12} {:<10} {}", "PID", "INTERVAL", "REMAINING", "COMMAND")];
    for p in &engine.processes {
        let interval_str = format_duration(p.interval);
        let remaining_str = match p.remaining {
            Some(r) => r.to_string(),
            None => "inf".to_string(),
        };
        lines.push(format!("{:<6} {:<12} {:<10} {}", p.id, interval_str, remaining_str, p.command));
    }
    TfCommandResult::Success(Some(lines.join("\n")))
}

/// Format a Duration for display
fn format_duration(d: Duration) -> String {
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
        let secs = (total_secs % 60.0) as u64;
        format!("{}h{}m{}s", hours, mins, secs)
    }
}

/// #kill pid - Kill background process
pub fn cmd_kill(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pid_str = args.trim();
    if pid_str.is_empty() {
        return TfCommandResult::Error("Usage: #kill pid".to_string());
    }

    if let Ok(pid) = pid_str.parse::<u32>() {
        let before = engine.processes.len();
        engine.processes.retain(|p| p.id != pid);
        if engine.processes.len() < before {
            TfCommandResult::Success(Some(format!("Process {} killed.", pid)))
        } else {
            TfCommandResult::Error(format!("Process {} not found.", pid))
        }
    } else {
        TfCommandResult::Error(format!("Invalid pid: {}", pid_str))
    }
}

/// Convert glob pattern to regex (re-exported from macros for use here)
pub use super::macros::glob_to_regex;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_beep() {
        let result = cmd_beep();
        assert!(matches!(result, TfCommandResult::Success(Some(s)) if s == "\x07"));
    }

    #[test]
    fn test_cmd_time() {
        let result = cmd_time("");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));

        let result = cmd_time("%s");
        if let TfCommandResult::Success(Some(s)) = result {
            assert!(s.parse::<u64>().is_ok());
        }
    }

    #[test]
    fn test_cmd_lcd() {
        let mut engine = TfEngine::new();

        // Show current directory
        let result = cmd_lcd(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));

        // Change to /tmp (should exist on most systems)
        let result = cmd_lcd(&mut engine, "/tmp");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert_eq!(engine.current_dir, Some("/tmp".to_string()));

        // Try non-existent directory
        let result = cmd_lcd(&mut engine, "/nonexistent_dir_12345");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_quote() {
        let mut engine = TfEngine::new();

        // Test literal text (no source specifier)
        let result = cmd_quote(&mut engine, "hello world");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines, vec!["hello world"]);
                assert_eq!(disposition, QuoteDisposition::Send);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test empty args
        let result = cmd_quote(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Error(_)));

        // Test with -decho option
        let result = cmd_quote(&mut engine, "-decho test message");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines, vec!["test message"]);
                assert_eq!(disposition, QuoteDisposition::Echo);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with -wworld option
        let result = cmd_quote(&mut engine, "-wmyworld hello");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines, vec!["hello"]);
                assert_eq!(disposition, QuoteDisposition::Send);
                assert_eq!(world, Some("myworld".to_string()));
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }
    }

    #[test]
    fn test_cmd_sh() {
        let result = cmd_sh("echo hello");
        if let TfCommandResult::Success(Some(s)) = result {
            assert!(s.contains("hello"));
        }

        let result = cmd_sh("");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_quote_file() {
        use std::io::Write;
        let mut engine = TfEngine::new();

        // Create a temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("clay_quote_test.txt");
        {
            let mut file = std::fs::File::create(&temp_file).unwrap();
            writeln!(file, "line one").unwrap();
            writeln!(file, "line two").unwrap();
            writeln!(file, "line three").unwrap();
        }

        // Test reading from file
        let path = temp_file.to_string_lossy();
        let result = cmd_quote(&mut engine, &format!("'\"{}\"", path));
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(lines[0], "line one");
                assert_eq!(lines[1], "line two");
                assert_eq!(lines[2], "line three");
                assert_eq!(disposition, QuoteDisposition::Send);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with prefix
        let result = cmd_quote(&mut engine, &format!("say '\"{}\"", path));
        match result {
            TfCommandResult::Quote { lines, disposition, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(lines[0], "say line one");
                assert_eq!(lines[1], "say line two");
                assert_eq!(lines[2], "say line three");
                assert_eq!(disposition, QuoteDisposition::Send);
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with -decho option
        let result = cmd_quote(&mut engine, &format!("-decho '\"{}\"", path));
        match result {
            TfCommandResult::Quote { lines, disposition, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(disposition, QuoteDisposition::Echo);
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Clean up
        let _ = std::fs::remove_file(&temp_file);
    }

    #[test]
    fn test_cmd_quote_shell() {
        let mut engine = TfEngine::new();

        // Test reading from shell command (using ! prefix)
        let result = cmd_quote(&mut engine, "!\"echo hello\"");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "hello");
                assert_eq!(disposition, QuoteDisposition::Send);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with prefix
        let result = cmd_quote(&mut engine, "say !\"echo world\"");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "say world");
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }
    }

    #[test]
    fn test_cmd_quote_internal() {
        let mut engine = TfEngine::new();

        // Test reading from internal command (using ` prefix)
        // #version returns a success message
        let result = cmd_quote(&mut engine, "`\"#version\"");
        match result {
            TfCommandResult::Quote { lines, disposition, .. } => {
                assert!(!lines.is_empty());
                assert!(lines[0].contains("Clay") || lines[0].contains("TF"));
                assert_eq!(disposition, QuoteDisposition::Send);
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with prefix
        let result = cmd_quote(&mut engine, "think `\"#version\"");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert!(!lines.is_empty());
                assert!(lines[0].starts_with("think "));
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test /version (Clay command) is also capturable
        let result = cmd_quote(&mut engine, "think `\"/version\"");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert!(!lines.is_empty());
                assert!(lines[0].starts_with("think Clay v"));
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }
    }

    #[test]
    fn test_cmd_gag_ungag() {
        let mut engine = TfEngine::new();

        // Add a gag
        let result = cmd_gag(&mut engine, "spam*");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert!(engine.macros.iter().any(|m| m.attributes.gag));

        // Remove the gag
        let result = cmd_ungag(&mut engine, "spam*");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert!(!engine.macros.iter().any(|m| m.attributes.gag && m.trigger.as_ref().map(|t| t.pattern == "spam*").unwrap_or(false)));
    }

    #[test]
    fn test_load_crypt_tf() {
        let mut engine = TfEngine::new();

        // Load crypt.tf from the project root
        let result = cmd_load(&mut engine, "crypt.tf");

        // Check loading succeeded (or at least didn't hard error)
        // Note: Some commands like #passwd might produce errors since they try to execute
        match &result {
            TfCommandResult::Success(_) => {
                // Good - loaded successfully
            }
            TfCommandResult::Error(e) => {
                // Check it's not a file-not-found error
                assert!(!e.contains("Cannot open"), "Failed to open crypt.tf: {}", e);
                // Other errors might be OK (e.g., from executing #passwd)
            }
            _ => {}
        }

        // Verify macros were defined
        let macro_names: Vec<&str> = engine.macros.iter().map(|m| m.name.as_str()).collect();

        // Check that key macros exist
        assert!(macro_names.contains(&"random"), "random macro not defined");
        assert!(macro_names.contains(&"passwd"), "passwd macro not defined");
        assert!(macro_names.contains(&"encrypt"), "encrypt macro not defined");
        assert!(macro_names.contains(&"decrypt"), "decrypt macro not defined");
        assert!(macro_names.contains(&"makeprintable"), "makeprintable macro not defined");
        assert!(macro_names.contains(&"e"), "e macro not defined");
        assert!(macro_names.contains(&"p"), "p macro not defined");
        assert!(macro_names.contains(&"listen_mush"), "listen_mush macro not defined");

        // Verify that %R was preserved in the random macro body
        let random_macro = engine.macros.iter().find(|m| m.name == "random").unwrap();
        assert!(random_macro.body.contains("%R"),
            "random macro body should contain %R, got: {}", random_macro.body);

        // Verify the "e" macro body contains say command with command substitution
        let e_macro = engine.macros.iter().find(|m| m.name == "e").unwrap();
        assert!(e_macro.body.contains("say"),
            "e macro body should contain 'say', got: {}", e_macro.body);
        assert!(e_macro.body.contains("\\$("),
            "e macro body should have \\$( for command substitution, got: {}", e_macro.body);

        // Verify listen_mush has a trigger pattern
        let listen_macro = engine.macros.iter().find(|m| m.name == "listen_mush").unwrap();
        assert!(listen_macro.trigger.is_some(), "listen_mush should have a trigger");
        assert_eq!(listen_macro.priority, 5000, "listen_mush should have priority 5000");

    }
}
