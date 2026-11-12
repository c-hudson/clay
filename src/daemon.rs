use std::io;
// The multiuser first-run wizard prompts on the terminal.
use std::sync::Arc;
use std::time::Duration;

use tokio::{
    net::TcpStream,
    sync::mpsc,
};

use crate::*;
use crate::{
    App, WorldSettings,
    ClientViewState, Command,
    enable_tcp_keepalive, current_timestamp_secs,
};
#[cfg(test)]
use crate::OutputLine;
use crate::actions::{action_commands_to_run,
    find_invocable_action, rewrite_slashless_action};
use crate::telnet_reader::{spawn_telnet_reader, TelnetTarget};
use crate::telnet_writer::spawn_telnet_writer;
use crate::commands::{execute_send_command, execute_log_command,
    execute_disconnect_command,
    execute_remove_world_command, prepare_world_connect_host_port,
    execute_mssp_command, execute_msdp_command, execute_stats_command,
    execute_send_gmcp, execute_send_msdp};
// Only used by the #[cfg(test)] helpers below (RemoteConsole's own non-test use moved into
// the now-shared App::handle_cycle_world in main.rs - T39).

/// Run headlessly as a local, loopback-only Clay instance for an embedding client — currently
/// the Android app's bundled server, spawned as a child process. No TUI, no native WebView; the
/// embedding client (e.g. the Android WebView) connects over ws://127.0.0.1:<port> using the
/// password supplied via the CLAY_WS_PASSWORD environment variable - for this run only, never
/// written to settings.dat (`WsOverride::Session`): the app keeps it and passes it at every
/// start. This mirrors desktop GUI master mode (`webview_gui::run_master_webgui` ->
/// `run_app_headless`) minus opening a window — the caller supplies its own client instead
/// of a native WebView.
pub async fn run_local_server(port_override: Option<u16>) -> io::Result<()> {
    let password = match std::env::var("CLAY_WS_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("clay: --local-server requires a non-empty CLAY_WS_PASSWORD environment variable.");
            std::process::exit(1);
        }
    };

    let lan = std::env::var("CLAY_LOCAL_SERVER_LAN").map(|v| v == "1").unwrap_or(false);
    if lan {
        println!("clay: starting local server (other devices allowed)...");
    } else {
        println!("clay: starting local server (loopback only)...");
    }
    crate::LOCAL_SERVER_LOOPBACK_ONLY.store(true, std::sync::atomic::Ordering::SeqCst);
    crate::LOCAL_SERVER_LAN.store(lan, std::sync::atomic::Ordering::SeqCst);

    // These channels satisfy run_app_headless's GUI-bridge API; nothing reads/writes them here
    // since the embedding client talks to the App over the WebSocket server, not this channel.
    let (gui_tx, gui_rx) = mpsc::unbounded_channel::<WsMessage>();
    // Dropped at once: the App skips the clone of each broadcast once the receiver is
    // gone (`ws_broadcast`); kept, every message queued here for the life of the process.
    drop(gui_rx);
    let (_gui_to_app_tx, gui_to_app_rx) = mpsc::unbounded_channel::<WsMessage>();

    crate::run_app_headless(gui_tx, gui_to_app_rx, Some(crate::WsOverride::Session(password)), None, port_override).await
}

/// `clay -D`: a headless Clay on the configured web port, for every interface to attach
/// to. The setup and the loop are `apploop`'s, shared with the GUI masters and the server
/// builds' instances (this used to be a copy that had drifted: no reload state, no GA/EOR
/// prompts, no FANSI login timeout, no timed prompts).
pub async fn run_daemon_server() -> io::Result<()> {
    let prepared = crate::apploop::prepare_app(crate::apploop::SetupOptions {
        load_reload_state: true,
        ws_override: None,
        port_override: None,
        web_server: crate::apploop::WebServer::Daemon,
        template: None,
        signals: true,
        gui_tx: None,
        gui_repaint: None,
        ws_server: None,
    }).await?;
    let crate::apploop::Prepared { mut app, event_tx, event_rx, .. } = prepared;

    // Check if any servers are running
    if app.https_server.is_none() {
        eprintln!("Error: No servers started. Enable HTTP in settings.");
        eprintln!("Use /web command to configure, or edit ~/.clay/settings.dat");
        return Ok(());
    }

    // Show allow list if configured (helps debug connection rejections)
    if !app.settings.websocket_allow_list.is_empty() {
        println!("Allow list: {}", app.settings.websocket_allow_list);
    }

    // TinyFugue's startup: the rc, -c, and the world named on the command line (stdout is
    // fine here - -D draws no TUI).
    crate::apploop::tf_startup(&mut app, &event_tx, crate::apploop::TfStartup::Daemon).await;

    println!("Daemon running. Press Ctrl+C to stop.");

    let exit = crate::apploop::run_app_loop(&mut app, event_rx, event_tx, crate::apploop::LoopOptions {
        gui_rx: None,
        gui_quit: false,
        cmd_rx: None,
        system_message: crate::apploop::SystemMessageSink::Stdout,
        hot_reload: true,
    }).await?;
    if let crate::apploop::LoopExit::Signal(event) = exit {
        crate::crashlog::exit_reason(if event == crate::tf::TfHookEvent::Sighup { "SIGHUP" } else { "SIGTERM" });
    }
    Ok(())
}

/// Start a shell or chat world for a client - the kinds that connect without a MUD
/// socket - and say so in the world. Returns false for a MUD world, which the caller
/// connects as it did. Every way a client connects a world through this handler goes
/// here first: `/worlds <name>` (what TF's `/connect` becomes), `/worlds -b` and the
/// world switch used to open a MUD connection to a shell world's empty address on a
/// `-D` or GUI-master host ("Connecting to :..." then "no host and port set"), while the
/// console's path (commands.rs) knew the world's type.
fn start_non_mud_world(app: &mut App, idx: usize, event_tx: &mpsc::Sender<AppEvent>) -> bool {
    let ty = app.worlds[idx].settings.world_type.clone();
    if ty.is_shell() {
        // A local terminal (crate::shell); reports back via AppEvent::Shell. A build
        // without shell worlds says so instead (`spawn_shell_connect`).
        if cfg!(feature = "shell-worlds") {
            app.emit_client_text(idx, "Starting shell...", true);
        }
        app.spawn_shell_connect(idx);
        true
    } else if !ty.is_mud() {
        // Slack/Discord worlds run a background chat session instead of
        // connect_daemon_world (MUD-only) - see crate::chat.
        if app.worlds[idx].settings.has_connection_settings() {
            app.emit_client_text(idx, "Connecting...", true);
            app.spawn_chat_connect(idx, event_tx);
        } else {
            app.emit_client_text(idx, "Set the bot token first (world editor).", true);
        }
        true
    } else {
        false
    }
}

/// Handle WebSocket message in daemon mode. Thin wrapper around
/// `handle_daemon_ws_message_impl` that also refreshes `tf_bound_keys_json` afterward
/// (TF-parity plan Job 22a/P2.7 - see `App::refresh_tf_bound_keys_if_changed`'s own doc
/// comment). This function is also used outside `-D`/`--multiuser` proper: main.rs's
/// GUI/headless run loops (`run_master_webgui`, `run_app_headless`) dispatch every WS
/// message through it too, so this one wrapper covers every single-user WS/GUI/web client
/// path that isn't the interactive console's own `handle_ws_send_command`/
/// `apply_pending_tf_console_ops` (main.rs/commands.rs).
pub async fn handle_daemon_ws_message(
    app: &mut App,
    client_id: u64,
    msg: WsMessage,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    // A client's command or key binding: TF's keyboard functions see the client's input line
    // (empty once a line is sent; a binding brings its own), and what they leave goes back
    // to that client - never to anyone's console.
    // A view the last message moved off the Activity world empties it before this one can
    // move a view back onto it (App::note_activity_viewers).
    app.note_activity_viewers();
    let clients_own = matches!(msg, WsMessage::SendCommand { .. } | WsMessage::RunKeyBinding { .. });
    let saved_keyboard = if clients_own { app.client_keyboard.replace(crate::tf::KeyboardBufferState::default()) } else { app.client_keyboard.clone() };
    if clients_own {
        // Left by a trigger or a timer: there is no console here to take them, and they
        // are not this client's.
        app.tf_engine.pending_keyboard_ops.clear();
        app.tf_engine.pending_xtitle = None;
        app.tf_engine.pending_limit_op = None;
    }
    handle_daemon_client_message(app, client_id, msg, event_tx).await;
    if clients_own {
        app.send_client_tf_ops(client_id);
    }
    app.client_keyboard = saved_keyboard;
    app.refresh_tf_bound_keys_if_changed();
}

async fn handle_daemon_client_message(
    app: &mut App,
    client_id: u64,
    msg: WsMessage,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    // A Clay that parses no commands (a clay_web visitor's): every typed line is text for
    // its one world, `/` or not, and only the view's own messages are answered.
    if !app.policy.parse_commands {
        if let WsMessage::SendCommand { command, .. } = msg {
            app.auto_resume_if_user_action(client_id, &WsMessage::SendCommand { world_index: 0, command: command.clone() });
            if !app.worlds.is_empty() {
                app.send_to_world_and_mark_sent(0, command);
            }
            return;
        }
        if let Some(why) = app.policy.message_refusal(&msg) {
            app.ws_tell(client_id, 0, why);
            return;
        }
        handle_daemon_ws_message_impl(app, client_id, msg, event_tx).await;
        return;
    }
    // A line the client typed that TF takes first, as at the console (App::client_line_for_tf).
    if let WsMessage::SendCommand { world_index, ref command } = msg {
        if let Some(line) = app.client_line_for_tf(world_index, command) {
            app.auto_resume_if_user_action(client_id, &msg);
            app.tf_echo_line(world_index, "kecho", "kprefix", command);
            let effects = app.run_typed_tf_in_world(world_index, &line);
            let ctx = crate::tfrun::TfEffectCtx::for_world(world_index, true);
            crate::tfrun::run_tf_effects_daemon(app, client_id, effects, ctx, event_tx).await;
            return;
        }
    }
    handle_daemon_ws_message_impl(app, client_id, msg, event_tx).await;
}

pub(crate) async fn handle_daemon_ws_message_impl(
    app: &mut App,
    client_id: u64,
    msg: WsMessage,
    event_tx: &mpsc::Sender<AppEvent>,
) {
    // Log every incoming command for diagnostics
    if let WsMessage::SendCommand { ref command, .. } = msg {
        crate::debug_log(is_debug_enabled(), &format!("DAEMON_CMD: client_id={} command={:?}", client_id, command));
    }
    // Auto-resume a paused session when the user interacts (mirrors "exit more mode by
    // acting") - was previously master-WS-only despite daemon sharing the same per-client
    // pause tracking (T38).
    app.auto_resume_if_user_action(client_id, &msg);
    match msg {
        // TF-parity plan Job 22a/P2.7: same handling as the master WS's own
        // `App::handle_ws_client_msg` arm (main.rs) - resolve `key` the same way the
        // console does (`chords::resolve_bound_command`), then run the result exactly like
        // a typed `SendCommand` by recursing into this same function (mirrors the existing
        // `WorldConnectHostPort` followup recursion further down). This is what makes
        // `RunKeyBinding` work for GUI/headless single-user modes too, not just the
        // interactive TUI console - `run_master_webgui`/`run_app_headless` dispatch every
        // WS message through `handle_daemon_ws_message`.
        WsMessage::RunKeyBinding { key, kbnum, input } => {
            if let Some(cmd) = crate::chords::resolve_bound_command(app, &key) {
                // The client's input line, for the binding's kb* functions; what they do to
                // it goes back to the client (handle_daemon_ws_message: send_client_tf_ops).
                app.client_keyboard = Some(input.map(crate::websocket::InputSnapshot::keyboard_state).unwrap_or_default());
                let world_index = app.ws_client_worlds.get(&client_id)
                    .map(|s| s.world_index)
                    .unwrap_or(app.current_world_index);
                // See main.rs's identical `RunKeyBinding` handler for why this swaps
                // through `app.input.kbnum` rather than writing the TF `%kbnum` global
                // directly - `sync_tf_world_info()` (called by the recursive `SendCommand`
                // dispatch below) unconditionally overwrites it from there.
                let prev_kbnum = app.input.kbnum;
                app.input.kbnum = kbnum;
                // A binding runs as a macro body (TF's rule for /bind), not as a typed line.
                let effects = app.run_tf_body_in_world(world_index, &cmd);
                let ctx = crate::tfrun::TfEffectCtx::for_world(world_index, true);
                Box::pin(crate::tfrun::run_tf_effects_daemon(app, client_id, effects, ctx, event_tx)).await;
                app.input.kbnum = prev_kbnum;
                app.tf_engine.unset_global("kbnum");
            }
            // Unknown key (stale client-side cache, or a race with an /unbind) - ignore
            // silently, per the plan's own ruling.
        }
        WsMessage::SendCommand { world_index, command } => {
            // A live shell world: a line of text (not a Clay command) is typed at the shell,
            // as the console master's handler does. A "/..." command falls through to
            // normal handling, so /dc and friends still reach it.
            if app.shell_live(world_index) && !command.starts_with('/') {
                app.shell_input(world_index, None, Some(format!("{command}\r")), false);
                return;
            }
            // TF's leading-slash forms (never at a password prompt): "//x" is "/x", "///x"
            // sends "/x" as text. ("^old^new" needs the client's own history, so the client
            // resolves it before sending.)
            let masked = app.worlds.get(world_index).is_some_and(|w| w.protocol.echo_masked);
            app.tf_echo_line(world_index, "kecho", "kprefix", &command);
            let command = match crate::tfrun::typed_slash_form(&command) {
                crate::tfrun::TypedSlashForm::SendText(text) if !masked => {
                    if world_index < app.worlds.len() && app.send_to_world(world_index, text.to_string()) {
                        app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                        app.worlds[world_index].clear_prompt_after_send();
                    }
                    return;
                }
                crate::tfrun::TypedSlashForm::Line(line) if !masked => line.to_string(),
                _ => command,
            };
            // Determine the current world name for action world-scoping.
            let world_name = app.worlds.get(world_index).map(|w| w.name.clone()).unwrap_or_default();
            // Rewrite slash-less action invocations: "common" → "/common"
            // Only rewrites if an action eligible for the current world exists.
            let command = rewrite_slashless_action(&command, &app.settings.actions, &world_name)
                .unwrap_or(command);
            // Use shared command parsing (same as console mode)
            let parsed = crate::parse_typed_line(&command);

            // Reset more-mode counter when user sends a command - except /dump, which is
            // passive, as at the console.
            if world_index < app.worlds.len() && !crate::is_dump_command(&command) {
                app.worlds[world_index].lines_since_pause = 0;
                app.worlds[world_index].last_user_command_time = Some(std::time::Instant::now());
                // Also clear paused flag if no pending lines
                if app.worlds[world_index].pending_lines.is_empty() {
                    app.worlds[world_index].paused = false;
                }
            }

            match parsed {
                Command::ActionCommand { name, args } => {
                    // Execute action if it exists (respects the action's world field).
                    if let Some(action) = find_invocable_action(&app.settings.actions, &name, &world_name) {
                        if !action.enabled {
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: format!("Action '{}' is disabled.", action.name),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });
                        } else {
                            let mut sent_to_server = false;
                            for cmd in action_commands_to_run(action, &args) {
                                // Unified command system - route through the TF parser and
                                // apply what each command did, in order (see tfrun).
                                if cmd.starts_with('/') {
                                    app.sync_tf_world_info();
                                    let effects = app.run_tf_in_world(world_index, &cmd);
                                    if effects.iter().any(|e| matches!(e, crate::tf::effects::TfEffect::Send { .. })) {
                                        sent_to_server = true;
                                    }
                                    let ctx = crate::tfrun::TfEffectCtx::for_world(world_index, true);
                                    Box::pin(crate::tfrun::run_tf_effects_daemon(app, client_id, effects, ctx, event_tx)).await;
                                } else {
                                    // Plain text - send to MUD server (captured via
                                    // send_to_world too).
                                    if app.send_to_world(world_index, cmd) {
                                        sent_to_server = true;
                                    }
                                }
                            }
                            if sent_to_server && world_index < app.worlds.len() {
                                app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                            }
                        }
                    } else {
                        // No matching action - try TF engine (handles /recall, /set, /echo,
                        // etc.). A command TF doesn't know either comes back as a Clay command,
                        // which tfrun reports as unknown rather than bouncing it around.
                        let effects = app.run_typed_tf_in_world(world_index, &command);
                        let ctx = crate::tfrun::TfEffectCtx::for_world(world_index, true);
                        Box::pin(crate::tfrun::run_tf_effects_daemon(app, client_id, effects, ctx, event_tx)).await;
                    }
                }
                Command::NotACommand { text } if app.tf_engine.sub_mode() != crate::tf::SubMode::Off => {
                    // With %sub on or full, typed plain text is TF's to process (`/help sub`).
                    let effects = app.run_typed_tf_in_world(world_index, &text);
                    let ctx = crate::tfrun::TfEffectCtx::for_world(world_index, true);
                    Box::pin(crate::tfrun::run_tf_effects_daemon(app, client_id, effects, ctx, event_tx)).await;
                }
                Command::NotACommand { text } => {
                    // SEND hook: typed plain text about to go to the MUD. A matching
                    // non-quiet hook means the original text is NOT sent - see /help
                    // hooks' SEND rule and finding C.10 / plan step P1.9 (this is the
                    // "headless/daemon" half of the same shared-helper wiring the
                    // console loop does in main.rs).
                    if world_index < app.worlds.len() {
                        let suppressed = app.fire_tf_hook(Some(world_index), tf::TfHookEvent::Send, &text, true);
                        if !suppressed {
                            if app.send_to_world(world_index, text) {
                                app.worlds[world_index].last_send_time = Some(std::time::Instant::now());
                                app.worlds[world_index].clear_prompt_after_send();
                            } else if !app.worlds[world_index].connected {
                                // As at the console, to the client that typed it.
                                app.ws_tell(client_id, world_index, "Not connected. Use /worlds to connect.");
                            }
                        }
                    }
                }
                // `/note <file>`: the file is on this machine (user ruling 2026-10-16).
                Command::Edit { filename: Some(ref path) } => app.ws_open_file_for_client(client_id, world_index, path),
                Command::Edit { .. } | Command::EditList => {
                    // The notes editor and list are the client's own windows: send it
                    // back for the client to open.
                    app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                }
                Command::Tag => {
                    // Toggle MUD tag display (same as F2) - silent, no output
                    app.show_tags = !app.show_tags;
                    // Broadcast to all clients
                    app.ws_broadcast(WsMessage::ShowTagsChanged { show_tags: app.show_tags });
                }
                Command::Dict { .. } | Command::Urban { .. } | Command::Translate { .. } | Command::TinyUrl { .. } => {
                    spawn_api_lookup(event_tx.clone(), client_id, world_index, parsed, app.settings.url_shorteners.clone());
                }
                Command::DictUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /dict <word>",
                        "  Looks up <word> in the dictionary and places the definition in the input buffer.",
                        "  Example: /dict hello",
                    ], true);
                }
                Command::UrbanUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /urban <word>",
                        "  Looks up <word> in Urban Dictionary and places the definition in the input buffer.",
                        "  Example: /urban yeet",
                    ], true);
                }
                Command::TranslateUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /translate <lang> <text>",
                        "  Translates <text> to <lang> and places the result in the input buffer.",
                        "  <lang> can be a code (es, fr, de) or name (spanish, french, german).",
                        "  Example: /translate spanish Hello, how are you?",
                        "  Example: /tr es Hello",
                    ], true);
                }
                Command::TinyUrlUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /url <url>",
                        "  Shortens <url> and places the result in the input buffer.",
                        "  Example: /url https://github.com/c-hudson/clay",
                    ], true);
                }
                // What the console's typed `/help <topic>` shows, to the client that asked
                // only (it used to go into the world's scrollback for every client).
                Command::HelpTopic { ref raw, .. } => app.handle_ws_help_via_tf(client_id, world_index, raw),
                Command::Help => app.handle_ws_help_via_tf(client_id, world_index, ""),
                Command::Unknown { cmd } => {
                    // NOMACRO hook - see /help hooks and finding C.10 / plan step P1.9.
                    app.fire_tf_hook(Some(world_index), tf::TfHookEvent::Nomacro, &cmd, true);
                    app.emit_client_text(world_index, &format!("Unknown command: {}", cmd), true);
                }
                Command::Send { text, all_worlds, target_world, world_type, no_newline, run_hook } => {
                    execute_send_command(app, &text, all_worlds, &target_world, &world_type, no_newline, run_hook, world_index, true);
                }
                Command::Log { world, log_input, log_local: _, log_global: _, action } => {
                    execute_log_command(app, &world, log_input, &action, world_index, true);
                }
                Command::Disconnect { world } => {
                    // Uses the same World::clear_connection_state the console/master-WS paths
                    // use (via execute_disconnect_command), instead of an inline field-by-field
                    // copy - see the removed comment's own history: the old inline version here
                    // was missing several fields (skip_auto_login, negotiated_encoding,
                    // telnet_mode, naws_enabled/naws_sent_size, fansi_detect_until/
                    // fansi_login_pending, active_media, timing fields), leaking stale state
                    // into the next connection attempt on this world in daemon mode.
                    execute_disconnect_command(app, &world, world_index, true);
                }
                Command::Chat { world, args } => {
                    crate::commands::execute_chat_command(app, &world, &args, world_index, true);
                }
                Command::Flush => {
                    if world_index < app.worlds.len() {
                        let line_count = app.worlds[world_index].output_lines.len();
                        app.worlds[world_index].output_lines.clear();
                        app.worlds[world_index].pending_lines.clear();
                        app.worlds[world_index].scroll_offset = 0;
                        app.worlds[world_index].lines_since_pause = 0;
                        app.worlds[world_index].paused = false;
                        app.ws_broadcast(WsMessage::WorldFlushed { world_index });
                        app.emit_client_text(world_index, &format!("Flushed {} lines from output buffer.", line_count), true);
                    }
                }
                Command::Remote => {
                    spawn_remote_ping_check(app, event_tx.clone(), client_id, world_index);
                }
                Command::RemoteKill { client_id } => {
                    let msg = if let Some(ref ws_server) = app.ws_server {
                        let ip = {
                            let clients = ws_server.clients.read().unwrap();
                            clients.get(&client_id).map(|c| c.ip_address.clone())
                        };
                        if let Some(ip) = ip {
                            let mut clients_mut = ws_server.clients.write().unwrap();
                            clients_mut.remove(&client_id);
                            format!("Disconnected remote client {} ({})", client_id, ip)
                        } else {
                            format!("No client with ID {}.", client_id)
                        }
                    } else {
                        "WebSocket server is not running.".to_string()
                    };
                    app.emit_client_text(world_index, &msg, true);
                }
                Command::RemotePause { client_id: pause_id } => {
                    let msg = match app.ws_toggle_client_paused(pause_id) {
                        Some((new_paused, ip)) => {
                            app.ws_send_to_client(pause_id, WsMessage::PausedState { paused: new_paused });
                            app.broadcast_activity();
                            if new_paused {
                                format!("Paused remote client {} ({}) — activity now visible on other sessions.", pause_id, ip)
                            } else {
                                format!("Resumed remote client {} ({}).", pause_id, ip)
                            }
                        }
                        None => format!("No client with ID {}.", pause_id),
                    };
                    app.emit_client_text(world_index, &msg, true);
                }
                Command::RemotePauseConsole => {
                    let msg = app.toggle_console_pause();
                    app.emit_client_text(world_index, &msg, true);
                }
                Command::Pause => {
                    let msg = app.pause_other_interfaces(Some(client_id));
                    app.emit_client_text(world_index, &msg, true);
                }
                Command::BanList => app.ws_ban_list(client_id, world_index),
                Command::Unban { host } => app.ws_unban(client_id, world_index, host),
                Command::TestMusic => {
                    let test_notes = crate::generate_test_music_notes();
                    crate::debug_log(is_debug_enabled(), &format!("TESTMUSIC: daemon path, client_id={}, notes={}", client_id, test_notes.len()));
                    app.ws_send_to_client(client_id, WsMessage::AnsiMusic {
                        world_index,
                        notes: test_notes,
                    });
                    app.ws_send_to_client(client_id, WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: "Playing test music (Super Mario Bros)...".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                }
                Command::Notify { message } => {
                    let title = if world_index < app.worlds.len() {
                        app.worlds[world_index].name.clone()
                    } else {
                        "Clay".to_string()
                    };
                    app.ws_broadcast(WsMessage::Notification {
                        title,
                        message: message.clone(),
                    });
                    app.emit_client_text(world_index, &format!("Notification sent: {}", message), true);
                }
                Command::Say { text } => app.ws_say(client_id, world_index, &text),
                Command::Dump { args } => app.ws_dump(client_id, world_index, "daemon", &args),
                Command::Mssp => {
                    execute_mssp_command(app, world_index, true);
                }
                Command::Info { world } => app.ws_info_text(client_id, world_index, world.as_deref()),
                Command::Stats { show } => {
                    execute_stats_command(app, world_index, show, true);
                }
                Command::Msdp { verb, target } => {
                    execute_msdp_command(app, world_index, &verb, target.as_deref(), true);
                }
                Command::MsdpUsage => {
                    app.emit_usage(world_index, &[
                        "Usage: /msdp <LIST|REPORT|UNREPORT|SEND> [target]",
                        "  LIST targets: COMMANDS, LISTS, CONFIGURABLE_VARIABLES,",
                        "                REPORTABLE_VARIABLES, SENDABLE_VARIABLES, REPORTED_VARIABLES",
                        "  Example: /msdp LIST REPORTABLE_VARIABLES",
                        "  Example: /msdp REPORT HEALTH",
                    ], true);
                }
                Command::Reload { ui, bad } => {
                    // `--console` and `--gui` switch the main Clay only from its own window
                    // (App::reload_switch_refusal), and only to an interface that can run here.
                    if let Some(why) = app.client_reload_refusal(client_id, ui, &bad, true) {
                        app.ws_tell(client_id, world_index, &why);
                    } else {
                        // The Android app's server can't hot reload: say so rather than nothing.
                        #[cfg(target_os = "android")]
                        app.ws_tell(client_id, world_index, "Hot reload is not available on this platform. Restart the app to apply changes.");
                        // Signal the event loop to perform reload
                        #[cfg(not(target_os = "android"))]
                        {
                            app.reload_ui = ui;
                            crate::debug_log(is_debug_enabled(), "DAEMON: Received /reload command, sending Sigusr1Received event");
                            let _ = event_tx.send(AppEvent::Sigusr1Received).await;
                        }
                    }
                }
                // The WebView GUI master's own window relaunches; a headless daemon (-D), or
                // any other client, is refused (App::ws_remote_attach).
                Command::RemoteAttach { addr, close, cancel } => app.ws_remote_attach(client_id, world_index, &addr, close, cancel),
                // An `/import` that reached the server (typed ones open the dialog
                // client-side): the client's Import dialog, where the password is typed.
                Command::Import { addr } => app.ws_send_to_client(client_id, WsMessage::OpenImport { addr }),
                // Commands that execute locally on the client
                Command::Quit => {
                    app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                }
                // The server's half of a client's /update (src/selfupdate.rs); a
                // multiuser server refuses inside start_server_update.
                Command::Update { force } => app.start_server_update(force, Some((client_id, world_index)), event_tx),
                Command::Tts { args } => app.tts_command(&args, Some((client_id, world_index)), event_tx),
                Command::Menu => app.ws_menu_is_console_only(client_id, world_index),
                Command::Split { .. } | Command::Unsplit => app.ws_split_is_the_clients(client_id, world_index, &command),
                Command::Map { args } => app.ws_map_command(client_id, world_index, &args, &command),
                // UI popup commands - send back to client for local handling
                Command::Font | Command::Setup | Command::Web | Command::Actions { .. } |
                Command::WorldsList | Command::WorldSelector | Command::WorldEdit { .. } => {
                    app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: command.clone() });
                }
                Command::Window { world } => {
                    // The client opens the window; a named world that isn't connected is
                    // connected first, as the console master's handler does.
                    let connect = app.window_world_to_connect(world.as_deref());
                    app.ws_send_to_client(client_id, WsMessage::OpenWindow { world });
                    if let Some(idx) = connect {
                        if !start_non_mud_world(app, idx, event_tx) {
                            let settings = app.worlds[idx].settings.clone();
                            let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                            app.emit_client_text(idx, &format!("Connecting to {}:{}{}...", settings.hostname, settings.port, ssl_msg), true);
                            app.spawn_world_connect(idx, crate::ConnectOrigin::Client { report_failure: true, client_id }, event_tx);
                        }
                    }
                }
                Command::Version => {
                    app.ws_send_to_client(client_id, WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: get_version_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                }
                Command::Reach { refresh, lookup } => app.ws_reach(client_id, world_index, refresh, lookup, event_tx),
                // AddWorld - add or update world definition
                Command::RemoveWorld { names } => {
                    execute_remove_world_command(app, &names, world_index, true);
                }
                Command::WorldConnectHostPort { host, port, use_ssl, no_login, background } => {
                    match prepare_world_connect_host_port(app, &host, &port, use_ssl, no_login, background) {
                        Ok(followup) => return Box::pin(handle_daemon_ws_message(
                            app, client_id, WsMessage::SendCommand { world_index, command: followup }, event_tx,
                        )).await,
                        Err(e) => app.emit_client_text(world_index, &e, true),
                    }
                }
                // Connect command - use daemon connection logic
                Command::Connect { .. } => {
                    if world_index < app.worlds.len() && !app.worlds[world_index].connected {
                        if start_non_mud_world(app, world_index, event_tx) {
                            return;
                        }
                        if app.worlds[world_index].settings.has_connection_settings() {
                            let settings = app.worlds[world_index].settings.clone();

                            let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: format!("Connecting to {}:{}{}...\n", settings.hostname, settings.port, ssl_msg),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });

                            // In the background: awaiting the connect here froze this whole loop (and
                            // every client) until it finished - see App::spawn_world_connect.
                            app.spawn_world_connect(world_index, crate::ConnectOrigin::Client { report_failure: true, client_id }, event_tx);
                        } else {
                            app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                                world_index,
                                data: "No connection settings configured for this world.".to_string(),
                                is_viewed: false,
                                ts: current_timestamp_secs(),
                                from_server: false,
                                seq: 0, end_seq: None,
                                flush: false, gagged: false, highlight_colors: Vec::new(),
                            });
                        }
                    } else if world_index < app.worlds.len() {
                        // As the console says it, to the client that asked.
                        app.ws_tell(client_id, world_index, "Already connected. Use /disconnect first.");
                    }
                }
                Command::WorldConnectBackground { ref name, no_login } => {
                    if let Some(idx) = app.worlds.iter().position(|w| w.name.eq_ignore_ascii_case(name)) {
                        // The console's answers, to the client that asked.
                        if let Some(why) = app.background_connect_refusal(idx, name) {
                            app.ws_tell(client_id, world_index, &why);
                        } else if !start_non_mud_world(app, idx, event_tx) {
                            // In the background - see App::spawn_world_connect. `/worlds -b` never
                            // reported a failure, so it still doesn't.
                            // `/worlds -b` logs in unless -l (handle_connection_success reads the flag).
                            app.worlds[idx].skip_auto_login = no_login;
                            app.spawn_world_connect(idx, crate::ConnectOrigin::Client { report_failure: false, client_id }, event_tx);
                        }
                    } else {
                        app.emit_client_text(world_index, &format!("World '{}' not found.", name), true);
                    }
                }
                Command::WorldSwitch { ref name } | Command::WorldConnectNoLogin { ref name } => {
                    if let Some(idx) = app.find_world_typed(name) {
                        // The server's foreground world follows (TF's WORLD hook, media,
                        // Esc-w's history), as a console's would; with no console it claims
                        // no ▶ there (`switch_world`). The requesting client's view moves,
                        // and it alone is told - as the console master does (handler A);
                        // broadcasting put every attached SSH console onto this world too.
                        // Its own `MarkWorldSeen` then claims the lines for it (deferred bug
                        // G1: `switch_world`'s console claim used to take them first).
                        app.switch_world(idx);
                        app.ws_client_view_world(client_id, idx);
                        app.ws_send_to_client(client_id, WsMessage::WorldSwitched { new_index: idx });
                        // Also send ExecuteLocalCommand so web clients can switch their local
                        // view: as `/world <name>`, the one form every client reads (TF's
                        // flags, `/world -q foo`, made them look for a world named "-q foo").
                        app.ws_send_to_client(client_id, WsMessage::ExecuteLocalCommand { command: format!("/world {}", app.worlds[idx].name) });
                        // Connect if not connected and has settings
                        if !app.worlds[idx].connected
                            && app.worlds[idx].settings.has_connection_settings()
                            && !start_non_mud_world(app, idx, event_tx)
                        {
                            if matches!(parsed, Command::WorldConnectNoLogin { .. }) {
                                app.worlds[idx].skip_auto_login = true;
                            }
                            let settings = app.worlds[idx].settings.clone();

                            let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                            app.emit_client_text(idx, &format!("Connecting to {}:{}{}...", settings.hostname, settings.port, ssl_msg), true);

                            // In the background: awaiting the connect here froze this whole loop (and
                            // every client) until it finished - see App::spawn_world_connect.
                            app.spawn_world_connect(idx, crate::ConnectOrigin::Client { report_failure: true, client_id }, event_tx);
                        }
                    } else {
                        app.emit_client_text(world_index, &format!("World '{}' not found.", name), true);
                    }
                }
            }
        }
        WsMessage::ConnectWorld { world_index } => {
            if world_index < app.worlds.len() && !app.worlds[world_index].connected {
                if start_non_mud_world(app, world_index, event_tx) {
                    return;
                }
                let settings = app.worlds[world_index].settings.clone();

                // Check if world has connection settings
                if !settings.has_connection_settings() {
                    app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                        world_index,
                        data: "Configure host/port in world settings.\n".to_string(),
                        is_viewed: false,
                        ts: current_timestamp_secs(),
                        from_server: false,
                        seq: 0, end_seq: None,
                        flush: false, gagged: false, highlight_colors: Vec::new(),
                    });
                    return;
                }

                let ssl_msg = if settings.use_ssl { " with SSL" } else { "" };
                app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                    world_index,
                    data: format!("Connecting to {}:{}{}...\n", settings.hostname, settings.port, ssl_msg),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0, end_seq: None,
                    flush: false, gagged: false, highlight_colors: Vec::new(),
                });

                // Attempt connection
                // In the background: awaiting the connect here froze this whole loop (and
                // every client) until it finished - see App::spawn_world_connect.
                app.spawn_world_connect(world_index, crate::ConnectOrigin::Client { report_failure: true, client_id }, event_tx);
            }
        }
        WsMessage::DisconnectWorld { world_index } => {
            // The same disconnect as /dc: this used to drop only our sender, leaving the
            // socket open and the server's text still arriving.
            if world_index < app.worlds.len() && app.worlds[world_index].connected {
                crate::commands::disconnect_world(app, world_index, true);
            }
        }
        // Was entirely absent from this handler (T40) - a daemon-mode client that hit a TLS
        // pin mismatch had no way to trust the new certificate and reconnect; the "Trust new
        // certificate" button in the cert-mismatch dialog silently did nothing. Re-pin, then
        // reconnect using the same inline pattern ConnectWorld above uses (daemon.rs doesn't
        // have a shared "connect this world" helper to call here - see Phase C's ConnectWorld
        // consolidation note for that separate, larger duplication).
        // Remote Access panel (reach.rs). Replies go to nobody in particular
        // (`None`): the dialog re-renders from the reachability broadcast, and an
        // output line would land in whichever world the server happens to show.
        WsMessage::SetPortMapping { enabled } => {
            app.set_port_map_enabled(enabled, "web");
        }
        WsMessage::FirewallRule { action } => {
            if action == "add" {
                app.request_firewall_rule(None, event_tx.clone());
            }
        }
        WsMessage::LookupPublicIp => {
            app.request_public_ip_lookup(None, event_tx.clone());
        }
        WsMessage::ChatLookup { request_id, world_index, world_type, token, app_token, server } => {
            app.spawn_chat_lookup(client_id, request_id, world_index, &world_type, token, app_token, server, event_tx);
        }
        WsMessage::RefreshReachability => {
            app.request_reachability_refresh(None, event_tx.clone());
        }
        WsMessage::TrustCertificate { world_index, host, new_fingerprint } => {
            if world_index < app.worlds.len() && app.can_trust_certificate(client_id, world_index) {
                persistence::replace_pin(&host, &new_fingerprint);
                app.ws_broadcast(WsMessage::ServerData { archive_sourced: false,
                    world_index,
                    data: format!("Trusting new certificate for {}, reconnecting...\n", host),
                    is_viewed: false,
                    ts: current_timestamp_secs(),
                    from_server: false,
                    seq: 0, end_seq: None,
                    flush: false, gagged: false, highlight_colors: Vec::new(),
                });
                if !app.worlds[world_index].connected {
                    // In the background: awaiting the connect here froze this whole loop (and
                    // every client) until it finished - see App::spawn_world_connect.
                    app.spawn_world_connect(world_index, crate::ConnectOrigin::Client { report_failure: true, client_id }, event_tx);
                }
            }
        }
        WsMessage::SwitchWorld { world_index } => {
            if world_index < app.worlds.len() {
                let changed = app.current_world_index != world_index;
                app.current_world_index = world_index;
                // The requester's view, and the requester alone is told (as handler A):
                // every attached SSH console used to follow a switch any client made.
                app.ws_client_view_world(client_id, world_index);
                app.ws_send_to_client(client_id, WsMessage::WorldSwitched { new_index: world_index });
                // TF's WORLD hook: the foreground world changed.
                if changed {
                    let name = app.worlds[world_index].name.clone();
                    app.fire_tf_hook(Some(world_index), crate::tf::TfHookEvent::World, &name, true);
                }
                // A shell world: the client now views it; send the whole screen and resize.
                app.send_shell_full_frame(client_id, world_index);
                app.resize_shell_worlds();
            }
        }
        WsMessage::ShellInput { world_index, key, text, paste } => {
            if world_index < app.worlds.len() {
                app.shell_input(world_index, key, text, paste);
            }
        }
        WsMessage::ShellMouse { world_index, kind, button, col, row, ctrl, alt, shift } => {
            if world_index < app.worlds.len() {
                app.shell_mouse(world_index, &kind, button, col, row, ctrl, alt, shift);
            }
        }
        WsMessage::UpdateGlobalSettings { more_mode_enabled, spell_check_enabled, temp_convert_enabled, world_switch_mode, show_tags, debug_enabled, ansi_music_enabled, console_theme, gui_theme, gui_transparency, color_offset_percent, wrapspace, remote_initial_lines, input_height, input_height_phone, input_height_tablet, input_height_desktop, font_name, font_size, web_font_size_phone, web_font_size_tablet, web_font_size_desktop, web_font_weight, web_font_line_height, web_font_letter_spacing, web_font_word_spacing, ws_allow_list, web_secure, http_enabled, http_port, web_path, ws_enabled: _, ws_port: _, ws_cert_file, ws_key_file, ws_password, tls_proxy_enabled, mouse_enabled: _, zwj_enabled, new_line_indicator, new_indicator, tts_mode, tts_speak_mode, tts_voice, scrollback_enabled, log_input_enabled, clear_input_on_send, keyboard_always_visible, tabs, icon_bar, picture_host, separator_style, text_reveal_profile, font_profile, tts_profile, taskbar_alert, tabs_profile, screen_reader_profile, announce_screen_reader, status_panel, icon_bar_profile } => {
            app.update_global_settings(
                client_id, more_mode_enabled, spell_check_enabled, temp_convert_enabled,
                world_switch_mode, show_tags, debug_enabled, ansi_music_enabled, console_theme,
                gui_theme, gui_transparency, color_offset_percent, wrapspace, remote_initial_lines,
                input_height, input_height_phone, input_height_tablet, input_height_desktop, font_name, font_size, web_font_size_phone, web_font_size_tablet,
                web_font_size_desktop, web_font_weight, web_font_line_height, web_font_letter_spacing,
                web_font_word_spacing, ws_allow_list, web_secure, http_enabled, http_port, web_path,
                ws_cert_file, ws_key_file, ws_password, tls_proxy_enabled,
                zwj_enabled, new_line_indicator, new_indicator, tts_mode, tts_speak_mode, tts_voice, scrollback_enabled,
                log_input_enabled, clear_input_on_send, keyboard_always_visible, tabs, icon_bar, picture_host, separator_style, text_reveal_profile, font_profile, tts_profile, taskbar_alert,
                crate::websocket::SettingsAdditions { tabs_profile, screen_reader_profile, announce_screen_reader, status_panel, icon_bar_profile },
            );
        }
        WsMessage::ToggleWorldGmcp { world_index } => app.toggle_world_gmcp(world_index),
        // The map (src/map): one shared method each, as handler A does.
        WsMessage::MapWatch { views } => app.map_watch(client_id, views),
        WsMessage::MapRoute { world_index, request_id, to, walk } => app.map_route_request(client_id, world_index, request_id, &to, walk),
        WsMessage::MapStop { world_index } => {
            app.map_stop(world_index);
        }
        WsMessage::MapSearch { world_index, request_id, query } => app.map_search(client_id, world_index, request_id, &query),
        // Android going to the background (and back), a minimised GUI: not watching, so
        // its ▶ markers and unseen counts work as on the console master's server.
        WsMessage::ClientVisibility { visible } => app.handle_client_visibility(client_id, visible),
        // A paused client was used (a key, a click, a menu): it resumes.
        WsMessage::ResumeSession => app.ws_resume_session(client_id),
        WsMessage::RequestInfo { world_index } => {
            let current = app.ws_client_worlds.get(&client_id).map(|v| v.world_index).unwrap_or(app.current_world_index);
            app.ws_info_report(client_id, world_index, current);
        }
        // Plan Job 13 (Phase 4, 4.4): folded into one shared function with main.rs's
        // App::handle_ws_client_msg and this file's other copy below - see
        // commands::execute_send_gmcp/execute_send_msdp.
        WsMessage::SendGmcp { world_index, package, data } => {
            execute_send_gmcp(app, world_index, &package, &data);
        }
        WsMessage::SendMsdp { world_index, variable, value } => {
            execute_send_msdp(app, world_index, &variable, &value);
        }
        // Theme editor, keybind editor, and action editor state messages were entirely absent
        // from this handler (T40) - opening any of these editors from a daemon-attached
        // client showed no data, and Save/Add/Delete actions silently did nothing.
        WsMessage::RequestThemeEditorState => {
            let themes_json = app.theme_file.to_json_all();
            let theme_names: Vec<String> = app.theme_file.themes.keys().cloned().collect();
            let active_theme = app.settings.gui_theme.clone();
            let separator_style = app.settings.separator_style.name().to_string();
            app.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                themes_json,
                theme_names,
                active_theme,
                separator_style,
            });
        }
        WsMessage::UpdateThemeColors { theme_name, colors_json } => {
            let base = theme::ThemeColors::builtin(&theme_name).unwrap_or_else(theme::ThemeColors::dark_default);
            let colors = theme::ThemeColors::from_json(&colors_json, &base);
            app.theme_file.set_theme(&theme_name, colors);
            // If updated theme is the active GUI theme, broadcast CSS vars update
            if theme_name == app.settings.gui_theme {
                let css_vars = app.gui_theme_colors().to_css_vars();
                let colors_json = app.gui_theme_colors().to_json();
                app.ws_broadcast(WsMessage::ThemeCssVarsUpdated {
                    css_vars,
                    colors_json: colors_json.clone(),
                });
                // Also broadcast GlobalSettingsUpdated for GUI clients
                let settings_msg = app.build_global_settings_msg();
                app.ws_broadcast(WsMessage::GlobalSettingsUpdated {
                    settings: settings_msg,
                    input_height: app.input_height,
                });
            }
        }
        WsMessage::AddTheme { name, copy_from } => {
            let base_colors = app.theme_file.get(&copy_from).clone();
            app.theme_file.set_theme(&name, base_colors);
            let themes_json = app.theme_file.to_json_all();
            let theme_names: Vec<String> = app.theme_file.themes.keys().cloned().collect();
            let active_theme = app.settings.gui_theme.clone();
            let separator_style = app.settings.separator_style.name().to_string();
            app.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                themes_json,
                theme_names,
                active_theme,
                separator_style,
            });
        }
        WsMessage::DeleteTheme { name } => {
            app.theme_file.remove_theme(&name);
            let themes_json = app.theme_file.to_json_all();
            let theme_names: Vec<String> = app.theme_file.themes.keys().cloned().collect();
            let active_theme = app.settings.gui_theme.clone();
            let separator_style = app.settings.separator_style.name().to_string();
            app.ws_send_to_client(client_id, WsMessage::ThemeEditorState {
                themes_json,
                theme_names,
                active_theme,
                separator_style,
            });
        }
        WsMessage::UpdateSeparatorStyle { style } => {
            app.set_separator_style(crate::SeparatorStyle::from_name(&style));
            let settings_msg = app.build_global_settings_msg();
            app.ws_broadcast(WsMessage::GlobalSettingsUpdated {
                settings: settings_msg,
                input_height: app.input_height,
            });
        }
        WsMessage::SaveThemeFile => {
            // The separator style lives in settings.dat, not theme.dat, but the theme
            // editor's single Save button owns both.
            let _ = persistence::save_settings(app);
            let content = app.theme_file.generate_file_content();
            let path = clay_config_path("theme.dat");
            match std::fs::write(&path, &content) {
                Ok(_) => {
                    app.ws_send_to_client(client_id, WsMessage::ThemeFileSaved { success: true, error: None });
                }
                Err(e) => {
                    app.ws_send_to_client(client_id, WsMessage::ThemeFileSaved { success: false, error: Some(e.to_string()) });
                }
            }
        }
        WsMessage::RequestActionEditorState => {
            let actions_json = serde_json::to_string(&app.settings.actions).unwrap_or_default();
            let world_names: Vec<&str> = app.worlds.iter().map(|w| w.name.as_str()).collect();
            let world_names_json = serde_json::to_string(&world_names).unwrap_or_default();
            app.ws_send_to_client(client_id, WsMessage::ActionEditorState {
                actions_json,
                world_names_json,
            });
        }
        WsMessage::RequestKeybindEditorState => {
            let bindings_json = app.keybindings.to_json();
            let defaults_json = keybindings::KeyBindings::tf_defaults().to_json();
            let actions_json = keybindings::KeyBindings::actions_json();
            app.ws_send_to_client(client_id, WsMessage::KeybindEditorState {
                bindings_json,
                defaults_json,
                actions_json,
            });
        }
        WsMessage::UpdateKeybindEditorBindings { bindings_json } => {
            app.keybindings = keybindings::KeyBindings::from_json(&bindings_json);
            app.ws_broadcast(WsMessage::KeybindingsUpdated {
                bindings_json: app.keybindings.to_json(),
            });
        }
        WsMessage::SaveKeybindFile => {
            let path = clay_config_path("keybindings.dat");
            match app.keybindings.save(&path) {
                Ok(_) => {
                    app.ws_send_to_client(client_id, WsMessage::KeybindFileSaved { success: true, error: None });
                }
                Err(e) => {
                    app.ws_send_to_client(client_id, WsMessage::KeybindFileSaved { success: false, error: Some(e.to_string()) });
                }
            }
        }
        WsMessage::ResetKeybindDefaults => {
            app.keybindings = keybindings::KeyBindings::tf_defaults();
            let bindings_json = app.keybindings.to_json();
            let defaults_json = keybindings::KeyBindings::tf_defaults().to_json();
            let actions_json = keybindings::KeyBindings::actions_json();
            app.ws_send_to_client(client_id, WsMessage::KeybindEditorState {
                bindings_json,
                defaults_json,
                actions_json,
            });
            app.ws_broadcast(WsMessage::KeybindingsUpdated {
                bindings_json: app.keybindings.to_json(),
            });
        }
        // Note editor (single-user daemon/--local-server: no ownership check
        // needed, same as the theme/keybind editors above; see
        // handle_multiuser_ws_message for the multiuser-owner-checked version).
        WsMessage::RequestNoteEditorState { world_index } => {
            if let Some(world) = app.worlds.get(world_index) {
                app.ws_send_to_client(client_id, WsMessage::NoteEditorState {
                    world_index,
                    world_name: world.name.clone(),
                    notes: world.settings.notes.clone(),
                });
            }
        }
        WsMessage::UpdateNote { world_index, notes } => {
            if let Some(world) = app.worlds.get_mut(world_index) {
                world.settings.notes = notes;
                let has_notes = !world.settings.notes.is_empty();
                let _ = persistence::save_settings(app);
                app.ws_broadcast(WsMessage::NotesChanged { world_index, has_notes });
            }
        }
        // A file the client's `/note <file>` opened (`ws_open_file_for_client`).
        WsMessage::SaveFile { path, content } => app.ws_save_file_for_client(client_id, path, content),
        // TF's expand_line key, for a client (App::ws_expand_line).
        WsMessage::ExpandLine { text } => app.ws_expand_line(client_id, &text),
        // Import Picture: a chunk of a picture to upload (src/picture.rs).
        WsMessage::PictureUpload { id, name, size, offset, data, last } => {
            app.ws_picture_chunk(client_id, id, name, size, offset, data, last);
        }
        // MCP simpleedit reply (plan Job 15): no ownership check needed, same as the
        // note editor above (single-user daemon). `content` is untrusted user-edited
        // text, framed but never interpreted; `reference`/`edit_type` are echoed back
        // exactly as the client received them in McpEditOpen.
        WsMessage::McpEditSet { world_index, reference, edit_type, content } => {
            if let Some(world) = app.worlds.get_mut(world_index) {
                match world.mcp.build_simpleedit_set(&reference, mcp::SimpleEditType::parse(&edit_type), &content) {
                    Some(lines) => {
                        for line in lines {
                            app.send_to_world(world_index, line);
                        }
                    }
                    None => app.emit_client_text(world_index, "Could not send the edited text: the MCP session is no longer available.", false),
                }
            }
        }
        WsMessage::SpellSuggest { word } => {
            let reply = app.spell_checker.suggest_reply(word);
            app.ws_send_to_client(client_id, reply);
        }
        WsMessage::RequestConnectionsList => {
            let current_idx = app.current_world_index;
            const KEEPALIVE_SECS: u64 = 5 * 60;
            let worlds_info: Vec<util::WorldListInfo> = app.worlds.iter().enumerate().map(|(idx, world)| {
                let now = std::time::Instant::now();
                let next_nop = if world.connected {
                    world.last_send_time.map(|t| KEEPALIVE_SECS.saturating_sub(t.elapsed().as_secs()))
                } else {
                    None
                };
                util::WorldListInfo {
                    name: world.name.clone(),
                    connected: world.connected,
                    is_current: idx == current_idx,
                    is_ssl: world.is_tls,
                    is_proxy: world.proxy_pid.is_some(),
                    unseen_lines: world.unseen_lines,
                    last_send_secs: world.last_user_command_time.map(|t| now.duration_since(t).as_secs()),
                    last_recv_secs: world.last_receive_time.map(|t| now.duration_since(t).as_secs()),
                    last_nop_secs: world.last_nop_time.map(|t| now.duration_since(t).as_secs()),
                    next_nop_secs: next_nop,
                    buffer_size: world.output_lines.len() + world.pending_lines.len(),
                }
            }).collect();
            let output = util::format_worlds_list(&worlds_info);
            let lines: Vec<String> = output.lines().map(|s| s.to_string()).collect();
            app.ws_send_to_client(client_id, WsMessage::ConnectionsListResponse { lines });
        }
        // Were entirely absent from this handler (T40) - a daemon-attached client could
        // never view or manage the ban list (the request just silently fell into the
        // catch-all, no reply).
        WsMessage::BanListRequest => app.ws_ban_list_response(client_id),
        WsMessage::UnbanRequest { host } => app.ws_unban_request(client_id, host),
        WsMessage::ChangePassword { .. } => app.ws_change_password(client_id),
        // Was entirely absent from this handler (T40) - a /remote liveness check
        // (spawn_remote_ping_check) against a daemon-attached client would always time out,
        // since the client's PongCheck reply had nowhere to land.
        WsMessage::PongCheck { nonce, acked } => {
            if nonce == app.remote_ping_nonce {
                if let Some(ref responses) = app.remote_ping_responses {
                    if let Ok(mut set) = responses.lock() {
                        set.insert(client_id);
                    }
                }
            }
            // Record the client's per-world delivery ack (PROTOCOL-ROADMAP.md Step 2) so a
            // future resume/backpressure path knows how caught-up it already is.
            if !acked.is_empty() {
                if let Some(ref server) = app.ws_server {
                    server.record_acked_seq(client_id, &acked);
                }
            }
            // ...then check it against what we owe (PROTOCOL-ROADMAP.md Phase C) - same
            // audit the master-WS PongCheck handler runs, since `-D` serves the same
            // single-user clients over the same protocol.
            app.audit_client_acks(client_id);
            // ...and the server's audit of itself (PROTOCOL-ROADMAP.md Phase F). Must be
            // here as well as in the master-WS handler: `-D` is what Android's local-server
            // mode runs, so leaving it out would blind the detector on the platform this
            // whole bug class keeps showing up on.
            app.audit_broadcast_ledger();
        }
        WsMessage::Ping => {
            app.ws_send_to_client(client_id, WsMessage::Pong);
        }
        WsMessage::UpdateViewState { world_index, visible_lines, visible_columns, splits } => {
            app.note_client_splits(client_id, splits);
            // Track client's view state for more-mode threshold calculation
            if world_index < app.worlds.len() {
                let dimensions = app.ws_client_worlds.get(&client_id).and_then(|s| s.dimensions);
                let vc = visible_columns.unwrap_or_else(|| app.ws_client_worlds.get(&client_id).map(|v| v.visible_columns).unwrap_or(0));
                let paused = app.ws_client_worlds.get(&client_id).map(|v| v.paused).unwrap_or(false);
                let visible = app.ws_client_worlds.get(&client_id).map(|v| v.visible).unwrap_or(true);
                let history = app.client_history_after(client_id, None, world_index);
                app.ws_client_worlds.insert(client_id, ClientViewState { world_index, history, visible_lines, visible_columns: vc, dimensions, paused, visible, disconnected_at: None });
                app.resize_shell_worlds();
            }
        }
        // Was entirely absent from this handler (T40) - a daemon-attached client reporting its
        // output dimensions (for NAWS) got no response, so NAWS updates never propagated to
        // the MUD server for daemon-attached clients.
        WsMessage::UpdateDimensions { width, height } => {
            if let Some(state) = app.ws_client_worlds.get_mut(&client_id) {
                let old_dims = state.dimensions;
                state.dimensions = Some((width, height));
                if old_dims != Some((width, height)) {
                    app.send_naws_to_all_worlds();
                    app.resize_shell_worlds();
                }
            }
        }
        WsMessage::MarkWorldSeen { world_index, previous_world_index, other_views } => {
            app.handle_mark_world_seen(client_id, world_index, previous_world_index, other_views);
        }
        WsMessage::OtherViews { worlds, dims } => {
            app.handle_other_views(client_id, worlds, dims);
        }
        WsMessage::ReleasePending { world_index, count } => {
            app.release_pending_lines(client_id, world_index, count);
        }
        WsMessage::SelectiveFlush { world_index } => {
            app.selective_flush(world_index);
        }
        WsMessage::ReportSeqMismatch { world_index, expected_seq_gt, actual_seq, line_text, source } => {
            // Always-on (not gated behind is_debug_enabled()): only fires on a real
            // connection-level fault, so no log-spam risk, and it was invisible in the field
            // until now (D-Termux-lines investigation).
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("SEQ-MISMATCH", &ip, &format!("[{}] in '{}': expected seq>{}, got seq={}, text={:?}",
                source, world_name, expected_seq_gt, actual_seq,
                line_text.chars().take(80).collect::<String>()));
        }
        WsMessage::ReportDuplicate { world_index, line_seq, max_seq, line_text, source } => {
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("DUPLICATE", &ip, &format!("[{}] in '{}': line_seq={}, max_seq={}, text={:?}",
                source, world_name, line_seq, max_seq,
                line_text.chars().take(200).collect::<String>()));
        }
        WsMessage::ReportOutOfOrder { world_index, line_seq, recovered_count, source } => {
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("OUT-OF-ORDER", &ip, &format!("[{}] in '{}': recovered {} line(s) starting at seq={} that had arrived out of order",
                source, world_name, recovered_count, line_seq));
        }
        WsMessage::ReportGap { world_index, hole_start, hole_end, attempts, source } => {
            // Always-on, same reasoning as the sibling reports above: only fires when a
            // client has genuinely given up on a range of output, which is exactly the
            // event that has been invisible in the field (PROTOCOL-ROADMAP.md Phase F).
            let world_name = app.worlds.get(world_index).map(|w| w.name.as_str()).unwrap_or("?").to_string();
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("SEQ-HOLE", &ip, &format!("[{}] in '{}': gave up on seq {}..={} ({} line(s)) after {} gap-fill attempt(s) returned nothing for it",
                source, world_name, hole_start, hole_end,
                hole_end.saturating_sub(hole_start).saturating_add(1), attempts));
        }
        WsMessage::ReportClientLifecycle { event, detail, source } => {
            // Same reasoning as the sibling reports above; see WsMessage::ReportClientLifecycle
            // in websocket.rs for why an Android lifecycle transition is worth a server log line.
            let ip = app.ws_server.as_ref().and_then(|s| s.get_client_ip(client_id)).unwrap_or_else(|| "?".to_string());
            crate::http::log_remote_event("CLIENT-LIFECYCLE", &ip, &format!("[{}] {}: {}", source, event, detail));
        }
        WsMessage::ClientTypeDeclaration { client_type } => {
            // Update client type in WebSocket server
            if let Some(ref server) = app.ws_server {
                server.set_client_type(client_id, client_type);
            }
        }
        WsMessage::CycleWorld { direction } => {
            app.handle_cycle_world(client_id, &direction);
        }
        WsMessage::RequestScrollback { world_index, count, before_seq, after_seq, request_id } => {
            app.handle_request_scrollback(client_id, world_index, count, before_seq, after_seq, request_id);
        }
        WsMessage::RequestHistory { world_index, count, request_id, before_seq, before_ts, archive_before } => {
            app.handle_request_history(client_id, world_index, count, request_id, before_seq, before_ts, archive_before);
        }
        WsMessage::RequestWorldState { world_index } => {
            app.handle_request_world_state(client_id, world_index);
        }
        WsMessage::UpdateActions { actions } => {
            app.handle_update_actions(actions);
        }
        WsMessage::CreateWorld { name } => {
            app.create_world(client_id, &name);
        }
        WsMessage::DeleteWorld { world_index } => {
            app.delete_world(world_index);
        }
        WsMessage::UpdateWorldSettings { world_index, name, hostname, port, user, password, use_ssl, log_enabled, encoding, auto_login, keep_alive_type, keep_alive_cmd, gmcp_packages, auto_reconnect_secs, msp_enabled, mcp_enabled, mccp2_enabled, world_type, prompt_wait_ms, tf_type, slack_token, slack_channel, slack_workspace, discord_token, discord_guild, discord_channel, discord_dm_user, chat, keep_alive_gag_enabled, keep_alive_gag, status_panel, login_command, first_command } => {
            app.update_world_settings(
                world_index, name, hostname, port, user, password, use_ssl, log_enabled,
                encoding, auto_login, keep_alive_type, keep_alive_cmd, gmcp_packages, auto_reconnect_secs,
                msp_enabled, mcp_enabled, mccp2_enabled,
                world_type, prompt_wait_ms, tf_type, slack_token, slack_channel, slack_workspace,
                discord_token, discord_guild, discord_channel, discord_dm_user, chat,
                keep_alive_gag_enabled, keep_alive_gag, status_panel, login_command, first_command,
            );
        }
        WsMessage::CalculateNextWorld { current_index, exclude } => {
            let next_idx = app.calculate_next_world_from(current_index, &exclude);
            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: next_idx });
        }
        WsMessage::CalculatePrevWorld { current_index, exclude } => {
            let prev_idx = app.calculate_prev_world_from(current_index, &exclude);
            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: prev_idx });
        }
        WsMessage::CalculateOldestPending { current_index, exclude } => {
            // The fallback world is this client's own last visit, not the console's.
            let history = app.ws_client_worlds.get(&client_id).map(|v| v.history.clone()).unwrap_or_default();
            let oldest_idx = app.calculate_oldest_pending_world_from(current_index, &history, &exclude);
            app.ws_send_to_client(client_id, WsMessage::CalculatedWorld { index: oldest_idx });
        }
        // /import export side: mirrors the handler in main.rs's App::handle_ws_message —
        // needed here too since --local-server (Android on-device mode) and -D (run_daemon_server)
        // both dispatch through this function instead. See plan `i-d-like-to-make-snuggly-rain.md`.
        WsMessage::RequestSettingsExport => app.ws_settings_export(client_id),
        // /import trigger side: mirrors the handler in main.rs's App::handle_ws_message.
        // Result comes back via AppEvent::ImportResult, handled by both callers of this
        // function's event loops (run_app_headless in main.rs, run_daemon_server in this module).
        WsMessage::ImportSettings { addr, password, auth_key, allow_insecure } => {
            app.ws_import_settings(client_id, addr, password, auth_key, allow_insecure, event_tx);
        }
        // Full state resync. Needed here too since --local-server (Android on-device mode)
        // and -D (run_daemon_server) both dispatch through this function instead of
        // main.rs's - without an arm here the message silently falls into the catch-all
        // below: no reply, no error (web/Android ping the server on visibility change and
        // call RequestState when the connection looks stale).
        WsMessage::RequestState => {
            app.handle_request_state(client_id);
        }
        _ => {}
    }
}


/// Connect a world in daemon mode (non-multiuser)
/// Returns (cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path) on success
/// Connect world `world_name` to `host:port` through a TLS proxy process (a separate
/// process holds the TLS connection so it survives hot reload; Clay talks to it over a
/// Unix socket, or a Named Pipe on Windows), then start the telnet writer/reader on that
/// link. Returns (cmd_tx, proxy_pid, socket/pipe path). Sends the auto-login itself
/// unless `skip_auto_login`.
///
/// Shared by `connect_daemon_world` and the console's `/worlds` connect task. Always
/// runs off the event loop: starting the proxy waits (a blocking sleep loop, hence
/// `spawn_blocking`) for the proxy to reach the MUD, which against a down host takes
/// seconds.
#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn connect_via_tls_proxy(
    world_name: &str,
    host: &str,
    port: &str,
    settings: &WorldSettings,
    event_tx: mpsc::Sender<AppEvent>,
    connection_id: u64,
    skip_auto_login: bool,
    gmcp_enabled: bool,
    diag: Option<Arc<crate::protocol_state::ConnDiag>>,
) -> Result<(mpsc::Sender<WriteCommand>, u32, std::path::PathBuf), String> {
    let (proxy_pid, proxy_path) = {
        let (w, h, p) = (world_name.to_string(), host.to_string(), port.to_string());
        tokio::task::spawn_blocking(move || spawn_tls_proxy(&w, &h, &p))
            .await
            .map_err(|e| format!("Failed to spawn TLS proxy: {}", e))?
            .map_err(|e| format!("Failed to spawn TLS proxy: {}", e))?
    };

    #[cfg(unix)]
    let (read_half, write_half) = {
        let mut stream = None;
        for attempt in 0..20 {
            match tokio::net::UnixStream::connect(&proxy_path).await {
                Ok(s) => { stream = Some(s); break; }
                Err(_) => {
                    if attempt < 19 { tokio::time::sleep(Duration::from_millis(100)).await; }
                }
            }
        }
        let Some(unix_stream) = stream else {
            unsafe { libc::kill(proxy_pid as libc::pid_t, libc::SIGTERM); }
            return Err("Failed to connect to TLS proxy".to_string());
        };
        let (r, w) = unix_stream.into_split();
        (StreamReader::Proxy(r), StreamWriter::Proxy(w))
    };
    #[cfg(windows)]
    let (read_half, write_half) = match connect_to_proxy_pipe(&proxy_path, 10).await {
        Some(pipe_client) => {
            let (r, w) = tokio::io::split(pipe_client);
            (StreamReader::NamedPipeProxy(r), StreamWriter::NamedPipeProxy(w))
        }
        None => {
            kill_proxy_process(proxy_pid);
            return Err("Failed to connect to TLS proxy".to_string());
        }
    };

    // Plan Job 13 (Phase 4, 4.4): spawn_telnet_writer. Fresh connect -
    // settings.encoding is the right initial encoding.
    let cmd_tx = spawn_telnet_writer(write_half, settings.encoding);
    if !skip_auto_login {
        let user = settings.user.clone();
        let password = settings.password.clone();
        let login_lines = settings.auto_connect_type.connect_lines(&user, &password);
        if !user.is_empty() && !password.is_empty() && !login_lines.is_empty() {
            let tx = cmd_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                for line in login_lines {
                    let _ = tx.send(WriteCommand::Text(line)).await;
                }
            });
        }
    }
    // Plan Job 5, 2.3: spawn_telnet_reader (MCCP2, TTYPE/CHARSET events, WontEchoSeen,
    // the unified "Connection closed by server." message on EOF).
    let telnet_cfg = TelnetConfig {
        term_type: std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string()),
        msp_enabled: settings.msp_enabled,
        mccp2_enabled: settings.mccp2_enabled,
        gmcp_enabled,
        // Job 12 (plan Phase 4, 4.1): the proxy path is only reached for TLS.
        is_tls: true,
        diag,
        ..TelnetConfig::default()
    };
    spawn_telnet_reader(
        read_half,
        cmd_tx.clone(),
        event_tx,
        TelnetTarget::World(world_name.to_string()),
        connection_id,
        telnet_cfg,
    );
    Ok((cmd_tx, proxy_pid, proxy_path))
}

/// One address a connection attempt failed on, as TinyFugue shows it after the world's
/// name in its ICONFAIL and CONFAIL hooks: "<address> <port>: <reason>". The address is
/// the one tried, or the host name when it never resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddrFailure {
    pub addr: String,
    pub port: String,
    pub reason: String,
}

impl AddrFailure {
    pub fn new(addr: impl Into<String>, port: impl Into<String>, reason: impl Into<String>) -> Self {
        AddrFailure { addr: addr.into(), port: port.into(), reason: reason.into() }
    }

    /// TF's text for it: "<address> <port>: <reason>".
    pub fn tf_text(&self) -> String {
        format!("{} {}: {}", self.addr, self.port, self.reason)
    }
}

/// A connection attempt that failed: every address tried, in order - TF fires ICONFAIL
/// for each but the last, then CONFAIL for the last (`App::report_connect_failure`).
/// Never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectFailure {
    pub attempts: Vec<AddrFailure>,
}

impl ConnectFailure {
    pub fn at(addr: impl Into<String>, port: impl Into<String>, reason: impl Into<String>) -> Self {
        ConnectFailure { attempts: vec![AddrFailure::new(addr, port, reason)] }
    }

    /// Where the attempt finally failed.
    pub fn last(&self) -> &AddrFailure {
        self.attempts.last().expect("a ConnectFailure is never empty")
    }

    /// Clay's own account of it: the first IPv4 address's error when one was tried (an
    /// IPv6 fallback's "Network is unreachable" usually says less), else the last.
    pub fn reason(&self) -> &str {
        self.attempts.iter()
            .find(|f| f.addr.parse::<std::net::Ipv4Addr>().is_ok())
            .unwrap_or_else(|| self.last())
            .reason.as_str()
    }
}

/// The TLS version a connection settled on, "TLS 1.3" or "TLS 1.2", for `/info`.
pub(crate) fn tls_version_name(conn: &rustls::ClientConnection) -> Option<String> {
    let version = conn.protocol_version()?;
    Some(match version {
        rustls::ProtocolVersion::TLSv1_3 => "TLS 1.3".to_string(),
        rustls::ProtocolVersion::TLSv1_2 => "TLS 1.2".to_string(),
        other => format!("{:?}", other),
    })
}

/// The cipher a TLS connection settled on, named as TF names it (its OpenSSL's names):
/// TF's CONNECT hook gets it after the world's name.
pub(crate) fn tls_cipher_name(conn: &rustls::ClientConnection) -> Option<String> {
    let suite = conn.negotiated_cipher_suite()?;
    Some(openssl_cipher_name(&format!("{:?}", suite.suite())))
}

/// rustls's name for a cipher suite as OpenSSL - and so TF - names it: TLS 1.3 suites by
/// their IANA names ("TLS_AES_256_GCM_SHA384"), TLS 1.2 ones the OpenSSL way
/// ("ECDHE-RSA-AES256-GCM-SHA384"). A suite not listed keeps its own name.
pub(crate) fn openssl_cipher_name(rustls_name: &str) -> String {
    match rustls_name {
        "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" => "ECDHE-ECDSA-AES128-GCM-SHA256".to_string(),
        "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" => "ECDHE-ECDSA-AES256-GCM-SHA384".to_string(),
        "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" => "ECDHE-ECDSA-CHACHA20-POLY1305".to_string(),
        "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" => "ECDHE-RSA-AES128-GCM-SHA256".to_string(),
        "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" => "ECDHE-RSA-AES256-GCM-SHA384".to_string(),
        "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" => "ECDHE-RSA-CHACHA20-POLY1305".to_string(),
        name => match name.strip_prefix("TLS13_") {
            Some(rest) => format!("TLS_{}", rest),
            None => name.to_string(),
        },
    }
}

/// A world's host field: "primary,fallback" names a second host to try once every address
/// of the first has failed.
pub(crate) fn split_host_list(raw: &str) -> (String, Option<String>) {
    match raw.split_once(',') {
        Some((primary, fallback)) => {
            let fallback = fallback.trim();
            (primary.trim().to_string(), (!fallback.is_empty()).then(|| fallback.to_string()))
        }
        None => (raw.trim().to_string(), None),
    }
}

/// Connect to `host` (or, once all of its addresses have failed, `fallback` - saying so in
/// `world`'s output) on `port`: every address a name resolves to, in turn, IPv4 first, as
/// TF does. On success the addresses that failed first come back too: TF's ICONFAILs.
pub(crate) async fn connect_tcp(
    world: &str,
    host: &str,
    fallback: Option<&str>,
    port: &str,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<(TcpStream, Vec<AddrFailure>), ConnectFailure> {
    let mut failures = Vec::new();
    if let Some(stream) = connect_host(host, port, &mut failures).await {
        return Ok((stream, failures));
    }
    if let Some(fallback) = fallback {
        let primary = ConnectFailure { attempts: failures.clone() };
        let _ = event_tx.send(AppEvent::ServerData(
            world.to_string(),
            format!("Primary host {} failed: {}, trying {}...\r\n", host, primary.reason(), fallback).into_bytes(),
        )).await;
        if let Some(stream) = connect_host(fallback, port, &mut failures).await {
            return Ok((stream, failures));
        }
    }
    Err(ConnectFailure { attempts: failures })
}

/// Try each address `host` resolves to (IPv4 first), recording every failure.
async fn connect_host(host: &str, port: &str, failures: &mut Vec<AddrFailure>) -> Option<TcpStream> {
    let mut addrs: Vec<std::net::SocketAddr> = match tokio::net::lookup_host(format!("{}:{}", host, port)).await {
        Ok(addrs) => addrs.collect(),
        Err(e) => {
            failures.push(AddrFailure::new(host, port, crate::telnet_reader::error_text(&e)));
            return None;
        }
    };
    if addrs.is_empty() {
        failures.push(AddrFailure::new(host, port, format!("Could not resolve {}", host)));
        return None;
    }
    addrs.sort_by_key(|a| !a.is_ipv4());
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Some(stream),
            Err(e) => failures.push(AddrFailure::new(
                addr.ip().to_string(),
                addr.port().to_string(),
                crate::telnet_reader::error_text(&e),
            )),
        }
    }
    None
}

/// `connect_daemon_world`'s result: (cmd_tx, socket_fd, is_tls, proxy_pid, proxy_socket_path),
/// or why it failed.
pub type DaemonConnectResult = Result<(mpsc::Sender<WriteCommand>, Option<SocketFd>, bool, Option<u32>, Option<std::path::PathBuf>), ConnectFailure>;

#[allow(clippy::too_many_arguments)]
pub async fn connect_daemon_world(
    _world_index: usize,
    world_name: String,
    settings: &WorldSettings,
    event_tx: mpsc::Sender<AppEvent>,
    connection_id: u64,
    skip_auto_login: bool,
    tls_proxy_enabled: bool,
    // `Policy::gmcp`: a clay_web visitor's game gets DONT GMCP.
    gmcp_enabled: bool,
    diag: Option<Arc<crate::protocol_state::ConnDiag>>,
) -> DaemonConnectResult {
    #[cfg(target_os = "android")]
    let _ = tls_proxy_enabled;
    let host = &settings.hostname;
    let port = &settings.port;
    let use_ssl = settings.use_ssl;

    if host.is_empty() || port.is_empty() {
        return Err(ConnectFailure::at(host.as_str(), port.as_str(), "no host and port set"));
    }

    // TLS proxy path — a separate proxy process holds the TLS connection so it
    // survives hot reload. Falls through to direct TLS if the proxy fails.
    #[cfg(not(target_os = "android"))]
    if use_ssl && tls_proxy_enabled {
        if let Ok((cmd_tx, proxy_pid, proxy_path)) = connect_via_tls_proxy(
            &world_name, host, port, settings, event_tx.clone(), connection_id, skip_auto_login, gmcp_enabled, diag.clone(),
        ).await {
            return Ok((cmd_tx, None, true, Some(proxy_pid), Some(proxy_path)));
        }
    }

    let (host_name, fallback) = split_host_list(host);
    match connect_tcp(&world_name, &host_name, fallback.as_deref(), port, &event_tx).await {
        Ok((tcp_stream, failed_first)) => {
            let _ = tcp_stream.set_nodelay(true);
            // Addresses that failed before this one: TF's ICONFAILs, before its CONNECT.
            if !failed_first.is_empty() {
                let _ = event_tx.send(AppEvent::ConnectAttemptsFailed(world_name.clone(), connection_id, failed_first)).await;
            }
            // Where a TLS failure happened, for TF's CONFAIL text.
            let (peer_addr, peer_port) = match tcp_stream.peer_addr() {
                Ok(a) => {
                    if let Some(d) = &diag {
                        d.set_peer(a.to_string());
                    }
                    (a.ip().to_string(), a.port().to_string())
                }
                Err(_) => (host_name.clone(), port.clone()),
            };
            let tls_failure = |reason: String| ConnectFailure::at(peer_addr.as_str(), peer_port.as_str(), reason);

            // Store the socket fd for hot reload (before splitting)
            #[cfg(unix)]
            let socket_fd: Option<SocketFd> = {
                use std::os::unix::io::AsRawFd;
                Some(tcp_stream.as_raw_fd())
            };
            #[cfg(windows)]
            let socket_fd: Option<SocketFd> = {
                use std::os::windows::io::AsRawSocket;
                Some(tcp_stream.as_raw_socket() as i64)
            };
            #[cfg(not(any(unix, windows)))]
            let socket_fd: Option<SocketFd> = None;

            // Enable TCP keepalive to detect dead connections faster
            enable_tcp_keepalive(&tcp_stream);

            // Handle SSL if needed
            let is_tls;
            let (read_half, write_half): (StreamReader, StreamWriter) = if use_ssl {
                is_tls = true;
                {
                    use rustls::RootCertStore;
                    use tokio_rustls::TlsConnector;
                    use rustls::pki_types::ServerName;

                    let mut root_store = RootCertStore::empty();
                    root_store.roots = webpki_roots::TLS_SERVER_ROOTS.to_vec();

                    let config = rustls::ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(crate::platform::danger_rustls::TofuVerifier::new(format!("{}:{}", host_name, port))))
                        .with_no_client_auth();

                    let connector = TlsConnector::from(Arc::new(config));
                    let server_name = match ServerName::try_from(host_name.clone()) {
                        Ok(sn) => sn,
                        Err(e) => return Err(tls_failure(format!("Invalid server name: {}", e))),
                    };

                    match connector.connect(server_name, tcp_stream).await {
                        Ok(tls_stream) => {
                            // TF's CONNECT hook names the cipher.
                            if let Some(cipher) = tls_cipher_name(tls_stream.get_ref().1) {
                                let _ = event_tx.send(AppEvent::TlsCipher(world_name.clone(), connection_id, cipher)).await;
                            }
                            if let (Some(d), Some(version)) = (&diag, tls_version_name(tls_stream.get_ref().1)) {
                                d.set_tls_version(version);
                            }
                            let (r, w) = tokio::io::split(tls_stream);
                            (StreamReader::Tls(r), StreamWriter::Tls(w))
                        }
                        Err(e) => return Err(tls_failure(format!("SSL handshake failed: {}", e))),
                    }
                }
            } else {
                is_tls = false;
                let (r, w) = tcp_stream.into_split();
                (StreamReader::Plain(r), StreamWriter::Plain(w))
            };

            // For TLS, socket_fd should be None (can't preserve across reload)
            let final_socket_fd = if is_tls { None } else { socket_fd };

            // Plan Job 13 (Phase 4, 4.4): migrated onto spawn_telnet_writer. Fresh
            // connect - settings.encoding is the right initial encoding.
            let cmd_tx = spawn_telnet_writer(write_half, settings.encoding);

            // Send auto-login if configured (skip if /worlds -l was used)
            if !skip_auto_login {
                let user = settings.user.clone();
                let password = settings.password.clone();
                let login_lines = settings.auto_connect_type.connect_lines(&user, &password);
                if !user.is_empty() && !password.is_empty() && !login_lines.is_empty() {
                    let tx = cmd_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        for line in login_lines {
                            let _ = tx.send(WriteCommand::Text(line)).await;
                        }
                    });
                }
            }

            // Clone for reader task
            let event_tx_read = event_tx.clone();
            let world_name_read = world_name.clone();
            let reader_conn_id = connection_id;

            // Terminal type for TTYPE IS responses — as of Job 12 (plan Phase 4,
            // 4.1) the session itself answers with the MTTS-cycled value (client
            // name, then this term_type, then MTTS <bitmask>) the instant a TTYPE
            // SEND arrives; `App` no longer builds a reply. Matches the existing
            // `$TERM`-or-"ANSI" fallback exactly.
            let telnet_cfg = TelnetConfig {
                term_type: std::env::var("TERM").unwrap_or_else(|_| "ANSI".to_string()),
                msp_enabled: settings.msp_enabled,
                mccp2_enabled: settings.mccp2_enabled,
                gmcp_enabled,
                is_tls, // Job 12 (plan Phase 4, 4.1): set above by the TLS/plain match
                diag,
                ..TelnetConfig::default()
            };

            // Spawn reader task (plan Job 5, 2.3: migrated onto the shared
            // `spawn_telnet_reader`, src/telnet_reader.rs — this was already the richest
            // of daemon.rs's four reader loops, so nothing is gained or lost here; see the
            // Job 5 report for what the other three gained).
            spawn_telnet_reader(
                read_half,
                cmd_tx.clone(),
                event_tx_read,
                TelnetTarget::World(world_name_read),
                reader_conn_id,
                telnet_cfg,
            );

            Ok((cmd_tx, final_socket_fd, is_tls, None, None))
        }
        Err(failure) => Err(failure),
    }
}


/// Generate splash screen content as strings (for web client)
pub fn generate_splash_strings() -> Vec<String> {
    vec![
        "".to_string(),
        "\x1b[38;5;180m          (\\/\\__o     \x1b[38;5;209m \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2557}      \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2557}   \u{2588}\u{2588}\u{2557}\x1b[0m".to_string(),
        "\x1b[38;5;180m  __      `-/ `_/     \x1b[38;5;208m\u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2557}\u{255a}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2554}\u{255d}\x1b[0m".to_string(),
        "\x1b[38;5;180m `--\\______/  |       \x1b[38;5;215m\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2551} \u{255a}\u{2588}\u{2588}\u{2588}\u{2588}\u{2554}\u{255d} \x1b[0m".to_string(),
        "\x1b[38;5;180m    /        /        \x1b[38;5;216m\u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2551}     \u{2588}\u{2588}\u{2554}\u{2550}\u{2550}\u{2588}\u{2588}\u{2551}  \u{255a}\u{2588}\u{2588}\u{2554}\u{255d}  \x1b[0m".to_string(),
        "\x1b[38;5;180m -`/_------'\\_.       \x1b[38;5;217m\u{255a}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}\u{2588}\u{2588}\u{2551}  \u{2588}\u{2588}\u{2551}   \u{2588}\u{2588}\u{2551}   \x1b[0m".to_string(),
        "\x1b[38;5;218m                       \u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255d}\u{255a}\u{2550}\u{255d}  \u{255a}\u{2550}\u{255d}   \u{255a}\u{2550}\u{255d}   \x1b[0m".to_string(),
        "".to_string(),
        "\x1b[38;5;213m\u{2728} A 90dies mud client written today \u{2728}\x1b[0m".to_string(),
        "".to_string(),
        "\x1b[38;5;244mSelect a world to connect\x1b[0m".to_string(),
        "".to_string(),
    ]
}


#[cfg(test)]
mod telnet_reader_migration_tests {
    // Plan Job 5 (2.3 of investigate-differences-between-tinyfugu-fluffy-stallman.md):
    // daemon.rs's four telnet reader loops (connect_daemon_world's direct-connect, its
    // two TLS-proxy loops, and connect_multiuser_world) were migrated from a hand-rolled
    // process_telnet/find_safe_split_point loop onto the shared spawn_telnet_reader
    // (telnet_reader.rs). This test drives a real fake MUD server (testserver.rs)
    // through the actual `connect_daemon_world` production function - not the separate
    // testharness.rs connection path - and proves a GMCP message survives the migrated
    // direct-connect loop all the way to the AppEvent channel App's own dispatch loop
    // (run_daemon_server) reads from. The proxy loops' MCCP2 gain and the close-message
    // unification are covered at the spawn_telnet_reader level in telnet_reader.rs's own
    // tests (`proxy_shaped_reader_decompresses_mccp2_and_still_closes_cleanly`), driven
    // over `TelnetTarget::World` - the exact shape both proxy loops now pass - since
    // wiring a real Unix-socket TLS proxy (spawned process, certs) into a test would be
    // disproportionate to what's being proven.
    use super::*;
    use crate::testserver::{self, ServerAction, PortScenario};
    use crate::telnet::{TELNET_IAC, TELNET_WILL, TELNET_SB, TELNET_SE, TELNET_OPT_GMCP};

    fn find_free_port() -> u16 {
        crate::testserver::free_port()
    }

    /// TF's CONNECT hook names a TLS connection's cipher as OpenSSL does (checked against
    /// tf 5.0b8: "tlsw TLS_AES_256_GCM_SHA384"): rustls's names, renamed to match.
    #[test]
    fn test_openssl_cipher_names() {
        assert_eq!(openssl_cipher_name("TLS13_AES_256_GCM_SHA384"), "TLS_AES_256_GCM_SHA384");
        assert_eq!(openssl_cipher_name("TLS13_CHACHA20_POLY1305_SHA256"), "TLS_CHACHA20_POLY1305_SHA256");
        assert_eq!(openssl_cipher_name("TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384"), "ECDHE-RSA-AES256-GCM-SHA384");
        assert_eq!(openssl_cipher_name("TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"), "ECDHE-ECDSA-CHACHA20-POLY1305");
        assert_eq!(openssl_cipher_name("SOMETHING_ELSE"), "SOMETHING_ELSE");
    }

    /// A failed connect names what real tf's ICONFAIL/CONFAIL text names: the address and
    /// port tried and the system's reason, or the host name when it never resolved. A
    /// fallback host is tried once the primary has failed, which is noted in the world and
    /// kept for ICONFAIL.
    #[tokio::test]
    async fn connect_tcp_reports_each_address() {
        let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(8);
        // Port 1 (tcpmux) has no listener, and only root could bind one.
        let failure = connect_tcp("w", "127.0.0.1", None, "1", &event_tx).await.expect_err("nothing listens on port 1");
        assert_eq!(failure.attempts, vec![AddrFailure::new("127.0.0.1", "1", "Connection refused")]);
        assert_eq!(failure.last().tf_text(), "127.0.0.1 1: Connection refused");

        let failure = connect_tcp("w", "nonexistent.invalid", None, "23", &event_tx).await.expect_err(".invalid never resolves");
        assert_eq!(failure.last().addr, "nonexistent.invalid");
        assert!(!failure.last().reason.starts_with("failed to lookup"), "{:?}", failure);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let (_stream, failed_first) = connect_tcp("w", "nonexistent.invalid", Some("127.0.0.1"), &port, &event_tx)
            .await.expect("the fallback host answers");
        assert_eq!(failed_first.len(), 1);
        assert_eq!(failed_first[0].addr, "nonexistent.invalid");
        match event_rx.try_recv() {
            Ok(AppEvent::ServerData(world, bytes)) => {
                assert_eq!(world, "w");
                assert!(String::from_utf8_lossy(&bytes).starts_with("Primary host nonexistent.invalid failed: "));
            }
            _ => panic!("expected the note about trying the fallback host"),
        }
        assert_eq!(split_host_list("a.org, b.org"), ("a.org".to_string(), Some("b.org".to_string())));
        assert_eq!(split_host_list(" a.org "), ("a.org".to_string(), None));
    }

    /// IAC WILL GMCP, then IAC SB GMCP Core.Hello {"foo":"bar"} IAC SE, as a single raw
    /// write (mirrors how a real MUD server frames a negotiation + immediate message).
    fn gmcp_scenario() -> PortScenario {
        let mut raw = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_GMCP];
        raw.extend_from_slice(&[TELNET_IAC, TELNET_SB, TELNET_OPT_GMCP]);
        raw.extend_from_slice(b"Core.Hello {\"foo\":\"bar\"}");
        raw.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        PortScenario {
            actions: vec![
                ServerAction::SendRaw(raw),
                ServerAction::Sleep(Duration::from_millis(300)),
                ServerAction::Disconnect,
            ],
            telnet_negotiate: false,
        }
    }

    /// End-to-end proof (plan Job 5's required test) that the migrated
    /// `connect_daemon_world` direct-connect reader loop carries a real protocol event —
    /// GMCP, matching finding 8's "one package family" scope, not a synthetic one — from
    /// the wire through `TelnetSession`/`spawn_telnet_reader` to the `AppEvent` channel
    /// `run_daemon_server`'s dispatch loop consumes. This is the same channel and the
    /// same production connect function daemon.rs uses for a real `-D`/single-user world;
    /// only the fake MUD server on the other end is test infrastructure.
    #[tokio::test]
    async fn connect_daemon_world_carries_gmcp_to_the_app_event_channel() {
        let port = find_free_port();
        let server = tokio::spawn(testserver::run_server_port(port, gmcp_scenario()));
        tokio::time::sleep(Duration::from_millis(100)).await;

        let settings = WorldSettings {
            hostname: "127.0.0.1".to_string(),
            port: port.to_string(),
            ..Default::default()
        };

        let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(32);
        let conn = connect_daemon_world(
            0,
            "gmcptest".to_string(),
            &settings,
            event_tx,
            1,     // connection_id
            true,  // skip_auto_login - no login needed for this scenario
            false, // tls_proxy_enabled - plain TCP, exercises the direct-connect loop
            true,  // gmcp_enabled - the scenario is GMCP
            None,
        ).await;
        assert!(conn.is_ok(), "connect_daemon_world should have connected to the fake server");

        // Drain events until both the negotiation mirror and the actual GMCP payload
        // have arrived, or time out.
        let mut saw_negotiated = false;
        let mut saw_gmcp: Option<(String, String)> = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline && (!saw_negotiated || saw_gmcp.is_none()) {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, event_rx.recv()).await {
                Ok(Some(AppEvent::Telnet(TelnetTarget::World(name), TelnetEvent::OptionEnabled(opt))))
                    if name == "gmcptest" && opt == TELNET_OPT_GMCP =>
                {
                    saw_negotiated = true;
                }
                Ok(Some(AppEvent::Telnet(TelnetTarget::World(name), TelnetEvent::GmcpMessage(pkg, json))))
                    if name == "gmcptest" =>
                {
                    saw_gmcp = Some((pkg, json));
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }

        assert!(saw_negotiated, "expected AppEvent::Telnet(.., OptionEnabled(TELNET_OPT_GMCP)) on the channel App reads from");
        let (pkg, json) = saw_gmcp.expect("expected AppEvent::Telnet(.., GmcpMessage(..)) on the channel App reads from");
        assert_eq!(pkg, "Core.Hello");
        assert_eq!(json, "{\"foo\":\"bar\"}");

        let _ = server.await;
    }
}

#[cfg(all(test, unix))] // the shell is a Unix command here
mod shell_world_connect_tests {
    //! A client connecting a shell world through this handler - `/worlds <name>` (what
    //! TF's `/connect` becomes), `/worlds -b <name>` and the `ConnectWorld` message - must
    //! start the shell, never a MUD connection to the world's empty address. Before
    //! `start_non_mud_world`, only `ConnectWorld` and a bare `/connect` knew the world's
    //! type; the other paths printed "Connecting to :..." and failed with "no host and
    //! port set" on a `-D` or GUI-master host, while the console's own path (commands.rs)
    //! started the shell. The native GUI's `/connect` reaches this handler as `/worlds`.
    use super::*;

    fn shell_app(tx: &mpsc::Sender<AppEvent>) -> App {
        let mut app = App::new();
        app.worlds.clear();
        let mut w = World::new("Sh");
        w.settings.world_type = WorldType::Shell;
        // The Command rides in `hostname`: one that exits at once, so no login shell is
        // left behind by the test.
        w.settings.hostname = "true".to_string();
        app.worlds.push(w);
        app.event_tx = Some(tx.clone());
        app.tf_event_tx = Some(tx.clone());
        app
    }

    async fn starts_the_shell(msg: WsMessage) {
        let (tx, _rx) = mpsc::channel::<AppEvent>(100);
        let mut app = shell_app(&tx);
        handle_daemon_ws_message(&mut app, 1, msg, &tx).await;
        let text: Vec<&str> = app.worlds[0].output_lines.iter().map(|l| l.text.as_str()).collect();
        assert!(!text.iter().any(|l| l.contains("Connecting to")), "no MUD connection for a shell world; output: {text:?}");
        #[cfg(feature = "shell-worlds")]
        {
            assert!(app.worlds[0].shell.is_some(), "the shell session was started; output: {text:?}");
            assert!(app.worlds[0].connected, "a shell world counts as connected once its shell runs");
            assert!(text.iter().any(|l| l.contains("Starting shell")), "the world says what happened; output: {text:?}");
            app.worlds[0].shell = None;
        }
        // A build without shell worlds keeps the world and says it can't run it.
        #[cfg(not(feature = "shell-worlds"))]
        {
            assert!(!app.worlds[0].connected, "nothing runs, so the world stays disconnected");
            assert!(!text.iter().any(|l| l.contains("Starting shell")), "no shell is started; output: {text:?}");
            assert!(text.iter().any(|l| l.contains(crate::shell::NOT_IN_THIS_BUILD)), "the world says why; output: {text:?}");
        }
    }

    #[tokio::test]
    async fn worlds_name_starts_the_shell() {
        starts_the_shell(WsMessage::SendCommand { world_index: 0, command: "/worlds Sh".to_string() }).await;
    }

    #[tokio::test]
    async fn worlds_background_starts_the_shell() {
        starts_the_shell(WsMessage::SendCommand { world_index: 0, command: "/worlds -b Sh".to_string() }).await;
    }

    #[tokio::test]
    async fn connect_command_starts_the_shell() {
        starts_the_shell(WsMessage::SendCommand { world_index: 0, command: "/connect".to_string() }).await;
    }

    #[tokio::test]
    async fn connect_world_message_starts_the_shell() {
        starts_the_shell(WsMessage::ConnectWorld { world_index: 0 }).await;
    }
}

#[cfg(test)]
mod client_parity_tests {
    //! A client of a server with no console of its own (handler B: the Android app's
    //! `--local-server`, the GUI masters, `-D`) gets what the console master's clients get.
    //! Client 0 is the embedded GUI channel (`gui_tx`), which these tests read.
    use super::*;

    fn app_with_gui() -> (App, tokio::sync::mpsc::UnboundedReceiver<WsMessage>, mpsc::Sender<AppEvent>) {
        let mut app = App::new();
        app.worlds.clear();
        app.worlds.push(World::new("w"));
        let (gui_tx, gui_rx) = tokio::sync::mpsc::unbounded_channel();
        app.gui_tx = Some(gui_tx);
        let (tx, _rx) = mpsc::channel::<AppEvent>(100);
        app.event_tx = Some(tx.clone());
        app.tf_event_tx = Some(tx.clone());
        (app, gui_rx, tx)
    }

    fn sent(rx: &mut tokio::sync::mpsc::UnboundedReceiver<WsMessage>) -> Vec<WsMessage> {
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    async fn typed(app: &mut App, tx: &mpsc::Sender<AppEvent>, line: &str) {
        handle_daemon_ws_message(app, 0, WsMessage::SendCommand { world_index: 0, command: line.into() }, tx).await;
    }

    /// `/reload --console|--gui` switches the main Clay only from its own window, and only
    /// to an interface it has to switch from; another client's is answered, not acted on.
    #[tokio::test]
    async fn a_reload_switch_is_taken_only_from_the_main_clays_own_window() {
        let (mut app, mut rx, _) = app_with_gui();
        let (tx, mut events) = mpsc::channel::<AppEvent>(100);
        let line = |text: &str| WsMessage::SendCommand { world_index: 0, command: text.into() };
        let says = |msgs: &[WsMessage], text: &str| msgs.iter()
            .any(|m| matches!(m, WsMessage::ServerData { data, .. } if data.contains(text)));
        // Another client (not the GUI's channel, not on the loopback).
        handle_daemon_ws_message(&mut app, 7, line("/reload --console"), &tx).await;
        assert!(events.try_recv().is_err(), "nothing reloads");
        // The main Clay's own window: judged against what this Clay runs (nothing, here).
        typed(&mut app, &tx, "/reload --console").await;
        assert!(events.try_recv().is_err());
        let msgs = sent(&mut rx);
        assert!(says(&msgs, "runs neither"), "{msgs:?}");
        // A word it doesn't know: the usage line.
        typed(&mut app, &tx, "/reload now").await;
        assert!(says(&sent(&mut rx), crate::RELOAD_USAGE));
        assert!(events.try_recv().is_err());
        // A plain /reload, from anyone, reloads.
        handle_daemon_ws_message(&mut app, 7, line("/reload"), &tx).await;
        assert!(matches!(events.try_recv(), Ok(AppEvent::Sigusr1Received)));
        assert_eq!(app.reload_ui, None);
    }

    fn says(msgs: &[WsMessage], text: &str) -> bool {
        msgs.iter().any(|m| matches!(m, WsMessage::ServerData { data, .. } if data.contains(text)))
    }

    /// A clay_multi user's Clay (`Policy::user()`) refuses what is the server operator's:
    /// /reload, /update, /sh, /ban, /reach, the port mapping, Change Password - each with
    /// a line to the asker, and none of it reaches the process.
    #[tokio::test]
    async fn a_users_clay_refuses_the_servers_own_commands() {
        let (mut app, mut rx, _) = app_with_gui();
        app.policy = crate::instance::Policy::user();
        let (tx, mut events) = mpsc::channel::<AppEvent>(100);
        typed(&mut app, &tx, "/reload").await;
        assert!(events.try_recv().is_err(), "nothing reloads");
        assert!(says(&sent(&mut rx), "operator reloads"));
        typed(&mut app, &tx, "/update").await;
        assert!(says(&sent(&mut rx), "operator updates it"));
        typed(&mut app, &tx, "/sh echo hi").await;
        assert!(says(&sent(&mut rx), "SH: restricted"));
        typed(&mut app, &tx, "/ban").await;
        assert!(says(&sent(&mut rx), "server operator's"));
        typed(&mut app, &tx, "/unban 1.2.3.4").await;
        assert!(says(&sent(&mut rx), "server operator's"));
        typed(&mut app, &tx, "/reach").await;
        assert!(says(&sent(&mut rx), "server operator's"));
        handle_daemon_ws_message(&mut app, 0, WsMessage::SetPortMapping { enabled: true }, &tx).await;
        assert!(matches!(app.port_map_status, crate::reach::PortMapStatus::Off), "the mapping is the operator's");
        handle_daemon_ws_message(&mut app, 0, WsMessage::UnbanRequest { host: "1.2.3.4".into() }, &tx).await;
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::UnbanResult { success: false, .. })));
        handle_daemon_ws_message(&mut app, 0, WsMessage::ChangePassword { old_password_hash: "a".into(), new_password_hash: "b".into() }, &tx).await;
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::PasswordChanged { success: false, error: Some(e) } if e.contains("password.dat"))));
        assert!(events.try_recv().is_err(), "no event reached the loop");
        // What is the user's own still works: a world of theirs, their TF.
        typed(&mut app, &tx, "/def -t'hello' greet = /echo hi").await;
        assert!(app.tf_engine.macros.iter().any(|m| m.name == "greet"), "the user's own TF engine");
    }

    /// A regular Clay (`Policy::MAIN`) answers Change Password with a reason of its own: it
    /// has no accounts.
    #[tokio::test]
    async fn a_single_user_clay_has_no_password_to_change() {
        let (mut app, mut rx, tx) = app_with_gui();
        handle_daemon_ws_message(&mut app, 0, WsMessage::ChangePassword { old_password_hash: "a".into(), new_password_hash: "b".into() }, &tx).await;
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::PasswordChanged { success: false, error: Some(e) } if e.contains("no accounts"))));
    }

    /// A clay_web visitor's Clay (`Policy::visitor()`): every typed line is text for its
    /// one world - a `/help` too - and a message outside the page's own is refused.
    #[tokio::test]
    async fn a_visitors_typed_line_is_text_and_other_messages_are_refused() {
        let (mut app, mut rx, tx) = app_with_gui();
        app.policy = crate::instance::Policy::visitor();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(16);
        app.worlds[0].command_tx = Some(cmd_tx);
        app.worlds[0].connected = true;
        typed(&mut app, &tx, "/help").await;
        assert!(matches!(cmd_rx.try_recv(), Ok(WriteCommand::Text(t)) if t == "/help"), "a /command is text");
        typed(&mut app, &tx, "look").await;
        assert!(matches!(cmd_rx.try_recv(), Ok(WriteCommand::Text(t)) if t == "look"));
        // Another world index is still world 0's text.
        handle_daemon_ws_message(&mut app, 0, WsMessage::SendCommand { world_index: 5, command: "say hi".into() }, &tx).await;
        assert!(matches!(cmd_rx.try_recv(), Ok(WriteCommand::Text(t)) if t == "say hi"));
        handle_daemon_ws_message(&mut app, 0, WsMessage::CreateWorld { name: "x".into() }, &tx).await;
        assert_eq!(app.worlds.len(), 1, "no world is created");
        assert!(says(&sent(&mut rx), "Not available on this page"));
        handle_daemon_ws_message(&mut app, 0, WsMessage::ExpandLine { text: "%{foo}".into() }, &tx).await;
        assert!(says(&sent(&mut rx), "Not available on this page"));
        // The page's own messages are answered as on any Clay.
        handle_daemon_ws_message(&mut app, 0, WsMessage::RequestInfo { world_index: Some(0) }, &tx).await;
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::InfoReport { .. })), "an allowed message is answered");
    }

    /// TF's /input, /xtitle and /limit run by a client act on that client's input line,
    /// title and filter (InputOps, SetTitle, LimitOp) - never on a console's.
    #[tokio::test]
    async fn a_clients_tf_input_title_and_limit_are_its_own() {
        let (mut app, mut rx, tx) = app_with_gui();
        typed(&mut app, &tx, "/input hello").await;
        let msgs = sent(&mut rx);
        assert!(msgs.iter().any(|m| matches!(m, WsMessage::InputOps { ops }
            if matches!(ops.as_slice(), [crate::websocket::InputOp::Insert { text, .. }] if text == "hello"))), "{msgs:?}");
        assert!(app.input.buffer.is_empty(), "the console's own input is untouched");
        assert!(app.tf_engine.pending_keyboard_ops.is_empty(), "nothing left for a console");

        typed(&mut app, &tx, "/xtitle My Title").await;
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::SetTitle { title } if title == "My Title")));

        typed(&mut app, &tx, "/limit orc").await;
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::LimitOp { action, pattern, .. } if action == "apply" && pattern == "orc")));
        typed(&mut app, &tx, "/unlimit").await;
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::LimitOp { action, .. } if action == "clear")));
    }

    /// A key binding's kb* functions read the input line the client sent with it.
    #[tokio::test]
    async fn a_key_binding_reads_the_clients_input_line() {
        let (mut app, mut rx, tx) = app_with_gui();
        typed(&mut app, &tx, "/bind ^X = /test kbhead()").await;
        let _ = sent(&mut rx);
        assert!(app.client_keyboard.is_none(), "restored after the command");
        let input = Some(crate::websocket::InputSnapshot { buffer: "abcdef".into(), cursor: 3 });
        handle_daemon_ws_message(&mut app, 0, WsMessage::RunKeyBinding { key: "^X".into(), kbnum: None, input }, &tx).await;
        assert_eq!(app.tf_engine.keyboard_state.buffer, "abcdef");
        assert_eq!(app.tf_engine.keyboard_state.cursor_position, 3);
        assert!(app.client_keyboard.is_none());
    }

    /// A user macro named like a Clay command runs for a client, as at the console (TF
    /// first); `/help <topic>` goes to the client that asked, not into the world.
    #[tokio::test]
    async fn typed_lines_go_through_tf_as_at_the_console() {
        let (mut app, mut rx, tx) = app_with_gui();
        typed(&mut app, &tx, "/def flush = /echo the macro ran").await;
        typed(&mut app, &tx, "/flush").await;
        assert!(app.worlds[0].output_lines.iter().any(|l| l.text.contains("the macro ran")), "the macro, not Clay's /flush");
        let _ = sent(&mut rx);

        let before = app.worlds[0].output_lines.len();
        typed(&mut app, &tx, "/help send").await;
        assert_eq!(app.worlds[0].output_lines.len(), before, "help is not stored in the world");
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::ServerData { data, .. } if !data.is_empty())), "help to the asker");
    }

    /// A client's typed `/worlds` (deferred bug G1): the server has no console, so nothing
    /// claims the world's unviewed lines before the client's own `MarkWorldSeen` does, the
    /// requester alone is told, and the server's foreground world follows.
    #[tokio::test]
    async fn a_typed_worlds_leaves_the_new_lines_for_the_client_to_claim() {
        let (mut app, mut rx, tx) = app_with_gui();
        let mut bravo = World::new("Bravo");
        bravo.output_lines.push(OutputLine::new("arrived while nobody looked".into(), 1));
        bravo.output_lines.push(OutputLine::new("and this".into(), 2));
        app.worlds.push(bravo);
        app.ws_client_worlds.insert(0, crate::ClientViewState {
            world_index: 0, history: Vec::new(), visible_lines: 24, visible_columns: 80,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });
        assert!(!app.console_tui, "a headless server");
        typed(&mut app, &tx, "/worlds Bravo").await;
        assert_eq!(app.current_world_index, 1, "the foreground world follows");
        assert_eq!(app.ws_client_worlds[&0].world_index, 1, "and the client's view");
        let msgs = sent(&mut rx);
        assert!(msgs.iter().any(|m| matches!(m, WsMessage::WorldSwitched { new_index: 1 })), "{msgs:?}");
        assert!(msgs.iter().any(|m| matches!(m, WsMessage::ExecuteLocalCommand { command } if command == "/world Bravo")), "{msgs:?}");
        assert!(app.worlds[1].output_lines.iter().all(|l| !l.viewed && l.display_id.is_none()),
            "no console took the lines: {:?}", app.worlds[1].output_lines.iter().map(|l| (l.viewed, l.display_id)).collect::<Vec<_>>());
        // The client starts displaying Bravo: its claim is answered with every line.
        handle_daemon_ws_message(&mut app, 0, WsMessage::MarkWorldSeen { world_index: 1, previous_world_index: Some(0), other_views: None }, &tx).await;
        let me = app.display_owner_id(0);
        assert!(app.worlds[1].output_lines.iter().all(|l| l.display_id == Some(me)), "the client owns them");
        assert!(sent(&mut rx).iter().any(|m| matches!(m, WsMessage::ClaimedNew { world_index: 1, seqs } if seqs == &vec![1, 2])));
    }

    /// Backgrounding (Android) reaches this handler: the client stops counting as a viewer.
    #[tokio::test]
    async fn client_visibility_reaches_this_handler() {
        let (mut app, _rx, tx) = app_with_gui();
        app.ws_client_worlds.insert(5, crate::ClientViewState {
            world_index: 0, history: Vec::new(), visible_lines: 0, visible_columns: 0,
            dimensions: None, paused: false, visible: true, disconnected_at: None,
        });
        handle_daemon_ws_message(&mut app, 5, WsMessage::ClientVisibility { visible: false }, &tx).await;
        assert!(!app.ws_client_worlds[&5].visible);
    }

    /// An MCP edit on a server with no console reaches the clients each time; no invisible
    /// console editor is left to refuse the next one.
    #[tokio::test]
    async fn every_mcp_edit_reaches_the_clients() {
        let (mut app, mut rx, _tx) = app_with_gui();
        for content in ["one", "two"] {
            app.handle_mcp_event(0, crate::mcp::McpEvent::SimpleEditContent {
                reference: "#1".into(), name: "R".into(), edit_type: crate::mcp::SimpleEditType::Str, content: content.into(),
            });
        }
        let opens = sent(&mut rx).into_iter().filter(|m| matches!(m, WsMessage::McpEditOpen { .. })).count();
        assert_eq!(opens, 2);
        assert!(!app.editor.visible);
    }
}
