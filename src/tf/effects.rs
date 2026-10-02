//! Ordered side effects of TF execution.
//!
//! Everything a TF command does that reaches outside the engine - text for the output
//! area, an error, a line for a MUD, a Clay command, a `/recall`, a `/repeat` process, a
//! world definition - is an effect. Effects are *emitted* in the order they happen (see
//! `TfEngine::emit`), never collected into a per-command "result" and folded later: that
//! folding is what used to lose all but the first Clay command of a macro body, drop a
//! loaded file's `/addworld` lines, and print a macro's `/echo` output after the effects
//! that followed it.
//!
//! Each public `parser::execute_command` call runs in its own frame (`TfEngine::begin_frame`),
//! and folds that frame back into one `TfCommandResult` for its caller - the familiar
//! single-variant shapes (`Success`, `ClayCommand`, `SendToMud`, ...) whenever the command
//! produced one kind of effect, `TfCommandResult::Effects` when it produced a mix. A caller
//! that collects sub-results (a macro body, a loop, a loaded file) re-emits each one as soon
//! as it gets it (`TfEngine::emit_result`), so order is preserved at every level.

use super::{control_flow, PendingWorldOp, QuoteDisposition, QuoteTiming, RecallOptions, TfCommandResult, TfProcess, TfValue};

/// One side effect of TF execution - see the module doc.
#[derive(Debug)]
pub enum TfEffect {
    /// Text for tfout. `world` is `/echo -w`'s target; None means the world the command ran
    /// for. `attrs` are TF attributes for the line: still to be applied (`echo()`'s 2nd
    /// argument) - or, when `plain` is the text as it was before them (`/echo -a`), already
    /// drawn into `text`, which a `$()` keeps as TF does. Either way the line remembers
    /// them as its own (for `/recall -a`).
    Output { text: String, attrs: String, world: Option<String>, plain: Option<String> },
    /// Text for tferr. `at` says where in a file being loaded it happened (`<file>, line N`,
    /// or `<file>, lines A-B` for a command continued over several lines); such an error
    /// prints TF-style, `% <at>: <msg>`.
    Error { msg: String, at: Option<String> },
    /// A line for a MUD. `world` None means the world the command ran for.
    Send { text: String, world: Option<String>, no_eol: bool },
    /// A command only Clay can run (`/addworld`, `/world`, `/log`, `/dc`, ...). `at`: as
    /// for `Error`, so a command Clay doesn't know either can say where it was.
    Clay { cmd: String, at: Option<String> },
    /// `/recall` - needs the App's output buffers.
    Recall(RecallOptions),
    /// `/repeat` - a process for the App's timer to run.
    StartProcess(TfProcess),
    /// A `/quote` the engine could not finish itself (it has a delay, or a `/recall` source).
    Quote(QuoteRequest),
    /// `addworld()` - define or update a world.
    WorldOp(PendingWorldOp),
    /// A command name that is neither a TF command, a macro, nor anything Clay knows.
    Unknown(String),
    /// `/connect`: open a world. The App checks the world exists when it gets here, after
    /// whatever `/addworld` came before it has been applied.
    Connect(ConnectRequest),
    /// A special variable that stands for one of Clay's own settings (`%more`, `%isize`,
    /// ...) was assigned this value: the App applies it (see `special_vars::BOUND`). The
    /// value travels with the effect - the variable itself may be re-synced from Clay's
    /// settings before the effect is applied.
    Setting(String, TfValue),
    /// `/sh [<command>]`: run <command> (None: an interactive %SHELL) on the terminal,
    /// as TF does - only Clay's own console has one; elsewhere a command's output is
    /// captured instead. Its exit status becomes `%?`.
    Shell { command: Option<String> },
    /// `/suspend`: stop Clay as ^Z would (console only).
    Suspend,
    /// `/prompt`, `prompt()`: `world`'s prompt is now `text` (attributes already applied).
    SetPrompt { text: String, world: String },
    /// `/localecho on|off`: ask `world`'s server not to echo (DONT ECHO), or to (DO ECHO).
    LocalEcho { on: bool, world: String },
    /// `/recordline`: put `text` (attributes already applied) into a history without
    /// showing it - `world`'s for `World` (None: the current world), with `time` (seconds
    /// since the epoch) instead of now when given.
    RecordLine { text: String, target: RecordTarget, world: Option<String>, time: Option<f64> },
}

/// Which history `/recordline` records into (`-w`, `-l`, `-g` (the default), `-i`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordTarget {
    World,
    Local,
    Global,
    Input,
}

/// What `/connect [-lqxbf] [<world>]` / `/connect <host> <port>` asked for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectRequest {
    /// The world to connect; None (without `host_port`) is the first defined world.
    pub world: Option<String>,
    /// `/connect <host> <port>`: a temporary world "(unnamed<N>)" for that address.
    pub host_port: Option<(String, String)>,
    /// `-l`: no automatic login.
    pub no_login: bool,
    /// `-q`: a quiet login.
    pub quiet: bool,
    /// `-x`: SSL for this connection.
    pub ssl: bool,
    /// Leave the foreground world where it is (`-b`, or a hook/trigger running for a
    /// background world, unless `-f`).
    pub background: bool,
}

/// The fields of `TfCommandResult::Quote`, as an effect.
#[derive(Debug)]
pub struct QuoteRequest {
    pub lines: Vec<String>,
    pub disposition: QuoteDisposition,
    pub world: Option<String>,
    pub timing: QuoteTiming,
    pub recall_opts: Option<(RecallOptions, String)>,
    pub strip_ansi: bool,
    pub pid: Option<u32>,
    pub label: String,
}

impl TfEffect {
    /// An error, not (yet) tied to a place in a file.
    pub fn error(msg: impl Into<String>) -> Self {
        TfEffect::Error { msg: msg.into(), at: None }
    }

    /// A Clay command, not (yet) tied to a place in a file.
    pub fn clay(cmd: impl Into<String>) -> Self {
        TfEffect::Clay { cmd: cmd.into(), at: None }
    }

    /// Tie an error or Clay command to the place in a file being loaded that caused it,
    /// unless it already names one (an error from a nested /load keeps its own file's).
    pub fn located(self, location: &str) -> Self {
        match self {
            TfEffect::Error { msg, at: None } => TfEffect::Error { msg, at: Some(location.to_string()) },
            TfEffect::Clay { cmd, at: None } => TfEffect::Clay { cmd, at: Some(location.to_string()) },
            other => other,
        }
    }
}

impl TfCommandResult {
    /// Results that steer execution rather than produce anything: `/return`, `/result`,
    /// `/exit` during a load, and a `/break` still unwinding enclosing loops. These
    /// propagate upward as results; everything else is emitted as effects.
    pub fn is_control(&self) -> bool {
        match self {
            TfCommandResult::Return(_) | TfCommandResult::Result(_) | TfCommandResult::ExitLoad(_) => true,
            TfCommandResult::Error(e) => control_flow::parse_break_marker(e).is_some(),
            _ => false,
        }
    }
}

/// Append the effects of a non-control result to `out`. Control results add nothing (the
/// caller is responsible for them; see `TfCommandResult::is_control`).
pub fn push_result_effects(result: TfCommandResult, out: &mut Vec<TfEffect>) {
    match result {
        TfCommandResult::Success(Some(text)) => out.push(TfEffect::Output { text, attrs: String::new(), world: None, plain: None }),
        TfCommandResult::Success(None) | TfCommandResult::NotTfCommand => {}
        TfCommandResult::Error(e) => {
            if control_flow::parse_break_marker(&e).is_none() {
                out.push(TfEffect::error(e));
            }
        }
        TfCommandResult::SendToMud(text) => out.push(TfEffect::Send { text, world: None, no_eol: false }),
        TfCommandResult::ClayCommand(cmd) => out.push(TfEffect::clay(cmd)),
        TfCommandResult::Recall(opts) => out.push(TfEffect::Recall(opts)),
        TfCommandResult::RepeatProcess(p) => out.push(TfEffect::StartProcess(p)),
        TfCommandResult::Quote { lines, disposition, world, timing, recall_opts, strip_ansi, pid, label } => {
            out.push(TfEffect::Quote(QuoteRequest { lines, disposition, world, timing, recall_opts, strip_ansi, pid, label }));
        }
        TfCommandResult::UnknownCommand(name) => out.push(TfEffect::Unknown(name)),
        TfCommandResult::Effects(effects) => out.extend(effects),
        TfCommandResult::Return(_) | TfCommandResult::Result(_) | TfCommandResult::ExitLoad(_) => {}
    }
}

/// Fold a frame's effects back into one result. A frame that produced a single kind of
/// effect folds to the classic single-variant shape (consecutive plain outputs are joined
/// with newlines, the way a macro's output always was), so callers that only ever see
/// simple commands are unaffected; a mix stays an ordered `TfCommandResult::Effects`.
pub fn effects_to_result(effects: Vec<TfEffect>) -> TfCommandResult {
    let plain_output = |e: &TfEffect| matches!(e, TfEffect::Output { attrs, world: None, plain: None, .. } if attrs.is_empty());
    if effects.is_empty() {
        return TfCommandResult::Success(None);
    }
    if effects.iter().all(plain_output) {
        let text: Vec<String> = effects.into_iter()
            .filter_map(|e| match e { TfEffect::Output { text, .. } => Some(text), _ => None })
            .collect();
        return TfCommandResult::Success(Some(text.join("\n")));
    }
    if effects.len() == 1 {
        let mut effects = effects;
        let effect = effects.pop().expect("one effect");
        return match effect {
            // A located error or command stays an effect: folded, it would lose its place.
            TfEffect::Error { msg, at: None } => TfCommandResult::Error(msg),
            TfEffect::Send { text, world: None, no_eol: false } => TfCommandResult::SendToMud(text),
            TfEffect::Clay { cmd, at: None } => TfCommandResult::ClayCommand(cmd),
            TfEffect::Recall(opts) => TfCommandResult::Recall(opts),
            TfEffect::StartProcess(p) => TfCommandResult::RepeatProcess(p),
            TfEffect::Quote(q) => TfCommandResult::Quote {
                lines: q.lines, disposition: q.disposition, world: q.world, timing: q.timing,
                recall_opts: q.recall_opts, strip_ansi: q.strip_ansi, pid: q.pid, label: q.label,
            },
            TfEffect::Unknown(name) => TfCommandResult::UnknownCommand(name),
            other => TfCommandResult::Effects(vec![other]),
        };
    }
    TfCommandResult::Effects(effects)
}

/// The text of every `Output` effect, in order - for capturing a command's output (`$()`),
/// where everything else it did still has to happen.
pub fn split_output(effects: Vec<TfEffect>) -> (Vec<String>, Vec<TfEffect>) {
    let mut text = Vec::new();
    let mut rest = Vec::new();
    for e in effects {
        match e {
            TfEffect::Output { text: t, world: None, .. } => text.push(t),
            other => rest.push(other),
        }
    }
    (text, rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tf::TfEngine;

    fn kinds(effects: &[TfEffect]) -> Vec<String> {
        effects.iter().map(|e| match e {
            TfEffect::Output { text, .. } => format!("out:{}", text),
            TfEffect::Error { msg, at: None } => format!("err:{}", msg),
            TfEffect::Error { msg, at: Some(at) } => format!("err:{}: {}", at, msg),
            TfEffect::Send { text, .. } => format!("send:{}", text),
            TfEffect::Clay { cmd, .. } => format!("clay:{}", cmd),
            TfEffect::Recall(_) => "recall".to_string(),
            TfEffect::StartProcess(p) => format!("process:{}", p.command),
            TfEffect::Quote(_) => "quote".to_string(),
            TfEffect::WorldOp(op) => format!("worldop:{}", op.name),
            TfEffect::Unknown(n) => format!("unknown:{}", n),
            TfEffect::Connect(req) => format!("connect:{:?}", req.world),
            TfEffect::Setting(name, _) => format!("setting:{}", name),
            TfEffect::Shell { command } => format!("sh:{}", command.as_deref().unwrap_or("")),
            TfEffect::Suspend => "suspend".to_string(),
            TfEffect::SetPrompt { text, world } => format!("prompt:{}:{}", world, text),
            TfEffect::LocalEcho { on, world } => format!("localecho:{}:{}", world, on),
            TfEffect::RecordLine { text, target, .. } => format!("recordline:{:?}:{}", target, text),
        }).collect()
    }

    /// Every Clay command in a macro body survives, in order with the body's output -
    /// the body used to keep only its first Clay command and lose its /echo text.
    #[test]
    fn test_macro_body_keeps_every_effect_in_order() {
        let mut engine = TfEngine::new();
        engine.execute("/def m = /echo a%; /zzz_one x%; think%; /echo b%; /zzz_two y");
        let effects = engine.run("/m");
        assert_eq!(kinds(&effects), vec![
            "out:a", "clay:/zzz_one x", "send:think", "out:b", "clay:/zzz_two y",
        ]);
    }

    /// $() captures a command's output; what else it did still happens.
    #[test]
    fn test_command_substitution_captures_output_and_keeps_the_rest() {
        let mut engine = TfEngine::new();
        engine.execute("/def m = /echo a%; /zzz_cmd%; /echo b");
        let effects = engine.run("/set x=$(/m)");
        assert_eq!(engine.get_var("x").map(|v| v.to_string_value()), Some("a\nb".to_string()));
        assert_eq!(kinds(&effects), vec!["clay:/zzz_cmd"]);
    }

    /// A loaded file's effects come out line by line, errors where they happened, and
    /// the Clay commands a .tfrc relies on (/addworld, /world, /log) are not dropped.
    #[test]
    fn test_loaded_file_effects_are_ordered_and_kept() {
        let path = std::env::temp_dir().join(format!("clay_effects_load_{}.tf", std::process::id()));
        std::fs::write(&path, "/echo one\n/set 123bad=x\n/addworld foo localhost 4000\n/echo two\n").unwrap();
        let mut engine = TfEngine::new();
        let effects = engine.run(&format!("/load -q {}", path.display()));
        let _ = std::fs::remove_file(&path);
        let k = kinds(&effects);
        assert_eq!(k.len(), 4, "{k:?}");
        assert_eq!(k[0], "out:one");
        assert!(k[1].starts_with("err:") && k[1].contains(", line 2: "), "{k:?}");
        assert_eq!(k[2], "worldop:foo", "{k:?}");
        assert_eq!(k[3], "out:two");
    }

    /// A trigger body with two Clay commands produces both.
    #[test]
    fn test_trigger_body_keeps_both_clay_commands() {
        let mut engine = TfEngine::new();
        engine.execute(r#"/def -t"hello*" t = /zzz_one%; /zzz_two"#);
        let result = crate::tf::bridge::process_line(&mut engine, "hello there", None, None);
        assert_eq!(kinds(&result.effects), vec!["clay:/zzz_one", "clay:/zzz_two"]);
    }

    /// /repeat waits one interval before its first run; -n runs it right away.
    #[test]
    fn test_repeat_first_run_waits_one_interval_unless_n() {
        let mut engine = TfEngine::new();
        let before = std::time::Instant::now();
        let effects = engine.run("/repeat -5 3 /echo x");
        match effects.as_slice() {
            [TfEffect::StartProcess(p)] => {
                assert!(p.next_run >= before + std::time::Duration::from_secs(5));
                assert_eq!(engine.get_var("?").map(|v| v.to_string_value()), Some(p.id.to_string()));
            }
            other => panic!("expected one process, got {:?}", kinds(other)),
        }
        let effects = engine.run("/repeat -n -5 3 /echo x");
        match effects.as_slice() {
            [TfEffect::StartProcess(p)] => assert!(p.next_run < before + std::time::Duration::from_secs(5)),
            other => panic!("expected one process, got {:?}", kinds(other)),
        }
    }

    /// A synchronous /repeat runs every iteration now, keeping each one's output.
    #[test]
    fn test_synchronous_repeat_keeps_every_iteration() {
        let mut engine = TfEngine::new();
        let effects = engine.run("/repeat -S 3 /echo x");
        // Consecutive output folds into one block of lines.
        assert_eq!(kinds(&effects), vec!["out:x\nx\nx"]);
    }

    /// tfclose("o") suppresses the rest of a macro body's output (stdlib's isvar idiom).
    #[test]
    fn test_tfclose_o_suppresses_rest_of_body_output() {
        let mut engine = TfEngine::new();
        engine.execute(r#"/def m = /echo before%; /test tfclose("o")%; /echo after"#);
        let effects = engine.run("/m");
        assert_eq!(kinds(&effects), vec!["out:before"]);
        // Closing tfout lasts only for that body.
        let effects = engine.run("/echo later");
        assert_eq!(kinds(&effects), vec!["out:later"]);
    }

    /// The fold back into a single result keeps the classic shapes for simple commands.
    #[test]
    fn test_effects_to_result_keeps_classic_shapes() {
        assert!(matches!(effects_to_result(vec![]), TfCommandResult::Success(None)));
        let r = effects_to_result(vec![
            TfEffect::Output { text: "a".into(), attrs: String::new(), world: None, plain: None },
            TfEffect::Output { text: "b".into(), attrs: String::new(), world: None, plain: None },
        ]);
        assert!(matches!(r, TfCommandResult::Success(Some(ref t)) if t == "a\nb"));
        assert!(matches!(effects_to_result(vec![TfEffect::clay("/x")]), TfCommandResult::ClayCommand(_)));
        let mixed = effects_to_result(vec![
            TfEffect::Output { text: "a".into(), attrs: String::new(), world: None, plain: None },
            TfEffect::clay("/x"),
        ]);
        assert!(matches!(mixed, TfCommandResult::Effects(ref v) if v.len() == 2));
    }

    /// A trigger runs with its own world as TF's current world: ${world_name},
    /// world_info() and world_info(field) all name the triggering (background) world,
    /// not the foreground one.
    #[test]
    fn test_trigger_sees_its_own_world_as_current() {
        use crate::tf::WorldInfoCache;
        let mut engine = TfEngine::new();
        engine.current_world = Some("Front".to_string());
        engine.world_info_cache = vec![
            WorldInfoCache { name: "Front".into(), host: "front.example".into(), ..Default::default() },
            WorldInfoCache { name: "Back".into(), host: "back.example".into(), ..Default::default() },
        ];
        engine.execute(r#"/def -t"ping" t = /echo ${world_name} $[world_info()] $[world_info("host")] $[world_info("Front", "host")]"#);
        let result = crate::tf::bridge::process_line(&mut engine, "ping", Some("Back"), None);
        assert_eq!(kinds(&result.effects), vec!["out:Back Back back.example front.example"]);
        // Outside the trigger, the foreground world is current again.
        assert_eq!(kinds(&engine.run("/echo ${world_name}")), vec!["out:Front"]);
    }

    /// A typed line follows %sub (`/help sub`): off (TF's default) runs a command
    /// exactly as typed; on splits it at %; and turns %% into %; full runs it as a
    /// macro body.
    #[test]
    fn test_typed_line_follows_sub() {
        let mut engine = TfEngine::new();
        engine.execute("/set x=hello");
        assert_eq!(kinds(&engine.run_typed("/echo %{x} $[1+2]")), vec!["out:%{x} $[1+2]"]);

        engine.execute("/set sub=on");
        assert_eq!(kinds(&engine.run_typed("/echo a%;/echo 100%%")), vec!["out:a", "out:100%"]);
        assert_eq!(crate::tf::sub_on_expand(r"x\65y%\z"), "xAy\nz");

        engine.execute("/set sub=full");
        assert_eq!(kinds(&engine.run_typed("/echo %{x} $[1+2]")), vec!["out:hello 3"]);
        // Plain text is sent (after full substitution).
        assert_eq!(kinds(&engine.run_typed("say %{x}")), vec!["send:say hello"]);
    }
}
