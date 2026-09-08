// Test harness module - in-process test framework for regression testing Clay
// Creates an App, connects to test servers, processes events, and captures test outcomes

use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::{
    App, World, OutputLine, WriteCommand, AutoConnectType, AppEvent,
    build_display_lines,
};
use crate::encoding::Encoding;
use crate::telnet::{StreamReader, StreamWriter, TelnetConfig, TelnetEvent};
use crate::telnet_reader::{spawn_telnet_reader, TelnetTarget};
use crate::telnet_writer::spawn_telnet_writer;
use crate::websocket::WsMessage;

/// The ▶ ownership id used for the harness's single simulated WebSocket client. Distinct
/// from `CONSOLE_DISPLAY_ID` so scenarios can tell console-owned markers from client-owned.
pub const HARNESS_WS_CLIENT_ID: u64 = 1;

/// Events captured during test execution
#[derive(Debug, Clone, PartialEq)]
pub enum TestEvent {
    /// World connected successfully
    Connected(String),
    /// World disconnected
    Disconnected(String),
    /// Text line received on a world
    TextReceived(String, String),
    /// More-mode triggered (world name, pending lines count)
    MoreTriggered(String, usize),
    /// More-mode released (world name)
    MoreReleased(String),
    /// Activity count changed
    ActivityChanged(usize),
    /// Unseen count changed for a world
    UnseenChanged(String, usize),
    /// Current world switched
    WorldSwitched(String),
    /// Auto-login text sent to server
    AutoLoginSent(String, String),
    /// Prompt received on a world
    PromptReceived(String, String),
    /// WS broadcast: ActivityUpdate { count }
    WsBroadcastActivity(usize),
    /// WS broadcast: UnseenUpdate { world_index, count }
    WsBroadcastUnseen(usize, usize),
    /// WS broadcast: PendingLinesUpdate { world_index, count }
    WsBroadcastPending(usize, usize),
    /// WS broadcast: PendingReleased { world_index, count }
    WsBroadcastReleased(usize, usize),
    /// WS broadcast: UnseenCleared { world_index }
    WsBroadcastUnseenCleared(usize),
    /// WS broadcast: ServerData { world_index } (content only - the ▶ new-text watermark is
    /// no longer part of ServerData, see WsClaimedNew)
    WsBroadcastServerData(usize),
    /// WS send: ClaimedNew - (world_index, number of lines claimed). Per-client, not a
    /// broadcast; see WsMessage::ClaimedNew's doc comment in websocket.rs.
    WsClaimedNew(usize, usize),
    /// Telnet negotiation/protocol events (plan Job 4,
    /// investigate-differences-between-tinyfugu-fluffy-stallman.md). Only observable now
    /// that the harness answers negotiation via `spawn_telnet_reader` instead of
    /// discarding `result.responses` entirely.
    TelnetDetected(String),
    NawsRequested(String),
    TtypeRequested(String),
    CharsetRequested(String, Vec<String>),
    GmcpReceived(String, String, String),
    MsdpReceived(String, String, String),
}

/// State checks for AssertState action
#[derive(Debug, Clone)]
pub enum StateCheck {
    /// Assert world.unseen_lines == expected
    UnseenLines(usize),
    /// Assert world.paused == expected
    Paused(bool),
    /// Assert world.pending_lines.len() == expected
    PendingCount(usize),
    /// Assert world.output_lines.len() == expected
    OutputLineCount(usize),
    /// Assert app.activity_count() == expected (world_name ignored)
    ActivityCount(usize),
}

/// Configuration for a test world
pub struct TestWorldConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub use_ssl: bool,
    pub auto_login_type: AutoConnectType,
    pub username: String,
    pub password: String,
    /// `TelnetSession` configuration for this world's connection, threaded through to
    /// `spawn_telnet_reader` (plan Job 4). Most scenarios just want
    /// `TelnetConfig::default()`.
    pub telnet_config: TelnetConfig,
}

/// Overall test configuration
pub struct TestConfig {
    pub worlds: Vec<TestWorldConfig>,
    pub output_height: u16,
    pub output_width: u16,
    pub more_mode_enabled: bool,
    pub max_duration: Duration,
}

/// Actions the test can inject mid-scenario
#[derive(Debug, Clone)]
pub enum TestAction {
    /// Simulate Tab press (release one screenful)
    TabRelease,
    /// Simulate Escape+j (release all pending)
    JumpToEnd,
    /// Switch to named world
    SwitchWorld(String),
    /// Wait for a specific event type to appear
    WaitForEvent(WaitCondition),
    /// Wait a fixed time
    Sleep(Duration),
    /// Send a command as if the user typed it
    SendCommand(String),
    /// Simulate WS client releasing pending lines for a world
    WsReleasePending { world_name: String, count: usize },
    /// Simulate WS client marking a world as seen
    WsMarkWorldSeen(String),
    /// Simulate WS client sending a command to a world
    WsSendCommand { world_name: String, command: String },
    /// Assert count of marked_new lines (output_lines + pending_lines)
    AssertMarkedNew { world_name: String, expected_count: usize },
    /// Assert a specific world/app state value
    AssertState { world_name: String, check: StateCheck },
    /// Assert properties of the rendered display output (what the user would see)
    AssertDisplay {
        world_name: String,
        visible_height: usize,
        term_width: usize,
        /// Expected total visible lines (if Some)
        line_count: Option<usize>,
        /// Last displayed line must contain this substring (if Some)
        last_line_contains: Option<String>,
        /// First displayed line must contain this substring (if Some)
        first_line_contains: Option<String>,
        /// Expected count of leading non-new (old context) lines (if Some)
        old_context_count: Option<usize>,
    },
}

/// Conditions to wait for
#[derive(Debug, Clone)]
pub enum WaitCondition {
    /// Wait until any MoreTriggered event appears
    MoreTriggered,
    /// Wait until any Disconnected event appears
    Disconnected,
    /// Wait until N TextReceived events have been captured
    TextReceivedCount(usize),
    /// Wait until any Connected event appears for a specific world
    Connected(String),
    /// Wait until all worlds are connected
    AllConnected,
}

/// Internal state for tracking a world's connection in the test harness
struct TestWorldConnection {
    /// Reader task handle
    _reader_handle: tokio::task::JoinHandle<()>,
    /// Command sender for the writer task
    cmd_tx: mpsc::Sender<WriteCommand>,
}

/// Channel message from reader tasks to the harness. This is deliberately its own
/// (smaller) type rather than `AppEvent` itself: `spawn_telnet_reader` (plan Job 4) always
/// speaks `AppEvent` — it's the same reader production code will eventually use — but
/// `run_test_scenario`'s event loop below only cares about the single-world-target subset
/// of it, via `app_event_to_reader_event`.
enum ReaderEvent {
    Data(String, Vec<u8>),    // world_name, cleaned bytes (telnet processed)
    Disconnected(String),      // world_name
    Prompt(String, Vec<u8>),   // world_name, prompt bytes
    /// Telnet negotiation/protocol events (plan Job 4) - the harness could not observe
    /// any of these before migrating onto `spawn_telnet_reader`, since its old hand-rolled
    /// reader loop discarded `result.responses` (and every event besides prompt/data)
    /// entirely.
    TelnetDetected(String),
    NawsRequested(String),
    TtypeRequested(String),
    CharsetRequested(String, Vec<String>),
    Gmcp(String, String, String),   // world_name, package, json_data
    Msdp(String, String, String),   // world_name, variable, value_json
}

/// Bridge from `spawn_telnet_reader`'s `AppEvent`s to the harness's own `ReaderEvent`.
/// `AppEvent` carries every event shape in the whole app (WebSocket, multiuser, media,
/// ...); the harness only ever spawns `TelnetTarget::World` readers, so only the
/// single-user world-shaped variants are relevant here - everything else is `None`
/// deliberately (unlike `telnet_reader::to_app_event`, this is test-only glue code, not
/// the anti-drift mapper, so a plain wildcard is fine).
fn app_event_to_reader_event(ev: AppEvent) -> Option<ReaderEvent> {
    match ev {
        AppEvent::ServerData(world_name, bytes) => Some(ReaderEvent::Data(world_name, bytes)),
        AppEvent::Disconnected(world_name, _conn_id) => Some(ReaderEvent::Disconnected(world_name)),
        AppEvent::Prompt(world_name, bytes) => Some(ReaderEvent::Prompt(world_name, bytes)),
        // AppEvent::Telnet (plan Phase 2, Step 2.7) replaced the nine formerly-separate
        // single-user telnet AppEvent variants this bridge used to match directly; unwrap
        // it here to the same TelnetTarget::World-only subset as before (a plain wildcard
        // covers the rest below, same as always).
        AppEvent::Telnet(TelnetTarget::World(world_name), ev) => match ev {
            TelnetEvent::TelnetDetected => Some(ReaderEvent::TelnetDetected(world_name)),
            TelnetEvent::NawsRequested => Some(ReaderEvent::NawsRequested(world_name)),
            TelnetEvent::TtypeRequested => Some(ReaderEvent::TtypeRequested(world_name)),
            TelnetEvent::CharsetRequest(charsets) => {
                Some(ReaderEvent::CharsetRequested(world_name, charsets))
            }
            TelnetEvent::GmcpMessage(package, json) => {
                Some(ReaderEvent::Gmcp(world_name, package, json))
            }
            TelnetEvent::MsdpVariable(variable, value) => {
                Some(ReaderEvent::Msdp(world_name, variable, value))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Run a test scenario with the given configuration and actions.
/// Returns all captured events.
pub async fn run_test_scenario(
    config: TestConfig,
    actions: Vec<TestAction>,
) -> Vec<TestEvent> {
    let mut app = App::new();
    app.settings.more_mode_enabled = config.more_mode_enabled;
    app.output_height = config.output_height;
    app.output_width = config.output_width;

    // Create worlds from config
    for wc in &config.worlds {
        let mut world = World::new(&wc.name);
        world.settings.hostname = wc.host.clone();
        world.settings.port = wc.port.to_string();
        world.settings.use_ssl = wc.use_ssl;
        world.settings.auto_connect_type = wc.auto_login_type;
        world.settings.user = wc.username.clone();
        world.settings.password = wc.password.clone();
        world.showing_splash = false;
        app.worlds.push(world);
    }

    let mut events: Vec<TestEvent> = Vec::new();
    let mut connections: Vec<Option<TestWorldConnection>> = Vec::new();
    for _ in 0..config.worlds.len() {
        connections.push(None);
    }

    // Channel for reader events
    let (reader_tx, mut reader_rx) = mpsc::unbounded_channel::<ReaderEvent>();

    // Connect all worlds
    for (idx, wc) in config.worlds.iter().enumerate() {
        let addr = format!("{}:{}", wc.host, wc.port);
        match TcpStream::connect(&addr).await {
            Ok(tcp_stream) => {
                let (read_half, write_half) = tcp_stream.into_split();
                let world_name = wc.name.clone();

                // Writer task. Plan Job 13 (Phase 4, 4.4): migrated onto
                // spawn_telnet_writer, same as every production writer loop - the
                // harness gets IAC-escaping and (should a scenario ever need it)
                // CHARSET-driven encoding switches for free instead of a fourteenth
                // hand-rolled copy. spawn_telnet_reader needs a clone of the returned
                // Sender too: telnet negotiation replies (outcome.wire) go out through
                // the same socket as user-issued commands, via the same writer task.
                // No scenario here exercises a non-UTF-8 world, so Encoding::Utf8 is
                // the harness's fixed initial encoding.
                let cmd_tx = spawn_telnet_writer(StreamWriter::Plain(write_half), Encoding::Utf8);

                // Bridge AppEvents from spawn_telnet_reader (plan Job 4) into the
                // harness's own ReaderEvent channel - see app_event_to_reader_event's
                // doc comment for why this indirection exists.
                let (app_event_tx, mut app_event_rx) = mpsc::channel::<AppEvent>(64);
                let bridge_tx = reader_tx.clone();
                tokio::spawn(async move {
                    while let Some(ev) = app_event_rx.recv().await {
                        if let Some(re) = app_event_to_reader_event(ev) {
                            let _ = bridge_tx.send(re);
                        }
                    }
                });

                // Spawn the real reader (plan Job 4). This is what makes the harness
                // stop discarding telnet negotiation replies (outcome.wire) and every
                // protocol event besides prompt/data - which its old hand-rolled loop
                // above did entirely.
                let reader_handle = spawn_telnet_reader(
                    StreamReader::Plain(read_half),
                    cmd_tx.clone(),
                    app_event_tx,
                    TelnetTarget::World(world_name.clone()),
                    1, // conn_id: the harness does not model reconnects today
                    wc.telnet_config.clone(),
                );

                // Mark world as connected
                app.worlds[idx].connected = true;
                app.worlds[idx].was_connected = true;
                app.worlds[idx].command_tx = Some(cmd_tx.clone());
                events.push(TestEvent::Connected(wc.name.clone()));

                // Handle auto-login for Connect type
                if !wc.username.is_empty() && !wc.password.is_empty()
                    && wc.auto_login_type == AutoConnectType::Connect
                {
                    let connect_cmd = format!("connect {} {}", wc.username, wc.password);
                    let _ = cmd_tx.try_send(WriteCommand::Text(connect_cmd.clone()));
                    events.push(TestEvent::AutoLoginSent(wc.name.clone(), connect_cmd));
                }

                connections[idx] = Some(TestWorldConnection {
                    _reader_handle: reader_handle,
                    cmd_tx,
                });
            }
            Err(e) => {
                panic!("Failed to connect world {} to {}: {}", wc.name, addr, e);
            }
        }
    }

    // Track state for change detection
    let mut prev_activity = app.activity_count();
    let mut prev_unseen: Vec<usize> = app.worlds.iter().map(|w| w.unseen_lines).collect();
    let mut prev_paused: Vec<bool> = app.worlds.iter().map(|w| w.paused).collect();
    // Track simulated WS client's current world (for MarkWorldSeen indicator clearing)
    let mut ws_client_world: Option<usize> = None;

    // Process actions and events
    let mut action_iter = actions.into_iter().peekable();
    let deadline = tokio::time::Instant::now() + config.max_duration;

    loop {
        // Check timeout
        if tokio::time::Instant::now() >= deadline {
            break;
        }

        // Process next action if available
        if let Some(action) = action_iter.peek() {
            match action {
                TestAction::TabRelease => {
                    let idx = app.current_world_index;
                    let visual_budget = (app.output_height as usize).saturating_sub(2);
                    let output_width = app.output_width as usize;
                    {
                        let metrics = crate::rendering::RowMetrics::new(&app.settings, app.show_tags, output_width.max(1));
                        app.worlds[idx].release_pending(visual_budget, &metrics)
                    };
                    action_iter.next();
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::JumpToEnd => {
                    let idx = app.current_world_index;
                    app.worlds[idx].release_all_pending();
                    action_iter.next();
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::SwitchWorld(name) => {
                    let name = name.clone();
                    action_iter.next();
                    if let Some(idx) = app.find_world_index(&name) {
                        // Mark current world as seen (like the console renderer does)
                        app.worlds[app.current_world_index].mark_seen();
                        app.switch_world(idx);
                        // Mark new world as seen
                        app.worlds[idx].mark_seen();
                        events.push(TestEvent::WorldSwitched(name));
                    }
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::SendCommand(cmd) => {
                    let cmd = cmd.clone();
                    action_iter.next();
                    let idx = app.current_world_index;
                    if let Some(conn) = &connections[idx] {
                        let _ = conn.cmd_tx.try_send(WriteCommand::Text(cmd));
                    }
                    // Reset lines_since_pause on user command (like the real client does)
                    app.worlds[idx].lines_since_pause = 0;
                    continue;
                }
                TestAction::Sleep(dur) => {
                    let dur = *dur;
                    action_iter.next();
                    tokio::time::sleep(dur).await;
                    continue;
                }
                TestAction::WaitForEvent(condition) => {
                    let met = check_wait_condition(condition, &events, &app);
                    if met {
                        action_iter.next();
                        continue;
                    }
                    // Fall through to process more reader events
                }
                TestAction::WsReleasePending { world_name, count } => {
                    let world_name = world_name.clone();
                    let count = *count;
                    action_iter.next();
                    if let Some(idx) = app.find_world_index(&world_name) {
                        // Use the App method that also broadcasts WS messages
                        let old_current = app.current_world_index;
                        app.current_world_index = idx;
                        app.release_pending_screenful();
                        app.current_world_index = old_current;
                        let _ = count; // count parameter available for future use
                    }
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::WsMarkWorldSeen(name) => {
                    let name = name.clone();
                    action_iter.next();
                    if let Some(idx) = app.find_world_index(&name) {
                        // Release this simulated client's own ▶ markers on the world it left,
                        // then claim the new one - mirroring App::handle_mark_world_seen.
                        if let Some(old_idx) = ws_client_world {
                            if old_idx != idx && old_idx < app.worlds.len() {
                                app.worlds[old_idx].release_claims(HARNESS_WS_CLIENT_ID);
                            }
                        }
                        ws_client_world = Some(idx);
                        app.worlds[idx].claim_unviewed(HARNESS_WS_CLIENT_ID);
                        app.worlds[idx].mark_seen();
                        // Broadcast UnseenCleared like the real WS handler does
                        app.ws_broadcast(WsMessage::UnseenCleared { world_index: idx });
                        app.broadcast_activity();
                    }
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::WsSendCommand { world_name, command } => {
                    let world_name = world_name.clone();
                    let command = command.clone();
                    action_iter.next();
                    if let Some(idx) = app.find_world_index(&world_name) {
                        // Reset lines_since_pause (like the real WS command handler)
                        app.worlds[idx].lines_since_pause = 0;
                        if let Some(conn) = &connections[idx] {
                            let _ = conn.cmd_tx.try_send(WriteCommand::Text(command));
                        }
                    }
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::AssertMarkedNew { world_name, expected_count } => {
                    let world_name = world_name.clone();
                    let expected_count = *expected_count;
                    action_iter.next();
                    if let Some(idx) = app.find_world_index(&world_name) {
                        let world = &app.worlds[idx];
                        // "New to somebody, or not yet shown to anybody": a line counts if it
                        // is currently owned (renders ▶ for that viewer) OR is still unviewed
                        // (will be claimed by whoever displays the world next). Scenario-level
                        // assertions here are about how much genuinely-new text a world is
                        // holding, not about whose marker it is; the per-owner distinction is
                        // covered by the dedicated ownership tests in tests.rs.
                        let is_new = |l: &&OutputLine| l.display_id.is_some() || !l.viewed;
                        let output_new = world.output_lines.iter().filter(is_new).count();
                        let pending_new = world.pending_lines.iter().filter(is_new).count();
                        let actual = output_new + pending_new;
                        assert_eq!(actual, expected_count,
                            "World '{}': expected {} marked_new (▶) lines, got {} (output: {}, pending: {})",
                            world_name, expected_count, actual, output_new, pending_new);
                    } else {
                        panic!("AssertMarkedNew: world '{}' not found", world_name);
                    }
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::AssertState { world_name, check } => {
                    let world_name = world_name.clone();
                    let check = check.clone();
                    action_iter.next();
                    match &check {
                        StateCheck::ActivityCount(expected) => {
                            let actual = app.activity_count();
                            assert_eq!(actual, *expected,
                                "Expected activity count {}, got {}", expected, actual);
                        }
                        _ => {
                            if let Some(idx) = app.find_world_index(&world_name) {
                                match &check {
                                    StateCheck::UnseenLines(expected) => {
                                        assert_eq!(app.worlds[idx].unseen_lines, *expected,
                                            "World '{}': expected unseen_lines {}, got {}",
                                            world_name, expected, app.worlds[idx].unseen_lines);
                                    }
                                    StateCheck::Paused(expected) => {
                                        assert_eq!(app.worlds[idx].paused, *expected,
                                            "World '{}': expected paused={}, got {}",
                                            world_name, expected, app.worlds[idx].paused);
                                    }
                                    StateCheck::PendingCount(expected) => {
                                        assert_eq!(app.worlds[idx].pending_lines.len(), *expected,
                                            "World '{}': expected pending_count {}, got {}",
                                            world_name, expected, app.worlds[idx].pending_lines.len());
                                    }
                                    StateCheck::OutputLineCount(expected) => {
                                        assert_eq!(app.worlds[idx].output_lines.len(), *expected,
                                            "World '{}': expected output_line_count {}, got {}",
                                            world_name, expected, app.worlds[idx].output_lines.len());
                                    }
                                    StateCheck::ActivityCount(_) => unreachable!(),
                                }
                            } else {
                                panic!("AssertState: world '{}' not found", world_name);
                            }
                        }
                    }
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
                TestAction::AssertDisplay {
                    world_name, visible_height, term_width,
                    line_count, last_line_contains, first_line_contains, old_context_count,
                } => {
                    let world_name = world_name.clone();
                    let visible_height = *visible_height;
                    let term_width = *term_width;
                    let line_count = *line_count;
                    let last_line_contains = last_line_contains.clone();
                    let first_line_contains = first_line_contains.clone();
                    let old_context_count = *old_context_count;
                    action_iter.next();
                    if let Some(idx) = app.find_world_index(&world_name) {
                        let display = build_display_lines(
                            &app.worlds[idx],
                            &app.settings,
                            visible_height,
                            term_width,
                            false, // show_tags
                        );
                        if let Some(expected) = line_count {
                            assert_eq!(display.len(), expected,
                                "AssertDisplay '{}': expected {} display lines, got {} (lines: {:?})",
                                world_name, expected, display.len(),
                                display.iter().map(|d| &d.text).collect::<Vec<_>>());
                        }
                        if let Some(ref substr) = last_line_contains {
                            let last = display.last()
                                .expect("AssertDisplay: no display lines to check last_line_contains");
                            assert!(last.text.contains(substr),
                                "AssertDisplay '{}': last line {:?} does not contain {:?}",
                                world_name, last.text, substr);
                        }
                        if let Some(ref substr) = first_line_contains {
                            let first = display.first()
                                .expect("AssertDisplay: no display lines to check first_line_contains");
                            assert!(first.text.contains(substr),
                                "AssertDisplay '{}': first line {:?} does not contain {:?}",
                                world_name, first.text, substr);
                        }
                        if let Some(expected_old) = old_context_count {
                            let actual_old = display.iter()
                                .take_while(|d| !d.marked_new)
                                .count();
                            assert_eq!(actual_old, expected_old,
                                "AssertDisplay '{}': expected {} old context lines, got {}",
                                world_name, expected_old, actual_old);
                        }
                    } else {
                        panic!("AssertDisplay: world '{}' not found", world_name);
                    }
                    check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                    continue;
                }
            }
        } else {
            // No more actions - check if all connections are done
            let all_disconnected = app.worlds.iter().all(|w| !w.connected);
            if all_disconnected {
                break;
            }
        }

        // Process reader events with a short timeout
        let timeout_dur = Duration::from_millis(50);
        match tokio::time::timeout(timeout_dur, reader_rx.recv()).await {
            Ok(Some(reader_event)) => {
                match reader_event {
                    ReaderEvent::Data(world_name, bytes) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            let height = app.output_height;
                            let width = app.output_width;

                            // Track state before processing
                            let output_before = app.worlds[idx].output_lines.len();
                            let pending_before = app.worlds[idx].pending_lines.len();

                            let _cmds = app.process_server_data(idx, &bytes, height, width, false);

                            // Capture TextReceived events for new output lines
                            let output_after = app.worlds[idx].output_lines.len();
                            for i in output_before..output_after {
                                let line_text = app.worlds[idx].output_lines[i].text.clone();
                                if !line_text.is_empty() {
                                    events.push(TestEvent::TextReceived(
                                        world_name.clone(),
                                        line_text,
                                    ));
                                }
                            }

                            // Capture TextReceived for new pending lines too
                            let pending_after = app.worlds[idx].pending_lines.len();
                            for i in pending_before..pending_after {
                                let line_text = app.worlds[idx].pending_lines[i].text.clone();
                                if !line_text.is_empty() {
                                    events.push(TestEvent::TextReceived(
                                        world_name.clone(),
                                        line_text,
                                    ));
                                }
                            }

                            check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                        }
                    }
                    ReaderEvent::Disconnected(world_name) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            app.worlds[idx].connected = false;
                            app.worlds[idx].command_tx = None;
                            connections[idx] = None;
                            events.push(TestEvent::Disconnected(world_name));
                            check_state_changes(&mut app, &mut events, &mut prev_activity, &mut prev_unseen, &mut prev_paused);
                        }
                    }
                    ReaderEvent::Prompt(world_name, prompt_bytes) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            let prompt_text = String::from_utf8_lossy(&prompt_bytes).to_string();
                            // Normalize: strip trailing spaces, add one
                            let normalized = format!("{} ", prompt_text.trim_end());
                            app.worlds[idx].prompt = normalized.clone();
                            app.worlds[idx].prompt_count += 1;
                            events.push(TestEvent::PromptReceived(world_name.clone(), normalized.clone()));

                            // Handle prompt-based auto-login
                            let auto_type = app.worlds[idx].settings.auto_connect_type;
                            let user = app.worlds[idx].settings.user.clone();
                            let password = app.worlds[idx].settings.password.clone();
                            let prompt_num = app.worlds[idx].prompt_count;

                            if !user.is_empty() && !password.is_empty() {
                                let cmd_to_send = match auto_type {
                                    AutoConnectType::Prompt => {
                                        match prompt_num {
                                            1 => Some(user),
                                            2 => Some(password),
                                            _ => None,
                                        }
                                    }
                                    AutoConnectType::MooPrompt => {
                                        match prompt_num {
                                            1 => Some(user.clone()),
                                            2 => Some(password),
                                            3 => Some(user),
                                            _ => None,
                                        }
                                    }
                                    AutoConnectType::Connect | AutoConnectType::NoLogin => None,
                                };

                                if let Some(cmd) = cmd_to_send {
                                    app.worlds[idx].prompt.clear();
                                    if let Some(conn) = &connections[idx] {
                                        let _ = conn.cmd_tx.try_send(WriteCommand::Text(cmd.clone()));
                                    }
                                    events.push(TestEvent::AutoLoginSent(world_name, cmd));
                                }
                            }
                        }
                    }
                    ReaderEvent::TelnetDetected(world_name) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            app.handle_telnet_detected(idx);
                            events.push(TestEvent::TelnetDetected(world_name));
                        }
                    }
                    ReaderEvent::NawsRequested(world_name) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            // Job 9 (T3.2): handle_naws_requested was deleted - its body
                            // now lives in ProtocolState::apply_telnet_event, reached via
                            // the unified handle_telnet_event dispatch, same as every
                            // production caller.
                            app.handle_telnet_event(idx, &TelnetEvent::NawsRequested);
                            events.push(TestEvent::NawsRequested(world_name));
                        }
                    }
                    ReaderEvent::TtypeRequested(world_name) => {
                        // Job 12 (plan Phase 4, 4.1): App no longer answers TTYPE
                        // itself - the TelnetSession already put the MTTS-cycled
                        // reply on the wire before this event was even produced
                        // (see handle_telnet_event's TtypeRequested arm).
                        if app.find_world_index(&world_name).is_some() {
                            events.push(TestEvent::TtypeRequested(world_name));
                        }
                    }
                    ReaderEvent::CharsetRequested(world_name, charsets) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            // Job 9: handle_charset_requested was deleted - see the
                            // NawsRequested arm above.
                            app.handle_telnet_event(idx, &TelnetEvent::CharsetRequest(charsets.clone()));
                            events.push(TestEvent::CharsetRequested(world_name, charsets));
                        }
                    }
                    ReaderEvent::Gmcp(world_name, package, json) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            // Job 9: handle_gmcp_received's store+broadcast body moved
                            // into ProtocolState::apply_gmcp_message - see the
                            // NawsRequested arm above.
                            app.handle_telnet_event(idx, &TelnetEvent::GmcpMessage(package.clone(), json.clone()));
                            events.push(TestEvent::GmcpReceived(world_name, package, json));
                        }
                    }
                    ReaderEvent::Msdp(world_name, variable, value) => {
                        if let Some(idx) = app.find_world_index(&world_name) {
                            // Job 9: handle_msdp_received's store+broadcast body moved
                            // into ProtocolState::apply_msdp_variable - see the
                            // NawsRequested arm above.
                            app.handle_telnet_event(idx, &TelnetEvent::MsdpVariable(variable.clone(), value.clone()));
                            events.push(TestEvent::MsdpReceived(world_name, variable, value));
                        }
                    }
                }
            }
            Ok(None) => {
                // Channel closed - all reader tasks done
                break;
            }
            Err(_) => {
                // Timeout - loop back to check actions
            }
        }
    }

    // Shut down any remaining connections
    for conn in connections.iter_mut().flatten() {
        let _ = conn.cmd_tx.try_send(WriteCommand::Shutdown);
    }

    events
}

/// Check for state changes and emit appropriate events
fn check_state_changes(
    app: &mut App,
    events: &mut Vec<TestEvent>,
    prev_activity: &mut usize,
    prev_unseen: &mut [usize],
    prev_paused: &mut [bool],
) {
    // Check activity count changes
    let current_activity = app.activity_count();
    if current_activity != *prev_activity {
        events.push(TestEvent::ActivityChanged(current_activity));
        *prev_activity = current_activity;
    }

    // Check per-world unseen and pause changes
    for (idx, world) in app.worlds.iter().enumerate() {
        if idx < prev_unseen.len() {
            let current_unseen = world.unseen_lines;
            if current_unseen != prev_unseen[idx] {
                events.push(TestEvent::UnseenChanged(world.name.clone(), current_unseen));
                prev_unseen[idx] = current_unseen;
            }
        }

        if idx < prev_paused.len() {
            let current_paused = world.paused;
            if current_paused && !prev_paused[idx] {
                // Just became paused
                events.push(TestEvent::MoreTriggered(
                    world.name.clone(),
                    world.pending_lines.len(),
                ));
            } else if !current_paused && prev_paused[idx] {
                // Just became unpaused
                events.push(TestEvent::MoreReleased(world.name.clone()));
            }
            prev_paused[idx] = current_paused;
        }
    }

    // Drain ws_broadcast_log and convert to TestEvent variants
    if let Ok(mut log) = app.ws_broadcast_log.lock() {
        for msg in log.drain(..) {
            match msg {
                WsMessage::ActivityUpdate { count } => {
                    events.push(TestEvent::WsBroadcastActivity(count));
                }
                WsMessage::UnseenUpdate { world_index, count } => {
                    events.push(TestEvent::WsBroadcastUnseen(world_index, count));
                }
                WsMessage::PendingLinesUpdate { world_index, count } => {
                    events.push(TestEvent::WsBroadcastPending(world_index, count));
                }
                WsMessage::PendingReleased { world_index, count } => {
                    events.push(TestEvent::WsBroadcastReleased(world_index, count));
                }
                WsMessage::UnseenCleared { world_index } => {
                    events.push(TestEvent::WsBroadcastUnseenCleared(world_index));
                }
                WsMessage::ServerData { world_index, .. } => {
                    events.push(TestEvent::WsBroadcastServerData(world_index));
                }
                WsMessage::ClaimedNew { world_index, ref seqs } => {
                    events.push(TestEvent::WsClaimedNew(world_index, seqs.len()));
                }
                _ => {
                    // Other WsMessage variants are not tracked
                }
            }
        }
    }
}

/// Check if a wait condition is met
fn check_wait_condition(
    condition: &WaitCondition,
    events: &[TestEvent],
    app: &App,
) -> bool {
    match condition {
        WaitCondition::MoreTriggered => {
            events.iter().any(|e| matches!(e, TestEvent::MoreTriggered(_, _)))
        }
        WaitCondition::Disconnected => {
            events.iter().any(|e| matches!(e, TestEvent::Disconnected(_)))
        }
        WaitCondition::TextReceivedCount(n) => {
            let count = events.iter().filter(|e| matches!(e, TestEvent::TextReceived(_, _))).count();
            count >= *n
        }
        WaitCondition::Connected(name) => {
            events.iter().any(|e| matches!(e, TestEvent::Connected(n) if n == name))
        }
        WaitCondition::AllConnected => {
            app.worlds.iter().all(|w| w.connected)
        }
    }
}
