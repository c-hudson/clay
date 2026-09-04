//! Hooks and keybindings for TinyFugue compatibility.
//!
//! Implements:
//! - Hook events (CONNECT, DISCONNECT, LOGIN, PROMPT, etc.)
//! - /hook, /unhook commands for direct hook management
//! - /bind, /unbind commands for keybindings
//! - Key name parsing (F1, ^A, etc.)

use super::{TfEngine, TfHookEvent, TfCommandResult, TfMacro};
use super::macros;

/// What a call to `fire_hook` actually did - beyond the executed macros' own
/// `TfCommandResult`s, callers need to know whether anything fired at all, and
/// (for SEND specifically) whether a *non-quiet* match occurred, since that's
/// what TF says decides whether the original text should still be sent (see
/// `/help hooks`: "If a SEND hook matches the text that would be sent, the
/// text is not sent (unless the hook was defined with /def -q)").
#[derive(Debug, Default)]
pub struct HookOutcome {
    /// A macro actually fired (pattern matched, shots remaining, etc).
    pub matched_any: bool,
    /// A macro that fired was NOT `-q` quiet.
    pub matched_non_quiet: bool,
    /// The `gag` attribute of the FIRST macro that fired, if any fired at all.
    /// Used by `parser::cmd_trigger`'s `-h` branch to decide whether to suppress
    /// its own real-tf-verified "echo the raw argument text" fallback (see
    /// `TfHookEvent::is_world_stream_event`'s doc comment) - mirrors real TF's
    /// "[the message] will be displayed with the attributes of the hook" rule.
    pub first_fired_gagged: Option<bool>,
    /// Results from every macro that fired, in firing order.
    pub results: Vec<TfCommandResult>,
}

/// Fire every macro hooked to `event` whose pattern (if any) matches `arg`, TF's
/// own priority rules (see `/help priority`): highest priority first; a
/// fall-through (`-F`) match keeps searching lower priorities, a non-fall-through
/// match stops the search (TF picks randomly among equal-priority
/// non-fall-throughs; Clay keeps the simpler "first" rule `macros::
/// process_triggers` already uses for line triggers, for the same reason).
///
/// `arg` is the event's own argument text (e.g. a world name for CONNECT, the
/// line being sent for SEND) - see the per-event doc comments at each call site.
/// It is matched against a macro's `-h"EVENT pattern"` pattern exactly like a
/// trigger (`macros::match_trigger`): same `-m` matching style, same capture
/// groups (populate `%P0../%PL/%PR`). A macro's own positional parameters
/// (`%1../%*/%#`) come from `arg`'s whitespace-separated words, not from the
/// pattern's captures - TF has no "command line" for a hook the way `/name args`
/// has one, so the argument text stands in for it.
///
/// Scans `engine.macros` directly (every hook-bearing macro, `/hook`-created or
/// plain `/def -h...`, named or nameless, has `.hook` set - see `cmd_def`) rather
/// than a separate by-name registry: that used to exist alongside this scan and
/// carefully skip-if-already-executed each other, which is strictly more state
/// for the same result and only ever covered *named* macros.
pub fn fire_hook(engine: &mut TfEngine, event: TfHookEvent, arg: &str) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    let mut to_remove = Vec::new();

    let mut idxs: Vec<usize> = engine.macros.iter().enumerate()
        .filter(|(_, m)| m.hook == Some(event))
        .map(|(i, _)| i)
        .collect();
    idxs.sort_by(|&a, &b| engine.macros[b].priority.cmp(&engine.macros[a].priority));

    for idx in idxs {
        if idx >= engine.macros.len() {
            continue; // an earlier macro's own body could have /purge'd this one
        }
        let macro_def = engine.macros[idx].clone();

        // -T world-type restriction: fire_hook has no world context to test the
        // pattern against, so a -T-restricted hook macro is conservatively treated
        // as "never matches" here - see macros::world_type_matches's doc comment.
        if macro_def.world_type.is_some() {
            continue;
        }
        if let Some(remaining) = macro_def.shots_remaining {
            if remaining == 0 {
                continue;
            }
        }

        let hook_trigger = compile_hook_pattern(&macro_def);
        let trigger_match = match &hook_trigger {
            Some(t) => match macros::match_trigger(t, arg) {
                Some(m) => Some(m),
                None => continue, // pattern didn't match this event's argument text
            },
            None => None, // no pattern: matches every occurrence (see /help hook)
        };

        outcome.matched_any = true;
        if outcome.first_fired_gagged.is_none() {
            outcome.first_fired_gagged = Some(macro_def.attributes.gag);
        }
        if !macro_def.quiet {
            outcome.matched_non_quiet = true;
        }

        let words: Vec<&str> = arg.split_whitespace().collect();
        let exec_results = macros::execute_macro(engine, &macro_def, &words, trigger_match.as_ref());
        outcome.results.extend(exec_results);

        // Decrement shots (match by sequence_number, not index - mirrors
        // process_triggers's own reasoning: a nameless macro has name == "", and
        // the macro's own body may have added/removed macros of its own).
        if let Some(cur_idx) = engine.macros.iter().position(|m| m.sequence_number == macro_def.sequence_number) {
            if let Some(ref mut remaining) = engine.macros[cur_idx].shots_remaining {
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    to_remove.push(cur_idx);
                }
            }
        }

        if !macro_def.fall_through {
            break;
        }
    }

    to_remove.sort_unstable_by(|a, b| b.cmp(a));
    to_remove.dedup();
    for idx in to_remove {
        if idx < engine.macros.len() {
            engine.macros.remove(idx);
        }
    }

    outcome
}

/// Compile a macro's `-h"EVENT pattern"` pattern (if any) into a `TfTrigger`,
/// the same way a `-t` pattern is compiled - same `-m` matching style (shared
/// with `-t`/`-T`, per `macros::world_type_matches`'s identical convention).
fn compile_hook_pattern(macro_def: &TfMacro) -> Option<super::TfTrigger> {
    let pattern = macro_def.hook_pattern.as_ref()?;
    let mode = macro_def.trigger.as_ref().map(|t| t.match_mode).unwrap_or_default();
    let compiled = macros::compile_pattern(pattern, mode).ok().flatten();
    Some(super::TfTrigger { pattern: pattern.clone(), match_mode: mode, compiled })
}

/// Remove hooked macros for `event`. `pattern`: `None` removes every macro
/// hooked to `event` regardless of its own `-h` pattern (real TF: bare
/// `/unhook EVENT`); `Some(pat)` removes only those whose OWN hook pattern is
/// exactly `pat` (verified against real tf: a macro hooked with pattern "foo*"
/// survives `/unhook SEND foo`, but not `/unhook SEND foo*` - the two pattern
/// STRINGS are compared for equality, not one run as a matcher against the
/// other). Returns the number of macros removed.
pub fn unregister_hooks(engine: &mut TfEngine, event: TfHookEvent, pattern: Option<&str>) -> usize {
    let before = engine.macros.len();
    engine.macros.retain(|m| {
        if m.hook != Some(event) {
            return true;
        }
        match pattern {
            None => false,
            Some(pat) => m.hook_pattern.as_deref() != Some(pat),
        }
    });
    before - engine.macros.len()
}

/// Parse a key name (or a chord of them) into its canonical string form.
///
/// This is a thin wrapper over the shared grammar in `crate::keynames` -
/// see that module's doc comment for exactly what's accepted (named keys,
/// `^X` control chars, `Ctrl-`/`Shift-`/`Alt-`/`Esc-` prefixes, chords like
/// `^X^R`, and TF's raw forms: `^[b`, `^[[A`, `\033`, `\0x1B`, `\27`, `\e`).
/// `bind_key`/`unbind_key`/`get_binding` all normalize through this, so
/// `/bind`, `/def -b`/`-B`, and a pressed key all agree on the same string
/// for the same logical key (plan finding A / step P2.1).
pub fn parse_key_name(name: &str) -> Result<String, String> {
    crate::keynames::parse_key_name(name).map(|seq| seq.canonical())
}

/// Register a keybinding directly in the `engine.keybindings` lookup cache,
/// with no backing macro. Real `/bind` no longer calls this - `parser::
/// cmd_bind` builds a real nameless `-b` `TfMacro` and registers it through
/// `parser::apply_macro_def` instead (finding 40 / plan Job 21), so the
/// command text defers substitution to keypress the same way `/def -b`'s
/// body always has. This low-level, cache-only form is kept for callers (and
/// this file's own tests) that just want the fast-lookup entry `get_binding`/
/// `chords::resolve_key_name` read, without needing a real macro in
/// `engine.macros` to back it (`/list -b`, `/purge`, etc. only ever see a
/// binding created through `apply_macro_def`).
pub fn bind_key(engine: &mut TfEngine, key: &str, command: String) -> Result<(), String> {
    let normalized = parse_key_name(key)?;
    engine.keybindings.insert(normalized, command);
    Ok(())
}

/// Remove a keybinding: real TF's own text is "Removes a macro with the
/// keybinding <sequence>" (`/help unbind`) - since finding 40 made `/bind`
/// create a real (nameless) macro instead of a bare cache entry, `/unbind`
/// must remove that macro too, not just the `engine.keybindings` lookup-cache
/// entry that speeds up every keypress. Removes the first macro found with a
/// matching `.keybinding` - real TF's own text is singular ("a macro"), and
/// Clay has no stacking-order concept beyond definition order to prefer one
/// over another when more than one macro happens to share a key (an edge
/// case TinyFugue's own kbbind.tf explicitly guards against with its
/// `~bind_if_not_bound` helper, rather than relying on any particular
/// removal order here).
pub fn unbind_key(engine: &mut TfEngine, key: &str) -> Result<bool, String> {
    let normalized = parse_key_name(key)?;
    let had_cache_entry = engine.keybindings.remove(&normalized).is_some();
    let macro_idx = engine.macros.iter().position(|m| m.keybinding.as_deref() == Some(normalized.as_str()));
    if let Some(idx) = macro_idx {
        engine.macros.remove(idx);
    }
    Ok(had_cache_entry || macro_idx.is_some())
}

/// Get command bound to a key - reads the `engine.keybindings` lookup cache
/// that `parser::apply_macro_def` (and the low-level `bind_key` above) keep
/// up to date; this is what `chords::resolve_key_name`/`input_handler.rs`'s
/// `/bind` check reads for every keypress, so it stays a plain hash lookup
/// rather than a scan over `engine.macros`.
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

/// List all hooks (bare `/hook` with no arguments) - every macro with `.hook`
/// set, in priority order, grouped by event with its pattern (if any) and body.
/// Scans `engine.macros` directly rather than a separate by-name registry, so
/// it naturally covers nameless and directly-`/def -h...`-created hooks too,
/// not just ones created through `/hook` itself - see `fire_hook`'s own doc
/// comment for why the direct scan is already the source of truth for firing.
pub fn list_hooks(engine: &TfEngine) -> String {
    let mut hooked: Vec<&super::TfMacro> = engine.macros.iter().filter(|m| m.hook.is_some()).collect();
    if hooked.is_empty() {
        return "No hooks registered.".to_string();
    }
    hooked.sort_by(|a, b| {
        a.hook.map(|h| h.name()).cmp(&b.hook.map(|h| h.name()))
            .then(b.priority.cmp(&a.priority))
            .then(a.sequence_number.cmp(&b.sequence_number))
    });

    let mut output = String::new();
    for m in hooked {
        let event_name = m.hook.expect("filtered above").name();
        match &m.hook_pattern {
            Some(pat) => output.push_str(&format!("{}: {} {} = {}\n", m.sequence_number, event_name, pat, m.body)),
            None => output.push_str(&format!("{}: {} = {}\n", m.sequence_number, event_name, m.body)),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_name_function_keys() {
        // plan P2.1: the shared grammar's raw-sequence table covers F1-F12
        // (TF's own vt100/xterm codes) and the named-key grammar itself goes
        // up to F20 (TF's own kbbind.tf table stops there too).
        assert_eq!(parse_key_name("F1"), Ok("F1".to_string()));
        assert_eq!(parse_key_name("f12"), Ok("F12".to_string()));
        assert_eq!(parse_key_name("f13"), Ok("F13".to_string()));
        assert_eq!(parse_key_name("F20"), Ok("F20".to_string()));
        assert!(parse_key_name("F21").is_err());
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
        // plan P2.1: Alt-<char>/\e<char> mean Esc-<char>, case preserved -
        // NOT "Alt-<UPPERCASED>" (that old uppercase-fold is finding A's
        // Alt-j/Alt-J collision bug). Alt-<NamedKey> is still its own
        // Modified token, see test_parse_key_name_alt_named_key below.
        assert_eq!(parse_key_name("Alt-A"), Ok("Esc-A".to_string()));
        assert_eq!(parse_key_name("Alt-a"), Ok("Esc-a".to_string()));
        assert_eq!(parse_key_name("\\eW"), Ok("Esc-W".to_string()));
    }

    #[test]
    fn test_parse_key_name_alt_named_key() {
        assert_eq!(parse_key_name("Alt-Up"), Ok("Alt-Up".to_string()));
        assert_eq!(parse_key_name("Alt-Down"), Ok("Alt-Down".to_string()));
    }

    #[test]
    fn test_parse_key_name_chords() {
        // plan P2.1: chords are multiple tokens written back to back.
        assert_eq!(parse_key_name("^X^R"), Ok("^X^R".to_string()));
        assert_eq!(parse_key_name("^X["), Ok("^X[".to_string()));
        assert_eq!(parse_key_name("^X^?"), Ok("^X^?".to_string()));
    }

    #[test]
    fn test_parse_key_name_raw_tf_forms() {
        // plan P2.1: TF's raw byte spellings normalise to the canonical
        // grammar - verified against tf-lib/kbbind.tf's own ~keyseq table.
        assert_eq!(parse_key_name("^[b"), Ok("Esc-b".to_string()));
        assert_eq!(parse_key_name("^[[A"), Ok("Up".to_string()));
        assert_eq!(parse_key_name("^[[1;5A"), Ok("Ctrl-Up".to_string()));
        assert_eq!(parse_key_name("\\033"), Ok("Escape".to_string()));
        assert_eq!(parse_key_name("\\0x1B"), Ok("Escape".to_string()));
        assert_eq!(parse_key_name("\\27"), Ok("Escape".to_string()));
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
    fn test_bind_raw_esc_sequence_looked_up_by_canonical_name() {
        // "/bind ^[b = ..." should be reachable by looking up the canonical
        // "Esc-b" - exactly what a pressed Escape-then-b resolves to via
        // keybindings::escape_key_to_name (see tests.rs's full dispatch
        // test for the end-to-end version of this).
        let mut engine = TfEngine::new();
        bind_key(&mut engine, "^[b", "/echo raw".to_string()).unwrap();
        assert_eq!(get_binding(&engine, "Esc-b"), Some("/echo raw".to_string()));
        assert_eq!(get_binding(&engine, "^[b"), Some("/echo raw".to_string()));
    }

    #[test]
    fn test_bind_chord_accepted_and_listed_canonically() {
        let mut engine = TfEngine::new();
        bind_key(&mut engine, "^X^R", "/reload".to_string()).unwrap();
        assert_eq!(get_binding(&engine, "^X^R"), Some("/reload".to_string()));
        assert!(list_bindings(&engine).contains("^X^R = /reload"));
    }

    /// Add a hooked macro directly (bypassing `/def`'s own parsing - see
    /// `macros::parse_def`'s own tests for that half) for tests that just need
    /// some macro registered on an event.
    fn add_hook_macro(engine: &mut TfEngine, event: TfHookEvent, pattern: Option<&str>, body: &str) {
        engine.add_macro(TfMacro {
            body: body.to_string(),
            hook: Some(event),
            hook_pattern: pattern.map(|p| p.to_string()),
            ..Default::default()
        });
    }

    #[test]
    fn test_unregister_hooks_no_pattern_removes_all() {
        let mut engine = TfEngine::new();
        add_hook_macro(&mut engine, TfHookEvent::Connect, None, "say Hello!");
        add_hook_macro(&mut engine, TfHookEvent::Connect, Some("foo*"), "look");

        let count = unregister_hooks(&mut engine, TfHookEvent::Connect, None);
        assert_eq!(count, 2);
        assert!(!engine.macros.iter().any(|m| m.hook == Some(TfHookEvent::Connect)));
    }

    #[test]
    fn test_unregister_hooks_pattern_is_exact_match() {
        // Verified against real tf: a hook defined with pattern "foo*" survives
        // `/unhook SEND foo` (not an exact match) but not `/unhook SEND foo*`.
        let mut engine = TfEngine::new();
        add_hook_macro(&mut engine, TfHookEvent::Send, Some("foo*"), "echo a");
        add_hook_macro(&mut engine, TfHookEvent::Send, Some("bar*"), "echo b");

        assert_eq!(unregister_hooks(&mut engine, TfHookEvent::Send, Some("foo")), 0);
        assert_eq!(engine.macros.len(), 2);

        assert_eq!(unregister_hooks(&mut engine, TfHookEvent::Send, Some("foo*")), 1);
        assert_eq!(engine.macros.len(), 1);
        assert_eq!(engine.macros[0].hook_pattern.as_deref(), Some("bar*"));
    }

    #[test]
    fn test_fire_hook_no_pattern_matches_every_occurrence() {
        let mut engine = TfEngine::new();
        add_hook_macro(&mut engine, TfHookEvent::Connect, None, "/echo connected");
        let outcome = fire_hook(&mut engine, TfHookEvent::Connect, "anyworld");
        assert!(outcome.matched_any);
        assert!(matches!(outcome.results.as_slice(), [TfCommandResult::Success(Some(s))] if s == "connected"));
    }

    #[test]
    fn test_fire_hook_glob_pattern() {
        let mut engine = TfEngine::new();
        add_hook_macro(&mut engine, TfHookEvent::Send, Some("greet*"), "/echo matched");
        assert!(fire_hook(&mut engine, TfHookEvent::Send, "greet bob").matched_any);
        assert!(!fire_hook(&mut engine, TfHookEvent::Send, "bye bob").matched_any);
    }

    #[test]
    fn test_fire_hook_regexp_pattern_and_captures() {
        let mut engine = TfEngine::new();
        engine.add_macro(TfMacro {
            body: "/echo cap=%P1".to_string(),
            hook: Some(TfHookEvent::Send),
            hook_pattern: Some(r"^go (\w+)$".to_string()),
            trigger: Some(super::super::TfTrigger {
                pattern: String::new(),
                match_mode: super::super::TfMatchMode::Regexp,
                compiled: None,
            }),
            ..Default::default()
        });
        let outcome = fire_hook(&mut engine, TfHookEvent::Send, "go north");
        assert!(matches!(outcome.results.as_slice(), [TfCommandResult::Success(Some(s))] if s == "cap=north"));
    }

    #[test]
    fn test_fire_hook_fallthrough_vs_first_match() {
        let mut engine = TfEngine::new();
        // Higher priority, NOT fall-through: should stop the search.
        engine.add_macro(TfMacro {
            body: "/echo first".to_string(),
            hook: Some(TfHookEvent::Send),
            priority: 10,
            ..Default::default()
        });
        engine.add_macro(TfMacro {
            body: "/echo second".to_string(),
            hook: Some(TfHookEvent::Send),
            priority: 5,
            ..Default::default()
        });
        let outcome = fire_hook(&mut engine, TfHookEvent::Send, "x");
        assert_eq!(outcome.results.len(), 1);
        assert!(matches!(&outcome.results[0], TfCommandResult::Success(Some(s)) if s == "first"));

        // Same setup, but the higher-priority one IS fall-through: both should fire.
        let mut engine2 = TfEngine::new();
        engine2.add_macro(TfMacro {
            body: "/echo first".to_string(),
            hook: Some(TfHookEvent::Send),
            priority: 10,
            fall_through: true,
            ..Default::default()
        });
        engine2.add_macro(TfMacro {
            body: "/echo second".to_string(),
            hook: Some(TfHookEvent::Send),
            priority: 5,
            ..Default::default()
        });
        let outcome2 = fire_hook(&mut engine2, TfHookEvent::Send, "x");
        assert_eq!(outcome2.results.len(), 2);
    }

    #[test]
    fn test_fire_hook_quiet_does_not_count_as_non_quiet_match() {
        let mut engine = TfEngine::new();
        engine.add_macro(TfMacro {
            body: "/echo quiet-fired".to_string(),
            hook: Some(TfHookEvent::Send),
            quiet: true,
            ..Default::default()
        });
        let outcome = fire_hook(&mut engine, TfHookEvent::Send, "x");
        assert!(outcome.matched_any);
        assert!(!outcome.matched_non_quiet);

        let mut engine2 = TfEngine::new();
        engine2.add_macro(TfMacro {
            body: "/echo loud-fired".to_string(),
            hook: Some(TfHookEvent::Send),
            ..Default::default()
        });
        let outcome2 = fire_hook(&mut engine2, TfHookEvent::Send, "x");
        assert!(outcome2.matched_non_quiet);
    }

    #[test]
    fn test_all_31_tf_hook_events_parse_case_insensitively() {
        for name in [
            "ACTIVITY", "BAMF", "BGTEXT", "BGTRIG", "CONFAIL", "CONFLICT", "CONNECT",
            "DISCONNECT", "ICONFAIL", "KILL", "LOAD", "LOADFAIL", "LOG", "LOGIN", "MAIL",
            "MORE", "NOMACRO", "PENDING", "PREACTIVITY", "PROCESS", "PROMPT", "PROXY",
            "REDEF", "RESIZE", "SEND", "SHADOW", "SHELL", "SIGHUP", "SIGTERM", "SIGUSR1",
            "SIGUSR2", "WORLD",
        ] {
            assert!(TfHookEvent::parse(name).is_some(), "{name} should parse");
            assert!(TfHookEvent::parse(&name.to_lowercase()).is_some(), "{name} lowercase should parse");
        }
        // Old alias still works (tf-help: "BGTRIG used to be called BACKGROUND").
        assert_eq!(TfHookEvent::parse("BACKGROUND"), Some(TfHookEvent::Bgtrig));
        assert_eq!(TfHookEvent::parse("background"), Some(TfHookEvent::Bgtrig));
        // Clay's own extras are kept.
        assert_eq!(TfHookEvent::parse("gmcp"), Some(TfHookEvent::Gmcp));
        assert_eq!(TfHookEvent::parse("MSDP"), Some(TfHookEvent::Msdp));
        // Unknown names are still an error.
        assert_eq!(TfHookEvent::parse("NOTAREALEVENT"), None);
    }
}
