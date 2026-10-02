//! TF engine state carried across a hot reload or crash restart.
//!
//! A reload `exec`s a new Clay process. Without this, every macro, key binding, hook,
//! `/repeat` and `/set` variable defined since startup was gone afterwards - a TF user's
//! whole `.tfrc` worth of state. The snapshot is JSON, base64'd onto one `tf_state=` line
//! of the reload state's `[reload]` section (`persistence::save_reload_state_to`), in a
//! shape of its own rather than the engine's types, so it stays readable across versions:
//! a newer Clay reads an older snapshot (absent fields take their defaults), and one that
//! can't be read at all is treated as no snapshot - a TF cold start.
//!
//! What is deliberately NOT carried: anything mid-execution (effect frames, pipes, an
//! open multi-line `/if`), open `tfopen()` files, and the App-synced caches (worlds, the
//! foreground world), which the App refills itself. Variables that merely mirror the
//! environment or the engine's own seeds are left for the new engine to produce again,
//! exactly as `TfEngine::persistable_globals` does for settings.dat.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::{
    PartialSpec, ProcessKind, QuoteDisposition, RestrictLevel, TfAttributes, TfEngine, TfHookEvent, TfMacro, TfMatchMode,
    TfProcess, TfTrigger, TfValue, WatchdogConfig, TRANSIENT_GLOBALS,
};

/// Bumped only for a change an older reader would misread (never for an added field).
pub const SNAPSHOT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct TfStateSnapshot {
    pub version: u32,
    /// Variables worth carrying (see the module doc), in name order.
    pub globals: Vec<(String, SnapValue)>,
    /// Environment or seeded variables the user had `/unset`, so a fresh engine's copy
    /// doesn't bring them back.
    pub unset: Vec<String>,
    /// Variables marked for export to the environment (`/setenv`, `/export`).
    pub exported: Vec<String>,
    pub macros: Vec<SnapMacro>,
    pub keybindings: Vec<(String, String)>,
    pub current_dir: Option<String>,
    pub processes: Vec<SnapProcess>,
    pub next_process_id: u32,
    pub next_macro_sequence: u32,
    pub loaded_tokens: Vec<String>,
    pub watchdog: SnapWatch,
    pub watchdog_overrides: Vec<(String, SnapWatch)>,
    pub watchname: SnapWatch,
    pub default_world_character: Option<String>,
    pub default_world_password: Option<String>,
    pub default_world_file: Option<String>,
    pub world_files: Vec<(String, String)>,
    pub restrict_level: String,
    /// The status area's fields, and whether a script has made it its own.
    pub status: super::status::StatusLayout,
}

/// A variable's value. A float is kept as its bit pattern: JSON has no NaN or infinity.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum SnapValue {
    S(String),
    I(i64),
    F(u64),
    E(i64, String),
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct SnapMacro {
    pub name: String,
    pub body: String,
    pub trigger: Option<SnapTrigger>,
    /// Hook events by name, the first one first.
    pub hooks: Vec<String>,
    pub hook_pattern: Option<String>,
    pub keybinding: Option<String>,
    /// `TfAttributes::canonical` (TF's own spelling, which `TfAttributes::parse` reads).
    pub attributes: String,
    pub priority: i32,
    pub priority_expr: Option<String>,
    pub fall_through: bool,
    /// `PartialSpec::format_list`.
    pub partials: String,
    pub one_shot: Option<u32>,
    pub shots_remaining: Option<u32>,
    pub condition: Option<String>,
    pub probability: Option<f32>,
    pub world: Option<String>,
    pub sequence_number: u32,
    pub invisible: bool,
    pub quiet: bool,
    pub world_type: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct SnapTrigger {
    pub pattern: String,
    pub match_mode: String,
    /// The compiled regex's own source, so it is rebuilt exactly as it was.
    pub regex: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct SnapProcess {
    pub id: u32,
    pub command: String,
    pub interval_ms: u64,
    pub count: Option<u32>,
    pub remaining: Option<u32>,
    /// Time left until its next run when the snapshot was taken.
    pub due_in_ms: u64,
    pub world: Option<String>,
    pub synchronous: bool,
    pub on_prompt: bool,
    pub priority: i32,
    /// "repeat" or "quote".
    pub kind: String,
    /// The interval was given rather than %ptime's.
    pub interval_given: bool,
    /// A quote's lines still to go, and "send", "echo" or "exec".
    pub lines: Vec<String>,
    pub disposition: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct SnapWatch {
    pub enabled: bool,
    pub n1: usize,
    pub n2: usize,
}

impl From<&TfValue> for SnapValue {
    fn from(value: &TfValue) -> Self {
        match value {
            TfValue::String(s) => SnapValue::S(s.clone()),
            TfValue::Integer(n) => SnapValue::I(*n),
            TfValue::Float(f) => SnapValue::F(f.to_bits()),
            TfValue::Enum(n, s) => SnapValue::E(*n, s.clone()),
        }
    }
}

impl From<SnapValue> for TfValue {
    fn from(value: SnapValue) -> Self {
        match value {
            SnapValue::S(s) => TfValue::String(s),
            SnapValue::I(n) => TfValue::Integer(n),
            SnapValue::F(bits) => TfValue::Float(f64::from_bits(bits)),
            SnapValue::E(n, s) => TfValue::Enum(n, s),
        }
    }
}

impl From<&TfMacro> for SnapMacro {
    fn from(m: &TfMacro) -> Self {
        SnapMacro {
            name: m.name.clone(),
            body: m.body.clone(),
            trigger: m.trigger.as_ref().map(|t| SnapTrigger {
                pattern: t.pattern.clone(),
                match_mode: t.match_mode.name().to_string(),
                regex: t.compiled.as_ref().map(|re| re.as_str().to_string()),
            }),
            hooks: m.hook_events().iter().map(|e| e.name()).collect(),
            hook_pattern: m.hook_pattern.clone(),
            keybinding: m.keybinding.clone(),
            attributes: m.attributes.canonical(),
            priority: m.priority,
            priority_expr: m.priority_expr.clone(),
            fall_through: m.fall_through,
            partials: PartialSpec::format_list(&m.partials),
            one_shot: m.one_shot,
            shots_remaining: m.shots_remaining,
            condition: m.condition.clone(),
            probability: m.probability,
            world: m.world.clone(),
            sequence_number: m.sequence_number,
            invisible: m.invisible,
            quiet: m.quiet,
            world_type: m.world_type.clone(),
        }
    }
}

impl SnapMacro {
    fn into_macro(self) -> TfMacro {
        let mut hooks = self.hooks.iter().filter_map(|h| TfHookEvent::parse(h));
        let hook = hooks.next();
        let extra_hooks: Vec<TfHookEvent> = hooks.collect();
        TfMacro {
            name: self.name,
            body: self.body,
            trigger: self.trigger.map(|t| TfTrigger {
                compiled: t.regex.as_deref().and_then(|src| regex::Regex::new(src).ok()),
                pattern: t.pattern,
                match_mode: TfMatchMode::parse(&t.match_mode).unwrap_or_default(),
            }),
            hook,
            hook_pattern: self.hook_pattern,
            extra_hooks,
            keybinding: self.keybinding,
            attributes: TfAttributes::parse(&self.attributes).unwrap_or_default(),
            priority: self.priority,
            priority_expr: self.priority_expr,
            fall_through: self.fall_through,
            partials: PartialSpec::parse_list(&self.partials).unwrap_or_default(),
            one_shot: self.one_shot,
            shots_remaining: self.shots_remaining,
            condition: self.condition,
            probability: self.probability,
            world: self.world,
            sequence_number: self.sequence_number,
            invisible: self.invisible,
            quiet: self.quiet,
            world_type: self.world_type,
        }
    }
}

impl From<&WatchdogConfig> for SnapWatch {
    fn from(w: &WatchdogConfig) -> Self {
        SnapWatch { enabled: w.enabled, n1: w.n1, n2: w.n2 }
    }
}

fn sorted_pairs<'a>(map: impl Iterator<Item = (&'a String, &'a String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = map.map(|(k, v)| (k.clone(), v.clone())).collect();
    out.sort();
    out
}

impl TfStateSnapshot {
    /// Everything about `engine` worth carrying into a new process; process timers are
    /// measured from `now`.
    pub fn capture(engine: &TfEngine, now: Instant) -> Self {
        let globals = engine.persistable_globals().into_iter()
            .map(|(name, value)| (name.clone(), SnapValue::from(value)))
            .collect();
        let mut unset: Vec<String> = engine.env_imported.keys().chain(engine.seed_values.keys())
            .filter(|name| !engine.global_vars.contains_key(name.as_str()))
            .filter(|name| !TRANSIENT_GLOBALS.contains(&name.as_str()))
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        unset.sort();
        let mut exported: Vec<String> = engine.env_vars.iter().cloned().collect();
        exported.sort();
        let mut loaded_tokens: Vec<String> = engine.loaded_tokens.iter().cloned().collect();
        loaded_tokens.sort();
        let mut watchdog_overrides: Vec<(String, SnapWatch)> = engine.watchdog_overrides.iter()
            .map(|(world, w)| (world.clone(), SnapWatch::from(w)))
            .collect();
        watchdog_overrides.sort_by(|a, b| a.0.cmp(&b.0));
        TfStateSnapshot {
            version: SNAPSHOT_VERSION,
            globals,
            unset,
            exported,
            macros: engine.macros.iter().map(SnapMacro::from).collect(),
            keybindings: sorted_pairs(engine.keybindings.iter()),
            current_dir: engine.current_dir.clone(),
            processes: engine.processes.iter().map(|p| SnapProcess {
                id: p.id,
                command: p.command.clone(),
                interval_ms: p.interval.as_millis() as u64,
                count: p.count,
                remaining: p.remaining,
                due_in_ms: p.next_run.saturating_duration_since(now).as_millis() as u64,
                world: p.world.clone(),
                synchronous: p.synchronous,
                on_prompt: p.on_prompt,
                priority: p.priority,
                kind: match p.kind {
                    ProcessKind::Repeat => "repeat",
                    ProcessKind::Quote => "quote",
                }.to_string(),
                interval_given: p.interval_given,
                lines: p.lines.iter().cloned().collect(),
                disposition: match p.disposition {
                    QuoteDisposition::Send => "send",
                    QuoteDisposition::Echo => "echo",
                    QuoteDisposition::Exec => "exec",
                }.to_string(),
            }).collect(),
            next_process_id: engine.next_process_id,
            next_macro_sequence: engine.next_macro_sequence,
            loaded_tokens,
            watchdog: SnapWatch { enabled: engine.watchdog_enabled, n1: engine.watchdog_n1, n2: engine.watchdog_n2 },
            watchdog_overrides,
            watchname: SnapWatch { enabled: engine.watchname_enabled, n1: engine.watchname_n1, n2: engine.watchname_n2 },
            default_world_character: engine.default_world_character.clone(),
            default_world_password: engine.default_world_password.clone(),
            default_world_file: engine.default_world_file.clone(),
            world_files: sorted_pairs(engine.world_files.iter()),
            restrict_level: engine.restrict_level.name().to_string(),
            status: engine.status.clone(),
        }
    }

    /// Put this state into a freshly created `engine` (one that has imported the
    /// environment and seeded its defaults); process timers resume from `now`.
    pub fn restore(self, engine: &mut TfEngine, now: Instant) {
        for (name, value) in self.globals {
            engine.set_global(&name, value.into());
        }
        for name in &self.unset {
            engine.unset_global(name);
        }
        engine.env_vars = self.exported.into_iter().collect();
        engine.macros = self.macros.into_iter().map(SnapMacro::into_macro).collect();
        engine.pattern_cache.clear();
        engine.keybindings = self.keybindings.into_iter().collect();
        engine.current_dir = self.current_dir;
        engine.processes = self.processes.into_iter().map(|p| TfProcess {
            id: p.id,
            command: p.command,
            interval: Duration::from_millis(p.interval_ms),
            count: p.count,
            remaining: p.remaining,
            next_run: now + Duration::from_millis(p.due_in_ms),
            world: p.world,
            synchronous: p.synchronous,
            on_prompt: p.on_prompt,
            priority: p.priority,
            kind: if p.kind == "quote" { ProcessKind::Quote } else { ProcessKind::Repeat },
            interval_given: p.interval_given,
            lines: p.lines.into(),
            disposition: match p.disposition.as_str() {
                "echo" => QuoteDisposition::Echo,
                "exec" => QuoteDisposition::Exec,
                _ => QuoteDisposition::Send,
            },
        }).collect();
        // Never hand out an id or sequence number already in use.
        let max_pid = engine.processes.iter().map(|p| p.id + 1).max().unwrap_or(0);
        engine.next_process_id = self.next_process_id.max(max_pid).max(engine.next_process_id);
        let max_seq = engine.macros.iter().map(|m| m.sequence_number + 1).max().unwrap_or(0);
        engine.next_macro_sequence = self.next_macro_sequence.max(max_seq);
        engine.loaded_tokens = self.loaded_tokens.into_iter().collect();
        engine.watchdog_enabled = self.watchdog.enabled;
        engine.watchdog_n1 = self.watchdog.n1;
        engine.watchdog_n2 = self.watchdog.n2;
        engine.watchdog_overrides = self.watchdog_overrides.into_iter()
            .map(|(world, w)| (world, WatchdogConfig { enabled: w.enabled, n1: w.n1, n2: w.n2 }))
            .collect();
        engine.watchname_enabled = self.watchname.enabled;
        engine.watchname_n1 = self.watchname.n1;
        engine.watchname_n2 = self.watchname.n2;
        engine.default_world_character = self.default_world_character;
        engine.default_world_password = self.default_world_password;
        engine.default_world_file = self.default_world_file;
        engine.world_files = self.world_files.into_iter().collect();
        engine.restrict_level = RestrictLevel::parse(&self.restrict_level).unwrap_or_default();
        // A snapshot from before the status area was kept has none: TF's default stays.
        if !self.status.rows.is_empty() {
            engine.status = self.status;
        }
    }

    /// One line's worth of text: base64 of the JSON.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        base64::engine::general_purpose::STANDARD.encode(json)
    }

    /// The snapshot `encode` made, or None for anything unreadable.
    pub fn decode(text: &str) -> Option<Self> {
        let json = base64::engine::general_purpose::STANDARD.decode(text.trim()).ok()?;
        serde_json::from_slice(&json).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything a .tfrc builds - macros of every kind, a binding, a hook, a /repeat,
    /// variables (and an unset seed) - comes back the same in a fresh engine.
    #[test]
    fn test_snapshot_round_trips_engine_state() {
        let mut engine = TfEngine::new();
        for line in [
            "/def -p5 -F -mregexp -t\"^You see ([A-Za-z]+)\" -aBCred see = /echo saw %P1",
            "/def -P1Cgreen;LBu -t\"(foo) bar\" partial",
            "/def -hCONNECT|DISCONNECT -ag conn = /echo connected",
            "/def -h\"SEND n*\" -q sendhook = /echo sending",
            "/def -n2 -c50 -w\"Arctic\" -T\"lp*\" -E\"x > 1\" limited = /echo lim",
            "/def -i hidden = /echo invisible",
            "/def -b\"^X^R\" rebind = /echo bound",
            "/bind ^Q = /echo plain bind",
            "/set greeting=hello there",
            "/set count=7",
            "/set more=on",
            "/unset wordpunct",
            "/setenv CLAY_SNAPSHOT_TEST yes",
            "/status_add -A@world hp:4",
            "/status_add -r1 -c mana:5",
        ] {
            engine.execute(line);
        }
        engine.set_global("ratio", TfValue::Float(1.0 / 3.0));
        engine.loaded_tokens.insert("my-token".into());
        engine.world_files.insert("arctic".into(), "~/arctic.tf".into());
        engine.default_world_character = Some("bob".into());
        engine.watchdog_enabled = true;
        engine.watchdog_n1 = 3;
        let now = Instant::now();
        // As a /repeat effect would have registered it.
        let effects = engine.run("/repeat -2 3 /echo tick");
        for effect in effects {
            if let crate::tf::effects::TfEffect::StartProcess(p) = effect {
                engine.processes.push(p);
            }
        }

        let snapshot = TfStateSnapshot::capture(&engine, now);
        let decoded = TfStateSnapshot::decode(&snapshot.encode()).expect("decodes");
        assert_eq!(decoded, snapshot);

        let mut fresh = TfEngine::new();
        decoded.restore(&mut fresh, now);

        assert_eq!(fresh.macros.len(), engine.macros.len());
        for (a, b) in engine.macros.iter().zip(fresh.macros.iter()) {
            assert_eq!(SnapMacro::from(a), SnapMacro::from(b), "macro {}", a.name);
            assert_eq!(a.trigger.as_ref().and_then(|t| t.compiled.as_ref()).map(|r| r.as_str().to_string()),
                b.trigger.as_ref().and_then(|t| t.compiled.as_ref()).map(|r| r.as_str().to_string()));
        }
        assert_eq!(fresh.keybindings, engine.keybindings);
        for name in ["greeting", "count", "ratio", "more", "CLAY_SNAPSHOT_TEST"] {
            assert_eq!(fresh.get_var(name), engine.get_var(name), "{name}");
        }
        assert!(fresh.get_var("wordpunct").is_none(), "an unset seed stays unset");
        assert!(fresh.env_vars.contains("CLAY_SNAPSHOT_TEST"));
        assert!(fresh.loaded_tokens.contains("my-token"));
        assert_eq!(fresh.world_files.get("arctic").map(String::as_str), Some("~/arctic.tf"));
        assert_eq!(fresh.default_world_character.as_deref(), Some("bob"));
        assert!(fresh.watchdog_enabled);
        assert_eq!(fresh.watchdog_n1, 3);
        assert_eq!(fresh.status, engine.status, "the status area, customized");
        assert!(fresh.status.customized);
        assert_eq!(fresh.processes.len(), 1);
        let (old, new) = (&engine.processes[0], &fresh.processes[0]);
        assert_eq!((old.id, &old.command, old.interval, old.remaining), (new.id, &new.command, new.interval, new.remaining));
        let drift = if new.next_run > old.next_run { new.next_run - old.next_run } else { old.next_run - new.next_run };
        assert!(drift < Duration::from_millis(50), "next run kept: {drift:?}");
        assert!(fresh.next_process_id > old.id);
        assert!(fresh.next_macro_sequence >= engine.next_macro_sequence);
        // The restored macros work: a trigger fires with its attributes, the hook list holds.
        let see = fresh.macros.iter().find(|m| m.name == "see").unwrap();
        assert!(see.attributes.bold && see.attributes.fg.as_deref() == Some("red"));
        let conn = fresh.macros.iter().find(|m| m.name == "conn").unwrap();
        assert!(conn.has_hook(TfHookEvent::Connect) && conn.has_hook(TfHookEvent::Disconnect));
        let result = crate::tf::bridge::process_line(&mut fresh, "You see Bob", None, None);
        assert!(result.effects.iter().any(|e| matches!(e, crate::tf::effects::TfEffect::Output { text, .. } if text == "saw Bob")),
            "{:?}", result.effects);
    }

    /// An empty or garbled snapshot reads as none (TF cold start), never as a panic.
    #[test]
    fn test_unreadable_snapshot_is_none() {
        assert!(TfStateSnapshot::decode("").is_none());
        assert!(TfStateSnapshot::decode("not base64 !!").is_none());
        let not_json = base64::engine::general_purpose::STANDARD.encode(b"{nope");
        assert!(TfStateSnapshot::decode(&not_json).is_none());
        // An older snapshot missing newer fields still reads.
        let old = base64::engine::general_purpose::STANDARD.encode(br#"{"version":1,"globals":[["x",{"S":"y"}]]}"#);
        let snap = TfStateSnapshot::decode(&old).expect("old snapshot reads");
        assert_eq!(snap.globals, vec![("x".to_string(), SnapValue::S("y".to_string()))]);
    }
}
