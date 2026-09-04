//! Additional builtin commands for TinyFugue compatibility.
//!
//! Implements:
//! - Output commands: /beep, /gag, /ungag, /recall, /quote
//! - File operations: /load, /save, /log
//! - Miscellaneous: /time, /sh, /lcd

use std::fs;
use std::path::Path;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};
use super::{TfEngine, TfProcess, TfCommandResult, RecallOptions, RecallSource, RecallRange, RecallMatchStyle};

/// /beep [number|on|off] - Sound the terminal bell
pub fn cmd_beep(engine: &mut super::TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim().to_lowercase();
    match arg.as_str() {
        "off" => {
            engine.set_global("beep", super::TfValue::from("0"));
            return TfCommandResult::Success(Some("beep off".to_string()));
        }
        "on" => {
            engine.set_global("beep", super::TfValue::from("1"));
            return TfCommandResult::Success(Some("beep on".to_string()));
        }
        _ => {}
    }
    // Check if beep is disabled
    let beep_val = engine.get_var("beep").map(|v| v.to_string_value()).unwrap_or_default();
    if beep_val == "0" {
        return TfCommandResult::Success(None);
    }
    // Parse count (default 3)
    let count = if arg.is_empty() {
        3
    } else {
        arg.parse::<usize>().unwrap_or(3).min(100)
    };
    let beeps = "\x07".repeat(count);
    TfCommandResult::Success(Some(beeps))
}

/// /time [<format>] - TF: print the current time formatted by `<format>`
/// (`ftime()`-style; defaults to `%time_format`, itself defaulting to
/// "%H:%M" - see `/help time` and `/help ftime()`, both verified against
/// real tf), and set `%?` to the formatted string, same as `ftime()` itself.
///
/// `/time /command...` (an argument starting with `/`) is Clay's own kept
/// extension (finding B: "both" ruling) rather than a TF form: run
/// `<command>` and report how long it took. TF's own equivalent is the
/// native `/runtime` below, which this does NOT replace - `/time /cmd` never
/// substitutes `args` itself (see the `is_recall_command`-style exemption in
/// `parser::execute_tf_command` - a format string's own "%" strftime
/// specifiers, e.g. `-t"%H:%M:%S"`-shaped text, must not be eaten as TF
/// variable sigils before `cmd_time` ever sees them), so `<command>` gets
/// substituted fresh, exactly as if it had been typed directly.
pub fn cmd_time(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if let Some(rest) = args.strip_prefix('/') {
        if rest.trim().is_empty() {
            return TfCommandResult::Error("Usage: /time /command".to_string());
        }
        let full_cmd = format!("/{}", rest);
        let start = Instant::now();
        let result = super::parser::execute_command(engine, &full_cmd);
        let elapsed = start.elapsed();
        let timing = TfCommandResult::Success(Some(format!("Elapsed: {:.3}s", elapsed.as_secs_f64())));
        return super::parser::aggregate_results_with_engine(engine, vec![result, timing]);
    }

    let format = if args.is_empty() {
        engine.get_var("time_format")
            .map(|v| v.to_string_value())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "%H:%M".to_string())
    } else {
        args.to_string()
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let epoch_secs = now.as_secs() as i64;
    let frac_secs = now.subsec_nanos() as f64 / 1_000_000_000.0;

    // "@" is ftime()'s own raw-system-time shorthand (`/help ftime()`), same as
    // expressions::evaluate's "ftime" function arm handles it.
    let formatted = if format == "@" {
        format!("{}.{:06}", epoch_secs, (frac_secs * 1_000_000.0).round() as i64)
    } else {
        let lt = crate::util::local_time_from_epoch(epoch_secs);
        super::expressions::format_tf_time(&lt, epoch_secs, frac_secs, &format)
    };

    engine.set_global("?", super::TfValue::String(formatted.clone()));
    TfCommandResult::Success(Some(formatted))
}

/// /runtime <command> - TF's stdlib macro (`stdlib.tf`: `/def -i runtime = ...
/// /test real:=time(), cpu:=cputime()%; /eval -s0 %{*}%; /let result=%?%;
/// /_echo real=$[time() - real] cpu=$[cputime() - cpu]%; /return result`),
/// implemented natively instead of shipped as GPL stdlib text (same
/// "native, not a shipped stdlib" call as the rest of finding C.11's
/// one-liners - survives hot reload, no three-UI plumbing needed). Runs
/// `<command>` exactly like `/eval -s0` (no extra substitution pass - `args`
/// already went through the ordinary top-level substitution before
/// `cmd_runtime` ever sees it, same as any other command's arguments), then
/// prints TF's own `"real=<secs> cpu=<secs>"` line - verified directly
/// against real tf's own `/runtime /echo x` output shape (the exact digits
/// are inherently timing-dependent and not reproducible). `cputime()` is -1
/// when unavailable (`expressions::process_cpu_time_secs`'s own documented
/// fallback); `cpu=` is then also -1, matching real tf on such a platform.
pub fn cmd_runtime(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let text = args.trim();
    if text.is_empty() {
        return TfCommandResult::Error("Usage: /runtime <command>".to_string());
    }

    let start_wall = Instant::now();
    let start_cpu = super::expressions::process_cpu_time_secs();

    let inner = if text.starts_with('/') {
        super::parser::execute_command_substituted(engine, text)
    } else {
        TfCommandResult::SendToMud(text.to_string())
    };

    let real = start_wall.elapsed().as_secs_f64();
    let cpu = if start_cpu >= 0.0 {
        let end_cpu = super::expressions::process_cpu_time_secs();
        if end_cpu >= 0.0 { end_cpu - start_cpu } else { -1.0 }
    } else {
        -1.0
    };

    let timing_line = format!(
        "real={} cpu={}",
        super::TfValue::Float(real).to_string_value(),
        super::TfValue::Float(cpu).to_string_value(),
    );

    super::parser::aggregate_results_with_engine(engine, vec![inner, TfCommandResult::Success(Some(timing_line))])
}

/// /lcd [<dir>] - Change local directory, or with no `<dir>`, report the
/// current one (`/help lcd`: "If <dir> is omitted with /lcd, the current
/// directory is displayed"). Message wording matches real tf directly
/// (verified: bare /lcd, and a successful `/lcd <dir>`, both say "Current
/// directory is <path>" - real tf does NOT say "Changed to <path>"; a
/// missing directory says "LCD: <path>: No such file or directory").
pub fn cmd_lcd(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // /restrict FILE disables /lcd outright, report form included (`/help restrict`
    // level 2 explicitly lists "/lcd"; verified directly: a bare `/lcd` under
    // /restrict FILE also errors "LCD: restricted", not just a directory change).
    if engine.restrict_level >= super::RestrictLevel::File {
        return TfCommandResult::Error("LCD: restricted".to_string());
    }

    let dir = args.trim();

    if dir.is_empty() {
        // Show current directory. Real tf's own %? after a successful /lcd (report or
        // change) is 1, not the printed path (verified directly - unlike /pwd below,
        // which returns the path itself) - see this function's other return sites.
        engine.set_global("?", super::TfValue::Integer(1));
        if let Some(ref cd) = engine.current_dir {
            return TfCommandResult::Success(Some(format!("Current directory is {}", cd)));
        }
        if let Ok(cwd) = std::env::current_dir() {
            return TfCommandResult::Success(Some(format!("Current directory is {}", cwd.display())));
        }
        return TfCommandResult::Success(Some("Current directory is .".to_string()));
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
        engine.set_global("?", super::TfValue::Integer(1));
        TfCommandResult::Success(Some(format!("Current directory is {}", expanded)))
    } else {
        engine.set_global("?", super::TfValue::Integer(0));
        TfCommandResult::Error(format!("LCD: {}: No such file or directory", expanded))
    }
}

/// /cd [<dir>] - Change local directory, defaulting to `$HOME` when `<dir>`
/// is omitted (unlike bare `/lcd`, which reports the current directory
/// instead - `/help lcd`: "If <dir> is omitted with /cd, %{HOME} is
/// assumed", matching stdlib.tf's own `/def -i cd = /lcd %{*-%HOME}`).
pub fn cmd_cd(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let dir = args.trim();
    if dir.is_empty() {
        match std::env::var("HOME") {
            Ok(home) if !home.is_empty() => cmd_lcd(engine, &home),
            _ => TfCommandResult::Error("CD: HOME is not set".to_string()),
        }
    } else {
        cmd_lcd(engine, dir)
    }
}

/// /pwd - Display the current working directory (`/help lcd`: "/pwd
/// displays the current working directory", matching stdlib.tf's own `/def
/// -i pwd = /last $(/@lcd)` - i.e. always the bare-/lcd report form, with
/// none of its own "Current directory is" wrapper: real tf's `/last`
/// extracts just the value there, verified directly - `/pwd` prints only
/// the path).
pub fn cmd_pwd(engine: &mut TfEngine) -> TfCommandResult {
    let path = if let Some(ref cd) = engine.current_dir {
        cd.clone()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.display().to_string()
    } else {
        ".".to_string()
    };
    // Command form both prints and returns the path itself (Job 15, verified directly:
    // %? after /pwd holds the same path text) - unlike /lcd's own %?, which is a plain
    // 1/0 success flag.
    engine.set_global("?", super::TfValue::String(path.clone()));
    TfCommandResult::Success(Some(path))
}

/// /sh [-q] [<command>] - Execute a shell command (`/help sh`). With no
/// `<command>`, real tf spawns an interactive shell in place; Clay's TUI
/// owns the whole screen and has no safe way to hand it to a subprocess
/// (unlike real tf's visual-mode "fix the screen first, restore it after"),
/// so bare `/sh` reports that instead of hanging (plan Job 14c). With a
/// `<command>`, runs it via `/bin/sh -c` and captures output (unchanged
/// from before this job); `-q` suppresses both the SHELL hook and the "%
/// Executing command: <command>" message real tf prints by default
/// (`/help sh`: "the SHELL hook will not be called, and the 'Executing'
/// line will not be printed" - `/help hooks`' own SHELL entry gives the
/// default message shape, "type, command '% Executing <type>: <command>'";
/// "command" as `<type>` is verified directly against real tf for this
/// one-shot form).
pub fn cmd_sh(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if engine.restrict_level >= super::RestrictLevel::Shell {
        return TfCommandResult::Error("SH: restricted".to_string());
    }
    let mut args = args.trim();
    let mut quiet = false;
    if let Some(rest) = args.strip_prefix("-q") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            quiet = true;
            args = rest.trim_start();
        }
    }

    let cmd = args;
    if cmd.is_empty() {
        return TfCommandResult::Error(
            "SH: an interactive shell is not supported in Clay; use /sh <command>".to_string()
        );
    }

    let mut messages = Vec::new();
    if !quiet {
        let outcome = super::hooks::fire_hook(engine, super::TfHookEvent::Shell, &format!("command {}", cmd));
        let gagged = outcome.matched_any && outcome.first_fired_gagged == Some(true);
        if !gagged {
            messages.push(format!("Executing command: {}", cmd));
        }
        for r in outcome.results {
            if let TfCommandResult::Success(Some(m)) = r {
                messages.push(m);
            }
        }
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
            if !stdout.is_empty() {
                messages.push(stdout.trim_end().to_string());
            }
            if !stderr.is_empty() {
                messages.push(stderr.trim_end().to_string());
            }
        }
        Err(e) => return TfCommandResult::Error(format!("Failed to execute: {}", e)),
    }

    if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
}

/// /quote [options] [prefix] source [suffix] - Generate text from file, command, or literal
/// Options: -dsend|echo|exec  -wworld  -<delay>  -S  -P  -A (keep ANSI sequences)
///
/// Sources:
///   '"file"     - Read lines from a file
///   `"command"  - Read output from internal Clay/TF command
///   !"command"  - Read output from shell command
///   text        - Send literal text (no special prefix)
///
/// Options:
///   -dsend      - Send each line to MUD (default when no prefix)
///   -decho      - Echo each line locally
///   -dexec      - Execute each line as TF command
///   -wworld     - Send to specified world
///   -S          - Synchronous mode (wait for completion)
///
/// Examples:
///   /quote hello world           - Send "hello world" to MUD
///   /quote '"/etc/motd"          - Send each line of /etc/motd to MUD
///   /quote say '"/tmp/lines.txt" - Send "say <line>" for each line
///   /quote think `"/version"     - Send "think <version>" to MUD
///   /quote !"ls -la"             - Send output of shell ls command
///   /quote -decho '"config.txt"  - Display file contents locally
pub fn cmd_quote(engine: &mut super::TfEngine, args: &str) -> TfCommandResult {
    use super::QuoteDisposition;
    use std::process::{Command, Stdio};

    if args.is_empty() {
        return TfCommandResult::Error("Usage: /quote [-dsend|echo|exec] [-wworld] [-A] [prefix] source [suffix]".to_string());
    }

    let mut input = args.trim();
    let mut disposition = QuoteDisposition::Send;
    let mut disposition_explicit = false;
    let mut world: Option<String> = None;
    let mut _synchronous = false;
    let mut _on_prompt = false;  // -P flag: run on prompt (not yet implemented)
    let mut delay_secs: f64 = 0.0;  // Timing between lines
    let mut strip_ansi = true;  // Strip ANSI/escape sequences by default; -A disables

    // Helper to parse time string: "seconds", "min:sec", or "hour:min:sec"
    fn parse_time_spec(s: &str) -> Option<f64> {
        if s == "S" {
            return Some(0.0);  // Synchronous = no delay
        }
        if s == "P" {
            return None;  // Prompt-based, handled separately
        }
        let parts: Vec<&str> = s.split(':').collect();
        match parts.len() {
            1 => parts[0].parse::<f64>().ok(),
            2 => {
                // Could be hours:minutes or minutes:seconds
                // TF treats it as hours:minutes, but we'll be flexible
                let a: f64 = parts[0].parse().ok()?;
                let b: f64 = parts[1].parse().ok()?;
                Some(a * 60.0 + b)  // Treat as minutes:seconds for practical use
            }
            3 => {
                let hours: f64 = parts[0].parse().ok()?;
                let mins: f64 = parts[1].parse().ok()?;
                let secs: f64 = parts[2].parse().ok()?;
                Some(hours * 3600.0 + mins * 60.0 + secs)
            }
            _ => None,
        }
    }

    // Check if string looks like a time spec (digits, colons, dots, or S/P)
    fn is_time_spec(s: &str) -> bool {
        if s == "S" || s == "P" {
            return true;
        }
        !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.')
    }

    // Parse options
    while input.starts_with('-') {
        if let Some(space_pos) = input.find(|c: char| c.is_whitespace()) {
            let opt = &input[..space_pos];
            input = input[space_pos..].trim_start();

            if let Some(disp_str) = opt.strip_prefix("-d") {
                disposition_explicit = true;
                disposition = match disp_str {
                    "send" => QuoteDisposition::Send,
                    "echo" => QuoteDisposition::Echo,
                    "exec" => QuoteDisposition::Exec,
                    _ => return TfCommandResult::Error(format!("Unknown disposition: {}. Use send, echo, or exec.", disp_str)),
                };
            } else if let Some(w) = opt.strip_prefix("-w") {
                world = Some(w.to_string());
            } else if opt == "-S" {
                _synchronous = true;
            } else if opt == "-P" {
                _on_prompt = true;
            } else if opt == "-A" {
                strip_ansi = false;
            } else if opt.len() >= 2 && is_time_spec(&opt[1..]) {
                // Timing option: -0, -1, -0.5, -1:30, -1:30:00, etc.
                let time_str = &opt[1..];
                if time_str == "P" {
                    _on_prompt = true;
                } else if let Some(secs) = parse_time_spec(time_str) {
                    delay_secs = secs;
                    if time_str == "S" {
                        _synchronous = true;
                    }
                } else {
                    return TfCommandResult::Error(format!("Invalid timing option: {}", opt));
                }
            } else {
                return TfCommandResult::Error(format!("Unknown option: {}", opt));
            }
        } else {
            // Option at end with no more args - check if it's a valid option
            if input.starts_with("-d") || input.starts_with("-w") || input == "-S" || input == "-P" || input == "-A" {
                return TfCommandResult::Error("No source specified after options".to_string());
            }
            // Check for timing option at end
            if input.len() >= 2 && is_time_spec(&input[1..]) {
                return TfCommandResult::Error("No source specified after options".to_string());
            }
            // Not an option - break to process as source
            break;
        }
    }

    // Find the source specifier: ' for file, ` or ! for shell, # for TF command
    // Format: [prefix] source [suffix]
    // source is: '"file"suffix or 'file suffix or `"cmd"suffix or !cmd suffix

    let (prefix, source_pos) = if let Some(pos) = input.find(['\'', '`', '!', '#']) {
        // Check if the # is actually a TF command source or just part of text
        let char_at_pos = input.chars().nth(pos).unwrap();
        if char_at_pos == '#' {
            // Only treat as source if followed by " (for #"command" syntax)
            let after_hash = &input[pos + 1..];
            if after_hash.starts_with('"') {
                // Keep trailing space in prefix (user controls spacing)
                (&input[..pos], Some(pos))
            } else {
                // No special source, treat entire input as literal text
                ("", None)
            }
        } else {
            // Keep trailing space in prefix (user controls spacing)
            (&input[..pos], Some(pos))
        }
    } else {
        // No special source character, treat entire input as literal text
        ("", None)
    };

    // If no source specifier found, send the text literally
    let source_start = match source_pos {
        Some(pos) => pos,
        None => {
            let literal = if strip_ansi {
                crate::util::strip_ansi_codes(input)
            } else {
                input.to_string()
            };
            return TfCommandResult::Quote {
                lines: vec![literal],
                disposition,
                world,
                delay_secs,
                recall_opts: None,
                strip_ansi,
            };
        }
    };

    let source_char = input.chars().nth(source_start).unwrap();
    let after_source_char = &input[source_start + 1..];

    // Parse the source: could be quoted ("...") or unquoted (word)
    let (source_value, suffix) = if after_source_char.starts_with('"') {
        // Quoted source: find closing quote
        let content_start = 1; // Skip opening quote
        let mut end = content_start;
        let chars: Vec<char> = after_source_char.chars().collect();
        let mut source_content = String::new();

        while end < chars.len() {
            if chars[end] == '\\' && end + 1 < chars.len() {
                // Escape sequence
                source_content.push(chars[end + 1]);
                end += 2;
            } else if chars[end] == '"' {
                // End of quoted string
                break;
            } else {
                source_content.push(chars[end]);
                end += 1;
            }
        }

        // Calculate byte position for suffix
        let byte_end = after_source_char
            .char_indices()
            .nth(end + 1)
            .map(|(i, _)| i)
            .unwrap_or(after_source_char.len());
        let suffix = after_source_char[byte_end..].trim();

        (source_content, suffix)
    } else if source_char == '`' || source_char == '!' {
        // Unquoted command source: rest of line is the command (commands contain spaces)
        (after_source_char.trim().to_string(), "")
    } else {
        // Unquoted file source: read until whitespace, rest is suffix
        if let Some(space_pos) = after_source_char.find(char::is_whitespace) {
            let source = after_source_char[..space_pos].to_string();
            let suffix = after_source_char[space_pos..].trim();
            (source, suffix)
        } else {
            (after_source_char.to_string(), "")
        }
    };

    // Read lines from the source
    let lines: Vec<String> = match source_char {
        '\'' => {
            // /restrict FILE disables /quote's file-read source (`/help restrict` level
            // 2: "'quote' with '"; verified directly: "QUOTE: files restricted").
            if engine.restrict_level >= super::RestrictLevel::File {
                return TfCommandResult::Error("QUOTE: files restricted".to_string());
            }
            // File source - expand ~ to home directory
            let path = if let Some(rest) = source_value.strip_prefix("~/") {
                if let Some(home) = home::home_dir() {
                    home.join(rest).to_string_lossy().into_owned()
                } else {
                    source_value.clone()
                }
            } else if source_value == "~" {
                home::home_dir()
                    .map(|h| h.to_string_lossy().into_owned())
                    .unwrap_or_else(|| source_value.clone())
            } else {
                source_value.clone()
            };
            match std::fs::File::open(&path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    reader.lines()
                        .map_while(Result::ok)
                        .map(|line| format!("{}{}{}", prefix, line, suffix))
                        .collect()
                }
                Err(e) => return TfCommandResult::Error(format!("Cannot open file '{}': {}", path, e)),
            }
        }
        '`' | '#' => {
            // `<TF_cmd>: capture the command's own output (finding 14) - executed through
            // the engine exactly like a typed command, so every `Success(Some(msg))` line
            // (a multi-line message split apart) becomes one generated line, `Error`
            // aborts the whole /quote, and a `Recall` result (either typed directly as
            // `` `"/recall args" `` or via the shorthand below) is bounced back to the
            // caller with the world's output_lines it needs - cmd_quote itself has none.
            // Native Clay captures (cmd_connections/`/l`, `/fg`, `/ban`) already return
            // real `Success(Some(text))` from `execute_command` (see those functions' own
            // doc comments), so they fall out of this the same way any other command does.
            //
            // #<recall_args>: TF's own shorthand for "capture `/recall <recall_args>`'s
            // output" (`/help quote`'s own "nearly equivalent pairs" list: "/quote <opts>
            // `/recall <args>" == "/quote <opts> #<args>") - prepend "/recall " so it
            // reaches the exact same Recall-result path as spelling it out with a backtick.
            let command_text = if source_char == '#' {
                format!("/recall {}", source_value)
            } else {
                source_value.clone()
            };
            let result = super::parser::execute_command(engine, &command_text);
            match result {
                TfCommandResult::Success(Some(msg)) => {
                    msg.lines()
                        .map(|line| format!("{}{}{}", prefix, line, suffix))
                        .collect()
                }
                TfCommandResult::Success(None) => {
                    vec![]
                }
                TfCommandResult::Error(e) => {
                    return TfCommandResult::Error(format!("Command '{}' failed: {}", command_text, e));
                }
                TfCommandResult::Recall(opts) => {
                    // Recall needs output_lines from the world - pass to caller
                    return TfCommandResult::Quote {
                        lines: vec![],
                        disposition,
                        world,
                        delay_secs,
                        recall_opts: Some((opts, prefix.to_string())),
                        strip_ansi,
                    };
                }
                _ => {
                    // Other result types (SendToMud, ClayCommand, etc.) don't produce capturable output
                    vec![]
                }
            }
        }
        '!' => {
            // /restrict SHELL disables /quote's shell-command source (`/help restrict`
            // level 1: "Disables ... '/quote !'"; verified directly: "QUOTE: <cmd>:
            // Operation not permitted").
            if engine.restrict_level >= super::RestrictLevel::Shell {
                return TfCommandResult::Error(format!("QUOTE: {}: Operation not permitted", source_value));
            }
            // Shell command source
            let mut cmd_builder = Command::new("sh");
            cmd_builder.arg("-c").arg(&source_value)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(ref dir) = engine.current_dir {
                cmd_builder.current_dir(dir);
            }
            match cmd_builder.output() {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let lines: Vec<String> = stdout
                        .lines()
                        .map(|line| format!("{}{}{}", prefix, line, suffix))
                        .collect();
                    if lines.is_empty() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let stderr_trimmed = stderr.trim();
                        if !stderr_trimmed.is_empty() {
                            return TfCommandResult::Error(format!("(no output) stderr: {}", stderr_trimmed));
                        }
                    }
                    lines
                }
                Err(e) => return TfCommandResult::Error(format!("Cannot execute shell command '{}': {}", source_value, e)),
            }
        }
        _ => unreachable!(),
    };

    if lines.is_empty() {
        let detail = if source_char == '!' {
            format!(" [cmd: {}]", source_value)
        } else {
            String::new()
        };
        return TfCommandResult::Success(Some(format!("(no output){}", detail)));
    }

    // If the user didn't explicitly set -d and the prefix starts with /,
    // auto-set disposition to Exec so the resulting lines are executed as commands
    // instead of sent to the MUD (e.g., "/quote /echo !who" should run /echo on each line)
    if !disposition_explicit && !prefix.is_empty() {
        let trimmed_prefix = prefix.trim();
        if trimmed_prefix.starts_with('/') {
            disposition = QuoteDisposition::Exec;
        }
    }

    let lines = if strip_ansi {
        lines.into_iter().map(|l| crate::util::strip_ansi_codes(&l)).collect()
    } else {
        lines
    };

    TfCommandResult::Quote {
        lines,
        disposition,
        world,
        delay_secs,
        recall_opts: None,
        strip_ansi,
    }
}

/// /recall [-<count>] <pattern> - Search output history
/// Examples:
///   /recall *combat*     - Show all lines matching *combat*
///   /recall -10 *combat* - Show last 10 lines matching *combat*
/// Parse a time string like "1:30" or "1:30:45" into seconds
fn parse_time_to_seconds(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        2 => {
            // hours:minutes
            let hours: f64 = parts[0].parse().ok()?;
            let minutes: f64 = parts[1].parse().ok()?;
            Some(hours * 3600.0 + minutes * 60.0)
        }
        3 => {
            // hours:minutes:seconds
            let hours: f64 = parts[0].parse().ok()?;
            let minutes: f64 = parts[1].parse().ok()?;
            let seconds: f64 = parts[2].parse().ok()?;
            Some(hours * 3600.0 + minutes * 60.0 + seconds)
        }
        _ => None,
    }
}

/// Strip surrounding or embedded double-quotes from an option value.
/// TinyFugue allows quoting option arguments (e.g. -t"..." or -w"world name")
/// to protect spaces; the quotes are not part of the value.
fn strip_quotes(s: &str) -> String {
    s.replace('"', "")
}

/// Check if a string looks like a time format (contains colon with digits)
fn looks_like_time(s: &str) -> bool {
    s.contains(':') && s.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.')
}

pub fn cmd_recall(args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Success(Some(
            "Usage: /recall [-wworld] [-ligv] [-t[format]] [-aattrs] [-mstyle] [-An] [-Bn] [-Cn] [#]range [pattern]".to_string()
        ));
    }

    let mut opts = RecallOptions::default();
    let mut remaining = args;
    let mut _saw_hash = false;

    // Parse options (start with -)
    while !remaining.is_empty() {
        let trimmed = remaining.trim_start();
        if trimmed.is_empty() {
            break;
        }

        // Check for # (show line numbers) - must be last option before range
        if trimmed.starts_with('#') && !trimmed.starts_with("#recall") {
            _saw_hash = true;
            opts.show_line_numbers = true;
            remaining = &trimmed[1..];
            break; // # must be last option
        }

        // Check for options starting with -
        if !trimmed.starts_with('-') {
            remaining = trimmed;
            break;
        }

        // Find end of this option (space or end), respecting double-quoted spans.
        // A space inside "..." does not end the token (TF allows -t"fmt with spaces").
        let opt_end = {
            let mut in_quote = false;
            let mut end = trimmed.len(); // default: whole remaining
            for (i, c) in trimmed[1..].char_indices() {
                match c {
                    '"' => in_quote = !in_quote,
                    ' ' | '\t' if !in_quote => { end = i + 1; break; }
                    _ => {}
                }
            }
            end
        };
        let opt = &trimmed[..opt_end];
        remaining = &trimmed[opt_end..];

        // Parse the option
        let opt_chars: Vec<char> = opt[1..].chars().collect();
        if opt_chars.is_empty() {
            // Just "-" alone, this is the start of range like "- -4"
            remaining = trimmed;
            break;
        }

        let mut i = 0;
        while i < opt_chars.len() {
            match opt_chars[i] {
                'w' => {
                    // -w or -wworld
                    if i + 1 < opt_chars.len() {
                        let world: String = opt_chars[i+1..].iter().collect();
                        opts.source = RecallSource::World(strip_quotes(&world));
                        i = opt_chars.len();
                    } else {
                        opts.source = RecallSource::CurrentWorld;
                        i += 1;
                    }
                }
                'l' => {
                    opts.source = RecallSource::Local;
                    i += 1;
                }
                'g' => {
                    opts.source = RecallSource::Global;
                    i += 1;
                }
                'i' => {
                    opts.source = RecallSource::Input;
                    i += 1;
                }
                'v' => {
                    opts.inverse_match = true;
                    i += 1;
                }
                'q' => {
                    opts.quiet = true;
                    i += 1;
                }
                't' => {
                    opts.show_timestamps = true;
                    // Check for optional format; strip quotes so -t"%H:%M:%S" works
                    if i + 1 < opt_chars.len() {
                        let fmt: String = opt_chars[i+1..].iter().collect();
                        opts.timestamp_format = Some(strip_quotes(&fmt));
                        i = opt_chars.len();
                    } else {
                        i += 1;
                    }
                }
                'a' => {
                    // -a<attrs> (/help recall: "suppress specified attributes, e.g. -ag
                    // shows gagged lines") - consumes the rest of this token as the
                    // attribute list, same convention -t/-m/-w already use here. Only 'g'
                    // has a distinct effect (see RecallOptions::suppress_attrs's doc
                    // comment); any other letter is accepted and stored, not applied.
                    let attrs: String = opt_chars[i+1..].iter().collect();
                    opts.show_gagged = attrs.contains('g');
                    opts.suppress_attrs = attrs;
                    i = opt_chars.len();
                }
                'm' => {
                    // -mstyle
                    if i + 1 < opt_chars.len() {
                        let style_raw: String = opt_chars[i+1..].iter().collect();
                        let style = strip_quotes(&style_raw);
                        opts.match_style = match style.to_lowercase().as_str() {
                            "simple" => RecallMatchStyle::Simple,
                            "glob" => RecallMatchStyle::Glob,
                            "regexp" | "regex" => RecallMatchStyle::Regexp,
                            _ => RecallMatchStyle::Glob,
                        };
                        i = opt_chars.len();
                    } else {
                        i += 1;
                    }
                }
                'A' => {
                    // -An context after
                    let num: String = opt_chars[i+1..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num.parse::<usize>() {
                        opts.context_after = n;
                        i += 1 + num.len();
                    } else {
                        i += 1;
                    }
                }
                'B' => {
                    // -Bn context before
                    let num: String = opt_chars[i+1..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num.parse::<usize>() {
                        opts.context_before = n;
                        i += 1 + num.len();
                    } else {
                        i += 1;
                    }
                }
                'C' => {
                    // -Cn context both
                    let num: String = opt_chars[i+1..].iter().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num.parse::<usize>() {
                        opts.context_before = n;
                        opts.context_after = n;
                        i += 1 + num.len();
                    } else {
                        i += 1;
                    }
                }
                'D' => {
                    opts.archive = true;
                    i += 1;
                }
                _ => {
                    // Unknown option or might be a negative range
                    // Check if rest looks like a number (negative range like -4)
                    let rest: String = opt_chars[i..].iter().collect();
                    if rest.chars().all(|c| c.is_ascii_digit()) {
                        // This is a negative range, put it back
                        remaining = trimmed;
                        break;
                    }
                    i += 1;
                }
            }
        }
    }

    // Parse range and pattern
    let remaining = remaining.trim();

    if remaining.is_empty() {
        // No range or pattern, recall all
        opts.range = RecallRange::All;
        return TfCommandResult::Recall(opts);
    }

    // Find where range ends and pattern begins
    // Range formats: /x, x, x-y, -y, x-, or time formats
    let mut range_end = 0;
    let chars: Vec<char> = remaining.chars().collect();

    if chars.first() == Some(&'/') {
        // /x format - last x matching lines
        let num_str: String = chars[1..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num_str.parse::<usize>() {
            opts.range = RecallRange::LastMatching(n);
            range_end = 1 + num_str.len();
        }
    } else if chars.first() == Some(&'-') && chars.len() > 1 {
        // Could be: - -y (with space) or just part of options we already parsed
        // Look for the number after the dash
        let rest: String = chars[1..].iter().collect();
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == ':' || *c == '.').collect();
        if !num_str.is_empty() {
            if looks_like_time(&num_str) {
                if let Some(secs) = parse_time_to_seconds(&num_str) {
                    opts.range = RecallRange::TimePeriod(secs);
                    range_end = 1 + num_str.len();
                }
            } else if let Ok(n) = num_str.parse::<usize>() {
                opts.range = RecallRange::Previous(n);
                range_end = 1 + num_str.len();
            }
        }
    } else {
        // Parse as: x, x-y, x-, or time
        let range_str: String = chars.iter().take_while(|c|
            c.is_ascii_digit() || **c == '-' || **c == ':' || **c == '.'
        ).collect();

        if !range_str.is_empty() {
            range_end = range_str.len();

            if range_str.contains('-') && !range_str.starts_with('-') {
                // x-y or x- format
                let parts: Vec<&str> = range_str.splitn(2, '-').collect();
                if parts.len() == 2 {
                    if parts[1].is_empty() {
                        // x- format (after x)
                        if looks_like_time(parts[0]) {
                            if let Some(secs) = parse_time_to_seconds(parts[0]) {
                                opts.range = RecallRange::TimeRange(secs, 0.0);
                            }
                        } else if let Ok(x) = parts[0].parse::<usize>() {
                            opts.range = RecallRange::After(x);
                        }
                    } else {
                        // x-y format
                        if looks_like_time(parts[0]) && looks_like_time(parts[1]) {
                            if let (Some(start), Some(end)) = (parse_time_to_seconds(parts[0]), parse_time_to_seconds(parts[1])) {
                                opts.range = RecallRange::TimeRange(start, end);
                            }
                        } else if let (Ok(x), Ok(y)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>()) {
                            opts.range = RecallRange::Range(x, y);
                        }
                    }
                }
            } else if looks_like_time(&range_str) {
                // Time period
                if let Some(secs) = parse_time_to_seconds(&range_str) {
                    opts.range = RecallRange::TimePeriod(secs);
                }
            } else if let Ok(n) = range_str.parse::<usize>() {
                // Plain number - last n lines
                opts.range = RecallRange::Last(n);
            }
        }
    }

    // Everything after range is the pattern
    let pattern = remaining[range_end..].trim();
    if !pattern.is_empty() {
        opts.pattern = Some(pattern.to_string());
    }

    TfCommandResult::Recall(opts)
}

/// /gag [pattern] - Add a gag pattern, or list current gags if no pattern given
pub fn cmd_gag(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        // List all gag patterns
        let gags: Vec<_> = engine.macros.iter()
            .filter(|m| m.attributes.gag && m.trigger.is_some())
            .collect();
        if gags.is_empty() {
            return TfCommandResult::Success(Some("No gag patterns defined.".to_string()));
        }
        let mut lines = vec!["Gag patterns:".to_string()];
        for m in &gags {
            if let Some(ref trigger) = m.trigger {
                lines.push(format!("  /gag {}  [{}]", trigger.pattern, m.name));
            }
        }
        return TfCommandResult::Success(Some(lines.join("\n")));
    }

    // Create a macro with gag attribute
    let gag_name = format!("__gag_{}", engine.next_macro_sequence);
    let macro_def = super::TfMacro {
        name: gag_name,
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

    engine.add_macro(macro_def);
    TfCommandResult::Success(Some(format!("Gagging '{}'", pattern)))
}

/// /ungag pattern - Remove a gag pattern
pub fn cmd_ungag(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: /ungag pattern".to_string());
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

/// Expand ~ and search TFPATH/TFLIBDIR for a file
fn resolve_file_path(engine: &TfEngine, filename: &str) -> Option<String> {
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

    // If absolute path, just check if it exists
    if expanded.starts_with('/') {
        let path = Path::new(&expanded);
        if path.exists() {
            return Some(expanded);
        }
        return None;
    }

    // Search order for relative paths (matches real TF):
    // 1. Current directory (from /lcd or actual cwd)
    // 2. If `filename` has no directory component: each directory in the
    //    engine's %TFPATH (colon-separated, TF semantics)
    // 3. If `filename` has no directory component: %TFLIBDIR

    if let Some(ref cd) = engine.current_dir {
        let full_path = format!("{}/{}", cd, expanded);
        if Path::new(&full_path).exists() {
            return Some(full_path);
        }
    } else if let Ok(cwd) = std::env::current_dir() {
        let full_path = cwd.join(&expanded);
        if full_path.exists() {
            return Some(full_path.display().to_string());
        }
    }

    // TF only searches TFPATH/TFLIBDIR for a bare filename (no '/' in it) -
    // a path with a directory component (even a relative one like
    // "sub/file.tf") is never joined onto a library directory.
    if expanded.contains('/') {
        return None;
    }

    let mut search_dirs: Vec<String> = Vec::new();

    // %TFPATH (colon-separated list of directories), read as an engine
    // variable (set with /set, or defaulted from $TFPATH at engine start -
    // see TfEngine::new) rather than the process environment.
    if let Some(tfpath) = engine.get_var("TFPATH").map(|v| v.to_string_value()) {
        for dir in tfpath.split(':') {
            if !dir.is_empty() {
                search_dirs.push(dir.to_string());
            }
        }
    }

    // %TFLIBDIR (searched after TFPATH), same source.
    if let Some(tflibdir) = engine.get_var("TFLIBDIR").map(|v| v.to_string_value()) {
        if !tflibdir.is_empty() {
            search_dirs.push(tflibdir);
        }
    }

    for dir in search_dirs {
        let full_path = format!("{}/{}", dir, expanded);
        if Path::new(&full_path).exists() {
            return Some(full_path);
        }
    }

    None
}

/// Fire the LOADFAIL hook for a failed `/load`/`/require` and build the
/// result `load_file_internal` should return (finding 34). LOADFAIL was
/// already being fired (`hooks::fire_hook`) at both of `load_file_internal`'s
/// error sites, but the call's own `HookOutcome` was discarded and the
/// default error text returned unconditionally - so a matching `-ag` (gag)
/// hook macro, like stdlib.tf's own
///
/// ```text
/// /def -hloadfail -ag ~gagloadfail
/// /eval /load %{TFLIBDIR}/local.tf
/// /undef ~gagloadfail
/// ```
///
/// (guarding the load of an admin-optional file that legitimately doesn't
/// exist on most installs), could never actually suppress anything. Verified
/// directly against real tf 5.0 beta 8: a NON-gagged LOADFAIL hook fires
/// AND the default error message still appears (in that order - error
/// first, then the hook's own output); a GAGGED hook suppresses the default
/// error message entirely (matching TF's general "-ag" hook convention -
/// see `/help hooks`' SEND example) and only the hook's own output (if any -
/// stdlib.tf's own gag macro has an empty body, so nothing else prints)
/// shows. `first_fired_gagged` records only the FIRST macro that matched,
/// per `fire_hook`'s own doc comment - the same "one hook decides" rule
/// already used for CONFAIL/REDEF/SEND elsewhere in this file and parser.rs.
fn fire_loadfail(engine: &mut TfEngine, hook_arg: &str, default_error: String) -> TfCommandResult {
    let outcome = super::hooks::fire_hook(engine, super::TfHookEvent::Loadfail, hook_arg);
    let gagged = outcome.matched_any && outcome.first_fired_gagged == Some(true);
    let hook_text: Vec<String> = outcome.results.into_iter().filter_map(|r| match r {
        TfCommandResult::Success(Some(m)) => Some(m),
        TfCommandResult::Error(e) => Some(e),
        _ => None,
    }).collect();
    if gagged {
        if hook_text.is_empty() {
            TfCommandResult::Success(None)
        } else {
            TfCommandResult::Success(Some(hook_text.join("\n")))
        }
    } else {
        let mut lines = vec![default_error];
        lines.extend(hook_text);
        TfCommandResult::Error(lines.join("\n"))
    }
}

/// Internal load implementation used by both /load and /require.
///
/// `pub(crate)` (not just used by `cmd_load`/`cmd_require`): the Phase 0 TF-script
/// test harness (`src/tf/script_tests.rs`) calls this directly to run a whole
/// fixture file headlessly and inspect the aggregated result, without going
/// through the App/TUI at all. See that module for the harness itself.
pub(crate) fn load_file_internal(engine: &mut TfEngine, filename: &str, quiet: bool) -> TfCommandResult {
    // Resolve the file path
    let resolved = match resolve_file_path(engine, filename) {
        Some(p) => p,
        None => {
            let reason = "Cannot find file";
            return fire_loadfail(
                engine,
                &format!("{} {}", filename, reason),
                format!("{}: {}", reason, filename),
            );
        }
    };

    // Open the file
    let file = match fs::File::open(&resolved) {
        Ok(f) => f,
        Err(e) => {
            return fire_loadfail(
                engine,
                &format!("{} {}", resolved, e),
                format!("Cannot open '{}': {}", resolved, e),
            );
        }
    };

    // Track that we're loading this file (for nested loads)
    engine.loading_files.push(resolved.clone());

    // Show loading message unless quiet
    let mut results = Vec::new();
    if !quiet {
        results.push(TfCommandResult::Success(Some(format!("Loading commands from {}", resolved))));
    }

    let reader = BufReader::new(file);
    let lines_iter = reader.lines().map(|l| l.unwrap_or_default());
    let (line_results, exit_remaining, open_line) = load_lines(engine, lines_iter, &resolved);
    results.extend(line_results);

    // EOF safety net (finding C.3): a file can leave the engine waiting on an
    // /if, /while or /for that never reaches its terminator - historically
    // this happened whenever a single-line block's closing keyword was glued
    // directly to "%;" with no space (now fixed - see
    // control_flow::split_percent_semi), but a script can still open a block
    // it genuinely never closes, or hit some other gap that leaves one open.
    // Either way, /load and /require must not leave the engine permanently
    // stuck waiting for a line that already went by - so reset the state and
    // report where the still-open block started. This does NOT apply to
    // someone interactively typing a multi-line /if a line at a time - that
    // path never calls load_file_internal, so it keeps waiting for /endif as
    // before.
    if !matches!(engine.control_state, super::control_flow::ControlState::None) {
        engine.control_state = super::control_flow::ControlState::None;
        results.push(TfCommandResult::Error(format!(
            "{}:{}: unterminated /if, /while or /for (block opened here)",
            resolved,
            open_line.unwrap_or(0)
        )));
    }

    // Remove this file from the loading stack
    engine.loading_files.pop();

    // Fire LOAD hook (even for early exit)
    let hook_outcome = super::hooks::fire_hook(engine, super::TfHookEvent::Load, &resolved);
    results.extend(hook_outcome.results);

    // Collect errors for detailed output
    let mut errors: Vec<String> = results.iter()
        .filter_map(|r| match r {
            TfCommandResult::Error(e) => Some(e.clone()),
            _ => None,
        })
        .collect();

    if !errors.is_empty() {
        // Finding 22: a file's successfully-echoed lines used to be discarded
        // entirely the moment ANY later line in the same file errored - real TF
        // interleaves output and errors instead. Fold the successful lines the
        // same way the error-free path below does (fold_load_result), and put
        // them ahead of the existing "Loaded ... with N error(s)" summary in
        // ONE TfCommandResult::Error - extending the result type is more
        // invasive than this call site needs, since a Success(Some) text and
        // an Error can't both be returned. Callers that need the two halves
        // back apart (script_tests::run_script) split on the summary line -
        // see is_load_error_summary_line's doc comment there.
        let mut messages: Vec<String> = Vec::new();
        let mut extra_errors: Vec<String> = Vec::new();
        for result in results {
            fold_load_result(engine, result, &mut messages, &mut extra_errors);
        }
        errors.extend(extra_errors);

        let mut output = String::new();
        if !messages.is_empty() {
            output.push_str(&messages.join("\n"));
            output.push('\n');
        }
        output.push_str(&format!("Loaded '{}' with {} error(s)", resolved, errors.len()));
        for error in &errors {
            output.push_str(&format!("\n   {}", error));
        }
        TfCommandResult::Error(output)
    } else if let Some(remaining) = exit_remaining {
        // Early exit, no errors - silent, except when there are still more
        // enclosing /load's to abort (`/exit n` with n > 1): re-raise
        // ExitLoad so the next `load_file_internal` up the call stack
        // catches it the same way this one just did.
        if remaining > 0 {
            TfCommandResult::ExitLoad(remaining)
        } else {
            TfCommandResult::Success(None)
        }
    } else {
        // Aggregate the file's own output instead of discarding it. This used to
        // unconditionally return Success(None) here - every /echo (etc.) a loaded
        // file produced at top level was silently thrown away even though errors
        // were preserved just above. Fold `results` the same way
        // `aggregate_results_with_engine` folds a macro body's results.
        aggregate_load_results(engine, &resolved, results)
    }
}

/// Fold a loaded file's per-line results into one `TfCommandResult`, mirroring
/// `aggregate_results_with_engine`'s treatment of a macro body: join echoed text,
/// queue MUD sends, and resolve a Clay-command pass-through (what /eval currently
/// produces for an already-substituted `/command` - see `cmd_eval`) exactly the
/// way interactive dispatch does (`Command::ActionCommand` in commands.rs: try the
/// TF engine once more, and if that is *also* a pass-through, give up - it must be
/// a genuinely Clay-native command, which a headless file load has no way to run).
/// Only called on the error-free path; `load_file_internal` keeps its own error
/// formatting untouched above.
fn aggregate_load_results(engine: &mut TfEngine, source: &str, results: Vec<TfCommandResult>) -> TfCommandResult {
    let mut messages: Vec<String> = Vec::new();
    let mut extra_errors: Vec<String> = Vec::new();

    for result in results {
        fold_load_result(engine, result, &mut messages, &mut extra_errors);
    }

    if !extra_errors.is_empty() {
        let mut output = format!("Loaded '{}' with {} error(s)", source, extra_errors.len());
        for error in &extra_errors {
            output.push_str(&format!("\n   {}", error));
        }
        TfCommandResult::Error(output)
    } else if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
}

/// See `aggregate_load_results`. Recurses at most once (via the `ClayCommand`
/// arm resolving into another call), matching the "avoid recursion" bound
/// `Command::ActionCommand`'s own TF-engine fallback uses.
fn fold_load_result(
    engine: &mut TfEngine,
    result: TfCommandResult,
    messages: &mut Vec<String>,
    extra_errors: &mut Vec<String>,
) {
    match result {
        TfCommandResult::Success(Some(msg)) => messages.push(msg),
        TfCommandResult::SendToMud(cmd) => {
            engine.pending_commands.push(super::TfCommand {
                command: cmd,
                world: None,
                no_eol: false,
            });
        }
        TfCommandResult::ClayCommand(cmd) if cmd.starts_with('/') => {
            let resolved = super::parser::execute_command(engine, &cmd);
            match resolved {
                // Resolving again produced another pass-through: this is a
                // genuinely Clay-native command (e.g. /quit) that a headless
                // file load has no App to hand it to. Nothing more to do.
                TfCommandResult::ClayCommand(_) => {}
                TfCommandResult::Error(e) => extra_errors.push(e),
                other => fold_load_result(engine, other, messages, extra_errors),
            }
        }
        // ClayCommand with non-'/' text, Quote/Recall/RepeatProcess,
        // Return/ExitLoad/NotTfCommand/UnknownCommand: none of these occur at
        // top level in practice; not meaningful to aggregate.
        _ => {}
    }
}

/// Load TF commands from a string (for tests and embedded scripts)
#[cfg(test)]
pub fn load_from_str(engine: &mut TfEngine, content: &str) -> TfCommandResult {
    let source = "<embedded>";
    let lines_iter = content.lines().map(|l| l.to_string());
    let (results, _exit_remaining, _open_line) = load_lines(engine, lines_iter, source);

    let errors: Vec<String> = results.iter()
        .filter_map(|r| match r {
            TfCommandResult::Error(e) => Some(e.clone()),
            _ => None,
        })
        .collect();

    if !errors.is_empty() {
        let mut output = format!("Loaded with {} error(s)", errors.len());
        for error in &errors {
            output.push_str(&format!("\n   {}", error));
        }
        TfCommandResult::Error(output)
    } else {
        TfCommandResult::Success(None)
    }
}

/// Core line processing shared by file loading and string loading.
/// Returns `(results, exit_remaining, open_line)`. `exit_remaining` is
/// `None` when no `/exit` fired; `Some(k)` when one did - `k` is how many
/// MORE enclosing `/load`s (beyond this one, already absorbed) still need
/// aborting, i.e. `/exit`'s own count minus one (`TfCommandResult::ExitLoad`'s
/// doc comment) - `load_file_internal` re-raises `ExitLoad(k)` when `k > 0`
/// instead of its usual `Success(None)`. `open_line` is the 1-based line
/// number at which `engine.control_state` most recently transitioned from
/// `None` to an open `/if`/`/while`/`/for` (`None` if it never did). Since a
/// nested control-flow construct is accumulated as raw body text inside the
/// outer one rather than as its own `control_state` transition (see
/// `control_flow::process_control_line`), this is exactly the line where
/// whatever block is still open at EOF was opened - used by
/// `load_file_internal`'s finding-C.3 safety net to report where an
/// unterminated block started.
/// Drain `engine.pending_outputs` (the side channel the `echo()` expression
/// function - and hence any macro built on top of it, like real TF stdlib's
/// own "/echo" - uses instead of a direct `Success(Some(text))` return),
/// appending each queued line to `results` in order. Called once per
/// physical line by `load_lines` so an echo()'d line lands immediately after
/// the line that produced it, not batched at the very end of the file (see
/// `load_lines`' own call site comment) - matches how the App's live
/// `commands::process_pending_tf_outputs` treats the exact same queue
/// (`process_attr_codes` on the text, `attrs` still undisplayed - same
/// documented gap as `/echo`'s own `-a<attrs>`).
fn drain_pending_echo_outputs(engine: &mut super::TfEngine, results: &mut Vec<TfCommandResult>) {
    for output in engine.pending_outputs.drain(..) {
        results.push(TfCommandResult::Success(Some(super::parser::process_attr_codes(&output.text))));
    }
}

fn load_lines(engine: &mut super::TfEngine, lines: impl Iterator<Item = String>, source: &str) -> (Vec<TfCommandResult>, Option<u32>, Option<usize>) {
    let mut results = Vec::new();
    let mut line_num = 0;
    let mut continued_line = String::new();
    let mut exit_remaining: Option<u32> = None;
    let mut open_line: Option<usize> = None;

    // Track the line currently being processed, in lockstep with `load_file_internal`'s
    // own `loading_files` push/pop around this call - see `TfEngine::diag_location_prefix`
    // (finding 25) for what reads this. Pushed/popped here rather than by the caller since
    // this is the only place `line_num` actually changes.
    engine.loading_lines.push(0);

    for line in lines {
        line_num += 1;
        if let Some(last) = engine.loading_lines.last_mut() {
            *last = line_num;
        }

        // Strip leading whitespace
        let trimmed = line.trim_start();

        // Check if this is a comment line (starts with ; or is just # or # followed by space)
        let is_comment = trimmed.starts_with(';')
            || trimmed == "#"
            || trimmed.starts_with("# ");

        // If this is a comment line, skip it entirely (even during line continuation)
        // The continuation just continues to the next non-comment line
        if is_comment {
            // If the comment ends with \, it's still a continuation but we skip the comment content
            if trimmed.ends_with('\\') && !trimmed.ends_with("%\\") {
                // Don't append the comment, but continue looking for more lines
                continue;
            }
            // Regular comment - just skip
            continue;
        }

        // Handle line continuation
        if trimmed.ends_with('\\') && !trimmed.ends_with("%\\") {
            // Line continues - append without the backslash
            continued_line.push_str(&trimmed[..trimmed.len() - 1]);
            continue;
        }

        // Build the complete line
        let complete_line = if !continued_line.is_empty() {
            let mut full = std::mem::take(&mut continued_line);
            full.push_str(trimmed);
            // Replace %\ with just \ (escaped backslash for line continuation)
            // Note: %; is NOT replaced here - it's handled during macro execution
            full.replace("%\\", "\\")
        } else {
            trimmed.replace("%\\", "\\")
        };

        let trimmed = complete_line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Execute the line
        let was_none = matches!(engine.control_state, super::control_flow::ControlState::None);
        let result = if trimmed.starts_with('/') {
            super::parser::execute_command(engine, trimmed)
        } else {
            // Non-command lines are sent to the MUD in TF, but we ignore them in Clay
            continue;
        };
        if was_none && !matches!(engine.control_state, super::control_flow::ControlState::None) {
            open_line = Some(line_num);
        }

        match &result {
            TfCommandResult::Error(e) => {
                results.push(TfCommandResult::Error(format!("{}:{}: {}", source, line_num, e)));
            }
            TfCommandResult::ExitLoad(n) => {
                // /exit was called - stop loading. This level absorbs one of
                // its `n` enclosing /load's; whatever's left (n - 1) still
                // needs aborting further out (see this function's own doc
                // comment and `TfCommandResult::ExitLoad`'s).
                exit_remaining = Some(n.saturating_sub(1));
                drain_pending_echo_outputs(engine, &mut results);
                break;
            }
            _ => results.push(result),
        }

        // Drain whatever the echo() expression function queued while
        // evaluating THIS line, immediately - not once at the very end after
        // the whole file loads. A user-defined macro shadows a same-named
        // builtin (finding 16), and real TinyFugue's own stdlib.tf defines
        // "/echo" as exactly such a macro (a thin wrapper around the echo()
        // function, `/return echo({*}, ...)`) - so any script that
        // `/require`s stdlib.tf routes every "/echo" through this side
        // channel instead of `cmd_echo`'s direct `Success(Some(text))`.
        // Draining only after the whole file (as `script_tests::run_script`
        // used to, and as this function itself used to not do at all) puts
        // every echo()'d line after the file's own last direct result
        // instead of interleaved in the order they actually ran - silently
        // reordering any file with more than one such line.
        drain_pending_echo_outputs(engine, &mut results);
    }

    engine.loading_lines.pop();

    (results, exit_remaining, open_line)
}

/// /load [-q] filename - Load and execute a TF script file
///
/// Options:
///   -q  Quiet mode - don't echo "Loading commands from..." message
///
/// The file may contain TF commands starting with /.
/// Blank lines and lines beginning with ';' or single '#' are ignored.
/// Lines ending in '\' continue on the next line (use %\ for literal backslash).
/// Use /exit to abort loading early.
pub fn cmd_load(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if engine.restrict_level >= super::RestrictLevel::File {
        return TfCommandResult::Error("LOAD: restricted".to_string());
    }

    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: /load [-q] filename".to_string());
    }

    // Parse options
    let mut quiet = false;
    let mut filename = args;

    if let Some(rest) = args.strip_prefix("-q") {
        quiet = true;
        filename = rest.trim_start();
        if filename.is_empty() {
            return TfCommandResult::Error("Usage: /load [-q] filename".to_string());
        }
    }

    load_file_internal(engine, filename, quiet)
}

/// /require [-q] filename - Load file only if not already loaded via /loaded
///
/// Same as /load, but if the file has already registered a token via /loaded,
/// the file will not be read again (but the LOAD hook is still called).
pub fn cmd_require(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if engine.restrict_level >= super::RestrictLevel::File {
        return TfCommandResult::Error("LOAD: restricted".to_string());
    }

    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: /require [-q] filename".to_string());
    }

    // Parse options
    let mut quiet = false;
    let mut filename = args;

    if let Some(rest) = args.strip_prefix("-q") {
        quiet = true;
        filename = rest.trim_start();
        if filename.is_empty() {
            return TfCommandResult::Error("Usage: /require [-q] filename".to_string());
        }
    }

    // Note: We don't check loaded_tokens here - that's done by /loaded inside the file.
    // /require just calls /load; the difference is that files designed for /require
    // will have /loaded as their first command, which will abort if already loaded.
    load_file_internal(engine, filename, quiet)
}

/// /loaded token - Mark this file as loaded (for use with /require)
///
/// Should be the first command in a file designed for /require.
/// If the token has already been registered, aborts the file load and returns success.
/// Token should be unique (file's full path is recommended).
pub fn cmd_loaded(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let token = args.trim();

    if token.is_empty() {
        return TfCommandResult::Error("Usage: /loaded token".to_string());
    }

    // Check if already loaded
    if engine.loaded_tokens.contains(token) {
        // Already loaded - abort this file load
        return TfCommandResult::ExitLoad(1);
    }

    // Register the token
    engine.loaded_tokens.insert(token.to_string());
    TfCommandResult::Success(None)
}

/// /exit [n] - Abort loading the current file early (`/help exit`).
///
/// "When called directly or indirectly during a /load, /exit aborts
/// execution of all enclosing macro bodies" - the macro-body half is
/// `execute_macro_with_context`'s own `TfCommandResult::ExitLoad(_)` check,
/// not this function - "and aborts <n> (default 1) enclosing /load's."
/// `n` floors at 1 (`/exit 0` and `/exit -5` both behave like a bare
/// `/exit` - verified directly against real tf); `load_file_internal`
/// decrements it by one per enclosing file as it propagates outward.
///
/// "When called outside of a /load, /exit has no effect" (verified directly
/// against real tf - NOT "equivalent to /quit", despite this function's old
/// doc comment, which real tf's own help flatly contradicts).
pub fn cmd_exit(engine: &TfEngine, args: &str) -> TfCommandResult {
    if engine.loading_files.is_empty() {
        // Not loading a file - /exit has no effect (per TF spec)
        TfCommandResult::Success(None)
    } else {
        let n = args.trim().parse::<i64>().unwrap_or(1).max(1) as u32;
        TfCommandResult::ExitLoad(n)
    }
}

/// /hilite [pattern [= response]] - Hilite matching text
/// With no args: sets %{hilite} to 1.
/// With args: creates a macro equivalent to /def -ah -t"pattern" [= response].
pub fn cmd_hilite(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        // No args: enable hilite flag
        engine.set_global("hilite", super::TfValue::Integer(1));
        return TfCommandResult::Success(Some("Hilite enabled.".to_string()));
    }

    // Parse: pattern [= response]
    let (pattern, body) = if let Some(eq_pos) = args.find('=') {
        let before = args[..eq_pos].trim_end();
        let after = args[eq_pos + 1..].trim_start();
        (before.to_string(), after.to_string())
    } else {
        (args.to_string(), String::new())
    };

    // Get hiliteattr from variable (default "B" = bold)
    let hiliteattr = engine.get_var("hiliteattr")
        .map(|v| v.to_string_value())
        .unwrap_or_else(|| "B".to_string());

    // Parse the attribute string to get TfAttributes
    let attrs = super::macros::parse_hiliteattr(&hiliteattr);

    let hilite_name = format!("__hilite_{}", engine.next_macro_sequence);
    let macro_def = super::TfMacro {
        name: hilite_name,
        body,
        trigger: Some(super::TfTrigger {
            pattern: pattern.clone(),
            match_mode: super::TfMatchMode::Glob,
            compiled: regex::Regex::new(&super::macros::glob_to_regex(&pattern)).ok(),
        }),
        attributes: attrs,
        ..Default::default()
    };

    let macro_num = engine.next_macro_sequence;
    engine.add_macro(macro_def);
    TfCommandResult::Success(Some(format!("{}", macro_num)))
}

/// /nohilite pattern - Remove hilite macro matching pattern
pub fn cmd_nohilite(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        // No args: disable hilite flag
        engine.set_global("hilite", super::TfValue::Integer(0));
        return TfCommandResult::Success(Some("Hilite disabled.".to_string()));
    }

    // Remove hilite macros matching the pattern
    let before = engine.macros.len();
    engine.macros.retain(|m| {
        if let Some(ref trigger) = m.trigger {
            // Remove if it's a hilite macro with matching pattern
            if (m.attributes.hilite.is_some() || m.attributes.bold)
                && trigger.pattern == pattern
            {
                return false;
            }
        }
        true
    });
    let removed = before - engine.macros.len();

    if removed > 0 {
        TfCommandResult::Success(Some(format!("Removed {} hilite(s) matching '{}'", removed, pattern)))
    } else {
        TfCommandResult::Success(Some(format!("No hilite found matching '{}'", pattern)))
    }
}

/// /partial regexp - Hilite matching portion of lines (partial hilite)
/// Equivalent to /def -Ph -F -tregexp
pub fn cmd_partial(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();

    if pattern.is_empty() {
        return TfCommandResult::Error("Usage: /partial regexp".to_string());
    }

    // Get hiliteattr from variable (default "B" = bold)
    let hiliteattr = engine.get_var("hiliteattr")
        .map(|v| v.to_string_value())
        .unwrap_or_else(|| "B".to_string());

    let attrs = super::macros::parse_hiliteattr(&hiliteattr);

    let partial_name = format!("__partial_{}", engine.next_macro_sequence);
    let macro_def = super::TfMacro {
        name: partial_name,
        body: String::new(),
        trigger: Some(super::TfTrigger {
            pattern: pattern.to_string(),
            match_mode: super::TfMatchMode::Regexp,
            compiled: regex::Regex::new(pattern).ok(),
        }),
        attributes: attrs,
        fall_through: true,
        partial_hilite: true,
        ..Default::default()
    };

    let macro_num = engine.next_macro_sequence;
    engine.add_macro(macro_def);
    TfCommandResult::Success(Some(format!("{}", macro_num)))
}

/// /export variable - Make a global variable an environment variable
pub fn cmd_export(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let var_name = args.trim();

    if var_name.is_empty() {
        return TfCommandResult::Error("Usage: /export variable".to_string());
    }

    if let Some(value) = engine.get_var(var_name) {
        let val_str = value.to_string_value();
        std::env::set_var(var_name, &val_str);
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Error(format!("Variable '{}' not found.", var_name))
    }
}

/// /save [-a] <file> [<list-options>] - Save matching macros to `<file>`,
/// one per line in reloadable `/def` form (`/help save`: "the <list-options>
/// are the same as those in the /list command" - `macros::MacroFilter`, Job
/// 7's grammar; "Invisible macros will not be saved unless -i is
/// specified" - `MacroFilter`'s own default `InvisibleMode`). `-a` appends;
/// otherwise `<file>` is overwritten. Real tf's own `/save` writes ONLY
/// macros (no variables, no keybinding table) - verified directly against
/// real tf; the previous Clay implementation also dumped every variable and
/// every raw `/bind` entry unconditionally, ignoring its own arguments
/// entirely (no filter support, no `-a`), which is why a `/save` round-trip
/// through `/load` used to duplicate content instead of reproducing it.
///
/// Message wording matches real tf directly (verified): "Writing macros to
/// <file>" / "Appending macros to <file>" - not the previous "Saved to
/// '<file>'".
pub fn cmd_save(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if engine.restrict_level >= super::RestrictLevel::File {
        return TfCommandResult::Error("SAVE: restricted".to_string());
    }
    let mut rest = args.trim_start();

    // "-a" must be its own token (`/help save`: "/SAVE [-a] <file>") - unlike
    // <list-options>' own `-a<attrs>` (a completely different option, /def's
    // attribute flag), which only ever appears further along.
    let append = if let Some(after) = rest.strip_prefix("-a") {
        if after.is_empty() || after.starts_with(char::is_whitespace) {
            rest = after.trim_start();
            true
        } else {
            false
        }
    } else {
        false
    };

    let (filename, options) = match rest.find(char::is_whitespace) {
        Some(pos) => (&rest[..pos], rest[pos..].trim_start()),
        None => (rest, ""),
    };
    if filename.is_empty() {
        return TfCommandResult::Error("Usage: /save [-a] <file> [<list-options>]".to_string());
    }

    let default_style = super::macros::default_matching_style(engine);
    let filter = match super::macros::MacroFilter::parse(options, super::macros::FilterKind::List, default_style) {
        Ok(f) => f,
        Err(e) => return TfCommandResult::Error(e),
    };

    let mut matched: Vec<&super::TfMacro> = engine.macros.iter().filter(|m| filter.matches(m)).collect();
    if filter.sort {
        matched.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let mut last_number = 0u32;
    let mut output = String::new();
    for macro_def in &matched {
        output.push_str(&super::macros::format_def_line(macro_def));
        output.push('\n');
        last_number = macro_def.sequence_number;
    }
    engine.set_global("?", super::TfValue::Integer(last_number as i64));

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

    let write_result = if append {
        use std::io::Write as _;
        fs::OpenOptions::new().create(true).append(true).open(&expanded)
            .and_then(|mut f| f.write_all(output.as_bytes()))
    } else {
        fs::write(&expanded, &output)
    };

    match write_result {
        Ok(()) => {
            let verb = if append { "Appending" } else { "Writing" };
            TfCommandResult::Success(Some(format!("{} macros to {}", verb, expanded)))
        }
        Err(e) => TfCommandResult::Error(format!("Cannot write '{}': {}", expanded, e)),
    }
}

/// /log [-w[<world>]] [-i] [-l] [-g] [OFF|ON|<file>] - per-world output logging.
///
/// Resolving this needs `&mut App` (per-world settings, enumerating every world for a bare
/// `/log`), which this engine-only function does not have - always bounces to Clay's own
/// `/log` (`Command::Log`, parsed by `parse_log_command` and executed by
/// `commands::execute_log_command`), the same way `/send`'s flag forms do. See
/// `execute_log_command`'s doc comment for exactly what maps onto Clay's simpler
/// one-log-per-world model and what's accepted but not distinct.
pub fn cmd_log(args: &str) -> TfCommandResult {
    TfCommandResult::ClayCommand(format!("/log {}", args.trim()))
}

/// Parse a TF time string into a Duration
/// Formats: "S" (seconds, supports decimals), "M:S", "H:M:S"
/// Leading '-' is stripped (TF convention for repeat intervals)
pub fn parse_tf_time(s: &str) -> Option<Duration> {
    let s = s.strip_prefix('-').unwrap_or(s);
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        1 => {
            // Just seconds (supports decimals)
            let secs: f64 = parts[0].parse().ok()?;
            if secs < 0.0 { return None; }
            Some(Duration::from_secs_f64(secs))
        }
        2 => {
            // M:S
            let mins: f64 = parts[0].parse().ok()?;
            let secs: f64 = parts[1].parse().ok()?;
            if mins < 0.0 || secs < 0.0 { return None; }
            Some(Duration::from_secs_f64(mins * 60.0 + secs))
        }
        3 => {
            // H:M:S
            let hours: f64 = parts[0].parse().ok()?;
            let mins: f64 = parts[1].parse().ok()?;
            let secs: f64 = parts[2].parse().ok()?;
            if hours < 0.0 || mins < 0.0 || secs < 0.0 { return None; }
            Some(Duration::from_secs_f64(hours * 3600.0 + mins * 60.0 + secs))
        }
        _ => None,
    }
}

/// /repeat [-w[world]] [-n] {[-time]|-S|-P} count command
pub fn cmd_repeat(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error(
            "Usage: /repeat [-w[world]] [-n] {[-time]|-S|-P} count command".to_string()
        );
    }

    let mut world: Option<String> = None;
    let mut no_initial_delay = false;
    let mut synchronous = false;
    let mut on_prompt = false;
    let mut interval: Option<Duration> = None;
    let mut priority: i32 = 0;
    let mut remaining = args;

    // Parse flags
    loop {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        if remaining.starts_with("-w") {
            // -w or -wworld
            let rest = &remaining[2..];
            if rest.starts_with(char::is_whitespace) || rest.is_empty() {
                // -w with no world name — current world
                world = Some(String::new());
                remaining = rest.trim_start();
            } else {
                // -wworld
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                world = Some(rest[..end].to_string());
                remaining = &rest[end..];
            }
            continue;
        }

        if remaining.starts_with("-n") && (remaining.len() == 2 || remaining[2..].starts_with(char::is_whitespace)) {
            no_initial_delay = true;
            remaining = &remaining[2..];
            continue;
        }

        if remaining.starts_with("-S") && (remaining.len() == 2 || remaining[2..].starts_with(char::is_whitespace)) {
            synchronous = true;
            remaining = &remaining[2..];
            continue;
        }

        if remaining.starts_with("-P") && (remaining.len() == 2 || remaining[2..].starts_with(char::is_whitespace)) {
            on_prompt = true;
            remaining = &remaining[2..];
            continue;
        }

        // Check for -p priority
        if remaining.starts_with("-p") {
            let rest = &remaining[2..];
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            if let Ok(p) = rest[..end].parse::<i32>() {
                priority = p;
                remaining = &rest[end..];
                continue;
            }
        }

        // Check for -time (e.g. -30, -0:30, -1:0:0)
        if remaining.starts_with('-') {
            let rest = &remaining[1..];
            let time_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let time_str = &rest[..time_end];
            // Must start with a digit to be a time value (not another flag)
            if time_str.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                if let Some(dur) = parse_tf_time(time_str) {
                    interval = Some(dur);
                    remaining = &rest[time_end..];
                    continue;
                }
            }
        }

        break;
    }

    remaining = remaining.trim_start();

    // Parse count: integer or "i" for infinite
    let count_end = remaining.find(char::is_whitespace).unwrap_or(remaining.len());
    let count_str = &remaining[..count_end];
    let count: Option<u32> = if count_str.eq_ignore_ascii_case("i") {
        None // infinite
    } else if let Ok(n) = count_str.parse::<u32>() {
        if n == 0 {
            return TfCommandResult::Error("/repeat: count must be > 0".to_string());
        }
        Some(n)
    } else {
        return TfCommandResult::Error(format!("/repeat: invalid count '{}'", count_str));
    };
    remaining = remaining[count_end..].trim_start();

    // Parse command (rest of args)
    let command = remaining.to_string();
    if command.is_empty() {
        return TfCommandResult::Error("/repeat: no command specified".to_string());
    }

    // Synchronous mode: execute all iterations immediately
    if synchronous {
        let iterations = count.unwrap_or(1);
        let mut last_result = TfCommandResult::Success(None);
        for _ in 0..iterations {
            last_result = engine.execute(&command);
        }
        return last_result;
    }

    // Need an interval for async mode
    let interval = interval.unwrap_or(Duration::from_secs(1));

    // Create process
    let id = engine.next_process_id;
    engine.next_process_id += 1;

    // Always run first iteration immediately, then wait interval between subsequent runs
    // The -n flag is now a no-op (kept for backwards compatibility)
    let _ = no_initial_delay;
    let next_run = Instant::now();

    // If no -w was specified, capture the current world so the repeat
    // stays bound to the world it was invoked on.
    let world = world.or_else(|| engine.current_world.clone());

    let process = TfProcess {
        id,
        command,
        interval,
        count,
        remaining: count,
        next_run,
        world,
        synchronous: false,
        on_prompt,
        priority,
        kind: super::ProcessKind::Repeat,
    };

    TfCommandResult::RepeatProcess(process)
}

/// /ps [-srq] [-w[<world>]] [<pid>] - List information about background
/// `/quote`/`/repeat` processes, or one specific `<pid>` (`/help ps`).
/// Clay's own PID/INTERVAL/REMAINING/COMMAND table predates this job and is
/// kept as-is - real tf's PID/NEXT/T/D/WORLD/PTIME/COUNT/COMMAND columns
/// don't all map onto `TfProcess` (there's no tracked per-process /quote
/// line disposition, for instance - plan Job 14c: "implement what maps onto
/// Clay's TfProcess fields, accept the rest"), so only the documented
/// FILTERS are new here: `-s` (PIDs only, one line, space-separated - no
/// header), `-r`/`-q` (`ProcessKind` - a real /repeat vs. a delayed /quote
/// line), `-w[<world>]` (bare `-w` means the current world, same as
/// `cmd_histsize`'s own -w; validated against `world_info_cache`), and a
/// trailing `<pid>` to show just one process. A totally-empty process list
/// keeps Clay's existing friendly "No background processes." message; a
/// FILTERED-to-empty result instead shows the (possibly headerless, for -s)
/// empty table, matching real tf's own behavior of always showing the
/// header for a plain `/ps` with none running (verified directly against
/// real tf).
pub fn cmd_ps(engine: &TfEngine, args: &str) -> TfCommandResult {
    let mut remaining = args.trim();
    let mut short = false;
    let mut repeats_only = false;
    let mut quotes_only = false;
    let mut world_arg: Option<Option<String>> = None; // Some(None) = bare -w; Some(Some(name)) = -w<name>

    while let Some(rest) = remaining.strip_prefix('-') {
        if rest.is_empty() {
            break;
        }
        if let Some(after_w) = rest.strip_prefix('w') {
            let token_end = after_w.find(char::is_whitespace).unwrap_or(after_w.len());
            let (value, tail) = after_w.split_at(token_end);
            world_arg = Some(if value.is_empty() { None } else { Some(value.to_string()) });
            remaining = tail.trim_start();
            continue;
        }
        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, tail) = rest.split_at(token_end);
        if !token.is_empty() && token.chars().all(|c| "srq".contains(c)) {
            for c in token.chars() {
                match c {
                    's' => short = true,
                    'r' => repeats_only = true,
                    'q' => quotes_only = true,
                    _ => unreachable!("filtered to srq above"),
                }
            }
            remaining = tail.trim_start();
            continue;
        }
        break;
    }

    let world_filter = match world_arg {
        Some(name) => {
            let resolved = name.or_else(|| engine.current_world.clone());
            match resolved {
                Some(name) if engine.world_info_cache.iter().any(|w| w.name.eq_ignore_ascii_case(&name)) => Some(name),
                Some(name) => return TfCommandResult::Error(format!("PS -w: No world {}", name)),
                None => return TfCommandResult::Error("PS -w: No world".to_string()),
            }
        }
        None => None,
    };

    let pid_filter: Option<u32> = if remaining.is_empty() {
        None
    } else {
        match remaining.parse::<u32>() {
            Ok(pid) => Some(pid),
            Err(_) => return TfCommandResult::Error(format!("Invalid pid: {}", remaining)),
        }
    };

    if engine.processes.is_empty() {
        return TfCommandResult::Success(Some("No background processes.".to_string()));
    }

    let procs: Vec<&TfProcess> = engine.processes.iter()
        .filter(|p| !repeats_only || p.kind == super::ProcessKind::Repeat)
        .filter(|p| !quotes_only || p.kind == super::ProcessKind::Quote)
        .filter(|p| world_filter.as_deref().map_or(true, |w| p.world.as_deref().is_some_and(|pw| pw.eq_ignore_ascii_case(w))))
        .filter(|p| pid_filter.map_or(true, |pid| p.id == pid))
        .collect();

    if short {
        if procs.is_empty() {
            return TfCommandResult::Success(None);
        }
        let ids: Vec<String> = procs.iter().map(|p| p.id.to_string()).collect();
        return TfCommandResult::Success(Some(ids.join(" ")));
    }

    let mut lines = vec![format!("{:<6} {:<12} {:<10} {}", "PID", "INTERVAL", "REMAINING", "COMMAND")];
    for p in procs {
        let interval_str = format_duration(p.interval);
        let remaining_str = match p.remaining {
            Some(r) => r.to_string(),
            None => "inf".to_string(),
        };
        lines.push(format!("{:<6} {:<12} {:<10} {}", p.id, interval_str, remaining_str, p.command));
    }
    TfCommandResult::Success(Some(lines.join("\n")))
}

/// Format a Duration for display
fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs_f64();
    if total_secs < 60.0 {
        if total_secs == total_secs.floor() {
            format!("{}s", total_secs as u64)
        } else {
            format!("{:.1}s", total_secs)
        }
    } else if total_secs < 3600.0 {
        let mins = (total_secs / 60.0) as u64;
        let secs = (total_secs % 60.0) as u64;
        format!("{}m{}s", mins, secs)
    } else {
        let hours = (total_secs / 3600.0) as u64;
        let mins = ((total_secs % 3600.0) / 60.0) as u64;
        let secs = (total_secs % 60.0) as u64;
        format!("{}h{}m{}s", hours, mins, secs)
    }
}

/// /kill <pid>... - For each `<pid>` given, terminate the corresponding
/// process (`/help kill`). Each pid is processed independently - one bad
/// pid doesn't stop the rest (same pattern as /undef and /undefn: `/kill 1
/// nosuch 2` still kills both 1 and 2). Silent on success (matches real
/// tf's own behaviour - verified directly: neither pid is echoed back); a
/// missing or non-numeric pid prints its own `% [loc]KILL: ...` diagnostic
/// (`TfEngine::format_diag`, finding 25's convention), real tf's own
/// wording ("no process N" / "invalid or missing numeric argument",
/// matching `/undefn`'s own two failure messages).
pub fn cmd_kill(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /kill <pid>...".to_string());
    }

    let mut messages = Vec::new();
    for tok in args.split_whitespace() {
        match tok.parse::<u32>() {
            Ok(pid) => {
                let before = engine.processes.len();
                engine.processes.retain(|p| p.id != pid);
                if engine.processes.len() < before {
                    // KILL hook: "pid (process ends)" - see /help hooks.
                    let outcome = super::hooks::fire_hook(engine, super::TfHookEvent::Kill, &pid.to_string());
                    for r in outcome.results {
                        if let TfCommandResult::Success(Some(m)) = r {
                            messages.push(m);
                        }
                    }
                } else {
                    messages.push(engine.format_diag(&format!("KILL: no process {}", pid)));
                }
            }
            Err(_) => {
                messages.push(engine.format_diag("KILL: invalid or missing numeric argument"));
            }
        }
    }

    if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
}

/// Convert glob pattern to regex (re-exported from macros for use here)
pub use super::macros::glob_to_regex;

// =============================================================================
// Tier 1: Simple commands
// =============================================================================

/// /toggle var - Toggle a variable between 0 and 1. Silent on success - real tf's own
/// `/toggle` (`/help toggle`) never echoes the new value (verified directly: only a
/// following `/echo %var` prints anything); Clay used to echo "name=newval" here, which
/// duplicated output for any script that also reads the variable back afterward (Job 15).
pub fn cmd_toggle(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.trim();
    if name.is_empty() {
        return TfCommandResult::Error("Usage: /toggle varname".to_string());
    }

    let current = engine.get_var(name)
        .map(|v| v.to_int().unwrap_or(0))
        .unwrap_or(0);

    let new_val = if current == 0 { 1 } else { 0 };
    engine.set_global(name, super::TfValue::Integer(new_val));
    TfCommandResult::Success(None)
}

/// /return [expr] - Stop macro execution, set %? to expr result
pub fn cmd_return(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Return("1".to_string());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => TfCommandResult::Return(value.to_string_value()),
        Err(e) => TfCommandResult::Error(format!("Expression error: {}", e)),
    }
}

/// /result [expr] - like /return, but when the enclosing macro was called
/// as a command (not as a function), the value is also echoed to tfout -
/// see macros::execute_macro_with_context. With no expression the value is
/// the empty string (TF: "If the expression is omitted, the return value
/// of the macro is the empty string" - unlike /return, whose no-argument
/// default is preserved above as-is per this job's brief).
pub fn cmd_result(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Result(String::new());
    }

    match super::expressions::evaluate(engine, args) {
        Ok(value) => TfCommandResult::Result(value.to_string_value()),
        Err(e) => TfCommandResult::Error(format!("Expression error: {}", e)),
    }
}

/// /suspend - Suspend the process (Ctrl+Z)
pub fn cmd_suspend() -> TfCommandResult {
    TfCommandResult::ClayCommand("/suspend".to_string())
}

/// /dokey name - Execute an edit key function by name
///
/// TF's full 35-name vocabulary (see `tf-help`'s `/dokey` table, and finding A / plan step
/// P1.11 in the TF-parity plan). Sets `%?` to TF's documented return value where that's
/// cheap to compute from the cached `KeyboardBufferState` (movement/deletion -> new cursor
/// position - deleting *forward* of the cursor never moves it, so DCH/DWORD/DEOL return the
/// unchanged position); everything else (recall, search, world switch, scrolling, redraw,
/// pause, ...) sets it to 1, same as `/not`'s pattern of `set_global("?", ...)` alongside
/// `Success(None)`.
///
/// Names that only need the cached buffer/cursor (BSPC, DLINE, LEFT, RIGHT, HOME, END, DCH,
/// WLEFT, WRIGHT) are performed synchronously via the existing `Goto`/`Delete`/`WordLeft`/
/// `WordRight` ops. Everything else needs real App/World state (input history, scrollback,
/// the world list, ...) that the engine can't reach, so it's deferred via
/// `PendingKeyboardOp::Dokey` to `App::process_pending_keyboard_ops` - see that function's
/// doc comment for how the resulting `KeyAction`s (NEWLINE -> `SendCommand`, REDRAW ->
/// `Redraw`, SOCKETB/F -> `SwitchedWorld`) are handled at the call site.
pub fn cmd_dokey(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    use super::{DokeyName, PendingKeyboardOp, TfValue};

    let name = args.trim().to_uppercase();
    if name.is_empty() {
        return TfCommandResult::Error("Usage: /dokey keyname".to_string());
    }

    // Cheap, synchronous names: cmd_dokey already has everything it needs in
    // `engine.keyboard_state`, so these don't need a PendingKeyboardOp::Dokey round trip.
    match name.as_str() {
        "BSPC" | "BACKSPACE" => {
            let pos = engine.keyboard_state.cursor_position;
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Delete(-1));
            engine.set_global("?", TfValue::Integer(pos.saturating_sub(1) as i64));
            return TfCommandResult::Success(None);
        }
        "DLINE" | "DELINE" => {
            // Delete entire line
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Goto(0));
            let len = engine.keyboard_state.buffer.len() as i32;
            if len > 0 {
                engine.pending_keyboard_ops.push(PendingKeyboardOp::Delete(len));
            }
            engine.set_global("?", TfValue::Integer(0));
            return TfCommandResult::Success(None);
        }
        "LEFT" => {
            let pos = engine.keyboard_state.cursor_position;
            if pos > 0 {
                engine.pending_keyboard_ops.push(PendingKeyboardOp::Goto(pos - 1));
            }
            engine.set_global("?", TfValue::Integer(pos.saturating_sub(1) as i64));
            return TfCommandResult::Success(None);
        }
        "RIGHT" => {
            let pos = engine.keyboard_state.cursor_position;
            let len = engine.keyboard_state.buffer.chars().count();
            let new_pos = if pos < len {
                engine.pending_keyboard_ops.push(PendingKeyboardOp::Goto(pos + 1));
                pos + 1
            } else {
                pos
            };
            engine.set_global("?", TfValue::Integer(new_pos as i64));
            return TfCommandResult::Success(None);
        }
        "HOME" => {
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Goto(0));
            engine.set_global("?", TfValue::Integer(0));
            return TfCommandResult::Success(None);
        }
        "END" => {
            let len = engine.keyboard_state.buffer.chars().count();
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Goto(len));
            engine.set_global("?", TfValue::Integer(len as i64));
            return TfCommandResult::Success(None);
        }
        "DCH" | "DELETE" => {
            let pos = engine.keyboard_state.cursor_position;
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Delete(1));
            engine.set_global("?", TfValue::Integer(pos as i64));
            return TfCommandResult::Success(None);
        }
        "WLEFT" => {
            let chars: Vec<char> = engine.keyboard_state.buffer.chars().collect();
            let mut pos = engine.keyboard_state.cursor_position;
            while pos > 0 && !chars[pos - 1].is_alphanumeric() {
                pos -= 1;
            }
            while pos > 0 && chars[pos - 1].is_alphanumeric() {
                pos -= 1;
            }
            engine.pending_keyboard_ops.push(PendingKeyboardOp::WordLeft);
            engine.set_global("?", TfValue::Integer(pos as i64));
            return TfCommandResult::Success(None);
        }
        "WRIGHT" => {
            let chars: Vec<char> = engine.keyboard_state.buffer.chars().collect();
            let mut pos = engine.keyboard_state.cursor_position;
            while pos < chars.len() && chars[pos].is_alphanumeric() {
                pos += 1;
            }
            while pos < chars.len() && !chars[pos].is_alphanumeric() {
                pos += 1;
            }
            engine.pending_keyboard_ops.push(PendingKeyboardOp::WordRight);
            engine.set_global("?", TfValue::Integer(pos as i64));
            return TfCommandResult::Success(None);
        }
        "BWORD" => {
            // Same skip-whitespace-then-skip-word algorithm as
            // InputArea::delete_word_before_cursor, run here against the cached buffer so
            // the cursor's landing position is cheap to report in %? without waiting for
            // App::process_pending_keyboard_ops to actually perform the delete.
            let buf = &engine.keyboard_state.buffer;
            let pos = engine.keyboard_state.cursor_position.min(buf.len());
            let mut chars: Vec<char> = buf[..pos].chars().collect();
            while chars.last().is_some_and(|c| c.is_whitespace()) {
                chars.pop();
            }
            while chars.last().is_some_and(|c| !c.is_whitespace()) {
                chars.pop();
            }
            let new_pos: usize = chars.iter().map(|c| c.len_utf8()).sum();
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Dokey(DokeyName::BackwardWord));
            engine.set_global("?", TfValue::Integer(new_pos as i64));
            return TfCommandResult::Success(None);
        }
        "DWORD" => {
            // Deleting forward of the cursor never moves it.
            let pos = engine.keyboard_state.cursor_position;
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Dokey(DokeyName::ForwardWord));
            engine.set_global("?", TfValue::Integer(pos as i64));
            return TfCommandResult::Success(None);
        }
        "DEOL" => {
            let pos = engine.keyboard_state.cursor_position;
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Dokey(DokeyName::KillToEol));
            engine.set_global("?", TfValue::Integer(pos as i64));
            return TfCommandResult::Success(None);
        }
        _ => {}
    }

    // Everything else needs real App/World state - deferred via PendingKeyboardOp::Dokey.
    // TF: "The return values of other actions aren't very useful" - 1 for all of them.
    let dokey_name = match name.as_str() {
        "UP" => DokeyName::CursorUp,
        "DOWN" => DokeyName::CursorDown,
        "NEWLINE" | "ENTER" => DokeyName::Newline,
        "RECALLB" => DokeyName::HistoryPrev,
        "RECALLF" => DokeyName::HistoryNext,
        "RECALLBEG" => DokeyName::HistoryBegin,
        "RECALLEND" => DokeyName::HistoryEnd,
        "SEARCHB" => DokeyName::HistorySearchBack,
        "SEARCHF" => DokeyName::HistorySearchForward,
        "SOCKETB" => DokeyName::WorldPrev,
        "SOCKETF" => DokeyName::WorldNext,
        "REDRAW" | "REFRESH" => DokeyName::Redraw,
        "CLEAR" => DokeyName::ClearView,
        "PAUSE" => DokeyName::Pause,
        "LNEXT" => DokeyName::LiteralNext,
        "PAGE" | "PAGEDN" | "PAGEDOWN" | "PGDN" => DokeyName::PageForward,
        "PAGEBACK" | "PAGEUP" | "PGUP" => DokeyName::PageBackward,
        "HPAGE" => DokeyName::HalfPageForward,
        "HPAGEBACK" => DokeyName::HalfPageBackward,
        "LINE" => DokeyName::LineForward,
        "LINEBACK" => DokeyName::LineBackward,
        "FLUSH" => DokeyName::Flush,
        "SELFLUSH" => DokeyName::SelectiveFlush,
        _ => return TfCommandResult::Error(format!("Unknown key name: {}", name)),
    };
    engine.pending_keyboard_ops.push(PendingKeyboardOp::Dokey(dokey_name));
    engine.set_global("?", TfValue::Integer(1));
    TfCommandResult::Success(None)
}

/// Every name TinyFugue's own `tf-lib/kbfunc.tf` defines a `/def -i dokey_<name>
/// = ...` wrapper macro for (see that file's own comment block and its 36
/// `dokey_<name>` lines) - the set `is_tf_command_name` accepts a `dokey_`
/// prefix for, and [`cmd_dokey_named`]'s own `match` arms below. TF-parity plan
/// Job 21 / P2.5.
pub const DOKEY_WRAPPER_NAMES: &[&str] = &[
    "bspc", "bword", "dch", "deol", "dline", "down", "dword", "end", "home", "left",
    "lnext", "newline", "pause", "recallb", "recallbeg", "recallend", "recallf",
    "redraw", "right", "searchb", "searchf", "socketb", "socketf", "up", "wleft",
    "wright", "page", "pageback", "hpage", "hpageback", "line", "lineback", "flush",
    "selflush", "pgup", "pgdn",
];

/// True iff `name` (already lower-cased, with no `dokey_` prefix) is one of
/// [`DOKEY_WRAPPER_NAMES`].
pub fn is_dokey_wrapper_name(name: &str) -> bool {
    DOKEY_WRAPPER_NAMES.contains(&name)
}

/// The current numeric prefix the way kbfunc.tf's own `(kbnum?:1)` idiom reads
/// it: `%kbnum` mirrored into `engine.global_vars` right before `execute()`
/// (`App::sync_tf_world_info`, TF-parity plan Job 20/P2.4) - unset *or exactly
/// zero* both default to `1`, matching TF's `?:` truthiness test exactly the
/// way `InputArea::take_kbnum` does on the App side.
fn engine_kbnum(engine: &TfEngine) -> i64 {
    match engine.global_vars.get("kbnum").and_then(super::TfValue::to_int) {
        Some(n) if n != 0 => n,
        _ => 1,
    }
}

/// `/dokey_<name>` (TF-parity plan Job 21/P2.5): the "second level" of TF's own
/// two-level key mapping (`/help keys`'s "Mapping Named Keys to functions" -
/// `key_<name>` macros call these). Real kbfunc.tf defines them as invisible
/// `-i` macros, which shadow this native fallback whenever the library is
/// loaded via ordinary macro-before-builtin precedence (`execute_command_impl`
/// checks `engine.macros` before `is_tf_command_name` - nothing special is
/// needed here for that).
///
/// Real TF's own `/dokey` builtin ([`cmd_dokey`]) is always a single step -
/// see that function's doc comment - so the handful of these wrappers that
/// kbfunc.tf documents as reading `%kbnum` itself (`dokey_left = /@test
/// kbgoto(kbpoint() - (kbnum?:1))`, and BSPC/DCH/WLEFT/WRIGHT/UP/DOWN
/// alongside it) have to apply that multiplication at this layer instead,
/// exactly the way the library macro does - a negative `kbnum` reverses
/// direction (tf-help `#kbnum`), same convention as `input_handler::
/// dispatch_action`'s own kbnum-aware arms. Everything else forwards to
/// `cmd_dokey` completely unchanged, either because it is already kbnum-aware
/// one layer down (DWORD, and every name `perform_dokey` routes through
/// `dispatch_action` - RECALLB/RECALLF/SEARCHB/SEARCHF/SOCKETB/SOCKETF/PAGE/
/// PAGEBACK/HPAGE/HPAGEBACK/LINE/LINEBACK all read `app.input.kbnum` there,
/// Job 20) or because kbfunc.tf's own body doesn't read `kbnum` for it either
/// (HOME/END/DLINE/DEOL/BWORD/NEWLINE/RECALLBEG/RECALLEND/REDRAW/PAUSE/LNEXT/
/// FLUSH/SELFLUSH).
pub fn cmd_dokey_named(engine: &mut TfEngine, name: &str) -> TfCommandResult {
    use super::{DokeyName, PendingKeyboardOp, TfValue};

    let n = engine_kbnum(engine);

    match name {
        "left" => {
            let pos = engine.keyboard_state.cursor_position as i64;
            let len = engine.keyboard_state.buffer.chars().count() as i64;
            let new_pos = (pos - n).clamp(0, len);
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Goto(new_pos as usize));
            engine.set_global("?", TfValue::Integer(new_pos));
            TfCommandResult::Success(None)
        }
        "right" => {
            let pos = engine.keyboard_state.cursor_position as i64;
            let len = engine.keyboard_state.buffer.chars().count() as i64;
            let new_pos = (pos + n).clamp(0, len);
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Goto(new_pos as usize));
            engine.set_global("?", TfValue::Integer(new_pos));
            TfCommandResult::Success(None)
        }
        "bspc" => {
            let pos = engine.keyboard_state.cursor_position as i64;
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Delete(-n as i32));
            let new_pos = if n >= 0 { (pos - n).max(0) } else { pos };
            engine.set_global("?", TfValue::Integer(new_pos));
            TfCommandResult::Success(None)
        }
        "dch" => {
            let pos = engine.keyboard_state.cursor_position as i64;
            engine.pending_keyboard_ops.push(PendingKeyboardOp::Delete(n as i32));
            let new_pos = if n >= 0 { pos } else { (pos + n).max(0) };
            engine.set_global("?", TfValue::Integer(new_pos));
            TfCommandResult::Success(None)
        }
        "wleft" => {
            let count = n.unsigned_abs();
            let forward = n < 0; // negative kbnum reverses direction
            let chars: Vec<char> = engine.keyboard_state.buffer.chars().collect();
            let mut pos = engine.keyboard_state.cursor_position.min(chars.len());
            for _ in 0..count {
                if forward {
                    while pos < chars.len() && chars[pos].is_alphanumeric() { pos += 1; }
                    while pos < chars.len() && !chars[pos].is_alphanumeric() { pos += 1; }
                    engine.pending_keyboard_ops.push(PendingKeyboardOp::WordRight);
                } else {
                    while pos > 0 && !chars[pos - 1].is_alphanumeric() { pos -= 1; }
                    while pos > 0 && chars[pos - 1].is_alphanumeric() { pos -= 1; }
                    engine.pending_keyboard_ops.push(PendingKeyboardOp::WordLeft);
                }
            }
            engine.set_global("?", TfValue::Integer(pos as i64));
            TfCommandResult::Success(None)
        }
        "wright" => {
            let count = n.unsigned_abs();
            let backward = n < 0; // negative kbnum reverses direction
            let chars: Vec<char> = engine.keyboard_state.buffer.chars().collect();
            let mut pos = engine.keyboard_state.cursor_position.min(chars.len());
            for _ in 0..count {
                if backward {
                    while pos > 0 && !chars[pos - 1].is_alphanumeric() { pos -= 1; }
                    while pos > 0 && chars[pos - 1].is_alphanumeric() { pos -= 1; }
                    engine.pending_keyboard_ops.push(PendingKeyboardOp::WordLeft);
                } else {
                    while pos < chars.len() && chars[pos].is_alphanumeric() { pos += 1; }
                    while pos < chars.len() && !chars[pos].is_alphanumeric() { pos += 1; }
                    engine.pending_keyboard_ops.push(PendingKeyboardOp::WordRight);
                }
            }
            engine.set_global("?", TfValue::Integer(pos as i64));
            TfCommandResult::Success(None)
        }
        "up" => {
            let count = n.unsigned_abs();
            let dokey = if n >= 0 { DokeyName::CursorUp } else { DokeyName::CursorDown };
            for _ in 0..count { engine.pending_keyboard_ops.push(PendingKeyboardOp::Dokey(dokey)); }
            engine.set_global("?", TfValue::Integer(1));
            TfCommandResult::Success(None)
        }
        "down" => {
            let count = n.unsigned_abs();
            let dokey = if n >= 0 { DokeyName::CursorDown } else { DokeyName::CursorUp };
            for _ in 0..count { engine.pending_keyboard_ops.push(PendingKeyboardOp::Dokey(dokey)); }
            engine.set_global("?", TfValue::Integer(1));
            TfCommandResult::Success(None)
        }
        // Everything else forwards unchanged to the raw single-step primitive -
        // see this function's own doc comment for why each of these either
        // already honors `kbnum` one layer down or is correct not to.
        other => cmd_dokey(engine, &other.to_uppercase()),
    }
}

/// /histsize [-lig] [-w[<world>]] [<size>] - Get/set history buffer size
/// (`/help histsize`). Real TF tracks four independent histories (local,
/// input, global - the default - and per-world); Clay has always tracked
/// just the one shared `%{histsize}` value for all of them, and `-l`/`-i`/
/// `-g` remain accepted-but-not-distinct (unchanged by this job, including
/// defaulting to `-i`'s behavior rather than real tf's own `-g` default -
/// plan Job 14c's own ruling, not an oversight).
///
/// `-w[<world>]` is new (Job 14c): Clay has no per-world scrollback size
/// limit to report separately, so once the world name is validated (real
/// tf's own "No world <name>" diagnostic on a bad name, or on a bare `-w`
/// with no current world - `TfEngine::current_world`/`world_info_cache`),
/// it falls through to the same shared value as -g/-l/-i.
pub fn cmd_histsize(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let mut remaining = args.trim();
    let mut world_arg: Option<Option<String>> = None; // Some(None) = bare -w; Some(Some(name)) = -w<name>

    while let Some(rest) = remaining.strip_prefix('-') {
        if rest.is_empty() {
            break;
        }
        if let Some(after_w) = rest.strip_prefix('w') {
            let token_end = after_w.find(char::is_whitespace).unwrap_or(after_w.len());
            let (value, tail) = after_w.split_at(token_end);
            world_arg = Some(if value.is_empty() { None } else { Some(value.to_string()) });
            remaining = tail.trim_start();
            continue;
        }
        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, tail) = rest.split_at(token_end);
        if !token.is_empty() && token.chars().all(|c| "lig".contains(c)) {
            remaining = tail.trim_start();
            continue;
        }
        break;
    }

    if let Some(world_name) = world_arg {
        let resolved = world_name.or_else(|| engine.current_world.clone());
        match resolved {
            Some(name) if engine.world_info_cache.iter().any(|w| w.name.eq_ignore_ascii_case(&name)) => {}
            Some(name) => return TfCommandResult::Error(format!("HISTSIZE -w: No world {}", name)),
            None => return TfCommandResult::Error("HISTSIZE -w: No world".to_string()),
        }
    }

    if remaining.is_empty() {
        let size = engine.get_var("histsize")
            .and_then(|v| v.to_int())
            .unwrap_or(1000);
        return TfCommandResult::Success(Some(format!("histsize={}", size)));
    }

    if let Ok(size) = remaining.parse::<i64>() {
        engine.set_global("histsize", super::TfValue::Integer(size));
        TfCommandResult::Success(Some(format!("histsize={}", size)))
    } else {
        TfCommandResult::Error(format!("Invalid size: {}", remaining))
    }
}

/// /localecho [on|off] - Toggle local echo mode
pub fn cmd_localecho(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim().to_lowercase();

    match arg.as_str() {
        "" => {
            let val = engine.get_var("localecho")
                .map(|v| v.to_string_value())
                .unwrap_or_else(|| "off".to_string());
            TfCommandResult::Success(Some(format!("localecho={}", val)))
        }
        "on" | "1" => {
            engine.set_global("localecho", super::TfValue::Integer(1));
            TfCommandResult::Success(Some("localecho=on".to_string()))
        }
        "off" | "0" => {
            engine.set_global("localecho", super::TfValue::Integer(0));
            TfCommandResult::Success(Some("localecho=off".to_string()))
        }
        _ => TfCommandResult::Error("Usage: /localecho [on|off]".to_string()),
    }
}

/// /sub [off|on|full] - Set substitution mode
pub fn cmd_sub(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim().to_lowercase();

    match arg.as_str() {
        "" => {
            let val = engine.get_var("sub")
                .map(|v| v.to_string_value())
                .unwrap_or_else(|| "on".to_string());
            TfCommandResult::Success(Some(format!("sub={}", val)))
        }
        "on" | "1" => {
            engine.set_global("sub", super::TfValue::String("on".to_string()));
            TfCommandResult::Success(Some("sub=on".to_string()))
        }
        "off" | "0" => {
            engine.set_global("sub", super::TfValue::String("off".to_string()));
            TfCommandResult::Success(Some("sub=off".to_string()))
        }
        "full" => {
            engine.set_global("sub", super::TfValue::String("full".to_string()));
            TfCommandResult::Success(Some("sub=full".to_string()))
        }
        _ => TfCommandResult::Error("Usage: /sub [off|on|full]".to_string()),
    }
}

/// /replace old new string - Replace occurrences in string
pub fn cmd_replace(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /replace old new string".to_string());
    }

    // Parse: first two words are old and new, rest is string
    let parts: Vec<&str> = args.splitn(3, char::is_whitespace).collect();
    if parts.len() < 3 {
        return TfCommandResult::Error("Usage: /replace old new string".to_string());
    }

    let old = parts[0];
    let new = parts[1];
    let string = parts[2];

    let result = string.replace(old, new);
    // Command form both echoes the result AND sets %? to it (Job 15, verified directly
    // against real tf: `/replace a o banana` prints "bonono" and a following `%?` also
    // reads "bonono") - same "echo (command) or return (function)" dual nature as
    // /escape and /pwd below.
    engine.set_global("?", super::TfValue::String(result.clone()));
    TfCommandResult::Success(Some(result))
}

/// /tr domain range string - Translate characters
/// Maps each char in domain to the corresponding char in range
pub fn cmd_tr(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let _ = engine;
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /tr domain range string".to_string());
    }

    let parts: Vec<&str> = args.splitn(3, char::is_whitespace).collect();
    if parts.len() < 3 {
        return TfCommandResult::Error("Usage: /tr domain range string".to_string());
    }

    let domain: Vec<char> = parts[0].chars().collect();
    let range: Vec<char> = parts[1].chars().collect();
    let string = parts[2];

    let result = tr_translate(&domain, &range, string);
    TfCommandResult::Success(Some(result))
}

/// Core tr translation logic - shared by /tr command and tr() function
pub fn tr_translate(domain: &[char], range: &[char], string: &str) -> String {
    string.chars().map(|c| {
        if let Some(pos) = domain.iter().position(|&d| d == c) {
            if pos < range.len() {
                range[pos]
            } else if !range.is_empty() {
                *range.last().unwrap()
            } else {
                c
            }
        } else {
            c
        }
    }).collect()
}

// =============================================================================
// Tier 2: Trigger shortcuts
// =============================================================================

/// /trig pattern = body - Create unnamed trigger (glob mode)
pub fn cmd_trig(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /trig pattern = body".to_string());
    }

    let (pattern, body) = split_trigger_pattern_body(args);
    create_trigger_macro(engine, &pattern, &body, 0, None)
}

/// /trigp pri pattern = body - Create trigger with priority
pub fn cmd_trigp(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
    if parts.len() < 2 {
        return TfCommandResult::Error("Usage: /trigp priority pattern = body".to_string());
    }

    let priority = parts[0].parse::<i32>().unwrap_or(0);
    let (pattern, body) = split_trigger_pattern_body(parts[1]);
    create_trigger_macro(engine, &pattern, &body, priority, None)
}

/// /trigc chance pattern = body - Create trigger with probability
pub fn cmd_trigc(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    let parts: Vec<&str> = args.splitn(2, char::is_whitespace).collect();
    if parts.len() < 2 {
        return TfCommandResult::Error("Usage: /trigc chance pattern = body".to_string());
    }

    let chance = parts[0].parse::<f32>().unwrap_or(1.0);
    let (pattern, body) = split_trigger_pattern_body(parts[1]);
    create_trigger_macro(engine, &pattern, &body, 0, Some(chance))
}

/// /trigpc pri chance pattern = body - Create trigger with priority and probability
pub fn cmd_trigpc(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    let parts: Vec<&str> = args.splitn(3, char::is_whitespace).collect();
    if parts.len() < 3 {
        return TfCommandResult::Error("Usage: /trigpc priority chance pattern = body".to_string());
    }

    let priority = parts[0].parse::<i32>().unwrap_or(0);
    let chance = parts[1].parse::<f32>().unwrap_or(1.0);
    let (pattern, body) = split_trigger_pattern_body(parts[2]);
    create_trigger_macro(engine, &pattern, &body, priority, Some(chance))
}

/// Split "pattern = body" or "pattern" from trigger shortcut args
fn split_trigger_pattern_body(args: &str) -> (String, String) {
    if let Some(eq_pos) = args.find('=') {
        let before = args[..eq_pos].trim_end();
        let after = args[eq_pos + 1..].trim_start();
        (before.to_string(), after.to_string())
    } else {
        (args.to_string(), String::new())
    }
}

/// Create a trigger macro (shared by /trig, /trigp, /trigc, /trigpc)
fn create_trigger_macro(engine: &mut TfEngine, pattern: &str, body: &str, priority: i32, probability: Option<f32>) -> TfCommandResult {
    let trig_name = format!("__trig_{}", engine.next_macro_sequence);
    let macro_def = super::TfMacro {
        name: trig_name,
        body: body.to_string(),
        trigger: Some(super::TfTrigger {
            pattern: pattern.to_string(),
            match_mode: super::TfMatchMode::Glob,
            compiled: regex::Regex::new(&super::macros::glob_to_regex(pattern)).ok(),
        }),
        priority,
        probability,
        ..Default::default()
    };

    let macro_num = engine.next_macro_sequence;
    engine.add_macro(macro_def);
    TfCommandResult::Success(Some(format!("{}", macro_num)))
}

/// /untrig [-a attrs] pattern - Remove triggers matching pattern
pub fn cmd_untrig(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /untrig pattern".to_string());
    }

    // Parse optional -a attrs
    let pattern = if let Some(rest) = args.strip_prefix("-a") {
        // Skip -a and attrs, get to pattern
        if let Some(space_pos) = rest.find(char::is_whitespace) {
            rest[space_pos..].trim_start()
        } else {
            return TfCommandResult::Error("Usage: /untrig [-a attrs] pattern".to_string());
        }
    } else {
        args
    };

    let before = engine.macros.len();
    engine.macros.retain(|m| {
        if let Some(ref trigger) = m.trigger {
            trigger.pattern != pattern
        } else {
            true
        }
    });

    let removed = before - engine.macros.len();
    if removed > 0 {
        TfCommandResult::Success(Some(format!("Removed {} trigger(s) matching '{}'", removed, pattern)))
    } else {
        TfCommandResult::Success(Some(format!("No trigger found matching '{}'", pattern)))
    }
}

// =============================================================================
// Tier 3: World management
// =============================================================================

/// /unworld <name>... - For each `<name>` given, remove the definition of
/// the world with that name (`/help unworld`). Bounces to Clay's own native
/// `/unworld` (`Command::RemoveWorld`, `commands::execute_remove_world_command`)
/// the same way `/addworld` already does - this engine-only function has no
/// `&mut App` to actually delete a world with (the previous implementation
/// bounced to a Clay `/close` command that has never existed, so `/unworld`
/// did nothing at all before this job). Multiple names are forwarded as one
/// space-separated `Command::RemoveWorld` call so each is diagnosed
/// independently, same pattern as /kill and /undef.
pub fn cmd_unworld(args: &str) -> TfCommandResult {
    let args = args.trim();
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /unworld <name>...".to_string());
    }
    TfCommandResult::ClayCommand(format!("/unworld {}", args))
}

// =============================================================================
// Tier 4: Spam detection
// =============================================================================

/// /watchdog [-w<world>] [off|on|n1 [n2]] - Suppress duplicate lines
pub fn cmd_watchdog(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let mut args = args.trim();

    // Parse optional -w<world> flag
    let mut target_world: Option<String> = None;
    if args.starts_with("-w") {
        let rest = &args[2..];
        if rest.is_empty() {
            return TfCommandResult::Error("Usage: /watchdog [-w<world>] [off|on|n1 [n2]]".to_string());
        }
        // -w<world> (no space) or -w <world> (with space)
        let (world_name, remainder) = if let Some(sp) = rest.find(char::is_whitespace) {
            (rest[..sp].trim(), rest[sp..].trim())
        } else {
            (rest.trim(), "")
        };
        if world_name.is_empty() {
            return TfCommandResult::Error("Usage: /watchdog [-w<world>] [off|on|n1 [n2]]".to_string());
        }
        target_world = Some(world_name.to_string());
        args = remainder;
    }

    if let Some(world) = target_world {
        // Per-world operation
        if args.is_empty() {
            // Report resolved config for this world
            let (status, n1, n2, source) = match engine.watchdog_overrides.get(&world) {
                Some(cfg) => (if cfg.enabled { "on" } else { "off" }, cfg.n1, cfg.n2, "(override)"),
                None => (
                    if engine.watchdog_enabled { "on" } else { "off" },
                    engine.watchdog_n1, engine.watchdog_n2, "(global)"
                ),
            };
            return TfCommandResult::Success(Some(format!(
                "watchdog[{}]={} (threshold={}, window={}) {}",
                world, status, n1, n2, source
            )));
        }

        match args.to_lowercase().as_str() {
            "on" => {
                let (n1, n2) = match engine.watchdog_overrides.get(&world) {
                    Some(cfg) => (cfg.n1, cfg.n2),
                    None => (engine.watchdog_n1, engine.watchdog_n2),
                };
                engine.watchdog_overrides.insert(world.clone(), super::WatchdogConfig { enabled: true, n1, n2 });
                TfCommandResult::Success(Some(format!("watchdog[{}]=on", world)))
            }
            "off" => {
                let (n1, n2) = match engine.watchdog_overrides.get(&world) {
                    Some(cfg) => (cfg.n1, cfg.n2),
                    None => (engine.watchdog_n1, engine.watchdog_n2),
                };
                engine.watchdog_overrides.insert(world.clone(), super::WatchdogConfig { enabled: false, n1, n2 });
                TfCommandResult::Success(Some(format!("watchdog[{}]=off", world)))
            }
            _ => {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if let Ok(n1) = parts[0].parse::<usize>() {
                    let n2 = if parts.len() > 1 {
                        parts[1].parse::<usize>().unwrap_or_else(|_| {
                            engine.watchdog_overrides.get(&world).map(|c| c.n2).unwrap_or(engine.watchdog_n2)
                        })
                    } else {
                        engine.watchdog_overrides.get(&world).map(|c| c.n2).unwrap_or(engine.watchdog_n2)
                    };
                    engine.watchdog_overrides.insert(world.clone(), super::WatchdogConfig { enabled: true, n1, n2 });
                    TfCommandResult::Success(Some(format!(
                        "watchdog[{}]=on (threshold={}, window={})", world, n1, n2
                    )))
                } else {
                    TfCommandResult::Error("Usage: /watchdog [-w<world>] [off|on|n1 [n2]]".to_string())
                }
            }
        }
    } else {
        // Global operation (original behavior)
        if args.is_empty() {
            let status = if engine.watchdog_enabled { "on" } else { "off" };
            let mut msg = format!(
                "watchdog={} (threshold={}, window={})",
                status, engine.watchdog_n1, engine.watchdog_n2
            );
            if !engine.watchdog_overrides.is_empty() {
                let mut worlds: Vec<&String> = engine.watchdog_overrides.keys().collect();
                worlds.sort();
                for world in worlds {
                    let cfg = &engine.watchdog_overrides[world];
                    let ws = if cfg.enabled { "on" } else { "off" };
                    msg.push_str(&format!(
                        "\n  {}: {} (threshold={}, window={})",
                        world, ws, cfg.n1, cfg.n2
                    ));
                }
            }
            return TfCommandResult::Success(Some(msg));
        }

        match args.to_lowercase().as_str() {
            "on" => {
                engine.watchdog_enabled = true;
                TfCommandResult::Success(Some("watchdog=on".to_string()))
            }
            "off" => {
                engine.watchdog_enabled = false;
                TfCommandResult::Success(Some("watchdog=off".to_string()))
            }
            _ => {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if let Ok(n1) = parts[0].parse::<usize>() {
                    engine.watchdog_n1 = n1;
                    if parts.len() > 1 {
                        if let Ok(n2) = parts[1].parse::<usize>() {
                            engine.watchdog_n2 = n2;
                        }
                    }
                    engine.watchdog_enabled = true;
                    TfCommandResult::Success(Some(format!(
                        "watchdog=on (threshold={}, window={})",
                        engine.watchdog_n1, engine.watchdog_n2
                    )))
                } else {
                    TfCommandResult::Error("Usage: /watchdog [-w<world>] [off|on|n1 [n2]]".to_string())
                }
            }
        }
    }
}

/// /watchname [off|on|n1 [n2]] - Suppress spam from repeated character names
pub fn cmd_watchname(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        let status = if engine.watchname_enabled { "on" } else { "off" };
        return TfCommandResult::Success(Some(format!(
            "watchname={} (threshold={}, window={})",
            status, engine.watchname_n1, engine.watchname_n2
        )));
    }

    match args.to_lowercase().as_str() {
        "on" => {
            engine.watchname_enabled = true;
            TfCommandResult::Success(Some("watchname=on".to_string()))
        }
        "off" => {
            engine.watchname_enabled = false;
            TfCommandResult::Success(Some("watchname=off".to_string()))
        }
        _ => {
            let parts: Vec<&str> = args.split_whitespace().collect();
            if let Ok(n1) = parts[0].parse::<usize>() {
                engine.watchname_n1 = n1;
                if parts.len() > 1 {
                    if let Ok(n2) = parts[1].parse::<usize>() {
                        engine.watchname_n2 = n2;
                    }
                }
                engine.watchname_enabled = true;
                TfCommandResult::Success(Some(format!(
                    "watchname=on (threshold={}, window={})",
                    engine.watchname_n1, engine.watchname_n2
                )))
            } else {
                TfCommandResult::Error("Usage: /watchname [off|on|n1 [n2]]".to_string())
            }
        }
    }
}

// =============================================================================
// Job 15: missing builtins + stdlib one-liners (see the TF-parity plan, section B
// "Missing TF commands", and Phase 1 step P1.14).
// =============================================================================

/// /ismacro <macro-options> - the command FORM (distinct from the `ismacro(name)`
/// expression FUNCTION in expressions.rs, which only ever does an exact-name check).
/// Takes the same macro-option filter grammar `/list`/`/purge` use (Job 7's
/// `MacroFilter`) and sets %? to the sequence number of the LAST macro that matches
/// every given option, or 0 if none match - no output is echoed either way, matching
/// real tf's own `/ismacro` (a stdlib.tf macro, `/def -i ismacro = /test tfclose("o")%;
/// /@list -s -i %{*-@}`) exactly (verified directly: only %? changes).
///
/// This is what unblocks kbbind.tf's own `~bind_if_not_bound` (finding 28): once
/// stdlib.tf is NOT loaded (no macro named "ismacro" in scope to shadow this), a plain
/// `/ismacro -msimple -ib'^R'` reaches this native command instead of falling through
/// to Clay's Clay-command fallback as literal, unsubstituted text. The OTHER half of
/// finding 28 - that macro's own condition losing its "%1" - was a separate bug in
/// `control_flow::evaluate_condition` (see that function's own doc comment), fixed
/// alongside this.
pub fn cmd_ismacro(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let default_style = super::macros::default_matching_style(engine);
    // Real tf's own "/ismacro" is itself a stdlib.tf macro whose body
    // hardcodes "-i" ahead of whatever the caller passed (`/def -i ismacro
    // = /test tfclose("o")%; /@list -s -i %{*-@}`) - so a caller that
    // never mentions invisibility at all still matches an invisible macro
    // (verified directly: `/def -i foo = ...` then a bare `/ismacro foo`,
    // with no "-i" of its own, leaves %? nonzero). Prepending it here the
    // same way, ahead of the caller's own args, means a caller-supplied
    // "-I" (only-invisible) still wins - same left-to-right override order
    // real tf's own reconstructed command line has. Without this,
    // spedwalk.tf's own "/if /ismacro ~speedwalk%; /then ..." toggle never
    // saw its own (invisible, `-i`-defined) hook macro as already defined,
    // so `/speedwalk` could only ever take the "enable" branch.
    let filter = match super::macros::MacroFilter::parse(&format!("-i {}", args), super::macros::FilterKind::List, default_style) {
        Ok(f) => f,
        Err(e) => return TfCommandResult::Error(e),
    };
    let last = engine.macros.iter()
        .filter(|m| filter.matches(m))
        .map(|m| m.sequence_number)
        .max()
        .unwrap_or(0);
    engine.set_global("?", super::TfValue::Integer(last as i64));
    TfCommandResult::Success(None)
}

/// /isvar <name> - 1 if <name> is set as a variable (local or global scope), else 0.
/// No output, %? only - same shape as /ismacro above and real tf's own `/isvar`
/// (`/def -i isvar = /test tfclose("o")%; /listvar -msimple -- %*`).
pub fn cmd_isvar(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.trim().trim_start_matches('%');
    let set = if name.is_empty() { false } else { engine.get_var(name).is_some() };
    engine.set_global("?", super::TfValue::Integer(if set { 1 } else { 0 }));
    TfCommandResult::Success(None)
}

/// /features [<name>] - with no argument, prints the same `+name`/`-name` list as the
/// `features()` expression function (sharing its table, `super::expressions::features_table`);
/// with a `<name>`, sets %? to 1/0 (enabled/disabled or unknown) and prints nothing -
/// verified directly against real tf 5.0 beta 8 for both forms.
pub fn cmd_features(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let name = args.trim();
    let (order, off) = super::expressions::features_table();
    if name.is_empty() {
        let parts: Vec<String> = order.iter().map(|(key, disp)| {
            let on = !off.contains(key);
            format!("{}{}", if on { "+" } else { "-" }, disp)
        }).collect();
        TfCommandResult::Success(Some(parts.join(" ")))
    } else {
        let lname = name.to_lowercase();
        let known = order.iter().any(|(key, _)| *key == lname);
        let on = known && !off.contains(&lname.as_str());
        engine.set_global("?", super::TfValue::Integer(if on { 1 } else { 0 }));
        TfCommandResult::Success(None)
    }
}

/// /true - stdlib.tf: `/def -i true = /@test 1`. Silent (no output), sets %?=1. Note
/// `/not <command>` (finding 13) lives in `parser.rs` as `cmd_not`, not here - it needs
/// `execute_command_substituted`/`parse_eval_level`, both private to that module.
pub fn cmd_true(engine: &mut TfEngine, _args: &str) -> TfCommandResult {
    engine.set_global("?", super::TfValue::Integer(1));
    TfCommandResult::Success(None)
}

/// /false - stdlib.tf: `/def -i false = /@test 0`. Silent, %?=0.
pub fn cmd_false(engine: &mut TfEngine, _args: &str) -> TfCommandResult {
    engine.set_global("?", super::TfValue::Integer(0));
    TfCommandResult::Success(None)
}

/// /: - stdlib.tf's null command: `/def -i : = /@test 1`. Silent, %?=1 (same as /true -
/// TF documents both as a no-op that "always succeeds").
pub fn cmd_null(engine: &mut TfEngine, _args: &str) -> TfCommandResult {
    engine.set_global("?", super::TfValue::Integer(1));
    TfCommandResult::Success(None)
}

/// /first <args...> - stdlib.tf: `/def -i first = /result {1}`. Prints (command form)
/// and returns (%?) the first whitespace-separated word of its arguments.
pub fn cmd_first(args: &str) -> TfCommandResult {
    let first = args.split_whitespace().next().unwrap_or("").to_string();
    TfCommandResult::Result(first)
}

/// /rest <args...> - stdlib.tf: `/def -i rest = /result {-1}` - TF's `{-N}` means "all
/// but the first N" (finding C.5, fixed Job 8), so `{-1}` is every word after the first.
pub fn cmd_rest(args: &str) -> TfCommandResult {
    let rest = match args.trim().split_once(char::is_whitespace) {
        Some((_, rest)) => rest.trim_start().to_string(),
        None => String::new(),
    };
    TfCommandResult::Result(rest)
}

/// /last <args...> - stdlib.tf: `/def -i last = /result {L}` - the last word.
pub fn cmd_last(args: &str) -> TfCommandResult {
    let last = args.split_whitespace().last().unwrap_or("").to_string();
    TfCommandResult::Result(last)
}

/// /nth <n> <args...> (stdlib.tf: `/def -i nth = /result {1} > 0 ? shift({1}), {1} : ""`):
/// drop the first <n> words, then take the next one (1-based). Real tf: a non-numeric
/// or non-positive <n> gives "" (verified: `{1} > 0` is false whenever `{1}` isn't a
/// positive number - TF's own `>` on a non-numeric string is false, not an error).
pub fn cmd_nth(args: &str) -> TfCommandResult {
    let mut words = args.split_whitespace();
    let n = match words.next().and_then(|s| s.parse::<i64>().ok()) {
        Some(n) if n > 0 => n as usize,
        _ => return TfCommandResult::Result(String::new()),
    };
    let nth = words.nth(n - 1).unwrap_or("").to_string();
    TfCommandResult::Result(nth)
}

/// /ver - stdlib.tf's own `/ver` extracts just the version number out of `/version`'s
/// text via a regexp match on real tf's own output shape ("version X.Y. Copyright").
/// Clay's `/version` output doesn't have that shape (`get_version_string()`: "Clay vX.Y.Z
/// (build ...) [platform/arch]"), so this returns Clay's bare version number directly
/// (the `VERSION` constant) rather than porting a regexp that would never match.
pub fn cmd_ver() -> TfCommandResult {
    TfCommandResult::Result(crate::VERSION.to_string())
}

/// /nogag [<pattern>] - `/help nogag`: with no argument, turn off the `%gag` flag
/// (disabling all gag attributes) and print "% Gags disabled."; with a <pattern>,
/// remove a gag-attributed macro matching it (delegates to the existing `/untrig -ag`
/// implementation, per this job's own brief).
pub fn cmd_nogag(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let pattern = args.trim();
    if pattern.is_empty() {
        engine.set_global("gag", super::TfValue::Integer(0));
        TfCommandResult::Success(Some("Gags disabled.".to_string()))
    } else {
        cmd_untrig(engine, &format!("-ag {}", pattern))
    }
}

/// /sys <command> - a genuine native builtin (unlike real tf's own `/sys`, which is a
/// stdlib.tf macro over `/quote -S -decho`, `/def -i sys = /quote -S -decho \!!%{*-:}` -
/// this job's brief asks for it natively instead). Runs `<command>` inline via `sh -c`
/// (no interactive tty, same constraint `/sh` documents), echoes every stdout/stderr
/// line, and sets %? to the REAL process exit status (verified directly against real
/// tf's own `/sys`/`/quote`: %? after a shell command is its exit code, e.g. 7 after
/// "exit 7" - not a 0/1 boolean the way most other commands' %? is). Honours /restrict
/// (>= SHELL refuses, matching real tf's own "/sh" and "/quote !").
pub fn cmd_sys(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if engine.restrict_level >= super::RestrictLevel::Shell {
        return TfCommandResult::Error("SYS: restricted".to_string());
    }
    let cmd = args.trim();
    if cmd.is_empty() {
        return TfCommandResult::Error("Usage: /sys <command>".to_string());
    }
    match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(output) => {
            let mut lines = Vec::new();
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                lines.push(line.to_string());
            }
            for line in String::from_utf8_lossy(&output.stderr).lines() {
                lines.push(line.to_string());
            }
            let code = output.status.code().unwrap_or(-1) as i64;
            engine.set_global("?", super::TfValue::Integer(code));
            if lines.is_empty() {
                TfCommandResult::Success(None)
            } else {
                TfCommandResult::Success(Some(lines.join("\n")))
            }
        }
        Err(e) => TfCommandResult::Error(format!("SYS: {}", e)),
    }
}

/// /restrict [SHELL|FILE|WORLD] - report or raise TF's security ratchet (`RestrictLevel`).
/// No argument: print "% restriction level: <level>" (verified directly against real
/// tf, lower-case level name). With an argument: raise the level - never lower it, per
/// `/help restrict`: "Once restriction has been set to a particular level, it can not
/// be lowered." Silent on success (verified: setting a level prints nothing itself,
/// only a later bare `/restrict` reports it).
pub fn cmd_restrict(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim();
    if arg.is_empty() {
        return TfCommandResult::Success(Some(format!("restriction level: {}", engine.restrict_level.name())));
    }
    match super::RestrictLevel::parse(arg) {
        Some(level) => {
            if level > engine.restrict_level {
                engine.restrict_level = level;
            }
            TfCommandResult::Success(None)
        }
        None => TfCommandResult::Error(format!("RESTRICT: {}: not a valid restriction level", arg)),
    }
}

/// /core - real tf: on receipt of a fatal signal, dump core if `features("core")` is
/// enabled (a debugging aid for tf's own C implementation). Meaningless for Clay, which
/// has no such native crash-dump concept - report that instead of silently no-opping,
/// matching the wording style of Clay's other "not applicable" stubs (`/telnet`,
/// `/changes`, ...).
pub fn cmd_core() -> TfCommandResult {
    TfCommandResult::Success(Some("% /core: Not supported in Clay.".to_string()))
}

/// /xtitle <text> - put <text> on the console's terminal-tab/titlebar (`/help xtitle`,
/// tools.tf's own `/def -i xtitle = ...`, implemented natively here per this job's
/// brief). The engine has no terminal to write to - queues the request
/// (`TfEngine::pending_xtitle`) for `App::apply_pending_tf_console_ops` (main.rs) to
/// apply via crossterm's `SetTitle` command, mirroring `/dokey`'s own
/// `PendingKeyboardOp` "engine records, App drains" pattern. CLAUDE.md forbids ever
/// printing a raw escape sequence into the output area once the TUI is live - this
/// never does; `SetTitle` is queued straight to stdout by the drain. Console-only by
/// construction: a web/GUI/remote-console/daemon client has no terminal tab of its own
/// to rename, so `/xtitle` there is accepted (the field is set) but has no visible
/// effect - not a missing feature, just nothing to apply it to.
pub fn cmd_xtitle(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let text = args.trim();
    if text.is_empty() {
        return TfCommandResult::Error("Usage: /xtitle <text>".to_string());
    }
    engine.pending_xtitle = Some(text.to_string());
    TfCommandResult::Success(None)
}

/// /more [on|off|1|0] - `/help more`: "Sets the value of the %{more} flag." Real tf's
/// own `/more` is a stdlib.tf macro (`/def -i more = /if (...) /echo -e ...%; /endif%;
/// /set more %*`) whose bare/invalid form is actually an ERROR (verified directly:
/// `/more` with no argument gives "more: Invalid more value \"\".  Valid values are:
/// off (0), on (1)." - %more is a validated boolean flag and `/set more` with an empty
/// value fails validation). This job's brief additionally asks /more to toggle CLAY's
/// own real more-mode setting (`Settings::more_mode_enabled`), which this engine has no
/// access to - queues the request (`TfEngine::pending_more_mode`) for
/// `App::apply_pending_tf_console_ops` to apply, persist and broadcast (console-only,
/// same reasoning as `/xtitle` above - see that function's doc comment). Also updates
/// the TF-visible `%more` variable unconditionally so a script reading `%{more}` back
/// sees the new value regardless of which client set it.
pub fn cmd_more(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim();
    let on = match arg.to_lowercase().as_str() {
        "on" | "1" => true,
        "off" | "0" => false,
        _ => {
            return TfCommandResult::Error(format!(
                "more: Invalid more value \"{}\".  Valid values are: off (0), on (1).", arg
            ));
        }
    };
    engine.set_global("more", super::TfValue::Integer(if on { 1 } else { 0 }));
    engine.pending_more_mode = Some(on);
    TfCommandResult::Success(None)
}

/// /wrap [on|off|<n>] - stdlib.tf: `/def -i wrap = /if ({*} =/ '[0-9]*') /set
/// wrapsize=%*%; /set wrap=1%; /else /set wrap %*%; /endif` - a numeric argument sets
/// `%wrapsize` and turns `%wrap` on; otherwise the argument (normally on/off) is set
/// into `%wrap` directly. Clay has a real analogue only for the numeric form: `Settings
/// ::wrapspace`, the console's own hang-indent wrap width (see its own doc comment in
/// main.rs) - queues `TfEngine::pending_wrapspace` for the same console-only drain as
/// `/more`/`/xtitle` above when given a number. `on`/`off` have no Clay-side output-
/// wrapping concept to toggle, so they only update the TF-visible `%wrap` variable, per
/// this job's own "otherwise accept and document" brief.
pub fn cmd_wrap(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let arg = args.trim();
    if arg.is_empty() {
        return TfCommandResult::Error("Usage: /wrap [on|off|<n>]".to_string());
    }
    if let Ok(n) = arg.parse::<i64>() {
        engine.set_global("wrapsize", super::TfValue::Integer(n));
        engine.set_global("wrap", super::TfValue::Integer(1));
        engine.pending_wrapspace = Some(n.clamp(0, u8::MAX as i64) as u8);
    } else {
        engine.set_global("wrap", super::TfValue::from(arg));
    }
    TfCommandResult::Success(None)
}

/// /limit [-v] [-a] [-m<style>] [<pattern>] - `/help limit`: redraw the window showing
/// only lines matching <pattern> (`-v`: only NON-matching lines; `-a`: only lines with
/// attributes; `-m<style>`: simple/glob/regexp instead of `%matching`'s default). A
/// real feature (a filtered scrollback view), implemented on top of Clay's existing F4
/// filter popup (`FilterPopup`, main.rs) rather than a new one - this engine has no
/// access to `App`/`FilterPopup`, so it only parses the arguments and queues a
/// `PendingLimitOp` for `App::apply_pending_tf_console_ops` to act on (same "engine
/// records, App drains" pattern as `/xtitle`/`/more`/`/wrap` above).
///
/// Console-only by construction (finding 33 in the TF-parity plan): a web/GUI client's
/// F4 filter is independent client-side state (`app.js`'s own `openFilterPopup`), and
/// there is no existing WS message that drives it remotely from server-side text -
/// building one is explicitly out of this job's scope.
///
/// With no options and no pattern, real tf's `/limit` silently returns 1/0 via %? ("a
/// limit is in effect" or not) - queues `PendingLimitOp::Report` instead, which prints
/// a short status line, since %? can't survive the queued round trip to `App` (a
/// documented deviation, not an oversight).
pub fn cmd_limit(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let mut remaining = args.trim();
    let mut invert = false;
    let mut attrs_only = false;
    let mut explicit_style: Option<super::TfMatchMode> = None;

    loop {
        if let Some(rest) = remaining.strip_prefix("-v") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                invert = true;
                remaining = rest.trim_start();
                continue;
            }
        }
        if let Some(rest) = remaining.strip_prefix("-a") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                attrs_only = true;
                remaining = rest.trim_start();
                continue;
            }
        }
        if let Some(rest) = remaining.strip_prefix("-m") {
            let (value, after) = match rest.find(char::is_whitespace) {
                Some(pos) => (&rest[..pos], rest[pos..].trim_start()),
                None => (rest, ""),
            };
            match super::TfMatchMode::parse(value) {
                Some(m) => {
                    explicit_style = Some(m);
                    remaining = after;
                    continue;
                }
                None => return TfCommandResult::Error(format!("Unknown match mode: {}", value)),
            }
        }
        break;
    }

    let pattern = if remaining.is_empty() { None } else { Some(remaining.to_string()) };
    if pattern.is_none() && !invert && !attrs_only && explicit_style.is_none() {
        engine.pending_limit_op = Some(super::PendingLimitOp::Report);
    } else {
        let style = explicit_style.unwrap_or_else(|| super::macros::default_matching_style(engine));
        engine.pending_limit_op = Some(super::PendingLimitOp::Apply { pattern, invert, attrs_only, style });
    }
    TfCommandResult::Success(None)
}

/// /unlimit - clear any active `/limit`. See `cmd_limit`'s doc comment.
pub fn cmd_unlimit(engine: &mut TfEngine, _args: &str) -> TfCommandResult {
    engine.pending_limit_op = Some(super::PendingLimitOp::Clear);
    TfCommandResult::Success(None)
}

/// /relimit - re-apply the most recently applied `/limit`. See `cmd_limit`'s doc comment.
pub fn cmd_relimit(engine: &mut TfEngine, _args: &str) -> TfCommandResult {
    engine.pending_limit_op = Some(super::PendingLimitOp::Reapply);
    TfCommandResult::Success(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::QuoteDisposition;
    use super::super::RecallRange;
    use super::super::ProcessKind;
    use super::super::WorldInfoCache;
    use super::super::macros;
    use super::super::parser::execute_command;

    // ---- /recall argument parsing tests ----

    fn recall_opts(args: &str) -> RecallOptions {
        match cmd_recall(args) {
            TfCommandResult::Recall(opts) => opts,
            other => panic!("Expected Recall result, got {:?}", other),
        }
    }

    #[test]
    fn test_recall_quoted_format_and_last_matching() {
        // The canonical bug: /recall -t"%m/%d/%y %H:%M:%S" /10
        let opts = recall_opts(r#"-t"%m/%d/%y %H:%M:%S" /10"#);
        assert!(opts.show_timestamps);
        assert_eq!(opts.timestamp_format.as_deref(), Some("%m/%d/%y %H:%M:%S"));
        assert_eq!(opts.range, RecallRange::LastMatching(10));
        assert_eq!(opts.pattern, None);
    }

    #[test]
    fn test_recall_i_parses_input_source() {
        // Cheap parser lock so a future flag refactor can't silently re-break -i (it was a
        // dead no-op for a while - see actions.rs's RecallSource::Input arm).
        assert_eq!(recall_opts("-i north").source, RecallSource::Input);
        assert_eq!(recall_opts("-i *tell*").pattern.as_deref(), Some("*tell*"));
    }

    #[test]
    fn test_recall_a_attrs_generalized() {
        // -ag: the one attribute letter with a distinct effect (show gagged lines).
        let opts = recall_opts("-ag combat");
        assert!(opts.show_gagged);
        assert_eq!(opts.suppress_attrs, "g");
        assert_eq!(opts.pattern.as_deref(), Some("combat"));

        // -a<attrs> now consumes the WHOLE token as the attribute list (matching -t/-m/-w's
        // own "rest of token" convention here), not just a lone trailing 'g' - a multi-letter
        // list still sets show_gagged when 'g' is anywhere in it, and the full list is kept
        // for round-tripping even though only 'g' has a distinct effect today.
        let opts = recall_opts("-agu combat");
        assert!(opts.show_gagged);
        assert_eq!(opts.suppress_attrs, "gu");

        // An attribute list without 'g' must NOT show gagged lines.
        let opts = recall_opts("-au combat");
        assert!(!opts.show_gagged);
        assert_eq!(opts.suppress_attrs, "u");
    }

    #[test]
    fn test_recall_context_options_parse() {
        let opts = recall_opts("-A2 combat");
        assert_eq!(opts.context_after, 2);
        assert_eq!(opts.context_before, 0);

        let opts = recall_opts("-B3 combat");
        assert_eq!(opts.context_before, 3);
        assert_eq!(opts.context_after, 0);

        let opts = recall_opts("-C1 combat");
        assert_eq!(opts.context_before, 1);
        assert_eq!(opts.context_after, 1);

        // Combined, in one command line.
        let opts = recall_opts("-B1 -A1 combat");
        assert_eq!(opts.context_before, 1);
        assert_eq!(opts.context_after, 1);
    }

    #[test]
    fn test_recall_hash_range_prefix_parses() {
        // "#" must be recognized immediately before <range>, setting show_line_numbers and
        // NOT itself becoming part of the range/pattern text.
        let opts = recall_opts("#10");
        assert!(opts.show_line_numbers);
        assert_eq!(opts.range, RecallRange::Last(10));

        let opts = recall_opts("#1-5 combat");
        assert!(opts.show_line_numbers);
        assert_eq!(opts.range, RecallRange::Range(1, 5));
        assert_eq!(opts.pattern.as_deref(), Some("combat"));

        // "#" combined with a preceding option.
        let opts = recall_opts("-l #10");
        assert!(opts.show_line_numbers);
        assert_eq!(opts.source, RecallSource::Local);
        assert_eq!(opts.range, RecallRange::Last(10));
    }

    #[test]
    fn test_recall_t_no_format() {
        // -t alone → show_timestamps true, no custom format, range applied
        let opts = recall_opts("-t /5");
        assert!(opts.show_timestamps);
        assert_eq!(opts.timestamp_format, None);
        assert_eq!(opts.range, RecallRange::LastMatching(5));
    }

    #[test]
    fn test_recall_quoted_format_with_pattern() {
        // Quoted format followed by a plain-text pattern
        let opts = recall_opts(r#"-t"%H:%M:%S" /10 hello"#);
        assert!(opts.show_timestamps);
        assert_eq!(opts.timestamp_format.as_deref(), Some("%H:%M:%S"));
        assert_eq!(opts.range, RecallRange::LastMatching(10));
        assert_eq!(opts.pattern.as_deref(), Some("hello"));
    }

    #[test]
    fn test_recall_unquoted_simple_format() {
        // -t%H:%M:%S (no spaces in format, no quotes needed)
        let opts = recall_opts("-t%H:%M:%S /3");
        assert!(opts.show_timestamps);
        assert_eq!(opts.timestamp_format.as_deref(), Some("%H:%M:%S"));
        assert_eq!(opts.range, RecallRange::LastMatching(3));
    }

    #[test]
    fn test_recall_no_args_returns_usage() {
        match cmd_recall("") {
            TfCommandResult::Success(Some(msg)) => assert!(msg.contains("Usage")),
            other => panic!("Expected usage message, got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_beep() {
        let mut engine = super::TfEngine::new();
        // Default: 3 beeps
        let result = cmd_beep(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Success(Some(ref s)) if s == "\x07\x07\x07"));
        // Explicit count
        let result = cmd_beep(&mut engine, "5");
        assert!(matches!(result, TfCommandResult::Success(Some(ref s)) if s == "\x07\x07\x07\x07\x07"));
        // Off
        let result = cmd_beep(&mut engine, "off");
        assert!(matches!(result, TfCommandResult::Success(Some(ref s)) if s == "beep off"));
        // Beep while off does nothing
        let result = cmd_beep(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Success(None)));
        // On
        let result = cmd_beep(&mut engine, "on");
        assert!(matches!(result, TfCommandResult::Success(Some(ref s)) if s == "beep on"));
        // Works again
        let result = cmd_beep(&mut engine, "1");
        assert!(matches!(result, TfCommandResult::Success(Some(ref s)) if s == "\x07"));
    }

    #[test]
    fn test_cmd_beep_on_off_case_insensitive_and_count_capped() {
        let mut engine = super::TfEngine::new();

        // ON/OFF (uppercase, matching /help beep's own usage text) work the same as
        // lowercase, and set the "beep" variable.
        cmd_beep(&mut engine, "OFF");
        assert_eq!(engine.get_var("beep").map(|v| v.to_string_value()).as_deref(), Some("0"));
        cmd_beep(&mut engine, "ON");
        assert_eq!(engine.get_var("beep").map(|v| v.to_string_value()).as_deref(), Some("1"));

        // A huge count is capped sensibly rather than generating gigabytes of output.
        let result = cmd_beep(&mut engine, "999999");
        match result {
            TfCommandResult::Success(Some(s)) => assert_eq!(s.len(), 100, "expected the count to be capped at 100 beeps"),
            other => panic!("Expected Success(Some), got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_time() {
        let mut engine = TfEngine::new();

        // No format: defaults to %time_format ("%H:%M" - see TfEngine::new), which
        // never contains a literal digit-only string, but %? must still be set to
        // whatever it produced.
        let result = cmd_time(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Success(Some(_))));
        assert!(engine.get_var("?").is_some());

        // An explicit numeric-yielding format
        let result = cmd_time(&mut engine, "%s");
        if let TfCommandResult::Success(Some(s)) = result {
            assert!(s.parse::<u64>().is_ok());
        } else {
            panic!("expected Success(Some(_))");
        }

        // A 4-digit year, unaffected by the current date
        let result = cmd_time(&mut engine, "%Y");
        if let TfCommandResult::Success(Some(s)) = result {
            assert_eq!(s.len(), 4);
            assert!(s.chars().all(|c| c.is_ascii_digit()));
        } else {
            panic!("expected Success(Some(_))");
        }

        // Clay's own kept extension: /time /command times a nested command instead.
        let result = cmd_time(&mut engine, "/echo hi");
        match result {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("hi"), "expected the echoed text in {msg:?}");
                assert!(msg.contains("Elapsed:"), "expected a timing line in {msg:?}");
            }
            other => panic!("expected Success(Some(_)), got {other:?}"),
        }
    }

    #[test]
    fn test_cmd_runtime() {
        let mut engine = TfEngine::new();

        let result = cmd_runtime(&mut engine, "/echo hi");
        match result {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("hi"), "expected the echoed text in {msg:?}");
                assert!(msg.contains("real="), "expected TF's real= line in {msg:?}");
                assert!(msg.contains("cpu="), "expected TF's cpu= line in {msg:?}");
            }
            other => panic!("expected Success(Some(_)), got {other:?}"),
        }

        // No argument is an error, matching /help runtime's "Usage" shape.
        assert!(matches!(cmd_runtime(&mut engine, ""), TfCommandResult::Error(_)));
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
        let mut engine = TfEngine::new();

        // Test literal text (no source specifier)
        let result = cmd_quote(&mut engine, "hello world");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines, vec!["hello world"]);
                assert_eq!(disposition, QuoteDisposition::Send);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test empty args
        let result = cmd_quote(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Error(_)));

        // Test with -decho option
        let result = cmd_quote(&mut engine, "-decho test message");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines, vec!["test message"]);
                assert_eq!(disposition, QuoteDisposition::Echo);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with -wworld option
        let result = cmd_quote(&mut engine, "-wmyworld hello");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines, vec!["hello"]);
                assert_eq!(disposition, QuoteDisposition::Send);
                assert_eq!(world, Some("myworld".to_string()));
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }
    }

    #[test]
    fn test_cmd_quote_ansi_stripping() {
        let mut engine = TfEngine::new();

        // Default: ANSI stripped from shell source
        let result = cmd_quote(&mut engine, "!printf '\\033[31mred\\033[0m'");
        match result {
            TfCommandResult::Quote { lines, strip_ansi, .. } => {
                assert!(strip_ansi, "strip_ansi should default to true");
                assert_eq!(lines, vec!["red"], "ANSI should be stripped by default");
            }
            _ => panic!("Expected Quote result"),
        }

        // -A: ANSI preserved from shell source
        let result = cmd_quote(&mut engine, "-A !printf '\\033[31mred\\033[0m'");
        match result {
            TfCommandResult::Quote { lines, strip_ansi, .. } => {
                assert!(!strip_ansi, "strip_ansi should be false with -A");
                assert!(lines[0].contains('\x1b'), "ANSI sequences should be preserved with -A");
            }
            _ => panic!("Expected Quote result"),
        }

        // Default: ANSI stripped from literal text
        let result = cmd_quote(&mut engine, "\x1b[31mhello\x1b[0m");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert_eq!(lines, vec!["hello"], "ANSI stripped from literal text by default");
            }
            _ => panic!("Expected Quote result"),
        }

        // -A trailing with no source: error
        let result = cmd_quote(&mut engine, "-A");
        assert!(matches!(result, TfCommandResult::Error(_)), "bare -A with no source should error");
    }

    #[test]
    fn test_cmd_sh() {
        let mut engine = TfEngine::new();
        let result = cmd_sh(&mut engine, "echo hello");
        if let TfCommandResult::Success(Some(s)) = result {
            assert!(s.contains("hello"));
        }

        // Bare /sh (no command): Clay can't hand the TUI to an interactive
        // shell, so this must error rather than hang (plan Job 14c).
        let result = cmd_sh(&mut engine, "");
        assert!(matches!(result, TfCommandResult::Error(_)));
    }

    /// Job 14c: `-q` suppresses both the SHELL hook and the default
    /// "Executing command: ..." message; without it, the message is present.
    #[test]
    fn test_cmd_sh_quiet_suppresses_executing_message() {
        let mut engine = TfEngine::new();

        let result = cmd_sh(&mut engine, "echo hi");
        match result {
            TfCommandResult::Success(Some(s)) => assert!(s.contains("Executing command: echo hi")),
            other => panic!("expected Success(Some(_)) with an Executing line, got {:?}", other),
        }

        let result = cmd_sh(&mut engine, "-q echo hi");
        match result {
            TfCommandResult::Success(Some(s)) => assert!(!s.contains("Executing")),
            TfCommandResult::Success(None) => {}
            other => panic!("expected Success without an Executing line, got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_quote_file() {
        use std::io::Write;
        let mut engine = TfEngine::new();

        // Create a temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("clay_quote_test.txt");
        {
            let mut file = std::fs::File::create(&temp_file).unwrap();
            writeln!(file, "line one").unwrap();
            writeln!(file, "line two").unwrap();
            writeln!(file, "line three").unwrap();
        }

        // Test reading from file
        let path = temp_file.to_string_lossy();
        let result = cmd_quote(&mut engine, &format!("'\"{}\"", path));
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(lines[0], "line one");
                assert_eq!(lines[1], "line two");
                assert_eq!(lines[2], "line three");
                assert_eq!(disposition, QuoteDisposition::Send);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with prefix
        let result = cmd_quote(&mut engine, &format!("say '\"{}\"", path));
        match result {
            TfCommandResult::Quote { lines, disposition, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(lines[0], "say line one");
                assert_eq!(lines[1], "say line two");
                assert_eq!(lines[2], "say line three");
                assert_eq!(disposition, QuoteDisposition::Send);
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with -decho option
        let result = cmd_quote(&mut engine, &format!("-decho '\"{}\"", path));
        match result {
            TfCommandResult::Quote { lines, disposition, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(disposition, QuoteDisposition::Echo);
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Clean up
        let _ = std::fs::remove_file(&temp_file);
    }

    #[test]
    fn test_cmd_quote_shell() {
        let mut engine = TfEngine::new();

        // Test reading from shell command (using ! prefix)
        let result = cmd_quote(&mut engine, "!\"echo hello\"");
        match result {
            TfCommandResult::Quote { lines, disposition, world, .. } => {
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "hello");
                assert_eq!(disposition, QuoteDisposition::Send);
                assert!(world.is_none());
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with prefix
        let result = cmd_quote(&mut engine, "say !\"echo world\"");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "say world");
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }
    }

    #[test]
    fn test_cmd_quote_internal() {
        let mut engine = TfEngine::new();

        // Test reading from internal command (using ` prefix)
        // /version returns a success message
        let result = cmd_quote(&mut engine, "`\"/version\"");
        match result {
            TfCommandResult::Quote { lines, disposition, .. } => {
                assert!(!lines.is_empty());
                assert!(lines[0].contains("Clay") || lines[0].contains("TF"));
                assert_eq!(disposition, QuoteDisposition::Send);
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test with prefix
        let result = cmd_quote(&mut engine, "think `\"/version\"");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert!(!lines.is_empty());
                assert!(lines[0].starts_with("think "));
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }

        // Test /version (Clay command) is also capturable
        let result = cmd_quote(&mut engine, "think `\"/version\"");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert!(!lines.is_empty());
                assert!(lines[0].contains("Clay v"));
            }
            _ => panic!("Expected Quote result, got {:?}", result),
        }
    }

    /// Regression test: `/quote :> `/l` used to silently produce "(no output)"
    /// because /l (and its full names /connections, /listsockets) were routed
    /// to TfCommandResult::ClayCommand, which cmd_quote's backtick-source
    /// handling explicitly cannot capture (see the D-quote-l-capture
    /// investigation). /l is now a genuine TF-native command
    /// (cmd_connections) that returns real text, same as /version already did.
    #[test]
    fn test_cmd_quote_l_captures_connections_output() {
        let mut engine = TfEngine::new();
        engine.current_world = Some("MyMud".to_string());
        engine.world_info_cache.push(WorldInfoCache {
            name: "MyMud".to_string(),
            is_connected: true,
            unseen_lines: 3,
            ..Default::default()
        });

        // Unquoted backtick source, exactly as the user typed it.
        let result = cmd_quote(&mut engine, ":> `/l");
        match result {
            TfCommandResult::Quote { lines, disposition, .. } => {
                assert!(!lines.is_empty(), "expected /l's world-list output to be captured, got no lines");
                assert!(lines.iter().any(|l| l.contains("MyMud")),
                    "expected the connected world's name in the captured output: {:?}", lines);
                assert!(lines.iter().all(|l| l.starts_with(":> ")),
                    "every captured line must carry the prefix: {:?}", lines);
                // ":> " doesn't start with '/', so this must stay a Send (not
                // auto-promoted to Exec).
                assert_eq!(disposition, QuoteDisposition::Send);
            }
            other => panic!("Expected Quote result, got {:?}", other),
        }

        // /connections and /listsockets are full-name aliases for the same command.
        for alias in ["`/connections", "`/listsockets"] {
            let result = cmd_quote(&mut engine, alias);
            match result {
                TfCommandResult::Quote { lines, .. } => {
                    assert!(lines.iter().any(|l| l.contains("MyMud")), "alias {alias} did not capture output");
                }
                other => panic!("alias {alias}: expected Quote result, got {:?}", other),
            }
        }
    }

    /// Same bug, same fix, different command: /fg with no arguments is documented as
    /// "equivalent to /connections" but still routed through ClayCommand until now.
    #[test]
    fn test_cmd_quote_fg_no_args_captures_connections_output() {
        let mut engine = TfEngine::new();
        engine.current_world = Some("MyMud".to_string());
        engine.world_info_cache.push(WorldInfoCache {
            name: "MyMud".to_string(),
            is_connected: true,
            ..Default::default()
        });

        let result = cmd_quote(&mut engine, "`/fg");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert!(lines.iter().any(|l| l.contains("MyMud")),
                    "expected /fg (no args) to capture the same output as /connections: {:?}", lines);
            }
            other => panic!("Expected Quote result, got {:?}", other),
        }

        // /fg <world> is a real switch action, not informational - must stay uncapturable
        // (still routes to ClayCommand), unlike the no-args form above.
        let result = cmd_quote(&mut engine, "`/fg MyMud");
        match result {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.starts_with("(no output)"), "expected /fg <world> to stay uncapturable: {:?}", msg);
            }
            other => panic!("Expected an uncaptured '(no output)' result for /fg <world>, got {:?}", other),
        }
    }

    /// Same bug, same fix: /ban (list banned hosts) was never in TF's own command
    /// table at all, so it always fell through to the generic ClayCommand fallback -
    /// same silent-drop symptom as /l before that fix.
    #[test]
    fn test_cmd_quote_ban_captures_banlist_output() {
        let mut engine = TfEngine::new();

        // Empty ban list: still real captured text, just the "no bans" message.
        let result = cmd_quote(&mut engine, "`/ban");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert!(lines.iter().any(|l| l.contains("No hosts are currently banned")),
                    "expected the empty-ban-list message to be captured: {:?}", lines);
            }
            other => panic!("Expected Quote result, got {:?}", other),
        }

        // Populated ban list: the banned host must appear in the captured text.
        engine.ban_info_cache.push(("1.2.3.4".to_string(), "temporary".to_string(), "too many failed logins".to_string()));
        let result = cmd_quote(&mut engine, "`/ban");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert!(lines.iter().any(|l| l.contains("1.2.3.4")),
                    "expected the banned host to appear in captured output: {:?}", lines);
            }
            other => panic!("Expected Quote result, got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_quote_backtick_captures_tf_command_output_finding_14() {
        // The exact mechanism grep.tf's /fgrep relies on: `` `"<TF_cmd>" `` must capture
        // the command's actual output (execute it through the engine and collect every
        // Success(Some) line), not produce an empty/uncaptured invocation.
        let mut engine = TfEngine::new();

        let result = cmd_quote(&mut engine, "`/echo banana");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert_eq!(lines, vec!["banana".to_string()]);
            }
            other => panic!("Expected Quote result, got {:?}", other),
        }

        // A multi-line Success(Some) message must be split into one generated line each.
        let result = cmd_quote(&mut engine, "`/echo one\ntwo\nthree");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert_eq!(lines, vec!["one".to_string(), "two".to_string(), "three".to_string()]);
            }
            other => panic!("Expected Quote result, got {:?}", other),
        }

        // <pre> is prepended to every captured line.
        let result = cmd_quote(&mut engine, "captured: `/echo banana");
        match result {
            TfCommandResult::Quote { lines, .. } => {
                assert_eq!(lines, vec!["captured: banana".to_string()]);
            }
            other => panic!("Expected Quote result, got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_quote_hash_source_is_recall_shorthand() {
        // "/help quote"'s own "nearly equivalent pairs": `/quote <opts> `/recall <args>`
        // == `/quote <opts> #<args>` - the '#' source must reach the SAME Recall-result
        // path the explicit backtick form does (needs the caller's output_lines, so it
        // comes back as recall_opts, not already-resolved lines).
        let mut engine = TfEngine::new();

        let result = cmd_quote(&mut engine, "#\"combat\"");
        match result {
            TfCommandResult::Quote { recall_opts: Some((opts, prefix)), lines, .. } => {
                assert_eq!(opts.pattern.as_deref(), Some("combat"));
                assert_eq!(prefix, "");
                assert!(lines.is_empty());
            }
            other => panic!("Expected Quote with recall_opts, got {:?}", other),
        }

        // Explicit backtick spelling of the exact same thing must behave identically.
        let result = cmd_quote(&mut engine, "`\"/recall combat\"");
        match result {
            TfCommandResult::Quote { recall_opts: Some((opts, _)), .. } => {
                assert_eq!(opts.pattern.as_deref(), Some("combat"));
            }
            other => panic!("Expected Quote with recall_opts, got {:?}", other),
        }
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

    const CRYPT_TF: &str = r#";
;
; encrypt.tf
;    This is an implimentation of some really simple encryption.
;    Its probably slightly more effective then say rot13. Don't
;    trust this code to deter dedicated people. Trust this code
;    to baffle newbies.
;
; Useage:
;    /e <text>                 Encrypts <text> using the password set by
;                              the /passwd command.
;    /passwd <text>            Set the password to <text>.
;

/def random = /echo -- %R

/def passwd = \
   /let i=0%;\
   /let eol=$[strlen({*})]%;\
   /while (i < eol) \
      /let char=$[ascii(substr({*},i,1))]%;\
      /if (char >= 32) \
         /if (char <=  126) \
            /let tmppwd=%tmppwd$[char(char)]%;\
         /endif%;\
      /endif%;\
      /test ++i%;\
   /done%;\
   /def crypt_pwd=%tmppwd%;\

/def encrypt = \
   /let i=0%;\
    /while (i < strlen({*})) \
      /let char=$[mod(ascii(substr({*},i,1)) + \
         ascii(substr(${crypt_pwd},mod(i,strlen(${crypt_pwd})),1)) - \
         64,95)+32]%;\
      /let printable=x$(/makeprintable %{i} %{char})x%;\
      /let result=%result$[substr(printable,1,strlen(printable)-2)]%;\
      /test ++i%;\
   /done%;\
   /echo -- %result%;\

/def decrypt = \
   /let i=1%;\
   /let j=0%;\
   /while (i < (strlen({-1}) - 1)) \
      /let char=$[ascii(substr({-1},i,1))]%;\
      /if ({1} & char == 92) \
         /let char=$[ascii(substr({-1},++i,1))]%;\
      /elseif ({1} & (substr({-1},i,2)) =/ "%b") \
         /let char=32%;\
         /test ++i%;\
      /endif%;\
      /let code=$[substr(code,0,strlen(code)-1)]$[char(mod({char} - \
         ascii(substr(${crypt_pwd},j,1)) + 190,95) + 32)]a%;\
      /let j=$[mod(++j,strlen(${crypt_pwd}))]%;\
      /test ++i%;\
   /done%;\
   /echo -- $[substr(code,0,strlen(code)-1)]

/def makeprintable = \
   /if ({-1} == 32) \
      /echo -- \%b%;\
   /elseif ({1} == 0) \
      /echo -- $[char({-1})]%;\
   /elseif ({-1}==92 | {-1}==91 | {-1}==93 | {-1}==123 | {-1}==125 | {-1}==37) \
      /echo -- \\$[char({-1})]%;\
   /else \
      /echo -- $[char({-1})]%;\
   /endif

/def e = \
   /echo -- say \\$(/encrypt %*3.14)%;\
   say \\$(/encrypt %*3.14)

/def p = \
   +pub \\$(/encrypt %*3.14)

/def -p5000 -mregexp -t' (say|says|says,|say,) "(.*)"$$' \
      listen_mush = \
   /if (substr({P2},0,1) =~ "\\") \
   	/let dcrypt=$(/decrypt 1 x%P2x)%;\
   /else \
        /let dcrypt=$(/decrypt 0 x%P2x)%;\
   /endif%;\
   /if (dcrypt =/ "*3.14") \
      /if (dcrypt =/ "\:*") \
         /echo -w${world_name} -ag -- %*%;\
         /substitute -aCred -- %% * %PL $[substr(dcrypt,strstr(dcrypt,":")+1,\
            strlen(dcrypt)-5)]%;\
      /else \
         /echo -w${world_name} -ag -- %*%;\
         /substitute -aCred -- %% %PL %P1 \
            "$[substr(dcrypt,0,strlen(dcrypt)-4)]"%;\
      /endif%;\
   /endif

;/passwd welcometoencryptionpartyongarth
/passwd Fredrik
; /passwd test
"#;

    #[test]
    fn test_load_crypt_tf() {
        let mut engine = TfEngine::new();

        // Load crypt.tf from embedded content
        let result = load_from_str(&mut engine, CRYPT_TF);

        match &result {
            TfCommandResult::Success(_) => {
                // Good - loaded successfully
            }
            TfCommandResult::Error(e) => {
                // Some errors might be OK (e.g., from executing /passwd)
                // but check it's not a fundamental failure
                panic!("Failed to load crypt.tf: {}", e);
            }
            _ => {}
        }

        // Verify macros were defined
        let macro_names: Vec<&str> = engine.macros.iter().map(|m| m.name.as_str()).collect();

        // Check that key macros exist
        assert!(macro_names.contains(&"random"), "random macro not defined");
        assert!(macro_names.contains(&"passwd"), "passwd macro not defined");
        assert!(macro_names.contains(&"encrypt"), "encrypt macro not defined");
        assert!(macro_names.contains(&"decrypt"), "decrypt macro not defined");
        assert!(macro_names.contains(&"makeprintable"), "makeprintable macro not defined");
        assert!(macro_names.contains(&"e"), "e macro not defined");
        assert!(macro_names.contains(&"p"), "p macro not defined");
        assert!(macro_names.contains(&"listen_mush"), "listen_mush macro not defined");

        // Verify that %R was preserved in the random macro body
        let random_macro = engine.macros.iter().find(|m| m.name == "random").unwrap();
        assert!(random_macro.body.contains("%R"),
            "random macro body should contain %R, got: {}", random_macro.body);

        // Verify the "e" macro body contains say command with command substitution
        let e_macro = engine.macros.iter().find(|m| m.name == "e").unwrap();
        assert!(e_macro.body.contains("say"),
            "e macro body should contain 'say', got: {}", e_macro.body);
        assert!(e_macro.body.contains("\\$("),
            "e macro body should have \\$( for command substitution, got: {}", e_macro.body);

        // Verify listen_mush has a trigger pattern
        let listen_macro = engine.macros.iter().find(|m| m.name == "listen_mush").unwrap();
        assert!(listen_macro.trigger.is_some(), "listen_mush should have a trigger");
        assert_eq!(listen_macro.priority, 5000, "listen_mush should have priority 5000");

        // Verify crypt_pwd was set by /passwd Fredrik (line 99 of crypt.tf)
        // The passwd macro uses /while, /if, /let, /def with / prefix - these must work in macro bodies
        let crypt_pwd_macro = engine.macros.iter().find(|m| m.name == "crypt_pwd");
        assert!(crypt_pwd_macro.is_some(), "crypt_pwd macro should be defined after /passwd Fredrik");
        assert_eq!(crypt_pwd_macro.unwrap().body, "Fredrik",
            "crypt_pwd should be 'Fredrik', got: '{}'", crypt_pwd_macro.unwrap().body);

    }

    #[test]
    fn test_capture_groups_in_expressions() {
        // Test that {P1} works in expression context within trigger macros
        let mut engine = TfEngine::new();

        // Define a simple trigger that uses {P1} in expression context
        let result = engine.execute(r#"/def -mregexp -t"^Hello (.+)$" test_capture = /let first=$[substr({P1},0,1)]%;/echo %{first}"#);
        assert!(matches!(result, TfCommandResult::Success(_)),
            "Failed to define trigger: {:?}", result);

        // Verify trigger was stored
        let mac = engine.macros.iter().find(|m| m.name == "test_capture");
        assert!(mac.is_some(), "test_capture macro not found");
        let mac = mac.unwrap();
        assert!(mac.trigger.is_some(), "trigger should be set");
        let trigger = mac.trigger.as_ref().unwrap();
        assert_eq!(trigger.pattern, "^Hello (.+)$",
            "trigger pattern wrong: {}", trigger.pattern);

        // Fire the trigger
        let results = crate::tf::macros::process_triggers(&mut engine, "Hello World", None, None);

        // The trigger should have fired and set P1 = "World"
        // Then {P1} in the expression should resolve, substr gets "W"
        let has_output = results.iter().any(|r| {
            if let TfCommandResult::Success(Some(msg)) = r {
                msg.contains("W")
            } else {
                false
            }
        });
        assert!(has_output, "Expected output containing 'W' from substr({{P1}},0,1), got: {:?}", results);
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        // Test encrypt→decrypt round trip with crypt.tf
        let mut engine = TfEngine::new();

        // Load crypt.tf from embedded content
        let _ = load_from_str(&mut engine, CRYPT_TF);

        // Verify crypt_pwd is set
        let pwd = engine.macros.iter().find(|m| m.name == "crypt_pwd");
        assert!(pwd.is_some(), "crypt_pwd should be set");
        assert_eq!(pwd.unwrap().body, "Fredrik");

        // Encrypt a test string
        let result = engine.execute("/encrypt Hello World3.14");
        let encrypted = match &result {
            TfCommandResult::Success(Some(msg)) => msg.trim().to_string(),
            other => panic!("Expected output from /encrypt, got: {:?}", other),
        };
        assert!(!encrypted.is_empty(), "Encrypted output should not be empty");

        // The encrypted text may contain backslash-escaped characters
        // Decrypt in mode 0 (no backslash handling - for worlds that evaluate escapes)
        // First, strip backslashes to simulate world evaluation
        let unescaped: String = {
            let chars: Vec<char> = encrypted.chars().collect();
            let mut result = String::new();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    // Skip backslash, keep next char
                    result.push(chars[i + 1]);
                    i += 2;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            result
        };

        // Also handle %b → space
        let unescaped = unescaped.replace("%b", " ");

        // Decrypt in mode 0 (no backslash escapes in text)
        let decrypt_cmd = format!("/decrypt 0 x{}x", unescaped);
        let result = engine.execute(&decrypt_cmd);
        let decrypted = match &result {
            TfCommandResult::Success(Some(msg)) => msg.trim().to_string(),
            other => panic!("Expected output from mode-0 /decrypt, got: {:?}", other),
        };
        assert_eq!(decrypted, "Hello World3.14",
            "Mode 0 decrypt should recover original text, got: '{}'", decrypted);

        // Decrypt in mode 1 (backslash escapes preserved - verbatim case)
        let decrypt_cmd = format!("/decrypt 1 x{}x", encrypted);
        let result = engine.execute(&decrypt_cmd);
        let decrypted = match &result {
            TfCommandResult::Success(Some(msg)) => msg.trim().to_string(),
            other => panic!("Expected output from mode-1 /decrypt, got: {:?}", other),
        };
        assert_eq!(decrypted, "Hello World3.14",
            "Mode 1 decrypt should recover original text, got: '{}'", decrypted);
    }

    // ---- resolve_file_path / %TFPATH / %TFLIBDIR search (finding C.2) ----

    /// A fresh, unique scratch directory under the system temp dir, so
    /// parallel `#[test]` threads never collide. Caller removes it.
    fn unique_scratch_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "clay_tf_resolve_{}_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            n
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn test_resolve_file_path_finds_via_tflibdir() {
        let cwd_dir = unique_scratch_dir("cwd_empty");
        let lib_dir = unique_scratch_dir("tflibdir");
        std::fs::write(lib_dir.join("dummy_lib.tf"), "/echo hi\n").unwrap();

        let mut engine = TfEngine::new();
        engine.current_dir = Some(cwd_dir.display().to_string());
        engine.set_global("TFLIBDIR", super::super::TfValue::String(lib_dir.display().to_string()));

        let resolved = resolve_file_path(&engine, "dummy_lib.tf");
        assert_eq!(
            resolved,
            Some(lib_dir.join("dummy_lib.tf").display().to_string()),
            "bare filename should resolve via %TFLIBDIR when not found relative to cwd"
        );

        let _ = std::fs::remove_dir_all(&cwd_dir);
        let _ = std::fs::remove_dir_all(&lib_dir);
    }

    #[test]
    fn test_resolve_file_path_finds_via_tfpath_entry() {
        let cwd_dir = unique_scratch_dir("cwd_empty2");
        let unrelated_dir = unique_scratch_dir("tfpath_miss");
        let path_dir = unique_scratch_dir("tfpath_hit");
        std::fs::write(path_dir.join("dummy_path.tf"), "/echo hi\n").unwrap();

        let mut engine = TfEngine::new();
        engine.current_dir = Some(cwd_dir.display().to_string());
        // TFPATH is colon-separated (TF semantics); the first entry doesn't
        // have the file, the second does.
        engine.set_global(
            "TFPATH",
            super::super::TfValue::String(format!("{}:{}", unrelated_dir.display(), path_dir.display())),
        );

        let resolved = resolve_file_path(&engine, "dummy_path.tf");
        assert_eq!(
            resolved,
            Some(path_dir.join("dummy_path.tf").display().to_string()),
            "bare filename should resolve via a %TFPATH directory when not found relative to cwd"
        );

        let _ = std::fs::remove_dir_all(&cwd_dir);
        let _ = std::fs::remove_dir_all(&unrelated_dir);
        let _ = std::fs::remove_dir_all(&path_dir);
    }

    #[test]
    fn test_resolve_file_path_prefers_cwd_over_tflibdir() {
        let cwd_dir = unique_scratch_dir("cwd_hit");
        let lib_dir = unique_scratch_dir("tflibdir_also_has_it");
        // Same filename in both places - the cwd copy must win.
        std::fs::write(cwd_dir.join("shared_name.tf"), "/echo from-cwd\n").unwrap();
        std::fs::write(lib_dir.join("shared_name.tf"), "/echo from-tflibdir\n").unwrap();

        let mut engine = TfEngine::new();
        engine.current_dir = Some(cwd_dir.display().to_string());
        engine.set_global("TFLIBDIR", super::super::TfValue::String(lib_dir.display().to_string()));

        let resolved = resolve_file_path(&engine, "shared_name.tf");
        assert_eq!(
            resolved,
            Some(cwd_dir.join("shared_name.tf").display().to_string()),
            "a file present relative to the current directory must win over %TFLIBDIR"
        );

        let _ = std::fs::remove_dir_all(&cwd_dir);
        let _ = std::fs::remove_dir_all(&lib_dir);
    }

    #[test]
    fn test_resolve_file_path_with_directory_component_ignores_tfpath() {
        // A filename that already has a directory component ("sub/x.tf")
        // must never be joined onto a %TFPATH/%TFLIBDIR entry - only a bare
        // filename gets that search, matching real TF.
        let cwd_dir = unique_scratch_dir("cwd_empty3");
        let lib_dir = unique_scratch_dir("tflibdir_with_sub");
        std::fs::create_dir_all(lib_dir.join("sub")).unwrap();
        std::fs::write(lib_dir.join("sub").join("x.tf"), "/echo hi\n").unwrap();

        let mut engine = TfEngine::new();
        engine.current_dir = Some(cwd_dir.display().to_string());
        engine.set_global("TFLIBDIR", super::super::TfValue::String(lib_dir.display().to_string()));

        let resolved = resolve_file_path(&engine, "sub/x.tf");
        assert_eq!(resolved, None, "a filename with a directory component must not search %TFLIBDIR");

        let _ = std::fs::remove_dir_all(&cwd_dir);
        let _ = std::fs::remove_dir_all(&lib_dir);
    }

    /// Plan Job 14c: `/KILL <pid>...` processes each pid independently - a
    /// bad pid in the middle doesn't stop the rest from being killed
    /// (verified directly against real tf: `/kill 1 nosuch 2` still kills
    /// both 1 and 2), and is silent on success.
    #[test]
    fn test_kill_multiple_pids_processed_independently() {
        let mut engine = TfEngine::new();
        for id in [1u32, 2u32] {
            engine.processes.push(TfProcess {
                id,
                command: "/echo hi".to_string(),
                interval: Duration::from_secs(10),
                count: Some(1),
                remaining: Some(1),
                next_run: Instant::now(),
                world: None,
                synchronous: false,
                on_prompt: false,
                priority: 0,
                kind: ProcessKind::Repeat,
            });
        }

        match cmd_kill(&mut engine, "1 nosuch 2") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("invalid or missing numeric argument"), "got {msg:?}");
            }
            other => panic!("expected a single diagnostic for the bad token, got {:?}", other),
        }
        assert!(engine.processes.is_empty(), "both 1 and 2 should have been killed");

        // Silent on success.
        engine.processes.push(TfProcess {
            id: 3,
            command: "/echo hi".to_string(),
            interval: Duration::from_secs(10),
            count: Some(1),
            remaining: Some(1),
            next_run: Instant::now(),
            world: None,
            synchronous: false,
            on_prompt: false,
            priority: 0,
            kind: ProcessKind::Repeat,
        });
        assert!(matches!(cmd_kill(&mut engine, "3"), TfCommandResult::Success(None)));
    }

    /// Plan Job 14c: `/ps -r`/`-q` filter by `ProcessKind`; `-s` lists PIDs
    /// only; `-w<world>` filters by world and validates the name.
    #[test]
    fn test_ps_filters() {
        let mut engine = TfEngine::new();
        engine.world_info_cache.push(WorldInfoCache {
            name: "myworld".to_string(),
            ..Default::default()
        });
        engine.processes.push(TfProcess {
            id: 1,
            command: "/echo repeat".to_string(),
            interval: Duration::from_secs(10),
            count: None,
            remaining: None,
            next_run: Instant::now(),
            world: Some("myworld".to_string()),
            synchronous: false,
            on_prompt: false,
            priority: 0,
            kind: ProcessKind::Repeat,
        });
        engine.processes.push(TfProcess {
            id: 2,
            command: "line".to_string(),
            interval: Duration::from_secs(1),
            count: Some(1),
            remaining: Some(1),
            next_run: Instant::now(),
            world: None,
            synchronous: false,
            on_prompt: false,
            priority: 0,
            kind: ProcessKind::Quote,
        });

        match cmd_ps(&engine, "-s") {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, "1 2"),
            other => panic!("expected both pids, got {:?}", other),
        }
        match cmd_ps(&engine, "-r -s") {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, "1"),
            other => panic!("expected only the repeat's pid, got {:?}", other),
        }
        match cmd_ps(&engine, "-q -s") {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, "2"),
            other => panic!("expected only the quote's pid, got {:?}", other),
        }
        match cmd_ps(&engine, "-wmyworld -s") {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, "1"),
            other => panic!("expected only the world-scoped process, got {:?}", other),
        }
        match cmd_ps(&engine, "-wnosuchworld") {
            TfCommandResult::Error(e) => assert!(e.contains("No world nosuchworld")),
            other => panic!("expected a No world diagnostic, got {:?}", other),
        }
        match cmd_ps(&engine, "1") {
            TfCommandResult::Success(Some(s)) => {
                assert!(s.lines().any(|line| line.trim_start().starts_with('1')),
                    "table should contain pid 1's row: {s:?}");
                assert!(!s.contains("line"), "should not contain the other process: {s:?}");
            }
            other => panic!("expected a one-row table, got {:?}", other),
        }
    }

    /// Plan Job 14c: `-w[<world>]` reports/sets the same shared value as
    /// -g/-l/-i (Clay has no separate per-world history size), but still
    /// validates the world name.
    #[test]
    fn test_histsize_dash_w() {
        let mut engine = TfEngine::new();
        engine.world_info_cache.push(WorldInfoCache {
            name: "myworld".to_string(),
            ..Default::default()
        });

        match cmd_histsize(&mut engine, "-wmyworld 500") {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, "histsize=500"),
            other => panic!("got {:?}", other),
        }
        // The shared value really was changed.
        match cmd_histsize(&mut engine, "") {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, "histsize=500"),
            other => panic!("got {:?}", other),
        }
        match cmd_histsize(&mut engine, "-wnosuchworld") {
            TfCommandResult::Error(e) => assert!(e.contains("No world nosuchworld")),
            other => panic!("expected a No world diagnostic, got {:?}", other),
        }
    }

    /// Plan Job 14c: bare `/lcd` reports the current directory; `/cd` with
    /// no argument defaults to `$HOME` instead (`/help lcd`); `/pwd` always
    /// reports the current directory with no wrapper text.
    #[test]
    fn test_lcd_cd_pwd() {
        let mut engine = TfEngine::new();
        let dir = unique_scratch_dir("lcd_cd_pwd");
        let dir_str = dir.display().to_string();

        match cmd_lcd(&mut engine, &dir_str) {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, format!("Current directory is {}", dir_str)),
            other => panic!("got {:?}", other),
        }
        match cmd_lcd(&mut engine, "") {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, format!("Current directory is {}", dir_str)),
            other => panic!("got {:?}", other),
        }
        match cmd_pwd(&mut engine) {
            TfCommandResult::Success(Some(s)) => assert_eq!(s, dir_str, "pwd has no 'Current directory is' wrapper"),
            other => panic!("got {:?}", other),
        }

        // /cd with no argument defaults to $HOME.
        std::env::set_var("HOME", dir.parent().unwrap());
        match cmd_cd(&mut engine, "") {
            TfCommandResult::Success(Some(s)) => {
                assert_eq!(s, format!("Current directory is {}", dir.parent().unwrap().display()));
            }
            other => panic!("got {:?}", other),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plan Job 14c: `/save -mglob -h0 -b{} -t{} ?*` (stdlib's own
    /// `/savedef` idiom) saves only matching, non-invisible macros in
    /// reloadable `/def` form, and a subsequent `/load` reproduces them
    /// exactly - the round trip the plan's own test list calls for.
    #[test]
    fn test_save_mglob_filters_then_load_round_trips() {
        let mut engine = TfEngine::new();
        // Plain def: matches the /savedef-style filter below.
        execute_command(&mut engine, "/def plainmac = /echo plain");
        // Has a trigger: -t{} (glob, matches only an EMPTY trigger) excludes it.
        execute_command(&mut engine, "/def -t\"foo*\" trigmac = /echo trig");
        // Invisible: excluded by default (no -i on /save's own filter).
        execute_command(&mut engine, "/def -i hiddenmac = /echo hidden");

        let dir = unique_scratch_dir("save_load_roundtrip");
        let file = dir.join("saved.tf");
        let file_str = file.display().to_string();

        match cmd_save(&mut engine, &format!("{} -mglob -h0 -b{{}} -t{{}} ?*", file_str)) {
            TfCommandResult::Success(Some(s)) => assert!(s.starts_with("Writing macros to")),
            other => panic!("got {:?}", other),
        }

        let saved = std::fs::read_to_string(&file).expect("saved file must exist");
        assert!(saved.contains("plainmac"), "saved file should contain plainmac: {saved:?}");
        assert!(!saved.contains("trigmac"), "saved file should exclude the triggered macro: {saved:?}");
        assert!(!saved.contains("hiddenmac"), "saved file should exclude the invisible macro: {saved:?}");

        // Round-trip: undef everything, then /load the saved file back.
        macros::undef_macro(&mut engine, "plainmac");
        assert!(!engine.macros.iter().any(|m| m.name == "plainmac"));

        if let TfCommandResult::Error(e) = cmd_load(&mut engine, &file_str) {
            panic!("load of saved file failed: {e}");
        }
        let reloaded = engine.macros.iter().find(|m| m.name == "plainmac")
            .expect("plainmac should be back after /load");
        assert_eq!(reloaded.body, "/echo plain");

        // -a appends rather than overwriting.
        match cmd_save(&mut engine, &format!("-a {} -mglob ?*", file_str)) {
            TfCommandResult::Success(Some(s)) => assert!(s.starts_with("Appending macros to")),
            other => panic!("got {:?}", other),
        }
        let appended = std::fs::read_to_string(&file).unwrap();
        assert!(appended.len() > saved.len(), "-a should have appended, not overwritten");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plan Job 14c: `/exit [n]` aborts `n` (default/floor 1) ENCLOSING
    /// `/load`s (`/help exit`). Verified directly against real tf with this
    /// exact two-level nesting shape: `/exit 2` from the innermost file
    /// aborts both it and its caller, but not a third, outer level.
    /// Verified via variable state, not echoed text: a `/load` that ends via
    /// early exit is silent on success (pre-existing behavior, unrelated to
    /// this job's own `n`-count addition - `load_file_internal`'s own doc
    /// comment on its `exit_remaining` handling), so a level that got
    /// aborted never surfaces its own "before" text either. `/set` side
    /// effects aren't affected by that, and more directly show exactly how
    /// many levels actually ran to completion.
    #[test]
    fn test_exit_n_aborts_n_enclosing_loads() {
        let dir = unique_scratch_dir("exit_n_nested_loads");
        let inner = dir.join("inner.tf");
        let outer = dir.join("outer.tf");
        let top = dir.join("top.tf");
        // /exit 2 from inner.tf must abort both inner.tf and outer.tf, but
        // NOT top.tf - a third enclosing level (verified directly against
        // real tf with this exact 3-level shape).
        std::fs::write(&inner, "/exit 2\n/set inner_ran_after 1\n").unwrap();
        std::fs::write(&outer, format!("/load {}\n/set outer_ran_after 1\n", inner.display())).unwrap();
        std::fs::write(&top, format!("/load {}\n/set top_ran_after 1\n", outer.display())).unwrap();

        let mut engine = TfEngine::new();
        if let TfCommandResult::Error(e) = cmd_load(&mut engine, &top.display().to_string()) {
            panic!("load of top.tf failed: {e}");
        }

        assert!(engine.get_var("inner_ran_after").is_none(), "exit 2 should abort inner.tf");
        assert!(engine.get_var("outer_ran_after").is_none(), "exit 2 should abort outer.tf too");
        assert!(engine.get_var("top_ran_after").is_some(),
            "exit 2 should NOT reach a third enclosing level (top.tf)");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bare `/exit` (n=1, the default) only aborts the file it's actually
    /// in - the caller's own remaining lines still run.
    #[test]
    fn test_bare_exit_aborts_only_the_current_file() {
        let dir = unique_scratch_dir("exit_default_one_level");
        let inner = dir.join("inner.tf");
        let outer = dir.join("outer.tf");
        std::fs::write(&inner, "/exit\n/set inner_ran_after 1\n").unwrap();
        std::fs::write(&outer, format!("/load {}\n/set outer_ran_after 1\n", inner.display())).unwrap();

        let mut engine = TfEngine::new();
        if let TfCommandResult::Error(e) = cmd_load(&mut engine, &outer.display().to_string()) {
            panic!("load of outer.tf failed: {e}");
        }

        assert!(engine.get_var("inner_ran_after").is_none(), "bare /exit should abort the inner file");
        assert!(engine.get_var("outer_ran_after").is_some(), "bare /exit should NOT abort the outer file");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plan Job 14c / finding 32: `/unworld <name>...` bounces to Clay's
    /// native `Command::RemoveWorld` (this engine-only function has no
    /// `&mut App` to actually delete a world with) - the previous
    /// implementation bounced to a `/close` command that never existed, so
    /// /unworld silently did nothing at all before this job.
    #[test]
    fn test_cmd_unworld_bounces_to_native_removeworld_command() {
        match cmd_unworld("foo bar") {
            TfCommandResult::ClayCommand(cmd) => assert_eq!(cmd, "/unworld foo bar"),
            other => panic!("got {:?}", other),
        }
        assert!(matches!(cmd_unworld(""), TfCommandResult::Error(_)));
    }

    // ========================================================================
    // Job 15: missing builtins + stdlib one-liners
    // ========================================================================

    #[test]
    fn test_cmd_ismacro_sets_last_matching_sequence_number_or_zero() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("foo = /echo hi").unwrap());
        let foo_seq = engine.add_macro(macros::parse_def("bar = /echo bye").unwrap());

        assert!(matches!(cmd_ismacro(&mut engine, "bar"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(foo_seq as i64));

        assert!(matches!(cmd_ismacro(&mut engine, "no_such_macro"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));
    }

    /// Finding 28's own reproduction: `-msimple -ib'<pattern>'` filters by bind text.
    #[test]
    fn test_cmd_ismacro_bind_filter_matches_kbbind_idiom() {
        let mut engine = TfEngine::new();
        // This engine's very first macro legitimately gets sequence number 0, same as
        // "no match" - so a second, later macro is used here to keep the assertion
        // unambiguous (a real /ismacro caller can't tell "matched seq 0" from "no
        // match" either, but that's a real tf property, not a test artifact).
        engine.add_macro(macros::parse_def("placeholder = /echo unrelated").unwrap());
        let bound_seq = engine.add_macro(macros::parse_def("-ib'^R' = /dokey refresh").unwrap());

        cmd_ismacro(&mut engine, "-msimple -ib'^R'");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(bound_seq as i64), "a macro bound to ^R should match");

        cmd_ismacro(&mut engine, "-msimple -ib'^Q'");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0), "nothing is bound to ^Q");
    }

    #[test]
    fn test_cmd_isvar_any_scope_no_output() {
        let mut engine = TfEngine::new();
        engine.set_global("HOME", crate::tf::TfValue::String("/home/x".to_string()));
        assert!(matches!(cmd_isvar(&mut engine, "HOME"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));

        assert!(matches!(cmd_isvar(&mut engine, "no_such_var_xyz"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));

        engine.push_scope();
        engine.set_local("localonly", crate::tf::TfValue::Integer(1));
        cmd_isvar(&mut engine, "localonly");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1), "isvar must see local scope too");
    }

    #[test]
    fn test_cmd_features_list_and_single_name() {
        let mut engine = TfEngine::new();
        match cmd_features(&mut engine, "") {
            TfCommandResult::Success(Some(s)) => {
                assert!(s.contains("+ssl"));
                assert!(s.contains("-core"));
            }
            other => panic!("expected the full list: {other:?}"),
        }

        assert!(matches!(cmd_features(&mut engine, "ssl"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));

        cmd_features(&mut engine, "CORE");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0), "case-insensitive, and core is off");

        cmd_features(&mut engine, "no_such_feature");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));
    }

    #[test]
    fn test_cmd_true_false_null_are_silent_and_set_status() {
        let mut engine = TfEngine::new();
        assert!(matches!(cmd_true(&mut engine, ""), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));
        assert!(matches!(cmd_false(&mut engine, ""), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));
        assert!(matches!(cmd_null(&mut engine, ""), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));
    }

    #[test]
    fn test_cmd_first_rest_last_nth() {
        assert!(matches!(cmd_first("a b c"), TfCommandResult::Result(ref s) if s == "a"));
        assert!(matches!(cmd_rest("a b c"), TfCommandResult::Result(ref s) if s == "b c"));
        assert!(matches!(cmd_last("a b c"), TfCommandResult::Result(ref s) if s == "c"));
        assert!(matches!(cmd_nth("2 a b c"), TfCommandResult::Result(ref s) if s == "b"));
        // Edge cases matching real tf: a single word has empty /rest; nth with a
        // non-positive or out-of-range n gives "".
        assert!(matches!(cmd_rest("only"), TfCommandResult::Result(ref s) if s.is_empty()));
        assert!(matches!(cmd_nth("0 a b c"), TfCommandResult::Result(ref s) if s.is_empty()));
        assert!(matches!(cmd_nth("99 a b c"), TfCommandResult::Result(ref s) if s.is_empty()));
    }

    #[test]
    fn test_cmd_ver_returns_bare_version_constant() {
        assert!(matches!(cmd_ver(), TfCommandResult::Result(ref s) if s == crate::VERSION));
    }

    #[test]
    fn test_cmd_nogag_no_arg_disables_and_sets_gag_zero() {
        let mut engine = TfEngine::new();
        match cmd_nogag(&mut engine, "") {
            TfCommandResult::Success(Some(ref s)) => assert_eq!(s, "Gags disabled."),
            other => panic!("got {:?}", other),
        }
        assert_eq!(engine.get_var("gag").and_then(|v| v.to_int()), Some(0));
    }

    #[test]
    fn test_cmd_nogag_with_pattern_delegates_to_untrig() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-ag -t'foo*' = /echo gagged").unwrap());
        cmd_nogag(&mut engine, "foo*");
        assert!(engine.macros.is_empty(), "the gag-attributed trigger matching the pattern should be removed");
    }

    #[test]
    fn test_cmd_sys_runs_shell_and_sets_real_exit_status() {
        let mut engine = TfEngine::new();
        match cmd_sys(&mut engine, "echo hi") {
            TfCommandResult::Success(Some(ref s)) => assert_eq!(s, "hi"),
            other => panic!("got {:?}", other),
        }
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));

        cmd_sys(&mut engine, "exit 7");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(7), "%? must be the real exit code, not a 0/1 boolean");
    }

    #[test]
    fn test_cmd_restrict_report_and_monotonic_raise() {
        let mut engine = TfEngine::new();
        match cmd_restrict(&mut engine, "") {
            TfCommandResult::Success(Some(ref s)) => assert_eq!(s, "restriction level: none"),
            other => panic!("got {:?}", other),
        }

        assert!(matches!(cmd_restrict(&mut engine, "FILE"), TfCommandResult::Success(None)));
        match cmd_restrict(&mut engine, "") {
            TfCommandResult::Success(Some(ref s)) => assert_eq!(s, "restriction level: file"),
            other => panic!("got {:?}", other),
        }

        // Never lowered, even by an explicit attempt to set a lower level.
        cmd_restrict(&mut engine, "SHELL");
        assert_eq!(engine.restrict_level, crate::tf::RestrictLevel::File, "restrict must never be lowered");

        cmd_restrict(&mut engine, "WORLD");
        assert_eq!(engine.restrict_level, crate::tf::RestrictLevel::World);

        assert!(matches!(cmd_restrict(&mut engine, "bogus"), TfCommandResult::Error(_)));
    }

    #[test]
    fn test_restrict_shell_blocks_sh_and_sys() {
        let mut engine = TfEngine::new();
        cmd_restrict(&mut engine, "SHELL");
        assert!(matches!(cmd_sh(&mut engine, "echo hi"), TfCommandResult::Error(ref e) if e == "SH: restricted"));
        assert!(matches!(cmd_sys(&mut engine, "echo hi"), TfCommandResult::Error(ref e) if e == "SYS: restricted"));
    }

    #[test]
    fn test_restrict_file_blocks_load_save_lcd() {
        let mut engine = TfEngine::new();
        cmd_restrict(&mut engine, "FILE");
        assert!(matches!(cmd_load(&mut engine, "foo.tf"), TfCommandResult::Error(ref e) if e == "LOAD: restricted"));
        assert!(matches!(cmd_save(&mut engine, "foo.tf"), TfCommandResult::Error(ref e) if e == "SAVE: restricted"));
        assert!(matches!(cmd_lcd(&mut engine, "/tmp"), TfCommandResult::Error(ref e) if e == "LCD: restricted"));
        assert!(matches!(cmd_lcd(&mut engine, ""), TfCommandResult::Error(ref e) if e == "LCD: restricted"), "the report form is restricted too");
        // /restrict FILE implies SHELL.
        assert!(matches!(cmd_sh(&mut engine, "echo hi"), TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_core_reports_not_supported() {
        match cmd_core() {
            TfCommandResult::Success(Some(ref s)) => assert!(s.contains("Not supported in Clay")),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_xtitle_queues_and_requires_text() {
        let mut engine = TfEngine::new();
        assert!(matches!(cmd_xtitle(&mut engine, "My Title"), TfCommandResult::Success(None)));
        assert_eq!(engine.pending_xtitle, Some("My Title".to_string()));
        assert!(matches!(cmd_xtitle(&mut engine, ""), TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_more_valid_values_and_error() {
        let mut engine = TfEngine::new();
        assert!(matches!(cmd_more(&mut engine, "on"), TfCommandResult::Success(None)));
        assert_eq!(engine.pending_more_mode, Some(true));
        assert_eq!(engine.get_var("more").and_then(|v| v.to_int()), Some(1));

        assert!(matches!(cmd_more(&mut engine, "0"), TfCommandResult::Success(None)));
        assert_eq!(engine.pending_more_mode, Some(false));

        match cmd_more(&mut engine, "") {
            TfCommandResult::Error(e) => assert!(e.contains("Invalid more value")),
            other => panic!("bare /more should error like real tf: {other:?}"),
        }
    }

    #[test]
    fn test_cmd_wrap_numeric_vs_on_off() {
        let mut engine = TfEngine::new();
        assert!(matches!(cmd_wrap(&mut engine, "12"), TfCommandResult::Success(None)));
        assert_eq!(engine.pending_wrapspace, Some(12));
        assert_eq!(engine.get_var("wrapsize").and_then(|v| v.to_int()), Some(12));
        assert_eq!(engine.get_var("wrap").and_then(|v| v.to_int()), Some(1));

        engine.pending_wrapspace.take(); // drain what the numeric call above queued
        assert!(matches!(cmd_wrap(&mut engine, "off"), TfCommandResult::Success(None)));
        assert_eq!(engine.pending_wrapspace, None, "on/off has no Clay-side wrap-width equivalent to queue");
        assert_eq!(engine.get_var("wrap").map(|v| v.to_string_value()), Some("off".to_string()));

        assert!(matches!(cmd_wrap(&mut engine, ""), TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_limit_family_queues_the_right_pending_op() {
        let mut engine = TfEngine::new();
        cmd_limit(&mut engine, "-v -a -msimple foo");
        match engine.pending_limit_op.take() {
            Some(crate::tf::PendingLimitOp::Apply { pattern, invert, attrs_only, style }) => {
                assert_eq!(pattern.as_deref(), Some("foo"));
                assert!(invert);
                assert!(attrs_only);
                assert_eq!(style, crate::tf::TfMatchMode::Simple);
            }
            other => panic!("got {:?}", other),
        }

        cmd_limit(&mut engine, "");
        assert!(matches!(engine.pending_limit_op.take(), Some(crate::tf::PendingLimitOp::Report)));

        cmd_unlimit(&mut engine, "");
        assert!(matches!(engine.pending_limit_op.take(), Some(crate::tf::PendingLimitOp::Clear)));

        cmd_relimit(&mut engine, "");
        assert!(matches!(engine.pending_limit_op.take(), Some(crate::tf::PendingLimitOp::Reapply)));

        assert!(matches!(cmd_limit(&mut engine, "-mbogus x"), TfCommandResult::Error(_)));
    }
}

