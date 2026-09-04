//! TinyFugue compatibility layer for Clay MUD client.
//!
//! This module provides TF-style commands using `/` prefix.
//! Commands work alongside existing Clay commands for full coexistence.

pub mod parser;
pub mod variables;
pub mod expressions;
pub mod control_flow;
pub mod macros;
pub mod hooks;
pub mod builtins;
pub mod bridge;
#[cfg(test)]
mod script_tests;

use std::collections::HashMap;
use std::time::{Duration, Instant};
use regex::Regex;

/// `$TFLIBDIR` if it names a real directory, else the system TinyFugue
/// library location (Debian's `tf5` package installs it at
/// `/usr/share/tf5/tf-lib`), else `None`. Shared by `TfEngine::new()` (which
/// seeds the engine's own `TFLIBDIR` variable from this) and
/// `script_tests.rs`'s `tf_lib_dir()` (which uses the same resolution to
/// decide whether a `;; requires-lib` fixture can run at all, then sets the
/// engine variable explicitly to whatever it found - which wins over this
/// default since it runs after `TfEngine::new()` returns).
pub(crate) fn default_tflibdir() -> Option<String> {
    if let Ok(dir) = std::env::var("TFLIBDIR") {
        if !dir.is_empty() && std::path::Path::new(&dir).is_dir() {
            return Some(dir);
        }
    }
    const SYSTEM_TFLIBDIR: &str = "/usr/share/tf5/tf-lib";
    if std::path::Path::new(SYSTEM_TFLIBDIR).is_dir() {
        return Some(SYSTEM_TFLIBDIR.to_string());
    }
    None
}

/// Value types for TF variables
#[derive(Debug, Clone, PartialEq)]
pub enum TfValue {
    String(String),
    Integer(i64),
    Float(f64),
}

/// Format a float the way real TF displays a computed "real" value: fixed
/// decimal notation, trailing zeros trimmed, but the decimal point always
/// kept (even for a whole number) so a float never prints identically to an
/// integer. Verified directly against real tf 5.0 beta 8: `pow(2,3)` prints
/// "8.", `sqrt(4)` prints "2.", `6.0/2` prints "3.", `ln(1)` prints "0.",
/// and `ln(2)` prints "0.693147180559945" (15 digits, untouched since none
/// of them are trailing zeros). This is a close approximation of TF's own
/// `%.15g`-ish formatting rather than a byte-exact port of it - real TF
/// also has a separate code path that keeps exactly one trailing zero for
/// some float **literals** combined with `+` (e.g. `3.0 + 0` prints
/// "3.0", not "3."), which no fixture in this test suite depends on, so
/// it isn't reproduced here.
fn format_tf_float(f: f64) -> String {
    if !f.is_finite() {
        return f.to_string();
    }
    let fixed = format!("{:.15}", f);
    let trimmed = fixed.trim_end_matches('0');
    trimmed.to_string()
}

impl TfValue {
    /// Convert value to string representation
    pub fn to_string_value(&self) -> String {
        match self {
            TfValue::String(s) => s.clone(),
            TfValue::Integer(i) => i.to_string(),
            TfValue::Float(f) => format_tf_float(*f),
        }
    }

    /// Try to convert value to integer
    pub fn to_int(&self) -> Option<i64> {
        match self {
            TfValue::Integer(i) => Some(*i),
            TfValue::Float(f) => Some(*f as i64),
            TfValue::String(s) => s.trim().parse().ok(),
        }
    }

    /// Try to convert value to float
    pub fn to_float(&self) -> Option<f64> {
        match self {
            TfValue::Float(f) => Some(*f),
            TfValue::Integer(i) => Some(*i as f64),
            TfValue::String(s) => s.trim().parse().ok(),
        }
    }

    /// Convert to boolean (TF semantics: 0 or empty string is false)
    pub fn to_bool(&self) -> bool {
        match self {
            TfValue::Integer(i) => *i != 0,
            TfValue::Float(f) => *f != 0.0,
            TfValue::String(s) => !s.is_empty() && s != "0",
        }
    }
}

impl Default for TfValue {
    fn default() -> Self {
        TfValue::String(String::new())
    }
}

impl From<&str> for TfValue {
    fn from(s: &str) -> Self {
        // Try to parse as integer first, then float, then keep as string
        if let Ok(i) = s.parse::<i64>() {
            TfValue::Integer(i)
        } else if let Ok(f) = s.parse::<f64>() {
            TfValue::Float(f)
        } else {
            TfValue::String(s.to_string())
        }
    }
}

impl From<String> for TfValue {
    fn from(s: String) -> Self {
        TfValue::from(s.as_str())
    }
}

/// Matching style for recall pattern
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecallMatchStyle {
    Simple,   // Plain text substring matching
    #[default]
    Glob,     // Wildcard matching (* and ?)
    Regexp,   // Regular expression
}

/// History source for recall
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecallSource {
    #[default]
    CurrentWorld,         // -w (default)
    World(String),        // -wworld
    Local,                // -l (TF output only)
    Global,               // -g (all worlds + local)
    Input,                // -i (input history)
}

/// Range specification for recall
#[derive(Debug, Clone, PartialEq)]
#[derive(Default)]
pub enum RecallRange {
    /// /x - last x matching lines
    LastMatching(usize),
    /// x - from last x lines (or time period)
    Last(usize),
    /// x-y - lines from x to y
    Range(usize, usize),
    /// -y - yth previous line
    Previous(usize),
    /// x- - lines after x
    After(usize),
    /// Time-based range (seconds from now)
    TimePeriod(f64),
    /// Time range (start_secs, end_secs from now)
    TimeRange(f64, f64),
    /// All lines (no range specified)
    #[default]
    All,
}


/// Options for the recall command
#[derive(Debug, Clone, Default)]
pub struct RecallOptions {
    pub source: RecallSource,
    pub range: RecallRange,
    pub pattern: Option<String>,
    pub match_style: RecallMatchStyle,
    pub inverse_match: bool,        // -v
    pub quiet: bool,                // -q
    pub show_timestamps: bool,      // -t
    pub timestamp_format: Option<String>,  // -t[format]
    pub show_line_numbers: bool,    // #
    pub show_gagged: bool,          // -a<attrs> containing 'g' (e.g. -ag)
    /// The raw `-a<attrs>` value verbatim (comma-optional attribute letters, `/help
    /// attributes`) - only 'g' (`show_gagged`, above) has a distinct effect in Clay's
    /// recall today; every other letter is accepted (so a script using them doesn't error)
    /// but otherwise a no-op, since Clay's history buffer doesn't track the rest of TF's
    /// per-line display-attribute set. Kept for round-tripping/testability rather than
    /// discarded the moment 'g' is checked.
    pub suppress_attrs: String,
    pub context_before: usize,      // -Bn
    pub context_after: usize,       // -An
    pub archive: bool,              // -D (search disk archive)
}

/// Which command created a `TfProcess` - `/ps -r`/`-q` filter on this
/// (`/help ps`: "-r list /repeats only. -q list /quotes only."). Real TF's
/// own table also has a per-process "D" (disposition) column for quotes,
/// which Clay doesn't track anywhere queryable, so `-r`/`-q` is as far as
/// this job's `/ps` goes toward that grammar (plan Job 14c: "implement what
/// maps onto Clay's TfProcess fields, accept the rest").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessKind {
    #[default]
    Repeat,
    Quote,
}

/// A background repeat process
#[derive(Debug)]
pub struct TfProcess {
    pub id: u32,
    pub command: String,
    pub interval: Duration,
    pub count: Option<u32>,        // None = infinite ("i")
    pub remaining: Option<u32>,    // Counts down
    pub next_run: Instant,
    pub world: Option<String>,     // -w option
    pub synchronous: bool,         // -S flag
    pub on_prompt: bool,           // -P flag
    pub priority: i32,             // -p option (higher = runs first)
    pub kind: ProcessKind,         // /repeat vs. a delayed /quote line - /ps -r/-q
}

/// Disposition for /quote command output
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuoteDisposition {
    /// Send each line to the MUD server (default when no prefix)
    #[default]
    Send,
    /// Echo each line locally
    Echo,
    /// Execute each line as a TF command
    Exec,
}

/// Result of executing a TF command
#[derive(Debug)]
pub enum TfCommandResult {
    /// Command executed successfully with optional output message
    Success(Option<String>),
    /// Command failed with error message
    Error(String),
    /// Command should be sent to the MUD server
    SendToMud(String),
    /// Command maps to a Clay command that should be executed
    ClayCommand(String),
    /// Recall output history with full options
    Recall(RecallOptions),
    /// Register a repeat process for the main loop to tick
    RepeatProcess(TfProcess),
    /// Quote output: multiple lines with disposition
    Quote {
        lines: Vec<String>,
        disposition: QuoteDisposition,
        world: Option<String>,
        delay_secs: f64,  // Delay between lines (0 = immediate)
        /// When backtick source is /recall, pass opts to caller for execution
        recall_opts: Option<(RecallOptions, String)>,  // (opts, prefix)
        /// Strip ANSI/escape sequences from lines (default true; -A disables)
        strip_ansi: bool,
    },
    /// Return from macro execution with optional value for %?
    Return(String),
    /// /result: like Return (stops the macro, sets %?/the call's value), but
    /// when the macro was called as a *command* (not as a `name(args)`
    /// function) it also echoes the value to tfout - see builtins::cmd_result
    /// and macros::execute_macro's handling of `called_as_function`.
    Result(String),
    /// Abort file loading early (/exit during load). Carries the number of
    /// enclosing `/load`s still to abort, TF's own `/exit [n]` count
    /// (default/floor 1 - see `builtins::cmd_exit`): `load_file_internal`
    /// absorbs one level per catch and, while the count is still >1,
    /// re-emits it decremented instead of the usual `Success(None)` so the
    /// next enclosing `/load` keeps aborting too.
    ExitLoad(u32),
    /// Not a TF command (doesn't start with /)
    NotTfCommand,
    /// Unknown TF command
    UnknownCommand(String),
}

/// Hook events that can trigger macros. All 31 of real TF's own events (see
/// `/help hooks` - `tf-help`'s `&hooks` section) plus Clay's own GMCP/MSDP
/// extras (finding C.10 / plan step P1.9). `Bgtrig` is TF's current name for
/// what used to be called `Background` - both strings still parse to it (see
/// `parse`), matching tf-help's own note: "BGTRIG used to be called
/// BACKGROUND, and the old name still works."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TfHookEvent {
    Activity,
    Bamf,
    BgText,
    Bgtrig,
    Confail,
    Conflict,
    Connect,
    Disconnect,
    Iconfail,
    Kill,
    Load,
    Loadfail,
    Log,
    Login,
    Mail,
    More,
    Nomacro,
    Pending,
    Preactivity,
    Process,
    Prompt,
    Proxy,
    Redef,
    Resize,
    Send,
    Shadow,
    Shell,
    Sighup,
    Sigterm,
    Sigusr1,
    Sigusr2,
    World,
    /// Clay-only extras - not real TF events, kept for GMCP/MSDP support.
    Gmcp,
    Msdp,
}

impl TfHookEvent {
    /// Parse hook event from string (case-insensitive, matching every `-h<event>`/
    /// `/hook`/`/unhook`/`/trigger -h` site in real TF's own library - e.g.
    /// `-hsend`, `-hloadfail`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "ACTIVITY" => Some(TfHookEvent::Activity),
            "BAMF" => Some(TfHookEvent::Bamf),
            "BGTEXT" => Some(TfHookEvent::BgText),
            "BGTRIG" | "BACKGROUND" => Some(TfHookEvent::Bgtrig),
            "CONFAIL" => Some(TfHookEvent::Confail),
            "CONFLICT" => Some(TfHookEvent::Conflict),
            "CONNECT" => Some(TfHookEvent::Connect),
            "DISCONNECT" => Some(TfHookEvent::Disconnect),
            "ICONFAIL" => Some(TfHookEvent::Iconfail),
            "KILL" => Some(TfHookEvent::Kill),
            "LOAD" => Some(TfHookEvent::Load),
            "LOADFAIL" => Some(TfHookEvent::Loadfail),
            "LOG" => Some(TfHookEvent::Log),
            "LOGIN" => Some(TfHookEvent::Login),
            "MAIL" => Some(TfHookEvent::Mail),
            "MORE" => Some(TfHookEvent::More),
            "NOMACRO" => Some(TfHookEvent::Nomacro),
            "PENDING" => Some(TfHookEvent::Pending),
            "PREACTIVITY" => Some(TfHookEvent::Preactivity),
            "PROCESS" => Some(TfHookEvent::Process),
            "PROMPT" => Some(TfHookEvent::Prompt),
            "PROXY" => Some(TfHookEvent::Proxy),
            "REDEF" => Some(TfHookEvent::Redef),
            "RESIZE" => Some(TfHookEvent::Resize),
            "SEND" => Some(TfHookEvent::Send),
            "SHADOW" => Some(TfHookEvent::Shadow),
            "SHELL" => Some(TfHookEvent::Shell),
            "SIGHUP" => Some(TfHookEvent::Sighup),
            "SIGTERM" => Some(TfHookEvent::Sigterm),
            "SIGUSR1" => Some(TfHookEvent::Sigusr1),
            "SIGUSR2" => Some(TfHookEvent::Sigusr2),
            "WORLD" => Some(TfHookEvent::World),
            "GMCP" => Some(TfHookEvent::Gmcp),
            "MSDP" => Some(TfHookEvent::Msdp),
            _ => None,
        }
    }

    /// Canonical uppercase event name, e.g. for `/list`/`/hook` display and
    /// `/trigger -h`'s own error messages. `{:?}` already renders every variant's
    /// Rust name in a form `.to_uppercase()` turns back into the exact wire name
    /// (`BgText` -> "BGTEXT", `Sigusr1` -> "SIGUSR1", ...), so this is just that,
    /// named for callers that don't want to spell out the `format!` each time.
    pub fn name(&self) -> String {
        format!("{:?}", self).to_uppercase()
    }

    /// TF's own hook table (see `/help hooks`) tags six events "W": their default
    /// message is displayed on the *world's own* output stream, not the generic
    /// alert/tferr stream everything else uses. Verified directly against real tf
    /// (`tf -n -v -q -f...`): `/trigger -h<event> <text>` never shows `<text>` as
    /// local-echo feedback for one of these (there is no live world to route it to
    /// under `/trigger`'s simulation), but does for every other event - see
    /// `parser::cmd_trigger`'s `-h` branch, the only place this matters (finding
    /// C.10 / plan step P1.9; `PENDING` is included too - empirically it never
    /// echoed either, plausibly for the same "needs a real world" reason its own
    /// first form is also tagged "W").
    pub fn is_world_stream_event(&self) -> bool {
        matches!(
            self,
            TfHookEvent::Bamf
                | TfHookEvent::Confail
                | TfHookEvent::Connect
                | TfHookEvent::Disconnect
                | TfHookEvent::Iconfail
                | TfHookEvent::Pending
                | TfHookEvent::World
        )
    }
}

/// Match mode for trigger patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TfMatchMode {
    /// Literal substring match
    Simple,
    /// Glob-style wildcards (* and ?)
    #[default]
    Glob,
    /// Full regular expression
    Regexp,
}

impl TfMatchMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "simple" => Some(TfMatchMode::Simple),
            "glob" => Some(TfMatchMode::Glob),
            "regexp" | "regex" => Some(TfMatchMode::Regexp),
            _ => None,
        }
    }
}

/// Attributes for macro display/behavior
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TfAttributes {
    pub gag: bool,
    pub norecord: bool,
    pub bold: bool,
    pub underline: bool,
    pub reverse: bool,
    pub flash: bool,
    pub dim: bool,
    pub bell: bool,
    pub hilite: Option<String>,  // Color name or code
}

/// A trigger pattern with optional compiled regex
#[derive(Debug, Clone)]
pub struct TfTrigger {
    pub pattern: String,
    pub match_mode: TfMatchMode,
    pub compiled: Option<Regex>,
}

/// A TF macro definition
#[derive(Debug, Clone, Default)]
pub struct TfMacro {
    pub name: String,
    pub body: String,
    pub trigger: Option<TfTrigger>,
    pub hook: Option<TfHookEvent>,
    /// The `-h"EVENT pattern"` pattern text, if any (`None` for a bare `-hEVENT`,
    /// which TF says matches every occurrence of the event - see `/help hook`'s
    /// "pattern will default to *"). Matched against the firing event's own
    /// argument text the same way a `-t` trigger pattern is matched against a MUD
    /// line - same `-m` style, same unanchored substring search, same capture
    /// groups (`hooks::fire_hook`) - see finding C.10 / plan step P1.9.
    pub hook_pattern: Option<String>,
    pub keybinding: Option<String>,
    pub attributes: TfAttributes,
    pub priority: i32,
    /// A `-p<expr>` whose value wasn't a plain decimal literal (e.g.
    /// stdlib.tf's own `-Fp'maxpri'`), deferred until a caller with engine
    /// access (`cmd_def`/`cmd_edit`) can evaluate it as a TF expression -
    /// `parse_def` itself has no `TfEngine` to look variables up in. Real
    /// tf: "/help def" -p: "the argument to -p may be an expression that
    /// has a numeric value... evaluated only once, when the macro is
    /// defined." Always `None` after `macros::resolve_priority_expr` has
    /// run; never itself compared for macro-redefinition equality (see
    /// `defs_equal_except_body`) since `priority` already reflects its
    /// resolved value by then.
    pub priority_expr: Option<String>,
    pub fall_through: bool,
    pub partial_hilite: bool,   // -P: hilite only the matched portion, not the whole line
    pub one_shot: Option<u32>,  // None = permanent, Some(n) = fire n times
    pub shots_remaining: Option<u32>,
    pub condition: Option<String>,  // Expression to evaluate before firing
    pub probability: Option<f32>,   // 0.0 to 1.0
    pub world: Option<String>,      // Restrict to specific world
    pub sequence_number: u32,       // Sequential definition number (TF-compatible)
    pub invisible: bool,            // -i/-I: hidden from /list, /save, /purge unless forced
    pub quiet: bool,                // -q: doesn't count toward BACKGROUND hook / /trigger return value; SEND hook doesn't suppress the original input
    pub world_type: Option<String>, // -T<type>: restrict trigger/hook matches to worlds of this type (glob/regexp per -m)
}

/// Per-world watchdog configuration override
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    pub enabled: bool,
    pub n1: usize,
    pub n2: usize,
}

/// The TinyFugue scripting engine
#[derive(Debug, Default)]
pub struct TfEngine {
    /// Global variables (set with /set, persisted)
    pub global_vars: HashMap<String, TfValue>,
    /// Stack of local variable scopes (for macro execution)
    pub local_vars_stack: Vec<HashMap<String, TfValue>>,
    /// Environment variables (exported to shell)
    pub env_vars: std::collections::HashSet<String>,
    /// Macro definitions
    pub macros: Vec<TfMacro>,
    /// Compiled regex cache for performance
    pub pattern_cache: HashMap<String, Regex>,
    /// Key bindings (key sequence -> macro name or command)
    pub keybindings: HashMap<String, String>,
    /// Current working directory for /lcd
    pub current_dir: Option<String>,
    /// Current control flow state (for multi-line if/while/for)
    pub control_state: control_flow::ControlState,
    /// Background repeat processes
    pub processes: Vec<TfProcess>,
    /// Next process ID counter
    pub next_process_id: u32,
    /// Next macro sequence number (for TF-compatible numbering)
    pub next_macro_sequence: u32,
    /// Tokens for files already loaded via /loaded//require
    pub loaded_tokens: std::collections::HashSet<String>,
    /// Stack of files currently being loaded (for nested loads)
    pub loading_files: Vec<String>,
    /// 1-based line number currently being processed in the matching entry of
    /// `loading_files` (same stack depth - `builtins::load_lines` pushes/updates/
    /// pops this in lockstep with `loading_files`). Used by `format_diag` to
    /// reproduce real TF's "% <path>, line <N>: " location prefix on DEF/UNDEF/
    /// UNDEFN diagnostics (finding 25) - empty outside of a file load, which is
    /// exactly when real TF omits the prefix too.
    pub loading_lines: Vec<usize>,
    /// Pending world operations (addworld calls from expressions)
    pub pending_world_ops: Vec<PendingWorldOp>,
    /// Regex capture groups from last regmatch() call (%P0-%P9)
    pub regex_captures: Vec<String>,
    /// Open file handles for tfopen/tfclose (handle_id -> TfFileHandle)
    pub open_files: HashMap<i32, TfFileHandle>,
    /// Next file handle ID
    pub next_file_handle: i32,
    /// Current world name (set by main app for fg_world/world_info)
    pub current_world: Option<String>,
    /// Connected worlds list (name, host, port, user, is_connected)
    pub world_info_cache: Vec<WorldInfoCache>,
    /// Snapshot of app.ban_list.get_ban_info() (ip, ban_type, reason), synced
    /// alongside world_info_cache - lets TF's own /ban (cmd_banlist) reproduce
    /// Command::BanList's output for /quote backtick capture, same reasoning
    /// as world_info_cache/cmd_connections.
    pub ban_info_cache: Vec<(String, String, String)>,
    /// Current keyboard buffer state (synced from InputArea)
    pub keyboard_state: KeyboardBufferState,
    /// Pending keyboard operations to be processed by main app
    pub pending_keyboard_ops: Vec<PendingKeyboardOp>,
    /// Pending commands to send (from send() function)
    pub pending_commands: Vec<TfCommand>,
    /// Pending echo outputs (from echo() function)
    pub pending_outputs: Vec<TfOutput>,
    /// Pending substitution (from substitute() function)
    pub pending_substitution: Option<TfSubstitution>,
    /// Watchdog: suppress duplicate lines
    pub watchdog_enabled: bool,
    pub watchdog_n1: usize,  // occurrence threshold (default 2)
    pub watchdog_n2: usize,  // window size (default 5)
    pub watchdog_overrides: HashMap<String, WatchdogConfig>,  // per-world overrides
    /// Watchname: suppress spam from repeated character names
    pub watchname_enabled: bool,
    pub watchname_n1: usize,  // occurrence threshold (default 4)
    pub watchname_n2: usize,  // window size (default 5)
    /// Current nested macro-call depth, incremented/decremented by
    /// `macros::execute_macro` around each call - TF's own `max_recur`
    /// guard (default 100; see `macros::MAX_MACRO_RECURSION`). Distinct
    /// from `local_vars_stack.len()`, which also grows for /for and /while
    /// loop-body scopes that aren't macro calls at all.
    pub macro_call_depth: u32,
    /// `/addworld DEFAULT <char> <pass> [<file>]` fallback character/password
    /// (finding 31 / plan Job 14b): `${world_character}`/`${world_password}`
    /// (`variables.rs`) fall back to these for any world whose own field is
    /// empty, matching real TF's documented DEFAULT-world behavior. Engine
    /// memory only - deliberately NOT a real entry in `world_info_cache` (that
    /// would make a fake "DEFAULT" world show up in /listworlds) and never
    /// persisted to settings.dat.
    pub default_world_character: Option<String>,
    pub default_world_password: Option<String>,
    /// `/addworld ... <name> ... [<file>]`'s per-world script, keyed by world
    /// name lower-cased - read back via `world_info(name, "file")`
    /// (`expressions.rs`). Engine memory only (finding 31): real TF loads this
    /// file automatically on connect, but wiring that up needs the persisted-
    /// settings-field + three-UI work this job explicitly defers.
    pub world_files: HashMap<String, String>,
    /// `/addworld ... DEFAULT ... [<file>]`'s own file - fallback for
    /// `world_info(name, "file")` when `world_files` has no entry for that
    /// world, mirroring the character/password fallback above.
    pub default_world_file: Option<String>,
    /// `/xtitle <text>` (Job 15, finding B) - queued for the console main loop's own
    /// drain (`App::apply_pending_tf_console_ops`, mirroring `pending_keyboard_ops`'
    /// established "engine records, App drains" pattern) to apply via crossterm's
    /// `SetTitle` command. CLAUDE.md forbids printing raw escape sequences into the
    /// output area once the TUI is live - `SetTitle` is queued straight to stdout by
    /// the drain, never through `add_output`/the line buffer. Only the console drain
    /// site consumes this, so a web/GUI/remote-console/daemon client's `/xtitle` is
    /// accepted (sets this field) but never visibly applied - none of those clients
    /// own a terminal tab to rename, so that's not a missing feature, just a no-op
    /// there (see `cmd_xtitle`'s own doc comment).
    pub pending_xtitle: Option<String>,
    /// `/more [on|off|1|0]` (Job 15) - queued for the same console-only drain as
    /// `pending_xtitle`, which actually flips `Settings::more_mode_enabled` and
    /// persists/broadcasts it. See `cmd_more`'s doc comment for why a bare `/more`
    /// is an error (matches real tf) and why only `on` prints a message.
    pub pending_more_mode: Option<bool>,
    /// `/wrap <n>` (Job 15) - queued for the console drain, which applies it to
    /// `Settings::wrapspace` (Clay's own real hang-indent wrap-width setting - see
    /// `cmd_wrap`'s doc comment for why `on`/`off` have no Clay-side equivalent and
    /// only update the TF-visible `%wrap` variable).
    pub pending_wrapspace: Option<u8>,
    /// `/limit`/`/unlimit`/`/relimit` (Job 15) - queued for the console drain, which
    /// drives the existing F4 filter popup (`FilterPopup`, main.rs). See
    /// `PendingLimitOp` and `cmd_limit`'s doc comment for why this is console-only
    /// (finding 33 in the TF-parity plan).
    pub pending_limit_op: Option<PendingLimitOp>,
    /// `/restrict [SHELL|FILE|WORLD]` (Job 15) - TF's own monotonic security ratchet;
    /// see `RestrictLevel` and `cmd_restrict`.
    pub restrict_level: RestrictLevel,
}

/// TF's `/restrict` security levels (`/help restrict`), monotonically increasing -
/// once raised, `cmd_restrict` never lowers it for the lifetime of the engine. Derives
/// `Ord` so call sites just compare `engine.restrict_level >= RestrictLevel::Shell`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum RestrictLevel {
    #[default]
    None,
    /// Disables `/sh`, `/sys`, `/quote !...`.
    Shell,
    /// Implies `Shell`. Disables `/load`, `/require`, `/save`, `/lcd` (and `/cd`, which
    /// wraps it), `/log` (opening/redirecting a log file), `/quote '...'`.
    File,
    /// Implies `File`. Disables `/addworld` and the `/world <host> <port>` /
    /// `/connect <host> <port>` "arbitrary connection" form.
    World,
}

impl RestrictLevel {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "SHELL" => Some(Self::Shell),
            "FILE" => Some(Self::File),
            "WORLD" => Some(Self::World),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Shell => "shell",
            Self::File => "file",
            Self::World => "world",
        }
    }
}

/// A queued `/limit`/`/unlimit`/`/relimit` request (Job 15) - the TF engine has no
/// access to `App`/`FilterPopup`, so this only records *what* was asked for;
/// `App::apply_pending_tf_console_ops` (main.rs) does the actual work. Console-only
/// by construction - see `cmd_limit`'s doc comment and finding 33.
#[derive(Debug, Clone)]
pub enum PendingLimitOp {
    /// Bare `/limit` (no options, no pattern): report whether a limit is active.
    /// Real tf answers this silently via `%?`; Clay prints a short status line
    /// instead (documented deviation - see `cmd_limit`).
    Report,
    /// `/unlimit`: clear any active limit.
    Clear,
    /// `/relimit`: re-apply the most recently applied `/limit`.
    Reapply,
    /// `/limit [-v] [-a] [-m<style>] [<pattern>]` with at least one option or a
    /// pattern: apply a new limit.
    Apply {
        pattern: Option<String>,
        invert: bool,
        attrs_only: bool,
        style: TfMatchMode,
    },
}

/// A pending world operation to be processed by the main app
#[derive(Debug, Clone)]
pub struct PendingWorldOp {
    pub name: String,
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub use_ssl: bool,
}

/// Cached world info for TF functions (fg_world, world_info, nactive) and for
/// TF's own /connections /listsockets /l (see cmd_connections in parser.rs,
/// which needs the extra fields below to reproduce Command::WorldsList's
/// output — see commands.rs — for /quote backtick capture).
#[derive(Debug, Clone, Default)]
pub struct WorldInfoCache {
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub is_connected: bool,
    pub use_ssl: bool,
    pub is_proxy: bool,
    pub unseen_lines: usize,
    pub last_receive_secs_ago: Option<i64>,
    pub last_send_secs_ago: Option<i64>,
    pub last_nop_secs_ago: Option<i64>,
    pub next_nop_secs: Option<u64>,
    pub buffer_size: usize,
    /// Distinct from last_send_secs_ago (which tracks ANY outbound send,
    /// including keepalives, for idle()/sidle()): this is specifically the
    /// last time the *user* sent something, matching Command::WorldsList's
    /// "Last" column (commands.rs uses world.last_user_command_time there).
    pub last_user_command_secs_ago: Option<i64>,
}

/// Cached keyboard buffer state for TF functions (kbhead, kbtail, etc.)
#[derive(Debug, Clone, Default)]
pub struct KeyboardBufferState {
    pub buffer: String,
    pub cursor_position: usize,
}

/// Pending keyboard operation to be processed by the main app
#[derive(Debug, Clone)]
pub enum PendingKeyboardOp {
    /// Move cursor to absolute position
    Goto(usize),
    /// Delete count characters at cursor (negative = before cursor)
    Delete(i32),
    /// Move cursor left by word
    WordLeft,
    /// Move cursor right by word
    WordRight,
    /// Insert text at cursor. The `bool` is TF's `%insert` at the moment this op was
    /// *pushed* (captured by the `input()` function/`/input`/`/grab`, not read again at
    /// drain time) - kbfunc.tf's `kb_capitalize_word`/`kb_downcase_word`/`kb_upcase_word`/
    /// `kb_transpose_chars` all temporarily `/set insert=0` around their own `input()`
    /// calls and restore it before the macro returns, so by the time
    /// `App::process_pending_keyboard_ops` drains the queue the engine's live `insert`
    /// variable is already back to its original value - only the captured snapshot still
    /// knows the op itself was meant to overwrite (TF-parity plan Job 20/P2.4).
    Insert(String, bool),
    /// A `/dokey` name that needs real App/World state (input history, scrollback,
    /// the world list, ...) beyond the cached `KeyboardBufferState` `cmd_dokey` can see -
    /// see `App::process_pending_keyboard_ops` / `App::perform_dokey`. The names that only
    /// need the cached buffer (BSPC, DLINE, LEFT, RIGHT, HOME, END, DCH, WLEFT, WRIGHT) are
    /// handled synchronously by `cmd_dokey` via the ops above instead.
    Dokey(DokeyName),
}

/// `/dokey` names routed through `PendingKeyboardOp::Dokey` (see its doc comment). One
/// variant per distinct *behavior* - TF spells several of these more than one way
/// (`PAGEBACK`/`PGUP`, `PAGE`/`PGDN`, `REDRAW`/`REFRESH`), and `cmd_dokey` maps every
/// spelling onto the same variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DokeyName {
    /// BWORD: delete the word before the cursor (space-delimited).
    BackwardWord,
    /// DWORD: delete the word after the cursor.
    ForwardWord,
    /// DEOL: delete from the cursor to the end of the line.
    KillToEol,
    /// UP: move the cursor up one line within a multi-line input (no history fallback -
    /// that's the *key's* job, not `/dokey UP`'s; see finding A in the TF-parity plan).
    CursorUp,
    /// DOWN: move the cursor down one line within a multi-line input.
    CursorDown,
    /// NEWLINE: submit the input line, exactly as pressing Enter does.
    Newline,
    /// RECALLB: recall the previous history entry.
    HistoryPrev,
    /// RECALLF: recall the next history entry.
    HistoryNext,
    /// RECALLBEG: recall the first (oldest) history entry.
    HistoryBegin,
    /// RECALLEND: recall the last (most recent) history entry.
    HistoryEnd,
    /// SEARCHB: search history backward for the current prefix.
    HistorySearchBack,
    /// SEARCHF: search history forward for the current prefix.
    HistorySearchForward,
    /// SOCKETB: switch to the previous world.
    WorldPrev,
    /// SOCKETF: switch to the next world.
    WorldNext,
    /// REDRAW/REFRESH: repaint the screen.
    Redraw,
    /// CLEAR: clear the output view (scrollback refills it on the next repaint).
    ClearView,
    /// PAUSE: pause output (more-mode) on the current world.
    Pause,
    /// LNEXT: treat the next key literally, ignoring any binding.
    LiteralNext,
    /// PAGE/PGDN: scroll one page forward ("more").
    PageForward,
    /// PAGEBACK/PGUP: scroll one page backward ("more").
    PageBackward,
    /// HPAGE: scroll half a page forward ("more").
    HalfPageForward,
    /// HPAGEBACK: scroll half a page backward ("more").
    HalfPageBackward,
    /// LINE: scroll forward one line ("more").
    LineForward,
    /// LINEBACK: scroll backward one line ("more").
    LineBackward,
    /// FLUSH: jump to the end of the scroll buffer, releasing all pending output.
    Flush,
    /// SELFLUSH: show highlighted pending lines and jump to the end of the buffer.
    SelectiveFlush,
}

/// A pending command to send to a world (from send() function)
#[derive(Debug, Clone)]
pub struct TfCommand {
    pub command: String,
    pub world: Option<String>,
    pub no_eol: bool,
}

/// A pending echo output (from echo() function)
#[derive(Debug, Clone)]
pub struct TfOutput {
    pub text: String,
    pub attrs: String,
    pub world: Option<String>,
}

/// A pending substitution (from substitute() function)
#[derive(Debug, Clone)]
pub struct TfSubstitution {
    pub text: String,
    pub attrs: String,
}

/// File handle mode for TF file I/O
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TfFileMode {
    Read,
    Write,
    Append,
}

/// Open file handle for TF file I/O
#[derive(Debug)]
pub struct TfFileHandle {
    pub path: String,
    pub mode: TfFileMode,
    pub read_position: u64,  // For read mode: current position in file
    pub file: Option<std::fs::File>,  // Keep file handle open
}

impl TfEngine {
    pub fn new() -> Self {
        let mut engine = TfEngine {
            watchdog_n1: 2,
            watchdog_n2: 5,
            watchname_n1: 4,
            watchname_n2: 5,
            ..Default::default()
        };

        // Real TF imports the WHOLE process environment as TF global
        // variables at startup - not just the handful with special meaning
        // to TF itself (HOME, SHELL, TERM, ... - `/help environment`'s own
        // "usually inherited from the environment when TF starts" wording
        // undersells it: verified directly against real tf that an
        // arbitrary, TF-meaningless env var like MY_CUSTOM_TEST_VAR is
        // ALSO a live TF variable afterward). This is what stdlib.tf's own
        // "isvar" macro (`/def -i isvar = /test tfclose("o")%; /listvar
        // -msimple -- %*`) depends on for `isvar("HOME")` - without this,
        // /listvar finds nothing and it always reports 0. Skip a name that
        // isn't a valid TF variable identifier (leading digit, or any
        // character besides letters/digits/underscore) rather than crash
        // or corrupt lookups on the rare env var real TF's own C `getenv`
        // loop would have choked on too; TFLIBDIR/TFPATH/maxpri/
        // time_format/redef below still get their own specific handling
        // (defaults, non-env-derived values) and always take precedence
        // over whatever this loop just set.
        for (key, value) in std::env::vars() {
            let mut chars = key.chars();
            let valid = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
            if valid {
                engine.set_global(&key, TfValue::String(value));
            }
        }

        // TFLIBDIR default - see `default_tflibdir`.
        if let Some(dir) = default_tflibdir() {
            engine.set_global("TFLIBDIR", TfValue::String(dir));
        }

        // stdlib.tf line 64 sets this at load time; kbfunc.tf's `-ip%maxpri` (and anything
        // else that relies on stdlib having run) needs it even when stdlib itself hasn't
        // been loaded (finding 27 / plan Job 12).
        engine.set_global("maxpri", TfValue::Integer(2147483647));

        // Predefined variable defaults (`/help time_format`, `/help redef`) - seeded
        // unconditionally, same reasoning as `maxpri` above, so a script that reads
        // `%time_format`/`%redef` before ever `/set`ting them sees real TF's own
        // out-of-the-box value instead of an empty string (finding B's `/time` ruling
        // and finding 25's `redef=off` ruling both depend on these).
        engine.set_global("time_format", TfValue::String("%H:%M".to_string()));
        engine.set_global("redef", TfValue::Integer(1));

        // TFPATH: the $TFPATH environment variable, if set (TF itself
        // leaves it unset by default; when it is set it's a colon-separated
        // search list, same as $PATH).
        if let Ok(tfpath) = std::env::var("TFPATH") {
            if !tfpath.is_empty() {
                engine.set_global("TFPATH", TfValue::String(tfpath));
            }
        }

        engine
    }

    /// Real TF's "% [<path>, line <N>: ]" location prefix for a DEF/UNDEF/UNDEFN-style
    /// diagnostic (finding 25): the path and line of whichever file is currently being
    /// loaded (`loading_files`/`loading_lines`, maintained by `builtins::load_lines`),
    /// or nothing at all when the diagnostic happens outside of a file load (typed
    /// interactively, or from a macro body run interactively) - verified directly
    /// against real tf 5.0 beta 8: `% /abs/path, line 3: DEF: Redefined macro a` while
    /// loading a file, vs. plain `% DEF: Redefined macro a` typed at the prompt.
    pub fn diag_location_prefix(&self) -> String {
        match (self.loading_files.last(), self.loading_lines.last()) {
            (Some(path), Some(line)) => format!("{}, line {}: ", path, line),
            _ => String::new(),
        }
    }

    /// Format a `"% ..."` diagnostic message, TF's own style for informational
    /// command output that isn't really an error (DEF's REDEF notice, UNDEF/UNDEFN's
    /// "was not defined" messages - finding 25) - `msg` is the category-tagged text
    /// after the location prefix, e.g. `"DEF: Redefined macro a"`.
    pub fn format_diag(&self, msg: &str) -> String {
        format!("% {}{}", self.diag_location_prefix(), msg)
    }

    /// Whether `/def`'s redefinition of an existing named macro is currently allowed.
    /// Real TF's `redef` flag (`/help redef`: "Allows redefinition of existing worlds,
    /// keybindings, and named macros", default on) - when a script turns it off,
    /// redefining an existing macro is a hard error instead (verified directly:
    /// `% <path>, line N: DEF: macro a already exists`, and the OLD definition is
    /// kept). Any value other than a literal "off"/"0" counts as on, matching how
    /// this codebase already treats other on/off-worded TF flags (see e.g.
    /// `expressions.rs`'s `send()` "no_eol" argument) rather than `TfValue::to_bool`,
    /// which would misread the *string* "off" as truthy.
    pub fn redef_enabled(&self) -> bool {
        match self.get_var("redef") {
            None => true,
            Some(v) => {
                let s = v.to_string_value();
                !(s.eq_ignore_ascii_case("off") || s == "0")
            }
        }
    }

    /// Get a variable value, checking local scope first, then global
    pub fn get_var(&self, name: &str) -> Option<&TfValue> {
        // Check local scopes from innermost to outermost
        for scope in self.local_vars_stack.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Some(val);
            }
        }
        // Fall back to global
        self.global_vars.get(name)
    }

    /// Current `%insert` mode (TF-parity plan Job 20/P2.4): `true` (TF's default) unless
    /// the variable is set and falsy. Read by `input()`/`cmd_input`/`cmd_grab` at the
    /// moment they queue a `PendingKeyboardOp::Insert`, since a `kb_*` macro's own
    /// temporary `/set insert=0` ... `/set insert=<old>` bracket has usually already
    /// restored the variable by the time the op is drained - see that variant's doc
    /// comment.
    pub fn insert_mode(&self) -> bool {
        self.get_var("insert").map(|v| v.to_bool()).unwrap_or(true)
    }

    /// Set a global variable
    pub fn set_global(&mut self, name: &str, value: TfValue) {
        self.global_vars.insert(name.to_string(), value);
    }

    /// Unset a global variable
    pub fn unset_global(&mut self, name: &str) -> bool {
        self.global_vars.remove(name).is_some()
    }

    /// Set a local variable in the current scope
    pub fn set_local(&mut self, name: &str, value: TfValue) {
        if let Some(scope) = self.local_vars_stack.last_mut() {
            scope.insert(name.to_string(), value);
        } else {
            // No local scope, treat as global
            self.set_global(name, value);
        }
    }

    /// Assign to a variable following TF's `:=` (and `++`/`--`/`+=`) rule
    /// (finding 20): update the binding wherever it already lives - the
    /// innermost local scope that has it, else the global table if it's
    /// bound there - and only create a *new* binding, at the GLOBAL level,
    /// when the name isn't bound anywhere yet. This is deliberately
    /// different from `/let`, which always creates/updates the current
    /// local scope (or global, if there is no local scope at all) and never
    /// looks further out - see `set_local` above. Without this distinction,
    /// an assignment inside a macro (e.g. stack-q.tf's
    /// `/push`: `%{2-stack} := strcat(...)`) would write into the macro's
    /// own scope and vanish the instant the macro returns, even though the
    /// variable was actually a pre-existing global.
    pub fn set_existing_or_global(&mut self, name: &str, value: TfValue) {
        for scope in self.local_vars_stack.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return;
            }
        }
        // Not bound in any local scope - update it if it's already a
        // global, or create it there if it isn't bound anywhere at all.
        self.set_global(name, value);
    }

    /// Push a new local variable scope (for macro execution)
    pub fn push_scope(&mut self) {
        self.local_vars_stack.push(HashMap::new());
    }

    /// Pop the current local variable scope
    pub fn pop_scope(&mut self) {
        self.local_vars_stack.pop();
    }

    /// Execute a TF command (starting with #)
    pub fn execute(&mut self, input: &str) -> TfCommandResult {
        parser::execute_command(self, input)
    }

    /// Perform variable substitution on a string
    /// Handles %{varname}, %varname, and {varname} in expressions
    pub fn substitute_vars(&self, text: &str) -> String {
        variables::substitute_variables(self, text)
    }

    /// Add a macro with an assigned sequence number
    pub fn add_macro(&mut self, mut macro_def: TfMacro) -> u32 {
        let seq = self.next_macro_sequence;
        self.next_macro_sequence += 1;
        macro_def.sequence_number = seq;
        self.macros.push(macro_def);
        seq
    }

    /// Replace an existing macro at the given index, preserving its sequence number
    pub fn replace_macro(&mut self, idx: usize, mut macro_def: TfMacro) {
        // Preserve the original sequence number when redefining
        macro_def.sequence_number = self.macros[idx].sequence_number;
        self.macros[idx] = macro_def;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tf_value_conversions() {
        let s = TfValue::String("hello".to_string());
        assert_eq!(s.to_string_value(), "hello");
        assert_eq!(s.to_int(), None);
        assert!(s.to_bool());

        let i = TfValue::Integer(42);
        assert_eq!(i.to_string_value(), "42");
        assert_eq!(i.to_int(), Some(42));
        assert!(i.to_bool());

        let zero = TfValue::Integer(0);
        assert!(!zero.to_bool());

        let f = TfValue::Float(3.25);
        assert_eq!(f.to_int(), Some(3));
        assert!((f.to_float().unwrap() - 3.25).abs() < 0.001);
    }

    #[test]
    fn test_tf_value_from_str() {
        assert_eq!(TfValue::from("42"), TfValue::Integer(42));
        assert_eq!(TfValue::from("-5"), TfValue::Integer(-5));
        assert!(matches!(TfValue::from("3.25"), TfValue::Float(_)));
        assert_eq!(TfValue::from("hello"), TfValue::String("hello".to_string()));
    }

    #[test]
    fn test_engine_variables() {
        let mut engine = TfEngine::new();

        // Global variable
        engine.set_global("foo", TfValue::String("bar".to_string()));
        assert_eq!(engine.get_var("foo").map(|v| v.to_string_value()), Some("bar".to_string()));

        // Local scope shadows global
        engine.push_scope();
        engine.set_local("foo", TfValue::String("local_bar".to_string()));
        assert_eq!(engine.get_var("foo").map(|v| v.to_string_value()), Some("local_bar".to_string()));

        // Pop scope reveals global again
        engine.pop_scope();
        assert_eq!(engine.get_var("foo").map(|v| v.to_string_value()), Some("bar".to_string()));
    }
}
