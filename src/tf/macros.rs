//! Macro system for TinyFugue compatibility.
//!
//! Implements:
//! - #def command with flags for triggers, hooks, keybindings, attributes
//! - #undef, #undefn, #undeft for removing macros
//! - #list for listing macros
//! - #purge for removing all macros
//! - Trigger pattern matching with capture groups

use regex::Regex;
use super::{TfEngine, TfMacro, TfTrigger, TfMatchMode, TfAttributes, TfHookEvent, TfCommandResult, TfValue};
use super::variables;

/// Parse the #def command and create a macro
///
/// Syntax: #def [options] name = body
/// Options:
///   -t"pattern"  Trigger pattern
///   -mMODE       Match mode (simple, glob, regexp)
///   -pPRIORITY   Priority (integer, higher = first)
///   -F           Fall-through (continue matching after this macro)
///   -1           One-shot (fire once then undefine)
///   -nCOUNT      Fire COUNT times then undefine
///   -aATTRS      Attributes (gag, bold, underline, etc.)
///   -E"expr"     Conditional expression
///   -cCHANCE     Probability (0.0 to 1.0)
///   -w"world"    Restrict to specific world
///   -h"event"    Hook event (CONNECT, DISCONNECT, etc.)
///   -b"keys"     Key binding (literal sequence)
///   -B"keyname"  Named key binding
pub fn parse_def(args: &str) -> Result<TfMacro, String> {
    let mut macro_def = TfMacro::default();
    let mut remaining = args.trim();

    // Parse options
    while remaining.starts_with('-') {
        let (opt, rest) = parse_option(remaining)?;
        remaining = rest.trim_start();

        match opt {
            DefOption::Trigger(pattern) => {
                macro_def.trigger = Some(TfTrigger {
                    pattern,
                    match_mode: macro_def.trigger.as_ref()
                        .map(|t| t.match_mode)
                        .unwrap_or_default(),
                    compiled: None,
                });
            }
            DefOption::MatchMode(mode) => {
                if let Some(ref mut trigger) = macro_def.trigger {
                    trigger.match_mode = mode;
                } else {
                    macro_def.trigger = Some(TfTrigger {
                        pattern: String::new(),
                        match_mode: mode,
                        compiled: None,
                    });
                }
            }
            DefOption::Priority(p) => macro_def.priority = p,
            DefOption::FallThrough => macro_def.fall_through = true,
            DefOption::OneShot => {
                macro_def.one_shot = Some(1);
                macro_def.shots_remaining = Some(1);
            }
            DefOption::ShotCount(n) => {
                macro_def.one_shot = Some(n);
                macro_def.shots_remaining = Some(n);
            }
            DefOption::Attributes(attrs) => macro_def.attributes = attrs,
            DefOption::Condition(expr) => macro_def.condition = Some(expr),
            DefOption::Probability(p) => macro_def.probability = Some(p),
            DefOption::World(w) => macro_def.world = Some(w),
            DefOption::Hook(event) => macro_def.hook = Some(event),
            DefOption::KeyBinding(keys) => macro_def.keybinding = Some(keys),
        }
    }

    // Parse name = body
    // Find the = separator
    let eq_pos = remaining.find('=')
        .ok_or_else(|| "Missing '=' in #def (syntax: #def [options] name = body)".to_string())?;

    let name = remaining[..eq_pos].trim();
    let body = remaining[eq_pos + 1..].trim();

    if name.is_empty() {
        return Err("Macro name cannot be empty".to_string());
    }

    macro_def.name = name.to_string();
    macro_def.body = body.to_string();

    // Compile trigger pattern if present
    if let Some(ref mut trigger) = macro_def.trigger {
        if !trigger.pattern.is_empty() {
            trigger.compiled = compile_pattern(&trigger.pattern, trigger.match_mode)?;
        }
    }

    Ok(macro_def)
}

/// Options that can be parsed from #def
enum DefOption {
    Trigger(String),
    MatchMode(TfMatchMode),
    Priority(i32),
    FallThrough,
    OneShot,
    ShotCount(u32),
    Attributes(TfAttributes),
    Condition(String),
    Probability(f32),
    World(String),
    Hook(TfHookEvent),
    KeyBinding(String),
}

/// Parse a single option from the input, return (option, remaining)
fn parse_option(input: &str) -> Result<(DefOption, &str), String> {
    if !input.starts_with('-') {
        return Err("Expected option starting with -".to_string());
    }

    let input = &input[1..]; // Skip -

    if input.is_empty() {
        return Err("Empty option".to_string());
    }

    let first_char = input.chars().next().unwrap();

    match first_char {
        't' => {
            // -t"pattern" or -tpattern
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            Ok((DefOption::Trigger(value), rest))
        }
        'm' => {
            // -mMODE
            let (value, rest) = parse_word(&input[1..]);
            let mode = TfMatchMode::parse(&value)
                .ok_or_else(|| format!("Unknown match mode: {}", value))?;
            Ok((DefOption::MatchMode(mode), rest))
        }
        'p' => {
            // -pPRIORITY
            let (value, rest) = parse_word(&input[1..]);
            let priority: i32 = value.parse()
                .map_err(|_| format!("Invalid priority: {}", value))?;
            Ok((DefOption::Priority(priority), rest))
        }
        'F' => {
            // -F (fall-through)
            Ok((DefOption::FallThrough, &input[1..]))
        }
        '1' => {
            // -1 (one-shot)
            Ok((DefOption::OneShot, &input[1..]))
        }
        'n' => {
            // -nCOUNT
            let (value, rest) = parse_word(&input[1..]);
            let count: u32 = value.parse()
                .map_err(|_| format!("Invalid shot count: {}", value))?;
            Ok((DefOption::ShotCount(count), rest))
        }
        'a' => {
            // -aATTRS
            let (value, rest) = parse_word(&input[1..]);
            let attrs = parse_attributes(&value)?;
            Ok((DefOption::Attributes(attrs), rest))
        }
        'E' => {
            // -E"expression"
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            Ok((DefOption::Condition(value), rest))
        }
        'c' => {
            // -cCHANCE
            let (value, rest) = parse_word(&input[1..]);
            let chance: f32 = value.parse()
                .map_err(|_| format!("Invalid probability: {}", value))?;
            if !(0.0..=1.0).contains(&chance) {
                return Err("Probability must be between 0.0 and 1.0".to_string());
            }
            Ok((DefOption::Probability(chance), rest))
        }
        'w' => {
            // -w"world"
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            Ok((DefOption::World(value), rest))
        }
        'h' => {
            // -h"event" or -hEVENT
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            let event = TfHookEvent::parse(&value)
                .ok_or_else(|| format!("Unknown hook event: {}", value))?;
            Ok((DefOption::Hook(event), rest))
        }
        'b' | 'B' => {
            // -b"keys" or -B"keyname"
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            Ok((DefOption::KeyBinding(value), rest))
        }
        _ => Err(format!("Unknown option: -{}", first_char)),
    }
}

/// Parse a quoted string or a word (non-whitespace sequence)
fn parse_quoted_or_word(input: &str) -> Result<(String, &str), String> {
    let input = input.trim_start();

    if input.starts_with('"') {
        // Quoted string
        let mut end = 1;
        let chars: Vec<char> = input.chars().collect();
        let mut result = String::new();

        while end < chars.len() {
            if chars[end] == '\\' && end + 1 < chars.len() {
                // Escape sequence
                result.push(chars[end + 1]);
                end += 2;
            } else if chars[end] == '"' {
                // End of quoted string
                let byte_end = input.char_indices()
                    .nth(end + 1)
                    .map(|(i, _)| i)
                    .unwrap_or(input.len());
                return Ok((result, &input[byte_end..]));
            } else {
                result.push(chars[end]);
                end += 1;
            }
        }

        Err("Unclosed quote in option".to_string())
    } else {
        // Unquoted word
        Ok(parse_word(input))
    }
}

/// Parse a word (sequence of non-whitespace, non-special characters)
fn parse_word(input: &str) -> (String, &str) {
    let end = input.find(|c: char| c.is_whitespace() || c == '=' || c == '-')
        .unwrap_or(input.len());

    (input[..end].to_string(), &input[end..])
}

/// Parse attribute string (e.g., "gag,bold,hilite:red")
fn parse_attributes(attrs: &str) -> Result<TfAttributes, String> {
    let mut result = TfAttributes::default();

    for attr in attrs.split(',') {
        let attr = attr.trim().to_lowercase();

        if let Some(color) = attr.strip_prefix("hilite:") {
            result.hilite = Some(color.to_string());
        } else {
            match attr.as_str() {
                "gag" => result.gag = true,
                "norecord" => result.norecord = true,
                "bold" => result.bold = true,
                "underline" => result.underline = true,
                "reverse" => result.reverse = true,
                "flash" => result.flash = true,
                "dim" => result.dim = true,
                "bell" => result.bell = true,
                "" => {} // Ignore empty
                _ => return Err(format!("Unknown attribute: {}", attr)),
            }
        }
    }

    Ok(result)
}

/// Compile a trigger pattern into a regex
fn compile_pattern(pattern: &str, mode: TfMatchMode) -> Result<Option<Regex>, String> {
    let regex_pattern = match mode {
        TfMatchMode::Simple => {
            // Literal substring match - escape all regex special chars
            regex::escape(pattern)
        }
        TfMatchMode::Glob => {
            // Glob pattern: * matches anything, ? matches single char
            glob_to_regex(pattern)
        }
        TfMatchMode::Regexp => {
            // Already a regex
            pattern.to_string()
        }
    };

    Regex::new(&regex_pattern)
        .map(Some)
        .map_err(|e| format!("Invalid pattern: {}", e))
}

/// Convert a glob pattern to a regex pattern
/// Supports \* and \? to match literal asterisk and question mark
pub fn glob_to_regex(glob: &str) -> String {
    let mut regex = String::with_capacity(glob.len() * 2);
    let mut chars = glob.chars().peekable();

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
            '*' => regex.push_str("(.*)"),
            '?' => regex.push_str("(.)"),
            '[' => {
                // Character class - pass through
                regex.push('[');
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    regex.push(nc);
                    if nc == ']' {
                        break;
                    }
                }
            }
            // Escape regex special characters
            '.' | '+' | '^' | '$' | '(' | ')' | '{' | '}' | '|' => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
    }

    regex
}

/// Match a line against a trigger and return captures if matched
pub struct TriggerMatch<'a> {
    pub full_match: &'a str,
    pub captures: Vec<&'a str>,
    pub left: &'a str,
    pub right: &'a str,
}

/// Try to match a line against a trigger pattern
pub fn match_trigger<'a>(trigger: &TfTrigger, line: &'a str) -> Option<TriggerMatch<'a>> {
    let regex = trigger.compiled.as_ref()?;

    let caps = regex.captures(line)?;
    let full = caps.get(0)?;

    let mut captures = Vec::new();
    for i in 1..caps.len() {
        if let Some(m) = caps.get(i) {
            captures.push(m.as_str());
        }
    }

    Some(TriggerMatch {
        full_match: full.as_str(),
        captures,
        left: &line[..full.start()],
        right: &line[full.end()..],
    })
}

/// Execute a macro with the given arguments/captures
pub fn execute_macro(
    engine: &mut TfEngine,
    macro_def: &TfMacro,
    args: &[&str],
    trigger_match: Option<&TriggerMatch>,
) -> Vec<TfCommandResult> {
    let mut results = Vec::new();

    // Check condition if present
    if let Some(ref condition) = macro_def.condition {
        match super::expressions::evaluate(engine, condition) {
            Ok(value) => {
                if !value.to_bool() {
                    return results; // Condition false, don't execute
                }
            }
            Err(e) => {
                results.push(TfCommandResult::Error(format!("Condition error: {}", e)));
                return results;
            }
        }
    }

    // Check probability
    if let Some(prob) = macro_def.probability {
        let random_val = super::expressions::simple_random() as f32 / u32::MAX as f32;
        if random_val > prob {
            return results; // Random check failed
        }
    }

    // Push a local scope for macro execution
    engine.push_scope();

    // Set positional parameters
    for (i, arg) in args.iter().enumerate() {
        engine.set_local(&format!("{}", i + 1), TfValue::String(arg.to_string()));
    }

    // Set special variables
    engine.set_local("*", TfValue::String(args.join(" ")));
    engine.set_local("#", TfValue::Integer(args.len() as i64));

    // Substitute variables and execute body
    let mut body = macro_def.body.clone();

    // Substitute positional parameters first
    body = variables::substitute_positional(&body, args);

    // Substitute capture groups if from trigger
    if let Some(tm) = trigger_match {
        let capture_refs: Vec<&str> = tm.captures.to_vec();
        body = variables::substitute_captures(&body, tm.full_match, &capture_refs, tm.left, tm.right);
    }

    // Split body by %; or ; for multiple commands
    // In TF macro definitions, %; is commonly used as command separator
    // (especially with line continuation), while ; also works
    for cmd in body.split("%;").flat_map(|s| s.split(';')) {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            continue;
        }

        // Execute the command
        let result = if cmd.starts_with('#') {
            super::parser::execute_command(engine, cmd)
        } else if cmd.starts_with('/') {
            TfCommandResult::ClayCommand(cmd.to_string())
        } else {
            TfCommandResult::SendToMud(cmd.to_string())
        };

        results.push(result);
    }

    // Pop the local scope
    engine.pop_scope();

    results
}

/// Find and execute all macros that match a line
pub fn process_triggers(engine: &mut TfEngine, line: &str, world: Option<&str>) -> Vec<TfCommandResult> {
    let mut results = Vec::new();
    let mut macros_to_remove = Vec::new();

    // Sort macros by priority (higher first)
    let mut macro_indices: Vec<usize> = (0..engine.macros.len()).collect();
    macro_indices.sort_by(|&a, &b| {
        engine.macros[b].priority.cmp(&engine.macros[a].priority)
    });

    for idx in macro_indices {
        let macro_def = &engine.macros[idx];

        // Check world restriction
        if let Some(ref macro_world) = macro_def.world {
            if let Some(current_world) = world {
                if macro_world != current_world {
                    continue;
                }
            }
        }

        // Check if macro has a trigger
        let trigger = match &macro_def.trigger {
            Some(t) if !t.pattern.is_empty() => t,
            _ => continue,
        };

        // Try to match
        if let Some(trigger_match) = match_trigger(trigger, line) {
            // Check shots remaining
            if let Some(remaining) = macro_def.shots_remaining {
                if remaining == 0 {
                    continue;
                }
            }

            // Clone necessary data for execution
            let macro_clone = macro_def.clone();
            let fall_through = macro_def.fall_through;

            // Execute the macro
            let exec_results = execute_macro(engine, &macro_clone, &[], Some(&trigger_match));
            results.extend(exec_results);

            // Decrement shots if one-shot/n-shot
            if let Some(ref mut remaining) = engine.macros[idx].shots_remaining {
                *remaining -= 1;
                if *remaining == 0 {
                    macros_to_remove.push(idx);
                }
            }

            // Stop if not fall-through
            if !fall_through {
                break;
            }
        }
    }

    // Remove exhausted macros (in reverse order to preserve indices)
    macros_to_remove.sort_by(|a, b| b.cmp(a));
    for idx in macros_to_remove {
        engine.macros.remove(idx);
    }

    results
}

/// List macros matching an optional pattern
pub fn list_macros(engine: &TfEngine, pattern: Option<&str>) -> String {
    let mut output = String::new();

    let pattern_regex = pattern.and_then(|p| Regex::new(p).ok());

    for macro_def in &engine.macros {
        // Filter by name pattern if provided
        if let Some(ref re) = pattern_regex {
            if !re.is_match(&macro_def.name) {
                continue;
            }
        }

        // Format: N: #def [opts] name = body (sparkle added by output system)
        output.push_str(&format!("{}: #def ", macro_def.sequence_number));

        // Show trigger if present (before name, like TF)
        if let Some(ref trigger) = macro_def.trigger {
            if !trigger.pattern.is_empty() {
                output.push_str(&format!("-t\"{}\" ", trigger.pattern));
                if trigger.match_mode != TfMatchMode::Glob {
                    output.push_str(&format!("-m{:?} ", trigger.match_mode).to_lowercase());
                }
            }
        }

        // Show other flags
        if macro_def.priority != 0 {
            output.push_str(&format!("-p{} ", macro_def.priority));
        }
        if macro_def.fall_through {
            output.push_str("-F ");
        }
        if let Some(n) = macro_def.one_shot {
            if n == 1 {
                output.push_str("-1 ");
            } else {
                output.push_str(&format!("-n{} ", n));
            }
        }
        if let Some(ref hook) = macro_def.hook {
            output.push_str(&format!("-h{:?} ", hook));
        }

        output.push_str(&format!("{} = {}\n", macro_def.name, macro_def.body));
    }

    if output.is_empty() {
        "No macros defined.".to_string()
    } else {
        output
    }
}

/// Remove a macro by exact name
pub fn undef_macro(engine: &mut TfEngine, name: &str) -> bool {
    if let Some(idx) = engine.macros.iter().position(|m| m.name == name) {
        engine.macros.remove(idx);
        true
    } else {
        false
    }
}

/// Remove macros by name pattern
pub fn undef_by_name_pattern(engine: &mut TfEngine, pattern: &str) -> usize {
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return 0,
    };

    let before = engine.macros.len();
    engine.macros.retain(|m| !re.is_match(&m.name));
    before - engine.macros.len()
}

/// Remove macros by trigger pattern
pub fn undef_by_trigger_pattern(engine: &mut TfEngine, pattern: &str) -> usize {
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return 0,
    };

    let before = engine.macros.len();
    engine.macros.retain(|m| {
        if let Some(ref trigger) = m.trigger {
            !re.is_match(&trigger.pattern)
        } else {
            true
        }
    });
    before - engine.macros.len()
}

/// Remove all macros
pub fn purge_macros(engine: &mut TfEngine) -> usize {
    let count = engine.macros.len();
    engine.macros.clear();
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_def_simple() {
        let result = parse_def("greet = say Hello!").unwrap();
        assert_eq!(result.name, "greet");
        assert_eq!(result.body, "say Hello!");
        assert!(result.trigger.is_none());
    }

    #[test]
    fn test_parse_def_with_trigger() {
        let result = parse_def("-t\"^You hit *\" attack = kick").unwrap();
        assert_eq!(result.name, "attack");
        assert_eq!(result.body, "kick");
        assert!(result.trigger.is_some());
        let trigger = result.trigger.unwrap();
        assert_eq!(trigger.pattern, "^You hit *");
        assert_eq!(trigger.match_mode, TfMatchMode::Glob);
    }

    #[test]
    fn test_parse_def_with_options() {
        let result = parse_def("-t\"test\" -mregexp -p10 -F -1 foo = bar").unwrap();
        assert_eq!(result.name, "foo");
        assert_eq!(result.priority, 10);
        assert!(result.fall_through);
        assert_eq!(result.one_shot, Some(1));
        let trigger = result.trigger.unwrap();
        assert_eq!(trigger.match_mode, TfMatchMode::Regexp);
    }

    #[test]
    fn test_parse_def_with_hook() {
        let result = parse_def("-hCONNECT on_connect = say Hello!").unwrap();
        assert_eq!(result.hook, Some(TfHookEvent::Connect));
    }

    #[test]
    fn test_glob_to_regex() {
        assert_eq!(glob_to_regex("hello"), "hello");
        assert_eq!(glob_to_regex("hello*"), "hello(.*)");
        assert_eq!(glob_to_regex("*world"), "(.*)world");
        assert_eq!(glob_to_regex("he?lo"), "he(.)lo");
        assert_eq!(glob_to_regex("test.txt"), "test\\.txt");
    }

    #[test]
    fn test_match_trigger() {
        let trigger = TfTrigger {
            pattern: "You hit (.+) for (\\d+) damage".to_string(),
            match_mode: TfMatchMode::Regexp,
            compiled: Some(Regex::new("You hit (.+) for (\\d+) damage").unwrap()),
        };

        let line = "You hit the goblin for 42 damage!";
        let result = match_trigger(&trigger, line).unwrap();

        assert_eq!(result.full_match, "You hit the goblin for 42 damage");
        assert_eq!(result.captures, vec!["the goblin", "42"]);
        assert_eq!(result.left, "");
        assert_eq!(result.right, "!");
    }

    #[test]
    fn test_parse_attributes() {
        let attrs = parse_attributes("gag,bold,hilite:red").unwrap();
        assert!(attrs.gag);
        assert!(attrs.bold);
        assert_eq!(attrs.hilite, Some("red".to_string()));
        assert!(!attrs.underline);
    }

    #[test]
    fn test_undef_macro() {
        let mut engine = TfEngine::new();
        engine.macros.push(TfMacro {
            name: "test".to_string(),
            body: "hello".to_string(),
            ..Default::default()
        });

        assert!(undef_macro(&mut engine, "test"));
        assert!(engine.macros.is_empty());
        assert!(!undef_macro(&mut engine, "test")); // Already removed
    }

    #[test]
    fn test_list_macros() {
        let mut engine = TfEngine::new();
        engine.add_macro(TfMacro {
            name: "greet".to_string(),
            body: "say Hello!".to_string(),
            ..Default::default()
        });
        engine.add_macro(TfMacro {
            name: "attack".to_string(),
            body: "kick".to_string(),
            trigger: Some(TfTrigger {
                pattern: "^You hit".to_string(),
                match_mode: TfMatchMode::Glob,
                compiled: None,
            }),
            ..Default::default()
        });

        let output = list_macros(&engine, None);
        // Format: N: #def [opts] name = body
        assert!(output.contains("0: #def greet = say Hello!"));
        assert!(output.contains("1: #def -t\"^You hit\" attack = kick"));
    }
}
