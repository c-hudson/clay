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

    // If there's content after the condition, it's a single-line if
    if !rest.is_empty() {
        Some((condition, rest.to_string()))
    } else {
        None
    }
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
    // Format: __tf_if_eval__ followed by JSON-like encoding
    let mut encoded = String::from("__tf_if_eval__:");
    for (i, cond) in if_state.conditions.iter().enumerate() {
        encoded.push_str(&format!("COND:{}:", cond));
        for line in &if_state.bodies[i] {
            encoded.push_str(&format!("LINE:{}:", line));
        }
        encoded.push_str("ENDCOND:");
    }
    if if_state.has_else {
        encoded.push_str("ELSE:");
        if let Some(else_body) = if_state.bodies.last() {
            for line in else_body {
                encoded.push_str(&format!("LINE:{}:", line));
            }
        }
        encoded.push_str("ENDELSE:");
    }

    commands.push(encoded);
    ControlResult::Execute(commands)
}

/// Generate commands for a while loop
fn generate_while_commands(while_state: &WhileState) -> Vec<String> {
    // Encode the while structure
    let mut encoded = String::from("__tf_while_eval__:");
    encoded.push_str(&format!("COND:{}:", while_state.condition));
    for line in &while_state.body {
        encoded.push_str(&format!("LINE:{}:", line));
    }
    encoded.push_str("ENDWHILE:");

    vec![encoded]
}

/// Generate commands for a for loop
fn generate_for_commands(for_state: &ForState) -> Vec<String> {
    // Encode the for structure
    let mut encoded = format!(
        "__tf_for_eval__:VAR:{}:START:{}:END:{}:STEP:{}:",
        for_state.var_name, for_state.start, for_state.end, for_state.step
    );
    for line in &for_state.body {
        encoded.push_str(&format!("LINE:{}:", line));
    }
    encoded.push_str("ENDFOR:");

    vec![encoded]
}

/// Execute a single-line if command
pub fn execute_single_if(engine: &mut TfEngine, condition: &str, command: &str) -> TfCommandResult {
    // Evaluate the condition
    match expressions::evaluate(engine, condition) {
        Ok(value) => {
            if value.to_bool() {
                // Execute the command
                super::parser::execute_command(engine, command)
            } else {
                TfCommandResult::Success(None)
            }
        }
        Err(e) => TfCommandResult::Error(format!("Condition error: {}", e)),
    }
}

/// Execute an encoded if block
pub fn execute_if_encoded(engine: &mut TfEngine, encoded: &str) -> Vec<TfCommandResult> {
    let mut results = vec![];

    // Parse the encoded if structure
    let content = encoded.strip_prefix("__tf_if_eval__:").unwrap_or(encoded);

    let mut conditions: Vec<String> = vec![];
    let mut bodies: Vec<Vec<String>> = vec![];
    let mut else_body: Option<Vec<String>> = None;

    let mut current_body: Vec<String> = vec![];
    let mut current_cond: Option<String> = None;
    let mut in_else = false;

    // Simple parser for the encoded format
    let mut remaining = content;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix("COND:") {
            if let Some(end) = rest.find(':') {
                current_cond = Some(rest[..end].to_string());
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix("LINE:") {
            if let Some(end) = rest.find(':') {
                current_body.push(rest[..end].to_string());
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix("ENDCOND:") {
            if let Some(cond) = current_cond.take() {
                conditions.push(cond);
                bodies.push(std::mem::take(&mut current_body));
            }
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("ELSE:") {
            in_else = true;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("ENDELSE:") {
            if in_else {
                else_body = Some(std::mem::take(&mut current_body));
            }
            remaining = rest;
        } else {
            // Skip unknown
            if let Some(idx) = remaining.find(':') {
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
                        for line in body {
                            results.push(super::parser::execute_command(engine, line));
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
        for line in &body {
            results.push(super::parser::execute_command(engine, line));
        }
    }

    results
}

/// Execute an encoded while loop
pub fn execute_while_encoded(engine: &mut TfEngine, encoded: &str) -> Vec<TfCommandResult> {
    let mut results = vec![];

    let content = encoded.strip_prefix("__tf_while_eval__:").unwrap_or(encoded);

    // Parse condition and body
    let mut condition = String::new();
    let mut body: Vec<String> = vec![];

    let mut remaining = content;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix("COND:") {
            if let Some(end) = rest.find(':') {
                condition = rest[..end].to_string();
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix("LINE:") {
            if let Some(end) = rest.find(':') {
                body.push(rest[..end].to_string());
                remaining = &rest[end + 1..];
            } else {
                break;
            }
        } else if remaining.starts_with("ENDWHILE:") {
            break;
        } else if let Some(idx) = remaining.find(':') {
            remaining = &remaining[idx + 1..];
        } else {
            break;
        }
    }

    // Execute while loop with iteration limit
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
        for line in &body {
            if line.trim().to_lowercase() == "#break" {
                should_break = true;
                break;
            }
            let result = super::parser::execute_command(engine, line);
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

    let content = encoded.strip_prefix("__tf_for_eval__:").unwrap_or(encoded);

    // Parse var, start, end, step, and body
    let mut var_name = String::new();
    let mut start: i64 = 0;
    let mut end: i64 = 0;
    let mut step: i64 = 1;
    let mut body: Vec<String> = vec![];

    let mut remaining = content;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix("VAR:") {
            if let Some(idx) = rest.find(':') {
                var_name = rest[..idx].to_string();
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix("START:") {
            if let Some(idx) = rest.find(':') {
                start = rest[..idx].parse().unwrap_or(0);
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix("END:") {
            if let Some(idx) = rest.find(':') {
                end = rest[..idx].parse().unwrap_or(0);
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix("STEP:") {
            if let Some(idx) = rest.find(':') {
                step = rest[..idx].parse().unwrap_or(1);
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if let Some(rest) = remaining.strip_prefix("LINE:") {
            if let Some(idx) = rest.find(':') {
                body.push(rest[..idx].to_string());
                remaining = &rest[idx + 1..];
            } else {
                break;
            }
        } else if remaining.starts_with("ENDFOR:") {
            break;
        } else if let Some(idx) = remaining.find(':') {
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
        for line in &body {
            if line.trim().to_lowercase() == "#break" {
                should_break = true;
                break;
            }
            let result = super::parser::execute_command(engine, line);
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

        // Create and execute a simple for loop
        let for_state = ForState::new("i".to_string(), 1, 3, 1);
        let encoded = format!(
            "__tf_for_eval__:VAR:{}:START:{}:END:{}:STEP:{}:LINE:#set sum (${{sum}} + %i):ENDFOR:",
            for_state.var_name, for_state.start, for_state.end, for_state.step
        );

        let results = execute_for_encoded(&mut engine, &encoded);

        // Should have executed 3 times (i=1,2,3), sum should be 6
        assert!(!results.iter().any(|r| matches!(r, TfCommandResult::Error(_))));
    }
}
