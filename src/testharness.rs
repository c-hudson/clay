// Test harness module - in-process test framework for regression testing Clay
// Creates an App, connects to test servers, processes events, and captures test outcomes

use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::{
    App, World, WriteCommand, AutoConnectType,
    telnet,
    find_safe_split_point,
};

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

/// Channel message from reader tasks to the harness
enum ReaderEvent {
    Data(String, Vec<u8>),    // world_name, cleaned bytes (telnet processed)
    Disconnected(String),      // world_name
    Prompt(String, Vec<u8>),   // world_name, prompt bytes
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
                let tx = reader_tx.clone();

                // Spawn reader task (mirrors the real reader task pattern from main.rs)
                let reader_handle = tokio::spawn(async move {
                    let mut reader = read_half;
                    let mut buf = vec![0u8; 8192];
                    let mut line_buffer: Vec<u8> = Vec::new();

                    loop {
                        match reader.read(&mut buf).await {
                            Ok(0) => {
                                // Flush remaining buffer
                                if !line_buffer.is_empty() {
                                    let result = telnet::process_telnet(&line_buffer);
                                    if let Some(prompt_bytes) = result.prompt {
                                        let _ = tx.send(ReaderEvent::Prompt(
                                            world_name.clone(), prompt_bytes,
                                        ));
                                    }
                                    if !result.cleaned.is_empty() {
                                        let _ = tx.send(ReaderEvent::Data(
                                            world_name.clone(), result.cleaned,
                                        ));
                                    }
                                }
                                let _ = tx.send(ReaderEvent::Disconnected(world_name.clone()));
                                break;
                            }
                            Ok(n) => {
                                line_buffer.extend_from_slice(&buf[..n]);

                                let split_at = find_safe_split_point(&line_buffer);
                                let to_send = if split_at > 0 {
                                    line_buffer.drain(..split_at).collect()
                                } else if !line_buffer.is_empty() {
                                    std::mem::take(&mut line_buffer)
                                } else {
                                    Vec::new()
                                };

                                if !to_send.is_empty() {
                                    let result = telnet::process_telnet(&to_send);

                                    // Send prompt first (like the real reader)
                                    if let Some(prompt_bytes) = result.prompt {
                                        let _ = tx.send(ReaderEvent::Prompt(
                                            world_name.clone(), prompt_bytes,
                                        ));
                                    }

                                    // Send cleaned data
                                    if !result.cleaned.is_empty() {
                                        let _ = tx.send(ReaderEvent::Data(
                                            world_name.clone(), result.cleaned,
                                        ));
                                    }
                                }
                            }
                            Err(_) => {
                                let _ = tx.send(ReaderEvent::Disconnected(world_name.clone()));
                                break;
                            }
                        }
                    }
                });

                // Spawn writer task
                let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriteCommand>(100);
                let mut writer = write_half;
                tokio::spawn(async move {
                    while let Some(cmd) = cmd_rx.recv().await {
                        let bytes = match &cmd {
                            WriteCommand::Text(text) => {
                                let mut b = text.as_bytes().to_vec();
                                b.extend_from_slice(b"\r\n");
                                b
                            }
                            WriteCommand::Raw(raw) => raw.clone(),
                            WriteCommand::Shutdown => break,
                        };
                        if tokio::io::AsyncWriteExt::write_all(&mut writer, &bytes).await.is_err() {
                            break;
                        }
                    }
                });

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
                    app.worlds[idx].release_pending(visual_budget, output_width);
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
    prev_unseen: &mut Vec<usize>,
    prev_paused: &mut Vec<bool>,
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
