//! TinyFugue compatibility layer for Clay MUD client.
//!
//! This module provides TF-style commands using `#` prefix instead of `/`.
//! Commands work alongside existing Clay commands for full coexistence.

pub mod parser;
pub mod variables;
pub mod expressions;
pub mod control_flow;
pub mod macros;

use std::collections::HashMap;
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
