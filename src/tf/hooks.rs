//! Hooks and keybindings for TinyFugue compatibility.
//!
//! Implements:
//! - Hook events (CONNECT, DISCONNECT, LOGIN, PROMPT, etc.)
//! - #hook, #unhook commands for direct hook management
//! - #bind, #unbind commands for keybindings
//! - Key name parsing (F1, ^A, etc.)

use super::{TfEngine, TfHookEvent, TfCommandResult};
use super::macros;

/// Fire all hooks registered for an event
pub fn fire_hook(engine: &mut TfEngine, event: TfHookEvent) -> Vec<TfCommandResult> {
    let mut results = Vec::new();

    // Get macro names registered for this hook
    let macro_names: Vec<String> = engine.hooks
        .get(&event)
        .cloned()
        .unwrap_or_default();

    // Find and execute each macro
    for name in macro_names {
        if let Some(macro_def) = engine.macros.iter().find(|m| m.name == name).cloned() {
            let exec_results = macros::execute_macro(engine, &macro_def, &[], None);
            results.extend(exec_results);
        }
    }

    // Also execute any macros with matching hook attribute
    let hook_macros: Vec<_> = engine.macros.iter()
        .filter(|m| m.hook == Some(event))
        .cloned()
        .collect();

    for macro_def in hook_macros {
        // Skip if already executed via hooks map
        if engine.hooks.get(&event).map(|v| v.contains(&macro_def.name)).unwrap_or(false) {
            continue;
        }
        let exec_results = macros::execute_macro(engine, &macro_def, &[], None);
        results.extend(exec_results);
    }

    results
}

/// Register a hook directly (not via #def)
pub fn register_hook(engine: &mut TfEngine, event: TfHookEvent, command: String) {
    // Create a unique name for this direct hook
    let name = format!("__hook_{:?}_{}", event, engine.hooks.get(&event).map(|v| v.len()).unwrap_or(0));

    // Create a simple macro for the command
    let macro_def = super::TfMacro {
        name: name.clone(),
        body: command,
        hook: Some(event),
        ..Default::default()
    };

    engine.macros.push(macro_def);
    engine.hooks.entry(event).or_default().push(name);
}

/// Remove all hooks for an event
pub fn unregister_hooks(engine: &mut TfEngine, event: TfHookEvent) -> usize {
    let count = engine.hooks.get(&event).map(|v| v.len()).unwrap_or(0);

    // Remove hook entries
    engine.hooks.remove(&event);

    // Remove macros that were created for this hook
    engine.macros.retain(|m| m.hook != Some(event));

    count
}

/// Parse a key name into a normalized form
/// Supports: F1-F12, ^A-^Z (Ctrl+letter), \e (escape), etc.
pub fn parse_key_name(name: &str) -> Result<String, String> {
    let name = name.trim();

    if name.is_empty() {
        return Err("Empty key name".to_string());
    }

    // Function keys: F1-F12
    if let Some(num) = name.strip_prefix('F').or_else(|| name.strip_prefix('f')) {
        if let Ok(n) = num.parse::<u8>() {
            if (1..=12).contains(&n) {
                return Ok(format!("F{}", n));
            }
        }
        return Err(format!("Invalid function key: {}", name));
    }

    // Control keys: ^A-^Z or Ctrl-A, Ctrl+A
    if let Some(letter) = name.strip_prefix('^') {
        if letter.len() == 1 {
            let c = letter.chars().next().unwrap().to_ascii_uppercase();
            if c.is_ascii_uppercase() {
                return Ok(format!("^{}", c));
            }
        }
        return Err(format!("Invalid control key: {}", name));
    }

    let lower = name.to_lowercase();
    if let Some(rest) = lower.strip_prefix("ctrl-").or_else(|| lower.strip_prefix("ctrl+")) {
        if rest.len() == 1 {
            let c = rest.chars().next().unwrap().to_ascii_uppercase();
            if c.is_ascii_uppercase() {
                return Ok(format!("^{}", c));
            }
        }
        return Err(format!("Invalid control key: {}", name));
    }

    // Alt/Meta keys: Alt-A, Meta-A, \eA
    if let Some(rest) = lower.strip_prefix("alt-")
        .or_else(|| lower.strip_prefix("alt+"))
        .or_else(|| lower.strip_prefix("meta-"))
        .or_else(|| lower.strip_prefix("meta+")) {
        return Ok(format!("Alt-{}", rest.to_uppercase()));
    }

    if let Some(rest) = name.strip_prefix("\\e") {
        return Ok(format!("Alt-{}", rest.to_uppercase()));
    }

    // Special keys
    match name.to_lowercase().as_str() {
        "enter" | "return" | "cr" => Ok("Enter".to_string()),
        "tab" => Ok("Tab".to_string()),
        "escape" | "esc" => Ok("Escape".to_string()),
        "space" => Ok("Space".to_string()),
        "backspace" | "bs" => Ok("Backspace".to_string()),
        "delete" | "del" => Ok("Delete".to_string()),
        "insert" | "ins" => Ok("Insert".to_string()),
        "home" => Ok("Home".to_string()),
        "end" => Ok("End".to_string()),
        "pageup" | "pgup" => Ok("PageUp".to_string()),
        "pagedown" | "pgdn" => Ok("PageDown".to_string()),
        "up" => Ok("Up".to_string()),
        "down" => Ok("Down".to_string()),
        "left" => Ok("Left".to_string()),
        "right" => Ok("Right".to_string()),
        _ => {
            // Single printable character
            if name.len() == 1 {
                Ok(name.to_string())
            } else {
                // Allow arbitrary key sequences as-is
                Ok(name.to_string())
            }
        }
    }
}

/// Register a keybinding
pub fn bind_key(engine: &mut TfEngine, key: &str, command: String) -> Result<(), String> {
    let normalized = parse_key_name(key)?;
    engine.keybindings.insert(normalized, command);
    Ok(())
}

/// Remove a keybinding
pub fn unbind_key(engine: &mut TfEngine, key: &str) -> Result<bool, String> {
    let normalized = parse_key_name(key)?;
    Ok(engine.keybindings.remove(&normalized).is_some())
}

/// Get command bound to a key
pub fn get_binding(engine: &TfEngine, key: &str) -> Option<String> {
    let normalized = parse_key_name(key).ok()?;
    engine.keybindings.get(&normalized).cloned()
}

/// List all keybindings
pub fn list_bindings(engine: &TfEngine) -> String {
    if engine.keybindings.is_empty() {
        return "No keybindings defined.".to_string();
    }

    let mut output = String::new();
    let mut bindings: Vec<_> = engine.keybindings.iter().collect();
    bindings.sort_by(|a, b| a.0.cmp(b.0));

    for (key, cmd) in bindings {
        output.push_str(&format!("{} = {}\n", key, cmd));
    }

    output
}

/// List all hooks
pub fn list_hooks(engine: &TfEngine) -> String {
    let mut output = String::new();

    let events = [
        TfHookEvent::Connect,
        TfHookEvent::Disconnect,
        TfHookEvent::Login,
        TfHookEvent::Prompt,
        TfHookEvent::Send,
        TfHookEvent::Activity,
        TfHookEvent::World,
        TfHookEvent::Resize,
        TfHookEvent::Load,
        TfHookEvent::Redef,
        TfHookEvent::Background,
    ];

    for event in &events {
        if let Some(macros) = engine.hooks.get(event) {
            if !macros.is_empty() {
                output.push_str(&format!("{:?}: {}\n", event, macros.join(", ")));
            }
        }
    }

    if output.is_empty() {
        "No hooks registered.".to_string()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_name_function_keys() {
        assert_eq!(parse_key_name("F1"), Ok("F1".to_string()));
        assert_eq!(parse_key_name("f12"), Ok("F12".to_string()));
        assert!(parse_key_name("F13").is_err());
        assert!(parse_key_name("F0").is_err());
    }

    #[test]
    fn test_parse_key_name_control_keys() {
        assert_eq!(parse_key_name("^A"), Ok("^A".to_string()));
        assert_eq!(parse_key_name("^z"), Ok("^Z".to_string()));
        assert_eq!(parse_key_name("Ctrl-A"), Ok("^A".to_string()));
        assert_eq!(parse_key_name("ctrl+x"), Ok("^X".to_string()));
    }

    #[test]
    fn test_parse_key_name_special_keys() {
        assert_eq!(parse_key_name("Enter"), Ok("Enter".to_string()));
        assert_eq!(parse_key_name("TAB"), Ok("Tab".to_string()));
        assert_eq!(parse_key_name("escape"), Ok("Escape".to_string()));
        assert_eq!(parse_key_name("PageUp"), Ok("PageUp".to_string()));
        assert_eq!(parse_key_name("pgdn"), Ok("PageDown".to_string()));
    }

    #[test]
    fn test_parse_key_name_alt_keys() {
        assert_eq!(parse_key_name("Alt-A"), Ok("Alt-A".to_string()));
        assert_eq!(parse_key_name("\\eW"), Ok("Alt-W".to_string()));
    }

    #[test]
    fn test_bind_unbind() {
        let mut engine = TfEngine::new();

        bind_key(&mut engine, "F1", "#help".to_string()).unwrap();
        assert_eq!(get_binding(&engine, "F1"), Some("#help".to_string()));

        unbind_key(&mut engine, "F1").unwrap();
        assert_eq!(get_binding(&engine, "F1"), None);
    }

    #[test]
    fn test_register_hook() {
        let mut engine = TfEngine::new();

        register_hook(&mut engine, TfHookEvent::Connect, "say Hello!".to_string());

        assert!(engine.hooks.get(&TfHookEvent::Connect).is_some());
        assert_eq!(engine.hooks.get(&TfHookEvent::Connect).unwrap().len(), 1);
    }

    #[test]
    fn test_unregister_hooks() {
        let mut engine = TfEngine::new();

        register_hook(&mut engine, TfHookEvent::Connect, "say Hello!".to_string());
        register_hook(&mut engine, TfHookEvent::Connect, "look".to_string());

        let count = unregister_hooks(&mut engine, TfHookEvent::Connect);
        assert_eq!(count, 2);
        assert!(engine.hooks.get(&TfHookEvent::Connect).is_none());
    }
}
