use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::tf;
use crate::util::{strip_ansi_codes, strip_mud_tag};
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

/// A single match pattern with a pre-compiled regex.
///
/// An action can hold multiple `MatchPattern`s; the action fires when **any** pattern
/// matches a line, and the **first** matching pattern supplies capture groups `$0..$9`.
/// The match type (Regexp/Wildcard) is stored on the parent `Action`, not per-pattern.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MatchPattern {
    /// The raw pattern string.
    pub pattern: String,
    /// Pre-compiled regex, rebuilt by `Action::compile_regex()`.  Not serialised.
    #[serde(skip)]
    pub compiled_regex: Option<Regex>,
}

impl Default for MatchPattern {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            compiled_regex: None,
        }
    }
}

/// Helper function for serde default to return true
fn default_enabled() -> bool { true }

/// User-defined action/trigger
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub name: String,           // Unique name (also used as /name command if no pattern)
    pub world: String,          // World name to match (empty = all worlds, comma-list OK)

    /// The authoritative list of match patterns.  Any matching pattern fires the action;
    /// the first matching pattern in list order supplies `$0..$9`.
    /// An empty list means the action is manual-only (invoked via `/name`).
    #[serde(default)]
    pub patterns: Vec<MatchPattern>,

    /// How to interpret all patterns: regular expression or glob/wildcard.
    /// This applies to every pattern in `patterns`.
    #[serde(default)]
    pub match_type: MatchType,

    /// Legacy single-pattern field — accepted on deserialise for backward-compat with
    /// old `settings.dat` and old WebSocket clients, but **never emitted** when
    /// serialising.  `Action::normalize()` migrates it into `patterns`.
    #[serde(default, skip_serializing)]
    pub pattern: String,

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
            patterns: Vec::new(),
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

    /// Migrate the legacy single-pattern field (`pattern`) into `patterns`.
    ///
    /// Idempotent: does nothing if `patterns` is already non-empty, or if the legacy
    /// `pattern` field is empty (i.e. the action is manual-only).  Call this after any
    /// deserialisation and before `compile_regex()`.  Does **not** touch `match_type`,
    /// which is now the authoritative action-level type.
    pub fn normalize(&mut self) {
        if self.patterns.is_empty() && !self.pattern.is_empty() {
            self.patterns.push(MatchPattern {
                pattern: std::mem::take(&mut self.pattern),
                compiled_regex: None,
            });
        }
    }

    /// Return a display-friendly preview of the first pattern (empty string if none).
    pub fn display_pattern(&self) -> &str {
        self.patterns.first().map(|mp| mp.pattern.as_str()).unwrap_or("")
    }

    /// Pre-compile the regex for **every** pattern in this action.
    ///
    /// Automatically calls `normalize()` first so legacy single-pattern fields are
    /// migrated into `patterns` before compilation.  Safe to call multiple times.
    /// The action-level `match_type` is applied uniformly to all patterns.
    /// - An empty pattern or a disabled action → `None` (never matches).
    /// - A bad regex → `None` (silently skipped; never panics).
    pub fn compile_regex(&mut self) {
        self.normalize();
        let match_type = self.match_type;
        for mp in &mut self.patterns {
            if mp.pattern.is_empty() || !self.enabled {
                mp.compiled_regex = None;
                continue;
            }
            let regex_pattern = match match_type {
                MatchType::Wildcard => wildcard_to_regex(&mp.pattern),
                MatchType::Regexp => mp.pattern.clone(),
            };
            mp.compiled_regex = RegexBuilder::new(&regex_pattern)
                .case_insensitive(true)
                .build()
                .ok();
        }
    }
}

/// Returns `true` when an action's `world` field names no specific world — i.e. the
/// field is empty or contains only blank comma segments.  Such an action applies to
/// all worlds.
pub fn world_field_is_global(action_world: &str) -> bool {
    action_world.split(',').all(|w| w.trim().is_empty())
}

/// Returns `true` when an action with the given `world` field is eligible to run in
/// `world_name`.  A global field (see [`world_field_is_global`]) matches every world.
/// Otherwise the field is split on commas, each segment is trimmed of whitespace, and
/// the comparison is case-insensitive.
///
/// Examples:
/// - `action_matches_world("", "MUD1")` → `true` (global)
/// - `action_matches_world("MUD1, MUD2", "mud2")` → `true`
/// - `action_matches_world(" MUD1 , MUD2 ", "MUD1")` → `true`
/// - `action_matches_world("MUD1, MUD2", "MUD3")` → `false`
pub fn action_matches_world(action_world: &str, world_name: &str) -> bool {
    world_field_is_global(action_world)
        || action_world.split(',').any(|w| w.trim().eq_ignore_ascii_case(world_name))
}

/// Find an action suitable for manual `/name` (or slash-less `name`) invocation
/// from the given world.
///
/// World eligibility is determined by [`action_matches_world`]: an empty `world`
/// field is available from every world; a comma-separated list is eligible from any
/// listed world (case-insensitive, whitespace around commas ignored). Within eligible
/// actions, an exact name match wins over a slash-stripped match, and a world-specific
/// action wins over a global one sharing the same name.
///
/// Only returns manual-only actions (no patterns) that are enabled and eligible for
/// `world_name`. Pattern-based actions are trigger-only and cannot be invoked as commands.
pub fn find_invocable_action<'a>(actions: &'a [Action], name: &str, world_name: &str) -> Option<&'a Action> {
    let no_slash = name.trim_start_matches('/');
    // Returns the first action in `scope` that matches by exact name then by
    // slash-stripped name.
    let find_in_scope = |world_specific: bool| -> Option<&'a Action> {
        let eligible = |a: &&Action| -> bool {
            // Pattern-based actions are trigger-only; they cannot be invoked as command aliases.
            if !a.patterns.is_empty() {
                return false;
            }
            if !a.enabled {
                return false;
            }
            if world_specific {
                !world_field_is_global(&a.world) && action_matches_world(&a.world, world_name)
            } else {
                world_field_is_global(&a.world)
            }
        };
        actions.iter().filter(|a| eligible(a)).find(|a| a.name.eq_ignore_ascii_case(name))
            .or_else(|| actions.iter().filter(|a| eligible(a)).find(|a| a.name.eq_ignore_ascii_case(no_slash)))
    };
    // Prefer a world-specific action over a global one.
    find_in_scope(true).or_else(|| find_in_scope(false))
}

/// If `command` has no leading slash and its first word matches an action that is
/// eligible for `world_name`, return the command rewritten with a leading slash so
/// it routes through the normal `ActionCommand` dispatch path.
///
/// This lets a user type `common` to invoke an action named `common` instead of
/// always requiring `/common`. World-scoped actions are only eligible when the
/// current world matches their `world` field; global actions (empty `world`) are
/// always eligible. Returns `None` when no rewrite is needed.
pub fn rewrite_slashless_action(command: &str, actions: &[Action], world_name: &str) -> Option<String> {
    let trimmed = command.trim();
    if trimmed.starts_with('/') {
        return None;
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    if first.is_empty() {
        return None;
    }
    if find_invocable_action(actions, first, world_name).is_some() {
        Some(format!("/{}{}", first, &trimmed[first.len()..]))
    } else {
        None
    }
}

/// Pre-compile regexes for all actions.
/// Call after loading settings, restoring reload state, or bulk action updates.
/// Also calls `normalize()` so legacy single-pattern actions are migrated.
pub fn compile_all_action_regexes(actions: &mut [Action]) {
    for action in actions.iter_mut() {
        action.normalize();
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
pub fn execute_recall(opts: &tf::RecallOptions, output_lines: &[OutputLine], show_tags: bool) -> (Vec<String>, Option<String>) {
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

        // Match against visible text only: when show_tags (F2) is off, strip MUD tags too
        let plain = if show_tags {
            strip_ansi_codes(&line.text)
        } else {
            strip_ansi_codes(&strip_mud_tag(&line.text))
        };
        let is_match = match &regex {
            Some(Ok(re)) => {
                let matched = re.is_match(&plain);
                if opts.inverse_match { !matched } else { matched }
            }
            Some(Err(_)) => false, // Invalid regex
            None => true, // No pattern = match all
        };

        if is_match {
            let mut display_line = if show_tags {
                line.text.clone()
            } else {
                strip_mud_tag(&line.text)
            };

            // Add timestamp if requested, honoring the optional -t[format] (default %H:%M:%S)
            if opts.show_timestamps {
                let epoch_secs = line.timestamp.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0) as i64;
                let lt = crate::util::local_time_from_epoch(epoch_secs);
                let fmt = opts.timestamp_format.as_deref().unwrap_or("%H:%M:%S");
                let ts_str = crate::util::format_local_time(&lt, fmt);
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

/// Check if a line matches any action triggers.
///
/// For each eligible action, patterns are tested in list order.  The **first**
/// matching pattern fires the action once and supplies capture groups `$0..$9`.
/// Returns `None` if no action matched.
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

        // Skip actions with no patterns (manual /name only)
        if action.patterns.is_empty() {
            continue;
        }

        // Check if world matches (empty or comma-list = eligible worlds, case-insensitive)
        if !action_matches_world(&action.world, world_name) {
            continue;
        }

        // Test each pattern in order; first match fires the action
        for mp in &action.patterns {
            if let Some(ref regex) = mp.compiled_regex {
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
    }

    None
}

/// Pre-compile action patterns into regexes for a specific world.
/// Flattens across all patterns of all eligible actions.
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
        if !action_matches_world(&action.world, world_name) {
            continue;
        }
        for mp in &action.patterns {
            if let Some(ref regex) = mp.compiled_regex {
                compiled.push(regex.clone());
            }
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

    // --- Action normalize() ---

    #[test]
    fn test_normalize_migrates_legacy_pattern() {
        let mut action = Action {
            pattern: "^You say".to_string(),
            match_type: MatchType::Wildcard,
            ..Action::default()
        };
        assert!(action.patterns.is_empty());
        action.normalize();
        assert_eq!(action.patterns.len(), 1);
        assert_eq!(action.patterns[0].pattern, "^You say");
        assert_eq!(action.match_type, MatchType::Wildcard); // action-level type preserved
        assert!(action.pattern.is_empty()); // legacy field cleared
    }

    #[test]
    fn test_normalize_empty_legacy_pattern_no_op() {
        let mut action = Action {
            pattern: String::new(),
            ..Action::default()
        };
        action.normalize();
        assert!(action.patterns.is_empty()); // manual-only action stays empty
    }

    #[test]
    fn test_normalize_idempotent_when_patterns_nonempty() {
        let mut action = Action {
            pattern: "legacy".to_string(), // would be migrated if patterns were empty
            ..Action::default()
        };
        action.patterns.push(MatchPattern { pattern: "existing".to_string(), compiled_regex: None });
        action.normalize();
        // patterns already non-empty, legacy field NOT migrated
        assert_eq!(action.patterns.len(), 1);
        assert_eq!(action.patterns[0].pattern, "existing");
    }

    // --- Action compile_regex() ---

    #[test]
    fn test_action_compile_regexp() {
        let mut action = Action {
            pattern: "^You say".to_string(),
            match_type: MatchType::Regexp,
            enabled: true,
            ..Action::default()
        };
        action.normalize();
        action.compile_regex();
        assert_eq!(action.patterns.len(), 1);
        assert!(action.patterns[0].compiled_regex.is_some());
        assert!(action.patterns[0].compiled_regex.as_ref().unwrap().is_match("You say hello"));
    }

    #[test]
    fn test_action_compile_wildcard() {
        let mut action = Action {
            pattern: "*tells you*".to_string(),
            match_type: MatchType::Wildcard,
            enabled: true,
            ..Action::default()
        };
        action.normalize();
        action.compile_regex();
        assert_eq!(action.patterns.len(), 1);
        assert!(action.patterns[0].compiled_regex.is_some());
        assert!(action.patterns[0].compiled_regex.as_ref().unwrap().is_match("Bob tells you hello"));
    }

    #[test]
    fn test_action_compile_disabled() {
        let mut action = Action {
            pattern: "test".to_string(),
            enabled: false,
            ..Action::default()
        };
        action.normalize();
        action.compile_regex();
        // Disabled: pattern exists in list but compiled_regex is None
        assert_eq!(action.patterns.len(), 1);
        assert!(action.patterns[0].compiled_regex.is_none());
    }

    #[test]
    fn test_action_compile_empty_pattern() {
        let mut action = Action {
            pattern: String::new(),
            enabled: true,
            ..Action::default()
        };
        action.normalize();
        action.compile_regex();
        // Empty pattern: patterns vec stays empty (manual-only action)
        assert!(action.patterns.is_empty());
    }

    #[test]
    fn test_compile_all_action_regexes() {
        let mut actions = vec![
            Action { pattern: "test1".to_string(), enabled: true, ..Action::default() },
            Action { pattern: "test2".to_string(), enabled: false, ..Action::default() },
            Action { pattern: "test3".to_string(), enabled: true, ..Action::default() },
        ];
        compile_all_action_regexes(&mut actions);
        assert!(actions[0].patterns[0].compiled_regex.is_some());
        assert!(actions[1].patterns[0].compiled_regex.is_none()); // disabled
        assert!(actions[2].patterns[0].compiled_regex.is_some());
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
        a.normalize();
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

    // --- Multi-pattern tests ---

    #[test]
    fn test_multi_pattern_any_match_fires() {
        // Action with two regexp patterns: either one matching should fire
        let mut action = Action {
            name: "test".to_string(),
            command: "wave".to_string(),
            enabled: true,
            match_type: MatchType::Regexp,
            patterns: vec![
                MatchPattern { pattern: "^You wave".to_string(), compiled_regex: None },
                MatchPattern { pattern: "^Bob waves".to_string(), compiled_regex: None },
            ],
            ..Action::default()
        };
        action.compile_regex();

        assert!(check_action_triggers("You wave goodbye", "", &[action.clone()]).is_some());
        assert!(check_action_triggers("Bob waves at you", "", &[action.clone()]).is_some());
        assert!(check_action_triggers("Nothing happened", "", &[action]).is_none());
    }

    #[test]
    fn test_multi_pattern_first_match_supplies_captures() {
        // Pattern 1: regexp with capture; Pattern 2: also regexp with capture.
        // Line matches ONLY pattern 1 → $1 comes from pattern 1's groups.
        let mut action = Action {
            name: "test".to_string(),
            command: "say $1".to_string(),
            enabled: true,
            match_type: MatchType::Regexp,
            patterns: vec![
                MatchPattern {
                    pattern: r"^(\w+) waves".to_string(),
                    compiled_regex: None,
                },
                MatchPattern {
                    pattern: r"^(\w+) tells you".to_string(),
                    compiled_regex: None,
                },
            ],
            ..Action::default()
        };
        action.compile_regex();

        // "Bob waves" matches pattern 1 → $1 = "Bob"
        let result = check_action_triggers("Bob waves", "", &[action.clone()]).unwrap();
        assert_eq!(result.commands, vec!["say Bob"]);

        // "Alice tells you hello" doesn't match pattern 1; matches pattern 2 → $1 = "Alice"
        let result2 = check_action_triggers("Alice tells you hello", "", &[action]).unwrap();
        assert_eq!(result2.commands, vec!["say Alice"]);
    }

    #[test]
    fn test_multi_pattern_fires_once_not_twice() {
        // Even if both patterns match the same line, the action fires only once
        let mut action = Action {
            name: "test".to_string(),
            command: "nod".to_string(),
            enabled: true,
            match_type: MatchType::Regexp,
            patterns: vec![
                MatchPattern { pattern: "hello".to_string(), compiled_regex: None },
                MatchPattern { pattern: "world".to_string(), compiled_regex: None },
            ],
            ..Action::default()
        };
        action.compile_regex();

        // "hello world" matches both patterns but check_action_triggers returns Some once
        let result = check_action_triggers("hello world", "", &[action]);
        assert!(result.is_some()); // fired once (not None, not multiple)
    }

    #[test]
    fn test_multi_pattern_wildcard_action() {
        // Wildcard match type applies to all patterns
        let mut action = Action {
            name: "test".to_string(),
            command: "wave".to_string(),
            enabled: true,
            match_type: MatchType::Wildcard,
            patterns: vec![
                MatchPattern { pattern: "* arrives *".to_string(), compiled_regex: None },
                MatchPattern { pattern: "* departs *".to_string(), compiled_regex: None },
            ],
            ..Action::default()
        };
        action.compile_regex();

        assert!(check_action_triggers("Bob arrives from the north", "", &[action.clone()]).is_some());
        assert!(check_action_triggers("Bob departs to the south", "", &[action.clone()]).is_some());
        assert!(check_action_triggers("Nothing happens", "", &[action]).is_none());
    }

    #[test]
    fn test_display_pattern_first_pattern() {
        let action = Action {
            patterns: vec![
                MatchPattern { pattern: "first".to_string(), ..Default::default() },
                MatchPattern { pattern: "second".to_string(), ..Default::default() },
            ],
            ..Action::default()
        };
        assert_eq!(action.display_pattern(), "first");
    }

    #[test]
    fn test_display_pattern_empty() {
        let action = Action::default();
        assert_eq!(action.display_pattern(), "");
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

    #[test]
    fn test_compiled_patterns_multi_pattern_action() {
        // An action with 2 patterns contributes 2 compiled regexes to the highlight set
        let mut action = Action {
            name: "test".to_string(),
            enabled: true,
            match_type: MatchType::Regexp,
            patterns: vec![
                MatchPattern { pattern: "hello".to_string(), compiled_regex: None },
                MatchPattern { pattern: "world".to_string(), compiled_regex: None },
            ],
            ..Action::default()
        };
        action.compile_regex();
        let actions = vec![action];

        let patterns = compile_action_patterns("", &actions);
        assert_eq!(patterns.len(), 2);
        assert!(line_matches_compiled_patterns("hello there", &patterns));
        assert!(line_matches_compiled_patterns("world peace", &patterns));
        assert!(!line_matches_compiled_patterns("nothing", &patterns));
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

    // --- find_invocable_action / rewrite_slashless_action ---

    fn named_action(name: &str) -> Action {
        Action { name: name.to_string(), ..Action::default() }
    }

    fn scoped_action(name: &str, world: &str) -> Action {
        Action { name: name.to_string(), world: world.to_string(), ..Action::default() }
    }

    // -- name matching (global actions, world_name = "") --

    #[test]
    fn test_find_invocable_action_exact() {
        let actions = vec![named_action("common")];
        let found = find_invocable_action(&actions, "common", "");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "common");
    }

    #[test]
    fn test_find_invocable_action_slash_strips_for_fallback() {
        // typing "/common" finds action named "common" via fallback
        let actions = vec![named_action("common")];
        let found = find_invocable_action(&actions, "/common", "");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "common");
    }

    #[test]
    fn test_find_invocable_action_slash_name_wins() {
        // action named "/common" wins over "common" when both exist
        let actions = vec![named_action("common"), named_action("/common")];
        let found = find_invocable_action(&actions, "/common", "");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "/common");
    }

    #[test]
    fn test_find_invocable_action_case_insensitive() {
        let actions = vec![named_action("Common")];
        assert!(find_invocable_action(&actions, "COMMON", "").is_some());
        assert!(find_invocable_action(&actions, "/common", "").is_some());
    }

    #[test]
    fn test_find_invocable_action_not_found() {
        let actions = vec![named_action("other")];
        assert!(find_invocable_action(&actions, "common", "").is_none());
    }

    // -- world filtering --

    #[test]
    fn test_find_invocable_action_world_specific_own_world() {
        // scoped action found from its own world
        let actions = vec![scoped_action("greet", "MUD1")];
        assert!(find_invocable_action(&actions, "greet", "MUD1").is_some());
        assert!(find_invocable_action(&actions, "/greet", "MUD1").is_some());
    }

    #[test]
    fn test_find_invocable_action_world_specific_other_world() {
        // scoped action NOT found from a different world
        let actions = vec![scoped_action("greet", "MUD1")];
        assert!(find_invocable_action(&actions, "greet", "MUD2").is_none());
        assert!(find_invocable_action(&actions, "greet", "").is_none());
    }

    #[test]
    fn test_find_invocable_action_world_case_insensitive() {
        let actions = vec![scoped_action("greet", "MUD1")];
        assert!(find_invocable_action(&actions, "greet", "mud1").is_some());
    }

    #[test]
    fn test_find_invocable_action_global_works_anywhere() {
        let actions = vec![named_action("greet")];
        assert!(find_invocable_action(&actions, "greet", "MUD1").is_some());
        assert!(find_invocable_action(&actions, "greet", "MUD2").is_some());
        assert!(find_invocable_action(&actions, "greet", "").is_some());
    }

    #[test]
    fn test_find_invocable_action_world_specific_wins_over_global() {
        // both "greet" (global) and "greet" (MUD1-scoped) exist; from MUD1 get the scoped one
        let global = named_action("greet");
        let local  = scoped_action("greet", "MUD1");
        let actions = vec![global, local];
        let found = find_invocable_action(&actions, "greet", "MUD1").unwrap();
        assert_eq!(found.world, "MUD1");
        // from another world, fall back to the global one
        let found2 = find_invocable_action(&actions, "greet", "MUD2").unwrap();
        assert!(found2.world.is_empty());
    }

    // -- rewrite_slashless_action --

    #[test]
    fn test_rewrite_slashless_no_op_for_slash_input() {
        let actions = vec![named_action("common")];
        assert_eq!(rewrite_slashless_action("/common", &actions, ""), None);
    }

    #[test]
    fn test_rewrite_slashless_no_op_for_unknown_word() {
        let actions = vec![named_action("common")];
        assert_eq!(rewrite_slashless_action("look", &actions, ""), None);
    }

    #[test]
    fn test_rewrite_slashless_rewrites_when_action_found() {
        let actions = vec![named_action("common")];
        assert_eq!(
            rewrite_slashless_action("common", &actions, ""),
            Some("/common".to_string())
        );
    }

    #[test]
    fn test_rewrite_slashless_preserves_args() {
        let actions = vec![named_action("common")];
        assert_eq!(
            rewrite_slashless_action("common foo bar", &actions, ""),
            Some("/common foo bar".to_string())
        );
    }

    #[test]
    fn test_rewrite_slashless_case_insensitive() {
        let actions = vec![named_action("Common")];
        assert_eq!(
            rewrite_slashless_action("COMMON", &actions, ""),
            Some("/COMMON".to_string())
        );
    }

    #[test]
    fn test_rewrite_slashless_world_scoped_own_world() {
        let actions = vec![scoped_action("greet", "MUD1")];
        assert_eq!(
            rewrite_slashless_action("greet", &actions, "MUD1"),
            Some("/greet".to_string())
        );
    }

    #[test]
    fn test_rewrite_slashless_world_scoped_other_world() {
        // scoped to MUD1, so from MUD2 the word is NOT rewritten
        let actions = vec![scoped_action("greet", "MUD1")];
        assert_eq!(rewrite_slashless_action("greet", &actions, "MUD2"), None);
    }

    // -- world_field_is_global --

    #[test]
    fn test_world_field_is_global_empty() {
        assert!(world_field_is_global(""));
    }

    #[test]
    fn test_world_field_is_global_blank() {
        assert!(world_field_is_global("  "));
    }

    #[test]
    fn test_world_field_is_global_blank_comma_segments() {
        // ", ," has only blank segments → still global
        assert!(world_field_is_global(", ,"));
        assert!(world_field_is_global(" , "));
    }

    #[test]
    fn test_world_field_is_global_single_name() {
        assert!(!world_field_is_global("MUD1"));
    }

    #[test]
    fn test_world_field_is_global_comma_list() {
        assert!(!world_field_is_global("MUD1, MUD2"));
    }

    // -- action_matches_world --

    #[test]
    fn test_action_matches_world_global_matches_any() {
        assert!(action_matches_world("", "MUD1"));
        assert!(action_matches_world("", ""));
    }

    #[test]
    fn test_action_matches_world_single_match() {
        assert!(action_matches_world("MUD1", "MUD1"));
        assert!(!action_matches_world("MUD1", "MUD2"));
    }

    #[test]
    fn test_action_matches_world_comma_list() {
        assert!(action_matches_world("MUD1, MUD2", "MUD1"));
        assert!(action_matches_world("MUD1, MUD2", "MUD2"));
        assert!(!action_matches_world("MUD1, MUD2", "MUD3"));
    }

    #[test]
    fn test_action_matches_world_case_insensitive() {
        assert!(action_matches_world("MUD1, MUD2", "mud2"));
        assert!(action_matches_world("mud1", "MUD1"));
    }

    #[test]
    fn test_action_matches_world_spaces_around_commas() {
        // extra whitespace is trimmed
        assert!(action_matches_world(" MUD1 , MUD2 ", "MUD1"));
        assert!(action_matches_world(" MUD1 , MUD2 ", "MUD2"));
        assert!(!action_matches_world(" MUD1 , MUD2 ", "MUD3"));
    }

    // -- find_invocable_action with comma-list world --

    #[test]
    fn test_find_invocable_action_multi_world_match() {
        let actions = vec![scoped_action("greet", "MUD1, MUD2")];
        assert!(find_invocable_action(&actions, "greet", "MUD1").is_some());
        assert!(find_invocable_action(&actions, "greet", "MUD2").is_some());
        assert!(find_invocable_action(&actions, "greet", "MUD3").is_none());
    }

    #[test]
    fn test_find_invocable_action_multi_world_wins_over_global() {
        let global = named_action("greet");
        let local  = scoped_action("greet", "MUD1, MUD2");
        let actions = vec![global, local];
        // from MUD1 → multi-world action wins
        let found = find_invocable_action(&actions, "greet", "MUD1").unwrap();
        assert_eq!(found.world, "MUD1, MUD2");
        // from MUD3 → falls back to global
        let found2 = find_invocable_action(&actions, "greet", "MUD3").unwrap();
        assert!(found2.world.is_empty());
    }

    #[test]
    fn test_find_invocable_action_ignores_pattern_based() {
        // An action with a pattern is trigger-only and must not be found as a command alias.
        let mut action = named_action("adrick");
        action.patterns.push(MatchPattern { pattern: "*adrick*".to_string(), compiled_regex: None });
        let actions = vec![action];
        assert!(find_invocable_action(&actions, "adrick", "").is_none());
        assert!(find_invocable_action(&actions, "/adrick", "").is_none());
    }

    #[test]
    fn test_find_invocable_action_ignores_disabled() {
        // A disabled action must not be found as a command alias (silently ignored).
        let mut action = named_action("greet");
        action.enabled = false;
        let actions = vec![action];
        assert!(find_invocable_action(&actions, "greet", "").is_none());
    }

    #[test]
    fn test_rewrite_slashless_multi_world() {
        let actions = vec![scoped_action("greet", "MUD1, MUD2")];
        assert_eq!(
            rewrite_slashless_action("greet", &actions, "MUD1"),
            Some("/greet".to_string())
        );
        assert_eq!(
            rewrite_slashless_action("greet", &actions, "MUD2"),
            Some("/greet".to_string())
        );
        assert_eq!(rewrite_slashless_action("greet", &actions, "MUD3"), None);
    }
}
