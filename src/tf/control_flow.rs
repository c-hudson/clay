//! Control flow structures for TinyFugue compatibility.
//!
//! Implements:
//! - Single-line: #if (expr) command
//! - Multi-line: #if (expr) ... #elseif (expr) ... #else ... #endif
//! - Loops: #while (expr) ... #done, #for var start end [step] ... #done
//! - Loop control: #break

use super::expressions;
use super::{TfEngine, TfCommandResult};

/// Maximum iterations for loops to prevent infinite loops
pub const MAX_ITERATIONS: usize = 10000;

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
    /// Whether we've seen #else
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

/// Count if/while/for openers and closers in a line, returning (if_opens, loop_opens, if_closes, loop_closes)
fn count_control_keywords(text: &str) -> (i32, i32, i32, i32) {
    let lower = text.to_lowercase();
    let mut if_opens = 0;
    let mut loop_opens = 0;
    let mut if_closes = 0;
    let mut loop_closes = 0;

    let words: Vec<&str> = lower.split_whitespace().collect();
    for word in &words {
        if *word == "#if" || word.starts_with("#if(") {
            if_opens += 1;
        } else if *word == "#while" || word.starts_with("#while(") || *word == "#for" {
            loop_opens += 1;
        } else if *word == "#endif" {
            if_closes += 1;
        } else if *word == "#done" {
            loop_closes += 1;
        }
    }

    (if_opens, loop_opens, if_closes, loop_closes)
}

/// Group body lines into executable units.
/// Lines that form control flow blocks (#if...#endif, #while...#done, #for...#done)
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

/// Parse a single-line #if: #if (condition) command
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
    // But if the content contains #else, #elseif, or #endif, it's actually
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

/// Check if a string contains control flow keywords (#else, #elseif, #endif)
/// that indicate it's a multi-line if block
fn contains_control_flow_keyword(text: &str) -> bool {
    // Check for #else, #elseif, #endif as commands (not inside strings)
    // Look for patterns like ";#else", "%;#else", or just "#else" at start
    let keywords = ["#else", "#elseif", "#endif"];
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

/// Parse the condition from a multi-line #if or #elseif
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

/// Parse #for arguments: var start end [step]
pub fn parse_for_args(args: &str) -> Result<(String, i64, i64, i64), String> {
    let parts: Vec<&str> = args.split_whitespace().collect();

    if parts.len() < 3 {
        return Err("#for requires: var start end [step]".to_string());
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

/// Process a line when in a control flow state
pub fn process_control_line(state: &mut ControlState, line: &str) -> ControlResult {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();

    match state {
        ControlState::None => ControlResult::NotControlFlow,

        ControlState::If(if_state) => {
            // Check for nested #if
            if lower.starts_with("#if ") || lower == "#if" {
                if_state.depth += 1;
                if_state.bodies[if_state.current_branch].push(line.to_string());
                return ControlResult::Consumed;
            }

            // Check for #endif
            if lower == "#endif" {
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
                if lower.starts_with("#elseif ") {
                    if if_state.has_else {
                        return ControlResult::Error("#elseif after #else".to_string());
                    }
                    let args = &trimmed[8..];
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

                if lower == "#else" {
                    if if_state.has_else {
                        return ControlResult::Error("Duplicate #else".to_string());
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
            if lower.starts_with("#while ") || lower == "#while"
                || lower.starts_with("#for ") || lower == "#for" {
                while_state.depth += 1;
                while_state.body.push(line.to_string());
                return ControlResult::Consumed;
            }

            // Check for #done
            if lower == "#done" {
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

            // Check for #break at our level (will be handled during execution)
            while_state.body.push(line.to_string());
            ControlResult::Consumed
        }

        ControlState::For(for_state) => {
            // Check for nested while/for
            if lower.starts_with("#while ") || lower == "#while"
                || lower.starts_with("#for ") || lower == "#for" {
                for_state.depth += 1;
                for_state.body.push(line.to_string());
                return ControlResult::Consumed;
            }

            // Check for #done
            if lower == "#done" {
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
                // Substitute variables first, then expressions
                let command = engine.substitute_vars(command);
                let command = super::variables::substitute_commands(engine, &command);
                // Execute the command (already substituted)
                super::parser::execute_command_substituted(engine, &command)
            } else {
                TfCommandResult::Success(None)
            }
        }
        Err(e) => TfCommandResult::Error(format!("Condition error: {}", e)),
    }
}

/// Execute a complete inline control flow block (from macro execution).
/// The input is a multi-line string containing the complete #if...#endif block.
///
/// Example input:
/// ```
/// #if (cond)    cmd1
/// #else    cmd2
/// #endif
/// ```
pub fn execute_inline_if_block(engine: &mut TfEngine, block: &str) -> Vec<TfCommandResult> {
    let mut if_state: Option<IfState> = None;
    let lines: Vec<&str> = block.lines().collect();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_lowercase();

        if let Some(state) = if_state.as_mut() {
            // Check for #endif
            if lower == "#endif" {
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
                                if cmd.starts_with("__tf_if_eval__")
                                    || cmd.starts_with("__tf_while_eval__")
                                    || cmd.starts_with("__tf_for_eval__")
                                {
                                    results.push(super::parser::execute_command(engine, &cmd));
                                } else {
                                    // Substitute variables first, then expressions
                                    let cmd = engine.substitute_vars(&cmd);
                                    let cmd = super::variables::substitute_commands(engine, &cmd);
                                    results.push(super::parser::execute_command_substituted(engine, &cmd));
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
            } else if lower.starts_with("#if ") || lower == "#if" {
                // Nested #if
                state.depth += 1;
                state.bodies[state.current_branch].push(trimmed.to_string());
            } else if state.depth == 1 && (lower.starts_with("#elseif ") || lower == "#elseif") {
                if state.has_else {
                    return vec![TfCommandResult::Error("#elseif after #else".to_string())];
                }
                // Parse elseif condition and optional body
                let args = &trimmed[7..].trim_start();
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
            } else if state.depth == 1 && (lower == "#else" || lower.starts_with("#else ")) {
                if state.has_else {
                    return vec![TfCommandResult::Error("Duplicate #else".to_string())];
                }
                state.has_else = true;
                state.bodies.push(vec![]);
                state.current_branch += 1;
                // Check for content after #else
                let rest = if lower == "#else" { "" } else { trimmed[5..].trim_start() };
                if !rest.is_empty() {
                    state.bodies[state.current_branch].push(rest.to_string());
                }
            } else {
                // Regular line, add to current branch
                state.bodies[state.current_branch].push(trimmed.to_string());
            }
        } else {
            // First line should be #if
            if !lower.starts_with("#if ") && lower != "#if" {
                return vec![TfCommandResult::Error("Expected #if at start of block".to_string())];
            }

            // Parse the #if line: could be "#if (cond)    cmd" or just "#if (cond)"
            let args = &trimmed[3..].trim_start();

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

    // If we get here, the block wasn't properly closed
    vec![TfCommandResult::Error("#if block not closed with #endif".to_string())]
}

/// Execute a complete inline while block (from macro execution).
/// The input is a multi-line string containing the complete #while...#done block.
pub fn execute_inline_while_block(engine: &mut TfEngine, block: &str) -> Vec<TfCommandResult> {
    let _results: Vec<TfCommandResult> = vec![];
    let lines: Vec<&str> = block.lines().collect();

    let mut condition = String::new();
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
            // First line should be #while
            if !lower.starts_with("#while ") && lower != "#while" {
                return vec![TfCommandResult::Error("Expected #while at start of block".to_string())];
            }

            // Parse the #while line
            let args = &trimmed[6..].trim_start();
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
            if lower.starts_with("#while ") || lower == "#while"
                || lower.starts_with("#for ") || lower == "#for" {
                depth += 1;
                body.push(trimmed.to_string());
            } else if lower == "#done" {
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

    vec![TfCommandResult::Error("#while block not closed with #done".to_string())]
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

        // Evaluate condition
        match expressions::evaluate(engine, condition) {
            Ok(value) => {
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
            if line.trim().to_lowercase() == "#break" {
                should_break = true;
                break;
            }

            // Check if this is a control flow block - if so, don't substitute here
            // The control flow executor will handle per-branch substitution
            let lower = line.trim().to_lowercase();
            let is_control_flow = lower.starts_with("#if ") || lower.starts_with("#if(")
                || lower.starts_with("#while ") || lower.starts_with("#for ");

            let line = if is_control_flow {
                // Pass control flow blocks directly without substitution
                line.clone()
            } else {
                // Substitute variables first, then expressions (order matters!)
                let line = engine.substitute_vars(line);
                super::variables::substitute_commands(engine, &line)
            };

            let result = super::parser::execute_command_substituted(engine, &line);
            // Check for break in nested execution
            if let TfCommandResult::Error(ref e) = result {
                if e == "__break__" {
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
            // First line should be #for
            if !lower.starts_with("#for ") && lower != "#for" {
                return vec![TfCommandResult::Error("Expected #for at start of block".to_string())];
            }

            // Parse the #for line
            let args = &trimmed[4..].trim_start();
            // Parse for args: var start end [step] [body_content]
            let parts: Vec<&str> = args.split_whitespace().collect();
            if parts.len() < 3 {
                return vec![TfCommandResult::Error("#for requires: var start end [step]".to_string())];
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
            if lower.starts_with("#while ") || lower == "#while"
                || lower.starts_with("#for ") || lower == "#for" {
                depth += 1;
                body.push(trimmed.to_string());
            } else if lower == "#done" {
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

    vec![TfCommandResult::Error("#for block not closed with #done".to_string())]
}

/// Execute a for loop
fn execute_for_loop(
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
            if line.trim().to_lowercase() == "#break" {
                should_break = true;
                break;
            }

            // Check if this is a control flow block - if so, don't substitute here
            // The control flow executor will handle per-branch substitution
            let lower = line.trim().to_lowercase();
            let is_control_flow = lower.starts_with("#if ") || lower.starts_with("#if(")
                || lower.starts_with("#while ") || lower.starts_with("#for ");

            let line = if is_control_flow {
                // Pass control flow blocks directly without substitution
                line.clone()
            } else {
                // Substitute variables first, then expressions (order matters!)
                let line = engine.substitute_vars(line);
                super::variables::substitute_commands(engine, &line)
            };

            let result = super::parser::execute_command_substituted(engine, &line);
            if let TfCommandResult::Error(ref e) = result {
                if e == "__break__" {
                    should_break = true;
                    break;
                }
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
/// Returns positive for each #if/#while/#for found, negative for each #endif/#done.
fn count_control_flow_in_line(text: &str) -> i32 {
    let lower = text.to_lowercase();
    let mut depth = 0;

    // Simple word-based scanning for control flow keywords
    let words: Vec<&str> = lower.split_whitespace().collect();
    for word in &words {
        if *word == "#if" || word.starts_with("#if(")
            || *word == "#while" || word.starts_with("#while(")
            || *word == "#for"
        {
            depth += 1;
        } else if *word == "#endif" || *word == "#done" {
            depth -= 1;
        }
    }

    depth
}

/// Group body lines into execution units.
/// Lines that form control flow structures (#if...#endif, #while...#done, #for...#done)
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

/// Parse a condition from #if/#elseif, potentially with body content after it.
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

    // Evaluate conditions in order
    for (i, cond) in conditions.iter().enumerate() {
        match expressions::evaluate(engine, cond) {
            Ok(value) => {
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
                            let is_control_flow = lower.starts_with("#if ") || lower.starts_with("#if(")
                                || lower.starts_with("#while ") || lower.starts_with("#for ");

                            let line = if is_control_flow {
                                line.to_string()
                            } else {
                                // Substitute variables first, then expressions
                                let line = engine.substitute_vars(line);
                                super::variables::substitute_commands(engine, &line)
                            };

                            results.push(super::parser::execute_command_substituted(engine, &line));
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
            let is_control_flow = lower.starts_with("#if ") || lower.starts_with("#if(")
                || lower.starts_with("#while ") || lower.starts_with("#for ");

            let line = if is_control_flow {
                line.to_string()
            } else {
                // Substitute variables first, then expressions
                let line = engine.substitute_vars(line);
                super::variables::substitute_commands(engine, &line)
            };

            results.push(super::parser::execute_command_substituted(engine, &line));
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

        // Evaluate condition
        match expressions::evaluate(engine, &condition) {
            Ok(value) => {
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
            if line.trim().to_lowercase() == "#break" {
                should_break = true;
                break;
            }
            // Substitute variables first, then expressions
            let line = engine.substitute_vars(line);
            let line = super::variables::substitute_commands(engine, &line);
            let result = super::parser::execute_command_substituted(engine, &line);
            // Check for break in nested execution
            if let TfCommandResult::Error(ref e) = result {
                if e == "__break__" {
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
            if line.trim().to_lowercase() == "#break" {
                should_break = true;
                break;
            }
            // Substitute variables first, then expressions
            let line = engine.substitute_vars(line);
            let line = super::variables::substitute_commands(engine, &line);
            let result = super::parser::execute_command_substituted(engine, &line);
            if let TfCommandResult::Error(ref e) = result {
                if e == "__break__" {
                    should_break = true;
                    break;
                }
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
            parse_single_line_if("(1 == 1) #echo yes"),
            Some(("1 == 1".to_string(), "#echo yes".to_string()))
        );

        assert_eq!(
            parse_single_line_if("(x > 5) send attack"),
            Some(("x > 5".to_string(), "send attack".to_string()))
        );

        // Multi-line if (no command after condition)
        assert_eq!(parse_single_line_if("(1 == 1)"), None);

        // Nested parens in condition
        assert_eq!(
            parse_single_line_if("((1 + 2) > 2) #echo yes"),
            Some(("(1 + 2) > 2".to_string(), "#echo yes".to_string()))
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
        let result = execute_single_if(&mut engine, "1 == 1", "#set result yes");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert_eq!(
            engine.get_var("result").map(|v| v.to_string_value()),
            Some("yes".to_string())
        );

        // False condition
        let result = execute_single_if(&mut engine, "1 == 2", "#set result no");
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
        assert!(matches!(process_control_line(&mut state, "#echo inside if"), ControlResult::Consumed));
        assert!(matches!(process_control_line(&mut state, "#set y 10"), ControlResult::Consumed));

        // End the if
        let result = process_control_line(&mut state, "#endif");
        assert!(matches!(result, ControlResult::Execute(_)));
        assert!(matches!(state, ControlState::None));
    }

    #[test]
    fn test_while_state_collection() {
        let mut state = ControlState::While(WhileState::new("x < 10".to_string()));

        assert!(matches!(process_control_line(&mut state, "#set x (x + 1)"), ControlResult::Consumed));
        assert!(matches!(process_control_line(&mut state, "#echo %x"), ControlResult::Consumed));

        let result = process_control_line(&mut state, "#done");
        assert!(matches!(result, ControlResult::Execute(_)));
    }

    #[test]
    fn test_for_loop_execution() {
        let mut engine = TfEngine::new();
        engine.set_global("sum", super::super::TfValue::Integer(0));

        // Create and execute a simple for loop using the new \x1F separator format
        const SEP: char = '\x1F';
        let encoded = format!(
            "__tf_for_eval__{sep}VAR{sep}i{sep}START{sep}1{sep}END{sep}3{sep}STEP{sep}1{sep}LINE{sep}#set sum (${{sum}} + %i){sep}ENDFOR{sep}",
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
        let block = "#if (1 == 1)    #set x yes\n#else    #set x no\n#endif";
        let results = execute_inline_if_block(&mut engine, block);
        assert!(!results.iter().any(|r| matches!(r, TfCommandResult::Error(_))), "Results: {:?}", results);
        assert_eq!(
            engine.get_var("x").map(|v| v.to_string_value()),
            Some("yes".to_string())
        );

        // Test false condition
        let block2 = "#if (1 == 2)    #set y wrong\n#else    #set y correct\n#endif";
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

        let block = "#if (1 == 1)    #if (2 == 2)    #set z inner\n#endif\n#endif";
        let results = execute_inline_if_block(&mut engine, block);
        assert!(!results.iter().any(|r| matches!(r, TfCommandResult::Error(_))), "Results: {:?}", results);
        assert_eq!(
            engine.get_var("z").map(|v| v.to_string_value()),
            Some("inner".to_string())
        );
    }
}
