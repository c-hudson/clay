//! Additional builtin commands for TinyFugue compatibility.
//!
//! Implements:
//! - Output commands: #beep, #gag, #ungag, #recall, #quote
//! - File operations: #load, #save, #log
//! - Miscellaneous: #time, #sh, #lcd

use std::fs;
use std::path::Path;
use std::io::{BufRead, BufReader};
use super::{TfEngine, TfCommandResult};

/// #beep - Sound the terminal bell
pub fn cmd_beep() -> TfCommandResult {
    // Return a special message that the main app can interpret
    TfCommandResult::Success(Some("\x07".to_string()))
}

/// #time [format] - Display current time
pub fn cmd_time(args: &str) -> TfCommandResult {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let format = args.trim();

    if format.is_empty() {
        // Default format: human readable
        let secs = now % 60;
        let mins = (now / 60) % 60;
        let hours = (now / 3600) % 24;
        TfCommandResult::Success(Some(format!("{:02}:{:02}:{:02}", hours, mins, secs)))
    } else if format == "%s" || format == "epoch" {
        // Unix timestamp
        TfCommandResult::Success(Some(now.to_string()))
    } else {
        // For now, just return the timestamp
        // Full strftime support could be added later
        TfCommandResult::Success(Some(now.to_string()))
    }
}

/// #lcd [directory] - Change local directory
pub fn cmd_lcd(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let dir = args.trim();

    if dir.is_empty() {
        // Show current directory
        if let Some(ref cd) = engine.current_dir {
            return TfCommandResult::Success(Some(cd.clone()));
        }
        if let Ok(cwd) = std::env::current_dir() {
            return TfCommandResult::Success(Some(cwd.display().to_string()));
        }
        return TfCommandResult::Success(Some(".".to_string()));
    }

    // Expand ~ to home directory
    let expanded = if dir.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy();
            if dir == "~" {
                home_str.to_string()
            } else if let Some(rest) = dir.strip_prefix("~/") {
                format!("{}/{}", home_str, rest)
            } else {
                dir.to_string()
            }
        } else {
            dir.to_string()
        }
    } else {
        dir.to_string()
    };

    // Verify directory exists
    let path = Path::new(&expanded);
    if path.is_dir() {
        engine.current_dir = Some(expanded.clone());
        TfCommandResult::Success(Some(format!("Changed to {}", expanded)))
    } else {
        TfCommandResult::Error(format!("Directory not found: {}", expanded))
    }
}

/// #sh command - Execute shell command
pub fn cmd_sh(args: &str) -> TfCommandResult {
    let cmd = args.trim();

    if cmd.is_empty() {
        return TfCommandResult::Error("Usage: #sh command".to_string());
    }

    // Execute command and capture output
    match std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(&stderr);
            }

            if result.is_empty() {
                TfCommandResult::Success(None)
            } else {
                TfCommandResult::Success(Some(result.trim_end().to_string()))
            }
        }
        Err(e) => TfCommandResult::Error(format!("Failed to execute: {}", e)),
    }
}

/// #quote text - Send text literally without processing
pub fn cmd_quote(args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: #quote text".to_string());
    }
    // Send directly to MUD without any processing
    TfCommandResult::SendToMud(args.to_string())
}

/// #recall [pattern] - Search output history
/// Note: This returns a message indicating the feature needs main.rs integration
pub fn cmd_recall(args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        TfCommandResult::Success(Some("Usage: #recall pattern - Search output history".to_string()))
    } else {
        // This would need integration with the main output buffer
        // For now, return a message
        TfCommandResult::Success(Some(format!("Recall '{}' - requires main.rs integration", pattern)))
    }
}

/// #gag pattern - Add a gag pattern (suppress matching output)
/// Note: Returns a message for main.rs integration
pub fn cmd_gag(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: #gag pattern".to_string());
    }

    // Create a macro with gag attribute
    let macro_def = super::TfMacro {
        name: format!("__gag_{}", engine.macros.len()),
        body: String::new(),
        trigger: Some(super::TfTrigger {
            pattern: pattern.to_string(),
            match_mode: super::TfMatchMode::Glob,
            compiled: regex::Regex::new(&super::macros::glob_to_regex(pattern)).ok(),
        }),
        attributes: super::TfAttributes {
            gag: true,
            ..Default::default()
        },
        ..Default::default()
    };

    engine.macros.push(macro_def);
    TfCommandResult::Success(Some(format!("Gagging '{}'", pattern)))
}

/// #ungag pattern - Remove a gag pattern
pub fn cmd_ungag(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: #ungag pattern".to_string());
    }

    let before = engine.macros.len();
    engine.macros.retain(|m| {
        if let Some(ref trigger) = m.trigger {
            !(m.attributes.gag && trigger.pattern == pattern)
        } else {
            true
        }
    });

    let removed = before - engine.macros.len();
    if removed > 0 {
        TfCommandResult::Success(Some(format!("Removed {} gag(s)", removed)))
    } else {
        TfCommandResult::Error(format!("Gag pattern '{}' not found", pattern))
    }
}

/// #load filename - Load and execute a TF script file
pub fn cmd_load(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let filename = args.trim();

    if filename.is_empty() {
        return TfCommandResult::Error("Usage: #load filename".to_string());
    }

    // Expand ~ to home directory
    let expanded = if filename.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy();
            if filename == "~" {
                home_str.to_string()
            } else if let Some(rest) = filename.strip_prefix("~/") {
                format!("{}/{}", home_str, rest)
            } else {
                filename.to_string()
            }
        } else {
            filename.to_string()
        }
    } else {
        filename.to_string()
    };

    // Read and execute the file
    let path = Path::new(&expanded);
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return TfCommandResult::Error(format!("Cannot open '{}': {}", expanded, e)),
    };

    let reader = BufReader::new(file);
    let mut results = Vec::new();
    let mut line_num = 0;

    for line in reader.lines() {
        line_num += 1;
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                results.push(TfCommandResult::Error(format!("Line {}: {}", line_num, e)));
                continue;
            }
        };

        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with(";;") {
            continue;
        }

        // Execute the line
        if trimmed.starts_with('#') || trimmed.starts_with('/') {
            let result = super::parser::execute_command(engine, trimmed);
            match &result {
                TfCommandResult::Error(e) => {
                    results.push(TfCommandResult::Error(format!("Line {}: {}", line_num, e)));
                }
                _ => results.push(result),
            }
        }
        // Non-command lines are ignored in script files
    }

    // Fire LOAD hook
    let hook_results = super::hooks::fire_hook(engine, super::TfHookEvent::Load);
    results.extend(hook_results);

    let error_count = results.iter().filter(|r| matches!(r, TfCommandResult::Error(_))).count();
    if error_count > 0 {
        TfCommandResult::Error(format!("Loaded '{}' with {} error(s)", expanded, error_count))
    } else {
        TfCommandResult::Success(Some(format!("Loaded '{}'", expanded)))
    }
}

/// #save filename - Save macros to a file
pub fn cmd_save(engine: &TfEngine, args: &str) -> TfCommandResult {
    let filename = args.trim();

    if filename.is_empty() {
        return TfCommandResult::Error("Usage: #save filename".to_string());
    }

    // Expand ~ to home directory
    let expanded = if filename.starts_with('~') {
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy();
            if filename == "~" {
                return TfCommandResult::Error("Cannot save to home directory".to_string());
            } else if let Some(rest) = filename.strip_prefix("~/") {
                format!("{}/{}", home_str, rest)
            } else {
                filename.to_string()
            }
        } else {
            filename.to_string()
        }
    } else {
        filename.to_string()
    };

    let mut output = String::new();

    // Save global variables
    output.push_str(";; TinyFugue script generated by Clay\n");
    output.push_str(";; Variables\n");
    for (name, value) in &engine.global_vars {
        output.push_str(&format!("#set {} {}\n", name, value.to_string_value()));
    }

    // Save macros
    output.push_str("\n;; Macros\n");
    for macro_def in &engine.macros {
        // Skip internal macros
        if macro_def.name.starts_with("__") {
            continue;
        }

        let mut def_line = String::from("#def ");

        // Add flags
        if let Some(ref trigger) = macro_def.trigger {
            if !trigger.pattern.is_empty() {
                def_line.push_str(&format!("-t\"{}\" ", trigger.pattern));
                if trigger.match_mode != super::TfMatchMode::Glob {
                    def_line.push_str(&format!("-m{:?} ", trigger.match_mode).to_lowercase());
                }
            }
        }

        if macro_def.priority != 0 {
            def_line.push_str(&format!("-p{} ", macro_def.priority));
        }
        if macro_def.fall_through {
            def_line.push_str("-F ");
        }
        if let Some(n) = macro_def.one_shot {
            if n == 1 {
                def_line.push_str("-1 ");
            } else {
                def_line.push_str(&format!("-n{} ", n));
            }
        }
        if let Some(ref hook) = macro_def.hook {
            def_line.push_str(&format!("-h{:?} ", hook));
        }
        if let Some(ref keys) = macro_def.keybinding {
            def_line.push_str(&format!("-b\"{}\" ", keys));
        }
        if let Some(ref world) = macro_def.world {
            def_line.push_str(&format!("-w\"{}\" ", world));
        }
        if let Some(ref cond) = macro_def.condition {
            def_line.push_str(&format!("-E\"{}\" ", cond));
        }
        if let Some(prob) = macro_def.probability {
            def_line.push_str(&format!("-c{} ", prob));
        }

        // Add attributes
        let mut attrs: Vec<String> = Vec::new();
        if macro_def.attributes.gag { attrs.push("gag".to_string()); }
        if macro_def.attributes.bold { attrs.push("bold".to_string()); }
        if macro_def.attributes.underline { attrs.push("underline".to_string()); }
        if macro_def.attributes.reverse { attrs.push("reverse".to_string()); }
        if macro_def.attributes.flash { attrs.push("flash".to_string()); }
        if macro_def.attributes.dim { attrs.push("dim".to_string()); }
        if macro_def.attributes.bell { attrs.push("bell".to_string()); }
        if let Some(ref color) = macro_def.attributes.hilite {
            attrs.push(format!("hilite:{}", color));
        }
        if !attrs.is_empty() {
            def_line.push_str(&format!("-a{} ", attrs.join(",")));
        }

        def_line.push_str(&format!("{} = {}\n", macro_def.name, macro_def.body));
        output.push_str(&def_line);
    }

    // Save keybindings
    output.push_str("\n;; Keybindings\n");
    for (key, cmd) in &engine.keybindings {
        output.push_str(&format!("#bind {} = {}\n", key, cmd));
    }

    // Write to file
    match fs::write(&expanded, output) {
        Ok(()) => TfCommandResult::Success(Some(format!("Saved to '{}'", expanded))),
        Err(e) => TfCommandResult::Error(format!("Cannot write '{}': {}", expanded, e)),
    }
}

/// #log [filename] - Toggle logging to file
/// Note: Actual logging needs main.rs integration, this just returns a message
pub fn cmd_log(args: &str) -> TfCommandResult {
    let filename = args.trim();

    if filename.is_empty() {
        TfCommandResult::Success(Some("Usage: #log filename - Toggle logging (requires main.rs integration)".to_string()))
    } else {
        TfCommandResult::Success(Some(format!("Log '{}' - requires main.rs integration", filename)))
    }
}

/// #ps - List background processes (placeholder)
pub fn cmd_ps() -> TfCommandResult {
    TfCommandResult::Success(Some("No background processes.".to_string()))
}

/// #kill pid - Kill background process (placeholder)
pub fn cmd_kill(args: &str) -> TfCommandResult {
    let pid = args.trim();
    if pid.is_empty() {
        TfCommandResult::Error("Usage: #kill pid".to_string())
    } else {
        TfCommandResult::Error(format!("Process {} not found", pid))
    }
}

/// Convert glob pattern to regex (re-exported from macros for use here)
pub use super::macros::glob_to_regex;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_beep() {
        let result = cmd_beep();
        assert!(matches!(result, TfCommandResult::Success(Some(s)) if s == "\x07"));
    }

    #[test]
    fn test_cmd_time() {
        let result = cmd_time("");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));

        let result = cmd_time("%s");
        if let TfCommandResult::Success(Some(s)) = result {
            assert!(s.parse::<u64>().is_ok());
        }
    }

    #[test]
    fn test_cmd_lcd() {
        let mut engine = TfEngine::new();

        // Show current directory
        let result = cmd_lcd(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));

        // Change to /tmp (should exist on most systems)
        let result = cmd_lcd(&mut engine, "/tmp");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert_eq!(engine.current_dir, Some("/tmp".to_string()));

        // Try non-existent directory
        let result = cmd_lcd(&mut engine, "/nonexistent_dir_12345");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_quote() {
        let result = cmd_quote("hello world");
        assert!(matches!(result, TfCommandResult::SendToMud(s) if s == "hello world"));

        let result = cmd_quote("");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_sh() {
        let result = cmd_sh("echo hello");
        if let TfCommandResult::Success(Some(s)) = result {
            assert!(s.contains("hello"));
        }

        let result = cmd_sh("");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_gag_ungag() {
        let mut engine = TfEngine::new();

        // Add a gag
        let result = cmd_gag(&mut engine, "spam*");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert!(engine.macros.iter().any(|m| m.attributes.gag));

        // Remove the gag
        let result = cmd_ungag(&mut engine, "spam*");
        assert!(matches!(result, TfCommandResult::Success(_)));
        assert!(!engine.macros.iter().any(|m| m.attributes.gag && m.trigger.as_ref().map(|t| t.pattern == "spam*").unwrap_or(false)));
    }
}
