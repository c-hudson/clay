//! Bridge module for integrating TF engine with Clay main application.
//!
//! This module provides the interface between the TF scripting engine
//! and the main Clay MUD client, allowing:
//! - Trigger processing on incoming MUD output
//! - Hook firing on connect/disconnect events
//! - Conversion between TF macros and Clay actions

use super::{TfEngine, TfCommandResult, TfHookEvent, TfMatchMode};
use super::macros;
use super::hooks;
use super::control_flow;

/// Result of processing TF triggers against a line
#[derive(Debug, Default)]
pub struct TfTriggerResult {
    /// Commands to send to the MUD server
    pub send_commands: Vec<String>,
    /// Commands to execute as Clay commands
    pub clay_commands: Vec<String>,
    /// Messages to display locally
    pub messages: Vec<String>,
    /// Whether to gag (suppress) the line
    pub should_gag: bool,
    /// Any errors that occurred
    pub errors: Vec<String>,
    /// Substituted text (replaces the original line)
    pub substitution: Option<(String, String)>,  // (text, attrs)
    /// `fire_event` only: a non-quiet hook macro matched. Per `/help hooks`'
    /// SEND rule ("If a SEND hook matches the text that would be sent, the text
    /// is not sent (unless the hook was defined with /def -q)"), a SEND caller
    /// uses this to decide whether to suppress sending the original text - see
    /// `App::fire_tf_hook`.
    pub matched_non_quiet: bool,
}

/// Process a line of MUD output against all TF triggers
/// Returns the combined results of all matching triggers
pub fn process_line(engine: &mut TfEngine, line: &str, world: Option<&str>, world_type: Option<&str>) -> TfTriggerResult {
    let mut result = TfTriggerResult::default();

    // Strip ANSI codes and trailing whitespace for pattern matching (like Clay actions do)
    let plain_line = crate::util::strip_ansi_codes(line);
    let plain_line = plain_line.trim_end();

    // Get trigger results from the macro system
    let command_results = macros::process_triggers(engine, plain_line, world, world_type);

    // Process each result
    for cmd_result in command_results {
        match cmd_result {
            TfCommandResult::Success(Some(msg)) => {
                result.messages.push(msg);
            }
            TfCommandResult::SendToMud(cmd) => {
                result.send_commands.push(cmd);
            }
            TfCommandResult::ClayCommand(cmd) => {
                result.clay_commands.push(cmd);
            }
            TfCommandResult::Error(e) if control_flow::parse_break_marker(&e).is_none() => {
                result.errors.push(e);
            }
            _ => {}
        }
    }

    // Check if any matching macro has gag attribute
    for macro_def in &engine.macros {
        if !macro_def.attributes.gag {
            continue;
        }

        let trigger = match &macro_def.trigger {
            Some(t) if !t.pattern.is_empty() => t,
            _ => continue,
        };

        if macros::match_trigger(trigger, plain_line).is_none() {
            continue;
        }

        // Check world restriction
        if let Some(ref macro_world) = macro_def.world {
            if let Some(current_world) = world {
                if macro_world != current_world {
                    continue;
                }
            }
        }

        // Check world-type restriction (-T)
        if !macros::world_type_matches(macro_def, world_type) {
            continue;
        }

        result.should_gag = true;
        break;
    }

    // Check for pending substitution
    if let Some(sub) = engine.pending_substitution.take() {
        result.substitution = Some((sub.text, sub.attrs));
    }

    result
}

/// Fire hooks for a specific event with `arg` as its argument text (finding
/// C.10 / plan step P1.9 - see `hooks::fire_hook`'s own doc comment for what
/// `arg` means per event and how pattern/capture matching works).
pub fn fire_event(engine: &mut TfEngine, event: TfHookEvent, arg: &str) -> TfTriggerResult {
    let mut result = TfTriggerResult::default();

    let outcome = hooks::fire_hook(engine, event, arg);
    result.matched_non_quiet = outcome.matched_non_quiet;

    for cmd_result in outcome.results {
        match cmd_result {
            TfCommandResult::Success(Some(msg)) => {
                result.messages.push(msg);
            }
            TfCommandResult::SendToMud(cmd) => {
                result.send_commands.push(cmd);
            }
            TfCommandResult::ClayCommand(cmd) => {
                result.clay_commands.push(cmd);
            }
            TfCommandResult::Error(e) => {
                result.errors.push(e);
            }
            _ => {}
        }
    }

    result
}

/// Convert TF macros with triggers to a format suitable for display
/// Returns a list of (name, pattern, command, world, match_type_name)
pub fn get_trigger_macros(engine: &TfEngine) -> Vec<TfMacroInfo> {
    engine.macros.iter()
        .filter(|m| m.trigger.as_ref().map(|t| !t.pattern.is_empty()).unwrap_or(false))
        .map(|m| TfMacroInfo {
            name: m.name.clone(),
            pattern: m.trigger.as_ref().map(|t| t.pattern.clone()).unwrap_or_default(),
            command: m.body.clone(),
            world: m.world.clone().unwrap_or_default(),
            match_type: m.trigger.as_ref()
                .map(|t| match t.match_mode {
                    TfMatchMode::Simple => "simple",
                    TfMatchMode::Glob => "glob",
                    TfMatchMode::Regexp => "regexp",
                })
                .unwrap_or("glob")
                .to_string(),
            priority: m.priority,
            is_gag: m.attributes.gag,
        })
        .collect()
}

/// Information about a TF macro for display purposes
#[derive(Debug, Clone)]
pub struct TfMacroInfo {
    pub name: String,
    pub pattern: String,
    pub command: String,
    pub world: String,
    pub match_type: String,
    pub priority: i32,
    pub is_gag: bool,
}

/// Check if a line matches any TF trigger pattern (for highlighting)
pub fn line_matches_trigger(engine: &TfEngine, line: &str, world: Option<&str>) -> bool {
    for macro_def in &engine.macros {
        // Check world restriction
        if let Some(ref macro_world) = macro_def.world {
            if let Some(current_world) = world {
                if macro_world != current_world {
                    continue;
                }
            }
        }

        if let Some(ref trigger) = macro_def.trigger {
            if !trigger.pattern.is_empty() && macros::match_trigger(trigger, line).is_some() {
                return true;
            }
        }
    }
    false
}

/// Get statistics about the TF engine
pub fn get_stats(engine: &TfEngine) -> TfEngineStats {
    let trigger_count = engine.macros.iter()
        .filter(|m| m.trigger.as_ref().map(|t| !t.pattern.is_empty()).unwrap_or(false))
        .count();

    let hook_count: usize = engine.macros.iter().filter(|m| m.hook.is_some()).count();

    TfEngineStats {
        variable_count: engine.global_vars.len(),
        macro_count: engine.macros.len(),
        trigger_count,
        hook_count,
        keybinding_count: engine.keybindings.len(),
    }
}

/// Statistics about the TF engine
#[derive(Debug, Clone)]
pub struct TfEngineStats {
    pub variable_count: usize,
    pub macro_count: usize,
    pub trigger_count: usize,
    pub hook_count: usize,
    pub keybinding_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::TfMacro;

    #[test]
    fn test_process_line_no_triggers() {
        let mut engine = TfEngine::new();
        let result = process_line(&mut engine, "Hello world", None, None);
        assert!(result.send_commands.is_empty());
        assert!(result.clay_commands.is_empty());
        assert!(!result.should_gag);
    }

    #[test]
    fn test_process_line_with_trigger() {
        let mut engine = TfEngine::new();

        // Add a trigger macro
        engine.macros.push(TfMacro {
            name: "test".to_string(),
            body: "say matched!".to_string(),
            trigger: Some(super::super::TfTrigger {
                pattern: "Hello.*".to_string(),
                match_mode: TfMatchMode::Regexp,
                compiled: regex::Regex::new("Hello.*").ok(),
            }),
            ..Default::default()
        });

        let result = process_line(&mut engine, "Hello world", None, None);
        assert!(result.send_commands.contains(&"say matched!".to_string()));
    }

    #[test]
    fn test_get_trigger_macros() {
        let mut engine = TfEngine::new();

        engine.macros.push(TfMacro {
            name: "trigger1".to_string(),
            body: "cmd1".to_string(),
            trigger: Some(super::super::TfTrigger {
                pattern: "pattern1".to_string(),
                match_mode: TfMatchMode::Glob,
                compiled: None,
            }),
            ..Default::default()
        });

        engine.macros.push(TfMacro {
            name: "notrigger".to_string(),
            body: "cmd2".to_string(),
            trigger: None,
            ..Default::default()
        });

        let macros = get_trigger_macros(&engine);
        assert_eq!(macros.len(), 1);
        assert_eq!(macros[0].name, "trigger1");
    }

    #[test]
    fn test_get_stats() {
        let mut engine = TfEngine::new();
        // TfEngine::new() may itself seed TFLIBDIR/TFPATH globals (see
        // "P1.3 library search path" - default_tflibdir()/$TFPATH), so the
        // baseline var count depends on the machine (e.g. whether it has a
        // system tf-lib). Compare against that baseline rather than
        // asserting an absolute count.
        let baseline_vars = engine.global_vars.len();
        engine.set_global("var1", super::super::TfValue::String("test".to_string()));
        engine.macros.push(TfMacro {
            name: "m1".to_string(),
            body: "test".to_string(),
            ..Default::default()
        });

        let stats = get_stats(&engine);
        assert_eq!(stats.variable_count, baseline_vars + 1);
        assert_eq!(stats.macro_count, 1);
    }
}
