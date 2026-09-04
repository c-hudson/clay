//! Variable storage and substitution for TinyFugue compatibility.

use super::TfEngine;

/// Get special built-in variable value (world_*, etc.)
fn get_special_var(engine: &TfEngine, name: &str) -> Option<String> {
    match name {
        // Current world info
        "world_name" => engine.current_world.clone(),
        "world_host" => {
            let current = engine.current_world.as_ref()?;
            engine.world_info_cache.iter()
                .find(|w| &w.name == current)
                .map(|w| w.host.clone())
        }
        "world_port" => {
            let current = engine.current_world.as_ref()?;
            engine.world_info_cache.iter()
                .find(|w| &w.name == current)
                .map(|w| w.port.clone())
        }
        // Real TF: "If a normal world is defined without a <character>, <pass>,
        // ... then that world will use the corresponding field of the 'default'
        // world if there is one." Clay keeps DEFAULT's character/password as
        // engine globals (`/addworld DEFAULT ...`, finding 31) rather than a
        // real world entry, so the fallback happens here instead of in
        // world_info_cache itself.
        "world_character" | "world_char" => {
            let from_world = engine.current_world.as_ref()
                .and_then(|current| engine.world_info_cache.iter().find(|w| &w.name == current))
                .map(|w| w.user.clone())
                .filter(|v| !v.is_empty());
            from_world.or_else(|| engine.default_world_character.clone())
        }
        "world_password" | "world_pass" => {
            let from_world = engine.current_world.as_ref()
                .and_then(|current| engine.world_info_cache.iter().find(|w| &w.name == current))
                .map(|w| w.password.clone())
                .filter(|v| !v.is_empty());
            from_world.or_else(|| engine.default_world_password.clone())
        }
        // Process info
        "pid" => Some(std::process::id().to_string()),
        // Time
        "time" => {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .ok()
        }
        // TF version
        "version" => Some("Clay TF 1.0".to_string()),
        // Status
        "nworlds" => Some(engine.world_info_cache.len().to_string()),
        "nactive" => Some(engine.world_info_cache.iter()
            .filter(|w| w.is_connected)
            .count()
            .to_string()),
        _ => None,
    }
}

/// Number of positional parameters in the current scope (the `%{#}` value),
/// used by the extended selectors below to compute "except first/last N".
fn arg_count(engine: &TfEngine) -> usize {
    engine.get_var("#")
        .and_then(|v| v.to_int())
        .filter(|n| *n > 0)
        .unwrap_or(0) as usize
}

/// The `idx`th (1-based) positional parameter's string value, or empty if
/// out of range - mirrors what a bare `%{idx}` substitution already does via
/// `engine.get_var`.
fn positional_arg(engine: &TfEngine, idx: usize) -> String {
    engine.get_var(&idx.to_string())
        .map(|v| v.to_string_value())
        .unwrap_or_default()
}

/// Resolve TF's "except first/last N" and "Nth from end" positional
/// selectors (see `/help substitution`): `-N` (all positional parameters
/// except the first N, joined with spaces), `L`/`LN` (the Nth positional
/// parameter from the end; bare "L" is "L1", the last one), and `-L`/`-LN`
/// (all positional parameters except the last N). These are the selector
/// forms that aren't already just a plain local-variable lookup (unlike a
/// bare name, digit, `*`, `#` or `?`, which all resolve through the normal
/// `engine.get_var` path since execute_macro already stores them as locals
/// under those exact names) - so this only ever returns `Some` for one of
/// those three shapes, and `None` for everything else, letting callers fall
/// back to their normal lookup chain.
///
/// Real TinyFugue behaviour (verified against `tf` 5.0 beta 8 directly,
/// since this is the part of finding C.5 the earlier investigation got
/// backwards - see this job's report): for arguments "a b c d", `{-1}` is
/// "b c d" (except the first one), not "d" - `-N` is never "Nth from end"
/// singular, that's what `LN` means.
pub(crate) fn resolve_extended_selector(engine: &TfEngine, selector: &str) -> Option<String> {
    let argc = arg_count(engine);

    if let Some(rest) = selector.strip_prefix("-L") {
        // "-L" / "-LN" - all positional parameters except the last N (N
        // defaults to 1 when omitted, so "-L" == "-L1").
        let n: usize = if rest.is_empty() {
            1
        } else {
            rest.parse().ok()?
        };
        if n == 0 || n >= argc {
            return Some(String::new());
        }
        let keep = argc - n;
        return Some((1..=keep).map(|i| positional_arg(engine, i)).collect::<Vec<_>>().join(" "));
    }

    if let Some(rest) = selector.strip_prefix('-') {
        // "-N" - all positional parameters except the first N. Only a pure
        // digit run counts (this must not swallow "-L..." above, nor an
        // unrelated selector that merely starts with '-').
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            let n: usize = rest.parse().ok()?;
            if n == 0 {
                return None;
            }
            if n >= argc {
                return Some(String::new());
            }
            return Some((n + 1..=argc).map(|i| positional_arg(engine, i)).collect::<Vec<_>>().join(" "));
        }
        return None;
    }

    if let Some(rest) = selector.strip_prefix('L') {
        // "L" / "LN" - the Nth positional parameter from the end (1 = last).
        if rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit()) {
            let n: usize = if rest.is_empty() { 1 } else { rest.parse().ok()? };
            if n == 0 || n > argc {
                return Some(String::new());
            }
            return Some(positional_arg(engine, argc - n + 1));
        }
        return None;
    }

    None
}

/// Split `%{...}`/`{...}` content into `(selector, default)` per TF's
/// `%{selector-default}` grammar (see `/help substitution`): the selector is
/// recognised first by consuming its own known shape (a leading "-"
/// optionally followed by "L", then any digit run; or "L" then a digit run;
/// or a plain run of identifier characters otherwise), and only a "-" found
/// immediately *after* that counts as the default separator. This is what
/// keeps a selector that itself starts with "-" (the "-N"/"-LN" family) from
/// being misparsed as "empty selector, default N" - and keeps a plain named
/// selector's own default (e.g. the "%{2-stack}" idiom TF's stack-q.tf uses
/// to pick a caller-supplied variable name or fall back to a fixed one)
/// working the same way.
pub(crate) fn split_selector_default(content: &str) -> (&str, Option<&str>) {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    if i < len && bytes[i] == b'-' {
        i += 1;
        if i < len && bytes[i] == b'L' {
            i += 1;
        }
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    } else if i < len && bytes[i] == b'L' {
        i += 1;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
    }

    if i < len && bytes[i] == b'-' {
        (&content[..i], Some(&content[i + 1..]))
    } else {
        (content, None)
    }
}

/// Resolve a `%{selector}` selector (WITHOUT applying its `-default`, if
/// any - callers that need the default do that themselves, since the
/// default's own text may need substitution the caller is responsible for)
/// to its current value, or an empty string if nothing matches. Shared by
/// `substitute_variables`'s own `%{...}` handling and
/// `substitute_commands`' (see its own doc comment), applying the same
/// lookup chain both times: extended selector (`-N`/`L`/`-L`), a special
/// var, a plain local/global variable, or a simple (trigger/hook-less)
/// macro's body.
pub(crate) fn resolve_braced_selector(engine: &TfEngine, selector: &str) -> String {
    // %{Pn}/%{PL}/%{PR}/%{P*} - regex capture groups from the last
    // successful regexp match, same as the bare %Pn form's own dedicated
    // match arm above (which this braced form otherwise has no access to,
    // since it falls straight to the generic get_var chain below - a real
    // gap: at.tf's own "%{P1-$[ftime(...)]}" idiom, verified directly
    // against real tf, depends on %{P1} reading the capture the same way
    // %P1 does).
    if let Some(rest) = selector.strip_prefix('P') {
        if let Ok(idx) = rest.parse::<usize>() {
            return if idx < engine.regex_captures.len() {
                engine.regex_captures[idx].clone()
            } else {
                // Trigger captures stored as locals by execute_macro
                engine.get_var(selector).map(|v| v.to_string_value()).unwrap_or_default()
            };
        }
        match rest {
            "L" | "R" => return engine.get_var(selector).map(|v| v.to_string_value()).unwrap_or_default(),
            "*" => {
                return if engine.regex_captures.is_empty() {
                    String::new()
                } else {
                    engine.regex_captures[1..].join(" ")
                };
            }
            _ => {}
        }
    }
    if let Some(v) = resolve_extended_selector(engine, selector) {
        v
    } else if let Some(v) = get_special_var(engine, selector) {
        v
    } else if let Some(v) = engine.get_var(selector) {
        v.to_string_value()
    } else if let Some(macro_def) = engine.macros.iter().find(|m|
        m.name == selector && m.trigger.is_none() && m.hook.is_none()
    ) {
        // Fall back to simple macros (no trigger, no hook)
        macro_def.body.clone()
    } else {
        // If nothing matched, substitute empty string (TF behavior)
        String::new()
    }
}

/// Perform variable substitution on text.
///
/// Supports:
/// - `%{varname}` - Standard TF variable substitution
/// - `%varname` - Short form (ends at non-alphanumeric)
/// - `%%` - Literal percent sign
pub fn substitute_variables(engine: &TfEngine, text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // \% -> % (escape sequence to output literal percent sign)
        if chars[i] == '\\' && i + 1 < len && chars[i + 1] == '%' {
            result.push('%');
            i += 2;
        } else if chars[i] == '%' {
            // TinyFugue's own escaping rule for a run of consecutive '%'
            // characters (verified directly against real tf 5.0 beta 8 -
            // see this job's report): a run is a live substitution
            // introducer only when its length is EXACTLY one. A run of N
            // >= 2 collapses to (N - 1) literal '%' characters, and
            // whatever follows (a "{...}" block, a bare name, ";", ...) is
            // left completely untouched by THIS pass - it is the NEXT
            // substitution pass (one per nesting level - e.g. each level
            // of a nested `/for`, per tf-help /for's own "%%{...}" example
            // and color.tf's triple-nested-for "%%%{red}" idiom) that
            // peels off one more '%' and gets one level closer to a bare,
            // evaluated single '%'. This is NOT a simple pairwise "%%"
            // collapse repeated until nothing is left - real tf leaves a
            // 3-run as 2 literal '%'s (not "collapse one pair, then
            // evaluate the leftover single %"), which is exactly the
            // distinction that used to make Clay evaluate one nesting
            // level too early.
            let run_len = {
                let mut n = 1;
                while i + n < len && chars[i + n] == '%' {
                    n += 1;
                }
                n
            };
            if run_len > 1 {
                for _ in 0..run_len - 1 {
                    result.push('%');
                }
                i += run_len;
                continue;
            }
            if i + 1 < len {
                match chars[i + 1] {
                    // %{selector} / %{selector-default} form
                    '{' => {
                        if let Some((content, end_idx)) = extract_braced_var(&chars, i + 2) {
                            let (selector, default) = split_selector_default(&content);
                            let mut value = resolve_braced_selector(engine, selector);
                            // TF: "If the substitution determined by the selector
                            // would be empty, and a default value is given, the
                            // default will be substituted instead" - the default
                            // text itself can carry further substitutions.
                            if value.is_empty() {
                                if let Some(def) = default {
                                    value = substitute_variables(engine, def);
                                }
                            }
                            result.push_str(&value);
                            i = end_idx + 1;
                        } else {
                            // Malformed, keep as-is
                            result.push('%');
                            i += 1;
                        }
                    }
                    // %* - all positional parameters
                    '*' => {
                        if let Some(value) = engine.get_var("*") {
                            result.push_str(&value.to_string_value());
                        }
                        i += 2;
                    }
                    // %# - argument count
                    '#' => {
                        if let Some(value) = engine.get_var("#") {
                            result.push_str(&value.to_string_value());
                        } else {
                            result.push('0');
                        }
                        i += 2;
                    }
                    // %L / %LN - last positional parameter / Nth from the end
                    // (must precede the alphabetic arm below, or "%L2" would
                    // parse as a lookup of a variable literally named "L2").
                    'L' => {
                        let mut j = i + 2;
                        while j < len && chars[j].is_ascii_digit() {
                            j += 1;
                        }
                        let selector: String = chars[i + 1..j].iter().collect();
                        if let Some(value) = resolve_extended_selector(engine, &selector) {
                            result.push_str(&value);
                        }
                        i = j;
                    }
                    // %R - a positional parameter at random. Unrelated to the
                    // "-N"/"L" family above (no digit suffix); kept as a
                    // plain variable lookup, matching its pre-existing
                    // (limited) behavior - out of scope for this job.
                    'R' => {
                        if let Some(value) = engine.get_var("R") {
                            result.push_str(&value.to_string_value());
                        }
                        i += 2;
                    }
                    // %-N / %-L / %-LN - "except first/last N" bare forms.
                    '-' => {
                        let mut j = i + 2;
                        let valid = if j < len && chars[j] == 'L' {
                            j += 1;
                            while j < len && chars[j].is_ascii_digit() {
                                j += 1;
                            }
                            true
                        } else {
                            let digit_start = j;
                            while j < len && chars[j].is_ascii_digit() {
                                j += 1;
                            }
                            j > digit_start
                        };
                        if valid {
                            let selector: String = chars[i + 1..j].iter().collect();
                            if let Some(value) = resolve_extended_selector(engine, &selector) {
                                result.push_str(&value);
                            }
                            i = j;
                        } else {
                            // Not a recognised selector (e.g. a bare "%-" at
                            // end of text, or "%-foo") - leave the "-" for
                            // the next pass to treat as plain text.
                            result.push('%');
                            i += 1;
                        }
                    }
                    // %P forms for capture groups (%P0-%P9, %PL, %PR, %P*).
                    // Must precede the general alphabetic arm so that %P2x is parsed as
                    // %P2 (capture) + x, not as variable name "P2x".
                    'P' if i + 2 < len => {
                        match chars[i + 2] {
                            c @ '0'..='9' => {
                                let idx = (c as usize) - ('0' as usize);
                                if idx < engine.regex_captures.len() {
                                    result.push_str(&engine.regex_captures[idx]);
                                } else if let Some(value) = engine.get_var(&format!("P{}", c)) {
                                    // Trigger captures stored as locals by execute_macro
                                    result.push_str(&value.to_string_value());
                                }
                                i += 3;
                            }
                            'L' => {
                                if let Some(value) = engine.get_var("PL") {
                                    result.push_str(&value.to_string_value());
                                }
                                i += 3;
                            }
                            'R' => {
                                if let Some(value) = engine.get_var("PR") {
                                    result.push_str(&value.to_string_value());
                                }
                                i += 3;
                            }
                            '*' => {
                                // All captures joined
                                if !engine.regex_captures.is_empty() {
                                    result.push_str(&engine.regex_captures[1..].join(" "));
                                }
                                i += 3;
                            }
                            _ => {
                                // Not a special %Pn form — fall through to general variable lookup.
                                // e.g. %Pfoo looks up variable "Pfoo".
                                let (var_name, end_idx) = extract_simple_var(&chars, i + 1);
                                if let Some(value) = engine.get_var(&var_name) {
                                    result.push_str(&value.to_string_value());
                                }
                                i = end_idx;
                            }
                        }
                    }
                    // %varname form - variable name is alphanumeric + underscore
                    c if c.is_alphabetic() || c == '_' => {
                        let (var_name, end_idx) = extract_simple_var(&chars, i + 1);
                        if let Some(value) = engine.get_var(&var_name) {
                            let val = value.to_string_value();
                            result.push_str(&val);
                        }
                        // If variable not found, substitute empty string
                        i = end_idx;
                    }
                    // %n (digit) - positional parameter, optionally followed
                    // by a literal "-default" clause exactly like the
                    // braced "%{n-default}" form (verified directly against
                    // real tf 5.0 beta 8 - see this job's report on
                    // stack-q.tf's own "/pop %1-queue"): "%1-queue" with %1
                    // unset expands to "queue"; with %1="X" it expands to
                    // just "X", never "X-queue" - the "-" and default text
                    // are consumed as part of the substitution regardless
                    // of whether the default ends up used. The default text
                    // runs until the next substitution introducer ('%' or
                    // '$' - which the OUTER scan then continues to process
                    // on its own either way, so a trailing "%2" in
                    // "%1-foo%2" is not swallowed into the default) or one
                    // of a small set of syntactically-significant
                    // characters real tf's own parser treats as a natural
                    // stop (quotes, parens/braces/brackets, ';', ':') -
                    // verified none of these are absorbed into the default
                    // text even with nothing else to delimit it. Plain
                    // prose - letters, digits, spaces, periods,
                    // underscores - is NOT a stop condition: "%1-hello
                    // world" with %1 unset expands to the full two-word
                    // "hello world", space included.
                    c if c.is_ascii_digit() => {
                        let var_name = c.to_string();
                        let after_digit = i + 2;
                        if after_digit < len && chars[after_digit] == '-' {
                            let default_start = after_digit + 1;
                            let mut j = default_start;
                            while j < len
                                && chars[j] != '%'
                                && chars[j] != '$'
                                && !matches!(chars[j], ')' | '}' | ']' | '(' | '{' | '[' | ';' | ':' | '"' | '\'')
                            {
                                j += 1;
                            }
                            let default_text: String = chars[default_start..j].iter().collect();
                            let value = engine.get_var(&var_name).map(|v| v.to_string_value()).unwrap_or_default();
                            if value.is_empty() {
                                result.push_str(&substitute_variables(engine, &default_text));
                            } else {
                                result.push_str(&value);
                            }
                            i = j;
                        } else {
                            if let Some(value) = engine.get_var(&var_name) {
                                result.push_str(&value.to_string_value());
                            }
                            // If not found, substitute empty string (TF behavior)
                            i = after_digit;
                        }
                    }
                    // %? - "the string return value of the most recently executed
                    // command" (`/help %?`; a real predefined variable, not an
                    // arbitrary name, so it needs its own single-char arm the same
                    // as %#/%* above - the generic %varname arm below never reaches
                    // it, since '?' is neither alphabetic nor '_'). Set by /def's own
                    // return value (finding B - `/undefn %?` is the documented idiom
                    // for removing the macro /def just created), /test, /trigger,
                    // /not, and elsewhere via `engine.set_global("?", ...)`.
                    '?' => {
                        if let Some(value) = engine.get_var("?") {
                            result.push_str(&value.to_string_value());
                        }
                        i += 2;
                    }
                    // Unknown, keep literal
                    _ => {
                        result.push('%');
                        i += 1;
                    }
                }
            } else {
                // Trailing %, keep as-is
                result.push('%');
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Extract variable name from %{varname} form.
/// Returns (variable_name, index_of_closing_brace) or None if malformed.
fn extract_braced_var(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut i = start;

    while i < chars.len() {
        match chars[i] {
            '}' => return Some((name, i)),
            c => {
                name.push(c);
                i += 1;
            }
        }
    }

    None  // No closing brace found
}

/// Extract variable name from %varname form.
/// Returns (variable_name, index_after_last_char).
fn extract_simple_var(chars: &[char], start: usize) -> (String, usize) {
    let mut name = String::new();
    let mut i = start;

    while i < chars.len() {
        let c = chars[i];
        if c.is_alphanumeric() || c == '_' {
            name.push(c);
            i += 1;
        } else {
            break;
        }
    }

    (name, i)
}

/// Substitute positional parameters (%1-%9, %*, %L, %R) in macro body.
/// Used when executing macros/actions with arguments.
pub fn substitute_positional(text: &str, args: &[&str]) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '%' && i + 1 < len {
            match chars[i + 1] {
                // %1-%9 positional parameters
                c @ '1'..='9' => {
                    let idx = (c as usize) - ('1' as usize);
                    if idx < args.len() {
                        result.push_str(args[idx]);
                    }
                    i += 2;
                }
                // %0 is the macro name (not typically used in body)
                '0' => {
                    i += 2;
                }
                // %* all arguments
                '*' => {
                    result.push_str(&args.join(" "));
                    i += 2;
                }
                // %# number of arguments
                '#' => {
                    result.push_str(&args.len().to_string());
                    i += 2;
                }
                // %R random argument
                'R' => {
                    if !args.is_empty() {
                        let idx = (super::expressions::simple_random() as usize) % args.len();
                        result.push_str(args[idx]);
                    }
                    i += 2;
                }
                // %P forms for regex capture groups
                'P' if i + 2 < len => {
                    match chars[i + 2] {
                        // %Pn positional capture
                        c @ '0'..='9' => {
                            // Will be handled with captures parameter
                            result.push('%');
                            result.push('P');
                            result.push(c);
                            i += 3;
                        }
                        // %PL left of match
                        'L' => {
                            result.push_str("%PL");
                            i += 3;
                        }
                        // %PR right of match
                        'R' => {
                            result.push_str("%PR");
                            i += 3;
                        }
                        // %P* all captures
                        '*' => {
                            result.push_str("%P*");
                            i += 3;
                        }
                        _ => {
                            result.push('%');
                            i += 1;
                        }
                    }
                }
                _ => {
                    result.push('%');
                    i += 1;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Substitute regex capture groups in text.
/// %P0 is full match, %P1-%P9 are capture groups.
pub fn substitute_captures(text: &str, full_match: &str, captures: &[&str], left: &str, right: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '%' && i + 1 < len && chars[i + 1] == 'P' && i + 2 < len {
            match chars[i + 2] {
                '0' => {
                    result.push_str(full_match);
                    i += 3;
                }
                c @ '1'..='9' => {
                    let idx = (c as usize) - ('1' as usize);
                    if idx < captures.len() {
                        result.push_str(captures[idx]);
                    }
                    i += 3;
                }
                'L' => {
                    result.push_str(left);
                    i += 3;
                }
                'R' => {
                    result.push_str(right);
                    i += 3;
                }
                '*' => {
                    result.push_str(&captures.join(" "));
                    i += 3;
                }
                _ => {
                    result.push(chars[i]);
                    i += 1;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Unified substitution pass: processes `%var`, `$()`, `$[]`, `${}`  in one walk.
///
/// This is the single entry point for all substitution.  Callers must NOT run
/// `substitute_variables` separately before calling this — that would expand
/// `%P2` (and other captured text) into the syntax stream before `$()` delimiters
/// are parsed, allowing user data with unbalanced parens to corrupt extraction.
///
/// Invariants:
/// 1. Variables inside `$(...)` are expanded *only* after `extract_balanced` has
///    delimited the region from the raw input, so capture content never interferes
///    with paren counting.
/// 2. The output of any `$(...)` is final — appended directly to the result and
///    never seen by another substitution pass.
pub fn substitute_commands(engine: &mut TfEngine, text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    // Plain-text buffer: accumulated between protected regions, flushed through
    // substitute_variables just before each region (and at end of input).
    let mut plain = String::new();
    let mut i = 0;

    while i < len {
        if chars[i] == '\\' && i + 1 < len {
            match chars[i + 1] {
                // \$( or \$[ -> emit backslash to plain, then let $(/$[ be processed next
                '$' => {
                    if i + 2 < len && (chars[i + 2] == '(' || chars[i + 2] == '[') {
                        plain.push('\\');
                        i += 1;
                    } else {
                        // \$ followed by anything else -> literal $, bypass plain buffer
                        if !plain.is_empty() {
                            result.push_str(&substitute_variables(engine, &plain));
                            plain.clear();
                        }
                        result.push('$');
                        i += 2;
                    }
                }
                // \\ -> literal \, bypass plain buffer so it's never re-processed
                '\\' => {
                    if !plain.is_empty() {
                        result.push_str(&substitute_variables(engine, &plain));
                        plain.clear();
                    }
                    result.push('\\');
                    i += 2;
                }
                _ => {
                    plain.push('\\');
                    i += 1;
                }
            }
        } else if chars[i] == '$' {
            // Same escaping rule as substitute_variables' '%' handling
            // above (see its doc comment): a run of N >= 2 consecutive '$'
            // collapses to (N - 1) literal '$' characters, deferring
            // whatever follows to a later substitution pass, and only a
            // bare run of exactly one '$' is a live "$(...)"/"$[...]"/
            // "${...}" introducer - needed by color.tf's own
            // "$$$[16 + red*36 + ...]" triple-nested-for idiom.
            let run_len = {
                let mut n = 1;
                while i + n < len && chars[i + n] == '$' {
                    n += 1;
                }
                n
            };
            if run_len > 1 {
                for _ in 0..run_len - 1 {
                    plain.push('$');
                }
                i += run_len;
                continue;
            }
            if i + 1 >= len {
                plain.push(chars[i]);
                i += 1;
                continue;
            }
            match chars[i + 1] {
                // $(...) - command substitution
                '(' => {
                    if let Some((cmd, end_idx)) = extract_balanced(&chars, i + 2, '(', ')') {
                        // Flush plain text before the $() region
                        if !plain.is_empty() {
                            result.push_str(&substitute_variables(engine, &plain));
                            plain.clear();
                        }
                        // Fully substitute the extracted content now, after safe
                        // extraction - not just %vars: a $(...) can itself contain
                        // another $(...)/$[...]/${...} (lisp.tf's own `/unique`
                        // recurses exactly this way, via
                        // "$(/unique $(/remove %1 %-1))"), and the invoked macro's
                        // own argument-splitting (execute_command_impl's macro
                        // branch, parser.rs) never substitutes args_str itself - it
                        // assumes whoever built the command line already did, the
                        // same assumption execute_tf_command's non-macro dispatch
                        // path relies on. Using substitute_variables here (as this
                        // used to) left an inner "$(...)" completely unresolved,
                        // and parse_macro_args then split its raw, un-executed text
                        // into bogus positional words instead of the inner command's
                        // actual output (verified directly: this call used to
                        // return the literal string "$(/cdr" for
                        // "$(/car $(/cdr a b c))", not "a").
                        let cmd = substitute_commands(engine, &cmd);
                        // Execute and append output as final literal — never re-substituted
                        let output = execute_for_substitution(engine, &cmd);
                        result.push_str(&output);
                        i = end_idx + 1;
                    } else {
                        plain.push('$');
                        i += 1;
                    }
                }
                // $[...] - expression substitution
                '[' => {
                    if let Some((expr, end_idx)) = extract_balanced(&chars, i + 2, '[', ']') {
                        if !plain.is_empty() {
                            result.push_str(&substitute_variables(engine, &plain));
                            plain.clear();
                        }
                        let expr = substitute_dollar_braces(engine, &expr);
                        if let Ok(value) = super::expressions::evaluate(engine, &expr) {
                            result.push_str(&value.to_string_value());
                        }
                        i = end_idx + 1;
                    } else {
                        plain.push('$');
                        i += 1;
                    }
                }
                // ${varname} - variable substitution (TF syntax)
                // Also checks simple macros (macros with no trigger/hook act like variables)
                '{' => {
                    if let Some((var_name, end_idx)) = extract_balanced(&chars, i + 2, '{', '}') {
                        if !plain.is_empty() {
                            result.push_str(&substitute_variables(engine, &plain));
                            plain.clear();
                        }
                        if let Some(value) = engine.get_var(&var_name) {
                            result.push_str(&value.to_string_value());
                        } else if let Some(macro_def) = engine.macros.iter().find(|m|
                            m.name == var_name && m.trigger.is_none() && m.hook.is_none()
                        ) {
                            result.push_str(&macro_def.body);
                        }
                        i = end_idx + 1;
                    } else {
                        plain.push('$');
                        i += 1;
                    }
                }
                _ => {
                    plain.push(chars[i]);
                    i += 1;
                }
            }
        } else if chars[i] == '%' && i + 1 < len && chars[i + 1] == '{'
            && (i == 0 || chars[i - 1] != '%')
        {
            // %{selector-default} whose DEFAULT itself contains a $(...)/
            // $[...]/${...} region - e.g. at.tf's own
            // "%{P1-$[ftime(\"%Y\")/100]}" idiom. This function's usual
            // strategy (see its own doc comment) flushes the plain-text
            // buffer right before each $-region, which would otherwise
            // hand substitute_variables just "%{P1-" (the closing "}" is
            // on the far side of the "$[...]", in a LATER flush) - an
            // unterminated %{...} it can only leave completely literal.
            // Extracting the WHOLE %{...} span up front, before any flush,
            // and resolving it as one unit (recursing into just the
            // default text, through this same function, for its own %/$
            // substitution) fixes that.
            //
            // Only an UNESCAPED "%{" (not preceded by another '%') is
            // intercepted here - an escaped "%%{...}" (deferred a level,
            // e.g. for a nested /for - see substitute_variables' own doc
            // comment on the escaping rule) is left for the plain-buffer
            // path below exactly as before, since a still-escaped
            // construct isn't ready to be resolved on this pass anyway.
            if let Some((content, end_idx)) = extract_braced_var(&chars, i + 2) {
                if !plain.is_empty() {
                    result.push_str(&substitute_variables(engine, &plain));
                    plain.clear();
                }
                let (selector, default) = split_selector_default(&content);
                let mut value = resolve_braced_selector(engine, selector);
                if value.is_empty() {
                    if let Some(def) = default {
                        value = substitute_commands(engine, def);
                    }
                }
                result.push_str(&value);
                i = end_idx + 1;
            } else {
                plain.push(chars[i]);
                i += 1;
            }
        } else {
            plain.push(chars[i]);
            i += 1;
        }
    }

    // Flush any remaining plain text
    if !plain.is_empty() {
        result.push_str(&substitute_variables(engine, &plain));
    }

    result
}

/// Extract content between balanced delimiters (handles nesting)
pub(crate) fn extract_balanced(chars: &[char], start: usize, open: char, close: char) -> Option<(String, usize)> {
    let mut content = String::new();
    let mut depth = 1;
    let mut i = start;

    while i < chars.len() {
        if chars[i] == open {
            depth += 1;
            content.push(chars[i]);
        } else if chars[i] == close {
            depth -= 1;
            if depth == 0 {
                return Some((content, i));
            }
            content.push(chars[i]);
        } else {
            content.push(chars[i]);
        }
        i += 1;
    }

    None // Unbalanced
}

/// Substitute ${varname} with variable/macro values inside an expression
/// This is used to pre-process expressions before evaluation
/// String values are quoted so they're parsed as string literals, not identifiers
pub(crate) fn substitute_dollar_braces(engine: &TfEngine, text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '$' && i + 1 < len && chars[i + 1] == '{' {
            // ${varname} - extract and substitute
            if let Some((var_name, end_idx)) = extract_balanced(&chars, i + 2, '{', '}') {
                // First check variables
                if let Some(value) = engine.get_var(&var_name) {
                    // Quote string values so they're parsed as literals
                    match value {
                        super::TfValue::String(s) => {
                            // Escape any quotes in the string
                            let escaped = s.replace('"', "\\\"");
                            result.push('"');
                            result.push_str(&escaped);
                            result.push('"');
                        }
                        super::TfValue::Integer(n) => {
                            result.push_str(&n.to_string());
                        }
                        super::TfValue::Float(f) => {
                            result.push_str(&f.to_string());
                        }
                    }
                } else {
                    // Fall back to simple macros (no trigger, no hook)
                    if let Some(macro_def) = engine.macros.iter().find(|m|
                        m.name == var_name && m.trigger.is_none() && m.hook.is_none()
                    ) {
                        // Quote macro body as a string literal
                        let escaped = macro_def.body.replace('"', "\\\"");
                        result.push('"');
                        result.push_str(&escaped);
                        result.push('"');
                    }
                    // If neither found, substitute empty string (as quoted empty)
                }
                i = end_idx + 1;
            } else {
                result.push('$');
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Execute a command for substitution and return its output
pub(crate) fn execute_for_substitution(engine: &mut TfEngine, cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return String::new();
    }

    // Execute the command
    let result = if cmd.starts_with('/') {
        super::parser::execute_command(engine, cmd)
    } else {
        // Non-command text - just return it as-is
        return cmd.to_string();
    };

    // Extract output from result
    let output = match result {
        super::TfCommandResult::Success(Some(msg)) => msg,
        super::TfCommandResult::Success(None) => String::new(),
        // A bare "/result" dispatched directly here (not through
        // execute_macro, e.g. "$(/result foo)" typed with no enclosing
        // macro) never goes through execute_macro's own command-vs-
        // function handling, so it arrives as this raw variant - treat it
        // the same as an echoed Success, matching /result's "called as a
        // command" rule (see builtins::cmd_result's doc comment).
        super::TfCommandResult::Result(val) => val,
        super::TfCommandResult::Error(e) => format!("[error: {}]", e),
        super::TfCommandResult::SendToMud(text) => {
            // Queue this to be sent later
            engine.pending_commands.push(super::TfCommand {
                command: text,
                world: None,
                no_eol: false,
            });
            String::new()
        }
        _ => String::new(),
    };

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tf::TfValue;

    #[test]
    fn test_substitute_braced_var() {
        let mut engine = TfEngine::new();
        engine.set_global("foo", TfValue::String("bar".to_string()));
        engine.set_global("num", TfValue::Integer(42));

        assert_eq!(substitute_variables(&engine, "hello %{foo} world"), "hello bar world");
        assert_eq!(substitute_variables(&engine, "value is %{num}"), "value is 42");
        assert_eq!(substitute_variables(&engine, "%{foo}%{num}"), "bar42");
        assert_eq!(substitute_variables(&engine, "%{undefined}"), "");
    }

    #[test]
    fn test_substitute_simple_var() {
        let mut engine = TfEngine::new();
        engine.set_global("foo", TfValue::String("bar".to_string()));
        engine.set_global("x", TfValue::Integer(5));

        assert_eq!(substitute_variables(&engine, "hello %foo world"), "hello bar world");
        assert_eq!(substitute_variables(&engine, "%x + %x = 10"), "5 + 5 = 10");
        assert_eq!(substitute_variables(&engine, "%foo.txt"), "bar.txt");
    }

    #[test]
    fn test_substitute_percent_escape() {
        let engine = TfEngine::new();
        assert_eq!(substitute_variables(&engine, "100%%"), "100%");
        // A run of N consecutive '%' collapses to (N - 1) literal '%'
        // characters - NOT a simple pairwise "%%" -> "%" halving repeated
        // until nothing is left. Verified directly against real tf 5.0
        // beta 8 (`/eval /echo [%%%%]` -> "[%%%]", not "[%%]") - see this
        // job's report and substitute_variables' own doc comment for the
        // full derivation (it's what makes a triply-nested command-form
        // `/for`'s "%%%{var}" resolve at the right nesting level instead
        // of one level too early).
        assert_eq!(substitute_variables(&engine, "%%%%"), "%%%");
        // \% outputs % (escape sequence to output literal percent sign)
        assert_eq!(substitute_variables(&engine, r"\%b"), "%b");
        assert_eq!(substitute_variables(&engine, r"say \%b here"), "say %b here");
    }

    #[test]
    fn test_substitute_positional() {
        let args = vec!["one", "two", "three"];
        assert_eq!(substitute_positional("arg1=%1 arg2=%2", &args), "arg1=one arg2=two");
        assert_eq!(substitute_positional("all=%*", &args), "all=one two three");
        assert_eq!(substitute_positional("count=%#", &args), "count=3");
        assert_eq!(substitute_positional("%9 is empty", &args), " is empty");
    }

    #[test]
    fn test_substitute_default_value() {
        // %{selector-default}: the default is substituted only when the
        // selector's own value would be empty (unset counts as empty) -
        // /help substitution: "%{1-foofle}" is the first word if there is
        // one, or "foofle" if not.
        let mut engine = TfEngine::new();
        assert_eq!(substitute_variables(&engine, "%{1-DEF}"), "DEF");
        engine.set_global("1", TfValue::String("hello".to_string()));
        assert_eq!(substitute_variables(&engine, "%{1-DEF}"), "hello");
        engine.set_global("1", TfValue::String(String::new()));
        assert_eq!(substitute_variables(&engine, "%{1-DEF}"), "DEF");
        // %{name-} - an explicit, empty default (same observable result as
        // no default at all, just written differently).
        assert_eq!(substitute_variables(&engine, "%{5-}"), "");
        // stack-q.tf's own idiom: %{2-stack} picks a caller-given name (arg
        // 2) or falls back to the fixed name "stack" when arg 2 is absent.
        assert_eq!(substitute_variables(&engine, "%{2-stack}"), "stack");
    }

    #[test]
    fn test_substitute_last_and_except_forms() {
        // %L/%LN ("Nth positional parameter from the end"), %-N ("all
        // positional parameters except the first N") and %-L/%-LN ("all
        // except the last N") - verified against real tf 5.0 beta 8
        // directly (see resolve_extended_selector's doc comment): for args
        // "a b c d", %-1 is "b c d" (except the first), NOT "d" (which is
        // %L1/%L).
        let mut engine = TfEngine::new();
        engine.set_global("#", TfValue::Integer(4));
        engine.set_global("1", TfValue::String("a".to_string()));
        engine.set_global("2", TfValue::String("b".to_string()));
        engine.set_global("3", TfValue::String("c".to_string()));
        engine.set_global("4", TfValue::String("d".to_string()));

        assert_eq!(substitute_variables(&engine, "%L"), "d");
        assert_eq!(substitute_variables(&engine, "%L2"), "c");
        assert_eq!(substitute_variables(&engine, "%{L}"), "d");
        assert_eq!(substitute_variables(&engine, "%{L2}"), "c");

        assert_eq!(substitute_variables(&engine, "%-1"), "b c d");
        assert_eq!(substitute_variables(&engine, "%-2"), "c d");
        assert_eq!(substitute_variables(&engine, "%{-1}"), "b c d");
        assert_eq!(substitute_variables(&engine, "%{-2}"), "c d");

        assert_eq!(substitute_variables(&engine, "%-L"), "a b c");
        assert_eq!(substitute_variables(&engine, "%-L2"), "a b");
        assert_eq!(substitute_variables(&engine, "%{-L}"), "a b c");
        assert_eq!(substitute_variables(&engine, "%{-L2}"), "a b");
    }

    #[test]
    fn test_substitute_captures() {
        let captures = vec!["group1", "group2"];
        assert_eq!(
            substitute_captures("matched %P0, first=%P1", "fullmatch", &captures, "left", "right"),
            "matched fullmatch, first=group1"
        );
        assert_eq!(
            substitute_captures("%PL[%P0]%PR", "MATCH", &captures, "before ", " after"),
            "before [MATCH] after"
        );
    }

    #[test]
    fn test_substitute_commands_escape() {
        let mut engine = TfEngine::new();
        // \$( outputs backslash AND allows command substitution (TF behavior)
        // This is how crypt.tf's \\$[char(x)] outputs \ + char result
        // $(test) with non-command text returns the text itself
        assert_eq!(substitute_commands(&mut engine, r"say \$(test)"), r"say \test");
        // \$[ also outputs backslash and allows expression substitution
        assert_eq!(substitute_commands(&mut engine, r"say \$[2+2]"), r"say \4");
        // \$ followed by non-( and non-[ becomes literal $
        assert_eq!(substitute_commands(&mut engine, r"say \$var"), "say $var");
        // \\ should become literal \
        assert_eq!(substitute_commands(&mut engine, r"say \\hello"), r"say \hello");
    }

    #[test]
    fn test_substitute_commands_expression() {
        let mut engine = TfEngine::new();
        // $[expr] should evaluate expression
        assert_eq!(substitute_commands(&mut engine, "value is $[2 + 3]"), "value is 5");
        assert_eq!(substitute_commands(&mut engine, "$[strlen(\"hello\")]"), "5");
    }

    #[test]
    fn test_substitute_commands_nested() {
        let mut engine = TfEngine::new();
        // Nested parentheses should work
        assert_eq!(substitute_commands(&mut engine, "$[max(1, min(5, 3))]"), "3");
    }

    #[test]
    fn test_plain_text_var_substitution() {
        let mut engine = TfEngine::new();
        engine.set_global("foo", TfValue::String("VAL".to_string()));
        // %var in plain text is expanded by the plain-buffer flush path
        let out = substitute_commands(&mut engine, "before %foo $[1+1] after");
        assert_eq!(out, "before VAL 2 after");
    }

    #[test]
    fn test_dollar_paren_output_is_literal() {
        let mut engine = TfEngine::new();
        engine.set_global("foo", TfValue::String("VAL".to_string()));
        // Macro emits literal "%foo". After $() resolves, that text must NOT be
        // expanded again to "VAL" — $() output is final.
        super::super::parser::execute_command(&mut engine, "/def emitpct = /echo -- %%foo");
        let out = substitute_commands(&mut engine, "$(/emitpct)");
        assert_eq!(out, "%foo");
    }

    #[test]
    fn test_dollar_paren_capture_with_unbalanced_paren() {
        let mut engine = TfEngine::new();
        // Simulate a trigger capture containing unbalanced '(' — the core crypt.tf bug.
        // %P2 must be expanded AFTER $() extraction so its parens don't confuse
        // extract_balanced.
        engine.set_global("P2", TfValue::String("(_data'".to_string()));
        super::super::parser::execute_command(&mut engine, "/def echoback = /echo -- got:%*");
        let out = substitute_commands(&mut engine, "$(/echoback x%P2x)");
        assert_eq!(out, "got:x(_data'x");
    }

    #[test]
    fn test_let_via_dollar_paren_with_paren_in_arg() {
        let mut engine = TfEngine::new();
        super::super::parser::execute_command(&mut engine, "/def myid = /echo -- %*");
        engine.set_global("P2", TfValue::String("a(b".to_string()));
        // Full pipeline: /let result=$(/myid x%P2x) with unbalanced ( in P2
        super::super::parser::execute_command(&mut engine, "/let result=$(/myid x%P2x)");
        let val = engine.get_var("result").unwrap().to_string_value();
        assert_eq!(val, "xa(bx");
    }

    /// Finding 31 / plan Job 14b: `/addworld DEFAULT <char> <pass>` sets a fallback
    /// used by `${world_character}`/`${world_password}` for any world missing its own.
    #[test]
    fn test_world_character_password_fall_back_to_default() {
        use crate::tf::WorldInfoCache;

        let mut engine = TfEngine::new();
        engine.default_world_character = Some("hero".to_string());
        engine.default_world_password = Some("secret".to_string());

        // No current world at all: still falls back.
        assert_eq!(substitute_variables(&engine, "%{world_character}"), "hero");
        assert_eq!(substitute_variables(&engine, "%{world_password}"), "secret");

        // Current world exists but has no character/password of its own: falls back.
        engine.current_world = Some("MyMUD".to_string());
        engine.world_info_cache = vec![WorldInfoCache {
            name: "MyMUD".to_string(),
            ..Default::default()
        }];
        assert_eq!(substitute_variables(&engine, "%{world_character}"), "hero");
        assert_eq!(substitute_variables(&engine, "%{world_password}"), "secret");

        // A world with its OWN character/password wins over the default.
        engine.world_info_cache[0].user = "alice".to_string();
        engine.world_info_cache[0].password = "hunter2".to_string();
        assert_eq!(substitute_variables(&engine, "%{world_character}"), "alice");
        assert_eq!(substitute_variables(&engine, "%{world_password}"), "hunter2");
    }

    /// Job 15b-i: real TF's escaping rule for a run of N consecutive '%'
    /// (verified directly against real tf 5.0 beta 8, `/eval /echo
    /// [%{x}] [%%{x}] [%%%{x}] [%%%%{x}] [%%%%%{x}]` -> "[5] [%{x}]
    /// [%%{x}] [%%%{x}] [%%%%{x}]" with x=5): only a run of EXACTLY one
    /// '%' is a live substitution introducer; a run of N >= 2 collapses to
    /// (N - 1) literal '%' characters and whatever follows is left
    /// completely untouched THIS pass - NOT a simple pairwise "%%" -> "%"
    /// collapse repeated until nothing is left (that would evaluate an
    /// odd-length run one level too early, which used to make color.tf's
    /// own triple-nested-for "%%%{red}" resolve at the wrong nesting
    /// depth - see tests/tf/xfail.txt's lib_color entry).
    #[test]
    fn test_percent_escape_run_length_table() {
        let mut engine = TfEngine::new();
        engine.set_global("x", TfValue::Integer(5));
        assert_eq!(substitute_variables(&engine, "[%{x}]"), "[5]");
        assert_eq!(substitute_variables(&engine, "[%%{x}]"), "[%{x}]");
        assert_eq!(substitute_variables(&engine, "[%%%{x}]"), "[%%{x}]");
        assert_eq!(substitute_variables(&engine, "[%%%%{x}]"), "[%%%{x}]");
        assert_eq!(substitute_variables(&engine, "[%%%%%{x}]"), "[%%%%{x}]");
    }

    /// Same rule, for the "$" sigil (verified directly against real tf:
    /// `/eval /echo [$[1+1]] [$$[1+1]] [$$$[1+1]] [$$$$[1+1]]` -> "[2]
    /// [$[1+1]] [$$[1+1]] [$$$[1+1]]") - needed by color.tf's own
    /// "$$$[16 + red*36 + ...]" triple-nested-for idiom.
    #[test]
    fn test_dollar_escape_run_length_table() {
        let mut engine = TfEngine::new();
        assert_eq!(substitute_commands(&mut engine, "[$[1+1]]"), "[2]");
        assert_eq!(substitute_commands(&mut engine, "[$$[1+1]]"), "[$[1+1]]");
        assert_eq!(substitute_commands(&mut engine, "[$$$[1+1]]"), "[$$[1+1]]");
        assert_eq!(substitute_commands(&mut engine, "[$$$$[1+1]]"), "[$$$[1+1]]");
    }

    /// A triple-nested command-form `/for` (color.tf's own rgb-cube idiom)
    /// needs exactly one extra level of "%" escaping per level of
    /// nesting for its own loop variable to resolve at the right depth -
    /// verified directly against real tf (`/for a 0 1 /for b 0 1 /for c
    /// 0 1 /echo done_%%%{a}_%%%{b}_%%%{c}` -> "done_0_0_0" ...
    /// "done_1_1_1", ALL THREE vars needing the SAME 3-level escaping
    /// regardless of which loop owns them). This only resolves correctly
    /// because `control_flow::execute_for_loop` substitutes a nested
    /// `/for`'s own header and body text once per ENCLOSING iteration
    /// (previously it skipped substitution entirely for any body line
    /// that looked like a nested `/for`, deferring completely to the
    /// inner loop's own single pass - one pass short for nesting deeper
    /// than one level).
    #[test]
    fn test_nested_for_needs_one_escape_level_per_nesting_level() {
        let mut engine = TfEngine::new();
        let result = engine.execute("/for a 0 1 /for b 0 1 /echo two_%%{a}_%%{b}");
        let text = match result {
            super::super::TfCommandResult::Success(Some(s)) => s,
            other => panic!("expected Success(Some(_)), got {:?}", other),
        };
        assert_eq!(text, "two_0_0\ntwo_0_1\ntwo_1_0\ntwo_1_1");

        let mut engine3 = TfEngine::new();
        let result3 = engine3.execute(
            "/for a 0 1 /for b 0 1 /for c 0 1 /echo done_%%%{a}_%%%{b}_%%%{c}",
        );
        let text3 = match result3 {
            super::super::TfCommandResult::Success(Some(s)) => s,
            other => panic!("expected Success(Some(_)), got {:?}", other),
        };
        assert_eq!(
            text3,
            "done_0_0_0\ndone_0_0_1\ndone_0_1_0\ndone_0_1_1\n\
             done_1_0_0\ndone_1_0_1\ndone_1_1_0\ndone_1_1_1"
        );
    }

    /// at.tf's own "%{P1-$[ftime(\"%Y\")/100]}" idiom: a `%{selector-
    /// default}` whose DEFAULT itself contains a `$[...]` expression.
    /// `substitute_commands`' usual strategy (flush the plain-text buffer
    /// right before each `$`-region) used to hand `substitute_variables`
    /// just "%{P1-" (the closing "}" is on the far side of the "$[...]",
    /// in a LATER flush) - an unterminated %{...} it could only leave
    /// completely literal, so at.tf's own "year" ended up as the literal
    /// text "%{P1-20}%{P2-26}" instead of a real value.
    #[test]
    fn test_braced_selector_default_containing_dollar_expression() {
        let mut engine = TfEngine::new();
        // Selector "missing" resolves to nothing, so the default - itself
        // a $[...] expression - must be substituted through this same
        // function (not left as literal text).
        assert_eq!(
            substitute_commands(&mut engine, "%{missing-$[1+1]}"),
            "2"
        );
        // When the selector DOES resolve, the default (and its $[...])
        // must not even be evaluated.
        engine.set_global("present", TfValue::String("hi".to_string()));
        assert_eq!(
            substitute_commands(&mut engine, "%{present-$[1+1]}"),
            "hi"
        );
        // An escaped "%%{...}" defers the SELECTOR to a later pass
        // (matching the unbraced-form escaping rule) - but a "$[...]"
        // inside its default is a DIFFERENT sigil, evaluated
        // independently regardless of the outer "%" escaping. Verified
        // directly against real tf: "%%{missing-$[1+1]}" -> "%{missing-2}",
        // not "%{missing-$[1+1]}".
        assert_eq!(
            substitute_commands(&mut engine, "%%{missing-$[1+1]}"),
            "%{missing-2}"
        );
    }

    /// `%{Pn}`/`%{PL}`/`%{PR}` (the BRACED form of a regex capture
    /// reference) must read `engine.regex_captures` the same way the bare
    /// `%Pn` form already does - at.tf's own "%{P1-...}"/"%{P2-...}"
    /// depend on this after a `regmatch()` call populates the captures.
    #[test]
    fn test_braced_capture_group_selector() {
        let mut engine = TfEngine::new();
        engine.regex_captures = vec![
            "2099-01-01".to_string(),
            "20".to_string(),
            "99".to_string(),
        ];
        assert_eq!(substitute_variables(&engine, "%{P0}"), "2099-01-01");
        assert_eq!(substitute_variables(&engine, "%{P1}"), "20");
        assert_eq!(substitute_variables(&engine, "%{P2}"), "99");
        // Out-of-range capture with a default still falls through to it.
        assert_eq!(substitute_variables(&engine, "%{P5-none}"), "none");
    }

    /// Real TF imports the WHOLE process environment as TF global
    /// variables at startup, not just the handful with documented special
    /// meaning (verified directly against real tf with an arbitrary,
    /// TF-meaningless env var) - stdlib.tf's own "isvar" macro
    /// (`/listvar -msimple -- %*`) depends on this for `isvar("HOME")`.
    /// PATH is universally set in any test environment, so use it as a
    /// deterministic stand-in rather than mutating process env state.
    #[test]
    fn test_engine_new_imports_environment_variables() {
        let engine = TfEngine::new();
        let path_from_env = std::env::var("PATH").expect("PATH should be set in the test environment");
        assert_eq!(
            engine.global_vars.get("PATH").map(|v| v.to_string_value()),
            Some(path_from_env)
        );
    }
}
