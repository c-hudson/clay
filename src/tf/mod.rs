//! TinyFugue compatibility layer for Clay MUD client.
//!
//! This module provides TF-style commands using `#` prefix instead of `/`.
//! Commands work alongside existing Clay commands for full coexistence.

pub mod parser;
pub mod variables;
pub mod expressions;
pub mod control_flow;
pub mod macros;
pub mod hooks;
pub mod builtins;
pub mod bridge;

use std::collections::HashMap;
use std::time::{Duration, Instant};
use regex::Regex;

/// Value types for TF variables
#[derive(Debug, Clone, PartialEq)]
pub enum TfValue {
    String(String),
    Integer(i64),
    Float(f64),
}

impl TfValue {
    /// Convert value to string representation
    pub fn to_string_value(&self) -> String {
        match self {
            TfValue::String(s) => s.clone(),
            TfValue::Integer(i) => i.to_string(),
            TfValue::Float(f) => f.to_string(),
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
    pub show_gagged: bool,          // -ag
    pub context_before: usize,      // -Bn
    pub context_after: usize,       // -An
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
}

/// Disposition for #quote command output
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
    },
    /// Abort file loading early (#exit during load)
    ExitLoad,
    /// Not a TF command (doesn't start with #)
    NotTfCommand,
    /// Unknown TF command
    UnknownCommand(String),
}

/// Hook events that can trigger macros
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TfHookEvent {
    Connect,
    Disconnect,
    Login,
    Prompt,
    Send,
    Activity,
    World,
    Resize,
    Load,
    Redef,
    Background,
    Gmcp,
    Msdp,
}

impl TfHookEvent {
    /// Parse hook event from string
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "CONNECT" => Some(TfHookEvent::Connect),
            "DISCONNECT" => Some(TfHookEvent::Disconnect),
            "LOGIN" => Some(TfHookEvent::Login),
            "PROMPT" => Some(TfHookEvent::Prompt),
            "SEND" => Some(TfHookEvent::Send),
            "ACTIVITY" => Some(TfHookEvent::Activity),
            "WORLD" => Some(TfHookEvent::World),
            "RESIZE" => Some(TfHookEvent::Resize),
            "LOAD" => Some(TfHookEvent::Load),
            "REDEF" => Some(TfHookEvent::Redef),
            "BACKGROUND" => Some(TfHookEvent::Background),
            "GMCP" => Some(TfHookEvent::Gmcp),
            "MSDP" => Some(TfHookEvent::Msdp),
            _ => None,
        }
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
#[derive(Debug, Clone, Default)]
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
    pub keybinding: Option<String>,
    pub attributes: TfAttributes,
    pub priority: i32,
    pub fall_through: bool,
    pub one_shot: Option<u32>,  // None = permanent, Some(n) = fire n times
    pub shots_remaining: Option<u32>,
    pub condition: Option<String>,  // Expression to evaluate before firing
    pub probability: Option<f32>,   // 0.0 to 1.0
    pub world: Option<String>,      // Restrict to specific world
    pub sequence_number: u32,       // Sequential definition number (TF-compatible)
}

/// The TinyFugue scripting engine
#[derive(Debug, Default)]
pub struct TfEngine {
    /// Global variables (set with #set, persisted)
    pub global_vars: HashMap<String, TfValue>,
    /// Stack of local variable scopes (for macro execution)
    pub local_vars_stack: Vec<HashMap<String, TfValue>>,
    /// Environment variables (exported to shell)
    pub env_vars: std::collections::HashSet<String>,
    /// Macro definitions
    pub macros: Vec<TfMacro>,
    /// Compiled regex cache for performance
    pub pattern_cache: HashMap<String, Regex>,
    /// Hooks registered for events
    pub hooks: HashMap<TfHookEvent, Vec<String>>,  // event -> macro names
    /// Key bindings (key sequence -> macro name or command)
    pub keybindings: HashMap<String, String>,
    /// Current working directory for #lcd
    pub current_dir: Option<String>,
    /// Current control flow state (for multi-line if/while/for)
    pub control_state: control_flow::ControlState,
    /// Background repeat processes
    pub processes: Vec<TfProcess>,
    /// Next process ID counter
    pub next_process_id: u32,
    /// Next macro sequence number (for TF-compatible numbering)
    pub next_macro_sequence: u32,
    /// Tokens for files already loaded via #loaded/#require
    pub loaded_tokens: std::collections::HashSet<String>,
    /// Stack of files currently being loaded (for nested loads)
    pub loading_files: Vec<String>,
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

/// Cached world info for TF functions (fg_world, world_info, nactive)
#[derive(Debug, Clone, Default)]
pub struct WorldInfoCache {
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub is_connected: bool,
    pub use_ssl: bool,
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
    /// Insert text at cursor
    Insert(String),
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
        TfEngine::default()
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

        let f = TfValue::Float(3.14);
        assert_eq!(f.to_int(), Some(3));
        assert!((f.to_float().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn test_tf_value_from_str() {
        assert_eq!(TfValue::from("42"), TfValue::Integer(42));
        assert_eq!(TfValue::from("-5"), TfValue::Integer(-5));
        assert!(matches!(TfValue::from("3.14"), TfValue::Float(_)));
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
