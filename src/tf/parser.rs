//! TinyFugue command parser.
//!
//! Parses commands starting with `/` and routes them to appropriate handlers.

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
pub fn is_tf_command_name(cmd: &str) -> bool {
    matches!(cmd,
        "help" |
        "set" | "unset" | "let" | "setenv" | "listvar" |
        "echo" | "beep" | "quote" | "substitute" | "escape" | "hilite" | "nohilite" | "partial" | "export" |
        "expr" | "test" | "eval" |
        "if" | "elseif" | "else" | "endif" | "while" | "for" | "done" | "break" |
        "def" | "undef" | "undefn" | "undeft" | "list" | "purge" |
        "bind" | "unbind" | "hook" | "unhook" |
        "load" | "save" | "require" | "loaded" | "lcd" | "log" |
        "sh" | "time" | "recall" | "repeat" | "ps" | "kill" |
        "fg" | "trigger" | "input" | "grab" | "gag" | "ungag" | "exit" | "shift" | "bamf" |
        // These are also TF commands (mapped to Clay equivalents)
        "quit" | "dc" | "disconnect" | "world" | "listworlds" |
        "listsockets" | "connections" | "addworld" | "version" |
        // Note: "send" maps to Clay's /send command, but TF's /send has different options
        // so we route it through TF to handle -w flag properly
        "send" |
        // Tier 1: Simple commands
        "toggle" | "return" | "not" | "suspend" | "dokey" | "histsize" |
        "localecho" | "sub" | "replace" | "tr" | "cat" | "paste" | "endpaste" |
        // Tier 2: Trigger shortcuts
        "trig" | "trigp" | "trigc" | "trigpc" | "untrig" |
        // Tier 3: World management
        "unworld" | "purgeworld" | "saveworld" |
        // Tier 4: Spam detection
        "watchdog" | "watchname" |
        // Tier 5: Stubs
        "telnet" | "finger" | "getfile" | "putfile" | "liststreams" |
        "changes" | "tick" | "recordline" | "edit"
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

    // Handle commands starting with /
    if input.starts_with('/') {
        // Parse command name from /command format
        let cmd_part = input.split_whitespace().next().unwrap_or("");
        let cmd_name = cmd_part.trim_start_matches('/').to_lowercase();
        let args_str = if input.len() > cmd_part.len() {
            input[cmd_part.len()..].trim_start()
        } else {
            ""
        };

        // Check for /tf prefix (TF-specific commands that conflict with Clay)
        // e.g., /tfhelp, /tfgag
        if cmd_name.starts_with("tf") && (cmd_name == "tfhelp" || cmd_name == "tfgag") {
            let tf_cmd_name = &cmd_name[2..]; // Strip "tf" prefix
            return execute_tf_command(engine, tf_cmd_name, args_str, skip_substitution);
        }

        // Check if it's a TF command that should be handled here
        if is_tf_command_name(&cmd_name) {
            return execute_tf_command(engine, &cmd_name, args_str, skip_substitution);
        }

        // Check if it's a user-defined macro
        if let Some(macro_def) = engine.macros.iter().find(|m| m.name.eq_ignore_ascii_case(&cmd_name)).cloned() {
            let macro_args: Vec<&str> = parse_macro_args(args_str);
            let results = super::macros::execute_macro(engine, &macro_def, &macro_args, None);
            return aggregate_results_with_engine(engine, results);
        }

        // Not a TF command or macro - route to Clay
        return TfCommandResult::ClayCommand(input.to_string());
    }

    TfCommandResult::NotTfCommand
}

/// Execute a TF command by name with the given arguments.
/// Handles variable substitution, control flow detection, and dispatch.
fn execute_tf_command(engine: &mut TfEngine, cmd_name: &str, args: &str, skip_substitution: bool) -> TfCommandResult {
    // Reconstruct as /command for substitution and inline control flow detection
    let full_input = if args.is_empty() {
        format!("/{}", cmd_name)
    } else {
        format!("/{} {}", cmd_name, args)
    };
    let input = &full_input;

    let rest_check = args.trim();
    let lower_cmd = cmd_name.to_lowercase();

    // Check if this is an inline control flow block (multi-line /while//for//if)
    // These should not have variables substituted here - the control flow executor handles it
    let is_inline_control_flow = input.contains('\n') && matches!(lower_cmd.as_str(), "while" | "for" | "if");

    // Check if this is a /def command - if so, don't substitute variables in the body
    // The body should be stored literally and only substituted when executed
    let is_def_command = lower_cmd == "def";

    // Perform variable and command substitution before parsing (except for /def bodies, inline control flow,
    // or when called with pre-substituted input from control_flow)
    let substituted;
    let args = if skip_substitution {
        // Already substituted by caller (control_flow)
        rest_check
    } else if is_inline_control_flow {
        // Don't substitute - control flow executor will handle per-iteration substitution
        rest_check
    } else if is_def_command {
        // For /def, only substitute variables in options, not in the body
        // Find the = separator and only substitute before it
        if let Some(eq_pos) = rest_check.find('=') {
            let before_eq = &rest_check[..eq_pos];
            let after_eq = &rest_check[eq_pos..];
            let substituted_before = engine.substitute_vars(before_eq);
            let substituted_before = super::variables::substitute_commands(engine, &substituted_before);
            substituted = format!("{}{}", substituted_before, after_eq);
            substituted.trim()
        } else {
            // No body (just /def or /def with options but no =), substitute normally
            let s = engine.substitute_vars(rest_check);
            substituted = super::variables::substitute_commands(engine, &s);
            substituted.trim()
        }
    } else {
        let s = engine.substitute_vars(rest_check);
        substituted = super::variables::substitute_commands(engine, &s);
        substituted.trim()
    };

    match lower_cmd.as_str() {
        // Variable commands
        "set" => cmd_set(engine, args),
        "unset" => cmd_unset(engine, args),
        "let" => cmd_let(engine, args),
        "setenv" => cmd_setenv(engine, args),

        // Output commands
        "echo" => cmd_echo(engine, args),
        "escape" => cmd_escape(args),
        "send" => cmd_send(engine, args),
        "substitute" => cmd_substitute(engine, args),

        // Hilite/trigger shortcuts
        "hilite" => builtins::cmd_hilite(engine, args),
        "nohilite" => builtins::cmd_nohilite(engine, args),
        "partial" => builtins::cmd_partial(engine, args),

        // Variable commands
        "export" => builtins::cmd_export(engine, args),

        // Mapped to Clay commands
        "quit" => TfCommandResult::ClayCommand("/quit".to_string()),
        "exit" => builtins::cmd_exit(engine),
        "dc" | "disconnect" => TfCommandResult::ClayCommand("/disconnect".to_string()),
        "world" => cmd_world(args),
        "listworlds" => cmd_listworlds(engine, args),
        "listsockets" | "connections" => TfCommandResult::ClayCommand("/connections".to_string()),
        "addworld" => cmd_addworld(args),

        // Info commands
        "help" => cmd_help(args),
        "version" => cmd_version(),

        // Control flow commands
        "if" => cmd_if(engine, args),
        "elseif" => TfCommandResult::Error("/elseif outside of /if block".to_string()),
        "else" => TfCommandResult::Error("/else outside of /if block".to_string()),
        "endif" => TfCommandResult::Error("/endif without matching /if".to_string()),
        "while" => cmd_while(engine, args),
        "for" => cmd_for(engine, args),
        "done" => TfCommandResult::Error("/done without matching /while or /for".to_string()),
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
        "beep" => builtins::cmd_beep(engine, args),
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

        // Portal/bamf
        "bamf" => cmd_bamf(engine, args),

        // Argument manipulation
        "shift" => cmd_shift(engine),

        // Variable management
        "listvar" => cmd_listvar(engine, args),

        // Trigger commands
        "trigger" => cmd_trigger(engine, args),

        // Input manipulation
        "input" => cmd_input(engine, args),
        "grab" => cmd_grab(args),

        // Tier 1: Simple commands
        "toggle" => builtins::cmd_toggle(engine, args),
        "return" => builtins::cmd_return(engine, args),
        "not" => builtins::cmd_not(engine, args),
        "suspend" => builtins::cmd_suspend(),
        "dokey" => builtins::cmd_dokey(engine, args),
        "histsize" => builtins::cmd_histsize(engine, args),
        "localecho" => builtins::cmd_localecho(engine, args),
        "sub" => builtins::cmd_sub(engine, args),
        "replace" => builtins::cmd_replace(engine, args),
        "tr" => builtins::cmd_tr(engine, args),
        "cat" => TfCommandResult::Success(Some("% /cat not supported in Clay. Use bracketed paste instead.".to_string())),
        "paste" => TfCommandResult::Success(Some("% /paste not supported in Clay. Use bracketed paste instead.".to_string())),
        "endpaste" => TfCommandResult::Success(None),

        // Tier 2: Trigger shortcuts
        "trig" => builtins::cmd_trig(engine, args),
        "trigp" => builtins::cmd_trigp(engine, args),
        "trigc" => builtins::cmd_trigc(engine, args),
        "trigpc" => builtins::cmd_trigpc(engine, args),
        "untrig" => builtins::cmd_untrig(engine, args),

        // Tier 3: World management
        "unworld" => builtins::cmd_unworld(args),
        "purgeworld" => TfCommandResult::Success(Some("% /purgeworld: Use /worlds to manage worlds in Clay.".to_string())),
        "saveworld" => TfCommandResult::Success(Some("% /saveworld: Worlds are auto-saved in Clay.".to_string())),

        // Tier 4: Spam detection
        "watchdog" => builtins::cmd_watchdog(engine, args),
        "watchname" => builtins::cmd_watchname(engine, args),

        // Tier 5: Stubs
        "telnet" => TfCommandResult::Success(Some("% /telnet: Use /worlds to connect in Clay.".to_string())),
        "finger" => TfCommandResult::Success(Some("% /finger: Command not available in Clay.".to_string())),
        "getfile" | "putfile" => TfCommandResult::Success(Some("% File transfer not available in Clay.".to_string())),
        "liststreams" => TfCommandResult::Success(Some("% /liststreams: Streams not available in Clay.".to_string())),
        "changes" => TfCommandResult::Success(Some("% /changes: Not applicable in Clay. See /version.".to_string())),
        "tick" => TfCommandResult::Success(Some("% /tick: Use /repeat for timed commands in Clay.".to_string())),
        "recordline" => TfCommandResult::Success(Some("% /recordline: Not available in Clay.".to_string())),
        "edit" => cmd_edit(engine, args),

        // Check for user-defined macro with this name
        _ => {
            // Look for a macro with this name (case-insensitive)
            if let Some(macro_def) = engine.macros.iter().find(|m| m.name.eq_ignore_ascii_case(&lower_cmd)).cloned() {
                // Parse arguments for the macro with delimiter-aware splitting
                let macro_args: Vec<&str> = parse_macro_args(args);
                let results = macros::execute_macro(engine, &macro_def, &macro_args, None);
                aggregate_results_with_engine(engine, results)
            } else {
                TfCommandResult::UnknownCommand(lower_cmd.to_string())
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
            TfCommandResult::Return(_) => {} // Handled in execute_macro
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


// =============================================================================
// Command Implementations
// =============================================================================

/// /set varname=value - Set a global variable
/// Supports both /set var=value and /set var = value
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

/// /unset varname - Remove a global variable
fn cmd_unset(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.trim();

    if name.is_empty() {
        return TfCommandResult::Error("Usage: /unset varname".to_string());
    }

    if engine.unset_global(name) {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Error(format!("Variable '{}' not found", name))
    }
}

/// /let varname=value - Set a local variable
/// Supports both /let var=value and /let var = value
fn cmd_let(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: /let varname=value".to_string());
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

/// /setenv varname value - Set an environment variable (exported to shell)
fn cmd_setenv(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();

    if parts.is_empty() || parts[0].is_empty() {
        return TfCommandResult::Error("Usage: /setenv varname value".to_string());
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

/// /echo [-w world] [-a attrs] [--] message - Display message locally
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

/// /escape metacharacters string - Escape metacharacters and backslashes in string
/// Echoes string with any metacharacters or '\' preceded by '\'.
fn cmd_escape(args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /escape metacharacters string".to_string());
    }
    // First word is the set of metacharacters, rest is the string
    let (metacharacters, string) = if let Some(space_pos) = args.find(char::is_whitespace) {
        let meta = &args[..space_pos];
        let rest = args[space_pos..].trim_start();
        (meta, rest)
    } else {
        // Only metacharacters provided, no string — result is empty
        return TfCommandResult::Success(Some(String::new()));
    };

    let result = tf_escape(metacharacters, string);
    TfCommandResult::Success(Some(result))
}

/// Core escape logic shared by /escape command and escape() function.
/// Precedes any character in `string` that is in `metacharacters` or is '\' with a '\'.
pub fn tf_escape(metacharacters: &str, string: &str) -> String {
    let mut result = String::with_capacity(string.len() * 2);
    for c in string.chars() {
        if c == '\\' || metacharacters.contains(c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

/// /substitute [-a attrs] [--] text - Replace trigger line with substituted text
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

/// /send [-w world] text - Send text to MUD
fn cmd_send(_engine: &TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: /send [-w world] text".to_string());
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
            return TfCommandResult::Error("Usage: /send -w world text".to_string());
        }
    }

    // Simple send
    TfCommandResult::SendToMud(args.to_string())
}

/// /world [name] - Switch to or connect to a world
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

/// /addworld - Define a new world or redefine an existing world
///
/// Command usage:
///   /addworld [-xe] [-Ttype] name [char pass] host port
///   /addworld [-Ttype] name
///
/// Options:
///   -x  Use SSL/TLS for connections
///   -e  Echo sent text back (ignored in Clay)
///   -Ttype  World type (ignored in Clay, defaults to MUD)
///
/// Examples:
///   /addworld MyMUD mud.example.com 4000
///   /addworld -x SecureMUD secure.example.com 4443
///   /addworld MyMUD player password mud.example.com 4000
/// /shift - Shift positional parameters left (%2→%1, %3→%2, etc.)
fn cmd_shift(engine: &mut TfEngine) -> TfCommandResult {
    let argc = engine.get_var("#")
        .and_then(|v| v.to_int())
        .unwrap_or(0) as usize;

    if argc == 0 {
        return TfCommandResult::Success(None);
    }

    // Shift: 2→1, 3→2, etc.
    for i in 1..argc {
        let next_val = engine.get_var(&(i + 1).to_string()).cloned()
            .unwrap_or(super::TfValue::String(String::new()));
        engine.set_local(&i.to_string(), next_val);
    }

    // Remove the last one
    engine.set_local(&argc.to_string(), super::TfValue::String(String::new()));

    // Decrement count
    engine.set_local("#", super::TfValue::Integer((argc - 1) as i64));

    // Rebuild %* from remaining args
    let mut parts = Vec::new();
    for i in 1..argc {
        if let Some(v) = engine.get_var(&i.to_string()) {
            let s = v.to_string_value();
            if !s.is_empty() {
                parts.push(s);
            }
        }
    }
    engine.set_local("*", super::TfValue::String(parts.join(" ")));

    TfCommandResult::Success(None)
}

/// /listworlds [-cs] [-Sfield] [name] - List world definitions (TF style)
fn cmd_listworlds(engine: &TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    let mut short = false;
    let mut cmd_format = false;
    let mut sort_field = "name";
    let mut name_pattern: Option<String> = None;

    // Parse options
    let mut i = 0;
    let parts: Vec<&str> = args.split_whitespace().collect();
    while i < parts.len() {
        let part = parts[i];
        if let Some(flags) = part.strip_prefix('-') {
            if flags.starts_with('S') {
                sort_field = match flags.chars().nth(1) {
                    Some('n') => "name",
                    Some('h') => "host",
                    Some('p') => "port",
                    Some('c') => "character",
                    Some('-') => "-",
                    _ => "name",
                };
            } else {
                for c in flags.chars() {
                    match c {
                        's' => short = true,
                        'c' => cmd_format = true,
                        'u' | 'm' | 'T' => {} // ignored
                        _ => {}
                    }
                }
            }
        } else {
            name_pattern = Some(part.to_string());
        }
        i += 1;
    }

    let mut worlds: Vec<&super::WorldInfoCache> = engine.world_info_cache.iter().collect();

    // Filter by name pattern
    if let Some(ref pattern) = name_pattern {
        let pat = pattern.to_lowercase();
        worlds.retain(|w| w.name.to_lowercase().contains(&pat));
    }

    // Sort
    match sort_field {
        "host" => worlds.sort_by(|a, b| a.host.cmp(&b.host)),
        "port" => worlds.sort_by(|a, b| a.port.cmp(&b.port)),
        "character" => worlds.sort_by(|a, b| a.user.cmp(&b.user)),
        "-" => {} // no sort
        _ => worlds.sort_by(|a, b| a.name.cmp(&b.name)),
    }

    if worlds.is_empty() {
        return TfCommandResult::Success(Some("No worlds defined.".to_string()));
    }

    if short {
        // Short format: names only
        let names: Vec<&str> = worlds.iter().map(|w| w.name.as_str()).collect();
        return TfCommandResult::Success(Some(names.join("\n")));
    }

    if cmd_format {
        // Command format: /test addworld("name", "type", "host", "port", "char", "pass")
        let mut lines = Vec::new();
        for w in &worlds {
            lines.push(format!("/test addworld(\"{}\", \"\", \"{}\", \"{}\", \"{}\", \"{}\")",
                w.name, w.host, w.port, w.user, w.password));
        }
        return TfCommandResult::Success(Some(lines.join("\n")));
    }

    // Table format matching TF: NAME  TYPE  HOST PORT  CHARACTER
    // TYPE is always empty (Clay doesn't have world types), right-aligned HOST
    let mut lines = Vec::new();
    let name_w = worlds.iter().map(|w| w.name.len()).max().unwrap_or(4).max(4).max(15);
    let host_w = worlds.iter().map(|w| w.host.len()).max().unwrap_or(4).max(4);
    let port_w = 5;

    lines.push(format!("{:<name_w$} {:<16}{:>host_w$} {:<port_w$}  {}",
        "NAME", "TYPE", "HOST", "PORT", "CHARACTER",
        name_w=name_w, host_w=host_w, port_w=port_w));

    for w in &worlds {
        lines.push(format!("{:<name_w$} {:<16}{:>host_w$} {:<port_w$}  {}",
            w.name, "", w.host, w.port, w.user,
            name_w=name_w, host_w=host_w, port_w=port_w));
    }

    TfCommandResult::Success(Some(lines.join("\n")))
}

fn cmd_addworld(args: &str) -> TfCommandResult {
    // Pass through to Clay's /addworld command which handles the actual creation
    if args.trim().is_empty() {
        return TfCommandResult::Error("Usage: /addworld [-xe] [-Ttype] name [char pass] host port".to_string());
    }
    TfCommandResult::ClayCommand(format!("/addworld {}", args))
}

/// /help [topic] or /tfhelp [topic] - Display TF help
fn cmd_help(args: &str) -> TfCommandResult {
    let topic = args.trim().trim_start_matches('/').to_lowercase();

    if topic.is_empty() {
        let help_text = r#"Getting Started:
  /setup               - Open settings (server, colors, etc.)
  /world               - Setup connection(s) to a world(s)
  /world <name>        - Connect/switch to a world
  /dc                  - Disconnect from current world
  /connections         - Show all connected worlds
  /quit                - Exit Clay

Keys:
  PgUp / PgDn          - Scroll through output history
  Ctrl-Up or Down      - Switch Worlds
  Tab                  - Release world output when paused.

Basic Configuration:
  /setup               - General settings popup
  /web                 - Web interface / remote access settings
  /actions             - Manage triggers and actions
  /keybinds            - Open keybinding editor (browser)

For more help:
  /help commands       - List of commands
  /help functions      - List of functions
  /help <command>      - Help on a specific command (e.g. /help def)
  /help keys           - All keyboard bindings
  /help web            - Websocket help (remote interfaces)"#;
        TfCommandResult::Success(Some(help_text.to_string()))
    } else {
        match topic.as_str() {
            "set" => TfCommandResult::Success(Some(
                "/set [name [value]]\n\nSet a global variable. Without arguments, lists all variables.\nExamples:\n  /set foo bar    - Set foo to \"bar\"\n  /set count 42   - Set count to 42\n  /set            - List all variables".to_string()
            )),
            "echo" => TfCommandResult::Success(Some(
                "/echo message\n\nDisplay a message locally (not sent to MUD).\nVariable substitution is performed on the message.\nExample: /echo Hello %{name}!".to_string()
            )),
            "escape" => TfCommandResult::Success(Some(
                "/escape metacharacters string\n\nEchoes string with any metacharacters or '\\' characters\npreceded by a '\\' character.\n\nFunction form: $[escape(metacharacters, string)]\n\nExample:\n  /def blue = /def -aCblue -t\"$(/escape \" %*)\"\n  /blue * pages, \"*\"\n  => /def -aCblue -t\"* pages, \\\"*\\\"\"".to_string()
            )),
            "hilite" => TfCommandResult::Success(Some(
                "/hilite [pattern [= response]]\n\nWith no args: enables hilite (sets %{hilite} to 1).\nWith args: creates a trigger that hilites matching lines.\nEquivalent to: /def -ah -t\"pattern\" [= response]\n\nHilite style is set by %{hiliteattr} (default: B = bold).\nExample: /hilite {*} tried to kill you!".to_string()
            )),
            "nohilite" => TfCommandResult::Success(Some(
                "/nohilite [pattern]\n\nWith no args: disables hilite (sets %{hilite} to 0).\nWith a pattern: removes hilite macros matching that pattern.".to_string()
            )),
            "partial" => TfCommandResult::Success(Some(
                "/partial regexp\n\nHilites the matched portion of lines (not the whole line).\nCreates a fall-through trigger so multiple can match.\nEquivalent to: /def -Ph -F -tregexp\n\nHilite style is set by %{hiliteattr} (default: B = bold).\nExample: /partial [Hh]awkeye".to_string()
            )),
            "export" => TfCommandResult::Success(Some(
                "/export variable\n\nMakes a global variable an environment variable,\navailable to /sh and /quote commands.\nLocal variables may not be exported.\n\nSee also: /setenv".to_string()
            )),
            "send" => TfCommandResult::Success(Some(
                "/send [-w world] text\n\nSend text to the MUD server.\n-w world: Send to specific world\nExample: /send say Hello everyone!".to_string()
            )),
            "def" => TfCommandResult::Success(Some(
                r#"/def [options] name = body
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
                "/if (expression) command\n/if (expr) ... /elseif (expr) ... /else ... /endif\n\nConditional execution.\nExamples:\n  /if (hp < 50) cast heal\n  /if (%1 == \"yes\") /echo Confirmed /else /echo Cancelled /endif".to_string()
            )),
            "while" => TfCommandResult::Success(Some(
                "/while (expression) ... /done\n\nRepeat commands while expression is true.\nExample:\n  /while (count < 10) /echo %count%; /set count $[count+1] /done".to_string()
            )),
            "for" => TfCommandResult::Success(Some(
                "/for variable start end [step] ... /done\n\nLoop from start to end.\nExample:\n  /for i 1 5 /echo Number %i /done".to_string()
            )),
            "expr" => TfCommandResult::Success(Some(
                "/expr expression\n\nEvaluate expression and display result.\nOperators: + - * / % == != < > <= >= & | ! =~ !~ ?:\nFunctions: strlen() substr() strcat() tolower() toupper() rand() time() abs() min() max()\nExample: /expr 2 + 2 * 3".to_string()
            )),
            "test" => TfCommandResult::Success(Some(
                r#"/test expression

Evaluate expression and return its value, setting %?.

Evaluates the expression and returns its value (any type).
Also sets the special variable %? to the result.
Useful for evaluating expressions for side effects.

Examples:
  /test 2 + 2           - Returns 4, sets %? to 4
  /test strlen("hello") - Returns 5, sets %? to 5
  /test hp < 50         - Returns 1 or 0, sets %?

Unlike /expr, /test does not display the result automatically.
The result is stored in %? for later use."#.to_string()
            )),
            "bind" => TfCommandResult::Success(Some(
                "/bind key = command\n\nBind a key to execute a command.\nKey names: F1-F12, ^A-^Z (Ctrl), @a-@z (Alt), PgUp, PgDn, Home, End, Insert, Delete\nExample: /bind F5 = cast heal".to_string()
            )),
            "hook" | "hooks" => TfCommandResult::Success(Some(
                "Hooks fire macros on events. Use /def -h<event> to register.\n\nEvents:\n  CONNECT     - When connected to MUD\n  DISCONNECT  - When disconnected\n  LOGIN       - After login\n  PROMPT      - On prompt received\n  SEND        - Before sending command\n\nExample: /def -hCONNECT auto_look = look".to_string()
            )),
            "repeat" => TfCommandResult::Success(Some(
                r#"/repeat [-w[world]] {[-time]|-S|-P} count command

Repeat a command on a timer. First iteration runs immediately,
then waits the interval before each subsequent iteration.

Options:
  -w[world]  Send to specific world (empty = current)
  -S         Synchronous (execute all iterations now)
  -P         Execute on prompt (not yet implemented)
  -time      Interval: seconds, M:S, or H:M:S

Count: integer or "i" for infinite

Examples:
  /repeat -30 5 /echo hi        - Now, then every 30s, 5 times total
  /repeat -0:30 i /echo hi      - Now, then every 30s, infinite
  /repeat -1:0:0 1 /echo hourly - Once now (1 hour interval unused)
  /repeat -S 3 /echo sync       - 3 times immediately"#.to_string()
            )),
            "ps" => TfCommandResult::Success(Some(
                "/ps\n\nList all background processes (from /repeat).\nShows PID, interval, remaining count, and command.".to_string()
            )),
            "kill" => TfCommandResult::Success(Some(
                "/kill pid\n\nKill a background process by its PID.\nUse /ps to see running processes.".to_string()
            )),
            "load" => TfCommandResult::Success(Some(
                r#"/load [-q] filename

Load and execute commands from a TF script file.

Options:
  -q  Quiet mode - don't echo "% Loading commands from..." message

The file may contain:
  - TF commands starting with / (e.g., /def, /set)
  - Comments: lines starting with ; or single # followed by space
  - Blank lines (ignored)

Line continuation: End a line with \ to continue on next line.
Use %\ for a literal backslash at end of line.

File search order (for relative paths):
  1. Current directory (from /lcd or actual cwd)
  2. Directories in $TFPATH (colon-separated)
  3. $TFLIBDIR

Use /exit to abort loading early.

Example:
  /load ~/.tf/init.tf
  /load -q mylib.tf"#.to_string()
            )),
            "require" => TfCommandResult::Success(Some(
                r#"/require [-q] filename

Load a file only if not already loaded via /loaded.

Same as /load, but if the file has already registered a token
via /loaded, the file will not be read again.

Files designed for /require should have /loaded as their first
command with a unique token (usually the file's full path).

Example file (mylib.tf):
  /loaded mylib.tf
  /def myfunc = /echo Hello from mylib

Usage:
  /require mylib.tf   - Loads the file
  /require mylib.tf   - Does nothing (already loaded)"#.to_string()
            )),
            "loaded" => TfCommandResult::Success(Some(
                r#"/loaded token

Mark a file as loaded (for use with /require).

Should be the first command in a file designed for /require.
If the token has already been registered by a previous /loaded
call, the current file load is aborted (returns success).

Token should be unique - the file's full path is recommended.

Example (in mylib.tf):
  /loaded mylib.tf
  ; Rest of file only executed once"#.to_string()
            )),
            "exit" => TfCommandResult::Success(Some(
                r#"/exit

Abort loading the current file early.

When called during /load or /require, stops reading the
current file immediately. Commands already executed are
not undone.

When called outside of file loading, /exit is equivalent
to /quit (exits Clay)."#.to_string()
            )),
            "bamf" => TfCommandResult::Success(Some(
                r#"/bamf [off|on|old]

Controls portal handling. A portal is text from a MUD server
of the form:
  #### Please reconnect to Name@addr (host) port NNN ####

If bamf is OFF (default), portal lines have no effect.

If bamf is ON, Clay will disconnect from the current world
and connect to the new world specified in the portal.

If bamf is OLD, Clay will connect to the new world without
disconnecting from the current one.

If %{login} is also set to 1, Clay will auto-login to the
new world using the current world's username and password.

Warning: On many servers, other users can spoof portal text
to redirect your client. Enable with caution.

Examples:
  /bamf on           Enable portals (disconnect + reconnect)
  /bamf old          Enable portals (keep old connection)
  /bamf off          Disable portals
  /set login=1       Enable auto-login on portal"#.to_string()
            )),
            "addworld" => TfCommandResult::Success(Some(
                r#"/addworld [-xe] [-Ttype] name [char pass] host port

Define a new world or update an existing world.

Command form:
  /addworld MyMUD mud.example.com 4000
  /addworld -x SecureMUD secure.example.com 4443
  /addworld MyMUD player password mud.example.com 4000

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
  /addworld Cave cave.tcp.com 2283
  /addworld -x Secure secure.tcp.com 4443
  /test addworld("Cave", "", "cave.tcp.com", "2283")
  /test addworld("Secure", "", "ssl.tcp.com", "4443", "", "", "", "x")"#.to_string()
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
  escape(meta, str)        - Escape metacharacters in string
  replace(str, old, new [,count]) - Replace occurrences
  tr(domain, range, str)   - Translate characters
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
  nactive(world)           - Unseen lines in named world
  nworlds()                - Total world count
  is_connected([world])    - Check if world is connected
  idle([world])            - Seconds since last receive
  sidle([world])           - Seconds since last send

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
  send(text [,world [,f]])  - Send text to MUD (f=0/"off": no EOL)
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
  tfflush(handle [,auto])  - Flush file buffer (auto: on/off)
  tfeof(handle)            - Check for end of file
  fwrite(file, text)       - Append text to file (simple)

Macro/Builtin Call Syntax:
  macroname(arg1, arg2)    - Call macro with positional params (%1, %2)
  command(args)            - Call builtin as function (e.g. def("-t..."))

Usage: $[function(args)] or in /expr//test"#.to_string()
            )),
            "commands" | "cmds" => TfCommandResult::Success(Some(
                r#"Commands (/help <command> for details)
Clay:
  /setup  /web  /actions  /keybinds  /menu  /connections
  /world  /connect  /disconnect  /dc  /addworld  /unworld
  /reload  /version  /quit  /remote  /ban  /unban
  /flush  /dump  /note  /tag  /notify
Variables:
  /set  /unset  /let  /setenv  /listvar  /toggle  /export
Expressions & Control Flow:
  /expr  /test  /eval  /not  /return
  /if  /elseif  /else  /endif  /while  /for  /done  /break
Macros & Triggers:
  /def  /undef  /undefn  /undeft  /list  /purge
  /trig  /trigp  /trigc  /trigpc  /untrig
  /bind  /unbind  /dokey
Output:
  /echo  /send  /beep  /quote  /recall  /substitute
  /hilite  /nohilite  /partial  /gag  /ungag
  /escape  /replace  /tr
World:
  /fg  /addworld  /dc  /watchdog  /watchname  /bamf
Files & Scripts:
  /load  /require  /loaded  /save  /lcd  /exit  /log  /sh
Process:
  /repeat  /ps  /kill
Settings:
  /histsize  /localecho  /sub  /suspend  /time  /shift
  /input  /grab  /trigger"#.to_string()
            )),
            "keys" | "keybindings" => TfCommandResult::Success(Some(
                r#"Keyboard Shortcuts

World Switching:
  Ctrl+Up/Down         - Switch between active worlds
  Shift+Up/Down        - Switch between all worlds
  Esc+W                - Switch to world with activity

Input Editing:
  Left/Right, Ctrl+B/F - Move cursor
  Up/Down              - Move cursor up/down lines
  Alt+Up/Down          - Resize input area
  Ctrl+A / Home        - Jump to start of line
  Ctrl+E / End         - Jump to end of line
  Ctrl+K               - Delete to end of line
  Ctrl+U               - Clear input
  Ctrl+W               - Delete word before cursor
  Ctrl+D               - Delete character under cursor
  Ctrl+T               - Transpose characters
  Ctrl+Y               - Yank (paste killed text)
  Esc+B / Esc+F        - Word left / word right
  Esc+D                - Delete word forward
  Esc+C                - Capitalize word forward
  Esc+L                - Lowercase word forward
  Esc+U                - Uppercase word forward
  Esc+Space            - Collapse spaces to one
  Esc+-                - Goto matching bracket
  Esc+. or Esc+_       - Insert last arg from history
  Ctrl+P/N             - Command history
  Esc+P                - Search history backward
  Esc+N                - Search history forward
  Ctrl+Q               - Spell suggestions
  Tab                  - Command completion / release pending output

Output:
  PageUp/PageDown      - Scroll output
  Esc+J (lowercase)    - Jump to end, release all
  Esc+J (uppercase)    - Selective flush (keep hilite)
  Esc+H                - Half-page scroll/release

Display:
  F1                   - Show help
  F2                   - Toggle MUD tag display
  F4                   - Filter output
  F8                   - Highlight action matches

System:
  Ctrl+C (twice)       - Quit
  Ctrl+L               - Redraw screen
  Ctrl+R               - Hot reload
  Ctrl+Z               - Suspend

Customize with /keybinds or /bind:
  /bind F5 = cast heal
  /bind ^S = /save macros.tf"#.to_string()
            )),
            "toggle" => TfCommandResult::Success(Some(
                "/toggle varname\n\nToggle a variable between 0 and 1.\nIf current value is 0, sets to 1; otherwise sets to 0.\n\nExample: /toggle gag".to_string()
            )),
            "return" => TfCommandResult::Success(Some(
                "/return [expression]\n\nStop executing the current macro and return.\nIf an expression is given, it is evaluated and %? is set to the result.\nWithout an argument, %? is set to 1.\n\nExample:\n  /def check = /if (hp > 50) /return 1%; /echo Low HP!".to_string()
            )),
            "not" => TfCommandResult::Success(Some(
                "/not expression\n\nNegate the result of an expression.\nSets %? to 1 if expression is false, 0 if true.\n\nExample: /not (connected)".to_string()
            )),
            "suspend" => TfCommandResult::Success(Some(
                "/suspend\n\nSuspend the process (equivalent to Ctrl+Z).".to_string()
            )),
            "dokey" => TfCommandResult::Success(Some(
                "/dokey keyname\n\nSimulate pressing a named edit key.\n\nKey names:\n  BSPC/BACKSPACE  - Backspace\n  DCH/DELETE      - Delete character\n  DLINE           - Delete entire line\n  LEFT/RIGHT      - Move cursor\n  HOME/END        - Start/end of line\n  UP/RECALLB      - Previous history\n  DOWN/RECALLF    - Next history\n  WLEFT/WRIGHT    - Word left/right\n  NEWLINE/ENTER   - Submit input\n  HPAGE/PAGEUP    - Page up\n  PAGE/PAGEDN     - Page down\n  REFRESH         - Redraw screen".to_string()
            )),
            "histsize" => TfCommandResult::Success(Some(
                "/histsize [-i] [size]\n\nGet or set the history buffer size.\n-i: Input history (default)\n\nExample: /histsize 500".to_string()
            )),
            "localecho" => TfCommandResult::Success(Some(
                "/localecho [on|off]\n\nGet or set local echo mode.\nWhen on, typed commands are displayed locally.".to_string()
            )),
            "sub" => TfCommandResult::Success(Some(
                "/sub [off|on|full]\n\nGet or set the substitution mode.\n  off  - No variable substitution\n  on   - Normal substitution (default)\n  full - Full substitution".to_string()
            )),
            "replace" => TfCommandResult::Success(Some(
                "/replace old new string\n\nReplace all occurrences of 'old' with 'new' in string.\nEchoes the result.\n\nFunction form: $[replace(str, old, new [,count])]\n\nExample: /replace foo bar \"foo and foo\"  => \"bar and bar\"".to_string()
            )),
            "tr" => TfCommandResult::Success(Some(
                "/tr domain range string\n\nTranslate characters: each character in 'domain' is replaced\nby the corresponding character in 'range'.\n\nFunction form: $[tr(domain, range, string)]\n\nExample: /tr abc ABC \"a big cat\"  => \"A Big CAt\"".to_string()
            )),
            "trig" => TfCommandResult::Success(Some(
                "/trig pattern = body\n\nCreate an unnamed trigger (glob mode).\nEquivalent to: /def -t\"pattern\" = body\n\nSee also: /trigp, /trigc, /trigpc, /untrig".to_string()
            )),
            "trigp" => TfCommandResult::Success(Some(
                "/trigp priority pattern = body\n\nCreate a trigger with specified priority.\nEquivalent to: /def -p<pri> -t\"pattern\" = body".to_string()
            )),
            "trigc" => TfCommandResult::Success(Some(
                "/trigc chance pattern = body\n\nCreate a trigger with specified probability (0.0-1.0).\nEquivalent to: /def -c<chance> -t\"pattern\" = body".to_string()
            )),
            "trigpc" => TfCommandResult::Success(Some(
                "/trigpc priority chance pattern = body\n\nCreate a trigger with both priority and probability.\nEquivalent to: /def -p<pri> -c<chance> -t\"pattern\" = body".to_string()
            )),
            "untrig" => TfCommandResult::Success(Some(
                "/untrig [-a attrs] pattern\n\nRemove triggers matching the given pattern.\n\nExample: /untrig * says *".to_string()
            )),
            "unworld" => TfCommandResult::Success(Some(
                "/unworld name\n\nRemove a world definition. Maps to /close in Clay.".to_string()
            )),
            "watchdog" => TfCommandResult::Success(Some(
                "/watchdog [off|on|n1 [n2]]\n\nSuppress duplicate lines from the MUD.\nIf a line has appeared n1 times in the last n2 lines, it is gagged.\n\nDefaults: n1=2 (threshold), n2=5 (window size)\n\nExamples:\n  /watchdog on       - Enable with defaults\n  /watchdog 3 10     - Gag after 3 repeats in last 10 lines\n  /watchdog off      - Disable\n  /watchdog          - Show current settings".to_string()
            )),
            "watchname" => TfCommandResult::Success(Some(
                "/watchname [off|on|n1 [n2]]\n\nSuppress spam from repeated character names.\nIf the first word of a line has appeared as the first word\nof n1 of the last n2 lines, the line is gagged.\n\nDefaults: n1=4 (threshold), n2=5 (window size)\n\nExamples:\n  /watchname on      - Enable with defaults\n  /watchname 3 8     - Gag after name appears 3 times in last 8 lines\n  /watchname off     - Disable".to_string()
            )),
            "unset" => TfCommandResult::Success(Some(
                "/unset name\n\nRemove a global variable.\n\nExample: /unset foo".to_string()
            )),
            "let" => TfCommandResult::Success(Some(
                "/let name=value\n\nSet a local variable in the current scope.\nLocal variables shadow globals and are removed when\nthe macro finishes executing.\n\nExamples:\n  /let x=hello\n  /let count=0".to_string()
            )),
            "setenv" => TfCommandResult::Success(Some(
                "/setenv name [value]\n\nSet or export a variable to the shell environment.\nIf value is given, sets the variable first.\nThe variable becomes available to /sh and child processes.\n\nExample: /setenv TERM vt100".to_string()
            )),
            "listvar" => TfCommandResult::Success(Some(
                "/listvar [pattern]\n\nList global variables matching a pattern.\nWithout a pattern, lists all variables.\nGlob patterns (* and ?) are supported.\n\nExamples:\n  /listvar        - List all variables\n  /listvar foo*   - List variables starting with foo".to_string()
            )),
            "eval" => TfCommandResult::Success(Some(
                "/eval expression\n\nEvaluate expression and execute the result as a command.\nIf the result starts with /, it is executed as a Clay command.\nOtherwise it is sent to the MUD.\n\nExample:\n  /set cmd=/echo hello\n  /eval cmd   - Executes /echo hello".to_string()
            )),
            "beep" => TfCommandResult::Success(Some(
                "/beep [count]\n\nSound the terminal bell.\nOptional count (default 1) specifies how many beeps.\n\nExample: /beep 3".to_string()
            )),
            "quote" => TfCommandResult::Success(Some(
                r#"/quote [-w world] [-S|-dC] [-0] [text | 'command | `command | !command]

Send text or command output without variable substitution.

Prefixes:
  'command    Echo output locally
  `command    Execute each line of output as TF command
  !command    Run shell command, send output to MUD

Options:
  -w world   Send to specific world
  -S         Use semicolons as newlines
  -dC        Use character C as delimiter
  -0         No newlines (NUL delimiter)

Examples:
  /quote Say %this literally    - Sends without expanding %this
  /quote !/bin/date             - Sends date output to MUD
  /quote '!cat file.txt         - Echoes file contents locally
  /quote `!echo /set x=1        - Executes /set x=1"#.to_string()
            )),
            "recall" => TfCommandResult::Success(Some(
                r#"/recall [options] [range] [pattern]

Search output history.

Options:
  -w[world]   Search specific world (default: current)
  -l          Search local (TF) output only
  -g          Search all worlds + local
  -i          Search input history
  -t[format]  Show timestamps
  -v          Invert match (show non-matching)
  -q          Quiet (set %? but don't display)
  -mtype      Match type: simple, glob (default), regexp
  -ag         Include gagged lines
  -An         Show n lines after each match
  -Bn         Show n lines before each match
  #           Show line numbers

Range: N (last N), -N (Nth previous), N-M, N-

Examples:
  /recall 20                    - Last 20 lines
  /recall -i /def               - Input history matching /def
  /recall -mregexp \d{3}-\d{4}  - Regex match"#.to_string()
            )),
            "gag" => TfCommandResult::Success(Some(
                "/gag [pattern]\n\nWith no args: list all gag triggers.\nWith a pattern: create a trigger that suppresses matching lines.\nEquivalent to: /def -ag -t\"pattern\"\n\nExample: /gag * has left the game.".to_string()
            )),
            "ungag" => TfCommandResult::Success(Some(
                "/ungag pattern\n\nRemove gag triggers matching the given pattern.\n\nExample: /ungag * has left the game.".to_string()
            )),
            "fg" => TfCommandResult::Success(Some(
                "/fg [world]\n\nSwitch to a world or list worlds.\nWithout arguments, equivalent to /connections.\n\nExample: /fg MyMUD".to_string()
            )),
            "dc" | "disconnect" => TfCommandResult::Success(Some(
                "/dc or /disconnect\n\nDisconnect from the current world.".to_string()
            )),
            "world" => TfCommandResult::Success(Some(
                "/world [name]\n\nSwitch to a world by name.\nWithout arguments, lists available worlds.\n\nExample: /world MyMUD".to_string()
            )),
            "listworlds" => TfCommandResult::Success(Some(
                "/listworlds\n\nList all defined worlds with their connection status.".to_string()
            )),
            "listsockets" | "connections" => TfCommandResult::Success(Some(
                "/listsockets or /connections\n\nList all connected worlds. Maps to Clay's /connections command.".to_string()
            )),
            "undef" => TfCommandResult::Success(Some(
                "/undef name\n\nRemove a macro by name.\n\nExample: /undef my_trigger".to_string()
            )),
            "undefn" => TfCommandResult::Success(Some(
                "/undefn number\n\nRemove a macro by its sequence number.\nUse /list to see sequence numbers.\n\nExample: /undefn 42".to_string()
            )),
            "undeft" => TfCommandResult::Success(Some(
                "/undeft pattern\n\nRemove all macros whose trigger matches the given pattern.\n\nExample: /undeft * tells you *".to_string()
            )),
            "list" => TfCommandResult::Success(Some(
                "/list [pattern]\n\nList macro definitions matching a pattern.\nWithout a pattern, lists all macros.\nGlob patterns (* and ?) are supported.\n\nExample: /list auto_*".to_string()
            )),
            "purge" => TfCommandResult::Success(Some(
                "/purge\n\nRemove all macro definitions.".to_string()
            )),
            "unbind" => TfCommandResult::Success(Some(
                "/unbind key\n\nRemove a key binding.\nKey names: F1-F12, ^A-^Z (Ctrl), @a-@z (Alt)\n\nExample: /unbind F5".to_string()
            )),
            "unhook" => TfCommandResult::Success(Some(
                "/unhook macro_name\n\nRemove a hook by macro name.\nThe macro is not deleted, only its hook association.\n\nExample: /unhook auto_look".to_string()
            )),
            "save" => TfCommandResult::Success(Some(
                "/save filename\n\nSave all current macro definitions to a file.\nThe file can be loaded later with /load.\n\nExample: /save ~/.tf/macros.tf".to_string()
            )),
            "lcd" => TfCommandResult::Success(Some(
                "/lcd path\n\nChange the local working directory.\nAffects /sh, /load, and file operations.\n\nExample: /lcd ~/tf-scripts".to_string()
            )),
            "log" => TfCommandResult::Success(Some(
                "/log [filename | off]\n\nStart or stop logging output to a file.\n/log filename - Start logging to file\n/log off      - Stop logging\n/log           - Show logging status\n\nExample: /log ~/mud.log".to_string()
            )),
            "sh" => TfCommandResult::Success(Some(
                "/sh command\n\nExecute a shell command and display the output.\nEnvironment variables set with /setenv or /export are available.\n\nExample: /sh ls -la".to_string()
            )),
            "time" => TfCommandResult::Success(Some(
                "/time [command]\n\nWithout arguments, display the current date and time.\nWith a command, execute it and display how long it took.\n\nExamples:\n  /time\n  /time /load big_script.tf".to_string()
            )),
            "version" => TfCommandResult::Success(Some(
                "/version\n\nDisplay Clay version information.".to_string()
            )),
            "quit" => TfCommandResult::Success(Some(
                "/quit\n\nExit Clay.".to_string()
            )),
            "shift" => TfCommandResult::Success(Some(
                "/shift\n\nShift positional parameters: %2 becomes %1, %3 becomes %2, etc.\nUsed inside macros to iterate through arguments.\n\nExample:\n  /def showargs = /while ({1} !~ \"\") /echo %1%; /shift%; /done".to_string()
            )),
            "trigger" => TfCommandResult::Success(Some(
                "/trigger [-d] pattern\n\nTest trigger matching against a pattern.\n-d: Delete matching triggers\nWithout -d: show which macros would fire for this text.\n\nExample: /trigger You are hungry".to_string()
            )),
            "input" => TfCommandResult::Success(Some(
                "/input text\n\nInsert text into the input buffer at the cursor position.\n\nFunction form: $[input(text)]\n\nExample: /input say hello".to_string()
            )),
            "grab" => TfCommandResult::Success(Some(
                "/grab text\n\nReplace the input buffer with the given text.\nIf no text given, grab the last line of output.\n\nExample: /grab say ".to_string()
            )),
            "substitute" => TfCommandResult::Success(Some(
                "/substitute text\n\nReplace the current trigger line with different text.\nOnly works inside a trigger body.\n\nFunction form: $[substitute(text [,attrs])]\n\nExample:\n  /def -t\"* says *\" colorize = /substitute [%1] %2".to_string()
            )),
            "cat" | "paste" => TfCommandResult::Success(Some(
                "/cat and /paste are not supported in Clay.\nUse bracketed paste instead (paste normally into the input area).".to_string()
            )),
            "help" | "tfhelp" => TfCommandResult::Success(Some(
                "/help [topic] or /tfhelp [topic]\n\nShow help on TF commands and features.\n\nTopics: set, echo, send, def, if, while, for, expr, test,\n  bind, hooks, repeat, load, recall, quote, gag, addworld,\n  watchdog, watchname, functions, and more.\n\nExample: /help def".to_string()
            )),
            "purgeworld" | "saveworld" => TfCommandResult::Success(Some(
                "/purgeworld and /saveworld are stubs.\nUse Clay's world management instead.".to_string()
            )),
            "telnet" | "finger" => TfCommandResult::Success(Some(
                "/telnet and /finger are not implemented.\nUse /sh to run system commands instead.\n\nExample: /sh telnet host port".to_string()
            )),
            "getfile" | "putfile" => TfCommandResult::Success(Some(
                "/getfile and /putfile are not implemented.\nFile transfer protocols are not supported.".to_string()
            )),
            "liststreams" => TfCommandResult::Success(Some(
                "/liststreams\n\nList active streams. Not implemented in Clay.".to_string()
            )),
            "changes" => TfCommandResult::Success(Some(
                "/changes\n\nShow TF changelog. Not implemented in Clay.\nUse /version for version info.".to_string()
            )),
            "tick" => TfCommandResult::Success(Some(
                "/tick\n\nTick timer (for MUD combat rounds). Not implemented in Clay.\nUse /repeat for periodic timers instead.".to_string()
            )),
            "recordline" => TfCommandResult::Success(Some(
                "/recordline\n\nRecord a line to history. Not implemented in Clay.".to_string()
            )),
            "edit" => TfCommandResult::Success(Some(
                r#"/edit [options] name [= body]

Edit an existing macro. The macro is found by name, or:
  #num       Find by sequence number (from /list)
  $pattern   Find by trigger pattern

Options are the same as /def. Only specified options are
changed; unspecified options remain from the original.

Body is only changed if "=" is present. Use "=" alone
to clear the body.

Examples:
  /edit -c0 greet           - Set probability to 0%
  /edit -p5 greet           - Change priority to 5
  /edit #42 = new body      - Edit macro #42's body
  /edit $"* says *" -ag     - Add gag to trigger

See also: /def, /list, /undef"#.to_string()
            )),
            _ => TfCommandResult::Success(Some(format!("No help available for '{}'\nUse /help for a list of all commands.", topic))),
        }
    }
}

/// /version - Show version info
fn cmd_version() -> TfCommandResult {
    TfCommandResult::Success(Some(crate::get_version_string()))
}

/// /expr expression - Evaluate expression and display result
fn cmd_expr(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /expr expression".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => TfCommandResult::Success(Some(value.to_string_value())),
        Err(e) => TfCommandResult::Error(format!("Expression error: {}", e)),
    }
}

/// /eval expression - Evaluate expression and execute result as command
fn cmd_eval(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /eval expression".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => {
            let cmd = value.to_string_value();
            if cmd.is_empty() {
                TfCommandResult::Success(None)
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

/// /test expression - Evaluate expression and return its value, setting %?
///
/// Evaluates the expression and returns its value (any type).
/// Also sets the special variable %? to the result.
/// Useful for evaluating expressions for side effects.
///
/// Examples:
///   /test 2 + 2           -> returns 4, sets %? to 4
///   /test strlen("hello") -> returns 5, sets %? to 5
///   /test regmatch("foo(.*)", "foobar") -> sets %P1 to "bar"
fn cmd_test(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /test expression".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => {
            // Set the special %? variable to the result
            engine.set_global("?", value.clone());
            // /test is silent - it only sets %?, doesn't produce output
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

/// /if (condition) [command] - Conditional execution
fn cmd_if(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Check if this is a complete inline block (from macro execution)
    // These contain newlines and the full /if.../endif structure
    let if_args_lower = args.to_lowercase();
    if args.contains('\n') && if_args_lower.contains("/endif") {
        // Reconstruct the full block by prepending "/if "
        let full_block = format!("/if {}", args);
        let results = control_flow::execute_inline_if_block(engine, &full_block);
        return aggregate_inline_results(engine, results);
    }

    // Check for single-line form: /if (condition) command
    if let Some((condition, command)) = control_flow::parse_single_line_if(args) {
        return control_flow::execute_single_if(engine, &condition, &command);
    }

    // Multi-line form: /if (condition)
    match control_flow::parse_condition(args) {
        Ok(condition) => {
            engine.control_state = ControlState::If(IfState::new(condition));
            TfCommandResult::Success(None)
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// Aggregate results from inline control flow execution
fn aggregate_inline_results(engine: &mut super::TfEngine, results: Vec<TfCommandResult>) -> TfCommandResult {
    // Use the engine-aware version which properly handles SendToMud
    // by queueing commands in engine.pending_commands.
    // Inline control flow (while/for/if) can produce SendToMud results that
    // must not be silently dropped.
    aggregate_results_with_engine(engine, results)
}

/// /while (condition) - Start a while loop
fn cmd_while(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Check if this is a complete inline block (from macro execution)
    // These contain newlines and the full /while.../done structure
    let args_lower = args.to_lowercase();
    if args.contains('\n') && args_lower.contains("/done") {
        // Reconstruct the full block by prepending "/while "
        let full_block = format!("/while {}", args);
        let results = control_flow::execute_inline_while_block(engine, &full_block);
        return aggregate_inline_results(engine, results);
    }

    match control_flow::parse_condition(args) {
        Ok(condition) => {
            engine.control_state = ControlState::While(WhileState::new(condition));
            TfCommandResult::Success(None)
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// /for var start end [step] - Start a for loop
fn cmd_for(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Check if this is a complete inline block (from macro execution)
    let for_args_lower = args.to_lowercase();
    if args.contains('\n') && for_args_lower.contains("/done") {
        let full_block = format!("/for {}", args);
        let results = control_flow::execute_inline_for_block(engine, &full_block);
        return aggregate_inline_results(engine, results);
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

/// /def [options] name = body - Define a macro
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

/// /edit [options] name [= body] - Edit an existing macro
fn cmd_edit(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args_trimmed = args.trim();
    if args_trimmed.is_empty() {
        return TfCommandResult::Error("Usage: /edit [options] name [= body]".to_string());
    }

    // Parse the edit args the same way as /def
    let edit_def = match macros::parse_def(args_trimmed) {
        Ok(d) => d,
        Err(e) => return TfCommandResult::Error(e),
    };

    // Find the existing macro by name, #num, or $pattern
    let name = &edit_def.name;
    let existing_idx = if let Some(num_str) = name.strip_prefix('#') {
        // #num — find by sequence number
        if let Ok(num) = num_str.parse::<u32>() {
            engine.macros.iter().position(|m| m.sequence_number == num)
        } else {
            None
        }
    } else if let Some(pattern) = name.strip_prefix('$') {
        // $pattern — find by trigger pattern
        engine.macros.iter().position(|m| {
            m.trigger.as_ref().map(|t| t.pattern == pattern).unwrap_or(false)
        })
    } else {
        engine.macros.iter().position(|m| m.name.eq_ignore_ascii_case(name))
    };

    let idx = match existing_idx {
        Some(i) => i,
        None => return TfCommandResult::Error(format!("Macro '{}' not found.", name)),
    };

    // Clone the existing macro and apply edits
    let mut edited = engine.macros[idx].clone();

    // Apply options from the edit command (only if explicitly given)
    // Trigger
    if edit_def.trigger.is_some() {
        edited.trigger = edit_def.trigger;
    }
    // Hook
    if edit_def.hook.is_some() {
        edited.hook = edit_def.hook;
    }
    // Keybinding
    if edit_def.keybinding.is_some() {
        edited.keybinding = edit_def.keybinding;
    }
    // Priority (non-default)
    if edit_def.priority != 0 {
        edited.priority = edit_def.priority;
    }
    // Fall-through
    if edit_def.fall_through {
        edited.fall_through = true;
    }
    // One-shot
    if edit_def.one_shot.is_some() {
        edited.one_shot = edit_def.one_shot;
        edited.shots_remaining = edit_def.one_shot;
    }
    // Attributes (apply if any are set)
    if edit_def.attributes.gag || edit_def.attributes.bold || edit_def.attributes.underline ||
       edit_def.attributes.reverse || edit_def.attributes.flash || edit_def.attributes.dim ||
       edit_def.attributes.bell || edit_def.attributes.norecord || edit_def.attributes.hilite.is_some() {
        edited.attributes = edit_def.attributes;
    }
    // Condition
    if edit_def.condition.is_some() {
        edited.condition = edit_def.condition;
    }
    // Probability
    if edit_def.probability.is_some() {
        edited.probability = edit_def.probability;
    }
    // World
    if edit_def.world.is_some() {
        edited.world = edit_def.world;
    }
    // Partial hilite
    if edit_def.partial_hilite {
        edited.partial_hilite = true;
    }

    // Body: only update if '=' was present in the args
    if args_trimmed.contains(" = ") || args_trimmed.ends_with(" =") {
        edited.body = edit_def.body;
    }

    // Replace the macro (preserves sequence number)
    engine.replace_macro(idx, edited);

    TfCommandResult::Success(Some(format!("Macro '{}' edited.", engine.macros[idx].name)))
}

/// /undef name - Undefine a macro by exact name
fn cmd_undef(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.trim();

    if name.is_empty() {
        return TfCommandResult::Error("Usage: /undef name".to_string());
    }

    if macros::undef_macro(engine, name) {
        TfCommandResult::Success(Some(format!("Macro '{}' undefined.", name)))
    } else {
        TfCommandResult::Error(format!("Macro '{}' not found.", name))
    }
}

/// /undefn pattern - Undefine macros by name pattern
fn cmd_undefn(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: /undefn pattern".to_string());
    }

    let count = macros::undef_by_name_pattern(engine, pattern);
    TfCommandResult::Success(Some(format!("{} macro(s) undefined.", count)))
}

/// /undeft pattern - Undefine macros by trigger pattern
fn cmd_undeft(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: /undeft pattern".to_string());
    }

    let count = macros::undef_by_trigger_pattern(engine, pattern);
    TfCommandResult::Success(Some(format!("{} macro(s) undefined.", count)))
}

/// /list [pattern] - List macros
fn cmd_list(engine: &TfEngine, args: &str) -> TfCommandResult {
    let pattern = if args.trim().is_empty() {
        None
    } else {
        Some(args.trim())
    };

    TfCommandResult::Success(Some(macros::list_macros(engine, pattern)))
}

/// /purge - Remove all macros
fn cmd_purge(engine: &mut TfEngine) -> TfCommandResult {
    let count = macros::purge_macros(engine);
    TfCommandResult::Success(Some(format!("{} macro(s) purged.", count)))
}

// =============================================================================
// Hook and Keybinding Commands
// =============================================================================

/// /hook [event [command]] - Register or list hooks
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

/// /unhook event - Remove all hooks for an event
fn cmd_unhook(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let event_str = args.trim();

    if event_str.is_empty() {
        return TfCommandResult::Error("Usage: /unhook event".to_string());
    }

    match TfHookEvent::parse(event_str) {
        Some(event) => {
            let count = hooks::unregister_hooks(engine, event);
            TfCommandResult::Success(Some(format!("{} hook(s) removed for {:?}", count, event)))
        }
        None => TfCommandResult::Error(format!("Unknown hook event: {}", event_str)),
    }
}

/// /bind [key [= command]] - Register or list keybindings
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

/// /unbind key - Remove a keybinding
fn cmd_unbind(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let key = args.trim();

    if key.is_empty() {
        return TfCommandResult::Error("Usage: /unbind key".to_string());
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

/// Set the bamf flag for portal handling
fn cmd_bamf(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim().to_lowercase();
    let value = match arg.as_str() {
        "on" | "1" => "1",
        "old" => "old",
        "off" | "0" | "" => "0",
        _ => {
            return TfCommandResult::Error(format!("Usage: /bamf [off|on|old] (got '{}')", args.trim()));
        }
    };
    engine.set_global("bamf", TfValue::from(value));
    let state = match value {
        "1" => "on (disconnect + reconnect)",
        "old" => "old (reconnect without disconnect)",
        _ => "off",
    };
    TfCommandResult::Success(Some(format!("bamf {}", state)))
}

/// List variables matching pattern
fn cmd_listvar(engine: &TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    let mut vars: Vec<(&String, &TfValue)> = engine.global_vars.iter().collect();
    vars.sort_by_key(|(k, _)| k.as_str());

    // Pre-compile glob regex once before the loop
    let compiled_glob = if !pattern.is_empty() && (pattern.contains('*') || pattern.contains('?')) {
        let regex_pattern = pattern.replace("*", ".*").replace("?", ".");
        regex::Regex::new(&format!("^{}$", regex_pattern)).ok()
    } else {
        None
    };

    let mut output = Vec::new();
    for (name, value) in vars {
        // If pattern given, filter by glob match
        if !pattern.is_empty() {
            let matches = if let Some(ref re) = compiled_glob {
                re.is_match(name)
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
        return TfCommandResult::Error("Usage: /trigger pattern".to_string());
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
        return TfCommandResult::Error("Usage: /input text".to_string());
    }

    // Substitute variables in the text
    let text = super::variables::substitute_variables(engine, args);

    // Queue the text insertion
    engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Insert(text));
    TfCommandResult::Success(None)
}

/// Grab text from output (stub - returns empty since we don't track grab state)
fn cmd_grab(_args: &str) -> TfCommandResult {
    // In TF, /grab grabs the current line from output. We don't have that context here.
    // Return success but note that grab is limited.
    TfCommandResult::Success(Some("Note: /grab is not fully supported in this implementation".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tf_command() {
        // Only / prefix is recognized as TF command
        assert!(is_tf_command("/quit"));
        assert!(is_tf_command("/set foo"));
        assert!(is_tf_command("  /echo hello"));
        assert!(!is_tf_command("#set foo bar"));  // # is not a command prefix
        assert!(!is_tf_command("say hello"));  // Plain text, not a command
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

        let result = execute_command(&mut engine, "/echo Attack the %{target}!");
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
            body: "/echo Hello there!".to_string(),
            ..Default::default()
        });

        // Invoke it by name
        let result = execute_command(&mut engine, "/greet");
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
            body: "/echo Works!".to_string(),
            ..Default::default()
        });

        // Should work with different cases
        let result = execute_command(&mut engine, "/mymacro");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));

        let result = execute_command(&mut engine, "/MYMACRO");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));
    }

    #[test]
    fn test_unknown_command_when_no_macro() {
        let mut engine = TfEngine::new();

        // /nonexistent is not a TF command or macro, so it goes to Clay
        let result = execute_command(&mut engine, "/nonexistent");
        assert!(matches!(result, TfCommandResult::ClayCommand(_)));
    }

    #[test]
    fn test_def_command_body_parsing() {
        let mut engine = TfEngine::new();

        // Define a macro using /def command
        let result = execute_command(&mut engine, "/def foo = bar");
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
        let result = execute_command(&mut engine, "/def greet = /echo Hello World");
        assert!(matches!(result, TfCommandResult::Success(_)));

        // Verify the body doesn't include the =
        let macro_def = engine.macros.iter().find(|m| m.name == "greet").unwrap();
        assert_eq!(macro_def.body, "/echo Hello World");

        // Invoke the macro
        let result = execute_command(&mut engine, "/greet");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Hello World"),
            _ => panic!("Expected success with 'Hello World', got {:?}", result),
        }
    }

    #[test]
    fn test_macro_with_arguments() {
        let mut engine = TfEngine::new();

        // Define a macro that uses %* (all arguments)
        execute_command(&mut engine, "/def say_all = /echo You said: %*");

        // Invoke the macro with arguments
        let result = execute_command(&mut engine, "/say_all hello world");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "You said: hello world"),
            _ => panic!("Expected success with 'You said: hello world', got {:?}", result),
        }

        // Define a macro that uses positional parameters
        execute_command(&mut engine, "/def greet_person = /echo Hello %1, you are %2");

        // Invoke with arguments
        let result = execute_command(&mut engine, "/greet_person Alice great");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Hello Alice, you are great"),
            _ => panic!("Expected success, got {:?}", result),
        }
    }

    #[test]
    fn test_macro_sequence_numbers() {
        let mut engine = TfEngine::new();

        // Define several macros
        execute_command(&mut engine, "/def first = one");
        execute_command(&mut engine, "/def second = two");
        execute_command(&mut engine, "/def third = three");

        // Check sequence numbers
        let first = engine.macros.iter().find(|m| m.name == "first").unwrap();
        let second = engine.macros.iter().find(|m| m.name == "second").unwrap();
        let third = engine.macros.iter().find(|m| m.name == "third").unwrap();

        assert_eq!(first.sequence_number, 0);
        assert_eq!(second.sequence_number, 1);
        assert_eq!(third.sequence_number, 2);

        // Redefine a macro - should keep its original sequence number
        execute_command(&mut engine, "/def second = two_updated");
        let second = engine.macros.iter().find(|m| m.name == "second").unwrap();
        assert_eq!(second.sequence_number, 1, "Redefining a macro should preserve its sequence number");
        assert_eq!(second.body, "two_updated");

        // Check /list output contains sequence numbers
        let list_output = super::super::macros::list_macros(&engine, None);
        assert!(list_output.contains("0: /def"), "List should contain sequence number 0");
        assert!(list_output.contains("1: /def"), "List should contain sequence number 1");
        assert!(list_output.contains("2: /def"), "List should contain sequence number 2");
    }

    #[test]
    fn test_def_preserves_body() {
        let mut engine = TfEngine::new();

        // Define a macro with %R and other variables in the body
        // The body should be preserved literally for later substitution when executed
        execute_command(&mut engine, "/def random = /echo -- %R");
        execute_command(&mut engine, "/def test = /echo %1 %* %L %myvar");

        let random = engine.macros.iter().find(|m| m.name == "random").unwrap();
        assert_eq!(random.body, "/echo -- %R", "Body should preserve %R literally");

        let test = engine.macros.iter().find(|m| m.name == "test").unwrap();
        assert_eq!(test.body, "/echo %1 %* %L %myvar", "Body should preserve all variables");

        // When a macro is EXECUTED (not defined), variables are substituted
        // This is handled by execute_macro, not by /def parsing
    }

    #[test]
    fn test_macro_while_loop_count() {
        // /def count = /let i=1%; /while (i <= {1}) /echo num: %{i}%; /let i=$[i + 1]%; /done
        // /count 10 → "num: 1" through "num: 10"
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def count =  /let i=1%;  /while (i <= {1})  /echo num: %{i}%;  /let i=$[i + 1]%; /done");

        let result = execute_command(&mut engine, "/count 10");
        match result {
            TfCommandResult::Success(Some(msg)) => {
                let lines: Vec<&str> = msg.lines().collect();
                assert_eq!(lines.len(), 10, "Expected 10 lines, got {}: {:?}", lines.len(), lines);
                for i in 1..=10 {
                    assert_eq!(lines[i - 1], format!("num: {}", i),
                        "Line {} should be 'num: {}', got '{}'", i, i, lines[i - 1]);
                }
            }
            other => panic!("Expected success with num output, got {:?}", other),
        }

        // Also test with plain text (SendToMud) via pending_commands
        execute_command(&mut engine, "/def count2 =  /let i=1%;  /while (i <= {1})  think num: %{i}%;  /let i=$[i + 1]%; /done");
        engine.pending_commands.clear();
        execute_command(&mut engine, "/count2 10");
        let cmds: Vec<String> = engine.pending_commands.iter().map(|c| c.command.clone()).collect();
        assert_eq!(cmds.len(), 10, "Expected 10 pending commands, got {:?}", cmds);
        for i in 1..=10 {
            assert_eq!(cmds[i - 1], format!("think num: {}", i));
        }
    }

    #[test]
    fn test_macro_while_shift() {
        // /def w = /while ({#}) /echo # %1%; /shift%; /done
        // /w global 8bit → "# global" then "# 8bit"
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def w = /while ({#}) /echo # %1%; /shift%; /done");

        let result = execute_command(&mut engine, "/w global 8bit");
        match result {
            TfCommandResult::Success(Some(msg)) => {
                let lines: Vec<&str> = msg.lines().collect();
                assert_eq!(lines.len(), 2, "Expected 2 lines, got {}: {:?}", lines.len(), lines);
                assert_eq!(lines[0], "# global");
                assert_eq!(lines[1], "# 8bit");
            }
            other => panic!("Expected success with world output, got {:?}", other),
        }
    }
}
