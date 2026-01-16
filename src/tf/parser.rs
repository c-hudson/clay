//! TinyFugue command parser.
//!
//! Parses commands starting with `#` and routes them to appropriate handlers.

use super::{TfCommandResult, TfEngine, TfValue};

/// Check if input is a TF command (starts with #)
pub fn is_tf_command(input: &str) -> bool {
    input.trim_start().starts_with('#')
}

/// Execute a TF command and return the result.
pub fn execute_command(engine: &mut TfEngine, input: &str) -> TfCommandResult {
    let input = input.trim();

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

        // Not yet implemented - placeholder
        "def" | "undef" | "undefn" | "undeft" | "list" | "purge" => {
            TfCommandResult::Error(format!("#{} not yet implemented (Phase 4)", cmd))
        }
        "if" | "elseif" | "else" | "endif" | "while" | "done" | "for" | "break" => {
            TfCommandResult::Error(format!("#{} not yet implemented (Phase 3)", cmd))
        }
        // Expression commands
        "expr" => cmd_expr(engine, args),
        "eval" => cmd_eval(engine, args),
        "test" => cmd_test(engine, args),
        "hook" | "unhook" | "bind" | "unbind" => {
            TfCommandResult::Error(format!("#{} not yet implemented (Phase 5)", cmd))
        }

        _ => TfCommandResult::UnknownCommand(cmd.to_string()),
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
    let topic = args.trim().to_lowercase();

    if topic.is_empty() {
        let help_text = r#"TinyFugue Commands (Phase 1)

Variables:
  #set [name [value]]  - Set/list global variables
  #unset name          - Remove a variable
  #let name value      - Set a local variable
  #setenv name value   - Set an environment variable

Output:
  #echo message        - Display message locally
  #send [-w world] text - Send text to MUD

World Management:
  #world [name]        - Switch to or list worlds
  #connect [world]     - Connect to a world
  #listworlds          - List all worlds
  #listsockets         - List connected worlds
  #dc, #disconnect     - Disconnect current world

Misc:
  #help [topic]        - Show this help
  #version             - Show version info
  #quit                - Exit Clay

Variable Substitution:
  %{varname}           - Substitute variable value
  %varname             - Short form (ends at non-alphanumeric)
  %%                   - Literal percent sign

More commands coming in future phases:
  Phase 2: #expr, #eval, #test (expressions)
  Phase 3: #if, #while, #for (control flow)
  Phase 4: #def, #undef (macros/triggers)
  Phase 5: #hook, #bind (hooks/keybindings)
  Phase 6: Additional builtins"#;
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
            _ => TfCommandResult::Success(Some(format!("No help available for '{}'", topic))),
        }
    }
}

/// #version - Show version info
fn cmd_version() -> TfCommandResult {
    TfCommandResult::Success(Some(
        "Clay MUD Client with TinyFugue compatibility\nTF compatibility layer: Phase 2".to_string()
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
}
