//! TinyFugue command parser.
//!
//! Parses commands starting with `/` and routes them to appropriate handlers.

use super::{TfCommandResult, TfEngine, TfValue, TfHookEvent, TfMacro, TfMatchMode};
use super::control_flow::{self, ControlState, ControlResult, IfState, WhileState, ForState};
use super::macros;
use super::hooks;
use super::builtins;

/// Split a macro-invocation argument string into positional parameters.
///
/// Real TF (`/help substitution`): "If called with the traditional
/// '/name ...' command syntax, each space-separated word is a positional
/// parameter" - a plain `split_whitespace()`, with no exception. This used
/// to special-case a "rest of the args starts and ends with the same
/// character" shape (meant for a "x...x"-delimited payload, e.g. Clay's own
/// `/decrypt`/`{-1}` idiom in builtins.rs) and fold it into a single
/// argument - but that heuristic fires on perfectly ordinary word lists
/// too, by coincidence, whenever the last word happens to start and end
/// with the same letter as the first remaining word (verified directly:
/// lisp.tf's own "/remove a b a c b" chain, real args ["a","b","a","c","b"],
/// was being collapsed to just ["a", "b a c b"] because the "rest" - "b a c
/// b" - both starts and ends with 'b'). Real TF has no such rule; `{-1}`'s
/// own "join remaining args with a single space" semantics already
/// reconstructs a `/decrypt`-style payload correctly on its own, since
/// `/encrypt`'s own output (builtins.rs's `makeprintable`) never contains a
/// literal space in the first place (it encodes one as the two-character
/// token "%b" specifically to survive ordinary word-splitting) - see
/// `test_encrypt_decrypt_roundtrip`, which still passes without this
/// heuristic.
fn parse_macro_args(args: &str) -> Vec<&str> {
    args.split_whitespace().collect()
}

/// Check if input is a TF command (starts with / prefix)
pub fn is_tf_command(input: &str) -> bool {
    let trimmed = input.trim_start();
    trimmed.starts_with('/')
}

/// Check if a command name (without prefix) is a TF command
pub fn is_tf_command_name(cmd: &str) -> bool {
    // `dokey_<name>` (TF-parity plan Job 21/P2.5): every name kbfunc.tf's own
    // `dokey_<name>` wrapper macros cover, checked as a dynamic prefix rather
    // than one `matches!` arm per name (36 of them) - see
    // `builtins::DOKEY_WRAPPER_NAMES`.
    if let Some(suffix) = cmd.strip_prefix("dokey_") {
        if builtins::is_dokey_wrapper_name(suffix) {
            return true;
        }
    }
    matches!(cmd,
        "help" |
        "set" | "unset" | "let" | "setenv" | "listvar" |
        "echo" | "beep" | "quote" | "substitute" | "escape" | "hilite" | "nohilite" | "partial" | "export" |
        "expr" | "test" | "eval" |
        "if" | "elseif" | "else" | "endif" | "while" | "for" | "done" | "break" |
        "def" | "undef" | "undefn" | "undeft" | "list" | "purge" |
        "bind" | "unbind" | "hook" | "unhook" |
        "load" | "save" | "require" | "loaded" | "lcd" | "cd" | "pwd" | "log" |
        "sh" | "time" | "runtime" | "recall" | "repeat" | "ps" | "kill" |
        "fg" | "trigger" | "input" | "grab" | "gag" | "ungag" | "exit" | "shift" | "bamf" |
        // These are also TF commands (mapped to Clay equivalents)
        "say" |
        "quit" | "dc" | "disconnect" | "world" | "listworlds" |
        "listsockets" | "connections" | "l" | "ban" | "addworld" | "version" |
        // Note: "send" maps to Clay's /send command, but TF's /send has different options
        // so we route it through TF to handle -w flag properly
        "send" |
        // Tier 1: Simple commands
        "toggle" | "return" | "result" | "not" | "suspend" | "dokey" | "histsize" |
        "localecho" | "sub" | "replace" | "tr" | "cat" | "paste" | "endpaste" |
        // Tier 2: Trigger shortcuts
        "trig" | "trigp" | "trigc" | "trigpc" | "untrig" |
        // Tier 3: World management
        "unworld" | "purgeworld" | "saveworld" |
        // Tier 4: Spam detection
        "watchdog" | "watchname" |
        // Tier 5: Stubs
        "telnet" | "finger" | "getfile" | "putfile" | "liststreams" |
        "changes" | "tick" | "recordline" | "edit" |
        // Job 15: missing builtins + stdlib one-liners (plan section B, P1.14)
        "ismacro" | "isvar" | "features" | "then" | "do" | "restrict" | "core" |
        "sys" | "xtitle" | "more" | "wrap" |
        "first" | "rest" | "last" | "nth" | "ver" | "man" | "nogag" |
        "true" | "false" | ":" |
        "limit" | "unlimit" | "relimit"
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
        return aggregate_results_with_engine(engine, results);
    }
    if input.starts_with("__tf_while_eval__\x1F") {
        let results = control_flow::execute_while_encoded(engine, input);
        return aggregate_results_with_engine(engine, results);
    }
    if input.starts_with("__tf_for_eval__\x1F") {
        let results = control_flow::execute_for_encoded(engine, input);
        return aggregate_results_with_engine(engine, results);
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
                aggregate_results_with_engine(engine, results)
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
        let raw_cmd_name = cmd_part.trim_start_matches('/').to_lowercase();
        let args_str = if input.len() > cmd_part.len() {
            input[cmd_part.len()..].trim_start()
        } else {
            ""
        };

        // "/@name" bypasses a same-named user-defined macro and forces the
        // builtin (finding C.6) - TinyFugue's own escape hatch for exactly
        // the precedence flip below. Only a *leading* "@" on the command
        // word itself is stripped.
        let (cmd_name, force_builtin) = match raw_cmd_name.strip_prefix('@') {
            Some(stripped) => (stripped.to_string(), true),
            None => (raw_cmd_name, false),
        };

        // Check for /tf prefix (TF-specific commands that conflict with Clay)
        // e.g., /tfhelp, /tfgag
        if cmd_name.starts_with("tf") && (cmd_name == "tfhelp" || cmd_name == "tfgag") {
            let tf_cmd_name = &cmd_name[2..]; // Strip "tf" prefix
            return execute_tf_command(engine, tf_cmd_name, args_str, skip_substitution);
        }

        // Control-flow keywords are TF syntax, not commands - real TF never
        // looks these up in the macro table at all, so a macro named e.g.
        // "if" can never shadow /if (this is the "at minimum keep Clay's
        // control flow working" case, independent of "/@").
        let is_control_flow_keyword = matches!(
            cmd_name.as_str(),
            "if" | "elseif" | "else" | "endif" | "while" | "for" | "done" | "break"
        );

        // TinyFugue runs a user-defined macro in preference to a builtin of
        // the same name (finding C.6) - "/@name" and the control-flow
        // keywords above are the only things a macro can never shadow.
        // This inverts Clay's historical order (builtin checked first via
        // is_tf_command_name, below), under which a same-named macro could
        // never be reached at all - see finding 16: a native stub like
        // "/tick" always won over a library's own same-named macro.
        if !force_builtin && !is_control_flow_keyword {
            if let Some(macro_def) = engine.macros.iter().find(|m| m.name.eq_ignore_ascii_case(&cmd_name)).cloned() {
                let macro_args: Vec<&str> = parse_macro_args(args_str);
                let results = super::macros::execute_macro(engine, &macro_def, &macro_args, None);
                return aggregate_results_with_engine(engine, results);
            }
        }

        // Check if it's a TF command that should be handled here
        if is_tf_command_name(&cmd_name) {
            return execute_tf_command(engine, &cmd_name, args_str, skip_substitution);
        }

        // Not a TF command or macro - route to Clay
        return TfCommandResult::ClayCommand(input.to_string());
    }

    TfCommandResult::NotTfCommand
}

/// Execute a TF command by name with the given arguments.
/// Handles variable substitution, control flow detection, and dispatch.
fn execute_tf_command(engine: &mut TfEngine, cmd_name: &str, args: &str, skip_substitution: bool) -> TfCommandResult {
    let rest_check = args.trim();
    let lower_cmd = cmd_name.to_lowercase();

    // /if, /while and /for must never have their argument string substituted
    // here, whether or not it happens to contain a literal newline. Each of
    // the three has its own body/condition text that has to reach cmd_if/
    // cmd_while/cmd_for completely raw and be substituted later, at the
    // right granularity, by control_flow.rs itself:
    //   - a multi-line block (macro body, or a single physical line using
    //     "%;" as the separator) needs PER-LINE substitution so a loop body
    //     re-expands %vars on every iteration instead of once up front
    //     (this was already true before P1.7/P1.8 - see the old
    //     "is_inline_control_flow" name this replaces).
    //   - TF's own `/for var min max command` form and the command-form
    //     `/if /cmd%; /then ...` / `/while /cmd%; /do ...` conditions
    //     (P1.7/P1.8, finding C.7/C.8) are NOT multi-line at all when typed
    //     as a single command (e.g. "/for i 1 3 /echo n=%i" has no "\n" or
    //     "%;" anywhere) - substituting eagerly here would expand "%i"
    //     using whatever stale value the variable had *before* the loop's
    //     own /let ever runs, per the plan's explicit warning. So the gate
    //     below no longer requires an embedded "\n": it always defers for
    //     these three commands, the same way /def already always defers
    //     for its own body.
    let is_control_flow_command = matches!(lower_cmd.as_str(), "while" | "for" | "if");

    // Check if this is a /def (or /bind, its exact equivalent per `/help bind` -
    // finding 40 / plan Job 21: `cmd_bind` now builds the very same nameless -b
    // `TfMacro` a `/def -b'<key>' = <command>` would) command - if so, don't
    // substitute variables in the body/command text. It should be stored
    // literally and only substituted when the macro it becomes is actually run.
    let is_def_like_command = matches!(lower_cmd.as_str(), "def" | "bind");

    // /recall and /time args must not be %-substituted: both take a time format
    // (e.g. -t"%H:%M:%S", or /time's own <format> argument) whose "%" sequences are
    // strftime specifiers, not TF variable sigils. TinyFugue itself does not %-expand
    // typed commands, so skipping substitution here is TF-consistent (and matches
    // /time's own "/command" form, finding B's "both" ruling - the nested command must
    // get an ordinary, fresh substitution when `cmd_time` actually dispatches it, not
    // a premature one here using whatever "%" happens to appear in the raw text).
    let is_recall_command = matches!(lower_cmd.as_str(), "recall" | "time");

    // /eval does its OWN substitution pass on its argument (finding B / plan step
    // P1.12) - TF's `eval()`/`/eval` help: "<text> is evaluated as a macro body: it
    // goes through substitution". Since Clay (finding C.12) has not yet stopped
    // substituting every top-level command's arguments unconditionally, /eval must be
    // exempted here the same way /def's body is, or its own substitution pass
    // (`cmd_eval`) would just be a no-op second pass over already-substituted text -
    // which would also make `-s0` (no substitution) impossible to honor, since the
    // substitution would already have happened before `cmd_eval` ever saw the text.
    // /not (finding 13) shares /eval's own "-s<level>, then substitute, then execute"
    // shape and its exemption from the generic substitution pass below - see cmd_not's
    // doc comment.
    let is_eval_command = matches!(lower_cmd.as_str(), "eval" | "not");

    // Perform variable and command substitution before parsing (except for /def bodies, inline control flow,
    // or when called with pre-substituted input from control_flow)
    let substituted;
    let args = if skip_substitution {
        // Already substituted by caller (control_flow)
        rest_check
    } else if is_control_flow_command {
        // Don't substitute - control flow executor will handle per-iteration substitution
        rest_check
    } else if is_recall_command {
        // Don't substitute - strftime % specifiers in time formats must not be eaten
        rest_check
    } else if is_eval_command {
        // Don't substitute - cmd_eval does its own pass (see is_eval_command above)
        rest_check
    } else if is_def_like_command {
        // For /def (and /bind, its equivalent - see is_def_like_command above),
        // only substitute variables in options/key, not in the body/command.
        // Find the = separator and only substitute before it
        if let Some(eq_pos) = rest_check.find('=') {
            let before_eq = &rest_check[..eq_pos];
            let after_eq = &rest_check[eq_pos..];
            let substituted_before = super::variables::substitute_commands(engine, before_eq);
            substituted = format!("{}{}", substituted_before, after_eq);
            substituted.trim()
        } else {
            // No body (just /def or /def with options but no =), substitute normally
            substituted = super::variables::substitute_commands(engine, rest_check);
            substituted.trim()
        }
    } else {
        substituted = super::variables::substitute_commands(engine, rest_check);
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
        "escape" => cmd_escape(engine, args),
        "send" => cmd_send(engine, args),
        "substitute" => cmd_substitute(engine, args),

        // Hilite/trigger shortcuts
        "hilite" => builtins::cmd_hilite(engine, args),
        "nohilite" => builtins::cmd_nohilite(engine, args),
        "partial" => builtins::cmd_partial(engine, args),

        // Variable commands
        "export" => builtins::cmd_export(engine, args),

        // Text-to-speech
        "say" => {
            if args.trim().is_empty() {
                TfCommandResult::Error("/say requires text to speak".to_string())
            } else {
                TfCommandResult::ClayCommand(format!("/say {}", args))
            }
        }

        // Mapped to Clay commands
        "quit" => TfCommandResult::ClayCommand("/quit".to_string()),
        "exit" => builtins::cmd_exit(engine, args),
        // /dc [<world>|-ALL] - forward args (plan Job 14b); this used to hardcode
        // "/disconnect" with no arguments at all, silently dropping a named-world
        // or -ALL target typed at the console.
        "dc" | "disconnect" => TfCommandResult::ClayCommand(format!("/disconnect {}", args).trim_end().to_string()),
        "world" => cmd_world(args),
        "listworlds" => cmd_listworlds(engine, args),
        "listsockets" | "connections" | "l" => cmd_connections(engine, args),
        "ban" => cmd_banlist(engine, args),
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
        "break" => cmd_break(args),

        // Macro commands
        "def" => cmd_def(engine, args),
        "undef" => cmd_undef(engine, args),
        "undefn" => cmd_undefn(engine, args),
        "undeft" => cmd_undeft(engine, args),
        "list" => cmd_list(engine, args),
        "purge" => cmd_purge(engine, args),

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
        "time" => builtins::cmd_time(engine, args),
        "runtime" => builtins::cmd_runtime(engine, args),
        "lcd" => builtins::cmd_lcd(engine, args),
        "cd" => builtins::cmd_cd(engine, args),
        "pwd" => builtins::cmd_pwd(engine),
        "sh" => builtins::cmd_sh(engine, args),
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
        "ps" => builtins::cmd_ps(engine, args),
        "kill" => builtins::cmd_kill(engine, args),

        // World switching
        "fg" => cmd_fg(engine, args),

        // Portal/bamf
        "bamf" => cmd_bamf(engine, args),

        // Argument manipulation
        "shift" => cmd_shift(engine, args),

        // Variable management
        "listvar" => cmd_listvar(engine, args),

        // Trigger commands
        "trigger" => cmd_trigger(engine, args),

        // Input manipulation
        "input" => cmd_input(engine, args),
        "grab" => cmd_grab(engine, args),

        // Tier 1: Simple commands
        "toggle" => builtins::cmd_toggle(engine, args),
        "return" => builtins::cmd_return(engine, args),
        "result" => builtins::cmd_result(engine, args),
        "not" => cmd_not(engine, args),
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

        // Job 15: missing builtins + stdlib one-liners (plan section B, P1.14). Every
        // native-command doc comment lives on its own function in builtins.rs (or,
        // for /not, right here in parser.rs - see cmd_not's own doc comment).
        "ismacro" => builtins::cmd_ismacro(engine, args),
        "isvar" => builtins::cmd_isvar(engine, args),
        "features" => builtins::cmd_features(engine, args),
        // Bare /then or /do (real tf: "unexpected /THEN in outer block") - reachable
        // only when NOT immediately following an /if or /while's own command-form
        // condition (that shape is consumed entirely inside execute_inline_if_block/
        // execute_inline_while_block before dispatch ever sees a bare "/then"/"/do").
        "then" => TfCommandResult::Error("unexpected /THEN in outer block".to_string()),
        "do" => TfCommandResult::Error("unexpected /DO in outer block".to_string()),
        "restrict" => builtins::cmd_restrict(engine, args),
        "core" => builtins::cmd_core(),
        "sys" => builtins::cmd_sys(engine, args),
        "xtitle" => builtins::cmd_xtitle(engine, args),
        "more" => builtins::cmd_more(engine, args),
        "wrap" => builtins::cmd_wrap(engine, args),
        "first" => builtins::cmd_first(args),
        "rest" => builtins::cmd_rest(args),
        "last" => builtins::cmd_last(args),
        "nth" => builtins::cmd_nth(args),
        "ver" => builtins::cmd_ver(),
        "man" => cmd_help(args),
        "nogag" => builtins::cmd_nogag(engine, args),
        "true" => builtins::cmd_true(engine, args),
        "false" => builtins::cmd_false(engine, args),
        ":" => builtins::cmd_null(engine, args),
        "limit" => builtins::cmd_limit(engine, args),
        "unlimit" => builtins::cmd_unlimit(engine, args),
        "relimit" => builtins::cmd_relimit(engine, args),

        // `dokey_<name>` native wrapper commands (plan Job 21/P2.5) - see
        // `is_tf_command_name`'s own matching arm and
        // `builtins::cmd_dokey_named`'s doc comment.
        name if name.strip_prefix("dokey_").is_some_and(builtins::is_dokey_wrapper_name) => {
            builtins::cmd_dokey_named(engine, &name["dokey_".len()..])
        }

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
pub(crate) fn aggregate_results_with_engine(engine: &mut super::TfEngine, results: Vec<TfCommandResult>) -> TfCommandResult {
    let mut messages = vec![];
    let mut has_error = false;
    let mut pending_clay_commands = vec![];

    for result in results {
        match result {
            TfCommandResult::Success(Some(msg)) => messages.push(msg),
            // A `/break` count that wasn't fully absorbed by an enclosing
            // /while or /for in THIS block must keep propagating outward
            // (see control_flow.rs's loop bodies, which only ever push this
            // back into their own results when it still has levels left to
            // unwind) - same "bounce it upward unresolved" treatment as
            // Return/Result below, not a real error. Same caveat as that
            // Return/Result arm's own doc comment too: any Success text
            // collected earlier in THIS SAME aggregation (i.e. echoed during
            // the one loop iteration that also triggered the multi-level
            // break) is discarded here, not carried along with the marker -
            // `TfCommandResult::Error` has no room for both. Side effects
            // (variable state) are unaffected; only display text from that
            // one iteration is lost. A `/break` with no count never reaches
            // this arm at all (the loop that catches it absorbs it locally
            // without ever pushing it into `results`), so this only matters
            // for `/break N` with N > 1.
            TfCommandResult::Error(e) if control_flow::parse_break_marker(&e).is_some() => return TfCommandResult::Error(e),
            TfCommandResult::Error(e) => {
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
            // A /return or /result nested inside an inline /if, /while or
            // /for block (this aggregates one such block's collected
            // per-line results - see aggregate_inline_results) must keep
            // propagating outward as the SAME variant, not be absorbed into
            // a plain Success here: macros::execute_macro's own
            // per-command loop is the only place that sets %? and, for
            // /result, decides whether to echo, and it can only do that if
            // the Return/Result actually reaches it. This was finding
            // C.5's second half - lib_factoral.tf's rfact()/ifact() both
            // call /result from inside a nested /if/elseif/else branch.
            //
            // Any Success text collected earlier in THIS SAME block must
            // still reach the screen, though (real tf: a /return does not
            // retroactively erase what an earlier /echo in the same branch
            // already printed) - at.tf's own usage-message branch is
            // exactly this shape ("/echo -e %% Usage: ...%; ...%; /return
            // 0"), and used to silently lose the whole usage line. There is
            // no room for both a Return/Result's own %?-value payload and
            // this accumulated display text in one `TfCommandResult`, so
            // route it through the same "engine records, something drains
            // it later" side channel `echo()` already uses
            // (`engine.pending_outputs`, drained per top-level line by
            // `builtins::load_lines` and once at the end of a probe line by
            // `script_tests::run_script`/the App's own
            // `commands::process_pending_tf_outputs`) instead of just
            // dropping it.
            r @ (TfCommandResult::Return(_) | TfCommandResult::Result(_)) => {
                if !messages.is_empty() {
                    engine.pending_outputs.push(super::TfOutput {
                        text: messages.join("\n"),
                        attrs: String::new(),
                        world: None,
                    });
                }
                return r;
            }
            // /exit during a /load must keep propagating outward the same way,
            // all the way out to whichever /load actually catches it
            // (`load_file_internal`'s own `exit_early` handling) - see
            // `execute_macro_with_context`'s matching doc comment.
            r @ TfCommandResult::ExitLoad(_) => return r,
            // A /quote generated inside a macro body (e.g. TinyFugue's own grep.tf:
            // "/quote -S /_fgrep `%-1", finding 14) used to just vanish here - this
            // function has no App to resolve a world switch, a scheduled delay, or
            // a backtick /recall against a World's output_lines, and the old `_ =>
            // {}` catch-all silently swallowed the whole Quote instead. Anything
            // this function genuinely cannot finish on its own (a scheduled delay,
            // or a `/recall`-sourced quote) is bounced upward unresolved instead -
            // exactly like a nested /return/Result above - so it keeps propagating
            // out to whichever caller *can* finish it (ultimately the App, at the
            // top level: see main.rs's own TfCommandResult::Quote handling, which
            // never even reaches this function for a plain top-level /quote). A
            // synchronous quote with a plain source, though, this function CAN
            // finish itself: -dsend queues each line into `pending_commands` (same
            // mechanism as a bare SendToMud, carrying the quote's own -w<world>
            // through as that command's target); -decho becomes ordinary echoed
            // text; -dexec runs each generated line back through the engine via
            // `execute_command` (matching what the App's own
            // `handle_ws_quote_result` does for the same disposition) and
            // recursively folds THEIR results back into this same aggregation, so
            // a quote-generated line that is itself a macro (grep.tf's `/_fgrep`)
            // still surfaces its own echoed output correctly.
            TfCommandResult::Quote { lines, disposition, world, delay_secs, recall_opts, strip_ansi } => {
                if recall_opts.is_some() || delay_secs > 0.0 {
                    return TfCommandResult::Quote { lines, disposition, world, delay_secs, recall_opts, strip_ansi };
                }
                match disposition {
                    super::QuoteDisposition::Send => {
                        for line in lines {
                            engine.pending_commands.push(super::TfCommand {
                                command: line,
                                world: world.clone(),
                                no_eol: false,
                            });
                        }
                    }
                    super::QuoteDisposition::Echo => {
                        if !lines.is_empty() {
                            messages.push(lines.join("\n"));
                        }
                    }
                    super::QuoteDisposition::Exec => {
                        let sub_results: Vec<TfCommandResult> = lines.iter()
                            .map(|line| execute_command(engine, line))
                            .collect();
                        match aggregate_results_with_engine(engine, sub_results) {
                            TfCommandResult::Success(Some(msg)) => messages.push(msg),
                            TfCommandResult::Success(None) => {}
                            TfCommandResult::Error(e) if control_flow::parse_break_marker(&e).is_some() => return TfCommandResult::Error(e),
                            TfCommandResult::Error(e) => {
                                messages.push(format!("Error: {}", e));
                                has_error = true;
                            }
                            TfCommandResult::ClayCommand(cmd) => pending_clay_commands.push(cmd),
                            // See the matching arm above (outer loop) - the
                            // OUTER `messages` accumulated so far (from
                            // before this Quote::Exec line ran) would
                            // otherwise be silently dropped the same way.
                            r @ (TfCommandResult::Return(_) | TfCommandResult::Result(_)) => {
                                if !messages.is_empty() {
                                    engine.pending_outputs.push(super::TfOutput {
                                        text: messages.join("\n"),
                                        attrs: String::new(),
                                        world: None,
                                    });
                                }
                                return r;
                            }
                            r @ TfCommandResult::ExitLoad(_) => return r,
                            // Send/scheduling/recall surfaced from a nested exec
                            // line - bounce upward the same way an unresolvable
                            // top-level Quote does above.
                            other => return other,
                        }
                    }
                }
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



// =============================================================================
// Command Implementations
// =============================================================================

/// Split a `/set`/`/let` argument string into (name, value) the way real TF
/// does (finding 19), rather than blindly `.trim()`-ing whatever comes after
/// an `=` found anywhere in the string:
///
/// - Leading whitespace before the name is skipped (it's just the separator
///   between the command word and its argument).
/// - If '=' immediately follows the name (no space before it), this is the
///   `name=value` form: `value` is everything after the '=' to the end of
///   the string, kept **completely verbatim** - TF's own `/set` help says
///   "there should be no spaces on either side of the '='" for this form,
///   precisely because any spaces that ARE there become part of the value.
/// - Otherwise (whitespace, or nothing, follows the name), this is the
///   `name value` form: the entire run of whitespace after the name (one
///   character or many - tf-lib's own color.tf lines up its `/set` values
///   in columns with runs of tabs) is consumed as the separator, and
///   `value` is everything from the first non-whitespace character to the
///   end of the string, kept verbatim from there - even if it happens to
///   start with a literal '=' (real TF prints a warning in that case -
///   "'=' following space is part of value" - but still keeps the '=' as
///   part of the value; Clay doesn't reproduce the warning, only the
///   storage behavior, since no fixture checks for it).
/// - A bare name with nothing after it, or only trailing whitespace after
///   it, (no '=') yields an empty value.
///
/// Verified directly against real tf 5.0 beta 8: `/set foo= bar ` stores
/// `" bar "`, `/set foo2 = bar2 ` stores `"= bar2 "` (the space-before-'='
/// warning case), `/set foo3 bar3 ` stores `"bar3 "`, and - the case that
/// matters for tab-aligned library files like color.tf -
/// `/set foo4\t\t\tbar4   ` stores `"bar4   "` (all three leading tabs
/// consumed as ONE separator, the three trailing spaces kept).
fn split_set_or_let_value(args: &str) -> Option<(&str, &str)> {
    let start = args.find(|c: char| !c.is_whitespace())?;
    let rest = &args[start..];
    let name_end = rest
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(rest.len());
    let name = &rest[..name_end];
    let after_name = &rest[name_end..];

    if let Some(value) = after_name.strip_prefix('=') {
        Some((name, value))
    } else {
        // after_name is empty, or starts with whitespace (the only other
        // way name_end could have stopped short of rest.len()) - skip the
        // whole run of whitespace, then take everything after it verbatim.
        let value_start = after_name.find(|c: char| !c.is_whitespace()).unwrap_or(after_name.len());
        Some((name, &after_name[value_start..]))
    }
}

/// /set varname=value - Set a global variable
/// Supports both /set var=value and /set var = value
fn cmd_set(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.trim().is_empty() {
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

    let (name, value) = split_set_or_let_value(args).unwrap_or((args, ""));

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
    if args.trim().is_empty() {
        return TfCommandResult::Error("Usage: /let varname=value".to_string());
    }

    let (name, value) = split_set_or_let_value(args).unwrap_or((args, ""));

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

/// /echo [-a<attrs>] [-p] [-o|-e|-A|-r] [-w[<world>]] [--] message - Display message.
///
/// Options (`/help echo`'s command form - `-poerA -a<string> -w<string>` per real tf's own
/// getopts error text):
///   -a<attrs>   Echo with the given (comma-separated) display attributes, same convention
///               as `/def -a`/`decode_attr()` - wraps the whole message in ANSI codes.
///   -p          Interpret "@{attr}" sequences inline (`process_attr_codes` below already
///               runs unconditionally regardless of -p, so this is accepted for TF
///               compatibility rather than gating anything new - seen since before this
///               function had any option parsing at all).
///   -o          Echo to the normal (tfout) stream - the default; accepted, no distinct
///               effect (Clay has no separate stream concept for /echo's destination).
///   -e          Echo to the tferr stream. Same acceptance as -o: still ordinary displayed
///               text (real tf's own automatic "E" attribute default for this destination
///               is not applied - Clay has no error-stream rendering path that both looks
///               different AND still counts as ordinary, non-failing output).
///   -A          Echo to the alert stream. Same acceptance as -o/-e.
///   -r          Raw: text is NOT run through `process_attr_codes` at all, so "@{...}"
///               sequences appear completely literally (overrides -p and the always-on
///               default, since Clay's /echo interprets "@{...}" unconditionally otherwise).
///   -w[<world>] Echo into `<world>`'s output instead of wherever this /echo's own result
///               would otherwise land (queued via `engine.pending_outputs`, the same sink
///               the `echo()` expression function's own "dest" argument uses - see
///               `commands::process_pending_tf_outputs`). Bare -w (blank world) means the
///               *current* world, which needs no redirection - handled as a plain,
///               synchronous `Success(Some(_))` exactly like no -w at all.
///   --          End of options: TF's own convention for letting <message> begin with '-'
///               without it being mistaken for a flag (`/help echo`: "'-' by itself can be
///               used to mark the end of command options"; matched here as a bare, empty
///               token after the leading '-', which also covers "--" itself).
///
/// Multiple single-letter flags may be bundled in one token (e.g. "-pr"), matching the
/// bundled-option convention already used by `/def`/`/recall` elsewhere in this file - `-a`
/// and `-w` each consume the remainder of their own token as their (possibly empty) value.
fn cmd_echo(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let mut remaining = args.trim_start();
    let mut world: Option<String> = None;
    let mut attrs = String::new();
    let mut raw = false;

    while remaining.starts_with('-') {
        let token_end = remaining.find(char::is_whitespace).unwrap_or(remaining.len());
        let token = &remaining[1..token_end];
        let after_token = remaining[token_end..].trim_start();

        if token.is_empty() {
            // A bare "-" (or "--", whose first char after the leading '-' is itself '-'
            // and falls through the same way below) - end of options.
            remaining = after_token;
            break;
        }

        let chars: Vec<char> = token.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                'a' => {
                    attrs = chars[i + 1..].iter().collect();
                    i = chars.len();
                }
                'w' => {
                    world = Some(chars[i + 1..].iter().collect());
                    i = chars.len();
                }
                'p' | 'o' | 'e' | 'A' => i += 1,
                'r' => {
                    raw = true;
                    i += 1;
                }
                // Unknown flag character - skip the rest of this token (matches the old
                // "unknown option, skip it" fallback) rather than erroring, since a future
                // TF option landing here should degrade gracefully.
                _ => i = chars.len(),
            }
        }
        remaining = after_token;
    }

    // @{attr} sequences: @{B} = bold, @{U} = underline, @{n} = normal/reset,
    // @{Crgb}/@{BCrgb} = foreground/background color - skipped entirely by -r.
    let message = if raw {
        remaining.to_string()
    } else {
        process_attr_codes(remaining)
    };
    let message = if attrs.is_empty() {
        message
    } else {
        let prefix = attrs_to_ansi_prefix(&attrs);
        if prefix.is_empty() {
            message
        } else {
            format!("{}{}\x1b[0m", prefix, message)
        }
    };

    if let Some(world_name) = world {
        if world_name.is_empty() {
            // Bare -w: the current world, same as omitting -w entirely.
            return TfCommandResult::Success(Some(message));
        }
        engine.pending_outputs.push(super::TfOutput {
            text: message,
            attrs: String::new(),
            world: Some(world_name),
        });
        return TfCommandResult::Success(None);
    }

    TfCommandResult::Success(Some(message))
}

/// /escape metacharacters string - Escape metacharacters and backslashes in string
/// Echoes string with any metacharacters or '\' preceded by '\'.
fn cmd_escape(engine: &mut TfEngine, args: &str) -> TfCommandResult {
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
        engine.set_global("?", TfValue::String(String::new()));
        return TfCommandResult::Success(Some(String::new()));
    };

    let result = tf_escape(metacharacters, string);
    // Command form both echoes AND returns the result (Job 15, verified directly
    // against real tf) - same dual nature as /replace and /pwd.
    engine.set_global("?", TfValue::String(result.clone()));
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

/// /substitute [-a<attrs>] [-p] [--] text - Replace trigger line with substituted text
/// Options (`/help substitute`):
///   -a<attrs> - Attributes given to <text> (comma-optional letters, /help attributes)
///   -p        - Interpret "@{attr}" strings inline, as in /echo (see cmd_echo's doc
///               comment - process_attr_codes below already runs unconditionally
///               regardless of -p, so this is accepted for TF compatibility rather than
///               gating anything new)
///   --        - End of options marker
fn cmd_substitute(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    // Parse options: -a<attrs>, -p, --
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
        } else if remaining == "-p" || remaining.starts_with("-p ") || remaining.starts_with("-p\t") {
            remaining = remaining[2..].trim_start();
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
pub(crate) fn process_attr_codes(text: &str) -> String {
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

/// Build an ANSI prefix for a comma-separated TF attribute list - the convention shared by
/// `/echo -a<attrs>`, `/substitute -a<attrs>`, and `decode_attr()`'s second argument (`/help
/// attributes`: "Use commas to separate attributes within an attribute list"). Empty parts
/// are skipped so a trailing/leading/doubled comma is harmless. Returns an empty string for
/// an empty or entirely-blank list - callers should skip the trailing reset in that case.
pub(crate) fn attrs_to_ansi_prefix(attrs: &str) -> String {
    let mut prefix = String::new();
    for part in attrs.split(',') {
        let part = part.trim();
        if !part.is_empty() {
            prefix.push_str(&attr_to_ansi(part));
        }
    }
    prefix
}

/// Convert TF attribute code to ANSI escape sequence
pub(crate) fn attr_to_ansi(attr: &str) -> String {
    // Real TF's "C<name>" (foreground) / "Cbg<name>" (background) color
    // attribute (see `/help attributes`, `/help colors`) - checked first so
    // it takes priority over Clay's own older, non-TF bare-name convention
    // below (kept for backward compatibility, though real TF always
    // requires the leading "C"). <name> is one of the 8 basic colors, one
    // of the 8 "bright"/aixterm colors, "rgb<R><G><B>" (6x6x6 color cube,
    // each digit 0-5), or "gray<N>" (0-23 grayscale ramp) - matching what
    // color.tf itself defines as %{start_color_<name>}/%{start_color_bg<name>}.
    if let Some(ansi) = tf_color_attr_to_ansi(attr) {
        return ansi;
    }

    match attr.to_uppercase().as_str() {
        // Basic attributes
        "N" | "NORMAL" => "\x1b[0m".to_string(),
        "B" | "BOLD" => "\x1b[1m".to_string(),
        "D" | "DIM" => "\x1b[2m".to_string(),
        "U" | "UNDERLINE" => "\x1b[4m".to_string(),
        "BLINK" | "FLASH" => "\x1b[5m".to_string(),
        "R" | "REVERSE" => "\x1b[7m".to_string(),

        // Standard colors (foreground) - Clay's own bare-name convention,
        // predating the TF-accurate "C<name>" form above; kept so existing
        // callers of Clay's /echo -p aren't broken.
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

        // 216-color cube: Crgb where r,g,b are 0-5 (Clay's older 4-char
        // shorthand, distinct from TF's own "Crgb###" form handled above)
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

/// Real TF's "C<name>" / "Cbg<name>" color attribute (see `attr_to_ansi`'s
/// doc comment). Returns `None` (not an empty string) for anything that
/// isn't recognized as this form at all, so `attr_to_ansi` can fall back to
/// its own older conventions instead of treating e.g. a bare "Cred" typo as
/// "recognized but produces nothing".
fn tf_color_attr_to_ansi(attr: &str) -> Option<String> {
    let rest = attr.strip_prefix('C').or_else(|| attr.strip_prefix('c'))?;
    if rest.is_empty() {
        return None;
    }
    let rest_lower = rest.to_lowercase();
    let (is_bg, lower) = match rest_lower.strip_prefix("bg") {
        Some(after_bg) => (true, after_bg.to_string()),
        None => (false, rest_lower),
    };
    if lower.is_empty() {
        return None;
    }

    // 6x6x6 color cube: "rgb<R><G><B>", each digit 0-5.
    if let Some(digits) = lower.strip_prefix("rgb") {
        let d: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
        if d.len() == 3 && d.len() == digits.chars().count() && d.iter().all(|&n| n <= 5) {
            let code = 16 + 36 * d[0] + 6 * d[1] + d[2];
            return Some(format!("\x1b[{};5;{}m", if is_bg { 48 } else { 38 }, code));
        }
    }

    // Grayscale ramp: "gray<N>", 0-23 - but NOT bare "gray" with no digits,
    // which is instead the bright-color name handled below.
    if let Some(digits) = lower.strip_prefix("gray") {
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = digits.parse::<u32>() {
                if n <= 23 {
                    let code = 232 + n;
                    return Some(format!("\x1b[{};5;{}m", if is_bg { 48 } else { 38 }, code));
                }
            }
        }
    }

    // 8 basic colors.
    let basic = match lower.as_str() {
        "black" => Some(0), "red" => Some(1), "green" => Some(2), "yellow" => Some(3),
        "blue" => Some(4), "magenta" => Some(5), "cyan" => Some(6), "white" => Some(7),
        _ => None,
    };
    if let Some(n) = basic {
        let base = if is_bg { 40 } else { 30 };
        return Some(format!("\x1b[{}m", base + n));
    }

    // 8 "bright"/aixterm colors (color.tf's own gray/brightred/.../brightwhite).
    let bright = match lower.as_str() {
        "gray" => Some(0), "brightred" => Some(1), "brightgreen" => Some(2), "brightyellow" => Some(3),
        "brightblue" => Some(4), "brightmagenta" => Some(5), "brightcyan" => Some(6), "brightwhite" => Some(7),
        _ => None,
    };
    if let Some(n) = bright {
        let base = if is_bg { 100 } else { 90 };
        return Some(format!("\x1b[{}m", base + n));
    }

    None
}

/// Strip TF's `@{...}` inline-attribute markup (in case it was never
/// decoded) and the ANSI SGR escape sequences `decode_attr()`/`/echo -p`
/// convert it into (in case it was) - the two forms an "attributed string"
/// can take in Clay's pragmatic text+embedded-codes representation (see
/// `expressions::eval_function`'s `decode_attr` doc comment). Used by the
/// `strip_attr()` function and by `strlen()`, which - like real TF, whose
/// attributes live in a channel entirely separate from the text - must
/// never count an attribute byte as a character.
pub(crate) fn strip_all_attributes(text: &str) -> String {
    let no_markup = strip_raw_attr_markup(text);
    strip_ansi_sgr(&no_markup)
}

/// Remove raw, undecoded "@{...}" sequences (does not interpret them).
fn strip_raw_attr_markup(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i < len {
        if chars[i] == '@' && i + 1 < len && chars[i + 1] == '{' {
            if let Some(end) = chars[i + 2..].iter().position(|&c| c == '}') {
                i = i + 3 + end;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Remove ANSI SGR escape sequences ("\x1b[...m") - the only kind
/// `attr_to_ansi` ever generates.
fn strip_ansi_sgr(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i < len {
        if chars[i] == '\x1b' && i + 1 < len && chars[i + 1] == '[' {
            if let Some(end) = chars[i + 2..].iter().position(|&c| c == 'm') {
                i = i + 3 + end;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Inverse of `attr_to_ansi`'s TF-color-name handling (`tf_color_attr_to_ansi`)
/// plus the basic bold/underline/reverse/reset codes: turns one SGR escape's
/// numeric parameter list back into a `@{...}` attribute name. Returns
/// `None` for a code with no reasonable `@{...}` equivalent - `encode_attr`
/// leaves those bytes as-is rather than inventing a name for them.
fn ansi_params_to_attr_code(params: &str) -> Option<String> {
    match params {
        "0" => return Some("n".to_string()),
        "1" => return Some("B".to_string()),
        "4" => return Some("U".to_string()),
        "7" => return Some("R".to_string()),
        _ => {}
    }
    if let Some(rest) = params.strip_prefix("38;5;") {
        return rest.parse::<u32>().ok()
            .and_then(cube_or_gray_code_to_name)
            .map(|n| format!("C{}", n));
    }
    if let Some(rest) = params.strip_prefix("48;5;") {
        return rest.parse::<u32>().ok()
            .and_then(cube_or_gray_code_to_name)
            .map(|n| format!("Cbg{}", n));
    }
    let n: u32 = params.parse().ok()?;
    const BASIC: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    const BRIGHT: [&str; 8] = ["gray", "brightred", "brightgreen", "brightyellow", "brightblue", "brightmagenta", "brightcyan", "brightwhite"];
    if (30..=37).contains(&n) {
        return Some(format!("C{}", BASIC[(n - 30) as usize]));
    }
    if (40..=47).contains(&n) {
        return Some(format!("Cbg{}", BASIC[(n - 40) as usize]));
    }
    if (90..=97).contains(&n) {
        return Some(format!("C{}", BRIGHT[(n - 90) as usize]));
    }
    if (100..=107).contains(&n) {
        return Some(format!("Cbg{}", BRIGHT[(n - 100) as usize]));
    }
    None
}

/// Inverse of the 6x6x6-cube/grayscale-ramp encoding in `tf_color_attr_to_ansi`.
fn cube_or_gray_code_to_name(code: u32) -> Option<String> {
    if (16..=231).contains(&code) {
        let c = code - 16;
        let (r, g, b) = (c / 36, (c % 36) / 6, c % 6);
        Some(format!("rgb{}{}{}", r, g, b))
    } else if (232..=255).contains(&code) {
        Some(format!("gray{}", code - 232))
    } else {
        None
    }
}

/// encode_attr() - inverse of `decode_attr()`/`process_attr_codes`: turns
/// embedded ANSI SGR escapes back into "@{...}" markup text. Round-trips
/// exactly for anything `attr_to_ansi` can produce (verified directly:
/// `encode_attr(decode_attr("@{Cbgrgb500}"))` == "@{Cbgrgb500}"), which is
/// the standard tf-lib idiom this needs to support (cylon.tf's own color
/// variables). An escape with no `@{...}` equivalent is left as-is.
pub(crate) fn encode_attr(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i < len {
        if chars[i] == '\x1b' && i + 1 < len && chars[i + 1] == '[' {
            if let Some(end) = chars[i + 2..].iter().position(|&c| c == 'm') {
                let params: String = chars[i + 2..i + 2 + end].iter().collect();
                match ansi_params_to_attr_code(&params) {
                    Some(name) => {
                        result.push_str("@{");
                        result.push_str(&name);
                        result.push('}');
                    }
                    None => {
                        // No @{...} equivalent - keep the raw escape.
                        result.push_str("\x1b[");
                        result.push_str(&params);
                        result.push('m');
                    }
                }
                i = i + 3 + end;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// /send [-W] [-T<type>] [-w[<world>]] [-n] [-h] text - send text to a world (or several).
///
/// TF options (`/help send`): `-W` sends to every connected world; `-T<type>` sends to
/// every connected world whose type glob-matches `<type>`; `-w<world>` sends to a named
/// world (attached, no space - `-w` bare means the current world); `-n` sends without an
/// end-of-line marker; `-h` fires the SEND hook first (by default `/send` does NOT run
/// hooks, per TF's own documented default - matches ordinary typed text, which always
/// does). Only `-w<world>` used to be parsed here (and with a Clay-specific, incompatible
/// "-w world" *space-separated* syntax, unlike every other TF `-w<world>` convention).
///
/// Any leading flag is bounced wholesale to Clay's own `/send` (`Command::Send`, built by
/// `parse_send_command` and executed by `execute_send_command` in commands.rs) - the world
/// list, world-type fan-out and SEND-hook firing all need `&mut App`, which this
/// engine-only function does not have, and `parse_send_command` already speaks this exact
/// attached-flag token grammar, so it is not reimplemented a second time here. A plain,
/// flag-free send (the overwhelming common case) skips that round trip.
fn cmd_send(_engine: &TfEngine, args: &str) -> TfCommandResult {
    let trimmed = args.trim_start();

    if trimmed.is_empty() {
        return TfCommandResult::Error("Usage: /send [-W] [-T<type>] [-w[<world>]] [-n] [-h] text".to_string());
    }

    let first_token = trimmed.split_whitespace().next().unwrap_or("");
    let is_flag = first_token == "-W" || first_token == "-n" || first_token == "-h"
        || first_token.starts_with("-w") || first_token.starts_with("-T");
    if !is_flag {
        return TfCommandResult::SendToMud(trimmed.to_string());
    }

    TfCommandResult::ClayCommand(format!("/send {}", trimmed))
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
/// /shift [n] - Shift positional parameters left by `n` (default 1): `%(n+1)
/// ... %#` are renamed to `%1 ... %(#-n)` (`/help shift`). `n` is clamped to
/// `argc` (verified directly against real tf: `/shift 5` with only 3
/// positional params leaves zero, it does not error) - "useful only during
/// macro expansion" per that same help text, so a bare `/shift` outside one
/// (`argc == 0`) is a silent no-op, same as before this job.
fn cmd_shift(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let argc = engine.get_var("#")
        .and_then(|v| v.to_int())
        .unwrap_or(0) as usize;

    if argc == 0 {
        return TfCommandResult::Success(None);
    }

    let requested = args.trim().parse::<i64>().unwrap_or(1).max(0) as usize;
    let n = requested.min(argc);
    if n == 0 {
        return TfCommandResult::Success(None);
    }

    // Shift: (n+1)->1, (n+2)->2, etc.
    for i in 1..=(argc - n) {
        let next_val = engine.get_var(&(i + n).to_string()).cloned()
            .unwrap_or(super::TfValue::String(String::new()));
        engine.set_local(&i.to_string(), next_val);
    }

    // Clear the vacated trailing slots
    for i in (argc - n + 1)..=argc {
        engine.set_local(&i.to_string(), super::TfValue::String(String::new()));
    }

    // Decrement count
    engine.set_local("#", super::TfValue::Integer((argc - n) as i64));

    // Rebuild %* from remaining args
    let mut parts = Vec::new();
    for i in 1..=(argc - n) {
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

/// /break [n] - Unconditionally terminate the nearest enclosing `/while` or
/// `/for` loop; with `<n>`, break out of `n` enclosing loops (`/help
/// break`). "If used outside a /while loop, the macro evaluation is
/// terminated" - both halves are implemented via one `TfCommandResult::Error`
/// marker (`control_flow::break_marker`/`parse_break_marker`): each of
/// control_flow.rs's 4 loop-body executors intercepts it, absorbing one
/// level and re-emitting it decremented if `n` isn't exhausted yet, while
/// `macros::execute_macro_with_context` intercepts an UNabsorbed one (no
/// enclosing loop at all) and stops the macro body outright. `n` floors at 1
/// (`/break 0`/`/break -5` both behave like a bare `/break` - verified
/// directly against real tf, same floor `/exit`'s own count uses).
fn cmd_break(args: &str) -> TfCommandResult {
    let n = args.trim().parse::<i64>().unwrap_or(1).max(1) as u32;
    TfCommandResult::Error(control_flow::break_marker(n))
}

/// /listworlds [-cus] [-m<style>] [-S<field>] [-T<type>] [name] - List world
/// definitions (TF style).
///
/// TF options added in plan Job 14b (`/help listworlds`): `-u` (include
/// unnamed temporary worlds) is accepted but not distinct - every Clay world,
/// including the ones `/world <host> <port>` creates on the fly, always has a
/// real `name` (there's no separate "unnamed" world class to include or
/// exclude). `-m<style>` and `-T<type>` are now parsed as attached-value
/// options (matching `-S<field>`'s own existing style) instead of being
/// iterated character-by-character - previously `-Tmud` would feed 'm', 'u',
/// 'd' back through the same per-char match as genuine short flags, so a type
/// value happening to contain one of those letters silently changed the
/// output. Neither is distinct: Clay has no per-world "type" field to filter
/// by, and always matches `<name>`/`<type>` by substring (no glob/regexp style
/// selector to switch). `-S<field>`'s "t" (type) is accepted and falls back to
/// name-sort like any other unimplemented field, for the same reason.
fn cmd_listworlds(engine: &TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();
    let mut short = false;
    let mut cmd_format = false;
    let mut include_unnamed = false; // -u: accepted, not distinct - see doc comment
    let mut sort_field = "name";
    let mut name_pattern: Option<String> = None;

    // Parse options
    let mut i = 0;
    let parts: Vec<&str> = args.split_whitespace().collect();
    while i < parts.len() {
        let part = parts[i];
        if let Some(flags) = part.strip_prefix('-') {
            if let Some(rest) = flags.strip_prefix('S') {
                sort_field = match rest.chars().next() {
                    Some('n') => "name",
                    Some('h') => "host",
                    Some('p') => "port",
                    Some('c') => "character",
                    Some('-') => "-",
                    _ => "name", // includes 't' (type): accepted, not distinct
                };
            } else if flags.starts_with('T') || flags.starts_with('m') {
                // -T<type> / -m<style>: accepted, not distinct (see doc comment).
            } else {
                for c in flags.chars() {
                    match c {
                        's' => short = true,
                        'c' => cmd_format = true,
                        'u' => include_unnamed = true,
                        _ => {}
                    }
                }
            }
        } else {
            name_pattern = Some(part.to_string());
        }
        i += 1;
    }
    let _ = include_unnamed; // accepted, not distinct (see doc comment)

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

/// /connections, /listsockets, /l - List connected worlds with stats (unseen count,
/// last recv/send, last/next keepalive, buffer size). This is the TF-native
/// implementation of the same output Command::WorldsList (commands.rs) produces for
/// non-TF-nested callers (e.g. a plain typed /l on a WS/GUI/web client, which never
/// reaches the TF engine at all - see handle_ws_send_command's own Command::WorldsList
/// arm). Returning real Success(Some(text)) here (instead of the ClayCommand bounce
/// those other paths use) is what makes `/quote `` `/l` `` capturable - cmd_quote's
/// backtick-source handling discards ClayCommand results as "not capturable output",
/// which is exactly what made this command silently produce "(no output)" before.
///
/// TF options added in plan Job 14b (`/help listsockets`): `-s` (short: one world
/// NAME per line, connected worlds only - what stdlib's `/send -W` depends on via
/// `$(/@listsockets -s)`), `<name>` (substring filter, same matching style
/// cmd_listworlds already uses), and `-S<field>` (sort by name/host/port/
/// character/lines/idle/-, "lines" meaning buffer_size and "idle" meaning time
/// since last receive). `-n` (print host/port in numeric form), `-m<style>`
/// (pattern matching style) and `-T<type>` are parsed - so their attached value
/// can't be misread as bundled boolean flags - but none of the three changes
/// Clay's output: Clay stores whatever host string the user typed (no separate
/// "numeric form" to toggle), always matches by substring (no glob/regexp style
/// selector), and has no per-world "type" field to filter or sort by (see
/// cmd_listworlds's identical rulings on the same three letters).
fn cmd_connections(engine: &TfEngine, args: &str) -> TfCommandResult {
    let mut short = false;
    let mut sort_field = "-";
    let mut name_pattern: Option<String> = None;

    for part in args.split_whitespace() {
        if let Some(flags) = part.strip_prefix('-') {
            if let Some(rest) = flags.strip_prefix('S') {
                sort_field = match rest.chars().next() {
                    Some('n') => "name",
                    Some('h') => "host",
                    Some('p') => "port",
                    Some('c') => "character",
                    Some('l') => "lines",
                    Some('i') => "idle",
                    _ => "-",
                };
            } else if flags.starts_with('T') || flags.starts_with('m') {
                // -T<type> / -m<style>: accepted, not distinct (see doc comment).
            } else {
                for c in flags.chars() {
                    match c {
                        's' => short = true,
                        'n' => {} // accepted, not distinct (see doc comment)
                        _ => {}
                    }
                }
            }
        } else {
            name_pattern = Some(part.to_string());
        }
    }

    let mut worlds: Vec<&super::WorldInfoCache> = engine.world_info_cache.iter()
        .filter(|w| w.is_connected)
        .collect();

    if let Some(ref pattern) = name_pattern {
        let pat = pattern.to_lowercase();
        worlds.retain(|w| w.name.to_lowercase().contains(&pat));
    }

    match sort_field {
        "name" => worlds.sort_by(|a, b| a.name.cmp(&b.name)),
        "host" => worlds.sort_by(|a, b| a.host.cmp(&b.host)),
        "port" => worlds.sort_by(|a, b| a.port.cmp(&b.port)),
        "character" => worlds.sort_by(|a, b| a.user.cmp(&b.user)),
        "lines" => worlds.sort_by(|a, b| a.buffer_size.cmp(&b.buffer_size)),
        "idle" => worlds.sort_by(|a, b| {
            a.last_receive_secs_ago.unwrap_or(i64::MAX).cmp(&b.last_receive_secs_ago.unwrap_or(i64::MAX))
        }),
        _ => {} // "-" (default): creation order, matching Clay's existing behavior
    }

    if short {
        let names: Vec<&str> = worlds.iter().map(|w| w.name.as_str()).collect();
        return if names.is_empty() {
            TfCommandResult::Success(None)
        } else {
            TfCommandResult::Success(Some(names.join("\n")))
        };
    }

    let worlds_info: Vec<crate::util::WorldListInfo> = worlds.iter().map(|w| {
        crate::util::WorldListInfo {
            name: w.name.clone(),
            connected: w.is_connected,
            is_current: engine.current_world.as_deref() == Some(w.name.as_str()),
            is_ssl: w.use_ssl,
            is_proxy: w.is_proxy,
            unseen_lines: w.unseen_lines,
            last_send_secs: w.last_user_command_secs_ago.map(|s| s.max(0) as u64),
            last_recv_secs: w.last_receive_secs_ago.map(|s| s.max(0) as u64),
            last_nop_secs: w.last_nop_secs_ago.map(|s| s.max(0) as u64),
            next_nop_secs: w.next_nop_secs,
            buffer_size: w.buffer_size,
        }
    }).collect();
    TfCommandResult::Success(Some(crate::util::format_worlds_list(&worlds_info)))
}

/// /ban - List currently banned hosts. TF-native counterpart to Command::BanList
/// (commands.rs), same reasoning as cmd_connections above: returning real text here
/// (instead of the ClayCommand bounce Command::BanList's own call site used) is what
/// makes `/quote `` `/ban` `` capturable.
///
/// Known, accepted gap: Command::BanList also broadcasts WsMessage::BanListResponse
/// so an open ban-management popup on another client stays fresh even when a
/// *different* interface is the one that ran /ban. This TF-native path has no App
/// access and can't send that broadcast — a console-typed plain `/ban` (which now
/// resolves here instead of round-tripping through Command::BanList) will show the
/// list correctly but won't proactively refresh someone else's already-open popup.
/// Not worth threading a side-channel through the TF engine for; the popup still
/// refreshes correctly the next time anyone actually changes the ban list.
fn cmd_banlist(engine: &TfEngine, _args: &str) -> TfCommandResult {
    let bans = &engine.ban_info_cache;
    if bans.is_empty() {
        return TfCommandResult::Success(Some("No hosts are currently banned.".to_string()));
    }
    let mut lines = vec![
        String::new(),
        "Banned Hosts:".to_string(),
        "─".repeat(70),
        format!("{:<20} {:<12} {}", "Host", "Type", "Last URL/Reason"),
        "─".repeat(70),
    ];
    for (ip, ban_type, reason) in bans {
        let reason_display = if reason.is_empty() { "(unknown)" } else { reason };
        lines.push(format!("{:<20} {:<12} {}", ip, ban_type, reason_display));
    }
    lines.push("─".repeat(70));
    lines.push("Use /unban <host> to remove a ban.".to_string());
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
  Esc-Left / Esc-Right - Switch connected worlds
  Ctrl-P / Ctrl-N      - Command history
  Tab                  - Release world output when paused.

Basic Configuration:
  /setup               - General settings popup
  /web                 - Web interface / remote access settings
  /actions             - Manage triggers and actions

For more help:
  /help commands       - List of commands
  /help functions      - List of functions
  /help <command>      - Help on a specific command (e.g. /help def)
  /help substitution   - %var, %{var-default}, %*, %-N, %L, $[..], $(..)
  /help keys           - All keyboard bindings
  /help web            - Websocket help (remote interfaces)

Keybind editor: open http://localhost:<port>/keybind-editor in a browser
(the port from /web; HTTP must be enabled there) - not a Clay command."#;
        TfCommandResult::Success(Some(help_text.to_string()))
    } else {
        match topic.as_str() {
            "set" => TfCommandResult::Success(Some(
                "/set [name [value]]\n\nSet a global variable. Without arguments, lists all variables.\nExamples:\n  /set foo bar    - Set foo to \"bar\"\n  /set count 42   - Set count to 42\n  /set            - List all variables".to_string()
            )),
            "echo" => TfCommandResult::Success(Some(
                "/echo [-a<attrs>] [-p] [-o|-e|-A|-r] [-w[<world>]] [--] message\n\nDisplay a message locally (not sent to MUD). Variable substitution is\nperformed on the message before /echo ever sees it.\n\nOptions:\n  -a<attrs>   Echo with the given display attributes (comma-separated,\n              /help attributes) - wraps the whole message in ANSI codes\n  -p          Interpret \"@{attr}\" sequences inline (already always on)\n  -o          Normal output stream (the default)\n  -e          Error stream (accepted; same rendering as -o)\n  -A          Alert stream (accepted; same rendering as -o)\n  -r          Raw: do not interpret \"@{...}\" sequences at all\n  -w[<world>] Echo into <world>'s output instead (bare -w = current world)\n  --          End of options, in case message begins with '-'\n\nExamples:\n  /echo Hello %{name}!\n  /echo -aCred -p @{u}Warning@{n}: low on cash\n  /echo -wOtherMUD Message for another world's window".to_string()
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
                "/send [-W] [-T<type>] [-w[<world>]] [-n] [-h] text\n\nSend text to a world (or several), bypassing macro/alias expansion.\n\nOptions:\n  -W          Send to every connected world\n  -T<type>    Send to every connected world whose type matches <type>\n  -w<world>   Send to <world> (attached, no space; bare -w means the current world)\n  -n          Do not append an end-of-line marker\n  -h          Fire the SEND hook first (by default /send does not run hooks)\n\nExamples:\n  /send say Hello everyone!\n  /send -wOtherMUD look\n  /send -W quit\n  /send -Tmud who".to_string()
            )),
            "say" => TfCommandResult::Success(Some(
                "/say <text>\n\nSpeak text aloud via text-to-speech.\nConsole: uses espeak, espeak-ng, say (macOS), or PowerShell (Windows).\nWeb/Android: uses the browser's Web Speech API.\n\nExample: /say Hello world\n\nEnable automatic TTS for MUD output in Setup > TTS.".to_string()
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
  -T type       Restrict to worlds of a given type (glob/regexp per -m)
  -hEVENT       Hook event (any of TF's 31 - see /help hooks), matches every occurrence
  -h"EVENT pat" Hook event with an argument pattern (matched like -t)
  -b"key"       Key binding
  -i, -I        Invisible: hidden from /list, /save, /purge unless forced
  -q            Quiet: doesn't count toward BACKGROUND hook / /trigger's
                return value; a SEND hook doesn't suppress the input
  -f            Same as -a, for backward compatibility

Name is optional if -t, -b, -B, or -h is given: such a macro is addressed
only by its number (#N, shown by /list).

Examples:
  /def -t"You are hungry" eat = get food bag%; eat food
  /def -t"^(\w+) tells you" -mregexp reply = tell %1 Got it!
  /def -hCONNECT greet = look
  /def -h"SEND greet*" hi = /echo greetings %*"#.to_string()
            )),
            "if" => TfCommandResult::Success(Some(
                "/if (expression) command\n/if (expr) ... /elseif (expr) ... /else ... /endif\n/if /command%; /then list [/elseif /command%; /then list]... [/else list] /endif\n\nConditional execution. The parenthesized form evaluates an expression;\nthe command form runs a command (or a \"%;\"-separated list of them) and\nuses its own return status as the truth value - nonzero is true. A\nleading \"/!\" on a command negates its status. Either form sets %?.\nExamples:\n  /if (hp < 50) cast heal\n  /if (%1 == \"yes\") /echo Confirmed /else /echo Cancelled /endif\n  /if /test 1%; /then /echo yes%; /endif\n  /if /!ismacro foo%; /then /echo not defined yet%; /endif".to_string()
            )),
            "while" => TfCommandResult::Success(Some(
                "/while (expression) ... /done\n/while /command%; /do list /done\n\nRepeat commands while expression (or a command's own return status,\nnonzero = true) is true. The command form re-runs its command(s) fresh\nbefore every iteration, so a condition like \"/let _i=...%; /@test _i >= 0\"\nsees the updated value each time; a leading \"/!\" negates a command's\nstatus. Either form sets %?.\nExamples:\n  /while (count < 10) /echo %count%; /set count $[count+1] /done\n  /while /let _i=$[strchr(_tail, _old)]%; /@test _i >= 0%; /do ... /done".to_string()
            )),
            "for" => TfCommandResult::Success(Some(
                "/for <var> <min> <max> <command>\n/for <var> <start> <end> [step] ... /done\n\nThe first form is TinyFugue's own: <var> takes on every integer from\n<min> to <max> inclusive (counting up only - nothing runs if <max> is\nless than <min>), and <command> (the rest of the line, optionally a\n\"%;\"-separated list) runs once per iteration with <var> set as a local;\nit is substituted fresh on each pass, so \"%i\"/\"${i}\" see that\niteration's value. The second, multi-line form is a Clay extension: an\nexplicit numeric [step] (default 1, or -1 when <end> < <start>), body\nlines collected up to a following /done.\nExamples:\n  /for i 1 3 /echo n=%i\n  /for i 1 5 /echo Number %i /done".to_string()
            )),
            "break" => TfCommandResult::Success(Some(
                "/break [<n>]\n\nDuring macro evaluation, unconditionally terminates the nearest\nenclosing /while or /for loop. With <n>, breaks out of <n> enclosing\nloops instead. If used outside a loop, terminates the macro evaluation\ninstead.\n\nExample:\n  /def worlds = /while ({#}) /if (%1 == \"stop\") /break%; /endif%; /world %1%; /shift%; /done".to_string()
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
                r#"/bind [<sequence> [= <command>]]

Bind a key sequence to a command. /bind <sequence> = <command> is exactly
equivalent to /def -b"<sequence>" = <command> - it creates a real (nameless)
macro, so <command> is substituted fresh every time the key is pressed, not
once at bind time.

  /bind                 List every binding.
  /bind <sequence>      Show what <sequence> is bound to.
  /unbind <sequence>    Remove the macro bound to <sequence>.

Key names: F1-F20, ^A-^Z (Ctrl), Esc-x/Alt-x/Meta-x/@x (all four equivalent),
Up/Down/Left/Right, Home/End, PgUp/PgDn, Insert/Delete, Tab - and chords of
these written back to back with no separator (^X^R, Esc-Left). TF's own raw
forms are also accepted: ^[b, \033, \0x1B, \27 (the escape byte); ^[[A etc
(raw terminal escape sequences).

Dispatch order for a pressed key: a /bind/-b/-B match first, then a
key_<name> macro (see /help keys), then the built-in action table.

Examples:
  /bind F5 = cast heal
  /bind ^S = /save macros.tf

See: keys, /def, /dokey, /unbind, /input"#.to_string()
            )),
            "hook" | "hooks" => TfCommandResult::Success(Some(
                r#"Hooks fire macros when something happens inside Clay, the same way
triggers fire macros on text from a socket. A hook has an <event> and an
optional argument <pattern> (matched against the event's own argument
text - see /help patterns - the way a trigger pattern matches a line).

Register with:
  /def -hEVENT name = body                  (matches every occurrence)
  /def -h"EVENT pattern" name = body        (pattern matched like -t)
  /hook EVENT[ pattern] [= body]            (equivalent to the above)

Manage with:
  /hook                    List all registered hooks
  /unhook EVENT            Remove every hook on EVENT (any pattern)
  /unhook EVENT pattern    Remove only the hook whose pattern is exactly this
  /trigger -hEVENT text    Fire a hook manually, as if <text> were its
                           argument - useful for testing without a live
                           connection (see /help /trigger)

Events (argument text in parens):
  ACTIVITY(world) BAMF(world) BGTEXT(world) BGTRIG(world) CONFAIL(world,
  reason) CONFLICT(macro) CONNECT(world, cipher) DISCONNECT(world, reason)
  ICONFAIL(world, reason) KILL(pid) LOAD(file) LOADFAIL(file, reason)
  LOG(file) LOGIN(world) MAIL(file) MORE NOMACRO(name) PENDING(world[,
  address]) PREACTIVITY(world) PROCESS(pid) PROMPT(text) PROXY(world)
  REDEF(obj_type, name) RESIZE(columns, lines) SEND(text) SHADOW(var_name)
  SHELL(type, command) SIGHUP SIGTERM SIGUSR1 SIGUSR2 WORLD(world)

SEND is special: if a non-quiet (no -q) SEND hook matches the text about
to be sent, that text is NOT sent - the hook's own body runs instead. This
is how /alias and speedwalking work. A quiet (-q) SEND hook runs alongside
the text without suppressing it.

Examples:
  /def -hCONNECT auto_look = look
  /def -h"SEND greet*" hi = /echo greetings %*
  /trigger -hSEND greet bob"#.to_string()
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
                "/ps [-srq] [-w[<world>]] [<pid>]\n\nList background /repeat and /quote processes, or one specific <pid>.\nShows PID, interval, remaining count, and command.\n\nOptions:\n  -s           Short form: list PIDs only, no header\n  -r           List /repeats only\n  -q           List /quotes only\n  -w[<world>]  List only processes for <world> (bare -w: current world)".to_string()
            )),
            "kill" => TfCommandResult::Success(Some(
                "/kill <pid>...\n\nKill each background process named by <pid>. Each pid is processed\nindependently - a bad pid doesn't stop the rest. Silent on success.\nUse /ps to see running processes.".to_string()
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
                r#"/exit [<n>]

Abort loading the current file early.

When called during /load or /require, stops reading the
current file immediately, and aborts <n> (default 1)
enclosing /load's - each one further out is only aborted
if <n> is high enough to still reach it. Commands already
executed are not undone.

When called outside of file loading, /exit has no effect."#.to_string()
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
                r#"/addworld [-pxe] [-Ttype] [-s<srchost>] name [char pass] host port [file]
/addworld [-Ttype] [-s<srchost>] name
/addworld [-Ttype] DEFAULT [char pass [file]]

Define a new world, update an existing world, or set DEFAULT's fallback
character/password/file for any world missing its own.

Command form:
  /addworld MyMUD mud.example.com 4000
  /addworld -x SecureMUD secure.example.com 4443
  /addworld MyMUD player password mud.example.com 4000
  /addworld DEFAULT player password

Function form:
  addworld(name, type, host, port, char, pass, file, flags, srchost)

Options:
  -x    Use SSL/TLS for connections
  -e    Echo sent text (ignored)
  -Ttype World type (ignored, defaults to MUD)
  -p    No proxy (ignored)
  -s<srchost>  Local bind address for the connection (accepted, not
               persisted - Clay has no per-world bind-address setting)
  file  Per-world script loaded on connect. Kept in memory only for this
        session (world_info(name, "file")); not saved to settings.dat.
  DEFAULT  Sets the character/password/file used by any world that has
           none of its own (${world_character}, ${world_password}).
           Stored as engine variables, not a real world - it never shows
           up in /listworlds.

Function flags string:
  "x" = use SSL

Examples:
  /addworld Cave cave.tcp.com 2283
  /addworld -x Secure secure.tcp.com 4443
  /addworld DEFAULT player password
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
  replace(old, new, str [,count]) - Replace occurrences (TF argument order;
                             note this is a behavior change from Clay's
                             previous (str, old, new) order)
  tr(domain, range, str)   - Translate characters
  ascii(str)               - ASCII code of first character
  char(code)               - Character from ASCII code
  sprintf(fmt, args...)    - Formatted string (%s, %d, %c, %%)
  pad(s, w, ...)           - Pad strings (+ = right-justify, - = left)
  strip_attr(str)          - Remove display attributes/color codes
  encode_attr(str)         - Encode display attributes as @{attr} codes
  decode_attr(str [,attrs [,f]]) - Interpret @{attr} codes (as /echo -p);
                             optional attrs applied to the whole string
  encode_ansi(str)         - Encode display attributes as terminal codes
  decode_ansi(str)         - Interpret terminal attribute codes
  strcmpattr(s, t)         - Compare strings, including attributes

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
  log(x), ln(x)            - Natural logarithm
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
  is_open([world])         - Check if world's socket is open
  idle([world])            - Seconds since last receive
  sidle([world])           - Seconds since last send

Info Functions:
  time()                   - Current Unix timestamp
  ftime([fmt [,time]])     - Format timestamp (default: now); supports
                             %Y %y %m %d %H %I %M %S %p %a %A %b %B %F %T
                             %j %w %s %% %@ (raw epoch.microseconds)
                             %. (microseconds since the whole second)
  mktime(y [,mo [,d [,h [,mi [,s [,usec]]]]]]) - Epoch seconds from local
                             date/time fields
  cputime()                - Process CPU time in seconds (real), or -1
  columns()                - Screen width
  lines()                  - Screen height
  winlines()               - Output window height (lines() minus reserved rows)
  moresize()               - Lines queued at more prompt
  morepaused([world])      - 1 if world's output is paused by more/pause
  getpid()                 - Process ID
  gethostname()            - Local host name
  systype()                - System type ("unix")
  filename(path)           - Expand ~ in path
  features([name])         - List optional features, or test one by name
  status_fields([i])       - Fields of status row i (always "" - Clay has
                             no configurable status bar)

Macro Functions:
  ismacro(name)            - Check if macro exists
  getopts(opts [,init])    - Parse the CURRENT macro's own positional
                             parameters (%1.. / %*) as command options,
                             per <opts> ("x" flag, "x:" string arg, "x#"
                             integer arg); sets local opt_x variables and
                             shifts the consumed options out of %*

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
  /setup  /web  /actions  /menu  /connections
  /world  /connect  /disconnect  /dc  /addworld  /unworld
  /reload  /version  /quit  /remote  /ban  /unban
  /flush  /dump  /note  /tag  /notify  /import  /window
  /font  /update  /dict  /urban  /translate  /url  /testmusic
Variables:
  /set  /unset  /let  /setenv  /listvar  /toggle  /export
Expressions & Control Flow:
  /expr  /test  /eval  /not  /return  /result
  /if  /elseif  /else  /endif  /while  /for  /done  /break
  /true  /false  /:  /then  /do
Macros & Triggers:
  /def  /undef  /undefn  /undeft  /list  /purge  /edit
  /trig  /trigp  /trigc  /trigpc  /untrig
  /bind  /unbind  /hook  /unhook  /dokey  /ismacro  /isvar
Output:
  /echo  /send  /beep  /quote  /recall  /substitute
  /hilite  /nohilite  /partial  /gag  /ungag  /nogag
  /escape  /replace  /tr  /first  /rest  /last  /nth
  /limit  /unlimit  /relimit  /xtitle
World:
  /fg  /addworld  /dc  /world  /listworlds  /listsockets  /l
  /watchdog  /watchname  /bamf
Files & Scripts:
  /load  /require  /loaded  /save  /lcd  /cd  /pwd  /exit  /log  /sh  /sys
Process:
  /repeat  /ps  /kill
Settings:
  /histsize  /localecho  /sub  /suspend  /time  /runtime  /shift
  /input  /grab  /trigger  /more  /wrap  /restrict  /core
  /features  /ver  /man  /say
Legacy/stub (accepted for script compatibility; mapped to a Clay
equivalent or a no-op - see /help <command>):
  /telnet  /finger  /getfile  /putfile  /liststreams  /changes
  /tick  /recordline  /purgeworld  /saveworld  /cat  /paste  /endpaste
TinyFugue prefix escape (checked before the table above):
  /tfhelp  /tfgag"#.to_string()
            )),
            "keys" | "keybindings" => TfCommandResult::Success(Some(
                r#"Keyboard Shortcuts (short form - see docs/markdown/07-keyboard-shortcuts.md
for the complete, generated-from-code table, including every chord)

World Switching:
  Esc+Left/Right, Esc+{/} - Cycle connected worlds (TF SOCKETB/SOCKETF)
  Shift+Up/Down        - Cycle through all worlds
  Esc+W                - Switch to world with activity

Input Editing:
  Left/Right, Ctrl+B/F - Move cursor
  Esc+B/F, Ctrl+Left/Right - Word left/right
  Up/Down              - Move cursor up/down lines
  Alt+Up/Down          - Resize input area
  Ctrl+A / Home        - Jump to start of line
  Ctrl+E / End         - Jump to end of line
  Ctrl+K               - Kill to end of line
  Ctrl+U               - Kill to start of line (TF's real ^U; kill ring kept)
  Ctrl+W               - Delete word before cursor
  Ctrl+D               - Delete character under cursor
  Ctrl+T               - Transpose characters
  Ctrl+Y               - Yank (paste killed text)
  Esc+D                - Delete word forward
  Esc+C/L/U            - Capitalize / lowercase / uppercase word forward
  Esc+Space            - Collapse spaces to one
  Esc+=                - Goto matching bracket
  Esc+. or Esc+_       - Insert last arg from history
  Ctrl+P/N, Ctrl+Up/Down - Command history
  Ctrl+Home/End, Esc+</> - Jump to oldest/newest history entry
  Esc+P                - Search history backward
  Esc+N                - Search history forward
  Insert, Esc+V        - Toggle insert/overwrite mode
  Esc+0-9, Esc+-       - Build a numeric repeat-count prefix (%kbnum)
  Ctrl+Q               - Spell suggestions
  Tab, Esc+Tab         - Release pending output / command completion

Output:
  PageUp/PageDown      - Scroll output
  Esc+J (lowercase)    - Jump to end, release all
  Esc+J (uppercase)    - Selective flush (keep hilite)
  Esc+H                - Half-page scroll/release
  Ctrl+S               - Pause output (TF PAUSE)

Display:
  F1                   - Show help
  F2                   - Toggle MUD tag display
  F4                   - Filter output
  F5                   - Search command history
  F8                   - Highlight action matches
  F9                   - Toggle GMCP media audio

System:
  Ctrl+C (twice)       - Quit
  Ctrl+L               - Redraw screen, keeping only server output (TF's own
                          plain repaint is the unbound refresh_line action)
  Ctrl+R, Ctrl+X Ctrl+R - Hot reload
  Ctrl+X Ctrl+V        - Show version
  Ctrl+Z               - Suspend

Mapping named keys to functions:
  Every key press is dispatched in this order:
    1. A /bind/-b/-B binding for the exact key (or chord) pressed.
    2. A key_<name> macro (TF's own two-level naming: a physical key like F5,
       Ctrl-Left, or Esc-Left is named "f5"/"ctrl_left"/"esc_left" - redefine
       its behavior with /def key_<name> = ... instead of rebinding the raw
       sequence). key_meta_<x> falls back to key_esc_<x> when undefined.
    3. Clay's built-in action for that key (customize in the keybind editor
       or keybindings.dat).
  /def -B<name> binds the physical key <name> directly (deprecated upstream,
  still accepted) - prefer key_<name> for most customization.
  /dokey_<name> (e.g. /dokey_home, /dokey_left) is the primitive a key_<name>
  macro typically calls; unlike bare /dokey <NAME> (always a single step),
  the movement-related dokey_<name> commands honor the numeric prefix above.

Customize with /bind, /def -b/-B, or /def key_<name> - or the browser-based
keybind editor at /keybind-editor, when the HTTP server is enabled (/web):
  /bind F5 = cast heal
  /def key_up = /dokey_recallb
  /bind ^S = /save macros.tf"#.to_string()
            )),
            "toggle" => TfCommandResult::Success(Some(
                "/toggle varname\n\nToggle a variable between 0 and 1.\nIf current value is 0, sets to 1; otherwise sets to 0.\n\nExample: /toggle gag".to_string()
            )),
            "return" => TfCommandResult::Success(Some(
                "/return [expression]\n\nStop executing the current macro and return.\nIf an expression is given, it is evaluated and %? is set to the result.\nWithout an argument, %? is set to 1.\n/return never echoes its value - see /help result for the form that can.\n\nExample:\n  /def check = /if (hp > 50) /return 1%; /echo Low HP!".to_string()
            )),
            "result" => TfCommandResult::Success(Some(
                "/result [expression]\n\nLike /return: stop executing the current macro, and set its return\nvalue (%? and the value seen by $[name(args)] / $(/name args)) to the\nstring value of expression - the empty string if expression is omitted.\n\nThe difference from /return: when the macro was called as a command\n(\"/name ...\", a trigger, or a hook) rather than as a function\n(\"name(args)\"), /result ALSO echoes the value, so the same macro works\nusefully as either. When called as a function, /result behaves exactly\nlike /return (no echo).\n\nExample:\n  /def dbl = /result {1} * 2\n  $[dbl(21)]   => 42 (no echo, just the value)\n  $(/dbl 5)    => \"10\" (the echoed value, captured)\n  /dbl 5       => echoes \"10\"\n\nSee also: /help substitution".to_string()
            )),
            "substitution" | "subst" => TfCommandResult::Success(Some(
                r#"Substitution: %selector, %{selector}, %{selector-default}

Before a macro body (or /eval's argument) runs, %-sequences are replaced:
  %{name}    Value of variable/macro "name" (braces optional if the text
             after it can't be mistaken for part of the name)
  %1, %2 ... The corresponding positional parameter (macro args, trigger
             match words, or function-call arguments - any number > %{#}
             is just empty)
  %*         All positional parameters, space-separated
  %#         Number of positional parameters
  %?         Return value of the most recently executed command
  %L, %L2 .. The last positional parameter, second-to-last, etc. ("%L" is
             "%L1")
  %-1, %-2 . All positional parameters except the first, first two, etc.
  %-L, %-L2  All positional parameters except the last, last two, etc.
  %P0-%P9    Regexp capture groups from the last match (%P0 = whole match)
  %PL, %PR   Text to the left/right of the last regexp match
  %{sel-def} "def" is substituted instead whenever "sel" would be empty -
             e.g. %{1-DEF} is arg 1, or "DEF" if there's no arg 1
  %%         A literal "%" (also true anywhere %% appears, e.g. inside
             %%; to keep a literal "%;" out of the command separator)

Other substitutions (see /help eval, /help expr):
  $[expression]   Value of an expression
  $(command)      A command's echoed output, captured as text
  ${name}         A macro's body, substituted literally

Example:
  /def greet = :waves to %{1-Jack}.
  /greet        => :waves to Jack.
  /greet Dave   => :waves to Dave.

See also: /help result, /help return, /help def, /help shift"#.to_string()
            )),
            "not" => TfCommandResult::Success(Some(
                "/not [-s<level>] <command>\n\nRun <command> exactly like /eval does (substitution, then execution),\nand set %? to the LOGICAL NEGATION of whatever %? the command left.\n\nExample:\n  /not /test 1   => %?=0\n  /not /test 0   => %?=1\n\nSee also: /help eval, /help test".to_string()
            )),
            "ismacro" => TfCommandResult::Success(Some(
                "/ismacro [<macro-options>]\n\nCommand form of the macro-existence test: takes the same option\ngrammar as /list and /purge (-i -I -m<style> -t -b -B -h -a -w -p -n ...)\nand sets %? to the sequence number of the LAST macro that matches every\ngiven option, or 0 if none do. No output.\n\nDistinct from the ismacro(name) FUNCTION (an exact-name-only check).\n\nExample: /ismacro -msimple -ib'^R'\n\nSee also: /help list, /help isvar".to_string()
            )),
            "isvar" => TfCommandResult::Success(Some(
                "/isvar <name>\n\nSets %? to 1 if <name> is set as a variable (local or global scope),\nelse 0. No output.\n\nExample: /isvar HOME".to_string()
            )),
            "features" => TfCommandResult::Success(Some(
                "/features [<name>]\n\nWith no argument, prints Clay's optional-feature list, each prefixed\n+ (enabled) or - (disabled). With <name>, sets %? to 1/0 and prints\nnothing.\n\nExample:\n  /features\n  /features ssl".to_string()
            )),
            "restrict" => TfCommandResult::Success(Some(
                "/restrict [SHELL|FILE|WORLD]\n\nWith no argument, reports the current restriction level. With an\nargument, RAISES the level - it can never be lowered again.\n\n  SHELL  Disables /sh, /sys, /quote !...\n  FILE   Implies SHELL. Disables /load, /require, /save, /lcd, /cd,\n         /log (opening a file), /quote '...\n  WORLD  Implies FILE. Disables /addworld and the arbitrary\n         \"<host> <port>\" form of /world.\n\nExample: /restrict shell".to_string()
            )),
            "core" => TfCommandResult::Success(Some(
                "/core\n\nNot supported in Clay - real tf uses this to dump core on a fatal\nsignal for debugging its own C implementation.".to_string()
            )),
            "sys" => TfCommandResult::Success(Some(
                "/sys <command>\n\nRun <command> via the shell, echo every stdout/stderr line, and set\n%? to the command's real process exit status (not a 0/1 boolean).\nHonours /restrict (SHELL and above refuse it).\n\nExample: /sys ls -la".to_string()
            )),
            "xtitle" => TfCommandResult::Success(Some(
                "/xtitle <text>\n\nPut <text> on the console's terminal-tab/titlebar. Console only - a\nweb/GUI/remote-console client has no terminal tab of its own to rename.".to_string()
            )),
            "more" => TfCommandResult::Success(Some(
                "/more [on|off|1|0]\n\nToggle Clay's more-mode (pause when the screen fills). A bare or\ninvalid argument is an error (matches real tf's own validated %more\nflag).\n\nExample: /more on".to_string()
            )),
            "wrap" => TfCommandResult::Success(Some(
                "/wrap [on|off|<n>]\n\nA numeric argument sets Clay's own output hang-indent width\n(Settings::wrapspace, same as the wrapspace setting) and turns on the\nTF-visible %wrap variable. on/off only update %wrap - Clay has no\noutput-wrapping on/off toggle to apply them to.\n\nExample: /wrap 4".to_string()
            )),
            "first" => TfCommandResult::Success(Some(
                "/first <args...>\n\nPrint and return (%?) the first whitespace-separated word.\n\nSee also: /help rest, /help last, /help nth".to_string()
            )),
            "rest" => TfCommandResult::Success(Some(
                "/rest <args...>\n\nPrint and return (%?) every word after the first.\n\nSee also: /help first, /help last, /help nth".to_string()
            )),
            "last" => TfCommandResult::Success(Some(
                "/last <args...>\n\nPrint and return (%?) the last whitespace-separated word.\n\nSee also: /help first, /help rest, /help nth".to_string()
            )),
            "nth" => TfCommandResult::Success(Some(
                "/nth <n> <args...>\n\nPrint and return (%?) the <n>th word (1-based). A non-numeric or\nnon-positive <n>, or one past the end, gives \"\".\n\nExample: /nth 2 a b c   => b".to_string()
            )),
            "ver" => TfCommandResult::Success(Some(
                "/ver\n\nPrint and return (%?) Clay's bare version number.\n\nSee also: /help version".to_string()
            )),
            "man" => TfCommandResult::Success(Some(
                "/man <topic>\n\nSame as /help <topic>.".to_string()
            )),
            "nogag" => TfCommandResult::Success(Some(
                "/nogag [<pattern>]\n\nWith no argument, turns off the %gag flag (disabling all gag\nattributes) and prints \"Gags disabled.\". With <pattern>, removes a\ngag-attributed macro matching it (same as /untrig -ag <pattern>).\n\nSee also: /help gag, /help untrig".to_string()
            )),
            "true" => TfCommandResult::Success(Some(
                "/true\n\nA no-op that always sets %?=1. No output.".to_string()
            )),
            "false" => TfCommandResult::Success(Some(
                "/false\n\nA no-op that always sets %?=0. No output.".to_string()
            )),
            ":" => TfCommandResult::Success(Some(
                "/:\n\nThe null command: a no-op that always sets %?=1. No output.".to_string()
            )),
            "limit" => TfCommandResult::Success(Some(
                "/limit [-v] [-a] [-m<style>] [<pattern>]\n\nOpen the F4 filter popup, showing only lines matching <pattern>\n(console only - see /help unlimit, /help relimit).\n\n  -v         show only lines that DON'T match <pattern>\n  -a         show only lines that have attributes\n  -m<style>  simple, glob or regexp instead of %matching's default\n\nWith no options or pattern, reports whether a limit is active.\n\nExample: /limit -v error".to_string()
            )),
            "unlimit" => TfCommandResult::Success(Some(
                "/unlimit\n\nClose the F4 filter popup opened by /limit. Console only.\n\nSee also: /help limit, /help relimit".to_string()
            )),
            "relimit" => TfCommandResult::Success(Some(
                "/relimit\n\nRe-apply the most recently applied /limit. Console only.\n\nSee also: /help limit, /help unlimit".to_string()
            )),
            "suspend" => TfCommandResult::Success(Some(
                "/suspend\n\nSuspend the process (equivalent to Ctrl+Z).".to_string()
            )),
            "dokey" => TfCommandResult::Success(Some(
                "/dokey keyname\n\nSimulate pressing a named edit key. Sets %? to TF's documented\nreturn value where that's cheap to compute (movement/deletion: new\ncursor position; otherwise 1).\n\nKey names:\n  BSPC/BACKSPACE     - Backspace\n  BWORD              - Delete previous word\n  DLINE/DELINE       - Delete entire line\n  REFRESH/REDRAW     - Redraw screen\n  LNEXT              - Treat the next key literally\n  UP                 - Cursor up (no history fallback - that's the key's job)\n  DOWN               - Cursor down (no history fallback)\n  LEFT/RIGHT         - Move cursor\n  HOME/END           - Start/end of line\n  NEWLINE/ENTER      - Submit input\n  RECALLB            - Previous history entry\n  RECALLF            - Next history entry\n  RECALLBEG          - First history entry\n  RECALLEND          - Last history entry\n  SEARCHB/SEARCHF    - Search history backward/forward\n  SOCKETB/SOCKETF    - Previous/next world\n  DWORD              - Delete next word\n  DCH/DELETE         - Delete character under cursor\n  CLEAR              - Clear the output view (scrollback refills it)\n  WLEFT/WRIGHT       - Word left/right\n  DEOL               - Delete to end of line\n  PAUSE              - Pause output\n  PAGE/PGDN          - Page forward\n  PAGEBACK/PGUP/PAGEUP - Page backward\n  HPAGE              - Half page forward\n  HPAGEBACK          - Half page backward\n  LINE               - Scroll forward one line\n  LINEBACK           - Scroll backward one line\n  FLUSH              - Jump to end, releasing all pending output\n  SELFLUSH           - Show highlighted pending lines, then jump to end".to_string()
            )),
            "histsize" => TfCommandResult::Success(Some(
                "/histsize [-lig] [-w[<world>]] [<size>]\n\nGet or set the history buffer size (Clay tracks one shared value for\nall of -l/-i/-g; -i is Clay's own default). -w<world> validates the\nworld name but reports/sets the same shared value - Clay has no\nseparate per-world history limit.\n\nOptions:\n  -l          Local history\n  -i          Input history (Clay's default)\n  -g          Global history (real tf's own default)\n  -w<world>   World history (bare -w means the current world)\n\nExample: /histsize 500".to_string()
            )),
            "localecho" => TfCommandResult::Success(Some(
                "/localecho [on|off]\n\nGet or set local echo mode.\nWhen on, typed commands are displayed locally.".to_string()
            )),
            "sub" => TfCommandResult::Success(Some(
                "/sub [off|on|full]\n\nGet or set the substitution mode.\n  off  - No variable substitution\n  on   - Normal substitution (default)\n  full - Full substitution".to_string()
            )),
            "replace" => TfCommandResult::Success(Some(
                "/replace old new string\n\nReplace all occurrences of 'old' with 'new' in string.\nEchoes the result.\n\nFunction form: $[replace(old, new, str [,count])]\n(TF argument order - note this is a behavior change from Clay's\nprevious (str, old, new) function argument order; the /replace\ncommand's own argument order is unchanged.)\n\nExample: /replace foo bar \"foo and foo\"  => \"bar and bar\"".to_string()
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
                "/unworld <name>...\n\nRemove the definition of each named world. Each name is processed\nindependently - a missing name doesn't stop the rest. Silent on\nsuccess. Clay never removes its last remaining world.\n\nExample: /unworld OldMud1 OldMud2".to_string()
            )),
            "watchdog" => TfCommandResult::Success(Some(
                "/watchdog [-w<world>] [off|on|n1 [n2]]\n\nSuppress duplicate lines from the MUD.\nIf a line has appeared n1 times in the last n2 lines, it is gagged.\n\nDefaults: n1=2 (threshold), n2=5 (window size)\n\n  -w<world>   Apply only to the named world. Per-world settings\n              take precedence over the global setting.\n\nExamples:\n  /watchdog on          - Enable globally with defaults\n  /watchdog 3 10        - Gag after 3 repeats in last 10 lines\n  /watchdog off         - Disable globally\n  /watchdog             - Show global settings\n  /watchdog -wMyMUD on  - Enable only on world MyMUD\n  /watchdog -wMyMUD 3 10 - Per-world threshold/window\n  /watchdog -wMyMUD off - Disable only on world MyMUD\n  /watchdog -wMyMUD     - Show settings for world MyMUD".to_string()
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
                "/listvar [-m<matching>] [-gxsv] [<name> [<value>]]\n\nList variables whose name (and, if given, value) match <name> and\n<value>, sorted by name. Default output (and -g/-x) is reloadable\n\"/set NAME=value\" / \"/setenv NAME=value\" text. Silent when nothing\nmatches.\n\nOptions:\n  -m<matching>  Matching style for <name>/<value> (default %{matching})\n  -g            List only global (unexported) variables\n  -x            List only exported variables\n  -s            Short format: names only\n  -v            Values only\n\nExamples:\n  /listvar        - List all variables\n  /listvar foo*   - List variables starting with foo\n  /listvar -s foo* - Just the matching names".to_string()
            )),
            "eval" => TfCommandResult::Success(Some(
                "/eval [-s<level>] <text>\n\nPerform one more substitution pass on <text> (%vars, $[...], $(...), and\nthe %; command separator), then execute the result: a /-command runs\nthrough the usual macro-or-builtin lookup, anything else is sent to the\nworld.\n\n-s0   Skip the substitution pass - dispatch <text> exactly as given.\n\nExamples:\n  /set v=7\n  /eval /echo v=%v      - prints \"v=7\"\n  /set cmdtail=echo hi\n  /eval /%cmdtail        - prints \"hi\"\n  /eval -s0 /echo v=%v   - prints \"v=%v\" literally (no substitution)".to_string()
            )),
            "beep" => TfCommandResult::Success(Some(
                "/beep [<number>|ON|OFF]\n\nSound the terminal bell <number> times (default 3, capped at 100).\nON re-enables /beep after OFF; OFF makes /beep (and gag-triggered\nbeeps) do nothing until ON is used again - sets the \"beep\" variable.\n\nExamples:\n  /beep       - Three beeps\n  /beep 5     - Five beeps\n  /beep OFF   - Disable beeping\n  /beep ON    - Re-enable beeping".to_string()
            )),
            "quote" => TfCommandResult::Success(Some(
                r#"/quote [options] [<pre>] '"<file>"[<suf>]
/quote [options] [<pre>] `"<TF_cmd>"[<suf>]
/quote [options] [<pre>] !"<shell_cmd>"[<suf>]
/quote [options] [<pre>] #"<recall_args>"[<suf>]

Generates lines of text, one per line from a file, TF command, shell command,
or /recall search, then sends/echoes/executes each one - the double quotes
(and <suf>) may be omitted, and unquoted `` `/!/# `` sources run to the end of
the line. With no source character at all, <text> itself is sent literally
(no %var/$() substitution). <pre> is prepended to every generated line.

Sources:
  '<file>          Read lines from a file
  `<TF_cmd>        Capture <TF_cmd>'s own output, one generated line per
                   line of output (real TF command output only - a plain
                   Clay-only command bounces through here with nothing to
                   capture)
  !<shell_cmd>     Run <shell_cmd> in the shell, capture its output
  #<recall_args>   Capture /recall <recall_args>'s output (shorthand for
                   `` `/recall <recall_args> ``)

Options:
  -d<disp>    Disposition of generated text: "send" (to the socket, the
              default when there is no <pre>), "echo" (to the screen), or
              "exec" (as a TF command, the default when there IS a <pre>)
  -w[<world>] Run generated commands with <world> as the current world (Clay
              extension: -w also selects the source/destination world itself,
              same convention /recall and /send already use)
  -S          Run synchronously, with no delay between lines (also accepted:
              a literal delay in seconds, or "H:M[:S]")
  -P          Run whenever a prompt is received
  -A          Keep ANSI/escape sequences in generated lines (Clay extension;
              stripped by default)

Examples:
  /quote hello world             - Sends "hello world" literally
  /quote '"/etc/motd"            - Sends each line of /etc/motd
  /quote say '"/tmp/lines.txt"   - Sends "say <line>" for each line of the file
  /quote think `"/version"       - Sends "think <TF version text>"
  /quote -decho `"/connections"  - Displays the connections table locally
  /quote -S /_fgrep `"/echo x"   - Executes "/_fgrep x" once, synchronously
  /quote :heard: #-l/2 *spam*    - Sends the last 2 recalled lines matching "*spam*"
  /quote !"ls -la"               - Sends the output of the shell "ls -la" command"#.to_string()
            )),
            "recall" => TfCommandResult::Success(Some(
                r#"/recall [options] [#]range [pattern]

Search output history.

Options:
  -w[world]   Search specific world (default: current)
  -l          Search local (TF) output + your typed input
  -g          Search all worlds + local + your typed input
  -i          Search your typed input only
  -D          Search long-term archive (~/.clay/scrollback.db)
  -t[format]  Show timestamps
  -v          Invert match (show non-matching)
  -q          Quiet (set %? but don't display)
  -mtype      Match type: simple, glob (default), regexp
  -a<attrs>   Suppress the given attributes (comma-optional letters, /help
              attributes - e.g. -ag shows gagged lines; every other letter is
              accepted but Clay's history has nothing else to suppress)
  -An         Show n lines of context after each match
  -Bn         Show n lines of context before each match
  -Cn         Equivalent to -An -Bn
  #           Show line numbers (must come right before range)

When -A, -B, or -C is used, non-adjacent groups of matched+context lines are
separated by a "--" line, matching real tf.

Your typed input is captured invisibly - it never appears in normal output.
Press F2 (show tags) to see it inline, or use /recall -i/-l/-g. Typed input is
never written to the archive (the "Log Input" setting writes it to the per-world
log FILE only), so -D reaches it through the in-memory half below.

-D searches the offline archive AND the in-memory buffer together, oldest
first, cut so the overlap between them isn't listed twice. Rows that came out
of the archive are marked with the same symbol Page Up scrollback uses; rows
still in memory keep the client-output symbol, as does Clay's own text
(e.g. "No matches").

Range: N (last N), -N (Nth previous), N-M, N-

Examples:
  /recall 20                       - Last 20 lines
  /recall -i /def                  - Input history matching /def
  /recall -i *tell*                - Commands you typed containing "tell"
  /recall -mregexp \d{3}-\d{4}     - Regex match
  /recall -D dragon                - Search archive for "dragon"
  /recall -D -wmud.example.com *   - All archived lines for a world"#.to_string()
            )),
            "gag" => TfCommandResult::Success(Some(
                "/gag [pattern]\n\nWith no args: list all gag triggers.\nWith a pattern: create a trigger that suppresses matching lines.\nEquivalent to: /def -ag -t\"pattern\"\n\nExample: /gag * has left the game.".to_string()
            )),
            "ungag" => TfCommandResult::Success(Some(
                "/ungag pattern\n\nRemove gag triggers matching the given pattern.\n\nExample: /ungag * has left the game.".to_string()
            )),
            "fg" => TfCommandResult::Success(Some(
                r#"/fg [-nsq<>l] [-c<N>] [<world>]

Switch to (foreground) a world. Without arguments, equivalent to
/connections.

Options:
  -<        Cycle to the previous connected world.
  ->        Cycle to the next connected world.
  -c<N>     Move N connected worlds forward (negative: backward) instead
            of one. If more than one of -c<N>/-</-> is given, the LAST
            one wins (matches real tf).
  -s        Suppress "world not found" instead of showing an error.
  -n        Accepted, not distinct: Clay's console always shows one
            world, so there is no "no world foreground" state to enter.
  -q, -l    Accepted, not distinct.

Examples:
  /fg MyMUD
  /fg ->
  /fg -c2 -<
  /fg -s SomeWorldThatMightNotExist"#.to_string()
            )),
            "dc" | "disconnect" => TfCommandResult::Success(Some(
                "/dc [<world>|-ALL]\n\nDisconnect from the current world, a named world, or (with -ALL,\ncase-insensitive) every connected world.\n\nExamples:\n  /dc\n  /dc MyMUD\n  /dc -ALL".to_string()
            )),
            "world" => TfCommandResult::Success(Some(
                r#"/world [-lqnxfb] [<name>]
/world [-lqnxfb] <host> <port>

Switch to or connect to a world by name, or connect directly to a
host/port (creates a temporary world named "host:port"). Without
arguments, opens the world selector.

Options (same as /fg and /connect):
  -l    Connect without auto-login.
  -b    Connect in the background (don't switch to it).
  -x    Use SSL - honored for the host/port form; for a named world,
        the world's own saved SSL setting is used instead.
  -q, -n, -f  Accepted, not distinct.
  -e    Edit the world's settings (Clay extra).

Examples:
  /world MyMUD
  /world -l MyMUD
  /world -b MyMUD
  /world mud.example.com 4000
  /world -e MyMUD"#.to_string()
            )),
            "listworlds" => TfCommandResult::Success(Some(
                r#"/listworlds [-cus] [-m<style>] [-S<field>] [-T<type>] [<name>]

List defined worlds (connected or not).

Options:
  -s          Short form: world names only.
  -c          Command form: printable /addworld-style definitions
              (includes passwords).
  -S<field>   Sort by name/host/port/character/- (default: name).
  -u, -m, -T  Accepted, not distinct - see /listsockets for the same
              rulings (no "unnamed" world class, no glob/regexp match
              style, no per-world type).
  <name>      Only worlds whose name contains this text.

Examples:
  /listworlds
  /listworlds -s
  /listworlds -c
  /listworlds -Sh Cave"#.to_string()
            )),
            "listsockets" | "connections" => TfCommandResult::Success(Some(
                r#"/listsockets [-sn] [-m<style>] [-S<field>] [-T<type>] [<name>]

List connected worlds (sockets). Maps to Clay's /connections table; /l is
the same command.

Options:
  -s          Short form: one world name per line (used by /send -W).
  -S<field>   Sort by name/host/port/character/lines/idle/- (default: -,
              i.e. unsorted).
  -n, -m, -T  Accepted, not distinct: Clay stores whatever host string
              you typed (no separate numeric form), always matches by
              substring (no glob/regexp style), and has no per-world
              type to filter or sort by.
  <name>      Only worlds whose name contains this text.

Examples:
  /listsockets
  /listsockets -s
  /listsockets -Sidle"#.to_string()
            )),
            "undef" => TfCommandResult::Success(Some(
                "/undef <name>...\n\nFor each <name> given, remove the macro with that exact name. Each name\nis processed independently - a missing name doesn't stop the rest.\nSilent on success; reports a diagnostic message per missing name.\n\nExample: /undef my_trigger other_macro".to_string()
            )),
            "undefn" => TfCommandResult::Success(Some(
                "/undefn number...\n\nRemove the macro(s) with the given sequence number(s) - see /list, or\nthe return value (%?) of the /def (or /edit) that created it. Silent on\nsuccess; a missing or invalid number reports its own diagnostic and the\nrest of the numbers are still processed.\n\nExample:\n  /def a = /echo a\n  /undefn %?".to_string()
            )),
            "undeft" => TfCommandResult::Success(Some(
                "/undeft pattern\n\nRemove all macros whose trigger matches the given pattern.\n\nExample: /undeft * tells you *".to_string()
            )),
            "list" => TfCommandResult::Success(Some(
r##"/list [-s] [-S] [-i|-I] [-m<style>] [-t[pat]] [-b[pat]] [-B[pat]] [-E[pat]]
      [-T[pat]] [-h[<event>[ <pattern>]]] [-a<attrs>] [-w[world]] [-p<pri>]
      [-n<shots>] [-F] [-P] [-q] [-] [<name>] [= <body>]

List macros having ALL the specified options. Omitted options are "don't
care". With no arguments, lists all non-invisible macros.

  -s          Short format: one "N: name" line per macro.
  -S          Sort macros by name.
  -m<style>   Matching style (simple/glob/regexp) for the pattern options
              below and for <name>/<body>. Default: %{matching}, or glob.
  -t[pat]     Has a trigger [matching pat]. "-t{}" (glob) or "-t^$" (regexp)
              selects macros with NO trigger.
  -b[pat]     Has a key binding [matching pat]; "-b{}"/"-b^$" = no binding.
  -B[pat]     Has a named-key binding [matching pat] (see -b).
  -E[pat]     Has a condition (-E) [matching pat].
  -T[pat]     Has a world-type restriction (-T) [matching pat].
  -h[event[ pattern]]
              "-h" alone = has any hook; "-h0" = no hook; "-hEVENT" = that
              hook event.
  -a<attrs>   Has one or more of the given display attributes.
  -w[world]   Restricted to a world [matching world].
  -p<pri>     Priority equals <pri>.
  -n<shots>   Shot count equals <shots> (0 = permanent).
  -F          Fall-through is set.
  -P          Partial-hilite (-P) is set.
  -q          Quiet (-q) is set.
  -           End option parsing (so <name> may itself start with "-").
  <name>      Pattern macro names must match; "#pattern" matches against
              macro numbers instead. "{}"/"^$" matches nameless macros.
  = <body>    Pattern macro bodies must match.

Examples:
  /list -mregexp -t -aurh ^foo    - triggered macros named foo* with any
                                     of underline/reverse/hilite
  /list -s -i ~alias_*            - short-format listing incl. invisible"##.to_string()
            )),
            "purge" => TfCommandResult::Success(Some(
r##"/purge [<macro-options>] [<name>] [= <body>]

Remove all macros matching the given filter - same <macro-options> as
/list (see "/help list"), except /purge never takes -s/-S. Invisible
macros are not purged unless -i/-I is given. With no arguments, removes
every non-invisible macro.

Examples:
  /purge drop                     - remove only the macro named "drop"
  /purge -mglob a*                - remove macros whose name matches a*
  /purge -I ~alias_call_*         - remove only matching invisible macros"##.to_string()
            )),
            "unbind" => TfCommandResult::Success(Some(
                "/unbind key\n\nRemove a key binding.\nKey names: F1-F12, ^A-^Z (Ctrl), @a-@z (Alt)\n\nExample: /unbind F5".to_string()
            )),
            "unhook" => TfCommandResult::Success(Some(
                "/unhook event [pattern]\n\nRemove hooked macros. Without pattern: removes every macro hooked to\nevent, regardless of its own pattern. With pattern: removes only the\nmacro whose own -h pattern is exactly this string (compared literally,\nnot matched against it) - see /help hooks.\n\nExamples:\n  /unhook CONNECT\n  /unhook SEND greet*".to_string()
            )),
            "save" => TfCommandResult::Success(Some(
                "/save [-a] <file> [<list-options>]\n\nSave macros matching <list-options> (same grammar as /list - see\n/help list) to <file>, one per line in reloadable /def form. Invisible\nmacros are excluded unless -i is given. -a appends; otherwise <file> is\noverwritten. The saved file can be loaded later with /load.\n\nExamples:\n  /save ~/.tf/macros.tf              - save every non-invisible macro\n  /save -a ~/.tf/macros.tf ~alias_*  - append only macros matching a pattern".to_string()
            )),
            "lcd" => TfCommandResult::Success(Some(
                "/lcd [<dir>]\n/cd [<dir>]\n/pwd\n\n/lcd and /cd change the local working directory (affects /sh, /load,\nand file operations). With no <dir>, /lcd reports the current\ndirectory; /cd instead defaults to $HOME. /pwd always reports the\ncurrent directory.\n\nExamples:\n  /lcd ~/tf-scripts\n  /lcd            - show the current directory\n  /cd             - change to $HOME".to_string()
            )),
            "log" => TfCommandResult::Success(Some(
                "/log [-w[<world>]] [-i] [-l] [-g] [OFF|ON|<file>]\n\nStart, stop, or list per-world output logging.\n\nOptions:\n  -w<world>   Act on <world> (attached, no space; bare -w = current world)\n  -i          Also toggle the global \"log input\" setting\n  -l          Accepted; Clay's log already includes local (client) output\n  -g          Accepted; Clay logs per-world, not one combined stream\n\nArguments:\n  ON          Resume the target's normal log file\n  <file>      Log the target to this exact file instead\n  OFF         Stop logging the target\n  (none)      With a -wilg option: same as ON. Otherwise: list every\n              world that is currently logging.\n\nExamples:\n  /log                - List every world currently logging\n  /log ON             - Start logging the current world\n  /log ~/mud.log      - Log the current world to this file\n  /log -wOtherMUD OFF - Stop logging a different world\n  /log -i             - Start logging the current world, plus typed input".to_string()
            )),
            "sh" => TfCommandResult::Success(Some(
                "/sh [-q] [<command>]\n\nExecute <command> and display its output. Environment variables set\nwith /setenv or /export are available. Without <command>, real tf\nspawns an interactive shell in place - Clay's TUI has no safe way to\ndo that, so bare /sh reports this instead of hanging.\n\n-q suppresses both the SHELL hook and the default\n\"Executing command: <command>\" message.\n\nExample: /sh ls -la".to_string()
            )),
            "time" => TfCommandResult::Success(Some(
                "/time [<format>]\n/time /command\n\nWithout a /command argument, print the current time formatted by\n<format> (ftime()-style conversions: %Y %m %d %H %M %S %a %A %b %B ...);\nwithout <format>, uses %{time_format} (default \"%H:%M\"). Sets %? to the\nformatted string.\n\nWith a \"/command\" argument (Clay's own kept extension - see /runtime for\nTF's own real=/cpu= timing report), run <command> and report how long it\ntook.\n\nExamples:\n  /time\n  /time %Y-%m-%d\n  /time /load big_script.tf".to_string()
            )),
            "runtime" => TfCommandResult::Success(Some(
                "/runtime <command>\n\nRun <command>, then print \"real=<secs> cpu=<secs>\" - the wall-clock and\nCPU time it took. <command>'s own output (if any) is shown first.\n\nExample:\n  /runtime /load big_script.tf".to_string()
            )),
            "version" => TfCommandResult::Success(Some(
                "/version\n\nDisplay Clay version information.".to_string()
            )),
            "quit" => TfCommandResult::Success(Some(
                "/quit\n\nExit Clay.".to_string()
            )),
            "shift" => TfCommandResult::Success(Some(
                "/shift [<n>]\n\nShift positional parameters left by <n> (default 1): %(n+1)...%# become\n%1...%(#-n). <n> is clamped to the argument count rather than erroring.\nUsed inside macros to iterate through arguments.\n\nExample:\n  /def showargs = /while ({1} !~ \"\") /echo %1%; /shift%; /done".to_string()
            )),
            "trigger" => TfCommandResult::Success(Some(
                "/trigger [-ln] [-g] [-w[<world>]] [-h[<event>]] [-d] <text>\n\nRun <text> through the trigger matcher exactly as if it had arrived from\na world (or, with -h, as if a hook event had fired with <text> as its\nargument) - the same priority/fall-through/one-shot/gag rules a real line\nwould get. Returns the number of non-quiet macros that fired.\n\n-g          Match only global triggers (not restricted to a world).\n-w[world]   Match triggers for [world], or the current world if omitted.\n            Neither -g nor -w given: both are matched (TF's default).\n-n          Don't execute - list which macros would match.\n-l          Like -n, but list each macro in full (as /list would).\n-h<event>   Match hooks for <event> instead of line triggers - <text> is\n            the hook's own argument text. See /help hooks.\n-d          Delete every macro whose trigger matches <text> (Clay extra).\n\nExamples:\n  /def -t\"hello*\" greet = /echo hi %1\n  /trigger hello world         - prints \"hi hello\"\n  /trigger -n You are hungry   - lists matches without running them\n  /trigger -hCONNECT somehost\n  /trigger -hSEND greet bob".to_string()
            )),
            "input" => TfCommandResult::Success(Some(
                "/input text\n\nInsert text into the input buffer at the cursor position.\n\nFunction form: $[input(text)]\n\nExample: /input say hello".to_string()
            )),
            "grab" => TfCommandResult::Success(Some(
                "/grab <text>\n\nPut <text> into the input buffer. Any text already in the input\nbuffer is discarded first (unlike /input, which inserts without\ndiscarding).\n\nExample:\n  /def reedit = /grab /edit %1 = $%1".to_string()
            )),
            "substitute" => TfCommandResult::Success(Some(
                "/substitute [-a<attrs>] [-p] text\n\nReplace the current trigger line with different text.\nOnly works inside a trigger body.\n\nOptions:\n  -a<attrs>  Give text the named display attributes\n  -p         Interpret \"@{attr}\" sequences inline, as in /echo\n\nFunction form: $[substitute(text [,attrs [,inline]])]\n\nExample:\n  /def -t\"* says *\" colorize = /substitute [%1] %2".to_string()
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
            _ => {
                // Try Clay's help topics before giving up
                if let Some(lines) = crate::popup::definitions::help::get_topic_help(topic.as_str()) {
                    TfCommandResult::Success(Some(lines.join("\n")))
                } else {
                    TfCommandResult::Success(Some(format!("No help available for '{}'\nUse /help for a list of all commands.", topic)))
                }
            }
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
        Ok(value) => {
            // Job 15: also set %? (verified directly against real tf: "/expr 1+2"
            // prints "3" AND leaves %?="3" - Clay used to only print it).
            engine.set_global("?", value.clone());
            TfCommandResult::Success(Some(value.to_string_value()))
        }
        Err(e) => TfCommandResult::Error(format!("Expression error: {}", e)),
    }
}

/// /eval [-s<level>] <text> - TF's "one more substitution pass, then execute"
/// (finding B; `/help eval`, verified against real tf 5.0 beta 8): `<text>`
/// arrives here completely raw (`execute_tf_command`'s `is_eval_command`
/// exemption - see its own doc comment) and this does the substitution pass
/// itself, rather than the old pass-through that just dispatched whatever the
/// generic top-level substitution had already produced.
///
/// `-s<level>` (default 1, TF's own default is "full") is accepted but
/// simplified to a boolean rather than TF's real off(0)/on(1)/full(2)
/// three-way `%sub` flag (`/help sub`) - the only value any real script in
/// this corpus ever passes is `-s0` (TF's stdlib `/runtime` and its own
/// `/help eval` example both use exactly that, meaning "don't substitute,
/// just dispatch"), so `level == 0` skips the substitution pass entirely and
/// anything else does the ordinary full pass. `-s1`/`-s2`/... are accepted
/// syntactically but all behave like the default.
///
/// After substitution, the result is executed as a genuine command line
/// (`execute_command_substituted` - a `/`-command dispatches through the same
/// macro-or-builtin lookup a typed command would, never re-substituted a
/// second time) or, if it doesn't start with `/`, sent to the MUD - exactly
/// TF's own "goes through substitution, and is executed" description of
/// `eval()`/`/eval`.
fn cmd_eval(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let (level, text) = parse_eval_level(args);
    let text = text.trim();
    if text.is_empty() {
        return TfCommandResult::Success(None);
    }

    let substituted = if level == 0 {
        text.to_string()
    } else {
        super::variables::substitute_commands(engine, text)
    };
    let substituted = substituted.trim();
    if substituted.is_empty() {
        return TfCommandResult::Success(None);
    }

    if substituted.starts_with('/') {
        execute_command_substituted(engine, substituted)
    } else {
        TfCommandResult::SendToMud(substituted.to_string())
    }
}

/// /not [-s<level>] <command> - finding 13: run <command> as a command (identical
/// substitution/dispatch to `cmd_eval` above, including the shared `-s<level>` option -
/// `/help eval`: "Command usage: /EVAL [-s<level>] <text> / /NOT [-s<level>] <text>")
/// and set %? to the LOGICAL NEGATION of whatever the command left there (`/help eval`:
/// "the return value of /not is the logical negation of return value of the last
/// command in <text>"). Clay used to treat the argument as a bare EXPRESSION, which
/// made e.g. "/not /test 1" fail outright ("Unexpected token: Slash") instead of
/// running /test and negating its result.
///
/// Lives here (not builtins.rs, where every other Tier-1 command is) because it needs
/// `execute_command_substituted` and `parse_eval_level`, both private to this module -
/// same reason `cmd_eval` itself is here. An `Error` result is passed through as-is,
/// without touching %? - there's no legitimate return value to negate in that case.
fn cmd_not(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let (level, text) = parse_eval_level(args);
    let text = text.trim();
    if text.is_empty() {
        return TfCommandResult::Error("Usage: /not [-s<level>] <command>".to_string());
    }

    let substituted = if level == 0 {
        text.to_string()
    } else {
        super::variables::substitute_commands(engine, text)
    };
    let substituted = substituted.trim();
    if substituted.is_empty() {
        return TfCommandResult::Error("Usage: /not [-s<level>] <command>".to_string());
    }

    let result = if substituted.starts_with('/') {
        execute_command_substituted(engine, substituted)
    } else {
        // A "simple command" (plain text sent to the world) - real tf's own condition
        // rule for this shape is "true iff there is a current socket"
        // (`control_flow::execute_condition_command` already implements the same rule
        // for "/if <text>%; /then" - mirrored here rather than shared, since that
        // function returns a `(TfValue, TfCommandResult)` pair this call site doesn't
        // need). Approximated as "always sent successfully" since this engine has no
        // socket-connectivity state of its own to consult.
        engine.set_global("?", TfValue::Integer(1));
        TfCommandResult::SendToMud(substituted.to_string())
    };

    if !matches!(result, TfCommandResult::Error(_)) {
        let current = engine.get_var("?").map(|v| v.to_bool()).unwrap_or(false);
        engine.set_global("?", TfValue::Integer(if current { 0 } else { 1 }));
    }

    result
}

/// Parse `/eval`'s (and `/not`'s, which shares the same option) leading
/// `-s<level>` option: `"-s0 %{*}"` -> `(0, "%{*}")`; `"/echo hi"` (no flag)
/// -> `(1, "/echo hi")`. Only recognises a `-s` immediately followed by
/// digits (then whitespace or end of string) as the flag, so eval'd text
/// that happens to start with a literal "-s..." (unlikely, but not
/// impossible) is left alone.
fn parse_eval_level(args: &str) -> (u32, &str) {
    let trimmed = args.trim_start();
    if let Some(rest) = trimmed.strip_prefix("-s") {
        let digit_end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if digit_end > 0 && (digit_end == rest.len() || rest[digit_end..].starts_with(char::is_whitespace)) {
            let level: u32 = rest[..digit_end].parse().unwrap_or(1);
            return (level, rest[digit_end..].trim_start());
        }
    }
    (1, trimmed)
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
    // Check if this is a complete inline block (from macro execution, or a
    // single physical line using TF's "%;" command separator - finding C.3).
    // A macro body's control-flow block already arrives with real newlines
    // (macros::split_body_preserving_control_flow); a single-line /if using
    // "%;" (with or without a space before the following keyword - both must
    // behave the same) is normalised to the same newline-joined form here so
    // both share the one inline-block executor below instead of the
    // whitespace-sensitive single-line heuristic mishandling one of them.
    let normalized = control_flow::normalize_percent_semi_to_lines(args);
    let if_args_lower = normalized.to_lowercase();
    if normalized.contains('\n') && if_args_lower.contains("/endif") {
        // Reconstruct the full block by prepending "/if "
        let full_block = format!("/if {}", normalized);
        let results = control_flow::execute_inline_if_block(engine, &full_block);
        return aggregate_inline_results(engine, results);
    }

    // Check for single-line form: /if (condition) command
    if let Some((condition, command)) = control_flow::parse_single_line_if(args) {
        return control_flow::execute_single_if(engine, &condition, &command);
    }

    // TF's command-form condition ("/if /command%; /then ..." - finding
    // C.8/P1.8) always needs an explicit /then on the same logical line
    // (joined above via "%;" or a macro body's own newlines, same as
    // /endif above) - real TF has no bare "/if /cmd body" shorthand the
    // way the parenthesized form does. If we get here, that didn't happen
    // (missing /then, missing /endif, or some other malformed shape), so
    // give a clear, TF-specific error instead of the generic "must be
    // enclosed in parentheses" below - and, like that Err arm, never touch
    // engine.control_state, so a forgotten /then can't leave the engine
    // stuck in an unterminated /if.
    if control_flow::is_command_form_condition(args) {
        return TfCommandResult::Error(
            "/if command-form condition (\"/if /command%; /then ...\") requires /then".to_string()
        );
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
    // Check if this is a complete inline block (from macro execution, or a
    // single physical line using "%;" as the body/terminator separator -
    // see cmd_if's comment and finding C.3).
    let normalized = control_flow::normalize_percent_semi_to_lines(args);
    let args_lower = normalized.to_lowercase();
    if normalized.contains('\n') && args_lower.contains("/done") {
        // Reconstruct the full block by prepending "/while "
        let full_block = format!("/while {}", normalized);
        let results = control_flow::execute_inline_while_block(engine, &full_block);
        return aggregate_inline_results(engine, results);
    }

    // TF's command-form condition ("/while /command%; /do ..." - finding
    // C.8/P1.8) - see cmd_if's matching comment for why this must always
    // reach the inline-block path above (an explicit /do on the same
    // logical line), and why falling through here means something in that
    // shape was malformed rather than that a bare "/while /cmd" should
    // start an open-ended ControlState.
    if control_flow::is_command_form_condition(args) {
        return TfCommandResult::Error(
            "/while command-form condition (\"/while /command%; /do ...\") requires /do".to_string()
        );
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
    // Check if this is a complete inline block (from macro execution, or a
    // single physical line using "%;" as the body/terminator separator -
    // see cmd_if's comment and finding C.3).
    let normalized = control_flow::normalize_percent_semi_to_lines(args);
    let for_args_lower = normalized.to_lowercase();
    if normalized.contains('\n') && for_args_lower.contains("/done") {
        let full_block = format!("/for {}", normalized);
        let results = control_flow::execute_inline_for_block(engine, &full_block);
        return aggregate_inline_results(engine, results);
    }

    // TF's OWN `/for var min max command` form (finding C.7/P1.7): unlike
    // Clay's numeric extension below, <command> is not a later physical
    // line collected via ControlState - it is the rest of *this* line, run
    // once per iteration (tf-help /for: "The <variable> will take on all
    // numeric values between <start> and <end> ... The <commands> will be
    // executed once for each of the values"). `split_for_command_form`
    // detects this by peeking at the 4th raw (still-unsubstituted - see
    // execute_tf_command's is_control_flow_command gate) token: Clay's own
    // numeric [step] extension never puts anything but a number there, so
    // anything else can only be the start of a command.
    //
    // Critical substitution timing (P1.7): `command_text` is passed to
    // execute_for_loop completely unsubstituted; that function (shared
    // with the numeric block form's /done path above) re-substitutes each
    // body line fresh on every iteration, which is what makes "%i" pick up
    // the loop variable's *current* value instead of whatever it was (or
    // wasn't) before the loop started.
    if let Some((var_name, min_text, max_text, command_text)) =
        control_flow::split_for_command_form(args)
    {
        let min_text = super::variables::substitute_commands(engine, &min_text);
        let max_text = super::variables::substitute_commands(engine, &max_text);
        let start: i64 = match min_text.trim().parse() {
            Ok(v) => v,
            Err(_) => return TfCommandResult::Error(format!("Invalid start value: {}", min_text.trim())),
        };
        let end: i64 = match max_text.trim().parse() {
            Ok(v) => v,
            Err(_) => return TfCommandResult::Error(format!("Invalid end value: {}", max_text.trim())),
        };
        // Real TF's /for only ever counts up - "If <end> is less than
        // <start>, <commands> will not be executed" (tf-help /for), not a
        // reversed step the way Clay's own numeric extension below has.
        let body: Vec<String> = control_flow::split_percent_semi(&command_text)
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let results = control_flow::execute_for_loop(engine, &var_name, start, end, 1, &body);
        return aggregate_inline_results(engine, results);
    }

    // Clay extension: /for var start end [step] ... /done - numeric bounds,
    // body arrives via later physical lines (documented in /help for).
    match control_flow::parse_for_args(&super::variables::substitute_commands(engine, args)) {
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
        return TfCommandResult::Success(Some(macros::list_macros(engine, None, false)));
    }

    match macros::parse_def(args) {
        Ok(mut macro_def) => {
            if let Err(e) = macros::resolve_priority_expr(engine, &mut macro_def) {
                return TfCommandResult::Error(e);
            }
            apply_macro_def(engine, macro_def)
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// Register an already-built [`TfMacro`] - shared by [`cmd_def`] (after
/// `macros::parse_def` builds `macro_def` from typed `/def` option syntax) and
/// `cmd_bind` (which builds the same nameless, keybinding-only `TfMacro`
/// directly, with no `/def` text to parse at all - TinyFugue: "`/bind
/// <sequence> = <command>` is equivalent to `/def -b"<sequence>" = <command>`",
/// `/help bind` - TF-parity plan finding 40 / Job 21). Handles redefinition,
/// the REDEF hook, and keybinding-table registration identically regardless of
/// which caller built `macro_def`, so `/bind`'s body defers substitution to
/// keypress exactly the way `/def -b`'s own body always has, instead of being
/// substituted once, eagerly, at bind time.
fn apply_macro_def(engine: &mut TfEngine, macro_def: TfMacro) -> TfCommandResult {
    // Check if macro with same name exists. A nameless macro (P1.2) never
    // redefines anything - every nameless /def creates a brand new macro,
    // matching TF (its only handle is its number, assigned fresh below).
    let existing_idx = if macro_def.name.is_empty() {
        None
    } else {
        engine.macros.iter().position(|m| m.name == macro_def.name)
    };

    // Hook registration needs no separate step: `macro_def.hook` (set above by
    // `macros::parse_def`) is all `hooks::fire_hook`/`list_hooks` need - both
    // scan `engine.macros` directly (finding C.10 / plan step P1.9 removed the
    // old by-name `engine.hooks` registry this used to also populate).

    // Register keybinding if present. A named macro is bound by re-injecting its
    // bare name as if typed (see input_handler.rs's KeyAction::SendCommand); a
    // nameless macro has no name to re-inject, so bind directly to its body
    // instead - the same "runs the body" contract, just without a name in the way.
    if let Some(ref keys) = macro_def.keybinding {
        let binding_target = if macro_def.name.is_empty() {
            macro_def.body.clone()
        } else {
            macro_def.name.clone()
        };
        engine.keybindings.insert(keys.clone(), binding_target);
    }

    // Replace existing or add new
    if let Some(idx) = existing_idx {
        if !engine.redef_enabled() {
            // TF: with `redef` off, redefining an existing macro is a hard
            // error instead - and the OLD definition is kept (verified
            // directly against real tf: `% <path>, line N: DEF: macro a
            // already exists`, `/help redef`). Finding 25.
            return TfCommandResult::Success(Some(engine.format_diag(
                &format!("DEF: macro {} already exists", macro_def.name)
            )));
        }

        // Real TF fires the REDEF hook - and prints its own default
        // message, "% [loc]DEF: Redefined macro <name>" - only when the
        // new definition is NOT identical to the old one (`/help hooks`:
        // "the REDEF hook will be called, unless the new macro is
        // identical to the original" - verified directly against real
        // tf). kbbind.tf relies on the hook half of this (`/def -i -ag
        // -hREDEF ~gag_redef` gags every REDEF message while it loads
        // its many key-name macro definitions, then `/undef`s the gag
        // macro when done) - finding 25.
        let identical = macro_definitions_equal(&engine.macros[idx], &macro_def);
        let name = macro_def.name.clone();
        let sequence_number = engine.macros[idx].sequence_number;
        engine.replace_macro(idx, macro_def);
        // %? = the macro's own number, same as a fresh /def below (`/help
        // def`: "the macro with /list, or from the return value of /def or
        // /edit") - preserved by `replace_macro`, so this is the SAME number
        // the macro already had.
        engine.set_global("?", TfValue::Integer(sequence_number as i64));

        if identical {
            return TfCommandResult::Success(None);
        }

        let outcome = hooks::fire_hook(engine, TfHookEvent::Redef, &name);
        let gagged = outcome.matched_any && outcome.first_fired_gagged == Some(true);
        let mut results = outcome.results;
        if !gagged {
            results.push(TfCommandResult::Success(Some(
                engine.format_diag(&format!("DEF: Redefined macro {}", name))
            )));
        }
        aggregate_results_with_engine(engine, results)
    } else {
        let sequence_number = engine.add_macro(macro_def);
        // %? = the new macro's number (`/help undefn`'s own cross-reference:
        // "Macro numbers can be determined with /list, or from the return
        // value of the command used to create the macro" - finding B, needed
        // for /undefn's own documented `/undefn %?` idiom to have anything to
        // read).
        engine.set_global("?", TfValue::Integer(sequence_number as i64));
        TfCommandResult::Success(None)
    }
}

/// Whether two macro definitions are "the same definition" for TF's own
/// REDEF-hook carve-out (`/help hooks`: "the REDEF hook will be called,
/// unless the new macro is identical to the original") - compares every
/// field that defines the macro's own behaviour, deliberately excluding
/// `sequence_number` (bookkeeping - a redefinition keeps the OLD one, see
/// `TfEngine::replace_macro`) and `shots_remaining` (runtime one-shot
/// countdown state, not part of what was actually /def'd).
fn macro_definitions_equal(a: &TfMacro, b: &TfMacro) -> bool {
    let trigger_key = |m: &TfMacro| m.trigger.as_ref().map(|t| (t.pattern.clone(), t.match_mode));
    a.body == b.body
        && trigger_key(a) == trigger_key(b)
        && a.hook == b.hook
        && a.hook_pattern == b.hook_pattern
        && a.keybinding == b.keybinding
        && a.attributes == b.attributes
        && a.priority == b.priority
        && a.fall_through == b.fall_through
        && a.partial_hilite == b.partial_hilite
        && a.one_shot == b.one_shot
        && a.condition == b.condition
        && a.probability == b.probability
        && a.world == b.world
        && a.invisible == b.invisible
        && a.quiet == b.quiet
        && a.world_type == b.world_type
}

/// /edit [options] name [= body] - Edit an existing macro
fn cmd_edit(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args_trimmed = args.trim();
    if args_trimmed.is_empty() {
        return TfCommandResult::Error("Usage: /edit [options] name [= body]".to_string());
    }

    // Parse the edit args the same way as /def
    let mut edit_def = match macros::parse_def(args_trimmed) {
        Ok(d) => d,
        Err(e) => return TfCommandResult::Error(e),
    };
    if let Err(e) = macros::resolve_priority_expr(engine, &mut edit_def) {
        return TfCommandResult::Error(e);
    }

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
    } else if name.is_empty() {
        // A nameless macro is addressed only by #N (or, if it has a trigger, by
        // $pattern above) - an empty name here must never match one by accident.
        None
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
    // World type (-T)
    if edit_def.world_type.is_some() {
        edited.world_type = edit_def.world_type;
    }
    // Invisible / quiet (sticky once set - /edit has no way to clear a bare flag)
    if edit_def.invisible {
        edited.invisible = true;
    }
    if edit_def.quiet {
        edited.quiet = true;
    }

    // Body: only update if '=' was present in the args
    if args_trimmed.contains(" = ") || args_trimmed.ends_with(" =") {
        edited.body = edit_def.body;
    }

    // Replace the macro (preserves sequence number)
    engine.replace_macro(idx, edited);

    TfCommandResult::Success(Some(format!("Macro '{}' edited.", engine.macros[idx].name)))
}

/// /undef <name>... - TF: for each `<name>` given, remove the macro with
/// that name (`/help undef`). Each name is processed independently - one
/// missing name doesn't stop the rest (same pattern as `cmd_undefn` below,
/// verified directly against real tf: `/undef a nosuch b` still removes
/// both `a` and `b`). Silent on success (verified directly against real tf:
/// neither an interactive nor a loaded-file `/undef` of an existing macro
/// prints anything); on a missing name, prints TF's own diagnostic per name
/// - finding 25 - `% [<path>, line <N>: ]UNDEF: Macro "<name>" was not
/// defined.` (`TfEngine::format_diag`).
fn cmd_undef(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: /undef <name>...".to_string());
    }

    let mut messages = Vec::new();
    for name in args.split_whitespace() {
        if !macros::undef_macro(engine, name) {
            messages.push(engine.format_diag(&format!("UNDEF: Macro \"{}\" was not defined.", name)));
        }
    }

    if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
}

/// /undefn <number>... - TF: remove the macro(s) with the given sequence
/// number(s) - `/def`'s own return value, `%?` (`/help undefn`; finding B:
/// Clay's `/undefn` used to take a NAME PATTERN instead, which is now
/// `/purge -mglob`). Silent on success; each bad token (not a number, or no
/// macro with that number) prints its own `% [loc]UNDEFN: ...` diagnostic
/// and processing continues with the rest of the tokens - verified directly
/// against real tf (`/undefn 555 999998 780` still removes 555 and 780 even
/// though the middle, nonexistent number errors on its own). The exact
/// wording ("no macro with number N" / "invalid or missing numeric
/// argument") is real tf's own, not the more generic phrasing `/help
/// undefn` itself might suggest.
fn cmd_undefn(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        return TfCommandResult::Error("Usage: /undefn <number>...".to_string());
    }

    let mut messages = Vec::new();
    for tok in args.split_whitespace() {
        match tok.parse::<u32>() {
            Ok(number) => {
                if !macros::undef_by_number(engine, number) {
                    messages.push(engine.format_diag(&format!("UNDEFN: no macro with number {}", number)));
                }
            }
            Err(_) => {
                messages.push(engine.format_diag("UNDEFN: invalid or missing numeric argument"));
            }
        }
    }

    if messages.is_empty() {
        TfCommandResult::Success(None)
    } else {
        TfCommandResult::Success(Some(messages.join("\n")))
    }
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

/// /list [<macro-options>] [<name>] [= <body>] - List macros matching a filter.
/// See `macros::MacroFilter` for the option grammar (finding C.4, plan step P1.5).
fn cmd_list(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let default_style = macros::default_matching_style(engine);
    match macros::MacroFilter::parse(args, macros::FilterKind::List, default_style) {
        Ok(filter) => {
            // Job 15: %? is the sequence number of the LAST matching macro, or 0 if
            // none match (verified directly against real tf: "/list -s -i foo" leaves
            // %?=foo's own number; a filter matching nothing leaves %?=0). This is what
            // makes stdlib.tf's own "ismacro"/"isvar" shadow macros (both wrappers
            // around "/@list ...") work once stdlib.tf is loaded, the same way this
            // job's native /ismacro (used when it ISN'T) sets %? itself.
            let last = engine.macros.iter()
                .filter(|m| filter.matches(m))
                .map(|m| m.sequence_number)
                .max()
                .unwrap_or(0);
            let text = macros::list_macros_with_filter(engine, &filter);
            engine.set_global("?", TfValue::Integer(last as i64));
            TfCommandResult::Success(Some(text))
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

/// /purge [<macro-options>] [<name>] [= <body>] - Remove macros matching a filter
/// (no arguments: every non-invisible macro). Same option grammar as /list, except
/// /purge never takes -s/-S. See `macros::MacroFilter`.
///
/// Silent on success (`Success(None)`), matching real TinyFugue exactly: verified
/// against `tf` directly (a `/purge` from a loaded file or typed interactively
/// prints nothing either way) and by tests/tf/cases/purge_args.tf's own oracle
/// `.expected` file, which has no "N macro(s) purged." line. `purge_macros_with_filter`
/// still returns the count for any future caller that needs it (e.g. TF's own %?).
fn cmd_purge(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let default_style = macros::default_matching_style(engine);
    match macros::MacroFilter::parse(args, macros::FilterKind::Purge, default_style) {
        Ok(filter) => {
            macros::purge_macros_with_filter(engine, &filter);
            TfCommandResult::Success(None)
        }
        Err(e) => TfCommandResult::Error(e),
    }
}

// =============================================================================
// Hook and Keybinding Commands
// =============================================================================

/// /hook [ON|OFF] | /hook <event>[ <pattern>] [= <body>] - the %hook flag, or
/// register a hook. Per `/help hook`: "`/hook <event>[ <pattern>] [=<response>]`
/// is equivalent to `/def -h"<event>[ <pattern>]" [=<response>]`" - so this
/// just builds that `-h"..."` argument string and delegates to `cmd_def`
/// rather than re-implementing macro-definition semantics (name-optional,
/// priority, silent-on-success, ...) a second time (finding C.10 / plan P1.9).
/// Verified against real tf: a bare `/hook EVENT` with no `=` creates a new
/// nameless, empty-body hook - it does NOT list existing hooks for that event
/// (Clay's own pre-Job-10 behavior did the latter; this fixes that to match).
fn cmd_hook(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let args = args.trim();

    if args.is_empty() {
        // Bare /hook: list all hooks (mirrors bare /def listing all macros).
        return TfCommandResult::Success(Some(hooks::list_hooks(engine)));
    }

    let upper = args.to_uppercase();
    if upper == "ON" || upper == "OFF" {
        engine.set_global("hook", TfValue::from(if upper == "ON" { "1" } else { "0" }));
        return TfCommandResult::Success(None);
    }

    let (event_and_pattern, body) = match args.find('=') {
        Some(pos) => (args[..pos].trim(), Some(args[pos + 1..].trim())),
        None => (args, None),
    };
    let escaped = event_and_pattern.replace('\\', "\\\\").replace('"', "\\\"");
    let def_args = match body {
        Some(b) => format!("-h\"{}\" = {}", escaped, b),
        None => format!("-h\"{}\"", escaped),
    };
    cmd_def(engine, &def_args)
}

/// /unhook <event> [<pattern>] - remove hooked macros. Verified against real
/// tf: omitting `<pattern>` removes EVERY macro hooked to `<event>` regardless
/// of its own pattern, silently, whether or not any existed; giving a
/// `<pattern>` removes only macros whose own hook pattern is EXACTLY that
/// string, and prints "No hook on <event> <pattern>." when nothing matched
/// (see `hooks::unregister_hooks`'s doc comment for the exact-match rule).
fn cmd_unhook(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return TfCommandResult::Error("Usage: /unhook event [pattern]".to_string());
    }

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let event_str = parts.next().unwrap_or("");
    let pattern = parts.next().map(|p| p.trim_start()).filter(|p| !p.is_empty());

    let event = match TfHookEvent::parse(event_str) {
        Some(e) => e,
        None => return TfCommandResult::Error(format!("Unknown hook event: {}", event_str)),
    };

    let removed = hooks::unregister_hooks(engine, event, pattern);
    if pattern.is_some() && removed == 0 {
        TfCommandResult::Error(format!("No hook on {}.", trimmed))
    } else {
        TfCommandResult::Success(None)
    }
}

/// /bind [key [= command]] - Register or list keybindings.
///
/// TF-parity plan finding 40 / Job 21: real TF's own `/help bind` says
/// "`/bind <sequence> = <command>` is equivalent to `/def -b"<sequence>" =
/// <command>`" - so `key = command` builds exactly the nameless,
/// keybinding-only `TfMacro` a typed `/def -b'<key>' = <command>` would, and
/// registers it through the same [`apply_macro_def`] `/def` itself uses,
/// rather than writing `command` into `engine.keybindings` directly. This is
/// what makes the command text defer substitution to keypress time (handled
/// by whatever runs the macro body later) instead of being substituted once,
/// eagerly, right now - the bug finding 40 identified (also see
/// `execute_tf_command`'s `is_def_like_command`, which keeps `/bind`'s own
/// argument line out of the generic top-level substitution pass the exact
/// same way it already does for `/def`'s).
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

        let canonical = match hooks::parse_key_name(key) {
            Ok(k) => k,
            Err(e) => return TfCommandResult::Error(e),
        };
        let macro_def = TfMacro {
            keybinding: Some(canonical),
            body: command.to_string(),
            ..Default::default()
        };
        apply_macro_def(engine, macro_def)
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

/// /fg [-nsq<>l] [-c<N>] [<world>] - switch to (foreground) a world.
///
/// Options added in plan Job 14b (`/help fg`), verified directly against real
/// tf 5.0 beta 8 (`/listsockets`'s `*`/`fg` marker as the oracle, since there's
/// no other way to observe which socket is "current" in batch mode):
///
///   -<, ->    Cycle to the previous/next CONNECTED world, in `world_info_cache`
///             order (Phase 2 wires Esc-Left/Esc-Right to these).
///   -c<N>     Move N sockets forward (negative: backward) instead of one.
///             Real TF quirk, verified empirically: when more than one of
///             `-c<N>`/`-<`/`->` appears on the same command line, only the
///             LAST one sets the actual move amount - `/fg -c2 ->` moves only
///             1 socket (the trailing `->` overwrites `-c2`'s pending 2 with
///             1), but `/fg -> -c2` moves 2 (here `-c2` is last). This
///             implementation matches that: each of the three just
///             overwrites the same pending "how far to move" value.
///   -s        Suppress the "world not found" error - Clay-specific scoping:
///             real TF's own `-s` covers "no socket named X" (not connected),
///             but Clay's plain `/fg <name>` (no other flags) has always
///             auto-connected an unconnected-but-*defined* world as a
///             convenience (bouncing to `/worlds <name>`, which has both
///             foreground-and-connect behavior) - `-s` only silences the case
///             that convenience can't already handle: `<name>` isn't a world
///             Clay knows about at all.
///   -n        Real TF backgrounds every open socket outright, discarding any
///             world/cycle argument that came with it (verified: `/fg -n foo`
///             leaves NO socket marked current, regardless of `foo`). Clay's
///             single-pane console always displays exactly one world, so
///             "no world is foreground" has nothing to represent - accepted,
///             not distinct: `-n` alone is a no-op success (stdlib's `/bg` is
///             `/fg -n`).
///   -q, -l    Accepted, not distinct: `-q` is TF's "jump to the bottom
///             instead of resuming the old scroll position" (Clay's world
///             switch doesn't track a separate "last foregrounded" scroll
///             offset to resume from a different place than the freshest
///             backlog), and `-l` is a no-op in real TF too ("/help fg": "-l
///             ignored").
///
/// Bare `/fg` (no args at all) is unchanged: shows the current world/
/// connections list, same as `/connections`/`/l` (see cmd_connections).
fn cmd_fg(engine: &TfEngine, args: &str) -> TfCommandResult {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        // No argument - show current world or list. Call cmd_connections directly
        // (same as /connections/l) instead of bouncing through ClayCommand, so
        // `/quote `` `/fg` `` can capture it — see cmd_connections's own doc comment.
        return cmd_connections(engine, "");
    }

    let mut silent = false;
    let mut no_fg = false;
    let mut cycle: Option<i32> = None;
    let mut rest_tokens: Vec<&str> = Vec::new();

    for tok in trimmed.split_whitespace() {
        if tok == "-<" {
            cycle = Some(-1);
        } else if tok == "->" {
            cycle = Some(1);
        } else if let Some(n) = tok.strip_prefix("-c").filter(|n| !n.is_empty()).and_then(|n| n.parse::<i32>().ok()) {
            cycle = Some(n);
        } else if tok.len() > 1 && tok.starts_with('-') && tok[1..].chars().all(|c| "nsql".contains(c)) {
            for c in tok[1..].chars() {
                match c {
                    'n' => no_fg = true,
                    's' => silent = true,
                    'q' | 'l' => {} // accepted, not distinct - see doc comment
                    _ => {}
                }
            }
        } else {
            rest_tokens.push(tok);
        }
    }

    if no_fg {
        // See -n's doc comment above: not representable in Clay's single-pane
        // display, so this is a silent no-op (matches real TF printing nothing
        // of its own on a successful /fg either).
        return TfCommandResult::Success(None);
    }

    if let Some(n) = cycle {
        let names: Vec<&str> = engine.world_info_cache.iter()
            .filter(|w| w.is_connected)
            .map(|w| w.name.as_str())
            .collect();
        if names.is_empty() {
            return if silent {
                TfCommandResult::Success(None)
            } else {
                TfCommandResult::Error("FG: no sockets are open".to_string())
            };
        }
        let current_pos = engine.current_world.as_deref()
            .and_then(|cur| names.iter().position(|name| *name == cur));
        let len = names.len() as i32;
        let target = match current_pos {
            Some(pos) => (((pos as i32 + n) % len) + len) % len,
            None => 0,
        };
        return TfCommandResult::ClayCommand(format!("/worlds {}", names[target as usize]));
    }

    let world = rest_tokens.join(" ");
    if world.is_empty() {
        return cmd_connections(engine, "");
    }

    if silent {
        let exists = engine.world_info_cache.iter().any(|w| w.name.eq_ignore_ascii_case(&world));
        if !exists {
            return TfCommandResult::Success(None);
        }
    }

    // Switch to specified world
    TfCommandResult::ClayCommand(format!("/worlds {}", world))
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

/// /listvar [-m<matching>] [-gxsv] [<name> [<value>]] - list variables
/// whose name and value match `<name>` and `<value>` under `<matching>`
/// (default `%{matching}`, `macros::default_matching_style`), sorted by
/// name (`/help listvar`). Default output (and `-g`/`-x`) is real tf's own
/// reloadable form - "/set NAME=value" for a plain global, "/setenv
/// NAME=value" for one exported via `/setenv`/`/export` - verified
/// directly against real tf (Clay used to print "NAME = value", which real
/// tf never does). With neither `-g` nor `-x`, both kinds are listed
/// together, each in its own form. `-s` lists names only, `-v` lists
/// values only (neither prefixed). Silent (empty) when nothing matches,
/// matching real tf exactly - no "No variables" placeholder line.
fn cmd_listvar(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let default_style = macros::default_matching_style(engine);
    let mut remaining = args.trim_start();
    let mut style = default_style;
    let mut globals_only = false;
    let mut exported_only = false;
    let mut short = false;
    let mut values_only = false;

    while let Some(rest) = remaining.strip_prefix('-') {
        if rest.is_empty() {
            break;
        }
        // "--" (end-of-options marker, needed so a <name> pattern that
        // itself starts with "-" can never be misread as more flags) -
        // stdlib.tf's own "isvar"/"ismacro" idiom relies on exactly this:
        // `/listvar -msimple -- %*` so `isvar("-foo")` isn't misparsed.
        // Verified directly against real tf: "--" is consumed and parsing
        // stops there, same as getopt(3)'s own convention.
        if let Some(after_dashes) = rest.strip_prefix('-') {
            if after_dashes.is_empty() || after_dashes.starts_with(char::is_whitespace) {
                remaining = after_dashes.trim_start();
                break;
            }
        }
        if let Some(after_m) = rest.strip_prefix('m') {
            let token_end = after_m.find(char::is_whitespace).unwrap_or(after_m.len());
            let (value, tail) = after_m.split_at(token_end);
            style = match TfMatchMode::parse(value) {
                Some(s) => s,
                None => return TfCommandResult::Error(format!("Unknown match mode: {}", value)),
            };
            remaining = tail.trim_start();
            continue;
        }
        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (token, tail) = rest.split_at(token_end);
        if !token.is_empty() && token.chars().all(|c| "gxsv".contains(c)) {
            for c in token.chars() {
                match c {
                    'g' => globals_only = true,
                    'x' => exported_only = true,
                    's' => short = true,
                    'v' => values_only = true,
                    _ => unreachable!("filtered to gxsv above"),
                }
            }
            remaining = tail.trim_start();
            continue;
        }
        break;
    }

    let mut parts = remaining.splitn(2, char::is_whitespace);
    let name_pattern = parts.next().unwrap_or("").trim();
    let value_pattern = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());

    // Collected into owned values (not `&String`/`&TfValue` borrows) so the
    // `%?` write below isn't blocked by an immutable borrow of `engine`
    // that would otherwise have to outlive this whole function.
    let mut vars: Vec<(String, TfValue)> = engine.global_vars.iter()
        .filter(|(name, _)| {
            let is_exported = engine.env_vars.contains(name.as_str());
            !((globals_only && is_exported) || (exported_only && !is_exported))
        })
        .filter(|(name, _)| name_pattern.is_empty() || macros::full_match(name, name_pattern, style))
        .filter(|(_, value)| {
            value_pattern.map_or(true, |p| macros::full_match(&value.to_string_value(), p, style))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    vars.sort_by(|a, b| a.0.cmp(&b.0));

    // `/help listvar`: "The return value of /listvar is the number of
    // variables listed" - verified directly against real tf (3 vars
    // matching a glob leaves %?=3, not capped at 1). stdlib.tf's own
    // "isvar" macro (`/def -i isvar = /test tfclose("o")%; /listvar
    // -msimple -- %*`) has no explicit /return - it depends entirely on
    // THIS setting %? itself; before this, %? was left exactly as
    // whatever the macro's earlier "/test tfclose(...)" call happened to
    // leave it (0), so isvar() reported "not set" even for a variable
    // that unquestionably existed.
    engine.set_global("?", TfValue::Integer(vars.len() as i64));

    if vars.is_empty() {
        return TfCommandResult::Success(None);
    }

    let mut lines = Vec::with_capacity(vars.len());
    for (name, value) in vars {
        if short {
            lines.push(name.clone());
        } else if values_only {
            lines.push(value.to_string_value());
        } else if engine.env_vars.contains(name.as_str()) {
            lines.push(format!("/setenv {}={}", name, value.to_string_value()));
        } else {
            lines.push(format!("/set {}={}", name, value.to_string_value()));
        }
    }
    TfCommandResult::Success(Some(lines.join("\n")))
}

/// Fire a trigger (or, with `-h<event>`, a hook) manually.
fn cmd_trigger(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    let trimmed = args.trim_start();

    // /trigger -h<event> <text> - fire a hook exactly as if <text> were its
    // argument (finding C.10 / plan step P1.9; see /help /trigger: "-h<event>
    // Match hooks where <event> matches the hook event and <text> matches the
    // hook argument pattern"). Real TF prints nothing of its own here - what
    // hooks.tf/lib_alias.tf actually see on screen comes from two things: the
    // fired macro(s)' own output, and (verified directly against real tf,
    // repeatable) a local-echo of <text> itself, standing in for the default
    // hook message that real TF would otherwise route to a world's own output
    // stream - except there's no live world under /trigger's simulation, so
    // for the six "W"-tagged events (see TfHookEvent::is_world_stream_event)
    // nothing is ever echoed. That echo is itself suppressed exactly the way a
    // real default hook message would be: gagged by the attributes of the
    // macro that actually fired (verified against alias.tf's own "-ag" SEND
    // hook, whose default message never appears - lib_alias.tf's oracle output
    // is just "greetings bob", no raw "greet bob" line).
    if let Some(rest) = trimmed.strip_prefix("-h") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return TfCommandResult::Error("Usage: /trigger -h<event> <text>".to_string());
        }
        let (event_str, text) = match rest.find(char::is_whitespace) {
            Some(pos) => (&rest[..pos], rest[pos..].trim_start()),
            None => (rest, ""),
        };
        let event = match TfHookEvent::parse(event_str) {
            Some(e) => e,
            None => return TfCommandResult::Error(format!("Unknown hook event: {}", event_str)),
        };

        let outcome = hooks::fire_hook(engine, event, text);
        let mut all_results = Vec::new();
        if !event.is_world_stream_event() && !text.is_empty() {
            let gagged = outcome.first_fired_gagged.unwrap_or(false);
            if !gagged {
                all_results.push(TfCommandResult::Success(Some(text.to_string())));
            }
        }
        all_results.extend(outcome.results);
        return aggregate_results_with_engine(engine, all_results);
    }

    // Parse the remaining flags (`/help /trigger`: "[-ln] [-g] [-w[<world>]] <text>"),
    // plus Clay's own kept `-d` extension (delete matching triggers - finding B ruling:
    // "TF, keep -d"). None of tf-help's own usage bundles these together with a real
    // fixture, so only single-flag tokens are recognised (no "-ln"-style bundling),
    // which is enough for every documented spelling to parse.
    let mut list_only = false;
    let mut list_full = false;
    let mut delete = false;
    let mut want_global = false;
    let mut world_filter: Option<Option<String>> = None; // Some(None) = bare -w; Some(Some(x)) = -wx
    let mut rest = trimmed;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("-w") {
            let (world_name, after) = match r.find(char::is_whitespace) {
                Some(pos) => (&r[..pos], &r[pos..]),
                None => (r, ""),
            };
            world_filter = Some(if world_name.is_empty() { None } else { Some(world_name.to_string()) });
            rest = after;
            continue;
        }
        if let Some(r) = rest.strip_prefix("-l") {
            if r.is_empty() || r.starts_with(char::is_whitespace) {
                list_only = true;
                list_full = true;
                rest = r;
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix("-n") {
            if r.is_empty() || r.starts_with(char::is_whitespace) {
                list_only = true;
                rest = r;
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix("-g") {
            if r.is_empty() || r.starts_with(char::is_whitespace) {
                want_global = true;
                rest = r;
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix("-d") {
            if r.is_empty() || r.starts_with(char::is_whitespace) {
                delete = true;
                rest = r;
                continue;
            }
        }
        break;
    }

    let text = rest.trim();
    if text.is_empty() {
        return TfCommandResult::Error("Usage: /trigger [-ln] [-g] [-w<world>] <text>".to_string());
    }

    // A world name that provably matches no real world's own macros, used below to
    // simulate "-g alone" (globals only, no world-scoped triggers at all) on top of
    // `process_triggers`'s/`collect_trigger_matches`'s existing world filter, which
    // otherwise has no "exclude every world-scoped macro" mode of its own (`world:
    // None` means "don't filter by world", i.e. match everything, not "globals
    // only" - the opposite of what bare `-g` needs).
    const GLOBAL_ONLY_SENTINEL: &str = "\u{0}__trigger_global_only__\u{0}";

    // "-g" and/or "-w<world>"; neither given means "both assumed" (`/help /trigger`),
    // which is exactly `process_triggers`'s own default world-filtering behaviour
    // (globals always match; a world-scoped macro matches only the given world).
    let world_arg: Option<String> = if want_global && world_filter.is_none() {
        Some(GLOBAL_ONLY_SENTINEL.to_string())
    } else if let Some(w) = world_filter {
        Some(w.unwrap_or_else(|| engine.current_world.clone().unwrap_or_default()))
    } else {
        engine.current_world.clone()
    };

    if delete {
        // Clay's own extension (not real TF): remove every macro whose trigger
        // pattern matches `text`, without executing anything.
        let mut to_remove: Vec<usize> = engine.macros.iter()
            .enumerate()
            .filter(|(_, m)| m.trigger.as_ref().is_some_and(|t| macros::match_trigger(t, text).is_some()))
            .map(|(idx, _)| idx)
            .collect();
        to_remove.sort_unstable_by(|a, b| b.cmp(a));
        for idx in to_remove {
            engine.macros.remove(idx);
        }
        return TfCommandResult::Success(None);
    }

    // Same matcher real socket input goes through (macros::process_triggers /
    // macros::match_trigger - finding B: this used to be Clay's own substring-based
    // approximation of a trigger pattern against the given text, matching neither
    // priority, fall-through, one-shot, world, nor real glob/regexp semantics).
    let matched = collect_trigger_matches(engine, text, world_arg.as_deref());

    if list_only {
        // "-n"/"-l": report what WOULD fire, without executing anything.
        if matched.is_empty() {
            return TfCommandResult::Success(None);
        }
        let lines: Vec<String> = matched.iter()
            .map(|&idx| {
                if list_full {
                    macros::format_macro_full(&engine.macros[idx])
                } else {
                    macros::format_trigger_match_summary(&engine.macros[idx])
                }
            })
            .collect();
        return TfCommandResult::Success(Some(lines.join("\n")));
    }

    if matched.is_empty() {
        // Real TF's /trigger returns 0 and prints nothing when no macro matches - it
        // does not error (see tf-help's /trigger: "the number of ... macros that were
        // executed"), and (verified directly against real tf) never echoes the raw
        // <text> either - unlike a real socket line, /trigger's own output is only
        // ever whatever the matched macro(s) themselves produce.
        return TfCommandResult::Success(None);
    }

    // %? = the number of NON-QUIET macros that fired (`/help /trigger`: "the return
    // value of /trigger is the number of (non-quiet) macros that were executed").
    let non_quiet_count = matched.iter().filter(|&&idx| !engine.macros[idx].quiet).count();
    engine.set_global("?", TfValue::Integer(non_quiet_count as i64));

    let results = macros::process_triggers(engine, text, world_arg.as_deref(), None);
    aggregate_results_with_engine(engine, results)
}

/// Indices (into `engine.macros`, priority order) of the macros that would actually
/// fire for `text` under `macros::process_triggers`'s own matching rules (world,
/// world-type, shots-remaining, pattern; stop at the first non-fall-through match) -
/// WITHOUT executing anything. Shared by `/trigger`'s `-n`/`-l` (list, don't fire)
/// modes and its own `%?` return-value count.
fn collect_trigger_matches(engine: &TfEngine, text: &str, world: Option<&str>) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..engine.macros.len()).collect();
    indices.sort_by(|&a, &b| engine.macros[b].priority.cmp(&engine.macros[a].priority));

    let mut matched = Vec::new();
    for idx in indices {
        let m = &engine.macros[idx];

        if let Some(ref macro_world) = m.world {
            if let Some(current_world) = world {
                if macro_world != current_world {
                    continue;
                }
            }
        }

        // -T world-type restriction: /trigger has no live world context to test the
        // pattern against, so (same conservative choice as hooks::fire_hook) a
        // -T-restricted macro is treated as never matching here.
        if !macros::world_type_matches(m, None) {
            continue;
        }

        if let Some(remaining) = m.shots_remaining {
            if remaining == 0 {
                continue;
            }
        }

        let trigger = match &m.trigger {
            Some(t) if !t.pattern.is_empty() => t,
            _ => continue,
        };

        if macros::match_trigger(trigger, text).is_some() {
            let fall_through = m.fall_through;
            matched.push(idx);
            if !fall_through {
                break;
            }
        }
    }
    matched
}

/// Insert text into input buffer
fn cmd_input(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /input text".to_string());
    }

    // Substitute variables in the text
    let text = super::variables::substitute_variables(engine, args);

    // Queue the text insertion
    let insert_mode = engine.insert_mode();
    engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Insert(text, insert_mode));
    TfCommandResult::Success(None)
}

/// /grab <text> - Put `<text>` into the input buffer, discarding whatever
/// was already there (`/help grab`: "Any text already in the input buffer
/// is discarded" - the one difference from `/input`, which inserts without
/// discarding; `cmd_input`'s own doc comment). Implemented as the same
/// clear-then-insert `PendingKeyboardOp` sequence `/dokey DLINE` uses to
/// clear the whole line (`builtins::cmd_dokey`'s "DLINE"/"DELINE" arm)
/// followed by `/input`'s own `Insert` (Job 12's `PendingKeyboardOp`
/// mechanism - job 14c's plan step explicitly calls this combination out).
fn cmd_grab(engine: &mut TfEngine, args: &str) -> TfCommandResult {
    if args.is_empty() {
        return TfCommandResult::Error("Usage: /grab text".to_string());
    }

    // Substitute variables in the text, same as /input.
    let text = super::variables::substitute_variables(engine, args);

    let len = engine.keyboard_state.buffer.len() as i32;
    engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Goto(0));
    if len > 0 {
        engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Delete(len));
    }
    let insert_mode = engine.insert_mode();
    engine.pending_keyboard_ops.push(super::PendingKeyboardOp::Insert(text, insert_mode));
    TfCommandResult::Success(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::WorldInfoCache;

    #[test]
    fn test_is_tf_command() {
        // Only / prefix is recognized as TF command
        assert!(is_tf_command("/quit"));
        assert!(is_tf_command("/set foo"));
        assert!(is_tf_command("  /echo hello"));
        assert!(!is_tf_command("#set foo bar"));  // # is not a command prefix
        assert!(!is_tf_command("say hello"));  // Plain text, not a command
    }

    /// TF-parity plan Job 23 (P2.8, docs): `/help commands` must (a) mention every
    /// name `is_tf_command_name` dispatches, and (b) mention no OTHER "/word" except
    /// a documented Clay-native-only command (verified against `main.rs::parse_command`)
    /// or the special `/tf<name>` prefix escape (`tfhelp`/`tfgag`, handled before the
    /// dispatch match in `execute_command_impl`). This is exactly the class of bug
    /// finding A called out - a stale command name (`/keybinds`) lingering in help text
    /// after the real command was renamed or removed - except automated, so it can't
    /// silently happen again.
    #[test]
    fn test_help_commands_matches_dispatch() {
        // Every literal name `is_tf_command_name`'s `matches!` accepts, pinned (the
        // `dokey_<name>` dynamic-prefix rule is excluded - not a fixed literal, and
        // none of its 35 individual names need their own mention in the help text).
        const DISPATCH_COMMANDS: &[&str] = &[
            ":", "addworld", "bamf", "ban", "beep", "bind", "break", "cat", "cd", "changes",
            "connections", "core", "dc", "def", "disconnect", "do", "dokey", "done", "echo",
            "edit", "else", "elseif", "endif", "endpaste", "escape", "eval", "exit", "export",
            "expr", "false", "features", "fg", "finger", "first", "for", "gag", "getfile", "grab",
            "help", "hilite", "histsize", "hook", "if", "input", "ismacro", "isvar", "kill", "l",
            "last", "lcd", "let", "limit", "list", "listsockets", "liststreams", "listvar",
            "listworlds", "load", "loaded", "localecho", "log", "man", "more", "nogag",
            "nohilite", "not", "nth", "partial", "paste", "ps", "purge", "purgeworld", "putfile",
            "pwd", "quit", "quote", "recall", "recordline", "relimit", "repeat", "replace",
            "require", "rest", "restrict", "result", "return", "runtime", "save", "saveworld",
            "say", "send", "set", "setenv", "sh", "shift", "sub", "substitute", "suspend", "sys",
            "telnet", "test", "then", "tick", "time", "toggle", "tr", "trig", "trigc", "trigger",
            "trigp", "trigpc", "true", "unbind", "undef", "undefn", "undeft", "ungag", "unhook",
            "unlimit", "unset", "untrig", "unworld", "ver", "version", "watchdog", "watchname",
            "while", "world", "wrap", "xtitle",
        ];
        assert_eq!(DISPATCH_COMMANDS.len(), 130, "unexpected count - is_tf_command_name's own list changed size?");
        for name in DISPATCH_COMMANDS {
            assert!(is_tf_command_name(name),
                "{name:?} is pinned in DISPATCH_COMMANDS but is_tf_command_name doesn't \
                 recognize it - update this list to match reality");
        }

        // Genuinely real Clay-native (non-TF) commands this text legitimately mentions
        // (reachable via `main.rs::parse_command`), plus the `/tf<name>` prefix escape.
        const CLAY_NATIVE_ONLY: &[&str] = &[
            "actions", "connect", "dict", "dump", "flush", "font", "import",
            "menu", "note", "notify", "reload", "remote", "setup", "tag",
            "testmusic", "tfgag", "tfhelp", "translate", "unban", "update",
            "urban", "url", "web", "window",
        ];

        let help_text = match cmd_help("commands") {
            TfCommandResult::Success(Some(text)) => text,
            other => panic!("expected /help commands to return text, got {other:?}"),
        };

        let mut missing: Vec<&str> = DISPATCH_COMMANDS.iter().copied()
            .filter(|name| !help_text.contains(&format!("/{name}")))
            .collect();
        missing.sort();
        assert!(missing.is_empty(), "/help commands text is missing dispatched command(s): {missing:?}");

        let known: std::collections::HashSet<&str> = DISPATCH_COMMANDS.iter().copied()
            .chain(CLAY_NATIVE_ONLY.iter().copied())
            .collect();
        let mut unexpected: Vec<String> = Vec::new();
        for line in help_text.lines() {
            for tok in line.split_whitespace() {
                let Some(rest) = tok.strip_prefix('/') else { continue };
                let trimmed = rest.trim_end_matches(|c: char| ".,:;)".contains(c));
                let name = if trimmed.is_empty() && rest.starts_with(':') { ":" } else { trimmed };
                if name.is_empty() || name.contains('<') {
                    continue; // "<command>", "<port>" placeholders in prose
                }
                if !known.contains(name) {
                    unexpected.push(format!("{tok:?} (parsed name {name:?})"));
                }
            }
        }
        unexpected.sort();
        unexpected.dedup();
        assert!(unexpected.is_empty(),
            "/help commands mentions a name that is neither a dispatched TF command nor \
             listed in CLAY_NATIVE_ONLY - stale, renamed, or typo'd: {unexpected:#?}");
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

    /// Finding 19: /set and /let must not trim the value - leading and
    /// trailing spaces in the value are meaningful and TF keeps them.
    /// Verified directly against real tf 5.0 beta 8 (see
    /// `split_set_or_let_value`'s own doc comment for the exact cases).
    #[test]
    fn test_cmd_set_does_not_trim_value_finding_19() {
        let mut engine = TfEngine::new();

        // "name=value" form: value is everything after '=', verbatim.
        cmd_set(&mut engine, "foo= bar ");
        assert_eq!(engine.get_var("foo").map(|v| v.to_string_value()), Some(" bar ".to_string()));

        // A space before '=' makes the '=' itself part of the value (real
        // tf's own "'=' following space is part of value" warning case).
        cmd_set(&mut engine, "foo2 = bar2 ");
        assert_eq!(engine.get_var("foo2").map(|v| v.to_string_value()), Some("= bar2 ".to_string()));

        // "name value" form: exactly the separator whitespace is consumed;
        // trailing whitespace in the value is kept.
        cmd_set(&mut engine, "foo3 bar3 ");
        assert_eq!(engine.get_var("foo3").map(|v| v.to_string_value()), Some("bar3 ".to_string()));

        // Tab-aligned values (tf-lib's own color.tf idiom): the WHOLE run
        // of separator whitespace is consumed, however many characters.
        cmd_set(&mut engine, "foo4\t\t\tbar4   ");
        assert_eq!(engine.get_var("foo4").map(|v| v.to_string_value()), Some("bar4   ".to_string()));
    }

    /// Finding 19 for /let, and finding 20's ": /let always creates/updates
    /// the CURRENT local scope (unlike := - see
    /// `TfEngine::set_existing_or_global`'s doc comment).
    #[test]
    fn test_cmd_let_does_not_trim_value_and_stays_local() {
        let mut engine = TfEngine::new();

        // No local scope: /let falls back to global (existing `set_local`
        // behavior, unchanged by finding 20).
        cmd_let(&mut engine, "x=hello");
        assert_eq!(engine.global_vars.get("x"), Some(&TfValue::String("hello".to_string())));

        // lisp.tf's own /remove idiom: a leading space in the value,
        // produced by "%{_result} %{1}" when _result is empty, must
        // survive.
        engine.push_scope();
        engine.set_global("outer", TfValue::Integer(1));
        cmd_let(&mut engine, "_result= b");
        assert_eq!(
            engine.local_vars_stack.last().unwrap().get("_result").map(|v| v.to_string_value()),
            Some(" b".to_string())
        );
        // /let never touches an existing global of the same name, unlike :=.
        cmd_let(&mut engine, "outer=2");
        assert_eq!(engine.global_vars.get("outer"), Some(&TfValue::Integer(1)), "global must be untouched");
        assert_eq!(
            engine.local_vars_stack.last().unwrap().get("outer").map(|v| v.to_int()),
            Some(Some(2))
        );
        engine.pop_scope();
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
        let mut engine = TfEngine::new();
        let result = cmd_echo(&mut engine, "Hello world");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "Hello world"),
            _ => panic!("Expected success with message"),
        }
    }

    fn echo_text(engine: &mut TfEngine, args: &str) -> String {
        match cmd_echo(engine, args) {
            TfCommandResult::Success(Some(msg)) => msg,
            other => panic!("Expected Success(Some) for {:?}, got {:?}", args, other),
        }
    }

    #[test]
    fn test_cmd_echo_destination_flags_accepted_no_distinct_effect() {
        // -o (default), -e (error stream) and -A (alert stream) are all accepted and all
        // land as ordinary displayed text - Clay has no separate stream for /echo's
        // destination (see cmd_echo's own doc comment on -e specifically: real tf prints
        // "/echo -e %% text" as an ordinary "% text" line, so it must reach the caller as
        // Success(Some(_)), never TfCommandResult::Error).
        let mut engine = TfEngine::new();
        assert_eq!(echo_text(&mut engine, "-o plain"), "plain");
        // "%%" -> "%" is done by the substitution pass BEFORE cmd_echo ever sees the text
        // (variables::substitute_commands), not by cmd_echo itself - this only checks that
        // -e's own text reaches cmd_echo's output untouched, matching real tf's "-e" simply
        // choosing a destination stream rather than transforming the text.
        assert_eq!(echo_text(&mut engine, "-e %% Warning: conflicting defintions."), "%% Warning: conflicting defintions.");
        assert_eq!(echo_text(&mut engine, "-A alert text"), "alert text");
    }

    #[test]
    fn test_cmd_echo_raw_skips_attr_interpretation() {
        let mut engine = TfEngine::new();
        // Without -r, "@{n}" is interpreted (always-on default - see cmd_echo's own doc
        // comment on -p).
        assert_eq!(echo_text(&mut engine, "before @{n} after"), "before \x1b[0m after");
        // With -r, it must appear completely literally.
        assert_eq!(echo_text(&mut engine, "-r before @{n} after"), "before @{n} after");
        // -r bundled with another boolean flag in one token.
        assert_eq!(echo_text(&mut engine, "-pr literal @{n}"), "literal @{n}");
    }

    #[test]
    fn test_cmd_echo_a_attrs_wraps_message() {
        let mut engine = TfEngine::new();
        let msg = echo_text(&mut engine, "-aCred hello");
        assert!(msg.starts_with("\x1b[") && msg.ends_with("\x1b[0m"), "got {:?}", msg);
        assert!(msg.contains("hello"));
    }

    #[test]
    fn test_cmd_echo_w_world_queues_pending_output() {
        let mut engine = TfEngine::new();
        // -w<world> (attached): redirected via pending_outputs, NOT returned directly.
        let result = cmd_echo(&mut engine, "-wOtherMUD hello there");
        assert!(matches!(result, TfCommandResult::Success(None)));
        assert_eq!(engine.pending_outputs.len(), 1);
        assert_eq!(engine.pending_outputs[0].text, "hello there");
        assert_eq!(engine.pending_outputs[0].world.as_deref(), Some("OtherMUD"));

        // Bare -w (blank world) means the current world - same as omitting -w entirely,
        // so it must NOT go through pending_outputs.
        engine.pending_outputs.clear();
        let result = cmd_echo(&mut engine, "-w hello there");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "hello there"),
            other => panic!("Expected Success(Some), got {:?}", other),
        }
        assert!(engine.pending_outputs.is_empty());
    }

    #[test]
    fn test_cmd_echo_end_of_options_marker() {
        let mut engine = TfEngine::new();
        // A message that itself begins with '-' needs the "-" end-of-options marker
        // (/help echo) so it isn't mistaken for a flag.
        assert_eq!(echo_text(&mut engine, "- -1 point penalty"), "-1 point penalty");
    }

    #[test]
    fn test_cmd_send() {
        let engine = TfEngine::new();

        // Simple send: no leading flag at all - the fast path, no ClayCommand round trip.
        let result = cmd_send(&engine, "say hello");
        match result {
            TfCommandResult::SendToMud(text) => assert_eq!(text, "say hello"),
            _ => panic!("Expected SendToMud"),
        }

        // Any recognized leading flag bounces the WHOLE, UNMODIFIED text to Clay's own
        // /send (Command::Send / parse_send_command), which already speaks TF's real
        // attached-flag grammar (-w<world>, no space) - not reimplemented a second time
        // here. This also means "-w TestWorld say hello" (space after -w) is bare -w
        // (current world) with "TestWorld say hello" as the literal message text, per real
        // tf's own -w[<world>] convention - NOT a Clay-specific "-w <world>" shorthand.
        let result = cmd_send(&engine, "-w TestWorld say hello");
        match result {
            TfCommandResult::ClayCommand(cmd) => assert_eq!(cmd, "/send -w TestWorld say hello"),
            _ => panic!("Expected ClayCommand"),
        }

        // The real TF attached form.
        let result = cmd_send(&engine, "-wTestWorld say hello");
        match result {
            TfCommandResult::ClayCommand(cmd) => assert_eq!(cmd, "/send -wTestWorld say hello"),
            _ => panic!("Expected ClayCommand"),
        }

        // New TF options (-W, -T<type>, -n, -h) all bounce the same way.
        for flagged in ["-W quit", "-Tmud who", "-n look", "-h say hi"] {
            match cmd_send(&engine, flagged) {
                TfCommandResult::ClayCommand(cmd) => assert_eq!(cmd, format!("/send {}", flagged)),
                other => panic!("Expected ClayCommand for {:?}, got {:?}", flagged, other),
            }
        }

        // A leading "-" that isn't one of send's own flags is sent as-is (matches real
        // tf's own getopts-style ambiguity - there is no escape hatch documented for /send).
        match cmd_send(&engine, "-1 point penalty") {
            TfCommandResult::SendToMud(text) => assert_eq!(text, "-1 point penalty"),
            other => panic!("Expected SendToMud, got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_substitute_p_flag_accepted() {
        let mut engine = TfEngine::new();

        // -p is accepted (a no-op beyond parsing - see cmd_substitute's own doc comment):
        // "@{...}" is already interpreted unconditionally, with or without -p.
        assert!(matches!(cmd_substitute(&mut engine, "-p hello"), TfCommandResult::Success(None)));
        assert_eq!(engine.pending_substitution.as_ref().unwrap().text, "hello");

        // -p combined with -a<attrs>, either order.
        engine.pending_substitution = None;
        cmd_substitute(&mut engine, "-aCred -p @{n}hello");
        let sub = engine.pending_substitution.take().unwrap();
        assert_eq!(sub.attrs, "Cred");
        assert_eq!(sub.text, "\x1b[0mhello");
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
    fn test_echo_e_flag_lands_as_ordinary_echoed_text() {
        // Real tf prints "/echo -e %% text" as an ordinary "% text" line - -e must land as
        // Success(Some(_)) (echoed), never TfCommandResult::Error, and the top-level
        // substitution pass (not cmd_echo itself) is what turns "%%" into a literal "%".
        let mut engine = TfEngine::new();
        let result = execute_command(&mut engine, "/echo -e %% Warning: conflicting defintions.");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "% Warning: conflicting defintions."),
            other => panic!("Expected Success(Some), got {:?}", other),
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
    fn test_quote_inside_macro_body_is_no_longer_swallowed() {
        // Finding 14 / grep.tf's own shape (/_fgrep + /fgrep): a macro's body issues a
        // synchronous /quote -S <prefix> `<TF_cmd>, whose auto-Exec disposition (a <pre>
        // defaults disposition to "exec" - /help quote) runs "<prefix><captured line>" as
        // a real command. aggregate_results_with_engine used to hit its `_ => {}`
        // catch-all and silently drop the whole TfCommandResult::Quote whenever /quote was
        // called from INSIDE a macro body, so none of this ever actually ran. It must now
        // resolve in-engine and fold the exec'd command's own output back into this call's
        // result.
        use super::super::TfMacro;
        let mut engine = TfEngine::new();
        engine.macros.push(TfMacro {
            name: "echoback".to_string(),
            body: "/echo got: %*".to_string(),
            ..Default::default()
        });
        engine.macros.push(TfMacro {
            name: "runner".to_string(),
            body: "/quote -S /echoback `/echo captured".to_string(),
            ..Default::default()
        });

        let result = execute_command(&mut engine, "/runner");
        match result {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "got: captured"),
            other => panic!("Expected Success(Some(\"got: captured\")), got {:?}", other),
        }
    }

    #[test]
    fn test_quote_send_disposition_inside_macro_body_queues_pending_command() {
        use super::super::TfMacro;
        let mut engine = TfEngine::new();
        engine.macros.push(TfMacro {
            name: "sayit".to_string(),
            body: "/quote -dsend look".to_string(),
            ..Default::default()
        });

        execute_command(&mut engine, "/sayit");
        assert_eq!(engine.pending_commands.len(), 1);
        assert_eq!(engine.pending_commands[0].command, "look");
    }

    #[test]
    fn test_quote_needing_app_bounces_upward_from_macro_body() {
        // A /quote this function genuinely cannot finish itself (here: a scheduled delay)
        // must propagate outward as a real TfCommandResult::Quote, not be silently dropped
        // or half-resolved - exactly like a nested /return/Result already does.
        use super::super::TfMacro;
        let mut engine = TfEngine::new();
        engine.macros.push(TfMacro {
            name: "delayed".to_string(),
            body: "/quote -1 hello".to_string(),
            ..Default::default()
        });

        match execute_command(&mut engine, "/delayed") {
            TfCommandResult::Quote { delay_secs, .. } => assert_eq!(delay_secs, 1.0),
            other => panic!("Expected the unresolved Quote to bounce upward, got {:?}", other),
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

    /// Plan Job 14c: default `/listvar` output (and `-g`/`-x`) is real tf's
    /// own reloadable "/set NAME=value" / "/setenv NAME=value" form
    /// (verified directly against real tf - Clay used to print "NAME =
    /// value", which real tf never does); `-s` names only, `-v` values
    /// only; a `<name>` pattern filters by name, an optional `<value>`
    /// pattern also filters by value; silent (no output) when nothing
    /// matches.
    #[test]
    fn test_listvar_default_form_and_filters() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/set fooA bar");
        execute_command(&mut engine, "/setenv fooB exported_val");

        match execute_command(&mut engine, "/listvar foo*") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "/set fooA=bar\n/setenv fooB=exported_val");
            }
            other => panic!("got {:?}", other),
        }

        match execute_command(&mut engine, "/listvar -g foo*") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "/set fooA=bar"),
            other => panic!("got {:?}", other),
        }
        match execute_command(&mut engine, "/listvar -x foo*") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "/setenv fooB=exported_val"),
            other => panic!("got {:?}", other),
        }
        match execute_command(&mut engine, "/listvar -s foo*") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "fooA\nfooB"),
            other => panic!("got {:?}", other),
        }
        match execute_command(&mut engine, "/listvar -v foo*") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "bar\nexported_val"),
            other => panic!("got {:?}", other),
        }
        // <name> <value> - both patterns must match.
        match execute_command(&mut engine, "/listvar foo* ba?") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "/set fooA=bar"),
            other => panic!("got {:?}", other),
        }
        match execute_command(&mut engine, "/listvar foo* zzz") {
            TfCommandResult::Success(None) => {}
            other => panic!("expected silent no-match, got {:?}", other),
        }
    }

    /// Job 15b-i: `/help listvar` - "The return value of /listvar is the
    /// number of variables listed" (verified directly against real tf: a
    /// single match leaves %?=1, three matches leave %?=3, no match
    /// leaves %?=0) - `cmd_listvar` used to take `&TfEngine` (not `&mut`)
    /// and so could never set %? at all, leaving stdlib.tf's own "isvar"
    /// macro (`/listvar -msimple -- %*`, no explicit /return) always
    /// reading whatever the PRECEDING command in its body left %? at.
    #[test]
    fn test_listvar_sets_question_mark_to_match_count() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/set fooA=1");
        execute_command(&mut engine, "/set fooB=2");
        execute_command(&mut engine, "/set fooC=3");

        execute_command(&mut engine, "/listvar -msimple -- fooA");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));

        execute_command(&mut engine, "/listvar foo*");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(3));

        execute_command(&mut engine, "/listvar no_such_var_xyz");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));
    }

    /// Job 15b-i: a bare "--" ends option parsing (real tf's own
    /// getopt(3)-style convention, needed so a <name> pattern that itself
    /// starts with "-" can never be misread as more flags) - verified
    /// directly against real tf: "/listvar -msimple -- HOME" matches HOME
    /// (leaving %?=1), not "--" itself as a literal (nonexistent) pattern.
    /// `cmd_listvar`'s option loop used to have no such case, so "--"
    /// consumed only its own first '-' and left "- HOME" as the <name>
    /// pattern (matching nothing).
    #[test]
    fn test_listvar_double_dash_ends_options() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/set myvar=1");
        execute_command(&mut engine, "/listvar -msimple -- myvar");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));
    }

    /// Job 15b-i: real tf's own "/ismacro" (a stdlib.tf macro, `/def -i
    /// ismacro = /test tfclose("o")%; /@list -s -i %{*-@}`) hardcodes
    /// "-i" ahead of whatever the caller passes, so it matches an
    /// INVISIBLE macro even when the caller's own args never mention
    /// invisibility at all - verified directly against real tf (`/def -i
    /// foo = ...` then a bare `/ismacro foo`, no "-i" of its own, leaves
    /// %? nonzero). Clay's native `/ismacro` command used to pass the
    /// caller's args straight through with no such default, silently
    /// excluding every invisible macro - which is how spedwalk.tf's own
    /// invisible `~speedwalk` hook macro (defined with `-ip%{maxpri}`)
    /// went unseen by `/if /ismacro ~speedwalk%; /then ...`.
    #[test]
    fn test_ismacro_finds_invisible_macros_by_default() {
        let mut engine = TfEngine::new();
        // A decoy macro first, so "foo"'s own sequence number is nonzero -
        // otherwise %?'s "found, sequence 0" and "not found" (also 0)
        // outcomes would be indistinguishable.
        engine.add_macro(macros::parse_def("decoy = /echo decoy").unwrap());
        let foo_seq = engine.add_macro(macros::parse_def("-i foo = /echo hi").unwrap());
        let result = builtins::cmd_ismacro(&mut engine, "foo");
        assert!(matches!(result, TfCommandResult::Success(None)), "{:?}", result);
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(foo_seq as i64));

        // An explicit "-I" (only-invisible) from the caller still applies
        // normally - forcing "-i" ahead of it must not prevent that.
        engine.add_macro(macros::parse_def("visible = /echo bye").unwrap());
        builtins::cmd_ismacro(&mut engine, "-I visible");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));
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
        let list_output = super::super::macros::list_macros(&engine, None, false);
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
    fn test_nameless_def_always_creates_a_new_macro() {
        // TF allows /def -t/-b/-B/-h with no name (finding C.9); such macros are
        // addressed only by number, and - unlike named macros - a second nameless
        // /def must never be treated as a redefinition of the first.
        let mut engine = TfEngine::new();

        let r1 = execute_command(&mut engine, r#"/def -t"pat" = /echo a"#);
        assert!(matches!(r1, TfCommandResult::Success(None)), "unexpected result: {:?}", r1);
        let r2 = execute_command(&mut engine, r#"/def -t"pat" = /echo b"#);
        assert!(matches!(r2, TfCommandResult::Success(None)), "unexpected result: {:?}", r2);

        let nameless: Vec<_> = engine.macros.iter().filter(|m| m.name.is_empty()).collect();
        assert_eq!(nameless.len(), 2,
            "each nameless /def must create a distinct macro, never a redefinition: {:?}",
            engine.macros.iter().map(|m| (&m.name, &m.body)).collect::<Vec<_>>());
        // Distinct sequence numbers confirm they're genuinely separate macros.
        assert_ne!(nameless[0].sequence_number, nameless[1].sequence_number);
    }

    #[test]
    fn test_nameless_binding_fires_through_get_binding() {
        // A nameless -B/-b macro has no name to re-invoke by, so cmd_def binds the key
        // directly to the macro's body instead (see cmd_def's keybinding registration
        // comment). Verify the binding round-trips through hooks::get_binding, the same
        // lookup input_handler.rs uses for a pressed key.
        let mut engine = TfEngine::new();
        let result = execute_command(&mut engine, r#"/def -B"F5" = /echo pressed"#);
        assert!(matches!(result, TfCommandResult::Success(None)), "unexpected result: {:?}", result);

        assert_eq!(hooks::get_binding(&engine, "F5"), Some("/echo pressed".to_string()));

        // A named macro's -b/-B binding, unaffected by this change, still binds to the
        // macro's bare name.
        let result = execute_command(&mut engine, r#"/def -B"F6" named = /echo named"#);
        assert!(matches!(result, TfCommandResult::Success(None)), "unexpected result: {:?}", result);
        assert_eq!(hooks::get_binding(&engine, "F6"), Some("named".to_string()));
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

    /// Plan Job 14c: `/shift [n]` shifts by `n` (default 1), clamped to the
    /// argument count rather than erroring (verified directly against real
    /// tf: `/shift 5` with only 3 positional params leaves zero).
    #[test]
    fn test_shift_with_count() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def w2 = /shift 2%; /echo #=%{#} star=%*");
        match execute_command(&mut engine, "/w2 a b c d") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "#=2 star=c d"),
            other => panic!("got {:?}", other),
        }

        execute_command(&mut engine, "/def w5 = /shift 5%; /echo #=%{#} star=%*");
        match execute_command(&mut engine, "/w5 a b c") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "#=0 star="),
            other => panic!("got {:?}", other),
        }
    }

    /// Plan Job 14c: `/break 2` inside a nested `/while` unwinds BOTH loops
    /// (`/help break`: "unconditionally terminates the nearest enclosing
    /// /WHILE loop. If <n> is specified, it will break out of <n> enclosing
    /// /WHILE loops") - it does not terminate the enclosing macro itself,
    /// so a command after the outer loop's /done still runs.
    #[test]
    fn test_break_2_unwinds_two_nested_while_loops() {
        // Verified via variable state (i, j) rather than echoed text: a
        // `/break N` with N > 1 that crosses an aggregation boundary loses
        // any Success text echoed earlier in that same iteration (see the
        // matching arm in `aggregate_results_with_engine`'s own doc comment
        // - the same documented trade-off `/return`/`/result` already have)
        // but variable side effects are unaffected either way, and are a
        // more direct check of exactly how many iterations actually ran.
        //
        // Without i incrementing (never reached: `/break 2` fires from
        // inside the inner loop, before the outer loop's own `/let
        // i=$[i+1]`) and with j incrementing once before the break check
        // trips on its second inner iteration, the expected final state
        // after both loops unwind is i=0, j=1 - proving the outer loop
        // never got past its first iteration either.
        let mut engine = TfEngine::new();
        execute_command(&mut engine,
            "/def bt = /let i=0%; \
             /while (i < 3) \
               /let j=0%; \
               /while (j < 3) \
                 /if (j == 1) /break 2%; /endif%; \
                 /let j=$[j+1]%; \
               /done%; \
               /let i=$[i+1]%; \
             /done%; \
             /echo after-loops i=%{i} j=%{j}"
        );

        match execute_command(&mut engine, "/bt") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "after-loops i=0 j=1",
                    "break 2 should stop both loops on the inner loop's second \
                     iteration, but let the macro body continue past the outer /done");
            }
            other => panic!("got {:?}", other),
        }
    }

    /// Same nested shape, but a bare `/break` (no count) only unwinds the
    /// INNER loop - the outer loop keeps running its remaining iterations.
    #[test]
    fn test_bare_break_unwinds_only_the_inner_loop() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine,
            "/def bt1 = /let i=0%; \
             /while (i < 2) \
               /let j=0%; \
               /while (j < 3) \
                 /if (j == 1) /break%; /endif%; \
                 /echo i=%{i} j=%{j}%; \
                 /let j=$[j+1]%; \
               /done%; \
               /let i=$[i+1]%; \
             /done"
        );

        match execute_command(&mut engine, "/bt1") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "i=0 j=0\ni=1 j=0",
                    "a bare /break should only stop the inner loop each time, \
                     letting the outer loop run both its iterations");
            }
            other => panic!("got {:?}", other),
        }
    }

    // =======================================================================
    // /list and /purge macro-option filters (finding C.4, plan step P1.5,
    // Job 7). Each idiom below is taken directly from real TinyFugue's own
    // stdlib.tf/alias.tf/color.tf ("/@list"/"/@purge" is tested here as plain
    // "list"/"purge" - the "/@" builtin-bypass prefix itself is Job 8).
    // =======================================================================

    #[test]
    fn test_cmd_purge_by_name_leaves_other_macros() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("keep = /echo keep").unwrap());
        engine.add_macro(macros::parse_def("drop = /echo drop").unwrap());

        let result = cmd_purge(&mut engine, "drop");
        assert!(matches!(result, TfCommandResult::Success(None)), "/purge must be silent, matching real TF: {result:?}");
        assert!(engine.macros.iter().any(|m| m.name == "keep"));
        assert!(!engine.macros.iter().any(|m| m.name == "drop"));
    }

    #[test]
    fn test_cmd_purge_glob_pattern_leaves_non_matching() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("a1 = /echo a1").unwrap());
        engine.add_macro(macros::parse_def("a2 = /echo a2").unwrap());
        engine.add_macro(macros::parse_def("keep = /echo keep").unwrap());

        cmd_purge(&mut engine, "-mglob a*");

        assert!(!engine.macros.iter().any(|m| m.name == "a1"));
        assert!(!engine.macros.iter().any(|m| m.name == "a2"));
        assert!(engine.macros.iter().any(|m| m.name == "keep"));
    }

    #[test]
    fn test_cmd_purge_bare_keeps_invisible_macros() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-i secret = /echo hi").unwrap());
        engine.add_macro(macros::parse_def("visible = /echo bye").unwrap());

        cmd_purge(&mut engine, "");

        assert!(engine.macros.iter().any(|m| m.name == "secret"), "bare /purge must not delete invisible macros");
        assert!(!engine.macros.iter().any(|m| m.name == "visible"));
    }

    #[test]
    fn test_field_filter_t_empty_selects_only_triggerless_macros() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def(r#"-t"foo" triggered = /echo t"#).unwrap());
        engine.add_macro(macros::parse_def("plain = /echo p").unwrap());

        match cmd_list(&mut engine, "-mglob -t{}") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("plain"), "expected the trigger-less macro listed: {msg}");
                assert!(!msg.contains("triggered"), "a macro WITH a trigger must not match -t{{}}: {msg}");
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_hook_filter_h0_selects_only_hookless_macros() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-hCONNECT hooked = /echo h").unwrap());
        engine.add_macro(macros::parse_def("plain = /echo p").unwrap());

        match cmd_list(&mut engine, "-mglob -h0") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("plain"));
                assert!(!msg.contains("hooked"));
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_bare_dash_ends_option_parsing_for_dash_prefixed_name() {
        let mut engine = TfEngine::new();
        // No macro is actually named "-weird" here - this only proves "-weird"
        // parses as a <name> pattern (via the "-" end-of-options marker)
        // instead of erroring out as an unrecognised option cluster.
        let result = cmd_list(&mut engine, "-mglob - -weird");
        assert!(matches!(result, TfCommandResult::Success(_)), "expected success, got {result:?}");
    }

    #[test]
    fn test_list_short_invisible_glob_alias_body() {
        // alias.tf: /list -s -i -mglob ~alias_body_*
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-i ~alias_body_greet = /echo hi %1").unwrap());
        engine.add_macro(macros::parse_def("-i ~alias_body_bye = /echo bye").unwrap());
        engine.add_macro(macros::parse_def("other = /echo other").unwrap());

        match cmd_list(&mut engine, "-s -i -mglob ~alias_body_*") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("~alias_body_greet"));
                assert!(msg.contains("~alias_body_bye"));
                assert!(!msg.contains("other"));
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_list_only_invisible_excludes_visible_same_name() {
        // alias.tf: /@list -s -I -mglob x (tested here as plain "list ...")
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("x = /echo visible-x").unwrap());
        match cmd_list(&mut engine, "-s -I -mglob x") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "No macros defined.", "a visible macro must not satisfy -I: {msg}");
            }
            other => panic!("expected success: {other:?}"),
        }

        engine.macros.clear();
        engine.add_macro(macros::parse_def("-i x = /echo invisible-x").unwrap());
        match cmd_list(&mut engine, "-s -I -mglob x") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains('x'), "expected the invisible macro to be listed: {msg}");
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_purge_invisible_regexp_color_on_off() {
        // color.tf: /purge -i -mregexp ^color_(on|off)$
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-i color_on = /echo on").unwrap());
        engine.add_macro(macros::parse_def("-i color_off = /echo off").unwrap());
        engine.add_macro(macros::parse_def("-i color_middle = /echo mid").unwrap());
        engine.add_macro(macros::parse_def("keep = /echo keep").unwrap());

        cmd_purge(&mut engine, r#"-i -mregexp ^color_(on|off)$"#);

        assert!(!engine.macros.iter().any(|m| m.name == "color_on"));
        assert!(!engine.macros.iter().any(|m| m.name == "color_off"));
        assert!(engine.macros.iter().any(|m| m.name == "color_middle"), "the regexp anchors must not over-match");
        assert!(engine.macros.iter().any(|m| m.name == "keep"));
    }

    #[test]
    fn test_purge_glob_alternation_retry_macros() {
        // stdlib.tf retry_off: /purge -mglob {~retry_fail_*|~retry_succ_*}
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("~retry_fail_1 = /echo f").unwrap());
        engine.add_macro(macros::parse_def("~retry_succ_1 = /echo s").unwrap());
        engine.add_macro(macros::parse_def("~retry_other = /echo o").unwrap());

        cmd_purge(&mut engine, "-mglob {~retry_fail_*|~retry_succ_*}");

        assert!(!engine.macros.iter().any(|m| m.name == "~retry_fail_1"));
        assert!(!engine.macros.iter().any(|m| m.name == "~retry_succ_1"));
        assert!(engine.macros.iter().any(|m| m.name == "~retry_other"));
    }

    #[test]
    fn test_purge_only_invisible_alias_call_macros() {
        // alias.tf purgealias: /purge -I ~alias_call_*
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-i ~alias_call_greet = /echo g").unwrap());
        engine.add_macro(macros::parse_def("~alias_call_visible = /echo v").unwrap());
        engine.add_macro(macros::parse_def("keep = /echo k").unwrap());

        cmd_purge(&mut engine, "-I ~alias_call_*");

        assert!(!engine.macros.iter().any(|m| m.name == "~alias_call_greet"));
        assert!(engine.macros.iter().any(|m| m.name == "~alias_call_visible"), "-I must not touch a visible macro even if the name matches");
        assert!(engine.macros.iter().any(|m| m.name == "keep"));
    }

    #[test]
    fn test_list_plain_defs_no_trigger_no_bind_no_hook() {
        // stdlib.tf listdef: /list -mglob -h0 -b{} -t{} ?*
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("plain = /echo p").unwrap());
        engine.add_macro(macros::parse_def(r#"-t"foo" triggered = /echo t"#).unwrap());
        engine.add_macro(macros::parse_def("-b'^A' bound = /echo b").unwrap());
        engine.add_macro(macros::parse_def("-hCONNECT hooked = /echo h").unwrap());

        match cmd_list(&mut engine, "-mglob -h0 -b{} -t{} ?*") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("plain"));
                assert!(!msg.contains("triggered"));
                assert!(!msg.contains("bound"));
                assert!(!msg.contains("hooked"));
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_list_fullhilite_style_filter() {
        // stdlib.tf listfullhilite: /list -mglob -h0 -b{} -t'pat' -aurfdhbBC0
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def(r#"-mglob -aur -t"pat" hilited = /echo h"#).unwrap());
        engine.add_macro(macros::parse_def(r#"-mglob -t"pat" plain_trigger = /echo p"#).unwrap());
        engine.add_macro(macros::parse_def(r#"-mglob -aur -t"other" wrong_pattern = /echo w"#).unwrap());

        match cmd_list(&mut engine, "-mglob -h0 -b{} -t'pat' -aurfdhbBC0") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("hilited"));
                assert!(!msg.contains("plain_trigger"), "no attributes must not satisfy -aurfdhbBC0: {msg}");
                assert!(!msg.contains("wrong_pattern"), "trigger 'other' must not match -t'pat': {msg}");
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_purge_bare_dash_then_wildcard_purgedef() {
        // stdlib.tf purgedef: /purge -mglob -h0 -b{} - ?*
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("plain = /echo p").unwrap());
        engine.add_macro(macros::parse_def("-hCONNECT hooked = /echo h").unwrap());

        cmd_purge(&mut engine, "-mglob -h0 -b{} - ?*");

        assert!(!engine.macros.iter().any(|m| m.name == "plain"), "purgedef must remove an ordinary macro");
        assert!(engine.macros.iter().any(|m| m.name == "hooked"), "a hooked macro must survive -h0");
    }

    #[test]
    fn test_list_hook_event_connect() {
        // stdlib.tf listhook idiom: /list -mglob -h'CONNECT'
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-hCONNECT on_connect = /echo c").unwrap());
        engine.add_macro(macros::parse_def("-hDISCONNECT on_disconnect = /echo d").unwrap());
        engine.add_macro(macros::parse_def("plain = /echo p").unwrap());

        match cmd_list(&mut engine, "-mglob -h'CONNECT'") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("on_connect"));
                assert!(!msg.contains("on_disconnect"));
                assert!(!msg.contains("plain"));
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_list_hook_event_with_pattern_filter() {
        // Job 10: -h"EVENT pattern" now filters on the macro's own hook
        // pattern too, not just the event.
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-h\"SEND greet*\" g = /echo g").unwrap());
        engine.add_macro(macros::parse_def("-h\"SEND bye*\" b = /echo b").unwrap());
        engine.add_macro(macros::parse_def("-hSEND anypat = /echo a").unwrap());

        match cmd_list(&mut engine, "-h\"SEND greet*\"") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("= /echo g"));
                assert!(!msg.contains("= /echo b"));
                assert!(!msg.contains("= /echo a"));
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_unhook_with_pattern_removes_only_exact_match() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-h\"SEND greet*\" g = /echo g").unwrap());
        engine.add_macro(macros::parse_def("-h\"SEND bye*\" b = /echo b").unwrap());

        let result = cmd_unhook(&mut engine, "SEND greet*");
        assert!(matches!(result, TfCommandResult::Success(None)), "got {result:?}");
        assert!(!engine.macros.iter().any(|m| m.name == "g"), "g should be removed");
        assert!(engine.macros.iter().any(|m| m.name == "b"), "b must survive");
    }

    #[test]
    fn test_unhook_without_pattern_removes_all_for_event() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-h\"SEND greet*\" g = /echo g").unwrap());
        engine.add_macro(macros::parse_def("-h\"SEND bye*\" b = /echo b").unwrap());
        engine.add_macro(macros::parse_def("-hCONNECT c = /echo c").unwrap());

        let result = cmd_unhook(&mut engine, "SEND");
        assert!(matches!(result, TfCommandResult::Success(None)), "got {result:?}");
        assert!(!engine.macros.iter().any(|m| m.hook == Some(TfHookEvent::Send)));
        assert!(engine.macros.iter().any(|m| m.name == "c"), "CONNECT hook must survive an /unhook SEND");
    }

    #[test]
    fn test_unhook_pattern_no_match_errors() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-h\"SEND greet*\" g = /echo g").unwrap());

        let result = cmd_unhook(&mut engine, "SEND nomatch*");
        assert!(matches!(result, TfCommandResult::Error(_)), "got {result:?}");
        assert!(engine.macros.iter().any(|m| m.name == "g"), "no match means nothing removed");
    }

    #[test]
    fn test_trigger_h_connect_fires_matching_hook() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-hCONNECT h = /echo connected").unwrap());

        match cmd_trigger(&mut engine, "-hCONNECT somehost") {
            TfCommandResult::Success(Some(msg)) => {
                // CONNECT is a "W"-tagged event (TfHookEvent::is_world_stream_event) -
                // no local-echo of "somehost" under /trigger's simulation, only the
                // macro's own output.
                assert_eq!(msg, "connected");
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_trigger_h_send_echoes_text_then_hook_output() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-h\"SEND greet*\" h = /echo send-hook %*").unwrap());

        match cmd_trigger(&mut engine, "-hSEND greet bob") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "greet bob\nsend-hook greet bob");
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_trigger_h_send_gagged_hook_suppresses_echo() {
        // lib_alias.tf's own idiom: -ag (gag) on the matching SEND hook means
        // its own default-message-echo of the raw text never shows, only the
        // macro's own output (verified against real tf: lib_alias.expected is
        // just "greetings bob", no separate "greet bob" line).
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-ag -h\"SEND greet*\" h = /shift%; /echo greetings %1").unwrap());

        match cmd_trigger(&mut engine, "-hSEND greet bob") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "greetings bob");
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_list_bundled_invisible_and_bind_options() {
        // "/list -ib'^A'": -i and -b'^A' bundled under one leading '-'.
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("-i -b'^A' bound_invisible = /echo a").unwrap());
        engine.add_macro(macros::parse_def("-b'^A' bound_visible = /echo av").unwrap());
        engine.add_macro(macros::parse_def("-b'^B' other_bind = /echo b").unwrap());

        match cmd_list(&mut engine, "-ib'^A'") {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("bound_invisible"));
                assert!(msg.contains("bound_visible"), "-i widens visibility rather than narrowing it: {msg}");
                assert!(!msg.contains("other_bind"));
            }
            other => panic!("expected success: {other:?}"),
        }
    }

    #[test]
    fn test_purge_char_class_hilite_page() {
        // stdlib.tf nohilite_page: /purge -mglob -I ~hilite_page[1-9]
        let mut engine = TfEngine::new();
        for n in 1..=5 {
            engine.add_macro(macros::parse_def(&format!("-i -ah ~hilite_page{} = /echo p{}", n, n)).unwrap());
        }
        engine.add_macro(macros::parse_def("-i ~hilite_page0 = /echo p0").unwrap());
        engine.add_macro(macros::parse_def("-i ~hilite_pageA = /echo pA").unwrap());

        cmd_purge(&mut engine, "-mglob -I ~hilite_page[1-9]");

        for n in 1..=5 {
            let name = format!("~hilite_page{}", n);
            assert!(!engine.macros.iter().any(|m| m.name == name), "{} should be purged", name);
        }
        assert!(engine.macros.iter().any(|m| m.name == "~hilite_page0"), "page0 ('0' is not in [1-9]) must survive");
        assert!(engine.macros.iter().any(|m| m.name == "~hilite_pageA"), "pageA (non-digit) must survive");
    }

    #[test]
    fn test_macro_takes_precedence_over_builtin() {
        // TinyFugue runs a user-defined macro in preference to a builtin of
        // the same name (finding C.6/16) - this is the inverse of Clay's
        // old order, under which a same-named macro could never be reached
        // at all. "tick" is one of Clay's own native stub commands
        // (parser.rs's Tier-5-stubs list; see also lib_tick.tf's xfail
        // history), so it's a real, reachable example of the shadowing
        // finding 16 describes, not just a synthetic one.
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def tick = /echo from-macro");
        match execute_command(&mut engine, "/tick") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "from-macro"),
            other => panic!("expected the macro to win over the builtin stub, got {:?}", other),
        }
    }

    #[test]
    fn test_at_prefix_forces_builtin_over_macro() {
        // "/@name" bypasses a same-named user-defined macro and runs the
        // builtin instead - TinyFugue's escape hatch for the precedence
        // flip above.
        let mut engine = TfEngine::new();
        // The macro's own body reaches the real builtin via "/@echo" -
        // without that escape hatch, plain "/echo" here would recurse into
        // this same macro and hit the recursion guard instead (see
        // test_recursion_guard_stops_self_calling_macro in macros.rs).
        execute_command(&mut engine, "/def echo = /@echo from-macro");
        // Plain "/echo" reaches the macro first (precedence flip).
        match execute_command(&mut engine, "/echo hi") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "from-macro"),
            other => panic!("expected the macro to run, got {:?}", other),
        }
        // "/@echo" bypasses that same macro and reaches the real builtin.
        match execute_command(&mut engine, "/@echo hi") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "hi"),
            other => panic!("expected /@echo to reach the builtin, got {:?}", other),
        }
    }

    #[test]
    fn test_control_flow_keywords_are_never_shadowed_by_a_macro() {
        // Control-flow keywords (/if /elseif /else /endif /while /for /done
        // /break) are TF syntax, not commands - a macro named "if" must
        // never intercept /if. Defining one, then running an ordinary
        // single-line /if, must still take the control-flow path.
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def if = /echo should-not-run");
        match execute_command(&mut engine, "/if (1) /echo real-if%; /endif") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "real-if"),
            other => panic!("expected /if's own control flow to run, got {:?}", other),
        }
    }

    // ---- Job 13: /eval, /trigger, /undefn, /undef, DEF's REDEF message ----

    #[test]
    fn test_eval_expands_vars_and_command_substitution_then_executes() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/set v=7");
        match execute_command(&mut engine, "/eval /echo v=%v") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "v=7"),
            other => panic!("expected Success(Some(\"v=7\")), got {:?}", other),
        }

        match execute_command(&mut engine, "/eval /echo nested=$(/echo $(/echo deep))") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "nested=deep"),
            other => panic!("expected Success(Some(\"nested=deep\")), got {:?}", other),
        }
    }

    #[test]
    fn test_eval_sends_non_command_text_to_the_mud() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/set greeting=hello");
        match execute_command(&mut engine, "/eval %greeting there") {
            TfCommandResult::SendToMud(text) => assert_eq!(text, "hello there"),
            other => panic!("expected SendToMud, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_s0_skips_the_substitution_pass() {
        // "-s0" (real tf's /help eval example, and stdlib's own /runtime) means
        // "don't substitute, just dispatch" - a literal, unexpanded "%v" reaches
        // /echo verbatim.
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/set v=7");
        match execute_command(&mut engine, "/eval -s0 /echo v=%v") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "v=%v"),
            other => panic!("expected the literal, unsubstituted text, got {:?}", other),
        }
    }

    #[test]
    fn test_trigger_matches_real_patterns_with_captures() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, r#"/def -t"hello*" -mglob greet = /echo matched %1"#);
        match execute_command(&mut engine, "/trigger hello world") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "matched hello"),
            other => panic!("expected Success(Some(\"matched hello\")), got {:?}", other),
        }

        execute_command(&mut engine, r#"/def -t"^goodbye (.*)$" -mregexp bye = /echo bye-to=%P1"#);
        match execute_command(&mut engine, "/trigger goodbye world") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "bye-to=world"),
            other => panic!("expected Success(Some(\"bye-to=world\")), got {:?}", other),
        }

        // No pattern matches: silent, not an error (real tf: /trigger returns 0
        // and prints nothing).
        assert!(matches!(
            execute_command(&mut engine, "/trigger this matches nothing at all"),
            TfCommandResult::Success(None)
        ));
    }

    #[test]
    fn test_trigger_honours_one_shot_and_gag() {
        let mut engine = TfEngine::new();
        // One-shot: fires once, then the macro is gone.
        execute_command(&mut engine, r#"/def -1 -t"once" fire_once = /echo fired"#);
        match execute_command(&mut engine, "/trigger once") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "fired"),
            other => panic!("expected the one-shot to fire, got {:?}", other),
        }
        assert!(matches!(
            execute_command(&mut engine, "/trigger once"),
            TfCommandResult::Success(None)
        ));

        // A gagged trigger still executes (and still counts toward %?), but
        // /trigger itself never echoes the raw triggering text either way - see
        // test_trigger_matches_real_patterns_with_captures's third probe. Verify
        // the gag attribute doesn't prevent the macro's own output.
        execute_command(&mut engine, r#"/def -ag -t"quietly" hush = /echo still-runs"#);
        match execute_command(&mut engine, "/trigger quietly") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "still-runs"),
            other => panic!("expected the gagged macro to still run, got {:?}", other),
        }
    }

    #[test]
    fn test_undefn_removes_by_number_and_is_silent() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def a = /echo a");
        let n = engine.get_var("?").unwrap().to_string_value();

        assert!(matches!(
            execute_command(&mut engine, &format!("/undefn {}", n)),
            TfCommandResult::Success(None)
        ));
        assert!(!engine.macros.iter().any(|m| m.name == "a"));
    }

    #[test]
    fn test_undefn_missing_number_reports_diagnostic_and_continues() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def a = /echo a");
        execute_command(&mut engine, "/def b = /echo b");
        let a_num = engine.macros.iter().find(|m| m.name == "a").unwrap().sequence_number;
        let b_num = engine.macros.iter().find(|m| m.name == "b").unwrap().sequence_number;

        match execute_command(&mut engine, &format!("/undefn {} 999999 {}", a_num, b_num)) {
            TfCommandResult::Success(Some(msg)) => {
                assert!(msg.contains("UNDEFN: no macro with number 999999"), "{msg}");
            }
            other => panic!("expected a diagnostic message, got {:?}", other),
        }
        // The valid numbers on either side of the bad one still got removed.
        assert!(!engine.macros.iter().any(|m| m.name == "a" || m.name == "b"));
    }

    #[test]
    fn test_undef_is_silent_on_success_and_diagnoses_a_missing_name() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def a = /echo a");

        assert!(matches!(execute_command(&mut engine, "/undef a"), TfCommandResult::Success(None)));

        match execute_command(&mut engine, "/undef a") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "% UNDEF: Macro \"a\" was not defined.");
            }
            other => panic!("expected a diagnostic message, got {:?}", other),
        }
    }

    /// Plan Job 14c: `/UNDEF <name>...` processes each name independently -
    /// a missing name in the middle doesn't stop the rest from being removed
    /// (verified directly against real tf: `/undef a nosuch b` still removes
    /// both `a` and `b`).
    #[test]
    fn test_undef_multiple_names_processed_independently() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def a = /echo a");
        execute_command(&mut engine, "/def b = /echo b");

        match execute_command(&mut engine, "/undef a nosuch b") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "% UNDEF: Macro \"nosuch\" was not defined.");
            }
            other => panic!("expected a single diagnostic for the missing name, got {:?}", other),
        }
        assert!(!macros::undef_macro(&mut engine, "a"), "a should already be gone");
        assert!(!macros::undef_macro(&mut engine, "b"), "b should already be gone");
    }

    #[test]
    fn test_def_redefinition_prints_redef_message_with_and_without_location() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def a = /echo a");

        // Interactively (no file being loaded): no location prefix.
        match execute_command(&mut engine, "/def a = /echo a2") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "% DEF: Redefined macro a");
            }
            other => panic!("expected the REDEF message, got {:?}", other),
        }

        // Simulate being inside a file load (`builtins::load_lines` maintains
        // these two stacks in lockstep - see `TfEngine::diag_location_prefix`).
        engine.loading_files.push("/tmp/fixture.tf".to_string());
        engine.loading_lines.push(9);
        match execute_command(&mut engine, "/def a = /echo a3") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "% /tmp/fixture.tf, line 9: DEF: Redefined macro a");
            }
            other => panic!("expected the located REDEF message, got {:?}", other),
        }
        engine.loading_files.pop();
        engine.loading_lines.pop();
    }

    #[test]
    fn test_def_redefinition_is_gagged_by_a_redef_hook() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def -i -ag -hREDEF gag_redef");
        execute_command(&mut engine, "/def a = /echo a");

        // The REDEF message is suppressed while the gagging hook is defined.
        assert!(matches!(
            execute_command(&mut engine, "/def a = /echo a2"),
            TfCommandResult::Success(None)
        ));

        // Once the gag hook is gone, the message returns.
        execute_command(&mut engine, "/undef gag_redef");
        match execute_command(&mut engine, "/def a = /echo a3") {
            TfCommandResult::Success(Some(msg)) => assert_eq!(msg, "% DEF: Redefined macro a"),
            other => panic!("expected the REDEF message again, got {:?}", other),
        }
    }

    #[test]
    fn test_def_identical_redefinition_is_completely_silent() {
        // `/help hooks`: "the REDEF hook will be called, unless the new macro is
        // identical to the original" - verified directly against real tf.
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def a = /echo a");
        assert!(matches!(
            execute_command(&mut engine, "/def a = /echo a"),
            TfCommandResult::Success(None)
        ));
    }

    #[test]
    fn test_def_redefinition_errors_when_redef_is_off() {
        let mut engine = TfEngine::new();
        execute_command(&mut engine, "/def a = /echo a");
        execute_command(&mut engine, "/set redef=off");
        match execute_command(&mut engine, "/def a = /echo a2") {
            TfCommandResult::Success(Some(msg)) => {
                assert_eq!(msg, "% DEF: macro a already exists");
            }
            other => panic!("expected the redef=off diagnostic, got {:?}", other),
        }
        // The old body is kept.
        assert_eq!(engine.macros.iter().find(|m| m.name == "a").unwrap().body, "/echo a");
    }

    // ---- Plan Job 14b: /fg, /listsockets, /listworlds option parsing ----

    fn fake_world(name: &str, connected: bool) -> WorldInfoCache {
        WorldInfoCache {
            name: name.to_string(),
            host: format!("{name}.example.com"),
            port: "4000".to_string(),
            is_connected: connected,
            ..Default::default()
        }
    }

    #[test]
    fn test_cmd_fg_cycle_next_and_prev() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![
            fake_world("Alpha", true),
            fake_world("Beta", true),
            fake_world("Gamma", true),
        ];
        engine.current_world = Some("Alpha".to_string());

        assert!(matches!(
            cmd_fg(&engine, "->"),
            TfCommandResult::ClayCommand(ref c) if c == "/worlds Beta"
        ));
        assert!(matches!(
            cmd_fg(&engine, "-<"),
            TfCommandResult::ClayCommand(ref c) if c == "/worlds Gamma"
        ), "cycling back from the first world must wrap to the last");
    }

    #[test]
    fn test_cmd_fg_cycle_by_count() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![
            fake_world("Alpha", true),
            fake_world("Beta", true),
            fake_world("Gamma", true),
            fake_world("Delta", true),
        ];
        engine.current_world = Some("Alpha".to_string());

        assert!(matches!(
            cmd_fg(&engine, "-c2"),
            TfCommandResult::ClayCommand(ref c) if c == "/worlds Gamma"
        ));
        assert!(matches!(
            cmd_fg(&engine, "-c-1"),
            TfCommandResult::ClayCommand(ref c) if c == "/worlds Delta"
        ), "negative -c<N> must move backward, wrapping");
    }

    /// Real tf 5.0 beta 8 quirk, verified directly against the oracle: when more than
    /// one of `-c<N>`/`-<`/`->` appears on one command line, only the LAST one sets the
    /// actual move amount - `/fg -c2 ->` moves only 1 (the trailing `->` overwrites -c2's
    /// pending 2 with 1), but `/fg -> -c2` moves 2.
    #[test]
    fn test_cmd_fg_last_cycle_flag_wins() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![
            fake_world("Alpha", true),
            fake_world("Beta", true),
            fake_world("Gamma", true),
        ];
        engine.current_world = Some("Alpha".to_string());

        assert!(matches!(
            cmd_fg(&engine, "-c2 ->"),
            TfCommandResult::ClayCommand(ref c) if c == "/worlds Beta"
        ), "-c2 ->  must move only 1 (-> is last)");
        assert!(matches!(
            cmd_fg(&engine, "-> -c2"),
            TfCommandResult::ClayCommand(ref c) if c == "/worlds Gamma"
        ), "-> -c2  must move 2 (-c2 is last)");
    }

    #[test]
    fn test_cmd_fg_silent_suppresses_not_found() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![fake_world("Alpha", true)];
        engine.current_world = Some("Alpha".to_string());

        // Without -s: a nonexistent world still bounces (Clay's own convenience lets
        // /worlds decide whether that's an error - cmd_fg only pre-checks under -s).
        assert!(matches!(cmd_fg(&engine, "Nope"), TfCommandResult::ClayCommand(_)));
        // With -s: cmd_fg itself short-circuits to a silent no-op.
        assert!(matches!(cmd_fg(&engine, "-s Nope"), TfCommandResult::Success(None)));
    }

    #[test]
    fn test_cmd_fg_no_fg_flag_is_noop() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![fake_world("Alpha", true)];
        engine.current_world = Some("Alpha".to_string());

        assert!(matches!(cmd_fg(&engine, "-n"), TfCommandResult::Success(None)));
        assert!(matches!(cmd_fg(&engine, "-n SomeWorld"), TfCommandResult::Success(None)),
            "real tf's -n discards any world argument too - see cmd_fg's doc comment");
    }

    #[test]
    fn test_cmd_connections_short_form_lists_connected_names_only() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![
            fake_world("Alpha", true),
            fake_world("Beta", false),
            fake_world("Gamma", true),
        ];

        match cmd_connections(&engine, "-s") {
            TfCommandResult::Success(Some(text)) => {
                assert_eq!(text, "Alpha\nGamma", "short form must list only CONNECTED worlds, one name per line");
            }
            other => panic!("expected Success(Some(...)), got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_connections_short_form_empty_is_silent() {
        let engine = TfEngine::new();
        assert!(matches!(cmd_connections(&engine, "-s"), TfCommandResult::Success(None)));
    }

    #[test]
    fn test_cmd_connections_name_filter_and_sort() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![
            fake_world("Zulu", true),
            fake_world("Alpha", true),
        ];

        match cmd_connections(&engine, "-Sn -s") {
            TfCommandResult::Success(Some(text)) => {
                assert_eq!(text, "Alpha\nZulu", "-Sn must sort by name");
            }
            other => panic!("expected Success(Some(...)), got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_connections_type_and_match_style_flags_dont_corrupt_output() {
        // -T<type> and -m<style> must consume their own attached value instead of being
        // scanned char-by-char (a value happening to contain 's' would otherwise wrongly
        // enable the short form) - regression guard for the bug this job's doc comment
        // describes.
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![fake_world("Alpha", true)];

        match cmd_connections(&engine, "-Tsomething -s") {
            TfCommandResult::Success(Some(text)) => assert_eq!(text, "Alpha"),
            other => panic!("expected short-form output, got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_listworlds_u_m_t_flags_dont_corrupt_output() {
        let mut engine = TfEngine::new();
        engine.world_info_cache = vec![fake_world("Alpha", false)];

        // -Tsomething must not be parsed char-by-char (an 's' in "something" would
        // otherwise wrongly enable -s's short form) - same regression class as
        // cmd_connections above.
        match cmd_listworlds(&engine, "-u -mglob -Tsomething -s") {
            TfCommandResult::Success(Some(text)) => assert_eq!(text, "Alpha"),
            other => panic!("expected short-form output, got {:?}", other),
        }
    }

    #[test]
    fn test_cmd_listworlds_c_format_includes_password() {
        let mut engine = TfEngine::new();
        let mut w = fake_world("Alpha", false);
        w.user = "hero".to_string();
        w.password = "secret".to_string();
        engine.world_info_cache = vec![w];

        match cmd_listworlds(&engine, "-c") {
            TfCommandResult::Success(Some(text)) => {
                assert!(text.contains("hero") && text.contains("secret"),
                    "-c must include character/password (TF: 'including passwords'): {text:?}");
            }
            other => panic!("expected Success(Some(...)), got {:?}", other),
        }
    }

    #[test]
    fn test_dc_dispatch_forwards_args_from_tf_layer() {
        // Job 14b regression guard: cmd_dispatch used to hardcode "/disconnect" with no
        // arguments at all, silently dropping a named-world or -ALL target typed at the
        // console (see finding: `is_tf_command_name` includes "dc"/"disconnect", so every
        // console-typed /dc went through this bounce first).
        let mut engine = TfEngine::new();
        assert!(matches!(
            execute_command(&mut engine, "/dc -ALL"),
            TfCommandResult::ClayCommand(ref c) if c == "/disconnect -ALL"
        ));
        assert!(matches!(
            execute_command(&mut engine, "/dc MyMUD"),
            TfCommandResult::ClayCommand(ref c) if c == "/disconnect MyMUD"
        ));
        assert!(matches!(
            execute_command(&mut engine, "/dc"),
            TfCommandResult::ClayCommand(ref c) if c == "/disconnect"
        ));
    }

    // ========================================================================
    // Job 15: /not (finding 13), and %? fixes for /expr, /escape, /replace, /list
    // ========================================================================

    /// Finding 13: /not must run its argument as a COMMAND and negate whatever %? it
    /// left, not treat the argument as a bare expression (the old behavior made
    /// "/not /test 1" fail outright: "Unexpected token: Slash").
    #[test]
    fn test_cmd_not_runs_command_and_negates_status() {
        let mut engine = TfEngine::new();
        assert!(matches!(execute_command(&mut engine, "/not /test 1"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));

        assert!(matches!(execute_command(&mut engine, "/not /test 0"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));
    }

    #[test]
    fn test_cmd_not_bare_is_a_clear_error() {
        let mut engine = TfEngine::new();
        assert!(matches!(execute_command(&mut engine, "/not"), TfCommandResult::Error(_)));
    }

    #[test]
    fn test_cmd_not_dash_s0_skips_substitution_like_eval() {
        let mut engine = TfEngine::new();
        engine.set_global("x", TfValue::Integer(1));
        // With substitution, "%x" would become "1" - /test 1 succeeds, /not negates to 0.
        assert!(matches!(execute_command(&mut engine, "/not /test %x"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));
    }

    #[test]
    fn test_cmd_expr_sets_status_as_well_as_printing() {
        let mut engine = TfEngine::new();
        match execute_command(&mut engine, "/expr 1+2") {
            TfCommandResult::Success(Some(ref s)) => assert_eq!(s, "3"),
            other => panic!("got {:?}", other),
        }
        assert_eq!(engine.get_var("?").map(|v| v.to_string_value()), Some("3".to_string()));
    }

    #[test]
    fn test_cmd_escape_echoes_and_sets_status_to_same_value() {
        let mut engine = TfEngine::new();
        // Metacharacters "x", string "axbxc" - every 'x' (and any '\') gets a
        // preceding backslash.
        match execute_command(&mut engine, "/escape x axbxc") {
            TfCommandResult::Success(Some(ref s)) => assert_eq!(s, r"a\xb\xc"),
            other => panic!("got {:?}", other),
        }
        assert_eq!(engine.get_var("?").map(|v| v.to_string_value()), Some(r"a\xb\xc".to_string()));
    }

    #[test]
    fn test_cmd_replace_echoes_and_sets_status_to_same_value() {
        let mut engine = TfEngine::new();
        match execute_command(&mut engine, "/replace a o banana") {
            TfCommandResult::Success(Some(ref s)) => assert_eq!(s, "bonono"),
            other => panic!("got {:?}", other),
        }
        assert_eq!(engine.get_var("?").map(|v| v.to_string_value()), Some("bonono".to_string()));
    }

    #[test]
    fn test_cmd_toggle_is_silent_and_does_not_touch_status() {
        let mut engine = TfEngine::new();
        engine.set_global("?", TfValue::Integer(42));
        engine.set_global("flag", TfValue::Integer(0));
        assert!(matches!(execute_command(&mut engine, "/toggle flag"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("flag").and_then(|v| v.to_int()), Some(1));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(42), "/toggle must never touch %?");
    }

    #[test]
    fn test_cmd_list_sets_status_to_last_matching_sequence_number() {
        let mut engine = TfEngine::new();
        engine.add_macro(macros::parse_def("foo = /echo hi").unwrap());
        let bar_seq = engine.add_macro(macros::parse_def("bar = /echo bye").unwrap());
        execute_command(&mut engine, "/list bar");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(bar_seq as i64));

        execute_command(&mut engine, "/list no_such_macro_xyz");
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(0));
    }

    /// Bare /then and /do (outside any /if or /while's own command-form condition) are
    /// clear, immediate errors - never a stuck ControlState (real tf: "unexpected /THEN
    /// in outer block").
    #[test]
    fn test_bare_then_and_do_are_clear_errors_not_a_stuck_state() {
        let mut engine = TfEngine::new();
        assert!(matches!(execute_command(&mut engine, "/then"), TfCommandResult::Error(_)));
        assert!(matches!(execute_command(&mut engine, "/do"), TfCommandResult::Error(_)));
        assert!(matches!(engine.control_state, ControlState::None));
        // The engine must still work normally afterward.
        assert!(matches!(execute_command(&mut engine, "/echo still alive"), TfCommandResult::Success(Some(_))));
    }

    /// Full command-dispatch coverage (not just the bare `cmd_*` functions) for the
    /// punctuation-named `/:` command and `/man`'s delegation to `/help`.
    #[test]
    fn test_colon_and_man_dispatch_end_to_end() {
        let mut engine = TfEngine::new();
        assert!(matches!(execute_command(&mut engine, "/:"), TfCommandResult::Success(None)));
        assert_eq!(engine.get_var("?").and_then(|v| v.to_int()), Some(1));

        match execute_command(&mut engine, "/man limit") {
            TfCommandResult::Success(Some(ref s)) => assert!(s.contains("/limit")),
            other => panic!("got {:?}", other),
        }
    }
}
