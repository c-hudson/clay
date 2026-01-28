//! TinyFugue command parser.
//!
//! Parses commands starting with `#` and routes them to appropriate handlers.

use super::{TfCommandResult, TfEngine, TfValue, TfHookEvent};
use super::control_flow::{self, ControlState, ControlResult, IfState, WhileState, ForState};
use super::macros;
use super::hooks;
use super::builtins;

/// Check if input is a TF command (starts with #)
pub fn is_tf_command(input: &str) -> bool {
    input.trim_start().starts_with('#')
}

/// Execute a TF command and return the result.
pub fn execute_command(engine: &mut TfEngine, input: &str) -> TfCommandResult {
    let input = input.trim();

    // Check for internal encoded commands (from control flow)
    if input.starts_with("__tf_if_eval__:") {
        let results = control_flow::execute_if_encoded(engine, input);
        return aggregate_results(results);
    }
    if input.starts_with("__tf_while_eval__:") {
        let results = control_flow::execute_while_encoded(engine, input);
        return aggregate_results(results);
    }
    if input.starts_with("__tf_for_eval__:") {
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

    if !input.starts_with('#') {
        return TfCommandResult::NotTfCommand;
    }

    // Perform variable substitution before parsing
    let substituted = engine.substitute_vars(input);
    let input = substituted.trim();

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

        // Mapped to Clay commands
        "quit" | "exit" => TfCommandResult::ClayCommand("/quit".to_string()),
        "dc" | "disconnect" => TfCommandResult::ClayCommand("/disconnect".to_string()),
        "world" => cmd_world(args),
        "listworlds" => TfCommandResult::ClayCommand("/worlds".to_string()),
        "listsockets" | "connections" => TfCommandResult::ClayCommand("/connections".to_string()),
        "connect" => cmd_connect(args),

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
        "quote" => builtins::cmd_quote(args),
        "recall" => builtins::cmd_recall(args),
        "gag" => builtins::cmd_gag(engine, args),
        "ungag" => builtins::cmd_ungag(engine, args),
        "load" => builtins::cmd_load(engine, args),
        "save" => builtins::cmd_save(engine, args),
        "log" => builtins::cmd_log(args),
        "repeat" => builtins::cmd_repeat(engine, args),
        "ps" => builtins::cmd_ps(engine),
        "kill" => builtins::cmd_kill(engine, args),

        // Check for user-defined macro with this name
        _ => {
            // Look for a macro with this name (case-insensitive)
            if let Some(macro_def) = engine.macros.iter().find(|m| m.name.eq_ignore_ascii_case(cmd)).cloned() {
                // Parse arguments for the macro
                let macro_args: Vec<&str> = if args.is_empty() {
                    vec![]
                } else {
                    args.split_whitespace().collect()
                };
                let results = macros::execute_macro(engine, &macro_def, &macro_args, None);
                aggregate_results(results)
            } else {
                TfCommandResult::UnknownCommand(cmd.to_string())
            }
        }
    }
}

/// Aggregate multiple results into one
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

/// #set varname value - Set a global variable
fn cmd_set(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();

    if parts.is_empty() || parts[0].is_empty() {
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

    let name = parts[0];

    // Validate variable name
    if !is_valid_var_name(name) {
        return TfCommandResult::Error(format!(
            "Invalid variable name '{}': must start with letter and contain only letters, numbers, underscores",
            name
        ));
    }

    let value = if parts.len() > 1 {
        TfValue::from(parts[1])
    } else {
        TfValue::String(String::new())
    };

    engine.set_global(name, value);
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

/// #let varname value - Set a local variable
fn cmd_let(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();

    if parts.is_empty() || parts[0].is_empty() {
        return TfCommandResult::Error("Usage: #let varname value".to_string());
    }

    let name = parts[0];

    if !is_valid_var_name(name) {
        return TfCommandResult::Error(format!(
            "Invalid variable name '{}': must start with letter and contain only letters, numbers, underscores",
            name
        ));
    }

    let value = if parts.len() > 1 {
        TfValue::from(parts[1])
    } else {
        TfValue::String(String::new())
    };

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

/// #echo message - Display message locally
fn cmd_echo(engine: &TfEngine, args: &str) -> TfCommandResult {
    // Variable substitution already done, just return the message
    let _ = engine;  // Engine already used for substitution
    let message = args.to_string();
    TfCommandResult::Success(Some(message))
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

/// #help [topic] - Display help
fn cmd_help(args: &str) -> TfCommandResult {
    let topic = args.trim().trim_start_matches('#').to_lowercase();

    if topic.is_empty() {
        let help_text = r#"TinyFugue Commands

Variables:
  #set [name [value]]  - Set/list global variables
  #unset name          - Remove a variable
  #let name value      - Set a local variable
  #setenv name         - Export variable to environment
  #listvar [pattern]   - List variables

Expressions:
  #expr expression     - Evaluate and display result
  #test expression     - Evaluate as boolean (0 or 1)
  #eval expression     - Evaluate and execute as command

Control Flow:
  #if (expr) cmd       - Conditional execution
  #if/#elseif/#else/#endif - Multi-line conditional
  #while (expr)/#done  - While loop
  #for var s e [step]/#done - For loop
  #break               - Exit loop

Macros/Triggers:
  #def [opts] name=body - Define macro (-t -m -p -F -1 -ag -h -b)
  #undef name          - Remove macro
  #list [pattern]      - List macros
  #purge [pattern]     - Remove all macros

Hooks & Keys:
  #bind key=command    - Bind key to command
  #unbind key          - Remove key binding

Output:
  #echo message        - Display message locally
  #send [-w world] text - Send text to MUD
  #beep                - Terminal bell
  #quote text          - Send without substitution
  #gag pattern         - Suppress matching lines
  #ungag pattern       - Remove gag
  #recall [pattern]    - Search output history

World Management:
  #world [name]        - Switch to or list worlds
  #connect [world]     - Connect to a world
  #listworlds          - List all worlds
  #listsockets         - List connected worlds
  #dc, #disconnect     - Disconnect current world

File Operations:
  #load filename       - Load TF script
  #save filename       - Save macros to file
  #lcd path            - Change local directory

Process:
  #repeat [opts] count cmd - Repeat command on timer
  #ps                  - List background processes
  #kill id             - Kill background process

Misc:
  #time                - Display current time
  #sh command          - Execute shell command
  #help [topic]        - Show this help
  #version             - Show version info
  #quit                - Exit Clay

Variable Substitution:
  %{varname}           - Variable value
  %1-%9, %*            - Positional params from trigger
  %L, %R               - Left/right of match
  %%                   - Literal percent sign

Use #help <command> for detailed help on specific commands."#;
        TfCommandResult::Success(Some(help_text.to_string()))
    } else {
        match topic.as_str() {
            "set" => TfCommandResult::Success(Some(
                "#set [name [value]]\n\nSet a global variable. Without arguments, lists all variables.\nExamples:\n  #set foo bar    - Set foo to \"bar\"\n  #set count 42   - Set count to 42\n  #set            - List all variables".to_string()
            )),
            "echo" => TfCommandResult::Success(Some(
                "#echo message\n\nDisplay a message locally (not sent to MUD).\nVariable substitution is performed on the message.\nExample: #echo Hello %{name}!".to_string()
            )),
            "send" => TfCommandResult::Success(Some(
                "#send [-w world] text\n\nSend text to the MUD server.\n-w world: Send to specific world\nExample: #send say Hello everyone!".to_string()
            )),
            "def" => TfCommandResult::Success(Some(
                r#"#def [options] name = body

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
  #def -t"You are hungry" eat = get food bag%; eat food
  #def -t"^(\w+) tells you" -mregexp reply = tell %1 Got it!
  #def -hCONNECT greet = look"#.to_string()
            )),
            "if" => TfCommandResult::Success(Some(
                "#if (expression) command\n#if (expr) ... #elseif (expr) ... #else ... #endif\n\nConditional execution.\nExamples:\n  #if (hp < 50) cast heal\n  #if (%1 == \"yes\") #echo Confirmed #else #echo Cancelled #endif".to_string()
            )),
            "while" => TfCommandResult::Success(Some(
                "#while (expression) ... #done\n\nRepeat commands while expression is true.\nExample:\n  #while (count < 10) #echo %count%; #set count $[count+1] #done".to_string()
            )),
            "for" => TfCommandResult::Success(Some(
                "#for variable start end [step] ... #done\n\nLoop from start to end.\nExample:\n  #for i 1 5 #echo Number %i #done".to_string()
            )),
            "expr" => TfCommandResult::Success(Some(
                "#expr expression\n\nEvaluate expression and display result.\nOperators: + - * / % == != < > <= >= & | ! =~ !~ ?:\nFunctions: strlen() substr() strcat() tolower() toupper() rand() time() abs() min() max()\nExample: #expr 2 + 2 * 3".to_string()
            )),
            "bind" => TfCommandResult::Success(Some(
                "#bind key = command\n\nBind a key to execute a command.\nKey names: F1-F12, ^A-^Z (Ctrl), @a-@z (Alt), PgUp, PgDn, Home, End, Insert, Delete\nExample: #bind F5 = cast heal".to_string()
            )),
            "hook" | "hooks" => TfCommandResult::Success(Some(
                "Hooks fire macros on events. Use #def -h<event> to register.\n\nEvents:\n  CONNECT     - When connected to MUD\n  DISCONNECT  - When disconnected\n  LOGIN       - After login\n  PROMPT      - On prompt received\n  SEND        - Before sending command\n\nExample: #def -hCONNECT auto_look = look".to_string()
            )),
            "repeat" => TfCommandResult::Success(Some(
                r#"#repeat [-w[world]] [-n] {[-time]|-S|-P} count command

Repeat a command on a timer.

Options:
  -w[world]  Send to specific world (empty = current)
  -n         No initial delay (run immediately first time)
  -S         Synchronous (execute all iterations now)
  -P         Execute on prompt (not yet implemented)
  -time      Interval: seconds, M:S, or H:M:S

Count: integer or "i" for infinite

Examples:
  #repeat -30 5 #echo hi        - Every 30s, 5 times
  #repeat -0:30 i #echo hi      - Every 30s, infinite
  #repeat -1:0:0 1 #echo hourly - After 1 hour, once
  #repeat -n -5 10 #echo fast   - Every 5s, 10 times, no delay
  #repeat -S 3 #echo sync       - 3 times immediately"#.to_string()
            )),
            "ps" => TfCommandResult::Success(Some(
                "#ps\n\nList all background processes (from #repeat).\nShows PID, interval, remaining count, and command.".to_string()
            )),
            "kill" => TfCommandResult::Success(Some(
                "#kill pid\n\nKill a background process by its PID.\nUse #ps to see running processes.".to_string()
            )),
            _ => TfCommandResult::Success(Some(format!("No help available for '{}'\nTry: set, echo, send, def, if, while, for, expr, bind, hooks, repeat, ps, kill", topic))),
        }
    }
}

/// #version - Show version info
fn cmd_version() -> TfCommandResult {
    TfCommandResult::Success(Some(
        "Clay MUD Client with TinyFugue compatibility\nTF compatibility layer: Complete".to_string()
    ))
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

/// #test expression - Evaluate expression as boolean, return 0 or 1
fn cmd_test(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: #test expression".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => {
            let result = if value.to_bool() { "1" } else { "0" };
            TfCommandResult::Success(Some(result.to_string()))
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

/// #while (condition) - Start a while loop
fn cmd_while(engine: &mut TfEngine, args: &str) -> TfCommandResult {
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
                engine.macros[idx] = macro_def;
                TfCommandResult::Success(Some("Macro redefined.".to_string()))
            } else {
                engine.macros.push(macro_def);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tf_command() {
        assert!(is_tf_command("#set foo bar"));
        assert!(is_tf_command("  #echo hello"));
        assert!(!is_tf_command("/quit"));
        assert!(!is_tf_command("say hello"));
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
}
