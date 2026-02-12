// Test server module - fake MUD server for regression testing
// Provides scripted TCP server scenarios for testing Clay's core behaviors

use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::io::AsyncReadExt;

use crate::telnet::{TELNET_IAC, TELNET_WILL, TELNET_GA, TELNET_OPT_SGA};

/// Actions the test server can perform on a connection
#[derive(Clone, Debug)]
pub enum ServerAction {
    /// Send text followed by \r\n
    SendLine(String),
    /// Send raw bytes (for telnet sequences)
    SendRaw(Vec<u8>),
    /// Send multiple lines rapidly (for more-mode flooding)
    SendLines(Vec<String>),
    /// Wait for client to send data matching this string (with timeout)
    WaitForData(String, Duration),
    /// Pause between actions
    Sleep(Duration),
    /// Send IAC GA (telnet Go Ahead) - prompt marker
    SendGA,
    /// Close the connection
    Disconnect,
}

/// Configuration for one port's behavior
#[derive(Clone, Debug)]
pub struct PortScenario {
    pub actions: Vec<ServerAction>,
    /// Send telnet WILL SGA on connect
    pub telnet_negotiate: bool,
}

/// Get a named scenario
pub fn get_scenario(name: &str) -> PortScenario {
    match name {
        "more_flood" => scenario_more_flood(30),
        "more_flood_500" => scenario_more_flood(500),
        "auto_login_connect" => scenario_auto_login_connect(),
        "auto_login_prompt" => scenario_auto_login_prompt(),
        "basic_output" => scenario_basic_output(),
        "disconnect_after" => scenario_disconnect_after(),
        "idle" => scenario_idle(),
        _ => scenario_basic_output(),
    }
}

/// Sends N lines rapidly to trigger more-mode
fn scenario_more_flood(count: usize) -> PortScenario {
    let lines: Vec<String> = (1..=count)
        .map(|i| format!("Line {:03}", i))
        .collect();
    PortScenario {
        actions: vec![
            ServerAction::SendLines(lines),
            // Keep alive briefly so client can process
            ServerAction::Sleep(Duration::from_secs(2)),
            ServerAction::Disconnect,
        ],
        telnet_negotiate: false,
    }
}

/// Sends telnet negotiate + GA, waits for "connect user pass"
fn scenario_auto_login_connect() -> PortScenario {
    PortScenario {
        actions: vec![
            ServerAction::Sleep(Duration::from_millis(50)),
            // Send a welcome message
            ServerAction::SendLine("Welcome to the test MUD!".to_string()),
            // Wait for the client to send "connect testuser testpass"
            ServerAction::WaitForData("connect testuser testpass".to_string(), Duration::from_secs(5)),
            ServerAction::SendLine("Connected!".to_string()),
            ServerAction::Sleep(Duration::from_secs(1)),
            ServerAction::Disconnect,
        ],
        telnet_negotiate: true,
    }
}

/// Sends telnet GA prompts for prompt-based auto-login
fn scenario_auto_login_prompt() -> PortScenario {
    // Build raw bytes: "login: " + IAC GA as single buffer to ensure atomicity
    let mut login_prompt = b"login: ".to_vec();
    login_prompt.extend_from_slice(&[TELNET_IAC, TELNET_GA]);

    let mut password_prompt = b"password: ".to_vec();
    password_prompt.extend_from_slice(&[TELNET_IAC, TELNET_GA]);

    PortScenario {
        actions: vec![
            ServerAction::Sleep(Duration::from_millis(50)),
            // Send prompt "login: " + GA as single write (atomic)
            ServerAction::SendRaw(login_prompt),
            // Wait for username
            ServerAction::WaitForData("testuser".to_string(), Duration::from_secs(5)),
            // Delay to ensure separate TCP segment
            ServerAction::Sleep(Duration::from_millis(200)),
            // Send prompt "password: " + GA as single write (atomic)
            ServerAction::SendRaw(password_prompt),
            // Wait for password
            ServerAction::WaitForData("testpass".to_string(), Duration::from_secs(5)),
            ServerAction::Sleep(Duration::from_millis(200)),
            ServerAction::SendLine("Logged in!".to_string()),
            ServerAction::Sleep(Duration::from_secs(1)),
            ServerAction::Disconnect,
        ],
        telnet_negotiate: true,
    }
}

/// Sends 5 lines of text, stays connected
fn scenario_basic_output() -> PortScenario {
    PortScenario {
        actions: vec![
            ServerAction::SendLine("Welcome to the test world!".to_string()),
            ServerAction::SendLine("This is line 2.".to_string()),
            ServerAction::SendLine("This is line 3.".to_string()),
            ServerAction::SendLine("This is line 4.".to_string()),
            ServerAction::SendLine("This is line 5.".to_string()),
            ServerAction::Sleep(Duration::from_secs(2)),
            ServerAction::Disconnect,
        ],
        telnet_negotiate: false,
    }
}

/// Sends a few lines then disconnects
fn scenario_disconnect_after() -> PortScenario {
    PortScenario {
        actions: vec![
            ServerAction::SendLine("Hello!".to_string()),
            ServerAction::SendLine("Goodbye!".to_string()),
            ServerAction::Sleep(Duration::from_millis(100)),
            ServerAction::Disconnect,
        ],
        telnet_negotiate: false,
    }
}

/// Accepts connection, sends nothing (for activity testing)
fn scenario_idle() -> PortScenario {
    PortScenario {
        actions: vec![
            ServerAction::Sleep(Duration::from_secs(30)),
            ServerAction::Disconnect,
        ],
        telnet_negotiate: false,
    }
}

/// Run a test server on the given port with the given scenario.
/// Returns when all connections have been handled.
pub async fn run_server_port(port: u16, scenario: PortScenario) {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    // Accept one connection and run the scenario
    if let Ok((mut stream, _)) = listener.accept().await {
        // Send telnet negotiation if requested
        if scenario.telnet_negotiate {
            let negotiate = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA];
            let _ = stream.write_all(&negotiate).await;
        }

        for action in &scenario.actions {
            match action {
                ServerAction::SendLine(text) => {
                    let data = format!("{}\r\n", text);
                    if stream.write_all(data.as_bytes()).await.is_err() {
                        break;
                    }
                }
                ServerAction::SendRaw(bytes) => {
                    if stream.write_all(bytes).await.is_err() {
                        break;
                    }
                }
                ServerAction::SendLines(lines) => {
                    // Send all lines as a single write for maximum throughput
                    let mut data = String::new();
                    for line in lines {
                        data.push_str(line);
                        data.push_str("\r\n");
                    }
                    if stream.write_all(data.as_bytes()).await.is_err() {
                        break;
                    }
                }
                ServerAction::WaitForData(expected, timeout) => {
                    let mut buf = vec![0u8; 4096];
                    let mut collected = String::new();
                    let deadline = tokio::time::Instant::now() + *timeout;
                    loop {
                        let remaining = deadline - tokio::time::Instant::now();
                        if remaining.is_zero() {
                            break;
                        }
                        match tokio::time::timeout(remaining, stream.read(&mut buf)).await {
                            Ok(Ok(n)) if n > 0 => {
                                let s = String::from_utf8_lossy(&buf[..n]);
                                collected.push_str(&s);
                                if collected.contains(expected.as_str()) {
                                    break;
                                }
                            }
                            _ => break,
                        }
                    }
                }
                ServerAction::Sleep(dur) => {
                    tokio::time::sleep(*dur).await;
                }
                ServerAction::SendGA => {
                    let ga = vec![TELNET_IAC, TELNET_GA];
                    if stream.write_all(&ga).await.is_err() {
                        break;
                    }
                }
                ServerAction::Disconnect => {
                    let _ = stream.shutdown().await;
                    break;
                }
            }
        }
    }
}

/// Run the test server binary main function.
/// Parses CLI args for --ports and --scenarios.
pub async fn run_server_main() {
    let args: Vec<String> = std::env::args().collect();

    let mut start_port: u16 = 19001;
    let mut scenario_names = vec!["more_flood", "auto_login_connect", "basic_output", "idle"];

    for arg in &args[1..] {
        if let Some(ports_str) = arg.strip_prefix("--ports=") {
            if let Some((start, _end)) = ports_str.split_once('-') {
                if let Ok(p) = start.parse::<u16>() {
                    start_port = p;
                }
            }
        } else if let Some(scenarios_str) = arg.strip_prefix("--scenarios=") {
            scenario_names = scenarios_str.split(',').collect();
        }
    }

    let mut handles = Vec::new();
    for (i, name) in scenario_names.iter().enumerate() {
        let port = start_port + i as u16;
        let scenario = get_scenario(name);
        handles.push(tokio::spawn(run_server_port(port, scenario)));
    }

    for handle in handles {
        let _ = handle.await;
    }
}
