//! TinyFugue command parser.
//!
//! Parses commands starting with `#` and routes them to appropriate handlers.

use super::{TfCommandResult, TfEngine, TfValue, TfHookEvent};
use super::control_flow::{self, ControlState, ControlResult, IfState, WhileState, ForState};
use super::macros;
use super::hooks;
use super::builtins;

/// Parse macro arguments with delimiter-aware splitting.
///
/// This handles the common TF pattern where delimited text (e.g., "x...x")
/// should be preserved as a single argument even if it contains spaces.
/// This is needed for macros like decrypt that use {-1} to get the last
/// argument, expecting it to be the complete delimited text.
fn parse_macro_args(args: &str) -> Vec<&str> {
    if args.is_empty() {
        return vec![];
    }

    let args = args.trim();
    if args.is_empty() {
        return vec![];
    }

    // Split into words first
    let words: Vec<&str> = args.split_whitespace().collect();
    if words.len() <= 1 {
        return words;
    }

    // Check if remaining args (after first) form a delimited pattern
    // Pattern: starts and ends with same single char (like x...x)
    let first_word_end = args.find(char::is_whitespace).unwrap_or(args.len());
    let rest = args[first_word_end..].trim_start();

    if !rest.is_empty() {
        let first_char = rest.chars().next().unwrap();
        let last_char = rest.chars().last().unwrap();

        // If rest starts and ends with same char, and that char appears
        // in the content (like x...x where ... contains x), keep as one arg
        if first_char == last_char && rest.len() > 2 {
            // Check if this delimiter char appears again in the middle
            // (to distinguish x...x patterns from coincidental same start/end)
            let middle = &rest[1..rest.len()-1];
            if middle.contains(first_char) || middle.contains(' ') {
                // This looks like a delimited pattern - keep as single arg
                return vec![&args[..first_word_end], rest];
            }
        }
    }

    // Default: split all on whitespace
    words
}

/// Check if input is a TF command (starts with / prefix)
pub fn is_tf_command(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with('/')
}

/// Check if a command name (without prefix) is a TF command
/// Note: "help" and "gag" are NOT included here because they have Clay equivalents.
/// Use /tfhelp and /tfgag for the TF versions.
fn is_tf_command_name(cmd: &str) -> bool {
    matches!(cmd,
        "set" | "unset" | "let" | "setenv" | "listvar" |
        "echo" | "beep" | "quote" | "substitute" |
        "expr" | "test" | "eval" |
        "if" | "elseif" | "else" | "endif" | "while" | "for" | "done" | "break" |
        "def" | "undef" | "undefn" | "undeft" | "list" | "purge" |
        "bind" | "unbind" | "hook" | "unhook" |
        "load" | "save" | "require" | "loaded" | "lcd" | "log" |
        "sh" | "time" | "recall" | "repeat" | "ps" | "kill" |
        "fg" | "trigger" | "input" | "grab" | "ungag" | "exit" |
        // These are also TF commands (mapped to Clay equivalents)
        "quit" | "dc" | "disconnect" | "world" | "listworlds" |
        "listsockets" | "connections" | "connect" | "addworld" | "version" |
        // Note: "send" maps to Clay's /send command, but TF's #send has different options
        // so we route it through TF to handle -w flag properly
        "send"
    )
}

/// Execute a TF command and return the result.
pub fn execute_command(engine: &mut TfEngine, input: &str) -> TfCommandResult {
    execute_command_impl(engine, input, false)
}

/// Execute a TF command with pre-substituted input (skip variable substitution).
/// Used by control_flow when it has already done substitution.
pub fn execute_command_substituted(engine: &mut TfEngine, input: &str) -> TfCommandResult {
    execute_command_impl(engine, input, true)
}

/// Internal implementation of execute_command.
fn execute_command_impl(engine: &mut TfEngine, input: &str, skip_substitution: bool) -> TfCommandResult {
    let input = input.trim();

    // Check for internal encoded commands (from control flow)
    // These use \x1F (unit separator) as delimiter to avoid conflicts with : in content
    if input.starts_with("__tf_if_eval__\x1F") {
        let results = control_flow::execute_if_encoded(engine, input);
        return aggregate_results(results);
    }
    if input.starts_with("__tf_while_eval__\x1F") {
        let results = control_flow::execute_while_encoded(engine, input);
        return aggregate_results(results);
    }
    if input.starts_with("__tf_for_eval__\x1F") {
        let results = control_flow::execute_for_encoded(engine, input);
        return aggregate_results(results);
    }

    // Check if we're currently in a control flow state
    if !matches!(engine.control_state, ControlState::None) {
        let result = control_flow::process_control_line(&mut engine.control_state, input);
        return match result {
            ControlResult::Consumed => TfCommandResult::Success(None),
            ControlResult::Execute(commands) => {
                // Execute the collected commands
                let mut results = vec![];
                for cmd in commands {
                    results.push(execute_command(engine, &cmd));
                }
                aggregate_results(results)
            }
            ControlResult::Error(e) => {
                engine.control_state = ControlState::None;
                TfCommandResult::Error(e)
            }
            ControlResult::NotControlFlow => {
                // Shouldn't happen, but fall through
                TfCommandResult::Success(None)
            }
        };
    }

    // Handle commands starting with / (unified command system)
    if input.starts_with('/') {
        // Check for /tf prefix (TF-specific commands that conflict with Clay)
        // e.g., /tfhelp, /tfgag
        let lower_input = input.to_lowercase();
        if lower_input.starts_with("/tfhelp") || lower_input.starts_with("/tfgag") {
            // Route to TF-specific handlers
            // Extract command name after /tf prefix
            let cmd_part = input.split_whitespace().next().unwrap_or("");
            let cmd_name = cmd_part[3..].to_lowercase(); // Skip "/tf"
            let args = if input.len() > cmd_part.len() {
                input[cmd_part.len()..].trim_start()
            } else {
                ""
            };
            let tf_cmd = format!("#{} {}", cmd_name, args);
            return execute_tf_specific_command(engine, tf_cmd.trim(), skip_substitution);
        }

        // Parse command name from /command format
        let cmd_part = input.split_whitespace().next().unwrap_or("");
        let cmd_name = cmd_part.trim_start_matches('/').to_lowercase();
        let args = if input.len() > cmd_part.len() {
            input[cmd_part.len()..].trim_start()
        } else {
            ""
        };

        // Check if it's a TF command that should be handled here
        if is_tf_command_name(&cmd_name) {
            // Convert / to # for internal processing
            let tf_cmd = format!("#{} {}", cmd_name, args);
            return execute_command_impl(engine, tf_cmd.trim(), skip_substitution);
        }

        // Check if it's a user-defined macro
        if let Some(macro_def) = engine.macros.iter().find(|m| m.name.eq_ignore_ascii_case(&cmd_name)).cloned() {
            let macro_args: Vec<&str> = parse_macro_args(args);
            let results = super::macros::execute_macro(engine, &macro_def, &macro_args, None);
            return aggregate_results_with_engine(engine, results);
        }

        // Not a TF command or macro - route to Clay
        return TfCommandResult::ClayCommand(input.to_string());
    }

    if !input.starts_with('#') {
        return TfCommandResult::NotTfCommand;
    }

    // Check if this is an inline control flow block (multi-line #while/#for/#if)
    // These should not have variables substituted here - the control flow executor handles it
    let rest_check = input[1..].trim_start();
    let lower_rest = rest_check.to_lowercase();
    let is_inline_control_flow = input.contains('\n') && (
        lower_rest.starts_with("while ") || lower_rest.starts_with("while\n")
        || lower_rest.starts_with("for ") || lower_rest.starts_with("for\n")
        || lower_rest.starts_with("if ") || lower_rest.starts_with("if\n") || lower_rest.starts_with("if(")
    );

    // Check if this is a #def command - if so, don't substitute variables in the body
    // The body should be stored literally and only substituted when executed
    let is_def_command = lower_rest.starts_with("def ")
        || lower_rest.starts_with("def\t")
        || lower_rest == "def";

    // Perform variable and command substitution before parsing (except for #def bodies, inline control flow,
    // or when called with pre-substituted input from control_flow)
    let input = if skip_substitution {
        // Already substituted by caller (control_flow)
        input.to_string()
    } else if is_inline_control_flow {
        // Don't substitute - control flow executor will handle per-iteration substitution
        input.to_string()
    } else if is_def_command {
        // For #def, only substitute variables in options, not in the body
        // Find the = separator and only substitute before it
        if let Some(eq_pos) = input.find('=') {
            let before_eq = &input[..eq_pos];
            let after_eq = &input[eq_pos..];
            let substituted_before = engine.substitute_vars(before_eq);
            let substituted_before = super::variables::substitute_commands(engine, &substituted_before);
            format!("{}{}", substituted_before, after_eq)
        } else {
            // No body (just #def or #def with options but no =), substitute normally
            let input = engine.substitute_vars(input);
            super::variables::substitute_commands(engine, &input)
        }
    } else {
        let input = engine.substitute_vars(input);
        super::variables::substitute_commands(engine, &input)
    };
    let input = input.trim();

    // Skip the # and parse the command
    let rest = &input[1..];

    // Handle empty command
    if rest.is_empty() {
        return TfCommandResult::Error("Empty command".to_string());
    }

    // Split into command and arguments
    let (cmd, args) = split_command(rest);
    let cmd_lower = cmd.to_lowercase();

    match cmd_lower.as_str() {
        // Variable commands
        "set" => cmd_set(engine, args),
        "unset" => cmd_unset(engine, args),
        "let" => cmd_let(engine, args),
        "setenv" => cmd_setenv(engine, args),

        // Output commands
        "echo" => cmd_echo(engine, args),
        "send" => cmd_send(engine, args),
        "substitute" => cmd_substitute(engine, args),

        // Mapped to Clay commands
        "quit" => TfCommandResult::ClayCommand("/quit".to_string()),
        "exit" => builtins::cmd_exit(engine),
        "dc" | "disconnect" => TfCommandResult::ClayCommand("/disconnect".to_string()),
        "world" => cmd_world(args),
        "listworlds" => TfCommandResult::ClayCommand("/worlds".to_string()),
        "listsockets" | "connections" => TfCommandResult::ClayCommand("/connections".to_string()),
        "connect" => cmd_connect(args),
        "addworld" => cmd_addworld(args),

        // Info commands
        "help" => cmd_help(args),
        "version" => cmd_version(),

        // Control flow commands
        "if" => cmd_if(engine, args),
        "elseif" => TfCommandResult::Error("#elseif outside of #if block".to_string()),
        "else" => TfCommandResult::Error("#else outside of #if block".to_string()),
        "endif" => TfCommandResult::Error("#endif without matching #if".to_string()),
        "while" => cmd_while(engine, args),
        "for" => cmd_for(engine, args),
        "done" => TfCommandResult::Error("#done without matching #while or #for".to_string()),
        "break" => TfCommandResult::Error("__break__".to_string()), // Special marker

        // Macro commands
        "def" => cmd_def(engine, args),
        "undef" => cmd_undef(engine, args),
        "undefn" => cmd_undefn(engine, args),
        "undeft" => cmd_undeft(engine, args),
        "list" => cmd_list(engine, args),
        "purge" => cmd_purge(engine),

        // Expression commands
        "expr" => cmd_expr(engine, args),
        "eval" => cmd_eval(engine, args),
        "test" => cmd_test(engine, args),

        // Hook and keybinding commands
        "hook" => cmd_hook(engine, args),
        "unhook" => cmd_unhook(engine, args),
        "bind" => cmd_bind(engine, args),
        "unbind" => cmd_unbind(engine, args),

        // Additional builtins
        "beep" => builtins::cmd_beep(),
        "time" => builtins::cmd_time(args),
        "lcd" => builtins::cmd_lcd(engine, args),
        "sh" => builtins::cmd_sh(args),
        "quote" => builtins::cmd_quote(engine, args),
        "recall" => builtins::cmd_recall(args),
        "gag" => builtins::cmd_gag(engine, args),
        "ungag" => builtins::cmd_ungag(engine, args),
        "load" => builtins::cmd_load(engine, args),
        "require" => builtins::cmd_require(engine, args),
        "loaded" => builtins::cmd_loaded(engine, args),
        "save" => builtins::cmd_save(engine, args),
        "log" => builtins::cmd_log(args),
        "repeat" => builtins::cmd_repeat(engine, args),
        "ps" => builtins::cmd_ps(engine),
        "kill" => builtins::cmd_kill(engine, args),

        // World switching
        "fg" => cmd_fg(args),

        // Variable management
        "listvar" => cmd_listvar(engine, args),

        // Trigger commands
        "trigger" => cmd_trigger(engine, args),

        // Input manipulation
        "input" => cmd_input(engine, args),
        "grab" => cmd_grab(args),

        // Check for user-defined macro with this name
        _ => {
            // Look for a macro with this name (case-insensitive)
            if let Some(macro_def) = engine.macros.iter().find(|m| m.name.eq_ignore_ascii_case(cmd)).cloned() {
                // Parse arguments for the macro with delimiter-aware splitting
                let macro_args: Vec<&str> = parse_macro_args(args);
                let results = macros::execute_macro(engine, &macro_def, &macro_args, None);
                aggregate_results_with_engine(engine, results)
            } else {
                TfCommandResult::UnknownCommand(cmd.to_string())
            }
        }
    }
}

/// Aggregate multiple results into one, queuing SendToMud commands in the engine
fn aggregate_results_with_engine(engine: &mut super::TfEngine, results: Vec<TfCommandResult>) -> TfCommandResult {
    let mut messages = vec![];
    let mut has_error = false;
    let mut pending_clay_commands = vec![];

    for result in results {
        match result {
            TfCommandResult::Success(Some(msg)) => messages.push(msg),
            TfCommandResult::Error(e) if e != "__break__" => {
                messages.push(format!("Error: {}", e));
                has_error = true;
            }
            TfCommandResult::SendToMud(cmd) => {
                // Queue the command to be sent by the main app
                engine.pending_commands.push(super::TfCommand {
                    command: cmd,
                    world: None,
                    no_eol: false,
                });
            }
            TfCommandResult::ClayCommand(cmd) => {
                // Collect clay commands to return (first one wins)
                pending_clay_commands.push(cmd);
            }
            _ => {}
        }
    }

    // If there are pending clay commands, return the first one
    if let Some(clay_cmd) = pending_clay_commands.into_iter().next() {
        return TfCommandResult::ClayCommand(clay_cmd);
    }

    if has_error {
        TfCommandResult::Error(messages.join("\n"))
    } else if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
}

/// Aggregate multiple results into one (without engine access - for tests)
#[allow(dead_code)]
fn aggregate_results(results: Vec<TfCommandResult>) -> TfCommandResult {
    let mut messages = vec![];
    let mut has_error = false;

    for result in results {
        match result {
            TfCommandResult::Success(Some(msg)) => messages.push(msg),
            TfCommandResult::Error(e) if e != "__break__" => {
                messages.push(format!("Error: {}", e));
                has_error = true;
            }
            TfCommandResult::SendToMud(cmd) => {
                // This should be handled by the caller
                messages.push(format!("[send: {}]", cmd));
            }
            TfCommandResult::ClayCommand(cmd) => {
                messages.push(format!("[clay: {}]", cmd));
            }
            _ => {}
        }
    }

    if has_error {
        TfCommandResult::Error(messages.join("\n"))
    } else if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
}

/// Execute TF-specific commands that conflict with Clay (/tfhelp, /tfgag)
/// Input is expected to be in #command format (already converted from /tf prefix)
fn execute_tf_specific_command(engine: &mut TfEngine, input: &str, _skip_substitution: bool) -> TfCommandResult {
    let rest = input.trim_start_matches('#');
    let (cmd, args) = split_command(rest);
    let cmd_lower = cmd.to_lowercase();

    match cmd_lower.as_str() {
        "help" => cmd_help(args),  // TF text-based help (vs Clay /help popup)
        "gag" => builtins::cmd_gag(engine, args),  // TF gag pattern (vs Clay /gag action command)
        _ => TfCommandResult::UnknownCommand(format!("/tf{}", cmd)),
    }
}

/// Split command into name and arguments
fn split_command(input: &str) -> (&str, &str) {
    if let Some(space_idx) = input.find(char::is_whitespace) {
        let cmd = &input[..space_idx];
        let args = input[space_idx..].trim_start();
        (cmd, args)
    } else {
        (input, "")
    }
}

// =============================================================================
// Command Implementations
// =============================================================================

/// #set varname=value - Set a global variable
/// Supports both #set var=value and #set var = value
fn cmd_set(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        // No args: list all variables
        if engine.global_vars.is_empty() {
            return TfCommandResult::Success(Some("No variables set.".to_string()));
        }
        let mut lines: Vec<String> = engine
            .global_vars
            .iter()
            .map(|(k, v)| format!("{}={}", k, v.to_string_value()))
            .collect();
        lines.sort();
        return TfCommandResult::Success(Some(lines.join("\n")));
    }

    // Parse name=value or name = value
    let (name, value) = if let Some(eq_pos) = args.find('=') {
        let name = args[..eq_pos].trim();
        let value = args[eq_pos + 1..].trim();
        (name, value)
    } else {
        // No = found, treat as name with empty value (or could be just a name to query)
        let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
        let name = parts[0];
        let value = if parts.len() > 1 { parts[1] } else { "" };
        (name, value)
    };

    // Validate variable name
    if !is_valid_var_name(name) {
        return TfCommandResult::Error(format!(
            "Invalid variable name '{}': must start with letter and contain only letters, numbers, underscores",
            name
        ));
    }

    engine.set_global(name, TfValue::from(value));
    TfCommandResult::Success(None)
}

/// #unset varname - Remove a global variable
fn cmd_unset(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.trim();

    if name.is_empty() {
        return TfCommandResult::Error("Usage: #unset varname".to_string());
    }

    if engine.unset_global(name) {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Error(format!("Variable '{}' not found", name))
    }
}

/// #let varname=value - Set a local variable
/// Supports both #let var=value and #let var = value
fn cmd_let(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: #let varname=value".to_string());
    }

    // Parse name=value or name = value
    let (name, value) = if let Some(eq_pos) = args.find('=') {
        let name = args[..eq_pos].trim();
        let value = args[eq_pos + 1..].trim();
        (name, value)
    } else {
        // No = found, treat as name with empty value
        let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
        let name = parts[0];
        let value = if parts.len() > 1 { parts[1] } else { "" };
        (name, value)
    };

    if !is_valid_var_name(name) {
        return TfCommandResult::Error(format!(
            "Invalid variable name '{}': must start with letter and contain only letters, numbers, underscores",
            name
        ));
    }

    let value = TfValue::from(value);

    engine.set_local(name, value);
    TfCommandResult::Success(None)
}

/// #setenv varname value - Set an environment variable (exported to shell)
fn cmd_setenv(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();

    if parts.is_empty() || parts[0].is_empty() {
        return TfCommandResult::Error("Usage: #setenv varname value".to_string());
    }

    let name = parts[0];

    if !is_valid_var_name(name) {
        return TfCommandResult::Error(format!("Invalid variable name '{}'", name));
    }

    let value = if parts.len() > 1 {
        TfValue::from(parts[1])
    } else {
        TfValue::String(String::new())
    };

    engine.set_global(name, value);
    engine.env_vars.insert(name.to_string());

    // Also set in actual environment
    std::env::set_var(name, if parts.len() > 1 { parts[1] } else { "" });

    TfCommandResult::Success(None)
}

/// #echo [-w world] [-a attrs] [--] message - Display message locally
/// Options:
///   -w<world> or -w <world> - Specify world (currently ignored)
///   -a<attrs> - Attributes: g=gag (currently ignored), h=highlight, etc.
///   -- - End of options marker
fn cmd_echo(engine: &TfEngine, args: &str) -> TfCommandResult {
    let _ = engine;  // Engine already used for substitution

    // Parse options: -w<world>, -a<attrs>, --
    let mut remaining = args.trim();
    let mut _world: Option<String> = None;
    let mut _attrs: Option<String> = None;

    while !remaining.is_empty() {
        if remaining.starts_with("--") {
            // End of options marker
            remaining = remaining[2..].trim_start();
            break;
        } else if remaining.starts_with("-w") {
            // -w<world> or -w <world>
            remaining = &remaining[2..];
            if remaining.starts_with(' ') {
                // -w <world>
                remaining = remaining.trim_start();
                if let Some(space_pos) = remaining.find(' ') {
                    _world = Some(remaining[..space_pos].to_string());
                    remaining = remaining[space_pos..].trim_start();
                } else {
                    _world = Some(remaining.to_string());
                    remaining = "";
                }
            } else {
                // -w<world> (no space)
                if let Some(space_pos) = remaining.find(' ') {
                    _world = Some(remaining[..space_pos].to_string());
                    remaining = remaining[space_pos..].trim_start();
                } else {
                    _world = Some(remaining.to_string());
                    remaining = "";
                }
            }
        } else if remaining.starts_with("-a") {
            // -a<attrs>
            remaining = &remaining[2..];
            if let Some(space_pos) = remaining.find(' ') {
                _attrs = Some(remaining[..space_pos].to_string());
                remaining = remaining[space_pos..].trim_start();
            } else {
                _attrs = Some(remaining.to_string());
                remaining = "";
            }
        } else if remaining.starts_with('-') && remaining.len() > 1 {
            // Unknown option, skip it
            if let Some(space_pos) = remaining.find(' ') {
                remaining = remaining[space_pos..].trim_start();
            } else {
                remaining = "";
            }
        } else {
            // Not an option, this is the message
            break;
        }
    }

    // Process TF attribute codes: @{attr} sequences
    // @{B} = bold, @{U} = underline, @{n} = normal/reset
    // @{Crgb} = foreground color, @{BCrgb} = background color
    let message = process_attr_codes(remaining);
    TfCommandResult::Success(Some(message))
}

/// #substitute [-a attrs] [--] text - Replace trigger line with substituted text
/// Options:
///   -a<attrs> - Attributes: C=color (e.g., Cred, Cgreen, Cbold)
///   -- - End of options marker
fn cmd_substitute(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Parse options: -a<attrs>, --
    let mut remaining = args.trim();
    let mut attrs = String::new();

    while !remaining.is_empty() {
        if remaining.starts_with("--") {
            remaining = remaining[2..].trim_start();
            break;
        } else if remaining.starts_with("-a") {
            // -a<attrs>
            remaining = &remaining[2..];
            if let Some(space_pos) = remaining.find(' ') {
                attrs = remaining[..space_pos].to_string();
                remaining = remaining[space_pos..].trim_start();
            } else {
                attrs = remaining.to_string();
                remaining = "";
            }
        } else if remaining.starts_with('-') && remaining.len() > 1 {
            // Unknown option, skip it
            if let Some(space_pos) = remaining.find(' ') {
                remaining = remaining[space_pos..].trim_start();
            } else {
                remaining = "";
            }
        } else {
            break;
        }
    }

    // Process TF attribute codes in the text
    let text = process_attr_codes(remaining);

    // Queue the substitution for main app to process
    engine.pending_substitution = Some(super::TfSubstitution {
        text,
        attrs,
    });

    TfCommandResult::Success(None)
}

/// Process TF attribute codes in text
/// @{B} = bold, @{U} = underline, @{n} = normal/reset
/// @{Crgb} = foreground color (where r,g,b are 0-5)
/// @{BCrgb} = background color
fn process_attr_codes(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '@' && i + 1 < len && chars[i + 1] == '{' {
            // Find closing brace
            if let Some(end) = chars[i + 2..].iter().position(|&c| c == '}') {
                let attr: String = chars[i + 2..i + 2 + end].iter().collect();
                let ansi = attr_to_ansi(&attr);
                result.push_str(&ansi);
                i = i + 3 + end;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Convert TF attribute code to ANSI escape sequence
fn attr_to_ansi(attr: &str) -> String {
    match attr.to_uppercase().as_str() {
        // Basic attributes
        "N" | "NORMAL" => "\x1b[0m".to_string(),
        "B" | "BOLD" => "\x1b[1m".to_string(),
        "D" | "DIM" => "\x1b[2m".to_string(),
        "U" | "UNDERLINE" => "\x1b[4m".to_string(),
        "BLINK" | "FLASH" => "\x1b[5m".to_string(),
        "R" | "REVERSE" => "\x1b[7m".to_string(),

        // Standard colors (foreground)
        "BLACK" => "\x1b[30m".to_string(),
        "RED" => "\x1b[31m".to_string(),
        "GREEN" => "\x1b[32m".to_string(),
        "YELLOW" => "\x1b[33m".to_string(),
        "BLUE" => "\x1b[34m".to_string(),
        "MAGENTA" => "\x1b[35m".to_string(),
        "CYAN" => "\x1b[36m".to_string(),
        "WHITE" => "\x1b[37m".to_string(),

        // Standard colors (background)
        "BGBLACK" => "\x1b[40m".to_string(),
        "BGRED" => "\x1b[41m".to_string(),
        "BGGREEN" => "\x1b[42m".to_string(),
        "BGYELLOW" => "\x1b[43m".to_string(),
        "BGBLUE" => "\x1b[44m".to_string(),
        "BGMAGENTA" => "\x1b[45m".to_string(),
        "BGCYAN" => "\x1b[46m".to_string(),
        "BGWHITE" => "\x1b[47m".to_string(),

        // 216-color cube: Crgb where r,g,b are 0-5
        _ if attr.len() == 4 && attr.starts_with('C') => {
            if let (Some(r), Some(g), Some(b)) = (
                attr.chars().nth(1).and_then(|c| c.to_digit(10)),
                attr.chars().nth(2).and_then(|c| c.to_digit(10)),
                attr.chars().nth(3).and_then(|c| c.to_digit(10)),
            ) {
                if r <= 5 && g <= 5 && b <= 5 {
                    // Convert to 256-color code: 16 + 36*r + 6*g + b
                    let code = 16 + 36 * r + 6 * g + b;
                    return format!("\x1b[38;5;{}m", code);
                }
            }
            String::new()
        }

        // Background 216-color: BCrgb
        _ if attr.len() == 5 && attr.starts_with("BC") => {
            if let (Some(r), Some(g), Some(b)) = (
                attr.chars().nth(2).and_then(|c| c.to_digit(10)),
                attr.chars().nth(3).and_then(|c| c.to_digit(10)),
                attr.chars().nth(4).and_then(|c| c.to_digit(10)),
            ) {
                if r <= 5 && g <= 5 && b <= 5 {
                    let code = 16 + 36 * r + 6 * g + b;
                    return format!("\x1b[48;5;{}m", code);
                }
            }
            String::new()
        }

        // Unknown attribute - return empty
        _ => String::new(),
    }
}

/// #send [-w world] text - Send text to MUD
fn cmd_send(_engine: &TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: #send [-w world] text".to_string());
    }

    // Check for -w flag
    if let Some(stripped) = args.strip_prefix("-w") {
        // Parse -w world syntax
        let rest = stripped.trim_start();
        if let Some(space_idx) = rest.find(char::is_whitespace) {
            let world = &rest[..space_idx];
            let text = rest[space_idx..].trim_start();
            // Map to Clay /send command
            return TfCommandResult::ClayCommand(format!("/send -w{} {}", world, text));
        } else {
            return TfCommandResult::Error("Usage: #send -w world text".to_string());
        }
    }

    // Simple send
    TfCommandResult::SendToMud(args.to_string())
}

/// #world [name] - Switch to or connect to a world
fn cmd_world(args: &str) -> TfCommandResult {
    let name = args.trim();

    if name.is_empty() {
        // No argument: list worlds (same as /worlds)
        TfCommandResult::ClayCommand("/worlds".to_string())
    } else {
        // Connect/switch to named world
        TfCommandResult::ClayCommand(format!("/worlds {}", name))
    }
}

/// #connect [world] - Connect to a world
fn cmd_connect(args: &str) -> TfCommandResult {
    let name = args.trim();

    if name.is_empty() {
        // Connect current world
        TfCommandResult::ClayCommand("/connect".to_string())
    } else {
        // Connect named world
        TfCommandResult::ClayCommand(format!("/worlds {}", name))
    }
}

/// #addworld - Define a new world or redefine an existing world
///
/// Command usage:
///   #addworld [-xe] [-Ttype] name [char pass] host port
///   #addworld [-Ttype] name
///
/// Options:
///   -x  Use SSL/TLS for connections
///   -e  Echo sent text back (ignored in Clay)
///   -Ttype  World type (ignored in Clay, defaults to MUD)
///
/// Examples:
///   #addworld MyMUD mud.example.com 4000
///   #addworld -x SecureMUD secure.example.com 4443
///   #addworld MyMUD player password mud.example.com 4000
fn cmd_addworld(args: &str) -> TfCommandResult {
    // Pass through to Clay's /addworld command which handles the actual creation
    if args.trim().is_empty() {
        return TfCommandResult::Error("Usage: #addworld [-xe] [-Ttype] name [char pass] host port".to_string());
    }
    TfCommandResult::ClayCommand(format!("/addworld {}", args))
}

/// /help [topic] or /tfhelp [topic] - Display TF help
fn cmd_help(args: &str) -> TfCommandResult {
    let topic = args.trim().trim_start_matches('/').trim_start_matches('#').to_lowercase();

    if topic.is_empty() {
        let help_text = r#"TinyFugue Commands (use / or # prefix)

Variables:
  /set [name [value]]  - Set/list global variables
  /unset name          - Remove a variable
  /let name value      - Set a local variable
  /setenv name         - Export variable to environment
  /listvar [pattern]   - List variables

Expressions:
  /expr expression     - Evaluate and display result
  /test expression     - Evaluate expression, set %?
  /eval expression     - Evaluate and execute as command

Control Flow:
  /if (expr) cmd       - Conditional execution
  /if /elseif /else /endif - Multi-line conditional
  /while (expr) /done  - While loop
  /for var s e [step] /done - For loop
  /break               - Exit loop

Macros/Triggers:
  /def [opts] name=body - Define macro (-t -m -p -F -1 -ag -h -b)
  /undef name          - Remove macro
  /list [pattern]      - List macros
  /purge [pattern]     - Remove all macros

Hooks & Keys:
  /bind key=command    - Bind key to command
  /unbind key          - Remove key binding

Output:
  /echo message        - Display message locally
  /send [-w world] text - Send text to MUD
  /beep                - Terminal bell
  /quote text          - Send without substitution
  /tfgag pattern       - Suppress matching lines (TF)
  /ungag pattern       - Remove gag
  /recall [pattern]    - Search output history

World Management:
  /fg [name]           - Switch to or list worlds
  /connect [world]     - Connect to a world
  /addworld [opts] name [args] - Add/update world
  /dc, /disconnect     - Disconnect current world

File Operations:
  /load [-q] filename  - Load TF script (-q = quiet)
  /require [-q] file   - Load file if not already loaded
  /loaded token        - Mark file as loaded (for /require)
  /save filename       - Save macros to file
  /lcd path            - Change local directory
  /exit                - Abort loading file / quit

Process:
  /repeat [opts] count cmd - Repeat command on timer
  /ps                  - List background processes
  /kill id             - Kill background process

Misc:
  /time                - Display current time
  /sh command          - Execute shell command
  /tfhelp [topic]      - Show this TF help
  /version             - Show version info
  /quit                - Exit Clay

Note: Use # prefix for backward compatibility (e.g., #set, #echo).
      Use /tfhelp and /tfgag for TF versions of conflicting commands.

Variable Substitution:
  %{varname}           - Variable value
  %1-%9, %*            - Positional params from trigger
  %L, %R               - Left/right of match
  %%                   - Literal percent sign

Use /tfhelp <command> for detailed help on specific commands.
Use /tfhelp functions for list of expression functions."#;
        TfCommandResult::Success(Some(help_text.to_string()))
    } else {
        match topic.as_str() {
            "set" => TfCommandResult::Success(Some(
                "/set [name [value]]  (or #set)\n\nSet a global variable. Without arguments, lists all variables.\nExamples:\n  /set foo bar    - Set foo to \"bar\"\n  /set count 42   - Set count to 42\n  /set            - List all variables".to_string()
            )),
            "echo" => TfCommandResult::Success(Some(
                "/echo message  (or #echo)\n\nDisplay a message locally (not sent to MUD).\nVariable substitution is performed on the message.\nExample: /echo Hello %{name}!".to_string()
            )),
            "send" => TfCommandResult::Success(Some(
                "/send [-w world] text  (or #send)\n\nSend text to the MUD server.\n-w world: Send to specific world\nExample: /send say Hello everyone!".to_string()
            )),
            "def" => TfCommandResult::Success(Some(
                r#"/def [options] name = body  (or #def)

Define a macro. Options:
  -t"pattern"   Trigger pattern (fires on matching MUD output)
  -mtype        Match type: simple, glob (default), regexp
  -p priority   Execution priority (higher = first)
  -F            Fall-through (continue checking other triggers)
  -1            One-shot (delete after firing once)
  -n count      Fire only N times
  -ag           Gag (suppress) matched line
  -ah           Highlight matched line
  -ab           Bold
  -au           Underline
  -E"expr"      Conditional (only fire if expression is true)
  -c chance     Probability (0.0-1.0)
  -w world      Restrict to specific world
  -h event      Hook event (CONNECT, DISCONNECT, etc.)
  -b"key"       Key binding

Examples:
  /def -t"You are hungry" eat = get food bag%; eat food
  /def -t"^(\w+) tells you" -mregexp reply = tell %1 Got it!
  /def -hCONNECT greet = look"#.to_string()
            )),
            "if" => TfCommandResult::Success(Some(
                "/if (expression) command  (or #if)\n/if (expr) ... /elseif (expr) ... /else ... /endif\n\nConditional execution.\nExamples:\n  /if (hp < 50) cast heal\n  /if (%1 == \"yes\") /echo Confirmed /else /echo Cancelled /endif".to_string()
            )),
            "while" => TfCommandResult::Success(Some(
                "/while (expression) ... /done  (or #while)\n\nRepeat commands while expression is true.\nExample:\n  /while (count < 10) /echo %count%; /set count $[count+1] /done".to_string()
            )),
            "for" => TfCommandResult::Success(Some(
                "/for variable start end [step] ... /done  (or #for)\n\nLoop from start to end.\nExample:\n  /for i 1 5 /echo Number %i /done".to_string()
            )),
            "expr" => TfCommandResult::Success(Some(
                "/expr expression  (or #expr)\n\nEvaluate expression and display result.\nOperators: + - * / % == != < > <= >= & | ! =~ !~ ?:\nFunctions: strlen() substr() strcat() tolower() toupper() rand() time() abs() min() max()\nExample: /expr 2 + 2 * 3".to_string()
            )),
            "test" => TfCommandResult::Success(Some(
                r#"#test expression

Evaluate expression and return its value, setting %?.

Evaluates the expression and returns its value (any type).
Also sets the special variable %? to the result.
Useful for evaluating expressions for side effects.

Examples:
  #test 2 + 2           - Returns 4, sets %? to 4
  #test strlen("hello") - Returns 5, sets %? to 5
  #test hp < 50         - Returns 1 or 0, sets %?

Unlike #expr, #test does not display the result automatically.
The result is stored in %? for later use."#.to_string()
            )),
            "bind" => TfCommandResult::Success(Some(
                "#bind key = command\n\nBind a key to execute a command.\nKey names: F1-F12, ^A-^Z (Ctrl), @a-@z (Alt), PgUp, PgDn, Home, End, Insert, Delete\nExample: #bind F5 = cast heal".to_string()
            )),
            "hook" | "hooks" => TfCommandResult::Success(Some(
                "Hooks fire macros on events. Use #def -h<event> to register.\n\nEvents:\n  CONNECT     - When connected to MUD\n  DISCONNECT  - When disconnected\n  LOGIN       - After login\n  PROMPT      - On prompt received\n  SEND        - Before sending command\n\nExample: #def -hCONNECT auto_look = look".to_string()
            )),
            "repeat" => TfCommandResult::Success(Some(
                r#"#repeat [-w[world]] {[-time]|-S|-P} count command

Repeat a command on a timer. First iteration runs immediately,
then waits the interval before each subsequent iteration.

Options:
  -w[world]  Send to specific world (empty = current)
  -S         Synchronous (execute all iterations now)
  -P         Execute on prompt (not yet implemented)
  -time      Interval: seconds, M:S, or H:M:S

Count: integer or "i" for infinite

Examples:
  #repeat -30 5 #echo hi        - Now, then every 30s, 5 times total
  #repeat -0:30 i #echo hi      - Now, then every 30s, infinite
  #repeat -1:0:0 1 #echo hourly - Once now (1 hour interval unused)
  #repeat -S 3 #echo sync       - 3 times immediately"#.to_string()
            )),
            "ps" => TfCommandResult::Success(Some(
                "#ps\n\nList all background processes (from #repeat).\nShows PID, interval, remaining count, and command.".to_string()
            )),
            "kill" => TfCommandResult::Success(Some(
                "#kill pid\n\nKill a background process by its PID.\nUse #ps to see running processes.".to_string()
            )),
            "load" => TfCommandResult::Success(Some(
                r#"#load [-q] filename

Load and execute commands from a TF script file.

Options:
  -q  Quiet mode - don't echo "% Loading commands from..." message

The file may contain:
  - TF commands starting with # (e.g., #def, #set)
  - Clay commands starting with / (e.g., /connect)
  - Comments: lines starting with ; or single # followed by space
  - Blank lines (ignored)

Line continuation: End a line with \ to continue on next line.
Use %\ for a literal backslash at end of line.

File search order (for relative paths):
  1. Current directory (from #lcd or actual cwd)
  2. Directories in $TFPATH (colon-separated)
  3. $TFLIBDIR

Use #exit to abort loading early.

Example:
  #load ~/.tf/init.tf
  #load -q mylib.tf"#.to_string()
            )),
            "require" => TfCommandResult::Success(Some(
                r#"#require [-q] filename

Load a file only if not already loaded via #loaded.

Same as #load, but if the file has already registered a token
via #loaded, the file will not be read again.

Files designed for #require should have #loaded as their first
command with a unique token (usually the file's full path).

Example file (mylib.tf):
  #loaded mylib.tf
  #def myfunc = #echo Hello from mylib

Usage:
  #require mylib.tf   - Loads the file
  #require mylib.tf   - Does nothing (already loaded)"#.to_string()
            )),
            "loaded" => TfCommandResult::Success(Some(
                r#"#loaded token

Mark a file as loaded (for use with #require).

Should be the first command in a file designed for #require.
If the token has already been registered by a previous #loaded
call, the current file load is aborted (returns success).

Token should be unique - the file's full path is recommended.

Example (in mylib.tf):
  #loaded mylib.tf
  ; Rest of file only executed once"#.to_string()
            )),
            "exit" => TfCommandResult::Success(Some(
                r#"#exit

Abort loading the current file early.

When called during #load or #require, stops reading the
current file immediately. Commands already executed are
not undone.

When called outside of file loading, #exit is equivalent
to #quit (exits Clay)."#.to_string()
            )),
            "addworld" => TfCommandResult::Success(Some(
                r#"#addworld [-xe] [-Ttype] name [char pass] host port

Define a new world or update an existing world.

Command form:
  #addworld MyMUD mud.example.com 4000
  #addworld -x SecureMUD secure.example.com 4443
  #addworld MyMUD player password mud.example.com 4000

Function form:
  addworld(name, type, host, port, char, pass, file, flags)

Options:
  -x    Use SSL/TLS for connections
  -e    Echo sent text (ignored)
  -Ttype World type (ignored, defaults to MUD)
  -p    No proxy (ignored)

Function flags string:
  "x" = use SSL

Examples:
  #addworld Cave cave.tcp.com 2283
  #addworld -x Secure secure.tcp.com 4443
  #test addworld("Cave", "", "cave.tcp.com", "2283")
  #test addworld("Secure", "", "ssl.tcp.com", "4443", "", "", "", "x")"#.to_string()
            )),
            "functions" | "func" | "funcs" => TfCommandResult::Success(Some(
                r#"Expression Functions

String Functions:
  strlen(str)              - Length of string
  substr(str, start [,len]) - Substring extraction
  strcat(s1, s2, ...)      - Concatenate strings
  strstr(str, substr)      - Find substring position (-1 if not found)
  strchr(str, chars)       - Find first char from set (-1 if not found)
  strrchr(str, chars)      - Find last char from set (-1 if not found)
  strcmp(s, t)             - Compare strings (<0, 0, >0)
  strncmp(s, t, n)         - Compare first n chars
  strrep(str, n)           - Repeat string n times
  tolower(str)             - Convert to lowercase
  toupper(str)             - Convert to uppercase
  replace(str, old, new [,count]) - Replace occurrences
  ascii(str)               - ASCII code of first character
  char(code)               - Character from ASCII code
  sprintf(fmt, args...)    - Formatted string (%s, %d, %c, %%)
  pad(s, w, ...)           - Pad strings (+ = right-justify, - = left)

Math Functions:
  abs(n)                   - Absolute value
  min(a, b, ...)           - Minimum value
  max(a, b, ...)           - Maximum value
  mod(i, j)                - Remainder of i / j
  trunc(x)                 - Integer part of float
  rand([max])              - Random number
  rand(min, max)           - Random in range [min, max]
  sin(x), cos(x), tan(x)   - Trigonometric (radians)
  asin(x), acos(x), atan(x) - Inverse trig
  exp(x)                   - e^x
  pow(x, y)                - x^y
  sqrt(x)                  - Square root
  log(x)                   - Natural logarithm
  log10(x)                 - Base-10 logarithm

Pattern Matching:
  regmatch(pattern, str)   - Regex match, sets %P0-%P9 captures

World Functions:
  fg_world()               - Current world name
  world_info(field [,world]) - Get world info (name/host/port/character)
  nactive()                - Count of connected worlds
  nworlds()                - Total world count
  is_connected([world])    - Check if world is connected
  idle()                   - Seconds since last input
  sidle()                  - Seconds since last send

Info Functions:
  time()                   - Current Unix timestamp
  ftime(fmt, time)         - Format timestamp (%Y %m %d %H %M %S)
  columns()                - Screen width
  lines()                  - Screen height
  moresize()               - Lines queued at more prompt
  getpid()                 - Process ID
  systype()                - System type ("unix")
  filename(path)           - Expand ~ in path

Macro Functions:
  ismacro(name)            - Check if macro exists
  getopts(opts, args)      - Parse command options

Command Functions:
  echo(text [,attrs])      - Display local message (queues output)
  send(text [,world])      - Send text to MUD (queues command)
  substitute(text [,attrs]) - Replace trigger line with text
  keycode(str)             - Key sequence for string (^X for ctrl)

Keyboard Buffer:
  kbhead()                 - Text before cursor
  kbtail()                 - Text after cursor
  kbpoint()                - Cursor position
  kblen()                  - Input buffer length
  kbgoto(pos)              - Move cursor to position
  kbdel(count)             - Delete characters
  kbmatch([pos])           - Find matching brace/paren
  kbword()                 - Word at cursor
  kbwordleft([pos])        - Position of word start left of pos
  kbwordright([pos])       - Position past word end right of pos
  input(text)              - Insert text at cursor

File I/O:
  tfopen(path, mode)       - Open file (r/w/a), returns handle
  tfclose(handle)          - Close file
  tfread(handle, var)      - Read line into variable
  tfwrite(handle, text)    - Write text to file
  tfflush(handle)          - Flush file buffer
  tfeof(handle)            - Check for end of file
  fwrite(file, text)       - Append text to file (simple)

Usage: $[function(args)] or in #expr/#test"#.to_string()
            )),
            _ => TfCommandResult::Success(Some(format!("No help available for '{}'\nTry: set, echo, send, def, if, while, for, expr, bind, hooks, repeat, ps, kill, load, require, loaded, exit, addworld, functions", topic))),
        }
    }
}

/// #version - Show version info
fn cmd_version() -> TfCommandResult {
    TfCommandResult::Success(Some(crate::get_version_string()))
}

/// #expr expression - Evaluate expression and display result
fn cmd_expr(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: #expr expression".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => TfCommandResult::Success(Some(value.to_string_value())),
        Err(e) => TfCommandResult::Error(format!("Expression error: {}", e)),
    }
}

/// #eval expression - Evaluate expression and execute result as command
fn cmd_eval(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: #eval expression".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => {
            let cmd = value.to_string_value();
            if cmd.is_empty() {
                TfCommandResult::Success(None)
            } else if cmd.starts_with('#') {
                // Execute as TF command (recursive)
                execute_command(engine, &cmd)
            } else if cmd.starts_with('/') {
                // Execute as Clay command
                TfCommandResult::ClayCommand(cmd)
            } else {
                // Send to MUD
                TfCommandResult::SendToMud(cmd)
            }
        }
        Err(e) => TfCommandResult::Error(format!("Expression error: {}", e)),
    }
}

/// #test expression - Evaluate expression and return its value, setting %?
///
/// Evaluates the expression and returns its value (any type).
/// Also sets the special variable %? to the result.
/// Useful for evaluating expressions for side effects.
///
/// Examples:
///   #test 2 + 2           -> returns 4, sets %? to 4
///   #test strlen("hello") -> returns 5, sets %? to 5
///   #test regmatch("foo(.*)", "foobar") -> sets %P1 to "bar"
fn cmd_test(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: #test expression".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => {
            // Set the special %? variable to the result
            engine.set_global("?", value.clone());
            // #test is silent - it only sets %?, doesn't produce output
            TfCommandResult::Success(None)
        }
        Err(e) => TfCommandResult::Error(format!("Expression error: {}", e)),
    }
}

/// Check if a variable name is valid (starts with letter, contains only alphanumeric and underscore)
fn is_valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {
            chars.all(|c| c.is_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

// =============================================================================
// Control Flow Commands
// =============================================================================

/// #if (condition) [command] - Conditional execution
fn cmd_if(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Check if this is a complete inline block (from macro execution)
    // These contain newlines and the full #if...#endif structure
    if args.contains('\n') && args.to_lowercase().contains("#endif") {
        // Reconstruct the full block by prepending "#if "
        let full_block = format!("#if {}", args);
        let results = control_flow::execute_inline_if_block(engine, &full_block);
        return aggregate_inline_results(results);
    }

    // Check for single-line form: #if (condition) command
    if let Some((condition, command)) = control_flow::parse_single_line_if(args) {
        return control_flow::execute_single_if(engine, &condition, &command);
    }

    // Multi-line form: #if (condition)
    match control_flow::parse_condition(args) {
        Ok(condition) => {
            engine.control_state = ControlState::If(IfState::new(condition));
            TfCommandResult::Success(None)
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// Aggregate results from inline control flow execution
fn aggregate_inline_results(results: Vec<TfCommandResult>) -> TfCommandResult {
    let mut messages = vec![];
    let mut has_error = false;

    for result in results {
        match result {
            TfCommandResult::Success(Some(msg)) => messages.push(msg),
            TfCommandResult::Error(e) => {
                messages.push(format!("Error: {}", e));
                has_error = true;
            }
            _ => {}
        }
    }

    if has_error {
        TfCommandResult::Error(messages.join("\n"))
    } else if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
}

/// #while (condition) - Start a while loop
fn cmd_while(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Check if this is a complete inline block (from macro execution)
    // These contain newlines and the full #while...#done structure
    if args.contains('\n') && args.to_lowercase().contains("#done") {
        // Reconstruct the full block by prepending "#while "
        let full_block = format!("#while {}", args);
        let results = control_flow::execute_inline_while_block(engine, &full_block);
        return aggregate_inline_results(results);
    }

    match control_flow::parse_condition(args) {
        Ok(condition) => {
            engine.control_state = ControlState::While(WhileState::new(condition));
            TfCommandResult::Success(None)
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// #for var start end [step] - Start a for loop
fn cmd_for(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Check if this is a complete inline block (from macro execution)
    if args.contains('\n') && args.to_lowercase().contains("#done") {
        let full_block = format!("#for {}", args);
        let results = control_flow::execute_inline_for_block(engine, &full_block);
        return aggregate_inline_results(results);
    }

    match control_flow::parse_for_args(args) {
        Ok((var_name, start, end, step)) => {
            engine.control_state = ControlState::For(ForState::new(var_name, start, end, step));
            TfCommandResult::Success(None)
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

// =============================================================================
// Macro Commands
// =============================================================================

/// #def [options] name = body - Define a macro
fn cmd_def(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.trim().is_empty() {
        // No args: list all macros
        return TfCommandResult::Success(Some(macros::list_macros(engine, None)));
    }

    match macros::parse_def(args) {
        Ok(macro_def) => {
            // Check if macro with same name exists
            let existing_idx = engine.macros.iter().position(|m| m.name == macro_def.name);

            // Register hook if present
            if let Some(ref event) = macro_def.hook {
                engine.hooks.entry(*event)
                    .or_default()
                    .push(macro_def.name.clone());
            }

            // Register keybinding if present
            if let Some(ref keys) = macro_def.keybinding {
                engine.keybindings.insert(keys.clone(), macro_def.name.clone());
            }

            // Replace existing or add new
            if let Some(idx) = existing_idx {
                engine.replace_macro(idx, macro_def);
                TfCommandResult::Success(Some("Macro redefined.".to_string()))
            } else {
                engine.add_macro(macro_def);
                TfCommandResult::Success(None)
            }
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// #undef name - Undefine a macro by exact name
fn cmd_undef(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.trim();

    if name.is_empty() {
        return TfCommandResult::Error("Usage: #undef name".to_string());
    }

    if macros::undef_macro(engine, name) {
        TfCommandResult::Success(Some(format!("Macro '{}' undefined.", name)))
    } else {
        TfCommandResult::Error(format!("Macro '{}' not found.", name))
    }
}

/// #undefn pattern - Undefine macros by name pattern
fn cmd_undefn(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: #undefn pattern".to_string());
    }

    let count = macros::undef_by_name_pattern(engine, pattern);
    TfCommandResult::Success(Some(format!("{} macro(s) undefined.", count)))
}

/// #undeft pattern - Undefine macros by trigger pattern
fn cmd_undeft(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: #undeft pattern".to_string());
    }

    let count = macros::undef_by_trigger_pattern(engine, pattern);
    TfCommandResult::Success(Some(format!("{} macro(s) undefined.", count)))
}

/// #list [pattern] - List macros
fn cmd_list(engine: &TfEngine, args: &str) -> TfCommandResult {
    let pattern = if args.trim().is_empty() {
        None
    } else {
        Some(args.trim())
    };

    TfCommandResult::Success(Some(macros::list_macros(engine, pattern)))
}

/// #purge - Remove all macros
fn cmd_purge(engine: &mut TfEngine) -> TfCommandResult {
    let count = macros::purge_macros(engine);
    TfCommandResult::Success(Some(format!("{} macro(s) purged.", count)))
}

// =============================================================================
// Hook and Keybinding Commands
// =============================================================================

/// #hook [event [command]] - Register or list hooks
fn cmd_hook(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        // List all hooks
        return TfCommandResult::Success(Some(hooks::list_hooks(engine)));
    }

    // Parse event and optional command
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
    let event_str = parts[0];

    let event = TfHookEvent::parse(event_str)
        .ok_or_else(|| format!("Unknown hook event: {}", event_str));

    match event {
        Ok(event) => {
            if parts.len() > 1 {
                // Register hook with command
                let command = parts[1].trim().to_string();
                hooks::register_hook(engine, event, command);
                TfCommandResult::Success(Some(format!("Hook registered for {:?}", event)))
            } else {
                // List hooks for this event
                let hook_list = engine.hooks.get(&event)
                    .map(|v| v.join(", "))
                    .unwrap_or_else(|| "none".to_string());
                TfCommandResult::Success(Some(format!("{:?}: {}", event, hook_list)))
            }
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// #unhook event - Remove all hooks for an event
fn cmd_unhook(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let event_str = args.trim();

    if event_str.is_empty() {
        return TfCommandResult::Error("Usage: #unhook event".to_string());
    }

    match TfHookEvent::parse(event_str) {
        Some(event) => {
            let count = hooks::unregister_hooks(engine, event);
            TfCommandResult::Success(Some(format!("{} hook(s) removed for {:?}", count, event)))
        }
        None => TfCommandResult::Error(format!("Unknown hook event: {}", event_str)),
    }
}

/// #bind [key [= command]] - Register or list keybindings
fn cmd_bind(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        // List all bindings
        return TfCommandResult::Success(Some(hooks::list_bindings(engine)));
    }

    // Parse key = command
    if let Some(eq_pos) = args.find('=') {
        let key = args[..eq_pos].trim();
        let command = args[eq_pos + 1..].trim();

        match hooks::bind_key(engine, key, command.to_string()) {
            Ok(()) => TfCommandResult::Success(None),
            Err(e) => TfCommandResult::Error(e),
        }
    } else {
        // Show binding for this key
        match hooks::get_binding(engine, args) {
            Some(cmd) => TfCommandResult::Success(Some(format!("{} = {}", args, cmd))),
            None => TfCommandResult::Success(Some(format!("{} is not bound", args))),
        }
    }
}

/// #unbind key - Remove a keybinding
fn cmd_unbind(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let key = args.trim();

    if key.is_empty() {
        return TfCommandResult::Error("Usage: #unbind key".to_string());
    }

    match hooks::unbind_key(engine, key) {
        Ok(true) => TfCommandResult::Success(Some(format!("Unbound {}", key))),
        Ok(false) => TfCommandResult::Error(format!("{} was not bound", key)),
        Err(e) => TfCommandResult::Error(e),
    }
}

/// Switch to a world (foreground)
fn cmd_fg(args: &str) -> TfCommandResult {
    let world = args.trim();
    if world.is_empty() {
        // No argument - show current world or list
        TfCommandResult::ClayCommand("/connections".to_string())
    } else {
        // Switch to specified world
        TfCommandResult::ClayCommand(format!("/worlds {}", world))
    }
}

/// List variables matching pattern
fn cmd_listvar(engine: &TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    let mut vars: Vec<(&String, &TfValue)> = engine.global_vars.iter().collect();
    vars.sort_by_key(|(k, _)| k.as_str());

    let mut output = Vec::new();
    for (name, value) in vars {
        // If pattern given, filter by glob match
        if !pattern.is_empty() {
            let matches = if pattern.contains('*') || pattern.contains('?') {
                // Glob pattern matching
                let regex_pattern = pattern
                    .replace("*", ".*")
                    .replace("?", ".");
                regex::Regex::new(&format!("^{}$", regex_pattern))
                    .map(|r| r.is_match(name))
                    .unwrap_or(false)
            } else {
                // Substring match
                name.contains(pattern)
            };
            if !matches {
                continue;
            }
        }
        output.push(format!("{} = {}", name, value.to_string_value()));
    }

    if output.is_empty() {
        if pattern.is_empty() {
            TfCommandResult::Success(Some("No variables defined".to_string()))
        } else {
            TfCommandResult::Success(Some(format!("No variables matching '{}'", pattern)))
        }
    } else {
        TfCommandResult::Success(Some(output.join("\n")))
    }
}

/// Fire a trigger manually
fn cmd_trigger(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: #trigger pattern".to_string());
    }

    // Find macros with triggers that match the pattern
    let matching_macros: Vec<_> = engine.macros.iter()
        .filter(|m| {
            if let Some(ref trigger) = m.trigger {
                trigger.pattern.contains(pattern) || pattern.contains(&trigger.pattern)
            } else {
                false
            }
        })
        .cloned()
        .collect();

    if matching_macros.is_empty() {
        return TfCommandResult::Error(format!("No triggers match '{}'", pattern));
    }

    // Execute each matching macro
    let mut results = Vec::new();
    for macro_def in matching_macros {
        // Pass None for trigger_match since this is a manual trigger invocation
        let result = macros::execute_macro(engine, &macro_def, &[], None);
        results.push(result);
    }

    // Flatten results
    let all_results: Vec<TfCommandResult> = results.into_iter().flatten().collect();
    aggregate_results(all_results)
}

/// Insert text into input buffer
fn cmd_input(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: #input text".to_string());
    }

    // Substitute variables in the text
    let text = super::variables::substitute_variables(engine, args);

    // Queue the text insertion
    engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Insert(text));
    TfCommandResult::Success(None)
}

/// Grab text from output (stub - returns empty since we don't track grab state)
fn cmd_grab(_args: &str) -> TfCommandResult {
    // In TF, #grab grabs the current line from output. We don't have that context here.
    // Return success but note that grab is limited.
    TfCommandResult::Success(Some("Note: #grab is not fully supported in this implementation".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tf_command() {
        // Only / prefix is recognized as TF command from user input
        // (# prefix is only used internally in macro bodies and scripts)
        assert!(is_tf_command("/quit"));
        assert!(is_tf_command("/set foo"));
        assert!(is_tf_command("  /echo hello"));
        assert!(!is_tf_command("#set foo bar"));  // # no longer recognized from user input
        assert!(!is_tf_command("say hello"));  // Plain text, not a command
    }

    #[test]
    fn test_split_command() {
        assert_eq!(split_command("set foo bar"), ("set", "foo bar"));
        assert_eq!(split_command("echo"), ("echo", ""));
        assert_eq!(split_command("send   -w world text"), ("send", "-w world text"));
    }

    #[test]
    fn test_is_valid_var_name() {
        assert!(is_valid_var_name("foo"));
        assert!(is_valid_var_name("_bar"));
        assert!(is_valid_var_name("foo_bar_123"));
        assert!(!is_valid_var_name("123foo"));
        assert!(!is_valid_var_name("foo-bar"));
        assert!(!is_valid_var_name(""));
    }

    #[test]
    fn test_cmd_set() {
        let mut engine = TfEngine::new();

        // Set a variable
        let result = cmd_set(&mut engine, "foo bar");
        assert!(matches!(result, TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("foo").map(|v| v.to_string_value()), Some("bar".to_string()));

        // Set numeric
        cmd_set(&mut engine, "num 42");
        assert_eq!(engine.get_var("num").and_then(|v| v.to_int()), Some(42));

        // Invalid name
        let result = cmd_set(&mut engine, "123bad value");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_unset() {
        let mut engine = TfEngine::new();
        engine.set_global("foo", TfValue::String("bar".to_string()));

        let result = cmd_unset(&mut engine, "foo");
        assert!(matches!(result, TfCommandResult::Success(None)));
        assert!(engine.get_var("foo").is_none());

        // Unset nonexistent
        let result = cmd_unset(&mut engine, "nonexistent");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_echo() {
        let engine = TfEngine::new();
        let result = cmd_echo(&engine, "Hello world");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Hello world"),
            _ => panic!("Expected success with message"),
        }
    }

    #[test]
    fn test_cmd_send() {
        let engine = TfEngine::new();

        // Simple send
        let result = cmd_send(&engine, "say hello");
        match result {
            TfCommandResult::SendToMud(text) => assert_eq!(text, "say hello"),
            _ => panic!("Expected SendToMud"),
        }

        // Send with world flag
        let result = cmd_send(&engine, "-w TestWorld say hello");
        match result {
            TfCommandResult::ClayCommand(cmd) => assert_eq!(cmd, "/send -wTestWorld say hello"),
            _ => panic!("Expected ClayCommand"),
        }
    }

    #[test]
    fn test_variable_substitution_in_command() {
        let mut engine = TfEngine::new();
        engine.set_global("target", TfValue::String("orc".to_string()));

        let result = execute_command(&mut engine, "#echo Attack the %{target}!");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Attack the orc!"),
            _ => panic!("Expected success with substituted message"),
        }
    }

    #[test]
    fn test_invoke_macro_by_name() {
        use super::super::TfMacro;

        let mut engine = TfEngine::new();

        // Define a simple macro
        engine.macros.push(TfMacro {
            name: "greet".to_string(),
            body: "#echo Hello there!".to_string(),
            ..Default::default()
        });

        // Invoke it by name
        let result = execute_command(&mut engine, "#greet");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Hello there!"),
            _ => panic!("Expected success with message, got {:?}", result),
        }
    }

    #[test]
    fn test_invoke_macro_case_insensitive() {
        use super::super::TfMacro;

        let mut engine = TfEngine::new();

        engine.macros.push(TfMacro {
            name: "MyMacro".to_string(),
            body: "#echo Works!".to_string(),
            ..Default::default()
        });

        // Should work with different cases
        let result = execute_command(&mut engine, "#mymacro");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));

        let result = execute_command(&mut engine, "#MYMACRO");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));
    }

    #[test]
    fn test_unknown_command_when_no_macro() {
        let mut engine = TfEngine::new();

        let result = execute_command(&mut engine, "#nonexistent");
        assert!(matches!(result, TfCommandResult::UnknownCommand(_)));
    }

    #[test]
    fn test_def_command_body_parsing() {
        let mut engine = TfEngine::new();

        // Define a macro using #def command
        let result = execute_command(&mut engine, "#def foo = bar");
        assert!(matches!(result, TfCommandResult::Success(_)));

        // Check the macro was defined correctly
        let macro_def = engine.macros.iter().find(|m| m.name == "foo").unwrap();
        assert_eq!(macro_def.name, "foo");
        assert_eq!(macro_def.body, "bar", "Body should be 'bar', not '= bar'");
    }

    #[test]
    fn test_def_and_invoke_macro() {
        let mut engine = TfEngine::new();

        // Define a macro that echoes
        let result = execute_command(&mut engine, "#def greet = #echo Hello World");
        assert!(matches!(result, TfCommandResult::Success(_)));

        // Verify the body doesn't include the =
        let macro_def = engine.macros.iter().find(|m| m.name == "greet").unwrap();
        assert_eq!(macro_def.body, "#echo Hello World");

        // Invoke the macro
        let result = execute_command(&mut engine, "#greet");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Hello World"),
            _ => panic!("Expected success with 'Hello World', got {:?}", result),
        }
    }

    #[test]
    fn test_macro_with_arguments() {
        let mut engine = TfEngine::new();

        // Define a macro that uses %* (all arguments)
        execute_command(&mut engine, "#def say_all = #echo You said: %*");

        // Invoke the macro with arguments
        let result = execute_command(&mut engine, "#say_all hello world");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "You said: hello world"),
            _ => panic!("Expected success with 'You said: hello world', got {:?}", result),
        }

        // Define a macro that uses positional parameters
        execute_command(&mut engine, "#def greet_person = #echo Hello %1, you are %2");

        // Invoke with arguments
        let result = execute_command(&mut engine, "#greet_person Alice great");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Hello Alice, you are great"),
            _ => panic!("Expected success, got {:?}", result),
        }
    }

    #[test]
    fn test_macro_sequence_numbers() {
        let mut engine = TfEngine::new();

        // Define several macros
        execute_command(&mut engine, "#def first = one");
        execute_command(&mut engine, "#def second = two");
        execute_command(&mut engine, "#def third = three");

        // Check sequence numbers
        let first = engine.macros.iter().find(|m| m.name == "first").unwrap();
        let second = engine.macros.iter().find(|m| m.name == "second").unwrap();
        let third = engine.macros.iter().find(|m| m.name == "third").unwrap();

        assert_eq!(first.sequence_number, 0);
        assert_eq!(second.sequence_number, 1);
        assert_eq!(third.sequence_number, 2);

        // Redefine a macro - should keep its original sequence number
        execute_command(&mut engine, "#def second = two_updated");
        let second = engine.macros.iter().find(|m| m.name == "second").unwrap();
        assert_eq!(second.sequence_number, 1, "Redefining a macro should preserve its sequence number");
        assert_eq!(second.body, "two_updated");

        // Check #list output contains sequence numbers
        let list_output = super::super::macros::list_macros(&engine, None);
        assert!(list_output.contains("0: #def"), "List should contain sequence number 0");
        assert!(list_output.contains("1: #def"), "List should contain sequence number 1");
        assert!(list_output.contains("2: #def"), "List should contain sequence number 2");
    }

    #[test]
    fn test_def_preserves_body() {
        let mut engine = TfEngine::new();

        // Define a macro with %R and other variables in the body
        // The body should be preserved literally for later substitution when executed
        execute_command(&mut engine, "#def random = #echo -- %R");
        execute_command(&mut engine, "#def test = #echo %1 %* %L %myvar");

        let random = engine.macros.iter().find(|m| m.name == "random").unwrap();
        assert_eq!(random.body, "#echo -- %R", "Body should preserve %R literally");

        let test = engine.macros.iter().find(|m| m.name == "test").unwrap();
        assert_eq!(test.body, "#echo %1 %* %L %myvar", "Body should preserve all variables");

        // When a macro is EXECUTED (not defined), variables are substituted
        // This is handled by execute_macro, not by #def parsing
    }
}
