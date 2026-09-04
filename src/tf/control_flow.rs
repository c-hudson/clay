//! Control flow structures for TinyFugue compatibility.
//!
//! Implements:
//! - Single-line: /if (expr) command
//! - Multi-line: /if (expr) ... /elseif (expr) ... /else ... /endif
//! - Loops: /while (expr) ... /done, /for var start end [step] ... /done
//! - Loop control: /break

use super::expressions;
use super::{TfEngine, TfCommandResult, TfValue};

/// Maximum iterations for loops to prevent infinite loops
pub const MAX_ITERATIONS: usize = 10000;

/// Build the `TfCommandResult::Error` marker text `/break [n]` uses to signal
/// an unwind (`/help break`: "unconditionally terminates the nearest
/// enclosing /WHILE loop. If <n> is specified, it will break out of <n>
/// enclosing /WHILE loops." - real TF's own wording covers /for too, per
/// `/help for`). A plain `TfCommandResult::Error` already doubles as Clay's
/// internal control-signal channel for this - a real user-facing error can
/// never legitimately be this exact text - so every site that used to filter
/// the old bare `"__break__"` string out of error display/propagation now
/// recognizes this whole family via `parse_break_marker` instead. `n` is
/// floored at 1 (`/break 0` and `/break -5` both behave like a bare
/// `/break` - verified directly against real tf, same floor `/exit`'s own
/// count uses).
pub(crate) fn break_marker(n: u32) -> String {
    format!("__break__:{}", n.max(1))
}

/// Parse a `TfCommandResult::Error`'s text as a break marker (see
/// `break_marker`), returning the remaining unwind count. `None` for any
/// other error text.
pub(crate) fn parse_break_marker(e: &str) -> Option<u32> {
    e.strip_prefix("__break__:")?.parse().ok()
}

/// Execute a body line from control flow (while/for/if).
/// Plain text (not starting with / or #) is sent to the MUD.
fn execute_body_line(engine: &mut TfEngine, line: &str) -> TfCommandResult {
    let trimmed = line.trim();
    if trimmed.starts_with('/') || trimmed.starts_with('#') {
        super::parser::execute_command_substituted(engine, trimmed)
    } else {
        TfCommandResult::SendToMud(trimmed.to_string())
    }
}

/// State for tracking multi-line control structures
#[derive(Debug, Clone, Default)]
pub enum ControlState {
    /// Not in a control structure
    #[default]
    None,
    /// Collecting lines for an if/elseif/else block
    If(IfState),
    /// Collecting lines for a while loop
    While(WhileState),
    /// Collecting lines for a for loop
    For(ForState),
}

/// State for multi-line if/elseif/else/endif
#[derive(Debug, Clone)]
pub struct IfState {
    /// Conditions for each branch (if, elseif, elseif, ...)
    pub conditions: Vec<String>,
    /// Bodies for each branch (parallel to conditions, plus one for else)
    pub bodies: Vec<Vec<String>>,
    /// Current branch index being collected
    pub current_branch: usize,
    /// Whether we've seen /else
    pub has_else: bool,
    /// Nesting depth for nested if statements
    pub depth: usize,
}

impl IfState {
    pub fn new(condition: String) -> Self {
        IfState {
            conditions: vec![condition],
            bodies: vec![vec![]],
            current_branch: 0,
            has_else: false,
            depth: 1,
        }
    }
}

/// State for while loops
#[derive(Debug, Clone)]
pub struct WhileState {
    /// Loop condition expression
    pub condition: String,
    /// Collected loop body
    pub body: Vec<String>,
    /// Nesting depth for nested loops
    pub depth: usize,
}

impl WhileState {
    pub fn new(condition: String) -> Self {
        WhileState {
            condition,
            body: vec![],
            depth: 1,
        }
    }
}

/// State for for loops
#[derive(Debug, Clone)]
pub struct ForState {
    /// Loop variable name
    pub var_name: String,
    /// Start value
    pub start: i64,
    /// End value
    pub end: i64,
    /// Step value (default 1)
    pub step: i64,
    /// Collected loop body
    pub body: Vec<String>,
    /// Nesting depth for nested loops
    pub depth: usize,
}

impl ForState {
    pub fn new(var_name: String, start: i64, end: i64, step: i64) -> Self {
        ForState {
            var_name,
            start,
            end,
            step,
            body: vec![],
            depth: 1,
        }
    }
}

/// Result of processing a control flow line
#[derive(Debug)]
pub enum ControlResult {
    /// Line was consumed by control flow (keep collecting)
    Consumed,
    /// Control structure completed, execute these commands
    Execute(Vec<String>),
    /// Error in control flow
    Error(String),
    /// Not a control flow command
    NotControlFlow,
}

/// Whether the "/for" token at `words[idx]` opens TF's own self-contained
/// `/for <var> <min> <max> <command>` form (finding C.7/P1.7) rather than
/// Clay's numeric `/for var start end [step] ... /done` extension. The
/// command-form never has a matching `/done` anywhere (tf-help /for
/// documents no such thing - <command> is simply the rest of the line) -
/// so every depth-counter in this module and macros.rs must NOT treat it
/// as an opener awaiting a future closer, or a body containing one (e.g.
/// tf-lib's testcolor.tf, whose "/for" command text itself embeds further
/// nested command-form "/for"s) gets wrongly merged with every sibling
/// line/piece after it while searching for a "/done" that will never come,
/// in practice an unbounded pathological merge across MAX_ITERATIONS on
/// every nesting level. Mirrors split_for_command_form's own "is the 4th
/// token an integer" heuristic, operating on an already-tokenized word
/// list (as these counters all do) instead of raw text.
pub(crate) fn is_self_contained_for(words: &[&str], idx: usize) -> bool {
    match words.get(idx + 4) {
        None => false, // no 4th token at all - Clay's numeric form (no step)
        Some(tok) => tok.parse::<i64>().is_err(),
    }
}

/// Count if/while/for openers and closers in a line, returning (if_opens, loop_opens, if_closes, loop_closes)
fn count_control_keywords(text: &str) -> (i32, i32, i32, i32) {
    let lower = text.to_lowercase();
    let mut if_opens = 0;
    let mut loop_opens = 0;
    let mut if_closes = 0;
    let mut loop_closes = 0;

    let words: Vec<&str> = lower.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        if *word == "/if" || word.starts_with("/if(") {
            if_opens += 1;
        } else if *word == "/while" || word.starts_with("/while(") {
            loop_opens += 1;
        } else if *word == "/for" {
            if !is_self_contained_for(&words, i) {
                loop_opens += 1;
            }
        } else if *word == "/endif" {
            if_closes += 1;
        } else if *word == "/done" {
            loop_closes += 1;
        }
    }

    (if_opens, loop_opens, if_closes, loop_closes)
}

/// Group body lines into executable units.
/// Lines that form control flow blocks (/if.../endif, /while.../done, /for.../done)
/// are collected together as a single unit. Other lines remain separate.
pub fn group_body_lines(body: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < body.len() {
        let line = &body[i];
        let trimmed = line.trim();

        // Count control flow keywords in this line
        let (if_opens, loop_opens, if_closes, loop_closes) = count_control_keywords(trimmed);

        // Check if this starts a control flow block
        if if_opens > 0 || loop_opens > 0 {
            // Track depths separately for if blocks and loop blocks
            let mut if_depth = if_opens - if_closes;
            let mut loop_depth = loop_opens - loop_closes;
            let mut block_lines = vec![trimmed.to_string()];
            i += 1;

            while i < body.len() && (if_depth > 0 || loop_depth > 0) {
                let inner = body[i].trim();
                let (inner_if_opens, inner_loop_opens, inner_if_closes, inner_loop_closes) = count_control_keywords(inner);

                if_depth += inner_if_opens - inner_if_closes;
                loop_depth += inner_loop_opens - inner_loop_closes;

                block_lines.push(inner.to_string());
                i += 1;
            }

            // Join the block lines into a single unit
            result.push(block_lines.join("\n"));
        } else {
            // Single line (no control flow openers)
            result.push(trimmed.to_string());
            i += 1;
        }
    }

    result
}

/// Parse a single-line /if: /if (condition) command
/// Returns Some((condition, command)) if valid, None otherwise
pub fn parse_single_line_if(args: &str) -> Option<(String, String)> {
    let args = args.trim();

    // Must start with (
    if !args.starts_with('(') {
        return None;
    }

    // Find matching closing paren
    let mut depth = 0;
    let mut end_paren = None;
    for (i, c) in args.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end_paren = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let end_paren = end_paren?;
    let condition = args[1..end_paren].trim().to_string();
    let rest = args[end_paren + 1..].trim();

    // If there's content after the condition, it might be a single-line if
    // But if the content contains /else, /elseif, or /endif, it's actually
    // a multi-line if that was joined via line continuation
    if !rest.is_empty() {
        let rest_lower = rest.to_lowercase();
        // Check for control flow keywords that indicate multi-line structure
        // Need to check for these as standalone commands (preceded by ; or %)
        if contains_control_flow_keyword(&rest_lower) {
            return None;  // Treat as multi-line
        }
        Some((condition, rest.to_string()))
    } else {
        None
    }
}

/// Check if a string contains control flow keywords (/else, /elseif, /endif)
/// that indicate it's a multi-line if block
fn contains_control_flow_keyword(text: &str) -> bool {
    // Check for /else, /elseif, /endif as commands (not inside strings)
    // Look for patterns like ";/else", "%;/else", or just "/else" at start
    let keywords = ["/else", "/elseif", "/endif"];
    for keyword in &keywords {
        // Check at start
        if let Some(after) = text.strip_prefix(keyword) {
            if after.is_empty() || after.starts_with(|c: char| c.is_whitespace() || c == ';' || c == '%') {
                return true;
            }
        }
        // Check after semicolon or %;
        for sep in [";", "%;"] {
            if let Some(idx) = text.find(&format!("{}{}", sep, keyword)) {
                let after_idx = idx + sep.len() + keyword.len();
                let after = &text[after_idx..];
                if after.is_empty() || after.starts_with(|c: char| c.is_whitespace() || c == ';' || c == '%') {
                    return true;
                }
            }
        }
    }
    false
}

/// Parse the condition from a multi-line /if or /elseif
pub fn parse_condition(args: &str) -> Result<String, String> {
    let args = args.trim();

    if !args.starts_with('(') {
        return Err("Condition must be enclosed in parentheses".to_string());
    }

    // Find matching closing paren
    let mut depth = 0;
    let mut end_paren = None;
    for (i, c) in args.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end_paren = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    match end_paren {
        Some(i) => Ok(args[1..i].trim().to_string()),
        None => Err("Unclosed parenthesis in condition".to_string()),
    }
}

/// Parse /for arguments: var start end [step]
pub fn parse_for_args(args: &str) -> Result<(String, i64, i64, i64), String> {
    let parts: Vec<&str> = args.split_whitespace().collect();

    if parts.len() < 3 {
        return Err("/for requires: var start end [step]".to_string());
    }

    let var_name = parts[0].to_string();
    let start: i64 = parts[1].parse()
        .map_err(|_| format!("Invalid start value: {}", parts[1]))?;
    let end: i64 = parts[2].parse()
        .map_err(|_| format!("Invalid end value: {}", parts[2]))?;
    let step: i64 = if parts.len() > 3 {
        parts[3].parse()
            .map_err(|_| format!("Invalid step value: {}", parts[3]))?
    } else if start <= end {
        1
    } else {
        -1
    };

    if step == 0 {
        return Err("Step cannot be zero".to_string());
    }

    Ok((var_name, start, end, step))
}

/// Split `/for`'s raw, still-unsubstituted argument text (everything after
/// "/for ") into TinyFugue's OWN `/for <var> <min> <max> <command>` form
/// (finding C.7/P1.7 - see tf-help /for) - returns `None` when this is
/// instead Clay's numeric `/for var start end [step]` extension (a bare
/// integer, or nothing at all, as the 4th token), so the caller keeps using
/// `parse_for_args` for that.
///
/// `min`/`max` come back unsubstituted (the caller substitutes them once,
/// same as Clay's own numeric form always has); `command` is the untouched
/// remainder of the line - byte-sliced from the original text rather than
/// rejoined from whitespace-split tokens, so quoted/spaced content inside
/// it survives intact. It MUST reach execution unsubstituted: TF's /for
/// re-expands the command fresh on every iteration (its own stdlib
/// implementation is a `/while` loop over it), which is what lets "%i" see
/// each iteration's value instead of whatever "i" held (or didn't) before
/// the loop ever ran - see execute_tf_command's is_control_flow_command
/// gate, which is what keeps this text from being substituted before it
/// ever reaches here.
pub fn split_for_command_form(args: &str) -> Option<(String, String, String, String)> {
    let mut token_spans: Vec<(usize, usize)> = Vec::new();
    let mut rest = args;
    let mut base = 0usize;
    for _ in 0..4 {
        let ws_len = rest.len() - rest.trim_start().len();
        base += ws_len;
        rest = &rest[ws_len..];
        if rest.is_empty() {
            break;
        }
        let tok_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
        token_spans.push((base, base + tok_len));
        base += tok_len;
        rest = &rest[tok_len..];
    }

    if token_spans.len() < 4 {
        // No 4th token at all - Clay's numeric form (with or without an
        // explicit [step]).
        return None;
    }

    let fourth = &args[token_spans[3].0..token_spans[3].1];
    if fourth.parse::<i64>().is_ok() {
        // A literal integer 4th token is Clay's own [step] - never a
        // command, which always starts with "/".
        return None;
    }

    let var_name = args[token_spans[0].0..token_spans[0].1].to_string();
    let min_text = args[token_spans[1].0..token_spans[1].1].to_string();
    let max_text = args[token_spans[2].0..token_spans[2].1].to_string();
    let command_text = args[token_spans[3].0..].to_string();
    Some((var_name, min_text, max_text, command_text))
}

/// True when `text` (the raw text right after `/if`/`/while`/`/elseif`,
/// already trimmed of leading whitespace by the caller - or, for a
/// command-form condition list already split by `%;`, one piece of it) is
/// TinyFugue's command-form condition ("/if /command%; /then ..." -
/// finding C.8/P1.8) rather than the parenthesized expression form
/// ("/if (expr) ..."). tf-help "evaluation": a line starting with "/" is a
/// command; here that means the *condition itself* is a command instead of
/// an `(expr)`.
pub fn is_command_form_condition(text: &str) -> bool {
    text.trim_start().starts_with('/')
}

/// Evaluate a /if or /while condition that is either a parenthesized TF
/// expression (`(expr)`, evaluated as always) or TF's command-form
/// condition (one or more "%;"-joined commands, stored joined the same
/// way by execute_inline_if_block/execute_inline_while_block - see their
/// own doc comments). Returns the condition's TF return status (also left
/// in `%?`, matching real TF - tf-help "evaluation": every command has a
/// return value) plus any of the executed commands' own results (echoed
/// text, queued MUD sends, ...) that the caller must fold into its own
/// results instead of discarding, since - per tf-help /if and /while -
/// each command in the list genuinely runs, not just the last one.
///
/// For the command-form: per tf-help "evaluation", "the return value of a
/// <list> is the return value of the last command executed in the list" -
/// so every piece runs (side effects included), and the LAST one's status
/// is the list's truth value.
pub fn evaluate_condition(
    engine: &mut TfEngine,
    condition: &str,
) -> Result<(TfValue, Vec<TfCommandResult>), String> {
    if !is_command_form_condition(condition) {
        return expressions::evaluate(engine, condition).map(|v| (v, Vec::new()));
    }

    let pieces = split_percent_semi(condition);
    let mut status = TfValue::Integer(1);
    let mut side_effects = Vec::new();
    for piece in &pieces {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        // Finding 28: a command-form condition's own text is never substituted before
        // this point (it arrives raw from execute_inline_if_block/the while-loop
        // equivalent, which store the condition verbatim so a /while's condition can
        // be re-substituted fresh on every iteration - see this function's own doc
        // comment). That's correct for a top-level "/if /cmd%; /then" with no
        // variables in play, but it silently dropped a MACRO's own positional
        // parameters when the condition itself referenced them (kbbind.tf's
        // "~bind_if_not_bound": "/if /!ismacro -msimple -ib'%1'%; /then ..." never
        // saw its own "%1" replaced with the macro's first argument - visible
        // directly as a literal "%1" reaching /ismacro). Substituting here, once per
        // piece right before it runs, fixes that while still re-substituting fresh
        // on every /while iteration (this function runs again each time around).
        let piece = super::variables::substitute_commands(engine, piece);
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        let (piece_status, result) = execute_condition_command(engine, piece);
        status = piece_status;
        let error = match &result {
            TfCommandResult::Error(e) => Some(e.clone()),
            _ => None,
        };
        side_effects.push(result);
        if let Some(e) = error {
            return Err(e);
        }
    }
    Ok((status, side_effects))
}

/// Run one command from a command-form /if or /while condition and return
/// its TF return status - also left in `%?`, matching real TF's rule that
/// every command has one (tf-help "evaluation"). A leading "/!" negates the
/// status (tf-help "evaluation": "If the '/' is followed by '!', the
/// return value of the command will be negated" - used throughout
/// kbbind.tf/spedwalk.tf, e.g. "/if /!ismacro ...%; /then ...").
///
/// `/test <expr>` (or "/@test") is not special-cased here at all: cmd_test
/// already writes the expression's own value to `%?` as a side effect (see
/// parser::cmd_test), and a user macro's own /return or /result is written
/// to `%?` the same way by macros::execute_macro - so simply reading `%?`
/// right after execution picks both up for free. A sentinel written first
/// is what tells the two apart from a command that does neither: anything
/// that leaves `%?` untouched falls back to the generic TF rule for
/// "for now any builtin" (P1.8; /ismacro's own real status is Job 15's
/// territory) - 1 on success, 0 on error.
fn execute_condition_command(engine: &mut TfEngine, piece: &str) -> (TfValue, TfCommandResult) {
    let piece = piece.trim();
    let (negate, piece) = match piece.strip_prefix("/!") {
        Some(rest) => (true, format!("/{}", rest)),
        None => (false, piece.to_string()),
    };

    const UNSET: &str = "\x1f__tf_condition_status_unset__\x1f";
    engine.set_global("?", TfValue::String(UNSET.to_string()));

    let result = if piece.starts_with('/') || piece.starts_with('#') {
        super::parser::execute_command(engine, &piece)
    } else {
        // A "simple command" per tf-help "evaluation": sent to the current
        // world, true iff there is a socket to send it on. Returning
        // SendToMud (rather than queueing it here directly) lets the
        // caller's own aggregation - aggregate_results_with_engine, the
        // same one every other loop/block result in this file goes
        // through - be the one place that queues it into
        // engine.pending_commands, so it is queued exactly once. No
        // fixture in this test corpus reaches this arm (every
        // condition-list item here is a TF command).
        TfCommandResult::SendToMud(piece.clone())
    };

    let mut status = match engine.get_var("?") {
        Some(TfValue::String(s)) if s == UNSET => match &result {
            TfCommandResult::Error(_) => TfValue::Integer(0),
            _ => TfValue::Integer(1),
        },
        Some(v) => v.clone(),
        None => TfValue::Integer(1),
    };

    if negate {
        status = TfValue::Integer(if status.to_bool() { 0 } else { 1 });
    }

    engine.set_global("?", status.clone());
    (status, result)
}

/// Split `text` on TinyFugue's "%;" command separator, skipping any
/// occurrence inside a single- or double-quoted region and any "%%;" (TF's
/// escaped literal "%" immediately followed by a plain ';' - never a
/// separator; both characters of the "%%" escape are left in the output for
/// a later substitution pass to unescape).
///
/// This is the fix for finding C.3: every place in this module that needs to
/// recognise `/endif`, `/else`, `/elseif` or `/done` as a *complete keyword*
/// used to do it with whitespace-sensitive, ad hoc text scans (exact-word
/// splits, or a literal `"%;{keyword}"` substring search) - so `%;/endif`
/// (glued, no space) and `%; /endif` (spaced) took different, both-buggy
/// code paths (see `cmd_if`/`cmd_while`/`cmd_for` in `parser.rs`, and
/// `process_control_line` below). Splitting on `%;` first and feeding the
/// resulting pieces through the existing per-line matching (which already
/// works correctly on genuinely separate lines) fixes every one of those
/// sites at once, because none of them ever again see a keyword glued to a
/// separator - by the time they run, `%;` has already become a line break.
///
/// Real TinyFugue applies this uniformly: once a single-line `/if`/`/while`/
/// `/for` contains so much as one `%;`, an explicit `/endif`/`/done` becomes
/// mandatory (real `tf` errors "expected /endif, found end of body" rather
/// than falling back to the implicit-end single-command shortcut) - so
/// callers should route through the inline-block executors whenever this
/// returns more than one piece, not just when a keyword happens to be found
/// textually.
pub(crate) fn split_percent_semi(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < len {
        let c = chars[i];
        match c {
            // NOT quote-aware, deliberately - verified directly against
            // real tf: a "%;" inside what looks like a quoted string still
            // splits the body there (`/let x="a%;b"` becomes two pieces,
            // "/let x=\"a" and "b\"" - the latter sent to the world as
            // plain text - not one /let with a literal "%;" in its value).
            // self.tf's own quine depends on exactly this: it builds and
            // reconstructs its own body text by having the SAME "%;" both
            // split it apart at define/call time AND appear literally
            // inside a double-quoted string (13 quote characters, an odd
            // count that could never balance if quoting were respected
            // here). A quote-tracking version of this function used to
            // exist and silently produced the wrong split for this shape.
            '%' => {
                // Real TF's escaping rule (see substitute_variables' doc
                // comment for the full derivation, verified directly
                // against real tf): a run of N consecutive '%' is a live
                // "%;" separator only when N == 1. A run of N >= 2 is left
                // completely UNCHANGED here (not reduced) - the reduction
                // to N - 1 literal '%' characters happens later, in the
                // substitution pass this piece goes through before it
                // executes (one pass per nesting level); reducing it here
                // too would double-reduce it. This is what lets
                // "%%%;" (needed by a triply-nested command-form /for,
                // e.g. color.tf's rgb loop) survive two levels un-split
                // and only become a real separator on the third.
                let run_len = {
                    let mut n = 1;
                    while i + n < len && chars[i + n] == '%' {
                        n += 1;
                    }
                    n
                };
                if run_len == 1 && i + 1 < len && chars[i + 1] == ';' {
                    parts.push(std::mem::take(&mut current));
                    i += 2;
                } else {
                    for _ in 0..run_len {
                        current.push('%');
                    }
                    i += run_len;
                }
            }
            _ => {
                current.push(c);
                i += 1;
            }
        }
    }
    parts.push(current);
    parts
}

/// `text` with every unquoted, unescaped "%;" (see `split_percent_semi`)
/// turned into a real newline, so a single physical line using "%;" to
/// separate an `/if`/`/while`/`/for` body from its terminator can be fed
/// through the same `.lines()`-based inline-block executors a macro body's
/// (already newline-joined) control-flow block uses. A no-op (returns
/// `text` unchanged) when there's no unquoted "%;" to convert.
pub(crate) fn normalize_percent_semi_to_lines(text: &str) -> String {
    let parts = split_percent_semi(text);
    if parts.len() <= 1 {
        text.to_string()
    } else {
        parts.join("\n")
    }
}

/// Process a line when in a control flow state.
///
/// A `line` here is one *physical* (or backslash-continued) line from a file
/// already inside an open `/if`/`/while`/`/for` (`engine.control_state !=
/// None` - see `execute_command_impl` in `parser.rs`). Such a line can still
/// itself carry a glued terminator, e.g. a continuation join that produces
/// `...%;/endif` as one physical line (finding C.3) - so this first splits
/// on `%;` (see `split_percent_semi`) and replays each piece through
/// `process_control_line_single` in order, exactly as if they had arrived as
/// separate physical lines. Once a piece closes the block (state resets to
/// `None`), any further pieces on the same line are ordinary commands, not
/// more of the block's body.
pub fn process_control_line(state: &mut ControlState, line: &str) -> ControlResult {
    let fragments = split_percent_semi(line);
    if fragments.len() <= 1 {
        return process_control_line_single(state, line);
    }

    let mut collected: Vec<String> = Vec::new();
    for fragment in &fragments {
        let fragment = fragment.trim();
        if fragment.is_empty() {
            continue;
        }
        if matches!(state, ControlState::None) {
            // The block already closed on an earlier fragment - anything
            // left on this line is a plain command to run afterward.
            collected.push(fragment.to_string());
            continue;
        }
        match process_control_line_single(state, fragment) {
            ControlResult::Consumed => {}
            ControlResult::Execute(cmds) => collected.extend(cmds),
            ControlResult::Error(e) => return ControlResult::Error(e),
            ControlResult::NotControlFlow => collected.push(fragment.to_string()),
        }
    }

    if collected.is_empty() {
        ControlResult::Consumed
    } else {
        ControlResult::Execute(collected)
    }
}

fn process_control_line_single(state: &mut ControlState, line: &str) -> ControlResult {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();

    match state {
        ControlState::None => ControlResult::NotControlFlow,

        ControlState::If(if_state) => {
            // Check for nested /if
            if lower.starts_with("/if ") || lower == "/if" {
                if_state.depth += 1;
                if_state.bodies[if_state.current_branch].push(line.to_string());
                return ControlResult::Consumed;
            }

            // Check for /endif
            if lower == "/endif" {
                if_state.depth -= 1;
                if if_state.depth == 0 {
                    // End of our if block - return the collected structure
                    let result = execute_if_block(if_state);
                    *state = ControlState::None;
                    return result;
                } else {
                    // Nested endif
                    if_state.bodies[if_state.current_branch].push(line.to_string());
                    return ControlResult::Consumed;
                }
            }

            // Only process elseif/else at our depth level
            if if_state.depth == 1 {
                if lower.starts_with("/elseif ") {
                    if if_state.has_else {
                        return ControlResult::Error("/elseif after /else".to_string());
                    }
                    let prefix_len = 8; // "/elseif " is 8 chars
                    let args = &trimmed[prefix_len..];
                    match parse_condition(args) {
                        Ok(cond) => {
                            if_state.conditions.push(cond);
                            if_state.bodies.push(vec![]);
                            if_state.current_branch += 1;
                            return ControlResult::Consumed;
                        }
                        Err(e) => return ControlResult::Error(e),
                    }
                }

                if lower == "/else" {
                    if if_state.has_else {
                        return ControlResult::Error("Duplicate /else".to_string());
                    }
                    if_state.has_else = true;
                    if_state.bodies.push(vec![]);
                    if_state.current_branch += 1;
                    return ControlResult::Consumed;
                }
            }

            // Regular line - add to current branch
            if_state.bodies[if_state.current_branch].push(line.to_string());
            ControlResult::Consumed
        }

        ControlState::While(while_state) => {
            // Check for nested while/for
            if lower.starts_with("/while ") || lower == "/while"
                || lower.starts_with("/for ") || lower == "/for"
            {
                while_state.depth += 1;
                while_state.body.push(line.to_string());
                return ControlResult::Consumed;
            }

            // Check for /done
            if lower == "/done" {
                while_state.depth -= 1;
                if while_state.depth == 0 {
                    let result = ControlResult::Execute(
                        generate_while_commands(while_state)
                    );
                    *state = ControlState::None;
                    return result;
                } else {
                    while_state.body.push(line.to_string());
                    return ControlResult::Consumed;
                }
            }

            // Check for /break at our level (will be handled during execution)
            while_state.body.push(line.to_string());
            ControlResult::Consumed
        }

        ControlState::For(for_state) => {
            // Check for nested while/for
            if lower.starts_with("/while ") || lower == "/while"
                || lower.starts_with("/for ") || lower == "/for"
            {
                for_state.depth += 1;
                for_state.body.push(line.to_string());
                return ControlResult::Consumed;
            }

            // Check for /done
            if lower == "/done" {
                for_state.depth -= 1;
                if for_state.depth == 0 {
                    let result = ControlResult::Execute(
                        generate_for_commands(for_state)
                    );
                    *state = ControlState::None;
                    return result;
                } else {
                    for_state.body.push(line.to_string());
                    return ControlResult::Consumed;
                }
            }

            for_state.body.push(line.to_string());
            ControlResult::Consumed
        }
    }
}

/// Execute an if block and return commands to run
fn execute_if_block(if_state: &IfState) -> ControlResult {
    // We can't evaluate here since we don't have the engine
    // Instead, return a special marker that the parser will handle
    // Actually, let's return the structure as commands that the engine can process

    // For now, return the raw structure - the engine will evaluate conditions
    let mut commands = vec![];

    // Encode the if structure as a special internal command
    // Use \x1F (unit separator) as delimiter - unlikely to appear in TF code
    const SEP: char = '\x1F';
    let mut encoded = String::from("__tf_if_eval__");
    encoded.push(SEP);
    for (i, cond) in if_state.conditions.iter().enumerate() {
        encoded.push_str(&format!("COND{}", SEP));
        encoded.push_str(cond);
        encoded.push(SEP);
        for line in &if_state.bodies[i] {
            encoded.push_str(&format!("LINE{}", SEP));
            encoded.push_str(line);
            encoded.push(SEP);
        }
        encoded.push_str(&format!("ENDCOND{}", SEP));
    }
    if if_state.has_else {
        encoded.push_str(&format!("ELSE{}", SEP));
        if let Some(else_body) = if_state.bodies.last() {
            for line in else_body {
                encoded.push_str(&format!("LINE{}", SEP));
                encoded.push_str(line);
                encoded.push(SEP);
            }
        }
        encoded.push_str(&format!("ENDELSE{}", SEP));
    }

    commands.push(encoded);
    ControlResult::Execute(commands)
}

/// Generate commands for a while loop
fn generate_while_commands(while_state: &WhileState) -> Vec<String> {
    // Use \x1F (unit separator) as delimiter - unlikely to appear in TF code
    const SEP: char = '\x1F';
    let mut encoded = String::from("__tf_while_eval__");
    encoded.push(SEP);
    encoded.push_str(&format!("COND{}", SEP));
    encoded.push_str(&while_state.condition);
    encoded.push(SEP);
    for line in &while_state.body {
        encoded.push_str(&format!("LINE{}", SEP));
        encoded.push_str(line);
        encoded.push(SEP);
    }
    encoded.push_str(&format!("ENDWHILE{}", SEP));

    vec![encoded]
}

/// Generate commands for a for loop
fn generate_for_commands(for_state: &ForState) -> Vec<String> {
    // Use \x1F (unit separator) as delimiter - unlikely to appear in TF code
    const SEP: char = '\x1F';
    let mut encoded = String::from("__tf_for_eval__");
    encoded.push(SEP);
    encoded.push_str(&format!("VAR{}{}{}", SEP, for_state.var_name, SEP));
    encoded.push_str(&format!("START{}{}{}", SEP, for_state.start, SEP));
    encoded.push_str(&format!("END{}{}{}", SEP, for_state.end, SEP));
    encoded.push_str(&format!("STEP{}{}{}", SEP, for_state.step, SEP));
    for line in &for_state.body {
        encoded.push_str(&format!("LINE{}", SEP));
        encoded.push_str(line);
        encoded.push(SEP);
    }
    encoded.push_str(&format!("ENDFOR{}", SEP));

    vec![encoded]
}

/// Execute a single-line if command
pub fn execute_single_if(engine: &mut TfEngine, condition: &str, command: &str) -> TfCommandResult {
    // Evaluate the condition
    match expressions::evaluate(engine, condition) {
        Ok(value) => {
            if value.to_bool() {
                let command = super::variables::substitute_commands(engine, command);
                execute_body_line(engine, &command)
            } else {
                TfCommandResult::Success(None)
            }
        }
        Err(e) => TfCommandResult::Error(format!("Condition error: {}", e)),
    }
}

/// Execute a complete inline control flow block (from macro execution).
/// The input is a multi-line string containing the complete /if.../endif block.
///
/// Example input:
/// ```
/// /if (cond)    cmd1
/// /else    cmd2
/// /endif
/// ```
pub fn execute_inline_if_block(engine: &mut TfEngine, block: &str) -> Vec<TfCommandResult> {
    let mut if_state: Option<IfState> = None;
    // Set while collecting a command-form condition's own list (finding
    // C.8/P1.8) for the CURRENT branch, before its /then has been seen -
    // every line is another list item until one starts with "/then". By
    // the time this function sees the block, normalize_percent_semi_to_lines
    // (see cmd_if) has already turned every "%;" into a real newline, so
    // each list item - and "/then" itself - is always its own line here,
    // never glued to a neighbour the way the raw source might have been.
    let mut collecting_condition: Option<Vec<String>> = None;
    let lines: Vec<&str> = block.lines().collect();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();

        if let Some(pieces) = collecting_condition.as_mut() {
            if lower == "/then" || lower.starts_with("/then ") {
                let body_start = trimmed[5..].trim_start().to_string();
                let condition = pieces.join("%;");
                collecting_condition = None;
                // `if_state` always exists once collecting_condition does -
                // it is set at the same time, right below.
                let state = if_state.as_mut().expect("if_state set with collecting_condition");
                let idx = state.current_branch;
                state.conditions[idx] = condition;
                if !body_start.is_empty() {
                    state.bodies[idx].push(body_start);
                }
            } else {
                pieces.push(trimmed.to_string());
            }
            continue;
        }

        if let Some(state) = if_state.as_mut() {
            // Check for /endif
            if lower == "/endif" {
                state.depth -= 1;
                if state.depth == 0 {
                    // Block complete, execute it
                    let result = execute_if_block(state);
                    return match result {
                        ControlResult::Execute(commands) => {
                            let mut results = vec![];
                            for cmd in commands {
                                // Don't substitute encoded control flow commands - they contain
                                // embedded line content that should only be substituted during
                                // decode/execution, not on the entire encoded string
                                let result = if cmd.starts_with("__tf_if_eval__")
                                    || cmd.starts_with("__tf_while_eval__")
                                    || cmd.starts_with("__tf_for_eval__")
                                {
                                    super::parser::execute_command(engine, &cmd)
                                } else {
                                    let cmd = super::variables::substitute_commands(engine, &cmd);
                                    execute_body_line(engine, &cmd)
                                };
                                // /return and /result stop the macro body that
                                // contains this /if - don't run the rest of
                                // this branch (matches the same early-stop
                                // this module's while/for loops already do;
                                // see execute_macro_with_context's doc comment
                                // for how the two differ once this reaches it).
                                let stops = matches!(
                                    result,
                                    TfCommandResult::Return(_) | TfCommandResult::Result(_)
                                );
                                results.push(result);
                                if stops {
                                    break;
                                }
                            }
                            results
                        }
                        ControlResult::Error(e) => vec![TfCommandResult::Error(e)],
                        _ => vec![],
                    };
                } else {
                    state.bodies[state.current_branch].push(trimmed.to_string());
                }
            } else if lower.starts_with("/if ") || lower == "/if" {
                // Nested /if
                state.depth += 1;
                state.bodies[state.current_branch].push(trimmed.to_string());
            } else if state.depth == 1 && (lower.starts_with("/elseif ") || lower == "/elseif")
            {
                if state.has_else {
                    return vec![TfCommandResult::Error("/elseif after /else".to_string())];
                }
                // Parse elseif condition and optional body
                let prefix_len = 7; // "/elseif" is 7 chars
                let args = &trimmed[prefix_len..].trim_start();
                match parse_condition_with_body(args) {
                    Ok((condition, body_start)) => {
                        state.conditions.push(condition);
                        state.bodies.push(vec![]);
                        state.current_branch += 1;
                        if !body_start.is_empty() {
                            state.bodies[state.current_branch].push(body_start);
                        }
                    }
                    Err(e) => return vec![TfCommandResult::Error(e)],
                }
            } else if state.depth == 1 && (lower == "/else" || lower.starts_with("/else "))
            {
                if state.has_else {
                    return vec![TfCommandResult::Error("Duplicate /else".to_string())];
                }
                state.has_else = true;
                state.bodies.push(vec![]);
                state.current_branch += 1;
                // Check for content after /else
                let prefix_len = 5; // /else is 5 chars
                let rest = if lower == "/else" { "" } else { trimmed[prefix_len..].trim_start() };
                if !rest.is_empty() {
                    state.bodies[state.current_branch].push(rest.to_string());
                }
            } else {
                // Regular line, add to current branch
                state.bodies[state.current_branch].push(trimmed.to_string());
            }
        } else {
            // First line should be /if
            if !lower.starts_with("/if ") && lower != "/if" {
                return vec![TfCommandResult::Error("Expected /if at start of block".to_string())];
            }

            // Parse the /if line
            let prefix_len = 3; // /if is 3 chars
            let args = &trimmed[prefix_len..].trim_start();

            // TF's command-form condition (finding C.8/P1.8): "/if
            // /command%; /then ...", never a bare "/if /cmd body" - real TF
            // has no such shorthand the way the parenthesized form does, so
            // this always starts a collecting_condition list awaiting an
            // explicit /then on a later (post-normalization) line; falling
            // off the end of the block while still collecting is reported
            // below, same as an unclosed /if.
            if is_command_form_condition(args) {
                if_state = Some(IfState::new(String::new()));
                collecting_condition = Some(vec![(*args).to_string()]);
                continue;
            }

            // Find the condition
            match parse_condition_with_body(args) {
                Ok((condition, body_start)) => {
                    let mut state = IfState::new(condition);
                    // If there's content after the condition, add it as the first body line
                    if !body_start.is_empty() {
                        // Count nested control flow in body_start
                        let depth_change = count_control_flow_in_line(&body_start);
                        if depth_change > 0 {
                            state.depth += depth_change as usize;
                        }
                        state.bodies[0].push(body_start);
                    }
                    if_state = Some(state);
                }
                Err(e) => return vec![TfCommandResult::Error(e)],
            }
        }
    }

    // If we get here, the block wasn't properly closed (P1.8 error rule:
    // a command-form condition with no /then must fail clearly rather than
    // leave anything stuck - execute_inline_if_block never touches
    // engine.control_state itself, so there's nothing to reset either way).
    if collecting_condition.is_some() {
        return vec![TfCommandResult::Error(
            "/if command-form condition requires /then".to_string()
        )];
    }
    vec![TfCommandResult::Error("/if block not closed with /endif".to_string())]
}

/// Execute a complete inline while block (from macro execution).
/// The input is a multi-line string containing the complete /while.../done block.
pub fn execute_inline_while_block(engine: &mut TfEngine, block: &str) -> Vec<TfCommandResult> {
    let _results: Vec<TfCommandResult> = vec![];
    let lines: Vec<&str> = block.lines().collect();

    let mut condition = String::new();
    let mut body: Vec<String> = vec![];
    let mut depth = 0;
    let mut in_body = false;
    // Set while collecting a command-form condition's own list (finding
    // C.8/P1.8), before its /do has been seen - see execute_inline_if_block's
    // matching comment for why "/do" is always its own line by the time
    // this function sees the block.
    let mut collecting_condition: Option<Vec<String>> = None;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();

        if let Some(pieces) = collecting_condition.as_mut() {
            if lower == "/do" || lower.starts_with("/do ") {
                let body_start = trimmed[3..].trim_start().to_string();
                condition = pieces.join("%;");
                collecting_condition = None;
                depth = 1;
                in_body = true;
                if !body_start.is_empty() {
                    body.push(body_start);
                }
            } else {
                pieces.push(trimmed.to_string());
            }
            continue;
        }

        if !in_body {
            // First line should be /while
            if !lower.starts_with("/while ") && lower != "/while" {
                return vec![TfCommandResult::Error("Expected /while at start of block".to_string())];
            }

            // Parse the /while line
            let prefix_len = 6; // /while is 6 chars
            let args = &trimmed[prefix_len..].trim_start();

            // TF's command-form condition (finding C.8/P1.8): "/while
            // /command%; /do ..." - see execute_inline_if_block's matching
            // comment on why this always starts a collecting_condition
            // list rather than a bare "/while /cmd body" shorthand.
            if is_command_form_condition(args) {
                collecting_condition = Some(vec![(*args).to_string()]);
                continue;
            }

            match parse_condition_with_body(args) {
                Ok((cond, body_start)) => {
                    condition = cond;
                    depth = 1;
                    in_body = true;
                    if !body_start.is_empty() {
                        body.push(body_start);
                    }
                }
                Err(e) => return vec![TfCommandResult::Error(e)],
            }
        } else {
            // Track nested while/for/if blocks
            if lower.starts_with("/while ") || lower == "/while"
                || lower.starts_with("/for ") || lower == "/for"
            {
                depth += 1;
                body.push(trimmed.to_string());
            } else if lower == "/done" {
                depth -= 1;
                if depth == 0 {
                    // Execute the while loop
                    return execute_while_loop(engine, &condition, &body);
                } else {
                    body.push(trimmed.to_string());
                }
            } else {
                body.push(trimmed.to_string());
            }
        }
    }

    // P1.8 error rule: a command-form condition with no /do must fail
    // clearly rather than leave anything stuck (this function never
    // touches engine.control_state either way).
    if collecting_condition.is_some() {
        return vec![TfCommandResult::Error(
            "/while command-form condition requires /do".to_string()
        )];
    }
    vec![TfCommandResult::Error("/while block not closed with /done".to_string())]
}

/// Execute a while loop with given condition and body
fn execute_while_loop(engine: &mut TfEngine, condition: &str, body: &[String]) -> Vec<TfCommandResult> {
    let mut results = vec![];
    let mut iterations = 0;

    // Group body lines so control flow blocks are kept together
    let grouped_body = group_body_lines(body);

    loop {
        if iterations >= MAX_ITERATIONS {
            results.push(TfCommandResult::Error(format!(
                "While loop exceeded maximum iterations ({})", MAX_ITERATIONS
            )));
            break;
        }

        // Evaluate condition - may itself be a command-form condition
        // (finding C.8/P1.8), whose own commands genuinely run on every
        // iteration (see evaluate_condition's doc comment), so their
        // results must be folded in exactly like a body line's.
        match evaluate_condition(engine, condition) {
            Ok((value, side_effects)) => {
                results.extend(side_effects);
                if !value.to_bool() {
                    break;
                }
            }
            Err(e) => {
                results.push(TfCommandResult::Error(format!("Condition error: {}", e)));
                break;
            }
        }

        // Execute body
        let mut should_break = false;
        for line in &grouped_body {
            let line_lower = line.trim().to_lowercase();
            if line_lower == "/break" {
                should_break = true;
                break;
            }

            // Real TF substitutes a nested /while or /for's own header and
            // body text exactly once per enclosing iteration (tf-help
            // /for: "<Commands> are executed in a new evaluation scope";
            // verified directly: a nested command-form /for's loop
            // variable needs one extra level of "%" escaping per level of
            // nesting, e.g. color.tf's triple-nested rgb loop's
            // "%%%{red}"). A nested /if is different - it is NOT its own
            // evaluation scope (verified directly: an unescaped "%i" from
            // an enclosing /for's loop variable resolves correctly inside
            // a nested /if with no extra escaping needed), and
            // `execute_inline_if_block`'s own per-branch-line dispatch
            // already substitutes each line itself - substituting here
            // too would run some of this same text through substitution
            // TWICE, corrupting any already-resolved value that happens
            // to contain a literal '%' or '$' (this broke
            // `test_encrypt_decrypt_roundtrip`'s crypt.tf, whose nested
            // `/if` bodies build strings one arbitrary byte at a time).
            let is_nested_if = line_lower.starts_with("/if ") || line_lower == "/if"
                || line_lower.starts_with("/if(");
            let line = if is_nested_if {
                line.clone()
            } else {
                super::variables::substitute_commands(engine, line)
            };

            let result = execute_body_line(engine, &line);
            // Check for break in nested execution
            if let TfCommandResult::Error(ref e) = result {
                if let Some(remaining) = parse_break_marker(e) {
                    // Absorb one level here; a count >1 must keep unwinding
                    // outward, so re-emit it decremented into `results` -
                    // whatever aggregates this loop's own results (a macro
                    // body, or another loop one level further out) applies
                    // this exact same check again.
                    if remaining > 1 {
                        results.push(TfCommandResult::Error(break_marker(remaining - 1)));
                    }
                    should_break = true;
                    break;
                }
            }
            // Check for /return - propagate up
            // /return and /result both stop and propagate up the same way
            // here (matching whatever this loop already did for /return -
            // see execute_macro_with_context's doc comment); which of the
            // two (echo or not) only matters once it reaches execute_macro's
            // own per-command loop, which distinguishes them.
            if matches!(result, TfCommandResult::Return(_) | TfCommandResult::Result(_)) {
                results.push(result);
                should_break = true;
                break;
            }
            results.push(result);
        }

        if should_break {
            break;
        }

        iterations += 1;
    }

    results
}

/// Execute a complete inline for block (from macro execution).
pub fn execute_inline_for_block(engine: &mut TfEngine, block: &str) -> Vec<TfCommandResult> {
    let _results: Vec<TfCommandResult> = vec![];
    let lines: Vec<&str> = block.lines().collect();

    let mut var_name = String::new();
    let mut start: i64 = 0;
    let mut end: i64 = 0;
    let mut step: i64 = 1;
    let mut body: Vec<String> = vec![];
    let mut depth = 0;
    let mut in_body = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();

        if !in_body {
            // First line should be /for
            if !lower.starts_with("/for ") && lower != "/for" {
                return vec![TfCommandResult::Error("Expected /for at start of block".to_string())];
            }

            // Parse the /for line
            let prefix_len = 4; // /for is 4 chars
            let args = &trimmed[prefix_len..].trim_start();
            // Parse for args: var start end [step] [body_content]
            let parts: Vec<&str> = args.split_whitespace().collect();
            if parts.len() < 3 {
                return vec![TfCommandResult::Error("/for requires: var start end [step]".to_string())];
            }

            var_name = parts[0].to_string();
            start = match parts[1].parse() {
                Ok(v) => v,
                Err(_) => return vec![TfCommandResult::Error(format!("Invalid start value: {}", parts[1]))],
            };
            end = match parts[2].parse() {
                Ok(v) => v,
                Err(_) => return vec![TfCommandResult::Error(format!("Invalid end value: {}", parts[2]))],
            };

            let mut body_start_idx = 3;
            if parts.len() > 3 {
                if let Ok(s) = parts[3].parse::<i64>() {
                    step = s;
                    body_start_idx = 4;
                } else if start <= end {
                    step = 1;
                } else {
                    step = -1;
                }
            } else if start <= end {
                step = 1;
            } else {
                step = -1;
            }

            depth = 1;
            in_body = true;

            // Any remaining content on the line is body
            if body_start_idx < parts.len() {
                let body_content = parts[body_start_idx..].join(" ");
                if !body_content.is_empty() {
                    body.push(body_content);
                }
            }
        } else {
            // Track nested while/for blocks
            if lower.starts_with("/while ") || lower == "/while"
                || lower.starts_with("/for ") || lower == "/for"
            {
                depth += 1;
                body.push(trimmed.to_string());
            } else if lower == "/done" {
                depth -= 1;
                if depth == 0 {
                    // Execute the for loop
                    return execute_for_loop(engine, &var_name, start, end, step, &body);
                } else {
                    body.push(trimmed.to_string());
                }
            } else {
                body.push(trimmed.to_string());
            }
        }
    }

    vec![TfCommandResult::Error("/for block not closed with /done".to_string())]
}

/// Execute a for loop. `pub(crate)` so parser::cmd_for can drive it
/// directly for TF's own `/for var min max command` single-line form
/// (finding C.7/P1.7), which never goes through the ControlState/encoded
/// round-trip the way the block form (`/for ... /done`) below does - it
/// runs to completion in one call, with no later physical lines to wait
/// for.
pub(crate) fn execute_for_loop(
    engine: &mut TfEngine,
    var_name: &str,
    start: i64,
    end: i64,
    step: i64,
    body: &[String],
) -> Vec<TfCommandResult> {
    let mut results = vec![];
    let mut iterations = 0;
    let mut current = start;

    // Group body lines so control flow blocks are kept together
    let grouped_body = group_body_lines(body);

    let should_continue = |cur: i64, end_val: i64, step_val: i64| -> bool {
        if step_val > 0 {
            cur <= end_val
        } else {
            cur >= end_val
        }
    };

    engine.push_scope();
    while should_continue(current, end, step) {
        if iterations >= MAX_ITERATIONS {
            results.push(TfCommandResult::Error(format!(
                "For loop exceeded maximum iterations ({})", MAX_ITERATIONS
            )));
            break;
        }

        // Set loop variable
        engine.set_local(var_name, super::TfValue::Integer(current));

        // Execute body
        let mut should_break = false;
        for line in &grouped_body {
            let line_lower = line.trim().to_lowercase();
            if line_lower == "/break" {
                should_break = true;
                break;
            }

            // Real TF substitutes a nested /while or /for's own header and
            // body text exactly once per enclosing iteration, but NOT a
            // nested /if (see execute_while_loop's matching comment for
            // the full derivation, verified directly against real tf both
            // ways: color.tf's triple-nested-for rgb loop needs one extra
            // level of "%" escaping per level of nesting for this to
            // resolve correctly, while an /if nested in a /for needs NO
            // extra escaping at all and must not be double-substituted).
            let is_nested_if = line_lower.starts_with("/if ") || line_lower == "/if"
                || line_lower.starts_with("/if(");
            let line = if is_nested_if {
                line.clone()
            } else {
                super::variables::substitute_commands(engine, line)
            };

            let result = execute_body_line(engine, &line);
            if let TfCommandResult::Error(ref e) = result {
                if let Some(remaining) = parse_break_marker(e) {
                    // Absorb one level here; a count >1 must keep unwinding
                    // outward, so re-emit it decremented into `results` -
                    // whatever aggregates this loop's own results (a macro
                    // body, or another loop one level further out) applies
                    // this exact same check again.
                    if remaining > 1 {
                        results.push(TfCommandResult::Error(break_marker(remaining - 1)));
                    }
                    should_break = true;
                    break;
                }
            }
            // /return and /result both stop and propagate up the same way
            // here (matching whatever this loop already did for /return -
            // see execute_macro_with_context's doc comment); which of the
            // two (echo or not) only matters once it reaches execute_macro's
            // own per-command loop, which distinguishes them.
            if matches!(result, TfCommandResult::Return(_) | TfCommandResult::Result(_)) {
                results.push(result);
                should_break = true;
                break;
            }
            results.push(result);
        }

        if should_break {
            break;
        }

        current += step;
        iterations += 1;
    }
    engine.pop_scope();

    results
}

/// Count the net change in control flow depth from a line of text.
/// Returns positive for each /if//while//for found, negative for each /endif//done.
fn count_control_flow_in_line(text: &str) -> i32 {
    let lower = text.to_lowercase();
    let mut depth = 0;

    // Simple word-based scanning for control flow keywords
    let words: Vec<&str> = lower.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        if *word == "/if" || word.starts_with("/if(")
            || *word == "/while" || word.starts_with("/while(")
        {
            depth += 1;
        } else if *word == "/for" {
            // See is_self_contained_for's doc comment: TF's own command-form
            // /for (finding C.7/P1.7) never has a matching /done and must
            // not be counted as an opener.
            if !is_self_contained_for(&words, i) {
                depth += 1;
            }
        } else if *word == "/endif" || *word == "/done" {
            depth -= 1;
        }
    }

    depth
}

/// Group body lines into execution units.
/// Lines that form control flow structures (/if.../endif, /while.../done, /for.../done)
/// are grouped together into single strings with newlines.
fn group_control_flow_lines(lines: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut current_group = String::new();
    let mut depth = 0;

    for line in lines {
        let trimmed = line.trim();
        let depth_change = count_control_flow_in_line(trimmed);

        if depth == 0 && depth_change > 0 {
            // Starting a new control flow block
            depth = depth_change;
            current_group = trimmed.to_string();
        } else if depth > 0 {
            // Inside a control flow block
            if !current_group.is_empty() {
                current_group.push('\n');
            }
            current_group.push_str(trimmed);
            depth += depth_change;

            if depth <= 0 {
                // End of control flow block
                result.push(std::mem::take(&mut current_group));
                depth = 0;
            }
        } else {
            // Regular line, not in control flow
            result.push(trimmed.to_string());
        }
    }

    // If there's remaining content (unclosed control flow), add it anyway
    if !current_group.is_empty() {
        result.push(current_group);
    }

    result
}

/// Parse a condition from /if//elseif, potentially with body content after it.
/// Returns (condition, body_content) where body_content may be empty.
fn parse_condition_with_body(args: &str) -> Result<(String, String), String> {
    let args = args.trim();

    if !args.starts_with('(') {
        return Err("Condition must be enclosed in parentheses".to_string());
    }

    // Find matching closing paren
    let mut depth = 0;
    let mut end_paren = None;
    for (i, c) in args.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end_paren = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    match end_paren {
        Some(i) => {
            let condition = args[1..i].trim().to_string();
            let rest = args[i + 1..].trim().to_string();
            Ok((condition, rest))
        }
        None => Err("Unclosed parenthesis in condition".to_string()),
    }
}

/// Execute an encoded if block
pub fn execute_if_encoded(engine: &mut TfEngine, encoded: &str) -> Vec<TfCommandResult> {
    let mut results = vec![];

    // Use \x1F (unit separator) as delimiter
    const SEP: char = '\x1F';
    let _sep_str = SEP.to_string();

    // Parse the encoded if structure
    let content = encoded.strip_prefix("__tf_if_eval__").unwrap_or(encoded);
    let content = content.strip_prefix(SEP).unwrap_or(content);

    let mut conditions: Vec<String> = vec![];
    let mut bodies: Vec<Vec<String>> = vec![];
    let mut else_body: Option<Vec<String>> = None;

    let mut current_body: Vec<String> = vec![];
    let mut current_cond: Option<String> = None;
    let mut in_else = false;

    // Simple parser for the encoded format
    let mut remaining = content;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix(&format!("COND{}", SEP)) {
            if let Some(end) = rest.find(SEP) {
                current_cond = Some(rest[..end].to_string());
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix(&format!("LINE{}", SEP)) {
            if let Some(end) = rest.find(SEP) {
                current_body.push(rest[..end].to_string());
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix(&format!("ENDCOND{}", SEP)) {
            if let Some(cond) = current_cond.take() {
                conditions.push(cond);
                bodies.push(std::mem::take(&mut current_body));
            }
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix(&format!("ELSE{}", SEP)) {
            in_else = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix(&format!("ENDELSE{}", SEP)) {
            if in_else {
                else_body = Some(std::mem::take(&mut current_body));
            }
            remaining = rest;
        } else {
            // Skip unknown
            if let Some(idx) = remaining.find(SEP) {
                remaining = &remaining[idx + 1..];
            } else {
                break;
            }
        }
    }

    // Evaluate conditions in order. Each one may be a command-form
    // condition (finding C.8/P1.8) - per tf-help "evaluation", every
    // /if//elseif's list genuinely runs (side effects included) as it's
    // checked, true or false, not only the branch that ends up taken; fold
    // those results in the same way a body line's would be.
    for (i, cond) in conditions.iter().enumerate() {
        match evaluate_condition(engine, cond) {
            Ok((value, side_effects)) => {
                results.extend(side_effects);
                if value.to_bool() {
                    // Execute this branch
                    if let Some(body) = bodies.get(i) {
                        // Group body lines into execution units (control flow blocks stay together)
                        let grouped = group_control_flow_lines(body);
                        for group in grouped {
                            let line = group.trim();
                            if line.is_empty() {
                                continue;
                            }

                            // Check if this is a nested control flow block
                            let lower = line.to_lowercase();
                            let is_control_flow = lower.starts_with("/if ") || lower.starts_with("/if(")
                                || lower.starts_with("/while ")
                                || lower.starts_with("/for ");

                            let line = if is_control_flow {
                                line.to_string()
                            } else {
                                                super::variables::substitute_commands(engine, line)
                            };

                            results.push(execute_body_line(engine, &line));
                        }
                    }
                    return results;
                }
            }
            Err(e) => {
                results.push(TfCommandResult::Error(format!("Condition error: {}", e)));
                return results;
            }
        }
    }

    // No condition matched, execute else if present
    if let Some(body) = else_body {
        let grouped = group_control_flow_lines(&body);
        for line in grouped {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Check if this is a nested control flow block
            let lower = line.to_lowercase();
            let is_control_flow = lower.starts_with("/if ") || lower.starts_with("/if(")
                || lower.starts_with("/while ")
                || lower.starts_with("/for ");

            let line = if is_control_flow {
                line.to_string()
            } else {
                super::variables::substitute_commands(engine, line)
            };

            results.push(execute_body_line(engine, &line));
        }
    }

    results
}

/// Execute an encoded while loop
pub fn execute_while_encoded(engine: &mut TfEngine, encoded: &str) -> Vec<TfCommandResult> {
    let mut results = vec![];

    // Use \x1F (unit separator) as delimiter
    const SEP: char = '\x1F';

    let content = encoded.strip_prefix("__tf_while_eval__").unwrap_or(encoded);
    let content = content.strip_prefix(SEP).unwrap_or(content);

    // Parse condition and body
    let mut condition = String::new();
    let mut body: Vec<String> = vec![];

    let mut remaining = content;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix(&format!("COND{}", SEP)) {
            if let Some(end) = rest.find(SEP) {
                condition = rest[..end].to_string();
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix(&format!("LINE{}", SEP)) {
            if let Some(end) = rest.find(SEP) {
                body.push(rest[..end].to_string());
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if remaining.starts_with(&format!("ENDWHILE{}", SEP)) {
            break;
        } else if let Some(idx) = remaining.find(SEP) {
            remaining = &remaining[idx + 1..];
        } else {
            break;
        }
    }

    // Execute while loop with iteration limit
    let grouped_body = group_body_lines(&body);
    let mut iterations = 0;
    loop {
        if iterations >= MAX_ITERATIONS {
            results.push(TfCommandResult::Error(format!(
                "While loop exceeded maximum iterations ({})", MAX_ITERATIONS
            )));
            break;
        }

        // Evaluate condition (may be command-form - see evaluate_condition's
        // doc comment for why its side effects are folded in here too).
        match evaluate_condition(engine, &condition) {
            Ok((value, side_effects)) => {
                results.extend(side_effects);
                if !value.to_bool() {
                    break;
                }
            }
            Err(e) => {
                results.push(TfCommandResult::Error(format!("Condition error: {}", e)));
                break;
            }
        }

        // Execute body
        let mut should_break = false;
        for line in &grouped_body {
            let line_lower = line.trim().to_lowercase();
            if line_lower == "/break" {
                should_break = true;
                break;
            }
            let line = super::variables::substitute_commands(engine, line);
            let result = execute_body_line(engine, &line);
            // Check for break in nested execution
            if let TfCommandResult::Error(ref e) = result {
                if let Some(remaining) = parse_break_marker(e) {
                    // Absorb one level here; a count >1 must keep unwinding
                    // outward, so re-emit it decremented into `results` -
                    // whatever aggregates this loop's own results (a macro
                    // body, or another loop one level further out) applies
                    // this exact same check again.
                    if remaining > 1 {
                        results.push(TfCommandResult::Error(break_marker(remaining - 1)));
                    }
                    should_break = true;
                    break;
                }
            }
            results.push(result);
        }

        if should_break {
            break;
        }

        iterations += 1;
    }

    results
}

/// Execute an encoded for loop
pub fn execute_for_encoded(engine: &mut TfEngine, encoded: &str) -> Vec<TfCommandResult> {
    let mut results = vec![];

    // Use \x1F (unit separator) as delimiter
    const SEP: char = '\x1F';

    let content = encoded.strip_prefix("__tf_for_eval__").unwrap_or(encoded);
    let content = content.strip_prefix(SEP).unwrap_or(content);

    // Parse var, start, end, step, and body
    let mut var_name = String::new();
    let mut start: i64 = 0;
    let mut end: i64 = 0;
    let mut step: i64 = 1;
    let mut body: Vec<String> = vec![];

    let mut remaining = content;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix(&format!("VAR{}", SEP)) {
            if let Some(idx) = rest.find(SEP) {
                var_name = rest[..idx].to_string();
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix(&format!("START{}", SEP)) {
            if let Some(idx) = rest.find(SEP) {
                start = rest[..idx].parse().unwrap_or(0);
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix(&format!("END{}", SEP)) {
            if let Some(idx) = rest.find(SEP) {
                end = rest[..idx].parse().unwrap_or(0);
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix(&format!("STEP{}", SEP)) {
            if let Some(idx) = rest.find(SEP) {
                step = rest[..idx].parse().unwrap_or(1);
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix(&format!("LINE{}", SEP)) {
            if let Some(idx) = rest.find(SEP) {
                body.push(rest[..idx].to_string());
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if remaining.starts_with(&format!("ENDFOR{}", SEP)) {
            break;
        } else if let Some(idx) = remaining.find(SEP) {
            remaining = &remaining[idx + 1..];
        } else {
            break;
        }
    }

    // Execute for loop
    let mut iterations = 0;
    let mut current = start;

    let should_continue = |cur: i64, end_val: i64, step_val: i64| -> bool {
        if step_val > 0 {
            cur <= end_val
        } else {
            cur >= end_val
        }
    };

    let grouped_body = group_body_lines(&body);
    engine.push_scope();
    while should_continue(current, end, step) {
        if iterations >= MAX_ITERATIONS {
            results.push(TfCommandResult::Error(format!(
                "For loop exceeded maximum iterations ({})", MAX_ITERATIONS
            )));
            break;
        }

        // Set loop variable
        engine.set_local(&var_name, super::TfValue::Integer(current));

        // Execute body
        let mut should_break = false;
        for line in &grouped_body {
            let line_lower = line.trim().to_lowercase();
            if line_lower == "/break" {
                should_break = true;
                break;
            }
            let line = super::variables::substitute_commands(engine, line);
            let result = execute_body_line(engine, &line);
            if let TfCommandResult::Error(ref e) = result {
                if let Some(remaining) = parse_break_marker(e) {
                    // Absorb one level here; a count >1 must keep unwinding
                    // outward, so re-emit it decremented into `results` -
                    // whatever aggregates this loop's own results (a macro
                    // body, or another loop one level further out) applies
                    // this exact same check again.
                    if remaining > 1 {
                        results.push(TfCommandResult::Error(break_marker(remaining - 1)));
                    }
                    should_break = true;
                    break;
                }
            }
            // /return and /result both stop and propagate up the same way
            // here (matching whatever this loop already did for /return -
            // see execute_macro_with_context's doc comment); which of the
            // two (echo or not) only matters once it reaches execute_macro's
            // own per-command loop, which distinguishes them.
            if matches!(result, TfCommandResult::Return(_) | TfCommandResult::Result(_)) {
                results.push(result);
                should_break = true;
                break;
            }
            results.push(result);
        }

        if should_break {
            break;
        }

        current += step;
        iterations += 1;
    }
    engine.pop_scope();

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_line_if() {
        assert_eq!(
            parse_single_line_if("(1 == 1) /echo yes"),
            Some(("1 == 1".to_string(), "/echo yes".to_string()))
        );

        assert_eq!(
            parse_single_line_if("(x > 5) send attack"),
            Some(("x > 5".to_string(), "send attack".to_string()))
        );

        // Multi-line if (no command after condition)
        assert_eq!(parse_single_line_if("(1 == 1)"), None);

        // Nested parens in condition
        assert_eq!(
            parse_single_line_if("((1 + 2) > 2) /echo yes"),
            Some(("(1 + 2) > 2".to_string(), "/echo yes".to_string()))
        );
    }

    #[test]
    fn test_parse_condition() {
        assert_eq!(parse_condition("(x > 5)"), Ok("x > 5".to_string()));
        assert_eq!(parse_condition("  ( foo == bar )  "), Ok("foo == bar".to_string()));
        assert!(parse_condition("x > 5").is_err()); // Missing parens
        assert!(parse_condition("(unclosed").is_err());
    }

    #[test]
    fn test_parse_for_args() {
        assert_eq!(
            parse_for_args("i 1 10"),
            Ok(("i".to_string(), 1, 10, 1))
        );

        assert_eq!(
            parse_for_args("x 10 1 -1"),
            Ok(("x".to_string(), 10, 1, -1))
        );

        // Auto step direction
        assert_eq!(
            parse_for_args("i 10 1"),
            Ok(("i".to_string(), 10, 1, -1))
        );

        assert!(parse_for_args("i 1").is_err()); // Missing end
        assert!(parse_for_args("i 1 10 0").is_err()); // Zero step
    }

    #[test]
    fn test_execute_single_if() {
        let mut engine = TfEngine::new();

        // True condition
        let result = execute_single_if(&mut engine, "1 == 1", "/set result yes");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert_eq!(
            engine.get_var("result").map(|v| v.to_string_value()),
            Some("yes".to_string())
        );

        // False condition
        let result = execute_single_if(&mut engine, "1 == 2", "/set result no");
        assert!(matches!(result, TfCommandResult::Success(None)));
        // result should still be "yes"
        assert_eq!(
            engine.get_var("result").map(|v| v.to_string_value()),
            Some("yes".to_string())
        );
    }

    #[test]
    fn test_if_state_collection() {
        let mut state = ControlState::If(IfState::new("x > 5".to_string()));

        // Add some lines
        assert!(matches!(process_control_line(&mut state, "/echo inside if"), ControlResult::Consumed));
        assert!(matches!(process_control_line(&mut state, "/set y 10"), ControlResult::Consumed));

        // End the if
        let result = process_control_line(&mut state, "/endif");
        assert!(matches!(result, ControlResult::Execute(_)));
        assert!(matches!(state, ControlState::None));
    }

    #[test]
    fn test_while_state_collection() {
        let mut state = ControlState::While(WhileState::new("x < 10".to_string()));

        assert!(matches!(process_control_line(&mut state, "/set x (x + 1)"), ControlResult::Consumed));
        assert!(matches!(process_control_line(&mut state, "/echo %x"), ControlResult::Consumed));

        let result = process_control_line(&mut state, "/done");
        assert!(matches!(result, ControlResult::Execute(_)));
    }

    #[test]
    fn test_for_loop_execution() {
        let mut engine = TfEngine::new();
        engine.set_global("sum", super::super::TfValue::Integer(0));

        // Create and execute a simple for loop using the new \x1F separator format
        const SEP: char = '\x1F';
        let encoded = format!(
            "__tf_for_eval__{sep}VAR{sep}i{sep}START{sep}1{sep}END{sep}3{sep}STEP{sep}1{sep}LINE{sep}/set sum (${{sum}} + %i){sep}ENDFOR{sep}",
            sep = SEP
        );

        let results = execute_for_encoded(&mut engine, &encoded);

        // Should have executed 3 times (i=1,2,3), sum should be 6
        assert!(!results.iter().any(|r| matches!(r, TfCommandResult::Error(_))));
    }
}

#[cfg(test)]
mod inline_tests {
    use super::*;

    #[test]
    fn test_execute_inline_if_block() {
        let mut engine = TfEngine::new();

        // Test true condition
        let block = "/if (1 == 1)    /set x yes\n/else    /set x no\n/endif";
        let results = execute_inline_if_block(&mut engine, block);
        assert!(!results.iter().any(|r| matches!(r, TfCommandResult::Error(_))), "Results: {:?}", results);
        assert_eq!(
            engine.get_var("x").map(|v| v.to_string_value()),
            Some("yes".to_string())
        );

        // Test false condition
        let block2 = "/if (1 == 2)    /set y wrong\n/else    /set y correct\n/endif";
        let results2 = execute_inline_if_block(&mut engine, block2);
        assert!(!results2.iter().any(|r| matches!(r, TfCommandResult::Error(_))), "Results2: {:?}", results2);
        assert_eq!(
            engine.get_var("y").map(|v| v.to_string_value()),
            Some("correct".to_string())
        );
    }

    #[test]
    fn test_nested_inline_if() {
        let mut engine = TfEngine::new();

        let block = "/if (1 == 1)    /if (2 == 2)    /set z inner\n/endif\n/endif";
        let results = execute_inline_if_block(&mut engine, block);
        assert!(!results.iter().any(|r| matches!(r, TfCommandResult::Error(_))), "Results: {:?}", results);
        assert_eq!(
            engine.get_var("z").map(|v| v.to_string_value()),
            Some("inner".to_string())
        );
    }
}

/// Unit tests for P1.7 (TF's own `/for <var> <min> <max> <command>` form)
/// and P1.8 (command-form `/if`/`/while` conditions) - plan Job 9.
#[cfg(test)]
mod command_form_tests {
    use super::*;

    /// Unwrap a `Success(Some(text))`, panicking with the actual result on
    /// anything else - every case below expects real echoed text back.
    fn success_text(result: &TfCommandResult) -> String {
        match result {
            TfCommandResult::Success(Some(s)) => s.clone(),
            other => panic!("expected Success(Some(_)), got {:?}", other),
        }
    }

    // ---- P1.7: TF's own `/for var min max command` form ----

    #[test]
    fn test_for_command_form_single_body() {
        let mut engine = TfEngine::new();
        let result = engine.execute("/for i 1 3 /echo n=%i");
        assert_eq!(success_text(&result), "n=1\nn=2\nn=3");
    }

    #[test]
    fn test_for_command_form_percent_semi_body() {
        // The "command" (everything past the 3rd token) may itself contain
        // "%;"-separated commands, all run every iteration.
        let mut engine = TfEngine::new();
        let result = engine.execute("/for i 1 2 /echo a=%i%; /echo b=%i");
        assert_eq!(success_text(&result), "a=1\nb=1\na=2\nb=2");
    }

    #[test]
    fn test_for_command_form_substitution_timing_bare_percent() {
        // CRITICAL substitution timing (P1.7): the body must reach EACH
        // iteration un-expanded, so "%i" is substituted fresh from that
        // iteration's value - never once, up front, from whatever "i" held
        // (or didn't) before the loop ever set it. Pre-seed "i" with a
        // stale value to prove it is not what gets echoed.
        let mut engine = TfEngine::new();
        engine.set_global("i", TfValue::String("stale".to_string()));
        let result = engine.execute("/for i 1 3 /echo v=%i");
        assert_eq!(success_text(&result), "v=1\nv=2\nv=3");
    }

    #[test]
    fn test_for_command_form_substitution_timing_braced_percent() {
        // Same timing rule, via "%{i}"-style braced substitution rather
        // than the bare "%i" shorthand.
        let mut engine = TfEngine::new();
        engine.set_global("i", TfValue::String("stale".to_string()));
        let result = engine.execute("/for i 1 3 /echo v=%{i}");
        assert_eq!(success_text(&result), "v=1\nv=2\nv=3");
    }

    #[test]
    fn test_for_command_form_nested_in_macro_body() {
        // The TF form must work identically whether it's typed in a macro
        // body (this test) or arrives as a top-level file line (the next
        // test) - both reach cmd_for the same way, un-substituted.
        let mut engine = TfEngine::new();
        let def_result = engine.execute("/def loopmac = /for i 1 3 /echo n=%i");
        assert!(!matches!(def_result, TfCommandResult::Error(_)), "{:?}", def_result);
        let result = engine.execute("/loopmac");
        assert_eq!(success_text(&result), "n=1\nn=2\nn=3");
    }

    #[test]
    fn test_for_command_form_via_load_file_top_level() {
        let mut engine = TfEngine::new();
        let script = "/set _acc=\n/for i 1 3 /set _acc=%{_acc}%i\n";
        let result = super::super::builtins::load_from_str(&mut engine, script);
        assert!(!matches!(result, TfCommandResult::Error(_)), "{:?}", result);
        assert_eq!(
            engine.get_var("_acc").map(|v| v.to_string_value()),
            Some("123".to_string())
        );
    }

    #[test]
    fn test_clay_numeric_for_done_still_works() {
        // Clay's own multi-line extension (documented in /help for) must
        // keep working: /for var start end [step] ... /done, body
        // collected across separate physical lines via ControlState.
        let mut engine = TfEngine::new();
        let start = engine.execute("/for i 1 3");
        assert!(matches!(start, TfCommandResult::Success(None)), "{:?}", start);
        let collected = engine.execute("/echo n=%i");
        assert!(matches!(collected, TfCommandResult::Success(None)), "{:?}", collected);
        let done = engine.execute("/done");
        assert_eq!(success_text(&done), "n=1\nn=2\nn=3");
    }

    // ---- P1.8: command-form `/if`/`/while` conditions ----

    #[test]
    fn test_if_command_form_then() {
        let mut engine = TfEngine::new();
        let result = engine.execute("/if /test 1%; /then /echo yes%; /endif");
        assert_eq!(success_text(&result), "yes");
    }

    #[test]
    fn test_if_command_form_then_else() {
        let mut engine = TfEngine::new();
        let result = engine.execute("/if /test 0%; /then /echo no%; /else /echo yes%; /endif");
        assert_eq!(success_text(&result), "yes");
    }

    #[test]
    fn test_if_command_form_negation() {
        // "/!" negates the return status of the one command it prefixes
        // (tf-help "evaluation") - "/test 0" is falsy, so "/!test 0" is true.
        let mut engine = TfEngine::new();
        let result = engine.execute("/if /!test 0%; /then /echo yes%; /endif");
        assert_eq!(success_text(&result), "yes");
    }

    /// Finding 28: a macro's own command-form `/if` condition must have the SAME
    /// substitution applied to it that every other body line gets - kbbind.tf's own
    /// "~bind_if_not_bound" idiom (`/def -i ~bind_if_not_bound = /if /!ismacro -msimple
    /// -ib'%1'%; /then /def -ib'%1' = %-1%; /endif`) lost its own "%1" here before this
    /// fix (visible directly as a literal, unsubstituted "%1" reaching /ismacro).
    #[test]
    fn test_command_form_if_condition_substitutes_macro_positional_params() {
        let mut engine = TfEngine::new();
        engine.execute(
            "/def -i ~bind_if_not_bound = /if /!ismacro -msimple -ib'%1'%; /then /def -ib'%1' = %-1%; /endif"
        );
        // Nothing is bound to ^R yet, so /ismacro (finding 28's other half - now a
        // native command) should report no match, negate to true, and the /then
        // branch should define a binding for ^R - proving "%1" reached /ismacro as
        // "^R", not the literal text "%1".
        engine.execute("/~bind_if_not_bound ^R /dokey refresh");
        let bound = engine.macros.iter().any(|m| m.keybinding.as_deref() == Some("^R"));
        assert!(bound, "the macro's own %1 must reach /ismacro substituted, not literal");

        // Calling it again for the SAME key must be a no-op (ismacro now finds the
        // existing binding, negates to false, and /then's body never runs) - this is
        // what "if not already bound" actually means, and only works if the
        // substitution above is genuinely fresh on every invocation, not just once.
        let before = engine.macros.len();
        engine.execute("/~bind_if_not_bound ^R /dokey something_else");
        assert_eq!(engine.macros.len(), before, "an existing binding must not be redefined");
    }

    #[test]
    fn test_while_command_form_tr_idiom() {
        // tr.tf's own /tr macro drives its loop exactly this way:
        // "/while /let _i=...%; /@test _i >= 0%; /do ...%; /done" - a
        // TWO-command condition list, whose return value (per tf-help
        // "evaluation") is the LAST command's status; both commands
        // genuinely re-run every iteration.
        let mut engine = TfEngine::new();
        engine.set_global("_i", TfValue::Integer(3));
        let result = engine.execute(
            "/while /let _i=$[_i - 1]%; /@test _i >= 0%; /do /echo i=%_i%; /done"
        );
        assert_eq!(success_text(&result), "i=2\ni=1\ni=0");
    }

    #[test]
    fn test_percent_question_after_if_command_form() {
        // "/test <expr>" leaves the expression's own value in %? (cmd_test
        // does this as a side effect); a command-form condition must leave
        // that same value in %? afterward, exactly as real TF does for
        // every command.
        let mut engine = TfEngine::new();
        let result = engine.execute("/if /test 5%; /then /echo yes%; /endif");
        assert!(!matches!(result, TfCommandResult::Error(_)), "{:?}", result);
        assert_eq!(
            engine.get_var("?").map(|v| v.to_string_value()),
            Some("5".to_string())
        );
    }

    #[test]
    fn test_percent_question_after_while_command_form_final_false() {
        // After the loop above exits, %? holds the FALSE-terminating
        // condition's own status ("/@test _i >= 0" with _i == -1: 0/false).
        let mut engine = TfEngine::new();
        engine.set_global("_i", TfValue::Integer(1));
        let result = engine.execute(
            "/while /let _i=$[_i - 1]%; /@test _i >= 0%; /do /echo i=%_i%; /done"
        );
        assert!(!matches!(result, TfCommandResult::Error(_)), "{:?}", result);
        assert_eq!(
            engine.get_var("?").map(|v| v.to_bool()),
            Some(false)
        );
    }

    #[test]
    fn test_if_command_form_missing_then_is_clear_error_not_stuck() {
        let mut engine = TfEngine::new();
        let result = engine.execute("/if /test 1");
        match &result {
            TfCommandResult::Error(e) => assert!(e.contains("/then"), "unexpected error: {}", e),
            other => panic!("expected Error, got {:?}", other),
        }
        assert!(matches!(engine.control_state, ControlState::None));
    }

    #[test]
    fn test_while_command_form_missing_do_is_clear_error_not_stuck() {
        let mut engine = TfEngine::new();
        let result = engine.execute("/while /test 1");
        match &result {
            TfCommandResult::Error(e) => assert!(e.contains("/do"), "unexpected error: {}", e),
            other => panic!("expected Error, got {:?}", other),
        }
        assert!(matches!(engine.control_state, ControlState::None));
    }

    /// Job 15b-i: `split_percent_semi` is deliberately NOT quote-aware -
    /// verified directly against real tf (`/def foo = /let x="a%;b"%;
    /// /echo x=%x` then `/foo` splits INTO the quoted string, leaving
    /// x="a and sending the orphaned `b"` to the world as plain text, not
    /// treating "a%;b" as one string with a literal "%;" inside). self.tf's
    /// own quine (lib_self.tf) depends on exactly this: it has an ODD
    /// count of `"` characters that could never balance under
    /// quote-tracking, and relies on some of its own "%;" splitting apart
    /// what a quote-aware reader would call "inside a string".
    #[test]
    fn test_split_percent_semi_is_not_quote_aware() {
        let parts = split_percent_semi(r#"/let x="a%;b"%;/echo x=%x"#);
        assert_eq!(parts, vec![r#"/let x="a"#, r#"b""#, "/echo x=%x"]);
    }

    /// Job 15b-i: an already-echoed message earlier in the SAME /if
    /// branch must survive a later /return in that branch, not be
    /// silently discarded - real tf: at.tf's own usage-message branch is
    /// exactly "/echo ...%; /set ...%; /return 0", and the echo's text
    /// must still reach the screen even though a /return follows it.
    /// `aggregate_results_with_engine`'s Return/Result arm used to
    /// `return r` outright, dropping every `Success(Some(...))` collected
    /// earlier in the same aggregation; fixed by queueing that text into
    /// `engine.pending_outputs` (the same side channel `echo()` uses)
    /// before propagating the Return/Result marker.
    #[test]
    fn test_echo_before_return_in_same_if_branch_is_not_lost() {
        let mut engine = TfEngine::new();
        engine.execute("/def foo = /if (1) /echo hi%; /return 0%; /endif");
        let result = engine.execute("/foo");
        // The direct return is Success(None) - the actual text went to
        // pending_outputs, mirroring what a real command dispatch would
        // drain (script_tests::run_script / builtins::load_lines /
        // commands::process_pending_tf_outputs all do this).
        assert!(matches!(result, TfCommandResult::Success(None)), "{:?}", result);
        assert_eq!(engine.pending_outputs.len(), 1);
        assert_eq!(engine.pending_outputs[0].text, "hi");
    }
}
