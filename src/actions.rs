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
    #[serde(skip)]
    pub compiled_regex: Option<Regex>,  // Pre-compiled regex for pattern matching
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
            compiled_regex: None,
        }
    }
}

impl Action {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-compile the regex pattern for this action.
    /// Call after changing pattern, match_type, or enabled.
    pub fn compile_regex(&mut self) {
        if self.pattern.is_empty() || !self.enabled {
            self.compiled_regex = None;
            return;
        }
        let regex_pattern = match self.match_type {
            MatchType::Wildcard => wildcard_to_regex(&self.pattern),
            MatchType::Regexp => self.pattern.clone(),
        };
        self.compiled_regex = RegexBuilder::new(&regex_pattern)
            .case_insensitive(true)
            .build()
            .ok();
    }
}

/// Pre-compile regexes for all actions.
/// Call after loading settings, restoring reload state, or bulk action updates.
pub fn compile_all_action_regexes(actions: &mut [Action]) {
    for action in actions.iter_mut() {
        action.compile_regex();
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

        // Filter by source: -w (default) = server only, -l = local only, -g = all
        match &opts.source {
            tf::RecallSource::CurrentWorld | tf::RecallSource::World(_) => {
                // Default: only MUD server output (not client-generated)
                if !line.from_server {
                    continue;
                }
            }
            tf::RecallSource::Local => {
                // -l: only client-generated output (TF output, system messages)
                if line.from_server {
                    continue;
                }
            }
            tf::RecallSource::Global => {
                // -g: all lines (server + local)
            }
            tf::RecallSource::Input => {
                // -i: input history - handled separately, skip all output lines
                continue;
            }
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
                let epoch_secs = line.timestamp.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0) as i64;
                let lt = crate::util::local_time_from_epoch(epoch_secs);
                let ts_str = format!("{:02}:{:02}:{:02}", lt.hour, lt.minute, lt.second);
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

        // Use pre-compiled regex
        if let Some(ref regex) = action.compiled_regex {
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- split_action_commands ---

    #[test]
    fn test_split_single_command() {
        assert_eq!(split_action_commands("say hello"), vec!["say hello"]);
    }

    #[test]
    fn test_split_multiple_commands() {
        assert_eq!(
            split_action_commands("say hello; wave; smile"),
            vec!["say hello", "wave", "smile"]
        );
    }

    #[test]
    fn test_split_escaped_semicolon() {
        assert_eq!(
            split_action_commands("say hello\\; world; wave"),
            vec!["say hello; world", "wave"]
        );
    }

    #[test]
    fn test_split_empty_segments() {
        // Empty segments between semicolons should be skipped
        assert_eq!(split_action_commands("say hi;;wave"), vec!["say hi", "wave"]);
    }

    #[test]
    fn test_split_empty_string() {
        let result: Vec<String> = Vec::new();
        assert_eq!(split_action_commands(""), result);
    }

    #[test]
    fn test_split_whitespace_trimming() {
        assert_eq!(
            split_action_commands("  say hi  ;  wave  "),
            vec!["say hi", "wave"]
        );
    }

    // --- substitute_action_args ---

    #[test]
    fn test_substitute_args_positional() {
        assert_eq!(
            substitute_action_args("say $1 to $2", "hello world"),
            "say hello to world"
        );
    }

    #[test]
    fn test_substitute_args_star() {
        assert_eq!(
            substitute_action_args("say $*", "hello world"),
            "say hello world"
        );
    }

    #[test]
    fn test_substitute_args_missing() {
        // Missing args substitute to nothing
        assert_eq!(
            substitute_action_args("say $1 $2 $3", "only one"),
            "say only one "
        );
    }

    #[test]
    fn test_substitute_args_no_substitution() {
        assert_eq!(
            substitute_action_args("plain text", "args"),
            "plain text"
        );
    }

    #[test]
    fn test_substitute_args_dollar_at_end() {
        assert_eq!(substitute_action_args("cost $", ""), "cost $");
    }

    #[test]
    fn test_substitute_args_dollar_zero_not_substituted() {
        // $0 is not substituted by substitute_action_args (only by pattern captures)
        assert_eq!(substitute_action_args("$0", "hello"), "$0");
    }

    // --- substitute_pattern_captures ---

    #[test]
    fn test_substitute_captures_basic() {
        let captures = vec!["full match", "group1", "group2"];
        assert_eq!(
            substitute_pattern_captures("got $1 and $2 from $0", &captures),
            "got group1 and group2 from full match"
        );
    }

    #[test]
    fn test_substitute_captures_missing() {
        let captures = vec!["match", "one"];
        assert_eq!(
            substitute_pattern_captures("$1 $2 $3", &captures),
            "one  "
        );
    }

    #[test]
    fn test_substitute_captures_star_preserved() {
        // $* should be preserved (not consumed) for later action arg substitution
        let captures = vec!["match"];
        assert_eq!(
            substitute_pattern_captures("$0 $*", &captures),
            "match $*"
        );
    }

    // --- wildcard_to_regex ---

    #[test]
    fn test_wildcard_star() {
        let re = Regex::new(&wildcard_to_regex("*hello*")).unwrap();
        assert!(re.is_match("say hello world"));
        assert!(!re.is_match("say goodbye"));
    }

    #[test]
    fn test_wildcard_question() {
        let re = Regex::new(&wildcard_to_regex("h?llo")).unwrap();
        assert!(re.is_match("hello"));
        assert!(re.is_match("hallo"));
        assert!(!re.is_match("hllo"));
    }

    #[test]
    fn test_wildcard_escaped() {
        let re = Regex::new(&wildcard_to_regex("what\\?")).unwrap();
        assert!(re.is_match("what?"));
        assert!(!re.is_match("whatx"));
    }

    #[test]
    fn test_wildcard_escaped_star() {
        let re = Regex::new(&wildcard_to_regex("5\\*5")).unwrap();
        assert!(re.is_match("5*5"));
        assert!(!re.is_match("5x5"));
    }

    #[test]
    fn test_wildcard_captures() {
        let re = Regex::new(&wildcard_to_regex("* tells you: *")).unwrap();
        let caps = re.captures("Bob tells you: Hello there").unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "Bob");
        assert_eq!(caps.get(2).unwrap().as_str(), "Hello there");
    }

    #[test]
    fn test_wildcard_regex_special_chars_escaped() {
        // Regex special chars like . + should be escaped
        let re = Regex::new(&wildcard_to_regex("file.txt")).unwrap();
        assert!(re.is_match("file.txt"));
        assert!(!re.is_match("filextxt"));
    }

    #[test]
    fn test_wildcard_curly_quotes_normalized() {
        let re = Regex::new(&wildcard_to_regex("say \"hello\"")).unwrap();
        assert!(re.is_match("say \"hello\""));
        assert!(re.is_match("say \u{201C}hello\u{201D}")); // curly double quotes
    }

    // --- Action compile_regex ---

    #[test]
    fn test_action_compile_regexp() {
        let mut action = Action {
            pattern: "^You say".to_string(),
            match_type: MatchType::Regexp,
            enabled: true,
            ..Action::default()
        };
        action.compile_regex();
        assert!(action.compiled_regex.is_some());
        assert!(action.compiled_regex.as_ref().unwrap().is_match("You say hello"));
    }

    #[test]
    fn test_action_compile_wildcard() {
        let mut action = Action {
            pattern: "*tells you*".to_string(),
            match_type: MatchType::Wildcard,
            enabled: true,
            ..Action::default()
        };
        action.compile_regex();
        assert!(action.compiled_regex.is_some());
        assert!(action.compiled_regex.as_ref().unwrap().is_match("Bob tells you hello"));
    }

    #[test]
    fn test_action_compile_disabled() {
        let mut action = Action {
            pattern: "test".to_string(),
            enabled: false,
            ..Action::default()
        };
        action.compile_regex();
        assert!(action.compiled_regex.is_none());
    }

    #[test]
    fn test_action_compile_empty_pattern() {
        let mut action = Action {
            pattern: String::new(),
            enabled: true,
            ..Action::default()
        };
        action.compile_regex();
        assert!(action.compiled_regex.is_none());
    }

    #[test]
    fn test_compile_all_action_regexes() {
        let mut actions = vec![
            Action { pattern: "test1".to_string(), enabled: true, ..Action::default() },
            Action { pattern: "test2".to_string(), enabled: false, ..Action::default() },
            Action { pattern: "test3".to_string(), enabled: true, ..Action::default() },
        ];
        compile_all_action_regexes(&mut actions);
        assert!(actions[0].compiled_regex.is_some());
        assert!(actions[1].compiled_regex.is_none()); // disabled
        assert!(actions[2].compiled_regex.is_some());
    }

    // --- check_action_triggers ---

    fn make_action(name: &str, pattern: &str, command: &str, match_type: MatchType) -> Action {
        let mut a = Action {
            name: name.to_string(),
            pattern: pattern.to_string(),
            command: command.to_string(),
            match_type,
            enabled: true,
            ..Action::default()
        };
        a.compile_regex();
        a
    }

    #[test]
    fn test_trigger_regexp_match() {
        let actions = vec![make_action("test", r"^You say", "nod", MatchType::Regexp)];
        let result = check_action_triggers("You say hello", "", &actions);
        assert!(result.is_some());
        assert_eq!(result.unwrap().commands, vec!["nod"]);
    }

    #[test]
    fn test_trigger_wildcard_match() {
        let actions = vec![make_action("test", "*tells you*", "nod", MatchType::Wildcard)];
        let result = check_action_triggers("Bob tells you hi", "", &actions);
        assert!(result.is_some());
        assert_eq!(result.unwrap().commands, vec!["nod"]);
    }

    #[test]
    fn test_trigger_no_match() {
        let actions = vec![make_action("test", "zzzzz", "nod", MatchType::Regexp)];
        let result = check_action_triggers("Hello world", "", &actions);
        assert!(result.is_none());
    }

    #[test]
    fn test_trigger_case_insensitive() {
        let actions = vec![make_action("test", "hello", "nod", MatchType::Regexp)];
        let result = check_action_triggers("HELLO WORLD", "", &actions);
        assert!(result.is_some());
    }

    #[test]
    fn test_trigger_gag_command() {
        let actions = vec![make_action("test", "spam", "/gag; say filtered", MatchType::Regexp)];
        let result = check_action_triggers("spam message", "", &actions).unwrap();
        assert!(result.should_gag);
        // /gag itself should be filtered from commands
        assert_eq!(result.commands, vec!["say filtered"]);
    }

    #[test]
    fn test_trigger_highlight_command() {
        let actions = vec![make_action("test", "important", "/highlight red", MatchType::Regexp)];
        let result = check_action_triggers("important message", "", &actions).unwrap();
        assert_eq!(result.highlight_color, Some("red".to_string()));
        // /highlight should be filtered from commands
        assert!(result.commands.is_empty());
    }

    #[test]
    fn test_trigger_capture_substitution() {
        let actions = vec![make_action(
            "test", r"(\w+) tells you: (.*)", "say Thanks, $1! Got: $2", MatchType::Regexp
        )];
        let result = check_action_triggers("Bob tells you: Hello", "", &actions).unwrap();
        assert_eq!(result.commands, vec!["say Thanks, Bob! Got: Hello"]);
    }

    #[test]
    fn test_trigger_wildcard_capture_substitution() {
        let actions = vec![make_action(
            "test", "* tells you: *", "say Thanks, $1!", MatchType::Wildcard
        )];
        let result = check_action_triggers("Bob tells you: Hello", "", &actions).unwrap();
        assert_eq!(result.commands, vec!["say Thanks, Bob!"]);
    }

    #[test]
    fn test_trigger_world_filter() {
        let mut action = make_action("test", "hello", "wave", MatchType::Regexp);
        action.world = "MyWorld".to_string();
        let actions = vec![action];

        // Should match when world matches (case-insensitive)
        assert!(check_action_triggers("hello", "myworld", &actions).is_some());
        // Should not match for a different world
        assert!(check_action_triggers("hello", "OtherWorld", &actions).is_none());
    }

    #[test]
    fn test_trigger_disabled_skipped() {
        let mut action = make_action("test", "hello", "wave", MatchType::Regexp);
        action.enabled = false;
        action.compile_regex(); // re-compile after disabling
        let actions = vec![action];
        assert!(check_action_triggers("hello", "", &actions).is_none());
    }

    #[test]
    fn test_trigger_empty_pattern_skipped() {
        let actions = vec![make_action("manual", "", "wave", MatchType::Regexp)];
        assert!(check_action_triggers("anything", "", &actions).is_none());
    }

    #[test]
    fn test_trigger_strips_ansi() {
        let actions = vec![make_action("test", "hello", "wave", MatchType::Regexp)];
        // Line with ANSI color codes should still match
        let result = check_action_triggers("\x1b[31mhello\x1b[0m", "", &actions);
        assert!(result.is_some());
    }

    // --- compile_action_patterns / line_matches_compiled_patterns ---

    #[test]
    fn test_compiled_patterns_match() {
        let actions = vec![
            make_action("a", "hello", "", MatchType::Regexp),
            make_action("b", "*world*", "", MatchType::Wildcard),
        ];
        let patterns = compile_action_patterns("", &actions);
        assert_eq!(patterns.len(), 2);
        assert!(line_matches_compiled_patterns("hello there", &patterns));
        assert!(line_matches_compiled_patterns("big world here", &patterns));
        assert!(!line_matches_compiled_patterns("nothing here", &patterns));
    }

    #[test]
    fn test_compiled_patterns_empty() {
        let patterns: Vec<Regex> = Vec::new();
        assert!(!line_matches_compiled_patterns("anything", &patterns));
    }

    #[test]
    fn test_compiled_patterns_world_filter() {
        let mut action = make_action("test", "hello", "", MatchType::Regexp);
        action.world = "MyWorld".to_string();
        let actions = vec![action];

        let patterns = compile_action_patterns("MyWorld", &actions);
        assert_eq!(patterns.len(), 1);

        let patterns = compile_action_patterns("Other", &actions);
        assert_eq!(patterns.len(), 0);
    }

    // --- MatchType ---

    #[test]
    fn test_match_type_parse() {
        assert_eq!(MatchType::parse("Wildcard"), MatchType::Wildcard);
        assert_eq!(MatchType::parse("wildcard"), MatchType::Wildcard);
        assert_eq!(MatchType::parse("Regexp"), MatchType::Regexp);
        assert_eq!(MatchType::parse("anything_else"), MatchType::Regexp);
    }

    #[test]
    fn test_match_type_as_str() {
        assert_eq!(MatchType::Regexp.as_str(), "Regexp");
        assert_eq!(MatchType::Wildcard.as_str(), "Wildcard");
    }
}
