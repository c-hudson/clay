//! TinyFugue compatibility layer for Clay MUD client.
//!
//! This module provides TF-style commands using `/` prefix.
//! Commands work alongside existing Clay commands for full coexistence.

pub mod attrs;
pub mod effects;
pub mod special_vars;
pub mod parser;
pub mod variables;
pub mod expressions;
pub mod control_flow;
pub mod macros;
pub mod hooks;
pub mod builtins;
pub mod bridge;
pub mod help;
pub mod status;
pub mod snapshot;
#[cfg(test)]
mod script_tests;

use std::collections::HashMap;
use std::time::{Duration, Instant};
use regex::Regex;

/// `%sub` - how typed lines are processed (see `TfEngine::run_typed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubMode {
    Off,
    On,
    Full,
}

/// `/sub on`'s processing of a typed line (`/help sub`): "%;" and "%\" become newlines,
/// "%%" becomes "%", and "\<n>" (decimal) becomes the character with code n.
pub(crate) fn sub_on_expand(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '%' if i + 1 < chars.len() && (chars[i + 1] == ';' || chars[i + 1] == '\\') => {
                out.push('\n');
                i += 2;
            }
            '%' if i + 1 < chars.len() && chars[i + 1] == '%' => {
                out.push('%');
                i += 2;
            }
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() => {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                let digits: String = chars[i + 1..j].iter().collect();
                match digits.parse::<u32>().ok().and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => out.extend(&chars[i..j]),
                }
                i = j;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Globals the engine or Clay set as a side effect - a command's result (`?`), state
/// mirrored from Clay's own input area (`kbnum`, `insert`), or the current event's data
/// for a GMCP/MSDP hook. They are never written to settings.dat.
pub(crate) const TRANSIENT_GLOBALS: &[&str] = &[
    "?", "kbnum", "insert", "gmcp_package", "gmcp_data", "msdp_var", "msdp_val",
];

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
    /// An enumerated special variable as read in an expression: its number (in
    /// arithmetic and as a truth value) and its name (when printed) - see
    /// `special_vars::expr_value`.
    Enum(i64, String),
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
            TfValue::Enum(_, name) => name.clone(),
        }
    }

    /// Try to convert value to integer
    pub fn to_int(&self) -> Option<i64> {
        match self {
            TfValue::Integer(i) => Some(*i),
            TfValue::Float(f) => Some(*f as i64),
            TfValue::String(s) => s.trim().parse().ok(),
            TfValue::Enum(n, _) => Some(*n),
        }
    }

    /// Try to convert value to float
    pub fn to_float(&self) -> Option<f64> {
        match self {
            TfValue::Float(f) => Some(*f),
            TfValue::Integer(i) => Some(*i as f64),
            TfValue::String(s) => s.trim().parse().ok(),
            TfValue::Enum(n, _) => Some(*n as f64),
        }
    }

    /// TF's numeric reading of a value (`/help expressions`): a string is the number at
    /// its start, after leading blanks - "12abc" is 12, "1.5x" is 1.5, " 7" is 7 - and a
    /// string with no number there ("abc", "on", "") is 0. Verified against real tf.
    pub fn tf_number(&self) -> TfValue {
        match self {
            TfValue::Integer(i) => TfValue::Integer(*i),
            TfValue::Float(f) => TfValue::Float(*f),
            TfValue::String(s) => leading_number(s),
            TfValue::Enum(n, _) => TfValue::Integer(*n),
        }
    }

    /// `tf_number` as an integer (a float is truncated).
    pub fn tf_int(&self) -> i64 {
        match self.tf_number() {
            TfValue::Integer(i) | TfValue::Enum(i, _) => i,
            TfValue::Float(f) => f as i64,
            TfValue::String(_) => 0,
        }
    }

    /// `tf_number` as a float.
    pub fn tf_float(&self) -> f64 {
        match self.tf_number() {
            TfValue::Integer(i) | TfValue::Enum(i, _) => i as f64,
            TfValue::Float(f) => f,
            TfValue::String(_) => 0.0,
        }
    }

    /// Truth value, TF's way: a value is true when its number (`tf_number`) is non-zero,
    /// so a non-numeric string - "abc", "on", "off" - is false (verified against real tf:
    /// `/set y=hello` then `/if (y)` takes the else branch).
    pub fn to_bool(&self) -> bool {
        match self.tf_number() {
            TfValue::Integer(i) | TfValue::Enum(i, _) => i != 0,
            TfValue::Float(f) => f != 0.0,
            TfValue::String(_) => false,
        }
    }
}

/// The number at the start of `s` (after leading blanks), as an integer, or a float when
/// it has a fractional part or exponent; 0 when `s` starts with no number at all.
fn leading_number(s: &str) -> TfValue {
    let t = s.trim_start();
    let bytes = t.as_bytes();
    let mut end = 0;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let int_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    let int_digits = end - int_start;
    let mut is_float = false;
    let mut frac_digits = 0;
    if end < bytes.len() && bytes[end] == b'.' {
        let mut j = end + 1;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        frac_digits = j - (end + 1);
        if int_digits > 0 || frac_digits > 0 {
            is_float = frac_digits > 0;
            end = j;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        return TfValue::Integer(0);
    }
    // An exponent only counts when digits follow it ("1e5", not "1e" or "1ex").
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut j = end + 1;
        if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            is_float = true;
            end = j;
        }
    }
    let num = &t[..end];
    if is_float {
        num.parse::<f64>().map(TfValue::Float).unwrap_or(TfValue::Integer(0))
    } else {
        num.trim_end_matches('.').parse::<i64>().map(TfValue::Integer)
            .or_else(|_| num.parse::<f64>().map(TfValue::Float))
            .unwrap_or(TfValue::Integer(0))
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

/// Line-TYPE filter for recall - which kind of line to return. Decoupled from *which
/// world* to read from (see `RecallWorld` below): before this split, `-w`/`-w<world>`
/// and `-i`/`-l`/`-g` all wrote the same field, so `-i -wmud` couldn't mean "input sent
/// to `mud`" - the second flag silently clobbered the first. Now `-w<world>`/`-w` only
/// ever touch `RecallOptions::world`, and `-i`/`-l`/`-g` only ever touch this field, so
/// any combination of one world selector + one source is expressible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecallSource {
    #[default]
    Server,                // -w / no source flag (default): MUD server output only
    Local,                 // -l (TF/client-generated output only)
    Global,                // -g (server + client + input, all together)
    Input,                 // -i: everything sent to the world as text input - typed
                            // AND sent by triggers/actions/hooks/`/repeat` (see
                            // `App::capture_sent_line`) - combines with `world` below.
}

/// Which world's buffer a recall reads from - independent of `RecallSource` (see its
/// doc comment). `-w` alone selects `Current` explicitly (a no-op vs. the default, kept
/// so `-w -i` round-trips); `-w<name>` selects `Named`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecallWorld {
    #[default]
    Current,
    Named(String),
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
    pub world: RecallWorld,
    pub range: RecallRange,
    pub pattern: Option<String>,
    pub match_style: RecallMatchStyle,
    pub inverse_match: bool,        // -v
    pub quiet: bool,                // -q
    pub show_timestamps: bool,      // -t
    pub timestamp_format: Option<String>,  // -t[format]
    pub show_line_numbers: bool,    // #
    pub show_gagged: bool,          // -a<attrs> containing 'g' (e.g. -ag)
    /// The raw `-a<attrs>` value verbatim (`/help attributes`): 'g' shows gagged lines
    /// (`show_gagged`, above); the display attributes are taken off each line's own
    /// (`OutputLine::tf_attrs`) as it is shown, as TF does - not off attributes inside
    /// the text, which TF leaves alone too.
    pub suppress_attrs: String,
    pub context_before: usize,      // -Bn
    pub context_after: usize,       // -An
    pub archive: bool,              // -D (search disk archive)
}

/// Which command created a `TfProcess` - `/ps -r`/`-q` filter on this
/// (`/help ps`: "-r list /repeats only. -q list /quotes only.").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessKind {
    #[default]
    Repeat,
    Quote,
}

/// A background /repeat or /quote (`/help processes`)
#[derive(Debug)]
pub struct TfProcess {
    pub id: u32,
    /// A /repeat's body; for a /quote, what /ps shows for it (`!"cmd"`, `'"file"`...).
    pub command: String,
    pub interval: Duration,
    /// The interval was given (`-<time>`), not %ptime - /ps shows only a given one.
    pub interval_given: bool,
    pub count: Option<u32>,        // None = infinite ("i")
    pub remaining: Option<u32>,    // Counts down
    pub next_run: Instant,
    pub world: Option<String>,     // -w option
    pub synchronous: bool,         // -S flag
    pub on_prompt: bool,           // -P flag
    pub priority: i32,             // -p option (higher = runs first)
    pub kind: ProcessKind,         // /repeat vs. /quote - /ps -r/-q
    /// A /quote's lines still to go, and what is done with each.
    pub lines: std::collections::VecDeque<String>,
    pub disposition: QuoteDisposition,
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

impl QuoteDisposition {
    /// /ps's D column.
    pub fn letter(self) -> char {
        match self {
            QuoteDisposition::Send => 's',
            QuoteDisposition::Echo => 'e',
            QuoteDisposition::Exec => 'x',
        }
    }
}

/// When a /quote's lines are done (`/help quote`, `/help processes`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QuoteTiming {
    /// `-S`: all of them, before anything after the /quote runs.
    Sync,
    /// One every `interval` (`-<time>`, else %ptime), the first an interval after the
    /// /quote; with no interval, all of them as soon as the /quote is done.
    Every { interval: Duration, given: bool },
    /// `-P`: one each time a prompt arrives.
    Prompt,
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
        timing: QuoteTiming,
        /// When backtick source is /recall, pass opts to caller for execution
        recall_opts: Option<(RecallOptions, String)>,  // (opts, prefix)
        /// Strip ANSI/escape sequences from lines (default true; -A disables)
        strip_ansi: bool,
        /// The process id a background quote runs as (already its %?).
        pid: Option<u32>,
        /// What /ps shows for it.
        label: String,
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
    /// Several effects of different kinds, in the order they happened - what a command
    /// whose body did more than one kind of thing folds to (see `effects`).
    Effects(Vec<effects::TfEffect>),
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
    /// TF's name for the style (`/def -m<name>`, `%matching`).
    pub fn name(&self) -> &'static str {
        match self {
            TfMatchMode::Simple => "simple",
            TfMatchMode::Glob => "glob",
            TfMatchMode::Regexp => "regexp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "simple" => Some(TfMatchMode::Simple),
            "glob" => Some(TfMatchMode::Glob),
            "regexp" | "regex" => Some(TfMatchMode::Regexp),
            _ => None,
        }
    }
}

pub use attrs::TfAttributes;

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
    /// Further events of a `-h"EVENT1|EVENT2 ..."` list (`hook` holds the first) - TF:
    /// "<Event> may be a single event name or a list separated by '|'".
    pub extra_hooks: Vec<TfHookEvent>,
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
    /// `-P[<part>]<attr>[;...]`: attributes for parts of a matched line (implies
    /// regexp matching) - see `PartialSpec`.
    pub partials: Vec<PartialSpec>,
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

impl TfMacro {
    /// The macro is hooked to `event` (its first or any further `-h` event).
    pub fn has_hook(&self, event: TfHookEvent) -> bool {
        self.hook == Some(event) || self.extra_hooks.contains(&event)
    }

    /// Every event the macro is hooked to, in the order given.
    pub fn hook_events(&self) -> Vec<TfHookEvent> {
        self.hook.into_iter().chain(self.extra_hooks.iter().copied()).collect()
    }
}

/// Which part of a matched line a `-P` partial hilite colors (`/help def`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialPart {
    /// "L": the text left of the match.
    Left,
    /// "R": the text right of the match.
    Right,
    /// "0": the whole match; "<n>": the nth parenthesized subexpression.
    Group(usize),
}

/// One `[<part>]<attr>` of a `-P` option.
#[derive(Debug, Clone, PartialEq)]
pub struct PartialSpec {
    pub part: PartialPart,
    pub attrs: TfAttributes,
}

impl PartialSpec {
    /// Parse a whole `-P` argument: `[<part>]<attr>` pairs separated by ';'; a missing
    /// part is 0 (the whole match).
    pub fn parse_list(text: &str) -> Result<Vec<PartialSpec>, String> {
        let mut out = Vec::new();
        for item in text.split(';') {
            if item.is_empty() {
                continue;
            }
            let (part, attrs) = if let Some(rest) = item.strip_prefix('L') {
                (PartialPart::Left, rest)
            } else if let Some(rest) = item.strip_prefix('R') {
                (PartialPart::Right, rest)
            } else {
                let digits = item.chars().take_while(|c| c.is_ascii_digit()).count();
                let n = if digits == 0 { 0 } else { item[..digits].parse::<usize>().map_err(|e| e.to_string())? };
                (PartialPart::Group(n), &item[digits..])
            };
            out.push(PartialSpec { part, attrs: TfAttributes::parse(attrs)? });
        }
        Ok(out)
    }

    /// TF's spelling of a `-P` argument, as `/list` shows it.
    pub fn format_list(specs: &[PartialSpec]) -> String {
        specs.iter().map(|p| {
            let part = match p.part {
                PartialPart::Left => "L".to_string(),
                PartialPart::Right => "R".to_string(),
                PartialPart::Group(n) => n.to_string(),
            };
            format!("{}{}", part, p.attrs.canonical())
        }).collect::<Vec<_>>().join(";")
    }
}

/// Per-world watchdog configuration override
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    pub enabled: bool,
    pub n1: usize,
    pub n2: usize,
}

/// The home directory of system user `user`, for `~user` (see `TfEngine::expand_tilde`).
#[cfg(unix)]
fn user_home_dir(user: &str) -> Option<String> {
    let name = std::ffi::CString::new(user).ok()?;
    // SAFETY: getpwnam_r writes only into `pwd` and `buf`, both sized here and alive for
    // the call; `pw_dir` points into `buf` and is copied out before `buf` is dropped.
    unsafe {
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut buf = vec![0 as libc::c_char; 16 * 1024];
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwnam_r(name.as_ptr(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut found);
        if rc != 0 || found.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(pwd.pw_dir).to_string_lossy().into_owned())
    }
}

#[cfg(not(unix))]
fn user_home_dir(_user: &str) -> Option<String> {
    None
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
    /// The process environment as imported into `global_vars` when the engine was
    /// created (name -> value). An imported variable is never written to settings.dat
    /// while its value is unchanged (see `persistable_globals`): the environment holds
    /// secrets (tokens, SSH agent sockets) and machine-specific paths, and a stale saved
    /// copy would otherwise override the live value on the next start.
    pub env_imported: HashMap<String, String>,
    /// Values the engine seeded itself (the TFLIBDIR default, maxpri, time_format, ...).
    /// Like `env_imported`, never persisted while unchanged.
    pub seed_values: HashMap<String, String>,
    /// One frame of effects per `parser::execute_command` currently running (innermost
    /// last) - see the `effects` module.
    pub effect_frames: Vec<Vec<effects::TfEffect>>,
    /// Effects emitted while no frame was open (a trigger or hook run directly by the App,
    /// `echo()` evaluated outside any command). The App drains it with `take_effects`.
    pub effects: Vec<effects::TfEffect>,
    /// The world each running trigger, hook, timer or client command belongs to
    /// (innermost last) - TF's "current world" for `${world_name}`, `world_info()`,
    /// `is_connected()` and friends. Empty means the foreground world (`current_world`).
    pub context_world: Vec<String>,
    /// tfin for each running `%|` pipe (innermost last): the lines the previous command
    /// wrote, read with `tfread()` (see macros::Pipe).
    pub tfin: Vec<std::collections::VecDeque<String>>,
    /// One entry per running macro body (innermost last): whether `tfclose("o")` closed
    /// tfout for the rest of it. Output emitted while any is closed is discarded - TF:
    /// "tfclose() can be used on the tfout stream (handle "o") within a macro body to
    /// prevent further output from subsequent commands in that macro body".
    pub tfout_closed: Vec<bool>,
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
    /// The 1-based (first, last) lines of the command currently being processed in the
    /// matching entry of `loading_files` (same stack depth - `builtins::load_lines`
    /// pushes/updates/pops this in lockstep with `loading_files`); first < last for a
    /// command continued over several lines. Used by `format_diag` to reproduce real
    /// TF's "% <path>, line <N>: " / "% <path>, lines <A>-<B>: " location prefix on
    /// DEF/UNDEF/UNDEFN diagnostics (finding 25) - empty outside of a file load, which
    /// is exactly when real TF omits the prefix too.
    pub loading_lines: Vec<(usize, usize)>,
    /// How many `/load -q`s are running: TF's `-q` also quiets every load nested in it.
    pub quiet_loads: u32,
    /// Files that already warned, in this load of them, that they hold a readable
    /// password (`/addworld`; TF warns once per file).
    pub password_warned_files: std::collections::HashSet<String>,
    /// How many typed lines are running (`run_typed`): what runs inside one is "in the
    /// foreground" in TF's sense - `/connect` then brings its world to the foreground.
    pub typed_depth: u32,
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
    /// `/limit`/`/unlimit`/`/relimit` (Job 15) - queued for the console drain, which
    /// drives the existing F4 filter popup (`FilterPopup`, main.rs). See
    /// `PendingLimitOp` and `cmd_limit`'s doc comment for why this is console-only
    /// (finding 33 in the TF-parity plan).
    pub pending_limit_op: Option<PendingLimitOp>,
    /// The App's screen (`ScreenInfo`).
    pub screen: ScreenInfo,
    /// TF's status area: its fields, and whether a script has made it its own.
    pub status: status::StatusLayout,
    /// How many of the mail files the App checks have unread mail (`nmail()`).
    pub mail_count: usize,
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

/// A world definition from `/addworld` or `addworld()`, for the App to apply. A field
/// left `None` (or empty) keeps the world's current value - re-running a .tfrc changes
/// nothing it didn't name - and `use_ssl` can only turn SSL on, as in TF.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingWorldOp {
    pub name: String,
    /// TF world type (`-T<type>`, `addworld()`'s 2nd argument).
    pub tf_type: Option<String>,
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    /// The world's macro file (loaded on connect).
    pub file: Option<String>,
    /// `-x` / flag "x": connect with SSL.
    pub use_ssl: bool,
    /// `-e` / flag "e": echo what is sent back as if received.
    pub echo: bool,
    /// `addworld()` is silent on success; `/addworld` says what it added or changed.
    pub quiet: bool,
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
    /// Clay's name for the world's type ("mud", "slack", ...).
    pub world_type: String,
    /// TF's type for the world (`/addworld -T`), "" when untyped.
    pub tf_type: String,
    /// A `/connect <host> <port>` world (listed only by `/listworlds -u`).
    pub is_temporary: bool,
    /// Automatic login is enabled for this world (`world_info(w, "login")`).
    pub login: bool,
    /// The server speaks telnet (it has negotiated something).
    pub telnet: bool,
    /// Typed input is echoed locally - the server hasn't taken echoing over (WILL ECHO).
    pub local_echo: bool,
    /// Output is paused at a more prompt (`morepaused()`), with this many lines waiting
    /// (`moresize()`).
    pub paused: bool,
    pub more_lines: usize,
    /// The world's output is being logged to a file (`nlog()`).
    pub logging: bool,
}

/// The screen the App shows things on, for TF's columns(), lines(), winlines() and
/// limit(); synced with the world info (`App::sync_tf_world_info`). Until then (a bare
/// engine, in tests) it is TF's own 80x24.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenInfo {
    pub columns: u16,
    pub lines: u16,
    /// The output window's lines (lines less the status and input rows).
    pub winlines: u16,
    /// A /limit is in effect.
    pub limit: bool,
}

impl Default for ScreenInfo {
    fn default() -> Self {
        ScreenInfo { columns: 80, lines: 24, winlines: 22, limit: false }
    }
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
            // TF's first process is pid 1 (and a pid of 0 means "failed").
            next_process_id: 1,
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
                engine.env_imported.insert(key.clone(), value.clone());
                engine.set_global(&key, TfValue::String(value));
            }
        }

        // TFLIBDIR default - see `default_tflibdir`.
        if let Some(dir) = default_tflibdir() {
            engine.seed_values.insert("TFLIBDIR".to_string(), dir.clone());
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
        // TF's defaults for its special variables, where seeding them changes nothing
        // Clay does by default (see special_vars::SPECIAL_VARS) - the environment wins.
        for var in special_vars::SPECIAL_VARS {
            if let Some(seed) = var.seed {
                if !engine.env_imported.contains_key(var.name) {
                    let value = match var.kind {
                        special_vars::Kind::Int => TfValue::Integer(seed.parse().unwrap_or(0)),
                        _ => TfValue::String(seed.to_string()),
                    };
                    engine.set_global(var.name, value);
                }
            }
        }
        if let Some(libdir) = engine.get_var("TFLIBDIR").map(|v| v.to_string_value()) {
            for (name, file) in [("TFHELP", "tf-help"), ("TFLIBRARY", "stdlib.tf")] {
                let path = format!("{}/{}", libdir.trim_end_matches('/'), file);
                if !engine.env_imported.contains_key(name) && std::path::Path::new(&path).exists() {
                    engine.set_global(name, TfValue::String(path));
                }
            }
        }
        engine.set_global("pi", TfValue::Float(std::f64::consts::PI));
        engine.set_global("e", TfValue::Float(std::f64::consts::E));
        status::seed(&mut engine);
        let mut seeded: Vec<String> = special_vars::SPECIAL_VARS.iter()
            .filter(|v| v.seed.is_some()).map(|v| v.name.to_string()).collect();
        seeded.extend(["TFHELP", "TFLIBRARY", "pi", "e"].iter().map(|s| s.to_string()));
        seeded.extend(status::seeded_names());
        for name in &seeded {
            if let Some(v) = engine.global_vars.get(name.as_str()) {
                let v = v.to_string_value();
                engine.seed_values.insert(name.clone(), v);
            }
        }
        for name in ["maxpri", "time_format"] {
            if let Some(v) = engine.global_vars.get(name) {
                let v = v.to_string_value();
                engine.seed_values.insert(name.to_string(), v);
            }
        }

        // TFPATH: the $TFPATH environment variable, if set (TF itself
        // leaves it unset by default; when it is set it's a colon-separated
        // search list, same as $PATH).
        if let Ok(tfpath) = std::env::var("TFPATH") {
            if !tfpath.is_empty() {
                engine.set_global("TFPATH", TfValue::String(tfpath));
            }
        }

        // Unit tests: %HOME is the per-process test home, so a test's `/save ~/x`,
        // `/log ~/x` or `/cd` can never reach the developer's real home directory.
        #[cfg(test)]
        {
            let home = crate::get_home_dir();
            engine.seed_values.insert("HOME".to_string(), home.clone());
            engine.set_global("HOME", TfValue::String(home));
        }

        engine
    }

    /// The TF globals worth writing to settings.dat, sorted by name: everything except
    /// values that merely mirror the environment or the engine's own seeds (unchanged),
    /// and transient engine-internal names. Real TF persists no variables at all; Clay
    /// keeps a deliberate `/set` across restarts, but must never copy the environment
    /// (secrets, stale TFLIBDIR/TFPATH) into the settings file.
    pub fn persistable_globals(&self) -> Vec<(&String, &TfValue)> {
        let mut out: Vec<(&String, &TfValue)> = self.global_vars.iter()
            .filter(|(name, value)| {
                if TRANSIENT_GLOBALS.contains(&name.as_str()) {
                    return false;
                }
                let v = value.to_string_value();
                if self.env_imported.get(name.as_str()).is_some_and(|e| *e == v) {
                    return false;
                }
                if self.seed_values.get(name.as_str()).is_some_and(|s| *s == v) {
                    return false;
                }
                true
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(b.0));
        out
    }

    /// The environment a child process (`/sh`, `/quote !`) gets on top of Clay's own: TF
    /// exports every variable that came from the environment or was /setenv'd, with its
    /// current value - or removes it, when it has been /unset.
    pub fn child_env(&self) -> Vec<(String, Option<String>)> {
        let mut names: Vec<&String> = self.env_imported.keys().chain(self.env_vars.iter()).collect();
        names.sort();
        names.dedup();
        names.into_iter()
            .filter(|name| !name.is_empty() && !name.contains('='))
            .map(|name| (name.clone(), self.global_vars.get(name.as_str()).map(|v| v.to_string_value())))
            .collect()
    }

    /// Give `cmd` the environment of `child_env`.
    pub fn apply_child_env(&self, cmd: &mut std::process::Command) {
        for (name, value) in self.child_env() {
            match value {
                Some(value) => { cmd.env(name, value); }
                None => { cmd.env_remove(name); }
            }
        }
    }

    /// The home directory TF commands use for `~` and bare `/cd`: the `%HOME` variable
    /// (TF's own rule - `/help environment`: "HOME ... used by /cd and filename
    /// expansion"), falling back to Clay's own idea of the home directory when it is unset
    /// or empty (Windows usually has no HOME in the environment).
    pub fn home_dir(&self) -> String {
        match self.get_var("HOME").map(|v| v.to_string_value()) {
            Some(h) if !h.is_empty() => h,
            _ => crate::get_home_dir(),
        }
    }

    /// TF's filename expansion (`/help filenames`): a leading `~` up to the first `/` names
    /// a user - empty means `home_dir()` (%HOME), otherwise that user's home directory
    /// (Unix only; an unknown user, or `~user` elsewhere, is left as written).
    pub fn expand_tilde(&self, path: &str) -> String {
        let Some(rest) = path.strip_prefix('~') else {
            return path.to_string();
        };
        let user_end = rest.find(|c: char| c == '/' || (cfg!(windows) && c == '\\')).unwrap_or(rest.len());
        let (user, tail) = rest.split_at(user_end);
        let home = if user.is_empty() { Some(self.home_dir()) } else { user_home_dir(user) };
        match home {
            Some(home) if tail.is_empty() => home,
            Some(home) => format!("{}{}", home.trim_end_matches(['/', '\\']), tail),
            None => path.to_string(),
        }
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
            (Some(path), Some(&(first, last))) if first < last => format!("{}, lines {}-{}: ", path, first, last),
            (Some(path), Some(&(_, line))) => format!("{}, line {}: ", path, line),
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
        self.get_var("insert").map(special_vars::flag_is_on).unwrap_or(true)
    }

    /// Set a global variable
    pub fn set_global(&mut self, name: &str, value: TfValue) {
        self.global_vars.insert(name.to_string(), value);
    }

    /// A user's assignment to a global (`/set`, `/toggle`): a special variable's value
    /// is normalized the way TF does it (see `special_vars::normalize`), or refused with
    /// TF's own message, leaving the old value.
    pub fn assign_global(&mut self, name: &str, value: TfValue) -> Result<(), String> {
        let value = match special_vars::lookup(name) {
            Some(var) => special_vars::normalize(var, &value)?,
            None => value,
        };
        if name.starts_with("status_") {
            if let Some(error) = status::on_assign(self, name, &value.to_string_value()) {
                return Err(error);
            }
        }
        if name == "status_fields" {
            // status::on_assign set row 0; the variable shows what it became.
            return Ok(());
        }
        if special_vars::BOUND.contains(&name) || special_vars::APPLIED.contains(&name) {
            self.emit(effects::TfEffect::Setting(name.to_string(), value.clone()));
        }
        self.set_global(name, value);
        Ok(())
    }

    /// A user's `:=`, `++` or `--`: update the binding wherever it already lives (see
    /// `set_existing_or_global`); a global special variable is normalized as by
    /// `assign_global`.
    pub fn assign_existing_or_global(&mut self, name: &str, value: TfValue) -> Result<(), String> {
        if self.local_vars_stack.iter().any(|scope| scope.contains_key(name)) {
            self.set_existing_or_global(name, value);
            return Ok(());
        }
        self.assign_global(name, value)
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

    /// Run one line and return everything it did, in order. Effects that happened while
    /// no frame was open (from earlier trigger/hook runs) are not included - see
    /// `take_effects`. A top-level `/result` echoes its value, as when typed.
    pub fn run(&mut self, input: &str) -> Vec<effects::TfEffect> {
        let result = parser::execute_command(self, input);
        let mut out = Vec::new();
        match result {
            TfCommandResult::Result(v) if !v.is_empty() => {
                out.push(effects::TfEffect::Output { text: v, attrs: String::new(), world: None, plain: None });
            }
            other => effects::push_result_effects(other, &mut out),
        }
        out
    }

    /// TF's "current world": the world of the trigger, hook, timer or client command
    /// running now, else the foreground world.
    pub fn context_world_name(&self) -> Option<String> {
        self.context_world.last().cloned().or_else(|| self.current_world.clone())
    }

    /// The `-T` types of `world` (or of the current world): TF's own (`/addworld -T`)
    /// and Clay's name for it - None when it isn't known.
    pub fn world_types(&self, world: Option<&str>) -> Option<(String, String)> {
        let name = match world {
            Some(w) => w.to_string(),
            None => self.context_world_name()?,
        };
        self.world_info_cache.iter()
            .find(|w| w.name.eq_ignore_ascii_case(&name))
            .map(|w| (w.tf_type.clone(), w.world_type.clone()))
    }

    /// Run `f` with `world` as TF's current world (see `context_world`).
    pub fn with_context_world<R>(&mut self, world: Option<&str>, f: impl FnOnce(&mut Self) -> R) -> R {
        match world {
            Some(name) => {
                self.context_world.push(name.to_string());
                let result = f(self);
                self.context_world.pop();
                result
            }
            None => f(self),
        }
    }

    /// One field of a world (`world_info()`, `${world_<field>}`): `world` None means the
    /// current world (`context_world_name`). Character, password and macro file fall
    /// back to the DEFAULT world's, as in TF. None when there is no such world.
    pub fn world_field(&self, world: Option<&str>, field: &str) -> Option<String> {
        let name = match world {
            Some(w) => Some(w.to_string()),
            None => self.context_world_name(),
        };
        let found = name.as_ref().and_then(|name| self.world_info_cache.iter().find(|w| w.name.eq_ignore_ascii_case(name)));
        let Some(w) = found else {
            // An explicitly named world that doesn't exist has nothing to say.
            if world.is_some() {
                return None;
            }
            // No current world: only the DEFAULT world's own fields can answer.
            return match field.to_lowercase().as_str() {
                "character" | "char" => self.default_world_character.clone(),
                "password" | "pass" => self.default_world_password.clone(),
                "file" | "mfile" => self.default_world_file.clone(),
                _ => None,
            };
        };
        let non_empty = |s: &str| if s.is_empty() { None } else { Some(s.to_string()) };
        Some(match field.to_lowercase().as_str() {
            "name" => w.name.clone(),
            "host" => w.host.clone(),
            "port" => w.port.clone(),
            "character" | "char" => non_empty(&w.user)
                .or_else(|| self.default_world_character.clone()).unwrap_or_default(),
            "password" | "pass" => non_empty(&w.password)
                .or_else(|| self.default_world_password.clone()).unwrap_or_default(),
            // TF's type when it has one, else Clay's name for it.
            "type" => if w.tf_type.is_empty() { w.world_type.clone() } else { w.tf_type.clone() },
            "login" => if w.login { "1" } else { "0" }.to_string(),
            "proxy" => if w.is_proxy { "1" } else { "0" }.to_string(),
            "ssl" | "secure" => if w.use_ssl { "1" } else { "0" }.to_string(),
            "file" | "mfile" => self.world_files.get(&w.name.to_lowercase()).cloned()
                .or_else(|| self.default_world_file.clone()).unwrap_or_default(),
            _ => String::new(),
        })
    }

    /// `%sub`: how a line the user types is processed (`/help sub`). Unset means off,
    /// TF's default.
    pub fn sub_mode(&self) -> SubMode {
        match self.get_var("sub").map(|v| v.to_string_value().to_lowercase()).as_deref() {
            Some("on") | Some("1") => SubMode::On,
            Some("full") | Some("2") => SubMode::Full,
            _ => SubMode::Off,
        }
    }

    /// Run a line the user typed (or `-c` on the command line), the way TF does by
    /// `%sub` (`/help sub`): off - a command runs exactly as typed, no `%`/`$[]`
    /// substitution; on - `%;` and `%\` split it into lines, `%%` is `%` and `\<n>` the
    /// character with code n; full - it runs as a macro body (full substitution).
    /// Plain text (not a `/command`) is sent. Returns what it did, in order.
    pub fn run_typed(&mut self, line: &str) -> Vec<effects::TfEffect> {
        self.typed_depth += 1;
        let out = self.run_typed_inner(line);
        self.typed_depth -= 1;
        out
    }

    fn run_typed_inner(&mut self, line: &str) -> Vec<effects::TfEffect> {
        match self.sub_mode() {
            SubMode::Full => self.run_body(line),
            SubMode::On => {
                let text = sub_on_expand(line);
                let mut out = Vec::new();
                for piece in text.split('\n') {
                    out.extend(self.run_typed_piece(piece));
                }
                out
            }
            SubMode::Off => self.run_typed_piece(line),
        }
    }

    fn run_typed_piece(&mut self, line: &str) -> Vec<effects::TfEffect> {
        if !line.trim_start().starts_with('/') {
            if line.is_empty() {
                return Vec::new();
            }
            return vec![effects::TfEffect::Send { text: line.to_string(), world: None, no_eol: false }];
        }
        let result = parser::execute_command_substituted(self, line);
        let mut out = Vec::new();
        match result {
            TfCommandResult::Result(v) if !v.is_empty() => {
                out.push(effects::TfEffect::Output { text: v, attrs: String::new(), world: None, plain: None });
            }
            other => effects::push_result_effects(other, &mut out),
        }
        out
    }

    /// Run `body` the way a macro body runs - split on `%;`, each command substituted when
    /// it runs, plain text sent - and return what it did, in order. TF runs a `/repeat`
    /// body this way (`/help repeat`: "<command> may be any legal macro body ... undergoes
    /// macro body substitution when it is executed").
    pub fn run_body(&mut self, body: &str) -> Vec<effects::TfEffect> {
        let body_macro = TfMacro { body: body.to_string(), ..Default::default() };
        self.begin_frame();
        let _ = macros::execute_macro(self, &body_macro, &[], None);
        self.end_frame()
    }

    /// Emit an effect into the innermost open frame, or the top-level queue.
    pub fn emit(&mut self, effect: effects::TfEffect) {
        if matches!(effect, effects::TfEffect::Output { world: None, .. }) && self.tfout_closed.iter().any(|c| *c) {
            return;
        }
        match self.effect_frames.last_mut() {
            Some(frame) => frame.push(effect),
            None => self.effects.push(effect),
        }
    }

    /// Emit a line of tfout text.
    pub fn emit_output(&mut self, text: String) {
        self.emit(effects::TfEffect::Output { text, attrs: String::new(), world: None, plain: None });
    }

    /// Emit a sub-result's effects now, in order. A control result (`/return`, `/result`,
    /// `/exit`, an unwinding `/break`) is handed back instead, for the caller to act on.
    pub fn emit_result(&mut self, result: TfCommandResult) -> Option<TfCommandResult> {
        if result.is_control() {
            return Some(result);
        }
        let mut list = Vec::new();
        effects::push_result_effects(result, &mut list);
        for effect in list {
            if let Some(control) = self.emit_effect(effect) {
                return Some(control);
            }
        }
        None
    }

    /// `emit`, for callers that also handle a control result. (A synchronous `/quote`
    /// runs its lines itself, in place - `builtins::cmd_quote` - so nothing is left to
    /// finish here.)
    pub fn emit_effect(&mut self, effect: effects::TfEffect) -> Option<TfCommandResult> {
        self.emit(effect);
        None
    }

    /// %mecho's prefix for a command run now - %mprefix once per level of macro and file
    /// nesting, as TF repeats it - or None when it isn't echoed: %mecho off, or "on" and
    /// the macro running is invisible (`all` echoes those too).
    pub fn mecho_prefix(&self, visible: bool) -> Option<String> {
        let mode = self.get_var("mecho").map(|v| v.to_string_value().to_lowercase()).unwrap_or_default();
        let on = match mode.as_str() {
            "all" | "2" => true,
            "on" | "1" => visible,
            _ => false,
        };
        if !on {
            return None;
        }
        let depth = self.macro_call_depth as usize + self.loading_lines.len();
        let prefix = self.get_var("mprefix").map(|v| v.to_string_value()).unwrap_or_else(|| "+".to_string());
        Some(prefix.repeat(depth.max(1)))
    }

    /// Run one line exactly as given - no `%` substitution, no `%;` split - as TF runs a
    /// line `/quote` generates for `-dexec`: a command runs, plain text is sent.
    pub fn run_unexpanded(&mut self, line: &str) -> Vec<effects::TfEffect> {
        self.run_typed_piece(line)
    }

    /// Open a frame: effects emitted until the matching `end_frame` are collected there.
    pub fn begin_frame(&mut self) {
        self.effect_frames.push(Vec::new());
    }

    /// Close the innermost frame and return what it collected.
    pub fn end_frame(&mut self) -> Vec<effects::TfEffect> {
        self.effect_frames.pop().unwrap_or_default()
    }

    /// Take the effects emitted while no frame was open.
    pub fn take_effects(&mut self) -> Vec<effects::TfEffect> {
        std::mem::take(&mut self.effects)
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
        // TF: a string with no number at its start is false (and 0).
        assert!(!s.to_bool());
        assert_eq!(s.tf_int(), 0);
        assert!(TfValue::String("12abc".to_string()).to_bool());
        assert_eq!(TfValue::String(" 7x".to_string()).tf_int(), 7);
        assert_eq!(TfValue::String("1.5x".to_string()).tf_float(), 1.5);
        assert_eq!(TfValue::String("-3".to_string()).tf_int(), -3);
        assert_eq!(TfValue::String("1e3".to_string()).tf_float(), 1000.0);
        assert_eq!(TfValue::String("1ex".to_string()).tf_number(), TfValue::Integer(1));
        assert!(!TfValue::String("on".to_string()).to_bool());
        assert!(!TfValue::String("".to_string()).to_bool());

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
