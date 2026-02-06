use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::tf;
use crate::util::strip_ansi_codes;
use crate::OutputLine;

/// Match type for action patterns
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum MatchType {
    #[default]
    Regexp,     // Regular expression matching
    Wildcard,   // Glob/wildcard matching (* and ?)
}

impl MatchType {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchType::Regexp => "Regexp",
            MatchType::Wildcard => "Wildcard",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "wildcard" => MatchType::Wildcard,
            _ => MatchType::Regexp,
        }
    }
}

/// Helper function for serde default to return true
fn default_enabled() -> bool { true }

/// User-defined action/trigger
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub name: String,           // Unique name (also used as /name command if no pattern)
    pub world: String,          // World name to match (empty = all worlds)
    #[serde(default)]
    pub match_type: MatchType,  // How to interpret pattern (regexp or wildcard)
    pub pattern: String,        // Pattern to match output (empty = manual /name only)
    pub command: String,        // Command(s) to execute, semicolon-separated
    #[serde(default)]
    pub owner: Option<String>,  // Username who owns this action (multiuser mode)
    #[serde(default = "default_enabled")]
    pub enabled: bool,          // If false, action will not fire
    #[serde(default)]
    pub startup: bool,          // If true, run commands on Clay startup
}

impl Default for Action {
    fn default() -> Self {
        Self {
            name: String::new(),
            world: String::new(),
            match_type: MatchType::Regexp,
            pattern: String::new(),
            command: String::new(),
            owner: None,
            enabled: true,
            startup: false,
        }
    }
}

impl Action {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Split action command string by semicolons, handling escaped semicolons (\;)
pub fn split_action_commands(command: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&';') {
            // Escaped semicolon - add literal semicolon
            chars.next(); // consume the semicolon
            current.push(';');
        } else if c == ';' {
            // Command separator
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                result.push(trimmed);
            }
            current.clear();
        } else {
            current.push(c);
        }
    }

    // Don't forget the last command
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }

    result
}

/// Substitute action arguments ($1-$9, $*) in a command string
/// args_str is the space-separated arguments passed to the action
pub fn substitute_action_args(command: &str, args_str: &str) -> String {
    // Split args into words
    let args: Vec<&str> = args_str.split_whitespace().collect();

    let mut result = String::with_capacity(command.len() + args_str.len());
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            if let Some(&next) = chars.peek() {
                if next == '*' {
                    // $* - all arguments
                    chars.next();
                    result.push_str(args_str);
                } else if next.is_ascii_digit() && next != '0' {
                    // $1-$9
                    chars.next();
                    let idx = (next as usize) - ('1' as usize);
                    if idx < args.len() {
                        result.push_str(args[idx]);
                    }
                    // If arg doesn't exist, substitute with nothing
                } else {
                    // Not a substitution pattern, keep the $
                    result.push(c);
                }
            } else {
                // $ at end of string
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Substitute pattern captures ($0-$9) in a command string
/// $0 is the entire match, $1-$9 are capture groups
/// captures[0] is the full match, captures[1..] are the groups
pub fn substitute_pattern_captures(command: &str, captures: &[&str]) -> String {
    let mut result = String::with_capacity(command.len() * 2);
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            if let Some(&next) = chars.peek() {
                if next.is_ascii_digit() {
                    // $0-$9
                    chars.next();
                    let idx = (next as usize) - ('0' as usize);
                    if idx < captures.len() {
                        result.push_str(captures[idx]);
                    }
                    // If capture doesn't exist, substitute with nothing
                } else if next == '*' {
                    // $* - keep as-is for manual arg substitution later
                    result.push(c);
                } else {
                    // Not a substitution pattern, keep the $
                    result.push(c);
                }
            } else {
                // $ at end of string
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Result of checking action triggers on a line
pub struct ActionTriggerResult {
    pub should_gag: bool,           // If true, suppress the line from output
    pub commands: Vec<String>,      // Commands to execute
    pub highlight_color: Option<String>, // If Some, highlight the line with this color
}

/// Convert a wildcard pattern (* and ?) to a regex pattern
/// Supports \* and \? to match literal asterisk and question mark
/// Normalizes quotes: any double quote matches all double quote variants,
/// any single quote matches all single quote variants
/// Each * and ? becomes a capture group for $1, $2, etc. substitution
pub fn wildcard_to_regex(pattern: &str) -> String {
    // Wildcard patterns must match the entire line (anchored at start and end)
    let mut regex = String::with_capacity(pattern.len() * 2 + 2);
    regex.push('^');
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
            // Wildcards become capture groups for $1, $2, etc.
            '*' => regex.push_str("(.*)"),
            '?' => regex.push_str("(.)"),
            // Normalize double quotes: " (U+0022), \u{201C} (U+201C), \u{201D} (U+201D)
            '"' | '\u{201C}' | '\u{201D}' => {
                regex.push_str("[\"\u{201C}\u{201D}]");
            }
            // Normalize single quotes: ' (U+0027), \u{2018} (U+2018), \u{2019} (U+2019)
            '\'' | '\u{2018}' | '\u{2019}' => {
                regex.push_str("['\u{2018}\u{2019}]");
            }
            // Escape regex special characters
            '.' | '+' | '^' | '$' | '|' | '(' | ')' | '[' | ']' | '{' | '}' => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
    }
    regex.push('$');
    regex
}

/// Execute recall command with options
/// Returns (matches, header_message) - matches is the list of matching lines
pub fn execute_recall(opts: &tf::RecallOptions, output_lines: &[OutputLine]) -> (Vec<String>, Option<String>) {
    use tf::{RecallMatchStyle, RecallRange};

    // Build regex from pattern based on match style
    let regex = opts.pattern.as_ref().map(|pattern| {
        let regex_pattern = match opts.match_style {
            RecallMatchStyle::Simple => {
                // Simple: escape everything, just substring match
                regex::escape(pattern)
            }
            RecallMatchStyle::Glob => {
                // Glob: convert wildcards to regex
                wildcard_to_regex(pattern)
            }
            RecallMatchStyle::Regexp => {
                // Regexp: use as-is
                pattern.clone()
            }
        };
        regex::RegexBuilder::new(&regex_pattern)
            .case_insensitive(true)
            .build()
    });

    // Determine line range based on RecallRange
    let (start_idx, end_idx) = match &opts.range {
        RecallRange::All => (0, output_lines.len()),
        RecallRange::Last(n) => {
            let start = output_lines.len().saturating_sub(*n);
            (start, output_lines.len())
        }
        RecallRange::LastMatching(_n) => {
            // Will be handled specially below - get last n MATCHING lines
            (0, output_lines.len())
        }
        RecallRange::Range(x, y) => {
            // x and y are 1-based line numbers
            let start = x.saturating_sub(1).min(output_lines.len());
            let end = (*y).min(output_lines.len());
            (start, end)
        }
        RecallRange::Previous(n) => {
            // -y means yth previous line
            let idx = output_lines.len().saturating_sub(*n);
            (idx, idx + 1)
        }
        RecallRange::After(x) => {
            // x- means lines after x (1-based)
            let start = x.saturating_sub(1).min(output_lines.len());
            (start, output_lines.len())
        }
        RecallRange::TimePeriod(secs) => {
            // Lines within the last `secs` seconds
            let now = std::time::SystemTime::now();
            let cutoff = now - std::time::Duration::from_secs_f64(*secs);
            let start = output_lines.iter().position(|line| line.timestamp >= cutoff).unwrap_or(output_lines.len());
            (start, output_lines.len())
        }
        RecallRange::TimeRange(start_secs, end_secs) => {
            // Lines between two time periods
            let now = std::time::SystemTime::now();
            let start_time = now - std::time::Duration::from_secs_f64(*start_secs);
            let end_time = now - std::time::Duration::from_secs_f64(*end_secs);
            let start = output_lines.iter().position(|line| line.timestamp >= start_time).unwrap_or(output_lines.len());
            let end = output_lines.iter().rposition(|line| line.timestamp <= end_time).map(|i| i + 1).unwrap_or(0);
            (start, end.max(start))
        }
    };

    // Collect matching lines
    let mut matches: Vec<(usize, String)> = Vec::new();
    let lines_to_check = &output_lines[start_idx..end_idx];

    for (rel_idx, line) in lines_to_check.iter().enumerate() {
        let abs_idx = start_idx + rel_idx;

        // Skip gagged lines unless show_gagged is set
        if line.gagged && !opts.show_gagged {
            continue;
        }

        let plain = strip_ansi_codes(&line.text);
        let is_match = match &regex {
            Some(Ok(re)) => {
                let matched = re.is_match(&plain);
                if opts.inverse_match { !matched } else { matched }
            }
            Some(Err(_)) => false, // Invalid regex
            None => true, // No pattern = match all
        };

        if is_match {
            let mut display_line = line.text.clone();

            // Add timestamp if requested
            if opts.show_timestamps {
                let ts = line.timestamp.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                // Simple timestamp format HH:MM:SS
                let secs = ts % 60;
                let mins = (ts / 60) % 60;
                let hours = (ts / 3600) % 24;
                let ts_str = format!("{:02}:{:02}:{:02}", hours, mins, secs);
                display_line = format!("[{}] {}", ts_str, display_line);
            }

            // Add line number if requested
            if opts.show_line_numbers {
                display_line = format!("{}: {}", abs_idx + 1, display_line);
            }

            matches.push((abs_idx, display_line));
        }
    }

    // Handle LastMatching - keep only last N matches
    if let RecallRange::LastMatching(n) = &opts.range {
        let skip = matches.len().saturating_sub(*n);
        matches = matches.into_iter().skip(skip).collect();
    }

    // TODO: Handle context lines (-A, -B, -C) if needed

    let result: Vec<String> = matches.into_iter().map(|(_, s)| s).collect();

    // Generate header if not quiet
    let header = if opts.quiet {
        None
    } else {
        Some("================ Recall start ================".to_string())
    };

    (result, header)
}

/// Check if a line matches any action triggers
/// Returns None if no match, Some(result) if matched
pub fn check_action_triggers(
    line: &str,
    world_name: &str,
    actions: &[Action],
) -> Option<ActionTriggerResult> {
    // Strip ANSI codes for pattern matching
    let plain_line = strip_ansi_codes(line);

    for action in actions {
        // Skip disabled actions
        if !action.enabled {
            continue;
        }

        // Skip actions without patterns (those are manual /name only)
        if action.pattern.is_empty() {
            continue;
        }

        // Check if world matches (empty = all worlds, case-insensitive)
        if !action.world.is_empty() && !action.world.eq_ignore_ascii_case(world_name) {
            continue;
        }

        // Convert pattern based on match type
        let regex_pattern = match action.match_type {
            MatchType::Wildcard => wildcard_to_regex(&action.pattern),
            MatchType::Regexp => action.pattern.clone(),
        };

        // Try to compile and match the regex (case-insensitive)
        if let Ok(regex) = RegexBuilder::new(&regex_pattern)
            .case_insensitive(true)
            .build()
        {
            if let Some(caps) = regex.captures(&plain_line) {
                // Extract capture groups: $0 is full match, $1-$9 are groups
                let captures: Vec<&str> = caps.iter()
                    .map(|m| m.map(|m| m.as_str()).unwrap_or(""))
                    .collect();

                let commands = split_action_commands(&action.command);
                let should_gag = commands.iter().any(|cmd|
                    cmd.eq_ignore_ascii_case("/gag") || cmd.to_lowercase().starts_with("/gag ")
                );

                // Check for /highlight command and extract color
                let highlight_color = commands.iter().find_map(|cmd| {
                    let lower = cmd.to_lowercase();
                    if lower == "/highlight" {
                        Some(String::new()) // No color specified, use default
                    } else if lower.starts_with("/highlight ") {
                        Some(cmd[11..].trim().to_string()) // Extract color after "/highlight "
                    } else {
                        None
                    }
                });

                // Filter out /gag and /highlight, then substitute captures in commands
                let filtered_commands: Vec<String> = commands.into_iter()
                    .filter(|cmd| {
                        let lower = cmd.to_lowercase();
                        !lower.eq_ignore_ascii_case("/gag")
                            && !lower.starts_with("/gag ")
                            && lower != "/highlight"
                            && !lower.starts_with("/highlight ")
                    })
                    .map(|cmd| substitute_pattern_captures(&cmd, &captures))
                    .collect();

                return Some(ActionTriggerResult {
                    should_gag,
                    commands: filtered_commands,
                    highlight_color,
                });
            }
        }
    }

    None
}

/// Pre-compile action patterns into regexes for a specific world.
/// Call once before iterating over lines, not per-line.
pub fn compile_action_patterns(
    world_name: &str,
    actions: &[Action],
) -> Vec<Regex> {
    let mut compiled = Vec::new();
    for action in actions {
        if !action.enabled {
            continue;
        }
        if action.pattern.is_empty() {
            continue;
        }
        if !action.world.is_empty() && !action.world.eq_ignore_ascii_case(world_name) {
            continue;
        }
        let regex_pattern = match action.match_type {
            MatchType::Wildcard => wildcard_to_regex(&action.pattern),
            MatchType::Regexp => action.pattern.clone(),
        };
        if let Ok(regex) = RegexBuilder::new(&regex_pattern)
            .case_insensitive(true)
            .build()
        {
            compiled.push(regex);
        }
    }
    compiled
}

/// Check if a line matches any pre-compiled action pattern (for highlighting)
pub fn line_matches_compiled_patterns(
    line: &str,
    patterns: &[Regex],
) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let plain_line = strip_ansi_codes(line);
    for regex in patterns {
        if regex.is_match(&plain_line) {
            return true;
        }
    }
    false
}
