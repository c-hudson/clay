//! TinyFugue's startup, for a TF user's habits: its command line
//! (`tf [-L<dir>] [-f[<file>]] [-c<cmd>] [-nlq] [<world> | <host> <port>]`) and what it
//! runs before the user types anything - the library's local.tf, the personal rc
//! (~/.tfrc and friends), ~/.tinytalk, the -c command, and a connect to the world named
//! on the command line. Clay's own options are untouched: everything starting with
//! "--", and -v (Clay's version, not TF's "no visual mode"), -h and -D.
//!
//! None of this runs on a hot reload that carried the TF state over (`tf::snapshot`):
//! the rc's definitions are already in place, and its connections are still open.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tokio::sync::mpsc;

use crate::tf::effects::TfEffect;
use crate::tfrun::{self, TfEffectCtx};
use crate::{App, AppEvent};

/// Which personal rc file to load.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) enum RcChoice {
    /// The first of ~/.tfrc, ~/tfrc, ./.tfrc, ./tfrc that exists.
    #[default]
    Search,
    /// `-f`: none.
    None,
    /// `-f<file>`.
    File(String),
}

/// TF's startup options from the command line.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TfStartupOpts {
    /// `-L<dir>`: the library directory, for this session only.
    pub lib_dir: Option<String>,
    pub rc: RcChoice,
    /// `-c<cmd>`: run after the rc, as if typed.
    pub commands: Vec<String>,
    /// `-l`: no login for the world named on the command line.
    pub no_login: bool,
    /// `-q`: a quiet login for it.
    pub quiet: bool,
    /// `<world>`: connect it once started.
    pub world: Option<String>,
    /// `<host> <port>`: connect a temporary world there once started.
    pub host_port: Option<(String, String)>,
    /// Any TF option or argument was given (they can't be combined with some of Clay's).
    pub given: bool,
}

static STARTUP_OPTS: OnceLock<TfStartupOpts> = OnceLock::new();

/// Record what the command line asked of TF's startup (once, from `main`).
pub(crate) fn set_startup_opts(opts: TfStartupOpts) {
    let _ = STARTUP_OPTS.set(opts);
}

fn startup_opts() -> TfStartupOpts {
    STARTUP_OPTS.get().cloned().unwrap_or_default()
}

/// Clay's own options; every other argument is TF's.
fn is_clay_arg(arg: &str) -> bool {
    arg.starts_with("--") || matches!(arg, "-v" | "-h" | "-D")
}

/// Split TinyFugue's options and arguments out of the command line, returning them and
/// the rest - Clay's own options - for Clay's parser. TF's option values may be attached
/// (`-L/usr/lib/tf`) or, for -L and -c, the next argument. An unknown dash option is left
/// for Clay's parser to reject. The error is the message to show.
pub(crate) fn split_tf_cli(args: &[String]) -> Result<(TfStartupOpts, Vec<String>), String> {
    let mut opts = TfStartupOpts::default();
    let mut rest = Vec::new();
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        // --grep and --grep-archive take every argument after them as their own.
        if arg == "--grep-archive" || arg.starts_with("--grep=") {
            rest.extend(args[i..].iter().cloned());
            break;
        }
        if is_clay_arg(arg) {
            rest.push(arg.clone());
            i += 1;
            continue;
        }
        let Some(body) = arg.strip_prefix('-') else {
            positional.push(arg.clone());
            opts.given = true;
            i += 1;
            continue;
        };
        let mut value_or_next = |name: &str, attached: &str| -> Result<String, String> {
            if !attached.is_empty() {
                return Ok(attached.to_string());
            }
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("-{} needs a value", name))
        };
        match body.chars().next() {
            Some('L') => opts.lib_dir = Some(value_or_next("L", &body[1..])?),
            Some('c') => opts.commands.push(value_or_next("c", &body[1..])?),
            Some('f') => {
                opts.rc = if body.len() == 1 { RcChoice::None } else { RcChoice::File(body[1..].to_string()) };
            }
            Some(_) if body.chars().all(|c| "nlqv".contains(c)) => {
                for c in body.chars() {
                    match c {
                        'l' => opts.no_login = true,
                        'q' => opts.quiet = true,
                        // TF's "don't auto-connect": Clay never does at startup anyway.
                        'n' => {}
                        _ => return Err("-v is Clay's version option; TinyFugue's -v (no visual \
                            mode) has no equivalent: Clay's console is always visual".to_string()),
                    }
                }
            }
            _ => {
                rest.push(arg.clone());
                i += 1;
                continue;
            }
        }
        opts.given = true;
        i += 1;
    }
    match positional.len() {
        0 => {}
        1 => opts.world = positional.pop(),
        2 => opts.host_port = Some((positional[0].clone(), positional[1].clone())),
        _ => return Err("expected a world name, or a host and port".to_string()),
    }
    Ok((opts, rest))
}

/// The command line without TinyFugue's startup options and world - what a hot reload or
/// crash restart passes on, so the new process never loads the rc or connects again.
pub(crate) fn clay_args_only(args: Vec<String>) -> Vec<String> {
    match split_tf_cli(&args) {
        Ok((_, rest)) => rest,
        Err(_) => args,
    }
}

/// The personal rc TF would load: `-f<file>`, none for `-f`, else the first of
/// ~/.tfrc, ~/tfrc, ./.tfrc, ./tfrc that exists.
pub(crate) fn find_personal_rc(choice: &RcChoice, home: &Path, cwd: &Path) -> Option<PathBuf> {
    match choice {
        RcChoice::None => None,
        RcChoice::File(file) => Some(PathBuf::from(file)),
        RcChoice::Search => [home.join(".tfrc"), home.join("tfrc"), cwd.join(".tfrc"), cwd.join("tfrc")]
            .into_iter()
            .find(|p| p.is_file()),
    }
}

/// The TF lines a startup runs, in TF's order (`tf(1)`): the library's local.tf and the
/// personal rc, then ~/.tinytalk - local.tf and .tinytalk only when a personal rc is
/// loaded, so a Clay user without one starts exactly as before.
fn startup_loads(engine: &crate::tf::TfEngine, opts: &TfStartupOpts) -> Vec<String> {
    let home = PathBuf::from(engine.home_dir());
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
    let Some(rc) = find_personal_rc(&opts.rc, &home, &cwd) else {
        return Vec::new();
    };
    let mut loads = Vec::new();
    if let Some(lib) = engine.get_var("TFLIBDIR").map(|v| v.to_string_value()).filter(|d| !d.is_empty()) {
        let local = Path::new(&lib).join("local.tf");
        if local.is_file() {
            loads.push(format!("/load {}", local.display()));
        }
    }
    loads.push(format!("/load {}", rc.display()));
    let tinytalk = home.join(".tinytalk");
    if opts.rc == RcChoice::Search && tinytalk.is_file() {
        loads.push(format!("/load {}", tinytalk.display()));
    }
    loads
}

/// The `/connect` for the world named on the command line, with -l/-q.
fn startup_connect(opts: &TfStartupOpts) -> Option<String> {
    let flags = match (opts.no_login, opts.quiet) {
        (true, true) => "-lq ",
        (true, false) => "-l ",
        (false, true) => "-q ",
        (false, false) => "",
    };
    match (&opts.world, &opts.host_port) {
        (Some(world), _) => Some(format!("/connect {}{}", flags, world)),
        (None, Some((host, port))) => Some(format!("/connect {}{} {}", flags, host, port)),
        (None, None) => None,
    }
}

/// The rc's messages - its errors above all - must be seen: take the splash down first,
/// as a MUD's first line would.
fn clear_splash(app: &mut App) {
    let world = app.current_world_mut();
    if world.showing_splash {
        world.showing_splash = false;
        world.needs_redraw = true;
        world.output_lines.clear();
        world.scroll_offset = 0;
        app.needs_output_redraw = true;
    }
}

/// Before the startup connect: the world the rc's messages went to, if it is Clay's
/// placeholder - which goes away as soon as a world connects.
fn placeholder_world(app: &App) -> Option<usize> {
    app.current_world().is_initial_world.then_some(app.current_world_index)
}

/// After it: if the connect brought another world to the foreground, show the rc's
/// messages there too (as TF's single screen would), before the MUD's own output.
fn carry_startup_messages(app: &mut App, placeholder: Option<usize>) {
    let Some(from) = placeholder else { return };
    if app.current_world_index == from || from >= app.worlds.len() {
        return;
    }
    let lines: Vec<String> = app.worlds[from].output_lines.iter()
        .filter(|l| !l.from_server)
        .map(|l| l.text.clone())
        .collect();
    if lines.is_empty() {
        return;
    }
    clear_splash(app);
    for line in lines {
        app.add_output(&line);
    }
}

/// Startup actions (Clay's own; split on ";"), as they always ran: TF commands only.
fn startup_action_commands(app: &App) -> Vec<String> {
    app.settings.actions.iter()
        .filter(|a| a.startup && a.enabled)
        .flat_map(|a| a.command.split(';').map(|c| c.trim().to_string()).collect::<Vec<_>>())
        .filter(|c| !c.is_empty())
        .collect()
}

/// `-L<dir>`: TFLIBDIR for this session only - marked as coming from the environment so
/// it is never saved.
fn apply_lib_dir(app: &mut App, opts: &TfStartupOpts) {
    if let Some(dir) = &opts.lib_dir {
        let dir = app.tf_engine.expand_tilde(dir);
        app.tf_engine.set_global("TFLIBDIR", crate::tf::TfValue::String(dir.clone()));
        app.tf_engine.env_imported.insert("TFLIBDIR".to_string(), dir);
    }
}

/// The console's startup (see the module doc), each step's effects applied before the
/// next: the rc's /addworld lines must exist before -c or the startup connect runs.
/// Returns true if one of them quit Clay.
pub(crate) async fn run_tf_startup_console(app: &mut App, event_tx: &mpsc::Sender<AppEvent>) -> bool {
    if app.tf_state_restored {
        return false;
    }
    let opts = startup_opts();
    apply_lib_dir(app, &opts);
    let loads = startup_loads(&app.tf_engine, &opts);
    if !loads.is_empty() || !opts.commands.is_empty() {
        clear_splash(app);
    }
    for load in loads {
        if tfrun::run_tf_line_console(app, &load, event_tx).await {
            return true;
        }
    }
    for cmd in startup_action_commands(app) {
        if !cmd.starts_with('/') {
            // Text to send to server - but we're not connected yet
            app.add_output(&format!("[Startup] Would send: {}", cmd));
            continue;
        }
        app.sync_tf_world_info();
        let effects = app.tf_engine.run(&cmd);
        let ctx = TfEffectCtx::for_world(app.current_world_index, false);
        if tfrun::run_tf_effects_console(app, effects, ctx, event_tx).await {
            return true;
        }
    }
    for cmd in opts.commands.iter() {
        if tfrun::run_tf_line_console(app, cmd, event_tx).await {
            return true;
        }
    }
    if let Some(connect) = startup_connect(&opts) {
        let placeholder = placeholder_world(app);
        if tfrun::run_tf_line_console(app, &connect, event_tx).await {
            return true;
        }
        carry_startup_messages(app, placeholder);
    }
    false
}

/// The same for the headless/GUI master and `-D` (whose Clay commands run through the
/// daemon's command path). `startup_actions`: whether this mode runs Clay's startup
/// actions (the GUI master does; -D never has). `tf_startup`: whether it loads the rc
/// and the rest (not for an embedded `--local-server`, which is someone else's app).
pub(crate) async fn run_tf_startup_daemon(app: &mut App, event_tx: &mpsc::Sender<AppEvent>, startup_actions: bool, tf_startup: bool) {
    let opts = if tf_startup { startup_opts() } else { TfStartupOpts { rc: RcChoice::None, ..Default::default() } };
    run_startup_daemon_with(app, event_tx, startup_actions, opts).await;
}

async fn run_startup_daemon_with(app: &mut App, event_tx: &mpsc::Sender<AppEvent>, startup_actions: bool, opts: TfStartupOpts) {
    if app.tf_state_restored {
        return;
    }
    apply_lib_dir(app, &opts);
    let loads = startup_loads(&app.tf_engine, &opts);
    if !loads.is_empty() || !opts.commands.is_empty() {
        clear_splash(app);
    }
    let mut lines: VecDeque<(String, bool)> = loads.into_iter().map(|l| (l, true)).collect();
    if startup_actions {
        for cmd in startup_action_commands(app) {
            if cmd.starts_with('/') {
                lines.push_back((cmd, false));
            } else {
                // Plain text can't be run at startup in headless mode
                crate::debug_log(crate::is_debug_enabled(), &format!("[Startup] Skipped command (headless): {}", cmd));
            }
        }
    }
    lines.extend(opts.commands.iter().cloned().map(|l| (l, true)));
    let connect = startup_connect(&opts);
    for (line, typed) in lines {
        let world_idx = app.current_world_index;
        let effects: Vec<TfEffect> = if typed {
            app.run_typed_tf_in_world(world_idx, &line)
        } else {
            app.sync_tf_world_info();
            app.tf_engine.run(&line)
        };
        tfrun::run_tf_effects_daemon(app, 0, effects, TfEffectCtx::for_world(world_idx, true), event_tx).await;
    }
    if let Some(connect) = connect {
        let placeholder = placeholder_world(app);
        let world_idx = app.current_world_index;
        let effects = app.run_typed_tf_in_world(world_idx, &connect);
        tfrun::run_tf_effects_daemon(app, 0, effects, TfEffectCtx::for_world(world_idx, true), event_tx).await;
        carry_startup_messages(app, placeholder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// TF's command line beside Clay's: values attached or (for -L/-c) separate, flag
    /// bundles, a world or a host and port; Clay's options pass through untouched.
    #[test]
    fn test_split_tf_cli() {
        let (opts, rest) = split_tf_cli(&strings(&["-L/opt/tf", "-fmy.rc", "-c/echo hi", "-nlq", "mymud", "--console"])).unwrap();
        assert_eq!(opts.lib_dir.as_deref(), Some("/opt/tf"));
        assert_eq!(opts.rc, RcChoice::File("my.rc".into()));
        assert_eq!(opts.commands, vec!["/echo hi"]);
        assert!(opts.no_login && opts.quiet && opts.given);
        assert_eq!(opts.world.as_deref(), Some("mymud"));
        assert_eq!(rest, strings(&["--console"]));

        let (opts, rest) = split_tf_cli(&strings(&["-L", "/opt/tf", "-c", "/set x=1", "-f", "host.example", "4000"])).unwrap();
        assert_eq!(opts.lib_dir.as_deref(), Some("/opt/tf"));
        assert_eq!(opts.commands, vec!["/set x=1"]);
        assert_eq!(opts.rc, RcChoice::None);
        assert_eq!(opts.host_port, Some(("host.example".into(), "4000".into())));
        assert!(rest.is_empty());

        // Clay's own: -v is the version, -h help, -D the daemon; --grep keeps its words.
        let (opts, rest) = split_tf_cli(&strings(&["-v", "-D", "--grep=h:9000", "-f", "pat"])).unwrap();
        assert!(!opts.given);
        assert_eq!(rest, strings(&["-v", "-D", "--grep=h:9000", "-f", "pat"]));
        // -v in a TF bundle is refused with the way to get TF's meaning.
        assert!(split_tf_cli(&strings(&["-nv"])).unwrap_err().contains("always visual"));
        // An unknown dash option is Clay's parser's to reject.
        assert_eq!(split_tf_cli(&strings(&["-z"])).unwrap().1, strings(&["-z"]));
        assert!(split_tf_cli(&strings(&["a", "b", "c"])).is_err());
        // A reload keeps only Clay's.
        assert_eq!(clay_args_only(strings(&["mymud", "-l", "--console", "-cfoo"])), strings(&["--console"]));
    }

    /// The rc search order (`tf(1)`): ~/.tfrc, ~/tfrc, ./.tfrc, ./tfrc; -f<file> names it,
    /// -f alone loads none.
    #[test]
    fn test_find_personal_rc() {
        let base = std::env::temp_dir().join(format!("clay_rc_search_{}", std::process::id()));
        let (home, cwd) = (base.join("home"), base.join("cwd"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        assert_eq!(find_personal_rc(&RcChoice::Search, &home, &cwd), None);
        std::fs::write(cwd.join("tfrc"), "").unwrap();
        assert_eq!(find_personal_rc(&RcChoice::Search, &home, &cwd), Some(cwd.join("tfrc")));
        std::fs::write(cwd.join(".tfrc"), "").unwrap();
        assert_eq!(find_personal_rc(&RcChoice::Search, &home, &cwd), Some(cwd.join(".tfrc")));
        std::fs::write(home.join("tfrc"), "").unwrap();
        assert_eq!(find_personal_rc(&RcChoice::Search, &home, &cwd), Some(home.join("tfrc")));
        std::fs::write(home.join(".tfrc"), "").unwrap();
        assert_eq!(find_personal_rc(&RcChoice::Search, &home, &cwd), Some(home.join(".tfrc")));
        assert_eq!(find_personal_rc(&RcChoice::None, &home, &cwd), None);
        assert_eq!(find_personal_rc(&RcChoice::File("x.rc".into()), &home, &cwd), Some(PathBuf::from("x.rc")));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A startup: the rc runs (its worlds defined, its variables set) and says so, then
    /// -c; a reload that carried the TF state over runs none of it.
    #[tokio::test]
    async fn test_startup_runs_the_rc_then_c() {
        let dir = std::env::temp_dir().join(format!("clay_startup_rc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rc = dir.join("my.tfrc");
        std::fs::write(&rc, "/addworld -Tlp w1 bob pw localhost 4000\n/set fromrc=1\n").unwrap();
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(crate::World::new("first"));
        app.current_world_index = 0;
        let (event_tx, _event_rx) = mpsc::channel::<AppEvent>(16);
        let opts = TfStartupOpts {
            rc: RcChoice::File(rc.display().to_string()),
            commands: vec!["/set fromc=$[fromrc+1]".into()],
            ..Default::default()
        };
        run_startup_daemon_with(&mut app, &event_tx, true, opts.clone()).await;
        let w1 = app.worlds.iter().find(|w| w.name == "w1").expect("the rc's world");
        assert_eq!(w1.settings.tf_type, "lp");
        assert_eq!(app.tf_engine.get_var("fromrc").map(|v| v.to_string_value()).as_deref(), Some("1"));
        // -c is typed input: %sub is off, so it is not expanded.
        assert_eq!(app.tf_engine.get_var("fromc").map(|v| v.to_string_value()).as_deref(), Some("$[fromrc+1]"));
        let shown: Vec<String> = app.worlds[0].output_lines.iter().map(|l| l.text.clone()).collect();
        assert!(shown.iter().any(|l| l == &format!("% Loading commands from {}.", rc.display())), "{shown:?}");

        let mut reloaded = App::new();
        reloaded.tf_state_restored = true;
        run_startup_daemon_with(&mut reloaded, &event_tx, true, opts).await;
        assert!(reloaded.tf_engine.get_var("fromrc").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The startup connect carries -l/-q.
    #[test]
    fn test_startup_connect() {
        let opts = TfStartupOpts { world: Some("mud".into()), no_login: true, ..Default::default() };
        assert_eq!(startup_connect(&opts).as_deref(), Some("/connect -l mud"));
        let opts = TfStartupOpts { host_port: Some(("h".into(), "23".into())), quiet: true, ..Default::default() };
        assert_eq!(startup_connect(&opts).as_deref(), Some("/connect -q h 23"));
        assert_eq!(startup_connect(&TfStartupOpts::default()), None);
    }
}
