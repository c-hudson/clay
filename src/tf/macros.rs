//! Macro system for TinyFugue compatibility.
//!
//! Implements:
//! - /def command with flags for triggers, hooks, keybindings, attributes
//! - /undef, /undefn, /undeft for removing macros
//! - /list for listing macros
//! - /purge for removing all macros
//! - Trigger pattern matching with capture groups

use regex::Regex;
use super::{TfEngine, TfMacro, TfTrigger, TfMatchMode, TfAttributes, TfHookEvent, TfCommandResult, TfValue};
use super::variables;
use super::control_flow;

/// Parse the /def command and create a macro
///
/// Syntax: /def [options] name = body
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
///   -T<type>     Restrict to worlds of a given type (glob/regexp per -m)
///   -hEVENT      Hook event (any of TF's 31 - see TfHookEvent::parse), matches every occurrence
///   -h"EVENT pattern"  Hook event with an argument pattern (matched like -t, see fire_hook)
///   -b"keys"     Key binding (literal sequence)
///   -B"keyname"  Named key binding
///   -i / -I      Invisible: hidden from /list, /save, /purge unless forced
///   -q           Quiet: see TfMacro::quiet
///   -f           Same as -a, for backward compatibility
pub fn parse_def(args: &str) -> Result<TfMacro, String> {
    let mut macro_def = TfMacro::default();
    let mut remaining = args.trim();

    // Parse options. Each '-'-prefixed token may be a *cluster* of bundled
    // short options (finding 24, e.g. "-iFp9999" = -i -F -p9999): after a
    // flag that takes no argument, keep parsing the rest of the same token
    // as further flags, without requiring another leading '-'. A flag that
    // takes an argument consumes the remainder of the token (or a quoted
    // string within it), which naturally ends the cluster - see
    // `parse_option_char`'s doc comment.
    while remaining.starts_with('-') {
        let mut cluster = &remaining[1..];
        loop {
            let (opt, rest) = parse_option_char(cluster)?;
            apply_def_option(&mut macro_def, opt);
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                remaining = rest.trim_start();
                break;
            }
            cluster = rest;
        }
    }

    // TinyFugue allows an empty name when the macro is addressed some other way - by
    // trigger pattern, keybinding, or hook - in which case it's a "nameless" macro,
    // referred to only by its number (#N). See finding C.9 / tests/tf/cases/macros.tf.
    let has_addressable_option = macro_def.trigger.as_ref().map(|t| !t.pattern.is_empty()).unwrap_or(false)
        || macro_def.keybinding.is_some()
        || macro_def.hook.is_some();

    // Parse name [= body]
    // The = and body are optional - a macro with just options (e.g., -ag -t"pattern" name)
    // applies attributes when triggered without executing any commands
    if let Some(eq_pos) = remaining.find('=') {
        let name = remaining[..eq_pos].trim();
        let body = remaining[eq_pos + 1..].trim();

        if name.is_empty() && !has_addressable_option {
            return Err("Macro name cannot be empty".to_string());
        }

        macro_def.name = name.to_string();
        macro_def.body = body.to_string();
    } else {
        // No = sign: entire remaining text is the macro name (no body)
        let name = remaining.trim();
        if name.is_empty() && !has_addressable_option {
            return Err("Macro name cannot be empty".to_string());
        }
        macro_def.name = name.to_string();
    }

    // Compile trigger pattern if present
    if let Some(ref mut trigger) = macro_def.trigger {
        if !trigger.pattern.is_empty() {
            trigger.compiled = compile_pattern(&trigger.pattern, trigger.match_mode)?;
        }
    }

    Ok(macro_def)
}

/// Apply one parsed `/def` option to a macro definition. Split out of
/// `parse_def`'s option loop so bundled-cluster parsing (finding 24) can
/// call it once per flag in a token like "-iFp9999" without duplicating the
/// match arms.
fn apply_def_option(macro_def: &mut TfMacro, opt: DefOption) {
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
        DefOption::PriorityExpr(expr) => macro_def.priority_expr = Some(expr),
        DefOption::FallThrough => macro_def.fall_through = true,
        DefOption::PartialHilite => macro_def.partial_hilite = true,
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
        DefOption::Hook(event, pattern) => {
            macro_def.hook = Some(event);
            macro_def.hook_pattern = pattern;
        }
        DefOption::KeyBinding(keys) => macro_def.keybinding = Some(keys),
        DefOption::Invisible => macro_def.invisible = true,
        DefOption::Quiet => macro_def.quiet = true,
        DefOption::WorldType(t) => macro_def.world_type = Some(t),
    }
}

/// Resolve a `/def`/`/edit`'s deferred `-p<expr>` priority expression (see
/// `TfMacro::priority_expr`'s doc comment) against `engine`, evaluating it
/// exactly once as a TF expression - real tf: "the argument to -p may be
/// an expression that has a numeric value... evaluated only once, when the
/// macro is defined" (`/help def`). A no-op when `parse_def` already
/// resolved a plain decimal literal itself (the overwhelmingly common
/// case), so every caller of `parse_def` can call this unconditionally
/// right after it. Errors (e.g. `maxpri` used before it's defined) surface
/// the same way an invalid plain-integer priority already did.
pub(crate) fn resolve_priority_expr(engine: &mut TfEngine, macro_def: &mut TfMacro) -> Result<(), String> {
    if let Some(expr) = macro_def.priority_expr.take() {
        let value = super::expressions::evaluate(engine, &expr)
            .map_err(|e| format!("Invalid priority: {}", e))?;
        macro_def.priority = value.to_int()
            .ok_or_else(|| format!("Invalid priority: {}", expr))? as i32;
    }
    Ok(())
}

/// Options that can be parsed from /def
enum DefOption {
    Trigger(String),
    MatchMode(TfMatchMode),
    Priority(i32),
    /// A `-p<expr>` whose value isn't a plain decimal literal - see
    /// `TfMacro::priority_expr`'s doc comment.
    PriorityExpr(String),
    FallThrough,
    PartialHilite,
    OneShot,
    ShotCount(u32),
    Attributes(TfAttributes),
    Condition(String),
    Probability(f32),
    World(String),
    /// `-h<event>` or `-h"<event> <pattern>"`/`-h'<event> <pattern>'` (finding
    /// C.10 / plan step P1.9). `None` pattern: bare event name, matches every
    /// occurrence (see `/help hook`'s "pattern will default to *").
    Hook(TfHookEvent, Option<String>),
    KeyBinding(String),
    Invisible,
    Quiet,
    WorldType(String),
}

/// Parse a single flag (and its argument, if it takes one) from `input`,
/// which is everything after a '-' - either the start of a fresh option
/// token, or the continuation of a bundled cluster like "Fp9999" after the
/// "i" in "-iFp9999" has already been consumed (finding 24). A flag that
/// takes no argument (F, P, 1, i, I, q) returns the remaining characters of
/// the same token unconsumed, so the caller can keep parsing them as more
/// flags; a flag that takes a value (t, m, p, n, a, E, c, w, T, f, h, b, B)
/// consumes through the end of the token or a quoted string, so the
/// returned remainder is empty or starts at whitespace - naturally ending
/// the cluster.
fn parse_option_char(input: &str) -> Result<(DefOption, &str), String> {
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
            // -pPRIORITY, or -p'expr'/-p"expr" (real tf's own "/help def":
            // "As in all numeric options, the argument to -p may be an
            // expression that has a numeric value. E.g. '/def -pmaxpri
            // ...' will set the macro's priority to the value of the
            // variable maxpri. The expression is evaluated only once, when
            // the macro is defined." - stdlib.tf's own "-Fp'maxpri'"
            // idiom). The plain-decimal case is a pure literal parse (no
            // engine needed here, and this is the overwhelmingly common
            // case - e.g. "-iFp9999"); anything else is deferred as
            // `PriorityExpr` and resolved by `resolve_priority_expr`,
            // called from `cmd_def`/`cmd_edit` where a `TfEngine` is
            // actually available to evaluate it against.
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            match value.parse::<i32>() {
                Ok(priority) => Ok((DefOption::Priority(priority), rest)),
                Err(_) => Ok((DefOption::PriorityExpr(value), rest)),
            }
        }
        'F' => {
            // -F (fall-through)
            Ok((DefOption::FallThrough, &input[1..]))
        }
        'P' => {
            // -P (partial hilite)
            Ok((DefOption::PartialHilite, &input[1..]))
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
        'T' => {
            // -T<type>: world-type restriction, matched per the macro's -m style
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            Ok((DefOption::WorldType(value), rest))
        }
        'i' | 'I' => {
            // -i / -I: invisible (not shown by /list, /save, /purge unless forced)
            Ok((DefOption::Invisible, &input[1..]))
        }
        'q' => {
            // -q: quiet (see TfMacro::quiet doc comment for the exact semantics)
            Ok((DefOption::Quiet, &input[1..]))
        }
        'f' => {
            // -f: same as -a, kept for backward compatibility
            let (value, rest) = parse_word(&input[1..]);
            let attrs = parse_attributes(&value)?;
            Ok((DefOption::Attributes(attrs), rest))
        }
        'h' => {
            // -hEVENT, or -h"EVENT pattern" / -h'EVENT pattern' (finding C.10 /
            // plan step P1.9): the quoted form's value is "EVENT pattern" as one
            // string (parse_quoted_or_word already un-escapes it) - split off the
            // first whitespace-delimited word as the event name, the rest (if
            // any) is the hook pattern.
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            let mut parts = value.splitn(2, char::is_whitespace);
            let event_str = parts.next().unwrap_or("");
            let pattern = parts.next()
                .map(|p| p.trim_start().to_string())
                .filter(|p| !p.is_empty());
            let event = TfHookEvent::parse(event_str)
                .ok_or_else(|| format!("Unknown hook event: {}", event_str))?;
            Ok((DefOption::Hook(event, pattern), rest))
        }
        'b' => {
            // -b"keys" - a literal character sequence, normalised through the
            // shared key-name grammar (keynames::parse_key_name) the same way
            // /bind does (plan P2.1), so e.g. -b'^[[A' is stored as "Up" and a
            // later /list -b / keybinding lookup sees the same canonical form
            // regardless of which raw spelling defined it.
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            let canonical = crate::keynames::parse_key_name(&value)
                .map(|seq| seq.canonical())
                .map_err(|e| format!("Invalid key sequence -b'{}': {}", value, e))?;
            Ok((DefOption::KeyBinding(canonical), rest))
        }
        'B' => {
            // -B"keyname" - TF's own *named* key binding (deprecated upstream,
            // but still accepted - `/help def`). Unlike -b, the value names a
            // key via TF's key_<name> naming convention (plan Job 21/P2.5:
            // keynames::tf_name_to_token), not a literal byte sequence -
            // `-B"F5"` means the physical F5 key, whatever bytes it actually
            // sends, not the two literal characters "F5". This used to be
            // treated identically to -b (Job 5/17's own finding), which only
            // happened to work by coincidence for names that parse as BOTH a
            // literal sequence and a named key ("F5" itself, "Up" - a bare
            // named-key word IS also valid raw grammar text) - it silently
            // gave the wrong answer for anything that isn't (`-B'ctrl_left'`
            // has no meaning as a literal sequence at all).
            let (value, rest) = parse_quoted_or_word(&input[1..])?;
            let token = crate::keynames::tf_name_to_token(&value)
                .map_err(|e| format!("Invalid key name -B'{}': {}", value, e))?;
            let canonical = crate::keynames::KeySeq(vec![token]).canonical();
            Ok((DefOption::KeyBinding(canonical), rest))
        }
        _ => Err(format!("Unknown option: -{}", first_char)),
    }
}

/// Parse a quoted string or a word (non-whitespace sequence)
/// Handles both double quotes ("...") and single quotes ('...')
fn parse_quoted_or_word(input: &str) -> Result<(String, &str), String> {
    let input = input.trim_start();

    let quote_char = if input.starts_with('"') {
        Some('"')
    } else if input.starts_with('\'') {
        Some('\'')
    } else {
        None
    };

    if let Some(quote) = quote_char {
        // Quoted string
        let mut end = 1;
        let chars: Vec<char> = input.chars().collect();
        let mut result = String::new();

        while end < chars.len() {
            if chars[end] == '\\' && end + 1 < chars.len() {
                // Escape sequence
                result.push(chars[end + 1]);
                end += 2;
            } else if chars[end] == quote {
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

/// Parse %{hiliteattr} variable value into TfAttributes.
/// Default is "B" (bold). Supports TF single-letter codes like "B", "Cred", etc.
pub fn parse_hiliteattr(hiliteattr: &str) -> super::TfAttributes {
    match parse_attributes(hiliteattr) {
        Ok(mut attrs) => {
            // If no explicit hilite/bold/underline was set, default to hilite marker
            if attrs.hilite.is_none() && !attrs.bold && !attrs.underline {
                attrs.hilite = Some("hilite".to_string());
            }
            attrs
        }
        Err(_) => {
            // Fallback to default bold hilite
            super::TfAttributes {
                hilite: Some("hilite".to_string()),
                ..Default::default()
            }
        }
    }
}

/// Parse attribute string (e.g., "gag,bold,hilite:red")
fn parse_attributes(attrs: &str) -> Result<TfAttributes, String> {
    let mut result = TfAttributes::default();

    for attr in attrs.split(',') {
        let attr = attr.trim();

        if attr.is_empty() {
            continue;
        }

        // Check for long-form names first (case-insensitive)
        let lower = attr.to_lowercase();
        if let Some(color) = lower.strip_prefix("hilite:") {
            result.hilite = Some(color.to_string());
            continue;
        }
        match lower.as_str() {
            "gag" => { result.gag = true; continue; }
            "norecord" | "nohistory" => { result.norecord = true; continue; }
            "nolog" => { continue; } // Accepted but not tracked
            "noactivity" => { continue; } // Accepted but not tracked
            "bold" => { result.bold = true; continue; }
            "underline" => { result.underline = true; continue; }
            "reverse" => { result.reverse = true; continue; }
            "flash" => { result.flash = true; continue; }
            "dim" => { result.dim = true; continue; }
            "bell" => { result.bell = true; continue; }
            _ => {}
        }

        // Parse TF single-letter attribute codes: g, G, L, A, u, r, B, b, h, n, x, C, E, W, d, f
        // Multiple codes can be concatenated (e.g., "gB" = gag + bold)
        let chars: Vec<char> = attr.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                'n' => {} // normal/none - reset (we just don't set anything)
                'x' => {} // exclusive - accepted but not tracked separately
                'g' => result.gag = true,
                'G' => result.norecord = true, // nohistory
                'L' => {} // nolog - accepted but not tracked
                'A' => {} // noactivity - accepted but not tracked
                'u' => result.underline = true,
                'r' => result.reverse = true,
                'B' => result.bold = true,
                'b' => result.bell = true,
                'h' => result.hilite = Some("hilite".to_string()),
                'd' | 'f' => {} // dim/flash - accepted for compat
                'E' | 'W' => {} // error/warning attrs - accepted but not tracked
                'C' => {
                    // Color: "Cname" or "Cbgname" - consume rest as color
                    let color: String = chars[i+1..].iter().collect();
                    if !color.is_empty() {
                        result.hilite = Some(color);
                    }
                    i = chars.len(); // consumed all remaining
                    continue;
                }
                _ => return Err(format!("Unknown attribute: {}", attr)),
            }
            i += 1;
        }
    }

    Ok(result)
}

/// Compile a trigger pattern into a regex. `pub(crate)`: also used by
/// `hooks::compile_hook_pattern` to compile a `-h"EVENT pattern"` the same way
/// (finding C.10 / plan step P1.9).
pub(crate) fn compile_pattern(pattern: &str, mode: TfMatchMode) -> Result<Option<Regex>, String> {
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
            // Already a regex, but convert TF $$ to regex $ (end-of-line anchor)
            // In TF, $$ is how you write $ in a pattern that goes through variable substitution
            pattern.replace("$$", "$")
        }
    };

    Regex::new(&regex_pattern)
        .map(Some)
        .map_err(|e| format!("Invalid pattern: {}", e))
}

/// Convert a glob pattern to a regex pattern
/// Supports \* and \? to match literal asterisk and question mark, "[...]"
/// character classes (passed through to regex directly - ranges and leading
/// "^" negation come along for free), and "{a|b|c}" alternation (TF: "curly
/// braces can be used to match any one of a list of words"). Each
/// alternative inside "{...}" is itself run back through this function, so
/// wildcards nest correctly (e.g. "{d*g}"). Real TF also requires a "{...}"
/// group to be bounded by a wildcard, space, or the start/end of the whole
/// pattern; that boundary rule is not enforced here - nothing that uses this
/// (triggers, hooks, /list, /purge, world-type matching) needs it rejected,
/// only accepted.
pub fn glob_to_regex(glob: &str) -> String {
    let chars: Vec<char> = glob.chars().collect();
    glob_to_regex_chars(&chars)
}

fn glob_to_regex_chars(chars: &[char]) -> String {
    let mut regex = String::with_capacity(chars.len() * 2);
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                // Check for escape sequences
                match chars.get(i + 1) {
                    Some('*') | Some('?') | Some('\\') => {
                        // Escaped wildcard or backslash - treat as literal
                        regex.push('\\');
                        regex.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    _ => {
                        // Lone backslash - escape it for regex
                        regex.push_str("\\\\");
                        i += 1;
                        continue;
                    }
                }
            }
            '*' => regex.push_str("(.*)"),
            '?' => regex.push_str("(.)"),
            '[' => {
                // Character class - pass through
                regex.push('[');
                i += 1;
                while i < chars.len() {
                    regex.push(chars[i]);
                    let closed = chars[i] == ']';
                    i += 1;
                    if closed {
                        break;
                    }
                }
                continue;
            }
            '{' => {
                // "{a|b|c}" alternation: find the matching '}' (no nesting -
                // real TF patterns don't nest these, and nothing here needs it).
                if let Some(rel) = chars[i + 1..].iter().position(|&ch| ch == '}') {
                    let close = i + 1 + rel;
                    let inner = &chars[i + 1..close];
                    let mut alts = Vec::new();
                    let mut start = 0;
                    for (j, &ch) in inner.iter().enumerate() {
                        if ch == '|' {
                            alts.push(glob_to_regex_chars(&inner[start..j]));
                            start = j + 1;
                        }
                    }
                    alts.push(glob_to_regex_chars(&inner[start..]));
                    regex.push_str("(?:");
                    regex.push_str(&alts.join("|"));
                    regex.push(')');
                    i = close + 1;
                    continue;
                } else {
                    // No matching '}' - not a valid alternation, treat literally.
                    regex.push_str("\\{");
                    i += 1;
                    continue;
                }
            }
            '}' => {
                regex.push_str("\\}");
                i += 1;
                continue;
            }
            // Escape regex special characters
            '.' | '+' | '^' | '$' | '(' | ')' | '|' => {
                regex.push('\\');
                regex.push(c);
            }
            _ => regex.push(c),
        }
        i += 1;
    }

    regex
}

/// Check whether a macro's `-T` world-type restriction (if any) matches the current world's
/// type. TF matches `-T`'s pattern using the macro's own `-m` matching style, the same as
/// `-t`/`-h`. A macro with no `-T` always matches (TF default: "any type"). When the caller
/// can't supply a world type (`current_type: None`) - e.g. `fire_hook`, which has no world
/// context - a `-T`-restricted macro never matches: this is the documented safe default for
/// a pattern (like TF's own `-T{tiny|tiny.*}`) that doesn't match any of Clay's own world
/// types, generalised to "can't be verified, so don't fire". See finding C.1/C.9.
pub fn world_type_matches(macro_def: &TfMacro, current_type: Option<&str>) -> bool {
    let Some(ref pattern) = macro_def.world_type else { return true; };
    let Some(current_type) = current_type else { return false; };
    let mode = macro_def.trigger.as_ref().map(|t| t.match_mode).unwrap_or_default();
    match compile_pattern(pattern, mode) {
        Ok(Some(re)) => re.is_match(current_type),
        _ => false,
    }
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

/// Split a macro body into execution units, preserving control flow blocks as single units.
///
/// This handles cases like:
///   /if (cond) cmd1%;/else cmd2%;/endif
/// Which should be treated as ONE control flow block, not split by %;
fn split_body_preserving_control_flow(body: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut control_depth = 0;  // Track nesting of /if//while//for blocks

    // Split on "%;" only - in TF, only %; is a command separator in macro
    // bodies, and bare ; is NOT a separator (unlike some other scripting
    // languages). Reuse control_flow's quote- and "%%;"-escape-aware
    // splitter (see its own doc comment) rather than a second, naive
    // `body.split("%;")` - that naive form didn't know "%%;" is TF's escaped
    // literal "%;" and not a separator (finding 15), which broke anything
    // relying on the idiom: lib_self.tf's own %%;-based quine, and
    // tick.tf's /repeat bodies (e.g. "/set _tick_pid1=0%%;/tick_warn" must
    // stay one piece, passed whole to /repeat, not split at the %%;).
    let parts = control_flow::split_percent_semi(body);

    for part in &parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Count control flow keywords within this part (handles inline nested structures)
        let depth_change = count_control_flow_depth_change(trimmed);

        if control_depth == 0 {
            if depth_change > 0 {
                // Starting a new control flow block
                control_depth = depth_change;
                current = trimmed.to_string();
            } else {
                // Regular command, add directly
                result.push(trimmed.to_string());
            }
        } else {
            // Inside a control flow block
            // Append to current block
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(trimmed);

            control_depth += depth_change;

            if control_depth <= 0 {
                control_depth = 0;
                // End of control flow block, emit it
                result.push(std::mem::take(&mut current));
            }
        }
    }

    // If there's remaining content (unclosed control flow), add it anyway
    if !current.is_empty() {
        result.push(current);
    }

    result
}

/// Count the net change in control flow depth from a piece of text.
/// Returns positive for opening keywords (/if, /while, /for), negative for closing (/endif, /done).
fn count_control_flow_depth_change(text: &str) -> i32 {
    let lower = text.to_lowercase();
    let mut depth = 0;

    // We need to find all occurrences of control flow keywords
    // This is tricky because "/if" could appear in a string, but for simplicity
    // we'll scan for them as whitespace-separated tokens

    // Look for control flow starts
    let words: Vec<&str> = lower.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        // Check if this is a control flow keyword (possibly with something attached)
        if *word == "/if" || word.starts_with("/if(")
            || *word == "/while" || word.starts_with("/while(")
        {
            depth += 1;
        } else if *word == "/for" {
            // TF's own command-form /for ("/for var min max command" -
            // finding C.7/P1.7) never has a matching /done - see
            // control_flow::is_self_contained_for's doc comment for why
            // this must not be counted as an opener the way Clay's own
            // numeric /for extension is.
            if !control_flow::is_self_contained_for(&words, i) {
                depth += 1;
            }
        } else if *word == "/endif" || *word == "/done" {
            depth -= 1;
        }
    }

    depth
}

/// TF's own `max_recur` default (see `/help max_recur`: "max_recur=100 ...
/// recursion count exceeded %max_recur (%d)"). Guards `execute_macro`
/// against a macro that calls itself (directly, or indirectly through a
/// builtin it shadows) without ever bottoming out - notably a macro named
/// after one of Clay's own stub commands (e.g. "echo") whose body calls
/// plain `/echo`: now that a same-named macro takes precedence over a
/// builtin (see `parser::execute_command_impl`), that call would otherwise
/// re-enter the same macro forever and overflow the real Rust stack instead
/// of failing cleanly. `/@echo` (the "force the builtin" prefix) is the
/// escape hatch such a macro body needs to reach the real builtin.
const MAX_MACRO_RECURSION: u32 = 100;

/// Execute a macro with the given arguments/captures, as if invoked with
/// ordinary `/name ...` command syntax (or as a trigger/hook, which TF
/// treats the same way for this purpose). See `execute_macro_with_context`
/// for the function-call form (`name(args)` inside an expression), where
/// `/result` must behave exactly like `/return` instead of also echoing.
pub fn execute_macro(
    engine: &mut TfEngine,
    macro_def: &TfMacro,
    args: &[&str],
    trigger_match: Option<&TriggerMatch>,
) -> Vec<TfCommandResult> {
    execute_macro_with_context(engine, macro_def, args, trigger_match, false)
}

/// Execute a macro, distinguishing whether it was called as a command
/// (`called_as_function = false`) or as a function (`name(args)` inside an
/// expression; `called_as_function = true`). The only behavioural
/// difference this controls is `/result` (see `builtins::cmd_result` and
/// TF's own "/return and /result" help): called as a function it is
/// identical to `/return` (sets the value, no output); called as a command
/// it *also* echoes the value to tfout, which is what lets a macro like
/// TF's own `lisp.tf`'s `/car` work both as `$(/car a b c)` (echoes "a",
/// captured by the command substitution) and as `car(a, b, c)` inside an
/// expression (just the value, no echo).
pub fn execute_macro_with_context(
    engine: &mut TfEngine,
    macro_def: &TfMacro,
    args: &[&str],
    trigger_match: Option<&TriggerMatch>,
    called_as_function: bool,
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

    // Recursion guard (TF's max_recur) - checked after the condition/chance
    // gates above (a macro that never actually begins executing doesn't
    // count as a stack frame), before the scope push below so a rejected
    // call leaves the depth counter untouched.
    if engine.macro_call_depth >= MAX_MACRO_RECURSION {
        results.push(TfCommandResult::Error(format!(
            "{}: recursion count exceeded max_recur ({})",
            macro_def.name, MAX_MACRO_RECURSION
        )));
        return results;
    }
    engine.macro_call_depth += 1;

    // Push a local scope for macro execution
    engine.push_scope();

    // Set positional parameters
    for (i, arg) in args.iter().enumerate() {
        engine.set_local(&format!("{}", i + 1), TfValue::String(arg.to_string()));
    }

    // Set special variables
    engine.set_local("*", TfValue::String(args.join(" ")));
    engine.set_local("#", TfValue::Integer(args.len() as i64));
    // %0 - "the name of the executing macro" (`/help substitution`),
    // verified directly against real tf. at.tf's own usage message
    // ("/echo -e %% Usage: /%0 ...") depends on this to print "/at"
    // instead of a bare "/" - without it, %0 fell through to the generic
    // %varname lookup and substituted empty, since nothing ever set a
    // local var literally named "0".
    engine.set_local("0", TfValue::String(macro_def.name.clone()));

    // Execute body - positional params are resolved at runtime from local scope
    // (not pre-substituted) so that /shift works correctly
    let body = macro_def.body.clone();

    // Set capture groups as locals for $() inner expansion and expression access.
    // Do NOT do an eager pre-pass on the body — that would expand %P2 etc. into
    // the syntax stream before $() delimiters are parsed, breaking extraction when
    // captured text contains unbalanced parens (e.g. crypt.tf ciphertext).
    if let Some(tm) = trigger_match {
        engine.set_local("P0", TfValue::String(tm.full_match.to_string()));
        for (i, cap) in tm.captures.iter().enumerate() {
            engine.set_local(&format!("P{}", i + 1), TfValue::String(cap.to_string()));
        }
        engine.set_local("PL", TfValue::String(tm.left.to_string()));
        engine.set_local("PR", TfValue::String(tm.right.to_string()));
    }

    // Split body into execution units, preserving control flow blocks as units
    let commands = split_body_preserving_control_flow(&body);

    for cmd in commands {
        let cmd = cmd.trim();
        if cmd.is_empty() {
            continue;
        }

        // Check if this is a control flow block - if so, don't substitute here.
        // A macro's own body is substituted per-COMMAND, at CALL time, but a
        // nested /if/while/for must NOT be substituted at this level: its own
        // dispatch (cmd_if/cmd_while/cmd_for) substitutes exactly the parts
        // that need it NOW (bounds, conditions - which can reference the
        // CALLING macro's own %1../%*, already bound in this scope) and
        // defers the body text to its own per-iteration/per-branch pass
        // (control_flow::execute_for_loop/execute_while_loop/
        // execute_inline_if_block), which is what lets a /for's own loop
        // variable (not yet bound at THIS point) survive unevaluated until
        // the loop actually starts - verified directly against real tf:
        // `/def loopmac = /for i 1 3 /echo n=%i` then `/loopmac` requires
        // "%i" to reach /for's own substitution unresolved, not be
        // pre-emptied here to "" before the loop variable ever exists.
        let lower = cmd.to_lowercase();
        let is_control_flow = lower.starts_with("/while ") || lower.starts_with("/while\n")
            || lower.starts_with("/for ") || lower.starts_with("/for\n")
            || lower.starts_with("/if ") || lower.starts_with("/if\n")
            || lower.starts_with("/if(");

        let cmd = if is_control_flow {
            // Pass control flow blocks directly without substitution
            cmd.to_string()
        } else {
            // substitute_commands is the unified pass: expands %vars in plain text
            // and $() / $[] / ${} regions with correct ordering (vars inside $()
            // are expanded only after extraction, never before).
            variables::substitute_commands(engine, cmd)
        };
        let cmd = cmd.trim();

        // Execute the command (already substituted above)
        // / prefixed commands are routed through the TF engine,
        // which returns ClayCommand for Clay-specific commands like /notify
        let result = if cmd.starts_with('/') {
            super::parser::execute_command_substituted(engine, cmd)
        } else {
            TfCommandResult::SendToMud(cmd.to_string())
        };

        // Check for /return - stop executing body, set %? to return value.
        // /return never echoes, regardless of call context.
        if let TfCommandResult::Return(ref val) = result {
            let val_str = val.clone();
            engine.set_global("?", TfValue::from(val_str.as_str()));
            break;
        }

        // Check for /result - like /return, but when the macro was called
        // as a command (not as a function), it additionally echoes the
        // value to tfout (see execute_macro_with_context's doc comment).
        if let TfCommandResult::Result(ref val) = result {
            let val_str = val.clone();
            engine.set_global("?", TfValue::from(val_str.as_str()));
            if !called_as_function {
                results.push(TfCommandResult::Success(if val_str.is_empty() {
                    None
                } else {
                    Some(val_str)
                }));
            }
            break;
        }

        // /break that escaped every enclosing /while or /for (control_flow.rs's
        // 4 loop-body executors only ever re-emit this into their own results
        // when they still have levels left to unwind - see
        // `control_flow::parse_break_marker`'s doc comment) terminates macro
        // evaluation outright (`/help break`: "If used outside a /while loop,
        // the macro evaluation is terminated") - unlike /return, it sets no
        // %? and is never itself pushed into `results`, so it cannot leak any
        // further than this exact point.
        if let TfCommandResult::Error(ref e) = result {
            if control_flow::parse_break_marker(e).is_some() {
                break;
            }
        }

        // /exit during a /load aborts this macro body too ("/exit ... aborts
        // execution of all enclosing macro bodies" - `/help exit`) - pushed
        // into `results` (unlike /break above) so it keeps propagating out to
        // whichever /load actually catches it (`load_file_internal`).
        if matches!(result, TfCommandResult::ExitLoad(_)) {
            results.push(result);
            break;
        }

        results.push(result);
    }

    // Pop the local scope
    engine.pop_scope();
    engine.macro_call_depth -= 1;

    results
}

/// Find and execute all macros that match a line
pub fn process_triggers(engine: &mut TfEngine, line: &str, world: Option<&str>, world_type: Option<&str>) -> Vec<TfCommandResult> {
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

        // Check world-type restriction (-T)
        if !world_type_matches(macro_def, world_type) {
            continue;
        }

        // A quiet (-q) macro firing as a LINE trigger (as opposed to a hook - see
        // hooks::fire_hook, which does track this via HookOutcome::matched_non_quiet)
        // must not count toward the BGTRIG hook or /trigger's own return value
        // (finding C.1). /trigger still has no return-value tracking and BGTRIG still
        // never fires for a real line trigger (neither is part of Job 10's scope), so
        // there's nothing to wire `quiet` into here yet.

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

            // Execute the macro. Per `/help triggers`: "The <text> which triggers a
            // macro is given to the macro as arguments, as if it had been called with
            // `/<macro> <text>`. Positional parameters (e.g., %1) refer to the
            // corresponding word in the triggering text" - i.e. %1../%*/%# come from
            // splitting `line` on whitespace, exactly like a typed command's own
            // arguments (`hooks::fire_hook` already does this correctly for hook
            // arguments; this call used to pass an empty slice, silently leaving
            // every trigger macro's %1../%* empty - surfaced by `/trigger`'s finding
            // B rewrite, which is the first thing in this corpus to exercise a
            // trigger macro's plain %1 rather than only %Pn regexp captures).
            let words: Vec<&str> = line.split_whitespace().collect();
            let exec_results = execute_macro(engine, &macro_clone, &words, Some(&trigger_match));
            results.extend(exec_results);

            // Decrement shots if one-shot/n-shot. Compare by sequence_number, not name -
            // a nameless macro (P1.2) has name == "", and more than one can coexist, so
            // a name comparison here could decrement/remove the wrong macro.
            if idx < engine.macros.len() && engine.macros[idx].sequence_number == macro_clone.sequence_number {
                if let Some(ref mut remaining) = engine.macros[idx].shots_remaining {
                    *remaining -= 1;
                    if *remaining == 0 {
                        macros_to_remove.push(idx);
                    }
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

// =============================================================================
// /list and /purge macro-option filters (finding C.4, plan step P1.5)
// =============================================================================
//
// TF's own rule (see "/list"/"/purge" in tf-help): a macro is selected only if
// EVERY given option matches; an omitted option is "don't care". `MacroFilter`
// is parsed once from a `/list`/`/purge` argument string and then applied to
// each macro; `list_macros_with_filter` and `purge_macros_with_filter` are the
// only things that use it today, but it's written to be reusable as-is by
// `/ismacro` (Job 15) and `/save`'s own filters (Job 14).

/// Which command is parsing its option string - `/purge` never takes `-s`/`-S`
/// (they aren't in its usage grammar; real TF would also reject them there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    List,
    Purge,
}

/// How a `/list`/`/purge` string-valued option (`-t`, `-b`, `-B`, `-E`, `-T`,
/// `-w`, plus `<name>` and `= <body>`) was given.
#[derive(Debug, Clone)]
pub enum FieldFilter {
    /// The option was given with no pattern (e.g. bare `-t`): matches macros
    /// that HAVE this field set to a non-empty value, regardless of its value.
    Present,
    /// The option was given with an explicit pattern: the field's value
    /// (empty string if the macro doesn't have this field at all) must match
    /// it under the filter's matching style. This is how `-t{}`/`-b{}` (glob)
    /// or `-t^$`/`-b^$` (regexp) select macros that DON'T have the option -
    /// the "value" being matched against is simply "".
    Pattern(String),
}

impl FieldFilter {
    fn from_optional(value: Option<String>) -> Self {
        match value {
            None => FieldFilter::Present,
            Some(v) => FieldFilter::Pattern(v),
        }
    }

    fn matches(&self, value: Option<&str>, style: TfMatchMode) -> bool {
        match self {
            FieldFilter::Present => value.map(|v| !v.is_empty()).unwrap_or(false),
            FieldFilter::Pattern(pat) => full_match(value.unwrap_or(""), pat, style),
        }
    }
}

/// `-h[<event>[ <pattern>]]` (finding C.10 / plan step P1.9: `TfMacro` now
/// stores a per-hook argument pattern, so the trailing `<pattern>` is a real
/// filter component, not just grammar-compatibility).
#[derive(Debug, Clone)]
pub enum HookFilter {
    /// `-h0`: matches macros WITHOUT any hook.
    NoHook,
    /// `-h` with no argument: matches macros WITH any hook.
    AnyHook,
    /// `-h<event>` (no pattern given): matches that event regardless of the
    /// macro's own hook pattern.
    /// `-h"<event> <pattern>"`: matches that event AND requires the macro's own
    /// hook pattern to be EXACTLY `<pattern>` (same exact-string comparison
    /// `/unhook <event> <pattern>` uses - verified against real tf).
    Event(TfHookEvent, Option<String>),
}

impl HookFilter {
    fn parse(value: Option<String>) -> Result<Self, String> {
        match value {
            None => Ok(HookFilter::AnyHook),
            Some(ref v) if v == "0" => Ok(HookFilter::NoHook),
            Some(v) => {
                let mut parts = v.splitn(2, char::is_whitespace);
                let event_name = parts.next().unwrap_or("");
                let pattern = parts.next()
                    .map(|p| p.trim_start().to_string())
                    .filter(|p| !p.is_empty());
                TfHookEvent::parse(event_name)
                    .map(|ev| HookFilter::Event(ev, pattern))
                    .ok_or_else(|| format!("Unknown hook event: {}", event_name))
            }
        }
    }

    fn matches(&self, macro_def: &TfMacro) -> bool {
        match self {
            HookFilter::NoHook => macro_def.hook.is_none(),
            HookFilter::AnyHook => macro_def.hook.is_some(),
            HookFilter::Event(ev, None) => macro_def.hook == Some(*ev),
            HookFilter::Event(ev, Some(pat)) => {
                macro_def.hook == Some(*ev) && macro_def.hook_pattern.as_deref() == Some(pat.as_str())
            }
        }
    }
}

/// `-i`/`-I`. Default (neither given): plain `/list`/`/purge` semantics -
/// "all non-invisible macros".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InvisibleMode {
    #[default]
    Default,
    /// `-i`: invisible macros are matched AS WELL AS normal ones.
    IncludeAll,
    /// `-I`: ONLY invisible macros are matched.
    OnlyInvisible,
}

/// A fully-parsed `/list`/`/purge` macro-option filter. See the module-level
/// doc comment above for the selection rule ("every given option matches").
#[derive(Debug, Clone, Default)]
pub struct MacroFilter {
    pub short: bool,
    pub sort: bool,
    pub invisible: InvisibleMode,
    pub style: TfMatchMode,
    pub trigger: Option<FieldFilter>,
    pub bind: Option<FieldFilter>,
    pub bind_name: Option<FieldFilter>,
    pub expr: Option<FieldFilter>,
    pub world_type: Option<FieldFilter>,
    pub world: Option<FieldFilter>,
    pub hook: Option<HookFilter>,
    pub attrs: Option<TfAttributes>,
    pub priority: Option<i32>,
    pub shots: Option<u32>,
    pub fall_through: Option<bool>,
    pub partial_hilite: Option<bool>,
    pub quiet: Option<bool>,
    pub name: Option<FieldFilter>,
    pub name_is_number: bool,
    pub body: Option<FieldFilter>,
}

/// The matching style `/list`/`/purge` use when their own `-m` is omitted:
/// TF's `%{matching}` variable if the script has set it, else "glob" - TF's
/// own default (see `/def -m`'s help: "If omitted, the value of %{matching}
/// ('glob' by default) is used").
pub fn default_matching_style(engine: &TfEngine) -> TfMatchMode {
    engine.get_var("matching")
        .and_then(|v| TfMatchMode::parse(&v.to_string_value()))
        .unwrap_or(TfMatchMode::Glob)
}

/// Parse an optional attached option-value: `input` is everything after the
/// option letter, already confined to one whitespace-free token (see
/// `MacroFilter::parse`'s doc comment on option bundling). Empty means the
/// option was given with no value at all (the `-t`/`-b`/... "has this option,
/// don't care about its value" form).
fn parse_optional_value(input: &str) -> Result<(Option<String>, &str), String> {
    if input.is_empty() {
        return Ok((None, input));
    }
    let (value, rest) = parse_quoted_or_word(input)?;
    Ok((Some(value), rest))
}

/// Find the end of the next whitespace-delimited token in `s`, treating a
/// matching pair of double or single quotes as one unit even if it contains
/// whitespace. Plain `str::find(char::is_whitespace)` would split
/// `-h"SEND greet*"` right after "SEND" (its own space, not a token
/// separator) - Job 10's `-h"EVENT pattern"` filter routinely has exactly
/// this shape, since a hook pattern is rarely just one word. Returns
/// `s.len()` if there is no whitespace outside any quoted region (or a quote
/// is left unclosed - `parse_quoted_or_word` reports that as its own error
/// once this token's body is actually parsed).
fn quote_aware_token_end(s: &str) -> usize {
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                } else if c.is_whitespace() {
                    return i;
                }
            }
        }
    }
    s.len()
}

impl MacroFilter {
    /// Parse a `/list`/`/purge` argument string into a `MacroFilter`, per the
    /// grammar in tf-help:
    ///
    /// `[-s] [-S] [-i|-I] [-m<style>] [-t[pat]] [-b[pat]] [-B[pat]] [-E[pat]]
    /// [-T[pat]] [-h[<event>[ <pattern>]]] [-a<attrs>] [-w[world]] [-p<pri>]
    /// [-n<shots>] [-F] [-P] [-q] [-] [<name>] [= <body>]`
    ///
    /// Options bundle under one leading `-` the way getopt clusters do: each
    /// token is a run of no-value flags (`s S i I F P q`) optionally ended by
    /// ONE value-taking flag, which consumes the rest of that token as its
    /// value (quoted or bare). This is not a simplification for Clay's sake -
    /// it's required to parse stdlib.tf idioms like `-ib'^A'` (`-i` bundled
    /// with `-b'^A'`) and `-mglob -h0 -b{} -t{}` (each option its own token,
    /// value attached with no space). A bare `-` token (exactly `-`, nothing
    /// attached) ends option parsing early, so a `<name>` that itself starts
    /// with `-` can still be given.
    pub fn parse(args: &str, kind: FilterKind, default_style: TfMatchMode) -> Result<Self, String> {
        let mut filter = MacroFilter::default();
        let mut explicit_style: Option<TfMatchMode> = None;
        let mut remaining = args.trim_start();

        loop {
            if remaining.is_empty() || !remaining.starts_with('-') {
                break;
            }
            // A bare "-" token (nothing attached, or only whitespace after it)
            // ends option parsing early.
            let after_dash = &remaining[1..];
            if after_dash.is_empty() || after_dash.starts_with(char::is_whitespace) {
                remaining = after_dash.trim_start();
                break;
            }

            let token_end = quote_aware_token_end(remaining);
            let (token, rest_after_token) = remaining.split_at(token_end);
            let mut body = &token[1..]; // token starts with '-'

            while !body.is_empty() {
                let c = body.chars().next().unwrap();
                let after = &body[c.len_utf8()..];
                let mut value_consumed = false;

                match c {
                    's' => {
                        if kind == FilterKind::Purge {
                            return Err("Unknown option: -s".to_string());
                        }
                        filter.short = true;
                    }
                    'S' => {
                        if kind == FilterKind::Purge {
                            return Err("Unknown option: -S".to_string());
                        }
                        filter.sort = true;
                    }
                    'i' => filter.invisible = InvisibleMode::IncludeAll,
                    'I' => filter.invisible = InvisibleMode::OnlyInvisible,
                    'F' => filter.fall_through = Some(true),
                    'P' => filter.partial_hilite = Some(true),
                    'q' => filter.quiet = Some(true),
                    'm' => {
                        let (value, rest) = parse_word(after);
                        explicit_style = Some(
                            TfMatchMode::parse(&value)
                                .ok_or_else(|| format!("Unknown match mode: {}", value))?,
                        );
                        body = rest;
                        value_consumed = true;
                    }
                    't' => {
                        let (v, rest) = parse_optional_value(after)?;
                        filter.trigger = Some(FieldFilter::from_optional(v));
                        body = rest;
                        value_consumed = true;
                    }
                    'b' => {
                        let (v, rest) = parse_optional_value(after)?;
                        filter.bind = Some(FieldFilter::from_optional(v));
                        body = rest;
                        value_consumed = true;
                    }
                    'B' => {
                        let (v, rest) = parse_optional_value(after)?;
                        filter.bind_name = Some(FieldFilter::from_optional(v));
                        body = rest;
                        value_consumed = true;
                    }
                    'E' => {
                        let (v, rest) = parse_optional_value(after)?;
                        filter.expr = Some(FieldFilter::from_optional(v));
                        body = rest;
                        value_consumed = true;
                    }
                    'T' => {
                        let (v, rest) = parse_optional_value(after)?;
                        filter.world_type = Some(FieldFilter::from_optional(v));
                        body = rest;
                        value_consumed = true;
                    }
                    'w' => {
                        let (v, rest) = parse_optional_value(after)?;
                        filter.world = Some(FieldFilter::from_optional(v));
                        body = rest;
                        value_consumed = true;
                    }
                    'h' => {
                        let (v, rest) = parse_optional_value(after)?;
                        filter.hook = Some(HookFilter::parse(v)?);
                        body = rest;
                        value_consumed = true;
                    }
                    'a' => {
                        let (value, rest) = parse_word(after);
                        filter.attrs = Some(parse_attributes(&value)?);
                        body = rest;
                        value_consumed = true;
                    }
                    'p' => {
                        let (value, rest) = parse_word(after);
                        let pri: i32 = value.parse()
                            .map_err(|_| format!("Invalid priority: {}", value))?;
                        filter.priority = Some(pri);
                        body = rest;
                        value_consumed = true;
                    }
                    'n' => {
                        let (value, rest) = parse_word(after);
                        let n: u32 = value.parse()
                            .map_err(|_| format!("Invalid shot count: {}", value))?;
                        filter.shots = Some(n);
                        body = rest;
                        value_consumed = true;
                    }
                    _ => return Err(format!("Unknown option: -{}", c)),
                }

                if value_consumed {
                    if !body.is_empty() {
                        return Err(format!(
                            "Unexpected trailing characters after -{}: {}",
                            c, body
                        ));
                    }
                    break;
                }
                body = after;
            }

            remaining = rest_after_token.trim_start();
        }

        filter.style = explicit_style.unwrap_or(default_style);

        let remaining = remaining.trim();
        if !remaining.is_empty() {
            let (name_part, body_part) = match remaining.find('=') {
                Some(eq_pos) => (remaining[..eq_pos].trim(), Some(remaining[eq_pos + 1..].trim())),
                None => (remaining, None),
            };
            if !name_part.is_empty() {
                if let Some(number_pattern) = name_part.strip_prefix('#') {
                    filter.name_is_number = true;
                    filter.name = Some(FieldFilter::Pattern(number_pattern.to_string()));
                } else {
                    filter.name = Some(FieldFilter::Pattern(name_part.to_string()));
                }
            }
            if let Some(body_pattern) = body_part {
                filter.body = Some(FieldFilter::Pattern(body_pattern.to_string()));
            }
        }

        Ok(filter)
    }

    /// Does `macro_def` satisfy every option this filter set?
    pub fn matches(&self, macro_def: &TfMacro) -> bool {
        let invisible_ok = match self.invisible {
            InvisibleMode::Default => !macro_def.invisible,
            InvisibleMode::IncludeAll => true,
            InvisibleMode::OnlyInvisible => macro_def.invisible,
        };
        if !invisible_ok {
            return false;
        }

        if let Some(ref f) = self.trigger {
            let value = macro_def.trigger.as_ref().map(|t| t.pattern.as_str());
            if !f.matches(value, self.style) { return false; }
        }
        if let Some(ref f) = self.bind {
            if !f.matches(macro_def.keybinding.as_deref(), self.style) { return false; }
        }
        if let Some(ref f) = self.bind_name {
            if !f.matches(macro_def.keybinding.as_deref(), self.style) { return false; }
        }
        if let Some(ref f) = self.expr {
            if !f.matches(macro_def.condition.as_deref(), self.style) { return false; }
        }
        if let Some(ref f) = self.world_type {
            if !f.matches(macro_def.world_type.as_deref(), self.style) { return false; }
        }
        if let Some(ref f) = self.world {
            if !f.matches(macro_def.world.as_deref(), self.style) { return false; }
        }
        if let Some(ref hf) = self.hook {
            if !hf.matches(macro_def) { return false; }
        }
        if let Some(ref wanted) = self.attrs {
            if !attrs_overlap(&macro_def.attributes, wanted) { return false; }
        }
        if let Some(pri) = self.priority {
            if macro_def.priority != pri { return false; }
        }
        if let Some(shots) = self.shots {
            if macro_def.one_shot.unwrap_or(0) != shots { return false; }
        }
        if let Some(want) = self.fall_through {
            if macro_def.fall_through != want { return false; }
        }
        if let Some(want) = self.partial_hilite {
            if macro_def.partial_hilite != want { return false; }
        }
        if let Some(want) = self.quiet {
            if macro_def.quiet != want { return false; }
        }
        if let Some(ref f) = self.name {
            let value = if self.name_is_number {
                macro_def.sequence_number.to_string()
            } else {
                macro_def.name.clone()
            };
            if !f.matches(Some(&value), self.style) { return false; }
        }
        if let Some(ref f) = self.body {
            if !f.matches(Some(&macro_def.body), self.style) { return false; }
        }

        true
    }
}

/// Does `have` share at least one of the display attributes set in `want`?
/// TF: "-a<attrs> Matches macros having one or more of the display
/// attributes in <attrs>" - deliberately ANY, not ALL (see the worked
/// example in tf-help's own "/list" page: "-aurh ... have any of the
/// underline, reverse, or hilite attributes").
fn attrs_overlap(have: &TfAttributes, want: &TfAttributes) -> bool {
    (want.gag && have.gag)
        || (want.norecord && have.norecord)
        || (want.bold && have.bold)
        || (want.underline && have.underline)
        || (want.reverse && have.reverse)
        || (want.flash && have.flash)
        || (want.dim && have.dim)
        || (want.bell && have.bell)
        || (want.hilite.is_some() && have.hilite.is_some())
}

/// Full-string match of `value` against `pattern` under a matching style, for
/// `/list`/`/purge` field comparisons. Unlike trigger matching (a substring
/// search against a line of MUD text - `compile_pattern`/`match_trigger`),
/// TF's own /list help says simple and glob styles are "compared directly" /
/// "similar to shell filename patterns", i.e. the WHOLE field value must
/// match - which is what makes the documented "-t{}"/"-b{}" (glob) or
/// "-t^$"/"-b^$" (regexp) idiom for "doesn't have this option" work: matching
/// the empty string exactly. Regexp is intentionally NOT auto-anchored
/// (real TF doesn't anchor regexps for you either) - a caller who wants a
/// full match writes "^...$" themselves, as every regexp idiom in stdlib.tf
/// and tests/tf/cases does.
pub(crate) fn full_match(value: &str, pattern: &str, style: TfMatchMode) -> bool {
    match style {
        TfMatchMode::Simple => value == pattern,
        TfMatchMode::Glob => {
            let re_pattern = format!("^(?:{})$", glob_to_regex(pattern));
            Regex::new(&re_pattern).map(|re| re.is_match(value)).unwrap_or(false)
        }
        TfMatchMode::Regexp => {
            let re_pattern = pattern.replace("$$", "$");
            Regex::new(&re_pattern).map(|re| re.is_match(value)).unwrap_or(false)
        }
    }
}

/// Render one macro's `/list -s` short-format line: "N: [(bind) 'keys' ]
/// [(trig) 'pattern' ][(hook) EVENT ][(attr-descriptors) ]name" (a nameless
/// macro prints with nothing after its last descriptor). Modeled on real
/// TinyFugue's own `/list -s` output for the same macro shapes.
fn format_short(macro_def: &TfMacro) -> String {
    let mut line = format!("{}: ", macro_def.sequence_number);

    if let Some(ref keys) = macro_def.keybinding {
        line.push_str(&format!("(bind) '{}' ", keys));
    }
    if let Some(ref trigger) = macro_def.trigger {
        if !trigger.pattern.is_empty() {
            line.push_str(&format!("(trig) '{}' ", trigger.pattern));
        }
    }
    if let Some(hook) = macro_def.hook {
        match &macro_def.hook_pattern {
            Some(pat) => line.push_str(&format!("(hook) {} '{}' ", hook.name(), pat)),
            None => line.push_str(&format!("(hook) {} ", hook.name())),
        }
    }

    let attrs = &macro_def.attributes;
    if attrs.gag { line.push_str("(gag) "); }
    if attrs.norecord { line.push_str("(nohistory) "); }
    if attrs.underline { line.push_str("(underline) "); }
    if attrs.reverse { line.push_str("(reverse) "); }
    if attrs.bold { line.push_str("(bold) "); }
    if let Some(ref color) = attrs.hilite {
        if color == "hilite" {
            line.push_str("(hilite) ");
        } else {
            line.push_str(&format!("({}) ", color));
        }
    }

    if !macro_def.name.is_empty() {
        line.push_str(&macro_def.name);
    } else {
        // Match TF's own trailing-space-then-nothing look for a nameless macro.
        while line.ends_with(' ') {
            line.pop();
        }
    }

    line
}

/// List macros matching a filter, TF's `/list` (see `MacroFilter::parse`'s
/// doc comment for the option grammar and finding C.4 for the history: Clay's
/// `/list`/`/purge` used to ignore their arguments entirely).
/// Format one macro the full way `/list` (without `-s`) shows it: a
/// directly-pasteable `N: /def [opts] [name] = body` line, no trailing
/// newline. Extracted from `list_macros_with_filter`'s own inline loop body
/// so `/trigger -l` (`/help /trigger`: "list each macro in full, as if by
/// /list") can format the specific macros it matched without going through
/// `MacroFilter` (which matches by name/trigger/etc. pattern, not "this
/// exact set of already-selected macros").
pub(crate) fn format_macro_full(macro_def: &TfMacro) -> String {
    // Format: N: /def [opts] [name] = body (sparkle added by output system)
    format!("{}: {}", macro_def.sequence_number, format_def_line(macro_def))
}

/// Build the reloadable "/def [opts] [name] = body" text for one macro -
/// everything `format_macro_full` shows except its own leading "N: "
/// sequence-number prefix, which is `/list`-display-only and NOT valid
/// `/load` syntax. Shared with `/save` (`builtins::cmd_save`, plan Job 14c:
/// "write them in /def ... form that /load re-reads"), which is why this
/// covers every option `parse_def`/`apply_def_option` accepts (`-a`/`-b`/
/// `-w`/`-E`/`-c` included) even though `/list`'s own historical display
/// never had all of them - a macro with a keybinding or attributes used to
/// silently lose them on a /save round-trip.
pub(crate) fn format_def_line(macro_def: &TfMacro) -> String {
    let mut output = String::from("/def ");

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
    if let Some(hook) = macro_def.hook {
        match &macro_def.hook_pattern {
            Some(pat) => output.push_str(&format!("-h\"{} {}\" ", hook.name(), pat)),
            None => output.push_str(&format!("-h{} ", hook.name())),
        }
    }
    if let Some(ref wt) = macro_def.world_type {
        output.push_str(&format!("-T\"{}\" ", wt));
    }
    if let Some(ref world) = macro_def.world {
        output.push_str(&format!("-w\"{}\" ", world));
    }
    if let Some(ref cond) = macro_def.condition {
        output.push_str(&format!("-E\"{}\" ", cond));
    }
    if let Some(prob) = macro_def.probability {
        // Default is 1.0 (100%, `parse_option_char`'s own 0.0..=1.0 range) - only
        // emit -c when the macro actually restricts it.
        if prob < 1.0 {
            output.push_str(&format!("-c{} ", prob));
        }
    }

    // Attributes (-a): long-form comma-joined names, all of which
    // `parse_attributes` accepts back (see its own doc comment).
    let attrs = &macro_def.attributes;
    let mut attr_names: Vec<&str> = Vec::new();
    if attrs.gag { attr_names.push("gag"); }
    if attrs.norecord { attr_names.push("norecord"); }
    if attrs.bold { attr_names.push("bold"); }
    if attrs.underline { attr_names.push("underline"); }
    if attrs.reverse { attr_names.push("reverse"); }
    if attrs.flash { attr_names.push("flash"); }
    if attrs.dim { attr_names.push("dim"); }
    if attrs.bell { attr_names.push("bell"); }
    let hilite_name = attrs.hilite.as_ref().map(|color| format!("hilite:{}", color));
    if !attr_names.is_empty() || hilite_name.is_some() {
        let mut joined = attr_names.join(",");
        if let Some(h) = hilite_name {
            if !joined.is_empty() {
                joined.push(',');
            }
            joined.push_str(&h);
        }
        output.push_str(&format!("-a{} ", joined));
    }

    if let Some(ref keys) = macro_def.keybinding {
        output.push_str(&format!("-b\"{}\" ", keys));
    }
    if macro_def.invisible {
        output.push_str("-i ");
    }
    if macro_def.quiet {
        output.push_str("-q ");
    }

    // A nameless macro (P1.2 - addressed only by its #N) prints with nothing between
    // its flags and "= body", so the line stays an unambiguous, directly-pasteable
    // "/def [opts] = body" rather than showing a misleading empty name token.
    if !macro_def.name.is_empty() {
        output.push_str(&macro_def.name);
        output.push(' ');
    }
    output.push_str(&format!("= {}", macro_def.body));

    output
}

/// Format one macro the short way `/trigger -n` wants (`/help /trigger`:
/// "display a list of each macro that would have matched, including its
/// fallthru flag, priority, and name") - deliberately not the same shape as
/// `format_short` (which shows attributes/trig/hook text instead), since
/// tf-help spells out exactly these three fields for `-n` specifically.
pub(crate) fn format_trigger_match_summary(macro_def: &TfMacro) -> String {
    format!(
        "{}: {} pri={} {}",
        macro_def.sequence_number,
        if macro_def.fall_through { "F" } else { "-" },
        macro_def.priority,
        if macro_def.name.is_empty() { "(nameless)" } else { &macro_def.name },
    )
}

pub fn list_macros_with_filter(engine: &TfEngine, filter: &MacroFilter) -> String {
    let mut matched: Vec<&TfMacro> = engine.macros.iter().filter(|m| filter.matches(m)).collect();
    if filter.sort {
        matched.sort_by(|a, b| a.name.cmp(&b.name));
    }

    if matched.is_empty() {
        return "No macros defined.".to_string();
    }

    let mut output = String::new();
    for macro_def in matched {
        if filter.short {
            output.push_str(&format_short(macro_def));
        } else {
            output.push_str(&format_macro_full(macro_def));
        }
        output.push('\n');
    }

    output
}

/// List macros matching an optional name pattern (glob-style) - a thin
/// backward-compatible convenience over `list_macros_with_filter` for call
/// sites (bare `/def` with no args) that only ever need a name filter and
/// the `-i` show-invisible toggle.
pub fn list_macros(engine: &TfEngine, pattern: Option<&str>, show_invisible: bool) -> String {
    let mut filter = MacroFilter {
        style: TfMatchMode::Glob,
        ..Default::default()
    };
    filter.invisible = if show_invisible { InvisibleMode::IncludeAll } else { InvisibleMode::Default };
    if let Some(p) = pattern {
        filter.name = Some(FieldFilter::Pattern(p.to_string()));
    }
    list_macros_with_filter(engine, &filter)
}

/// Remove all macros matching a filter, TF's `/purge` (see `MacroFilter`'s
/// doc comment). Bare `/purge` (an all-default `MacroFilter`) reproduces the
/// old "wipe everything non-invisible" behaviour through the exact same
/// code path as a targeted purge - no special-casing needed.
pub fn purge_macros_with_filter(engine: &mut TfEngine, filter: &MacroFilter) -> usize {
    let before = engine.macros.len();
    engine.macros.retain(|m| !filter.matches(m));
    before - engine.macros.len()
}

/// Remove a macro by exact name
pub fn undef_macro(engine: &mut TfEngine, name: &str) -> bool {
    // A nameless macro (P1.2) has name == "" - never let an empty lookup match one of
    // those by accident; nameless macros are addressed only by #N.
    if name.is_empty() {
        return false;
    }
    if let Some(idx) = engine.macros.iter().position(|m| m.name == name) {
        engine.macros.remove(idx);
        true
    } else {
        false
    }
}

/// Remove the macro with the given sequence number (`/def`'s own return
/// value, `%?` - see `/help undefn`). Finding B: `/undefn` used to take a
/// NAME PATTERN instead (that's now `/purge -mglob`, per the plan's B
/// ruling) - this is TF's real `/UNDEFN <number>...` semantics.
pub fn undef_by_number(engine: &mut TfEngine, number: u32) -> bool {
    if let Some(idx) = engine.macros.iter().position(|m| m.sequence_number == number) {
        engine.macros.remove(idx);
        true
    } else {
        false
    }
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
        assert_eq!(result.hook_pattern, None);
    }

    #[test]
    fn test_parse_def_with_hook_pattern_double_and_single_quoted() {
        // -h"EVENT pattern" and -h'EVENT pattern' (finding C.10 / plan step P1.9).
        let result = parse_def("-h\"SEND greet*\" h2 = /echo hi").unwrap();
        assert_eq!(result.hook, Some(TfHookEvent::Send));
        assert_eq!(result.hook_pattern.as_deref(), Some("greet*"));

        let result2 = parse_def("-h'SEND greet*' h3 = /echo hi").unwrap();
        assert_eq!(result2.hook, Some(TfHookEvent::Send));
        assert_eq!(result2.hook_pattern.as_deref(), Some("greet*"));
    }

    #[test]
    fn test_parse_def_hook_event_case_insensitive() {
        let result = parse_def("-hloadfail lf = /echo x").unwrap();
        assert_eq!(result.hook, Some(TfHookEvent::Loadfail));
    }

    #[test]
    fn test_parse_def_invisible_flags() {
        // Both -i and -I set the same `invisible` field (finding C.1 / P1.1).
        let result = parse_def("-i inv = say hi").unwrap();
        assert!(result.invisible);

        let result = parse_def("-I inv2 = say hi").unwrap();
        assert!(result.invisible);
    }

    #[test]
    fn test_parse_def_quiet_flag() {
        let result = parse_def("-q quietmac = say hi").unwrap();
        assert!(result.quiet);
    }

    #[test]
    fn test_parse_def_dash_f_same_as_dash_a() {
        // -f is documented as "same as -a, for backward compatibility".
        let with_f = parse_def("-fg gagged_f = say hi").unwrap();
        let with_a = parse_def("-ag gagged_a = say hi").unwrap();
        assert!(with_f.attributes.gag);
        assert_eq!(with_f.attributes.gag, with_a.attributes.gag);
    }

    #[test]
    fn test_parse_def_world_type_flag() {
        let result = parse_def("-Tmud typed = say hi").unwrap();
        assert_eq!(result.world_type, Some("mud".to_string()));

        // Braced glob-alternation form, as used by TF's own stdlib.tf.
        let result = parse_def(r#"-T"{tiny|tiny.*}" typed2 = say hi"#).unwrap();
        assert_eq!(result.world_type, Some("{tiny|tiny.*}".to_string()));
    }

    // =========================================================================
    // Finding 24: /def must accept TF's bundled short options - after a
    // flag that takes no argument, the rest of the same token continues to
    // be parsed as more flags (see `parse_option_char`'s doc comment). Each
    // idiom below is a real one found in tf-lib (stdlib.tf, kbbind.tf,
    // kbfunc.tf, map.tf, tintin.tf, quoter.tf, alias.tf, watch.tf).
    // =========================================================================

    #[test]
    fn test_parse_def_bundled_i_f_p_priority() {
        // map.tf's /mark: "-iFp9999 -mglob -h'send ...' _map_hook = ..."
        let result = parse_def("-iFp9999 -mglob -h'send {n|s}' _map_hook = /echo x").unwrap();
        assert!(result.invisible);
        assert!(result.fall_through);
        assert_eq!(result.priority, 9999);
        assert_eq!(result.name, "_map_hook");
        let trigger = result.trigger.as_ref().unwrap();
        assert_eq!(trigger.match_mode, TfMatchMode::Glob);
        assert_eq!(result.hook, Some(TfHookEvent::Send));
    }

    #[test]
    fn test_parse_def_bundled_ip_priority() {
        // kbfunc.tf's own "-ip'maxpri'" idiom (quoted priority expression -
        // see test_parse_def_priority_expression below for that half) and
        // the plain numeric form "-ip2".
        let result = parse_def("-ip2 foo = bar").unwrap();
        assert!(result.invisible);
        assert_eq!(result.priority, 2);
        assert_eq!(result.name, "foo");
    }

    /// Job 15b-i / finding: stdlib.tf's own "-Fp'maxpri'" idiom (`/def
    /// -iFp'maxpri' -agG -hPROXY proxy_hook = ...`) used to error "Invalid
    /// priority: 'maxpri'" - `parse_option_char`'s 'p' arm only ever tried
    /// `value.parse::<i32>()`, but real tf's own `/help def`: "As in all
    /// numeric options, the argument to -p may be an expression that has
    /// a numeric value. E.g. '/def -pmaxpri ...' will set the macro's
    /// priority to the value of the variable maxpri." A non-numeric value
    /// is deferred as `priority_expr`, resolved by `resolve_priority_expr`
    /// (called from `cmd_def`/`cmd_edit`, which have engine access -
    /// `parse_def` itself does not).
    #[test]
    fn test_parse_def_priority_expression() {
        let result = parse_def("-p'maxpri' foo = bar").unwrap();
        assert_eq!(result.priority_expr.as_deref(), Some("maxpri"));
        // Not resolved yet - parse_def alone never touches `priority`
        // itself for a deferred expression (still its struct default)
        // until resolve_priority_expr runs.
        assert_eq!(result.priority, TfMacro::default().priority);
    }

    #[test]
    fn test_resolve_priority_expr_evaluates_against_engine() {
        let mut engine = TfEngine::new();
        engine.set_global("maxpri", TfValue::Integer(2147483647));
        let mut macro_def = parse_def("-p'maxpri' foo = bar").unwrap();
        resolve_priority_expr(&mut engine, &mut macro_def).unwrap();
        assert_eq!(macro_def.priority, 2147483647);
        assert_eq!(macro_def.priority_expr, None);

        // A plain numeric literal never even sets priority_expr, so this
        // is a no-op (verified it doesn't error or change anything).
        let mut plain = parse_def("-p10 bar = baz").unwrap();
        resolve_priority_expr(&mut engine, &mut plain).unwrap();
        assert_eq!(plain.priority, 10);
    }

    /// Job 15b-i: `%0` ("the name of the executing macro", per `/help
    /// substitution`) was never set as a local var, so at.tf's own
    /// "/echo ... /%0 ..." usage message printed a bare "/" instead of
    /// "/at" - verified directly against real tf.
    #[test]
    fn test_percent_zero_is_executing_macro_name() {
        let mut engine = TfEngine::new();
        engine.execute("/def foo = /echo name=%0");
        let result = engine.execute("/foo");
        match result {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, "name=foo"),
            other => panic!("expected Success(Some(_)), got {:?}", other),
        }
    }

    #[test]
    fn test_parse_def_bundled_ib_keybinding() {
        // kbbind.tf's pervasive "-ib'^A'" idiom: -i (no arg) then -b
        // (quoted keybinding value), bundled in one token.
        let result = parse_def("-ib'^A' = /dokey_home").unwrap();
        assert!(result.invisible);
        assert_eq!(result.keybinding, Some("^A".to_string()));
        assert_eq!(result.name, "");
    }

    #[test]
    fn test_parse_def_bundled_i_b_named_keybinding() {
        let result = parse_def("-iB'Home' = /dokey_home").unwrap();
        assert!(result.invisible);
        assert_eq!(result.keybinding, Some("Home".to_string()));
    }

    #[test]
    fn test_parse_def_b_normalizes_raw_tf_sequence() {
        // plan P2.1: -b'^[[A' (TF's raw escape sequence for the Up arrow)
        // must be stored under the same canonical name a pressed Up arrow
        // resolves to ("Up"), not the raw bytes verbatim.
        let result = parse_def("-b'^[[A' = /dokey_up").unwrap();
        assert_eq!(result.keybinding, Some("Up".to_string()));
    }

    #[test]
    fn test_parse_def_b_rejects_unparseable_key_sequence() {
        assert!(parse_def("-b'Ctrl-1' = /echo x").is_err());
    }

    #[test]
    fn test_parse_def_bundled_f_q_no_value_flags() {
        // Two no-argument flags bundled together with nothing else.
        let result = parse_def("-Fq foo = bar").unwrap();
        assert!(result.fall_through);
        assert!(result.quiet);
        assert_eq!(result.name, "foo");
    }

    #[test]
    fn test_parse_def_bundled_one_f_one_shot_and_fallthrough() {
        let result = parse_def("-1F foo = bar").unwrap();
        assert_eq!(result.one_shot, Some(1));
        assert!(result.fall_through);
    }

    #[test]
    fn test_parse_def_mglob_still_works_unbundled() {
        // -mMODE already worked before finding 24 (a single flag taking
        // the rest of the token as its value) - must keep working.
        let result = parse_def("-mglob -tfoo bar = baz").unwrap();
        assert_eq!(result.trigger.as_ref().unwrap().match_mode, TfMatchMode::Glob);
    }

    #[test]
    fn test_parse_def_bundled_i_f_p_with_hloadfail() {
        // stdlib.tf's own "-iFp'maxpri' ... -hDISCONNECT" family, and
        // kbfunc.tf's "-hloadfail"/"-hnomacro" lowercase hook names -
        // exercised together since both are common bundling idioms.
        let result = parse_def("-iFp1000 -hDISCONNECT cleanup_hook = /echo x").unwrap();
        assert!(result.invisible);
        assert!(result.fall_through);
        assert_eq!(result.priority, 1000);
        assert_eq!(result.hook, Some(TfHookEvent::Disconnect));
    }

    #[test]
    fn test_parse_def_bundled_ag_g_attributes_not_split() {
        // "-agG": 'a' takes the REST of the token as its value ("gG"),
        // i.e. two attribute letters concatenated - this is NOT bundled
        // flags in the finding-24 sense (parse_word already consumed the
        // whole thing as -a's own value before finding 24 existed), but
        // it's one of the idioms named in the task and must still work.
        let result = parse_def("-agG gagged = say hi").unwrap();
        assert!(result.attributes.gag);
        assert!(result.attributes.norecord);
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
        // Long-form names
        let attrs = parse_attributes("gag,bold,hilite:red").unwrap();
        assert!(attrs.gag);
        assert!(attrs.bold);
        assert_eq!(attrs.hilite, Some("red".to_string()));
        assert!(!attrs.underline);

        // TF single-letter codes
        let attrs = parse_attributes("g").unwrap();
        assert!(attrs.gag);
        let attrs = parse_attributes("gB").unwrap();
        assert!(attrs.gag);
        assert!(attrs.bold);

        // Combined single-letter codes
        let attrs = parse_attributes("ur").unwrap();
        assert!(attrs.underline);
        assert!(attrs.reverse);

        // Color attribute
        let attrs = parse_attributes("Cred").unwrap();
        assert_eq!(attrs.hilite, Some("red".to_string()));

        // Mixed: single-letter with comma-separated long-form
        let attrs = parse_attributes("B,gag").unwrap();
        assert!(attrs.bold);
        assert!(attrs.gag);
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

        let output = list_macros(&engine, None, false);
        // Format: N: /def [opts] name = body
        assert!(output.contains("0: /def greet = say Hello!"));
        assert!(output.contains("1: /def -t\"^You hit\" attack = kick"));
    }

    #[test]
    fn test_list_macros_hides_invisible_unless_asked() {
        let mut engine = TfEngine::new();
        engine.add_macro(parse_def("-i secret = say hi").unwrap());
        engine.add_macro(TfMacro {
            name: "visible".to_string(),
            body: "say bye".to_string(),
            ..Default::default()
        });

        // Plain /list hides the -i macro.
        let hidden = list_macros(&engine, None, false);
        assert!(!hidden.contains("secret"), "plain /list must hide an invisible macro: {hidden}");
        assert!(hidden.contains("visible"));

        // /list -i shows it (alongside normal macros - TF's -i means "as well as", not
        // "only"; -I ["only invisible"] is Job 7).
        let shown = list_macros(&engine, None, true);
        assert!(shown.contains("secret"), "/list -i must show an invisible macro: {shown}");
        assert!(shown.contains("visible"));
    }

    #[test]
    fn test_world_type_restricts_trigger_match() {
        let mut engine = TfEngine::new();
        engine.add_macro(parse_def(r#"-Tmud -t"hello" greet = /echo matched"#).unwrap());

        // Wrong world type: the -T restriction must prevent the trigger from firing.
        let results = process_triggers(&mut engine, "hello", None, Some("slack"));
        assert!(results.is_empty(), "-Tmud macro must not fire for a slack world: {results:?}");

        // Matching world type: it must fire normally.
        let results = process_triggers(&mut engine, "hello", None, Some("mud"));
        assert!(!results.is_empty(), "-Tmud macro must fire for a mud world: {results:?}");

        // Unknown/unsupplied world type: the safe default is "don't fire" (see
        // world_type_matches's doc comment).
        let results = process_triggers(&mut engine, "hello", None, None);
        assert!(results.is_empty(), "-Tmud macro must not fire when the world type is unknown: {results:?}");
    }

    #[test]
    fn test_world_type_pattern_matching_clay_worlds_never_fires() {
        // TF's own stdlib.tf binds its LOGIN hook with -T{tiny|tiny.*}; none of Clay's
        // own world types (mud/slack/discord) match that pattern, so it must simply
        // never fire (the documented "safe" behaviour - see finding C.1/C.9).
        let mut engine = TfEngine::new();
        engine.add_macro(parse_def(r#"-T"{tiny|tiny.*}" -t"connect" login = /echo connect"#).unwrap());

        for world_type in ["mud", "slack", "discord"] {
            let results = process_triggers(&mut engine, "connect", None, Some(world_type));
            assert!(results.is_empty(), "-T{{tiny|tiny.*}} must never match Clay world type {world_type:?}: {results:?}");
        }
    }

    // ===================================================================
    // glob_to_regex: "{a|b}" alternation and "[...]" character classes
    // (finding C.4 / Job 7 - needed by MacroFilter's name/trigger/etc.
    // matching, e.g. real TinyFugue's own
    // "/purge -mglob {~retry_fail_*|~retry_succ_*}" and
    // "/purge -mglob -I ~hilite_page[1-9]").
    // ===================================================================

    #[test]
    fn test_glob_to_regex_alternation() {
        assert_eq!(glob_to_regex("{}"), "(?:)");
        assert_eq!(glob_to_regex("{a|b}"), "(?:a|b)");
        // Wildcards inside an alternative are translated too.
        assert_eq!(glob_to_regex("{a*|b}c"), "(?:a(.*)|b)c");
        // An unmatched "{" is treated as a literal character rather than erroring.
        assert_eq!(glob_to_regex("{oops"), "\\{oops");
    }

    #[test]
    fn test_glob_to_regex_char_class_unchanged() {
        // Character classes ("[...]") passed straight through, unaffected by the
        // new "{...}" handling - regression check against the pre-Job-7 behaviour.
        assert_eq!(glob_to_regex("x[1-9]"), "x[1-9]");
        assert_eq!(glob_to_regex("[^a-z]"), "[^a-z]");
    }

    // ===================================================================
    // MacroFilter (/list, /purge macro-option filters - finding C.4, plan
    // step P1.5). Idioms below are taken directly from real TinyFugue's own
    // stdlib.tf/alias.tf/color.tf (see each test's doc comment).
    // ===================================================================

    fn macro_named(name: &str) -> TfMacro {
        TfMacro { name: name.to_string(), body: "/echo x".to_string(), ..Default::default() }
    }

    #[test]
    fn test_macro_filter_field_present_vs_pattern() {
        // Bare "-t" (Present): has a trigger at all, value doesn't matter.
        let mut with_trigger = macro_named("a");
        with_trigger.trigger = Some(TfTrigger { pattern: "anything".to_string(), match_mode: TfMatchMode::Glob, compiled: None });
        let without_trigger = macro_named("b");

        let filter = MacroFilter { trigger: Some(FieldFilter::Present), style: TfMatchMode::Glob, ..Default::default() };
        assert!(filter.matches(&with_trigger));
        assert!(!filter.matches(&without_trigger));

        // "-t{}" (Pattern("{}")): only a macro WITHOUT a trigger.
        let filter = MacroFilter { trigger: Some(FieldFilter::Pattern("{}".to_string())), style: TfMatchMode::Glob, ..Default::default() };
        assert!(!filter.matches(&with_trigger));
        assert!(filter.matches(&without_trigger));
    }

    #[test]
    fn test_macro_filter_attrs_any_not_all() {
        // tf-help's own worked example for /list -a: "-aurh ... have ANY of the
        // underline, reverse, or hilite attributes" - deliberately not ALL.
        let mut underline_only = macro_named("u");
        underline_only.attributes.underline = true;
        let mut reverse_only = macro_named("r");
        reverse_only.attributes.reverse = true;
        let neither = macro_named("n");

        let filter = MacroFilter {
            attrs: Some(parse_attributes("ur").unwrap()),
            style: TfMatchMode::Glob,
            ..Default::default()
        };
        assert!(filter.matches(&underline_only), "underline alone must satisfy -aur (ANY)");
        assert!(filter.matches(&reverse_only), "reverse alone must satisfy -aur (ANY)");
        assert!(!filter.matches(&neither));
    }

    #[test]
    fn test_macro_filter_sort_by_name() {
        let mut engine = TfEngine::new();
        engine.add_macro(macro_named("zebra"));
        engine.add_macro(macro_named("apple"));
        engine.add_macro(macro_named("mango"));

        let filter = MacroFilter { sort: true, style: TfMatchMode::Glob, ..Default::default() };
        let output = list_macros_with_filter(&engine, &filter);
        let apple_pos = output.find("apple").unwrap();
        let mango_pos = output.find("mango").unwrap();
        let zebra_pos = output.find("zebra").unwrap();
        assert!(apple_pos < mango_pos && mango_pos < zebra_pos, "expected alphabetical order: {output}");
    }

    #[test]
    fn test_purge_with_filter_never_deletes_more_than_selected() {
        let mut engine = TfEngine::new();
        engine.add_macro(macro_named("keep1"));
        engine.add_macro(macro_named("keep2"));
        engine.add_macro(macro_named("drop"));

        let filter = MacroFilter {
            name: Some(FieldFilter::Pattern("drop".to_string())),
            style: TfMatchMode::Glob,
            ..Default::default()
        };
        let removed = purge_macros_with_filter(&mut engine, &filter);
        assert_eq!(removed, 1);
        assert!(engine.macros.iter().any(|m| m.name == "keep1"));
        assert!(engine.macros.iter().any(|m| m.name == "keep2"));
        assert!(!engine.macros.iter().any(|m| m.name == "drop"));
    }

    #[test]
    fn test_macro_filter_purge_rejects_dash_s_and_dash_big_s() {
        // /purge's own usage grammar has no -s/-S (see "/help purge" / tf-help's
        // own "/purge" page: "same as /list" EXCEPT this) - real TF would reject
        // them too, so MacroFilter::parse must error rather than silently accept.
        assert!(MacroFilter::parse("-s", FilterKind::Purge, TfMatchMode::Glob).is_err());
        assert!(MacroFilter::parse("-S", FilterKind::Purge, TfMatchMode::Glob).is_err());
        assert!(MacroFilter::parse("-s", FilterKind::List, TfMatchMode::Glob).is_ok());
        assert!(MacroFilter::parse("-S", FilterKind::List, TfMatchMode::Glob).is_ok());
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;

    #[test]
    fn test_split_body_preserving_control_flow() {
        // Simulating crypt.tf pattern: /if...%;/else...%;/endif
        let body = "/if (cond)    cmd1%;/else    cmd2%;/endif";
        let parts = split_body_preserving_control_flow(body);

        // Should be ONE part - the entire control flow block
        assert_eq!(parts.len(), 1, "Parts: {:?}", parts);
        assert!(parts[0].contains("/if") && parts[0].contains("/endif"));
    }

    #[test]
    fn test_split_mixed_commands() {
        // Mix of regular commands and control flow
        let body = "cmd1%;/if (x)    inside%;/endif%;cmd2";
        let parts = split_body_preserving_control_flow(body);

        // Should be 3 parts: cmd1, the if block, cmd2
        assert_eq!(parts.len(), 3, "Parts: {:?}", parts);
        assert_eq!(parts[0], "cmd1");
        assert!(parts[1].contains("/if") && parts[1].contains("/endif"));
        assert_eq!(parts[2], "cmd2");
    }

    #[test]
    fn test_split_nested_control_flow() {
        // Nested /if blocks (like in crypt.tf)
        let body = "/if (a)    /if (b)    inner%;/endif%;outer%;/endif";
        let parts = split_body_preserving_control_flow(body);

        // Should be ONE part - the entire outer control flow block
        assert_eq!(parts.len(), 1, "Parts: {:?}", parts);
    }

    #[test]
    fn test_split_crypt_tf_pattern() {
        // Pattern from crypt.tf: two sequential /if blocks
        let body = "/if (a)    cmd1%;/else    cmd2%;/endif%;/if (b)    /if (c)    inner%;/else    other%;/endif%;outer%;/endif";
        let parts = split_body_preserving_control_flow(body);

        // Should be TWO parts - two separate control flow blocks
        assert_eq!(parts.len(), 2, "Parts: {:?}", parts);
        assert!(parts[0].contains("/if (a)") && parts[0].contains("/endif"));
        assert!(parts[1].contains("/if (b)") && parts[1].contains("/endif"));
    }

    #[test]
    fn test_split_listen_mush() {
        // Simulated listen_mush body from crypt.tf
        let body = r#"/if (substr({P2},0,1) =~ "\") /let dcrypt=$(/decrypt 1 x%P2x)%;/else /let dcrypt=$(/decrypt 0 x%P2x)%;/endif%;/if (dcrypt =/ "*3.14") /if (dcrypt =/ "\:*") /echo -w${world_name} -ag -- %*%;/substitute -aCred -- %% * %PL $[substr(dcrypt,strstr(dcrypt,":")+1,strlen(dcrypt)-5)]%;/else /echo -w${world_name} -ag -- %*%;/substitute -aCred -- %% %PL %P1 "$[substr(dcrypt,0,strlen(dcrypt)-4)]"%;/endif%;/endif"#;
        let parts = split_body_preserving_control_flow(body);

        // Should be TWO parts - two separate /if.../endif blocks
        assert_eq!(parts.len(), 2, "Expected 2 parts, got {}: {:?}", parts.len(), parts);
        assert!(parts[0].contains("/if (substr") && parts[0].contains("/endif"), "First block should contain first if..endif");
        assert!(parts[1].contains("/if (dcrypt =/ \"*3.14\")") && parts[1].contains("/endif"), "Second block should contain second if..endif");
    }

    #[test]
    fn test_split_escaped_percent_semi_is_not_a_separator() {
        // Finding 15: "%%;" is TF's escaped literal "%;" (percent
        // compression - see /help substitution's "%%" entry), not a
        // command separator. tick.tf's /repeat bodies rely on exactly this:
        // "/set _tick_pid1=0%%;/tick_warn" must stay ONE piece passed whole
        // to /repeat, not split into two commands at the %%;.
        let body = "/set _tick_pid1=0%%;/tick_warn%;/set _tick_pid1=%?";
        let parts = split_body_preserving_control_flow(body);
        assert_eq!(parts.len(), 2, "Parts: {:?}", parts);
        assert_eq!(parts[0], "/set _tick_pid1=0%%;/tick_warn");
        assert_eq!(parts[1], "/set _tick_pid1=%?");
    }

    #[test]
    fn test_execute_escaped_percent_semi_runs_as_one_command() {
        use super::TfEngine;

        // The "%%;" in a macro body must survive splitting intact and then
        // unescape to a literal "%;" during the normal substitution pass
        // (substitute_variables's own "%%" -> "%" handling), landing inside
        // a single command's text rather than becoming a break between two
        // commands.
        let mut engine = TfEngine::new();
        // /set (not /let) so the value is still visible after execute_macro
        // pops the macro's own local scope.
        let macro_def = TfMacro {
            name: "test".to_string(),
            body: "/set x=a%%;b".to_string(),
            ..Default::default()
        };
        execute_macro(&mut engine, &macro_def, &[], None);
        assert_eq!(
            engine.get_var("x").map(|v| v.to_string_value()),
            Some("a%;b".to_string()),
            "the escaped %%; should unescape to a literal %; inside /set's value, \
             not split /set x=a away from a trailing ;b"
        );
    }

    #[test]
    fn test_execute_nested_if_block() {
        use super::TfEngine;

        let mut engine = TfEngine::new();

        // Set up dcrypt variable with a value that should match "*3.14"
        engine.set_global("dcrypt", super::TfValue::String("foobar3.14".to_string()));

        // Simulated second part of listen_mush: nested if block
        let block = r#"/if (dcrypt =/ "*3.14") /if (dcrypt =/ "\:*") /echo COLON PATH%;/else /echo ELSE PATH: $[substr(dcrypt,0,strlen(dcrypt)-4)]%;/endif%;/endif"#;

        // Create a minimal macro to execute
        let macro_def = TfMacro {
            name: "test".to_string(),
            body: block.to_string(),
            ..Default::default()
        };

        let results = execute_macro(&mut engine, &macro_def, &[], None);

        // Should have some output
        assert!(!results.is_empty(), "Should have some results");

        // Check for the expected message (foobar3.14 - "3.14" = "foobar")
        let has_foobar = results.iter().any(|r| {
            match r {
                super::TfCommandResult::Success(Some(msg)) => msg.contains("foobar"),
                _ => false,
            }
        });
        assert!(has_foobar, "Should output 'foobar', got: {:?}", results);
    }

    #[test]
    fn test_result_as_function_and_command() {
        use super::TfEngine;

        // /def dbl = /result {1} * 2 - callable both ways per /help
        // return: "dbl(21)" (function syntax, evaluated directly the same
        // way $[...] does) must equal 42, and "$(/dbl 5)" (command syntax,
        // captured through command substitution the same way it is inside
        // a macro body) must equal "10" - /result echoes only in the
        // second case. This exercises the exact two channels /result's
        // value travels through: expressions::evaluate (what $[...]
        // resolves to) and variables::substitute_commands's "$(...)"
        // handling (what a captured command's output resolves to).
        let mut engine = TfEngine::new();
        engine.execute("/def dbl = /result {1} * 2");

        let function_value = super::super::expressions::evaluate(&mut engine, "dbl(21)")
            .expect("dbl(21) should evaluate");
        assert_eq!(
            function_value.to_string_value(), "42",
            "dbl(21) should be 42, got {:?}", function_value
        );

        let command_value = super::super::variables::substitute_commands(&mut engine, "$(/dbl 5)");
        assert_eq!(
            command_value, "10",
            "$(/dbl 5) should be \"10\", got {:?}", command_value
        );
    }

    #[test]
    fn test_result_no_expression_is_empty() {
        // "/result [<expression>]" - "If the expression is omitted, the
        // return value of the macro is the empty string" (/help return).
        let mut engine = TfEngine::new();
        engine.execute("/def empty = /result");

        let function_value = super::super::expressions::evaluate(&mut engine, "empty()")
            .expect("empty() should evaluate");
        assert_eq!(function_value.to_string_value(), "");

        let command_value = super::super::variables::substitute_commands(&mut engine, "$(/empty)");
        assert_eq!(command_value, "");
    }

    #[test]
    fn test_recursion_guard_stops_self_calling_macro() {
        // A macro whose body calls itself by name must fail cleanly with a
        // recursion-depth error instead of overflowing the real call stack
        // - this is the scenario finding 16's precedence fix makes
        // reachable for the first time: a macro named after one of Clay's
        // own builtins (e.g. "echo") whose body calls plain "/echo" now
        // recurses into itself, since a same-named macro is checked before
        // the builtin.
        let mut engine = TfEngine::new();
        engine.execute("/def echo = /echo hi");
        let result = engine.execute("/echo hi");
        match result {
            super::TfCommandResult::Error(e) => {
                assert!(
                    e.to_lowercase().contains("recursion"),
                    "expected a recursion error, got: {}",
                    e
                );
            }
            other => panic!("Expected a recursion Error, got {:?}", other),
        }
    }

    #[test]
    fn test_at_prefix_bypasses_self_calling_macro_recursion() {
        // The same shadowed "echo" macro from the test above must be able
        // to reach the real builtin via "/@echo" - the escape hatch finding
        // 16 pairs with the precedence flip.
        let mut engine = TfEngine::new();
        engine.execute("/def echo = /@echo hi");
        let result = engine.execute("/echo hi");
        match result {
            super::TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "hi"),
            other => panic!("Expected Success(Some(\"hi\")) via /@echo, got {:?}", other),
        }
    }
}

    #[test]
    fn test_list_macros_output() {
        let mut engine = TfEngine::new();

        // Add a simple macro manually
        engine.add_macro(TfMacro {
            name: "test".to_string(),
            body: "echo hello".to_string(),
            ..Default::default()
        });

        let output = list_macros(&engine, None, false);
        assert!(!output.is_empty(), "list_macros should return non-empty string");
        assert!(output.contains("test"), "Output should contain macro name");
    }
