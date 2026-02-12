// Standalone fake MUD server for regression testing
// Usage: clay-test-server [--ports=19001-19004] [--scenarios=more_flood,auto_login_connect,basic_output,idle]

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// Telnet constants
const TELNET_IAC: u8 = 255;
const TELNET_WILL: u8 = 251;
const TELNET_GA: u8 = 249;
const TELNET_OPT_SGA: u8 = 3;

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum ServerAction {
    SendLine(String),
    SendRaw(Vec<u8>),
    SendLines(Vec<String>),
    WaitForData(String, Duration),
    Sleep(Duration),
    SendGA,
    Disconnect,
}

#[derive(Clone, Debug)]
struct PortScenario {
    actions: Vec<ServerAction>,
    telnet_negotiate: bool,
}

fn get_scenario(name: &str) -> PortScenario {
    match name {
        "more_flood" => {
            let lines: Vec<String> = (1..=30).map(|i| format!("Line {:03}", i)).collect();
            PortScenario {
                actions: vec![
                    ServerAction::SendLines(lines),
                    ServerAction::Sleep(Duration::from_secs(2)),
                    ServerAction::Disconnect,
                ],
                telnet_negotiate: false,
            }
        }
        "auto_login_connect" => PortScenario {
            actions: vec![
                ServerAction::Sleep(Duration::from_millis(50)),
                ServerAction::SendLine("Welcome to the test MUD!".to_string()),
                ServerAction::WaitForData("connect testuser testpass".to_string(), Duration::from_secs(5)),
                ServerAction::SendLine("Connected!".to_string()),
                ServerAction::Sleep(Duration::from_secs(1)),
                ServerAction::Disconnect,
            ],
            telnet_negotiate: true,
        },
        "basic_output" => PortScenario {
            actions: vec![
                ServerAction::SendLine("Welcome to the test world!".to_string()),
                ServerAction::SendLine("This is line 2.".to_string()),
                ServerAction::Sleep(Duration::from_secs(2)),
                ServerAction::Disconnect,
            ],
            telnet_negotiate: false,
        },
        "idle" => PortScenario {
            actions: vec![
                ServerAction::Sleep(Duration::from_secs(30)),
                ServerAction::Disconnect,
            ],
            telnet_negotiate: false,
        },
        _ => PortScenario {
            actions: vec![
                ServerAction::SendLine("Hello!".to_string()),
                ServerAction::Sleep(Duration::from_secs(2)),
                ServerAction::Disconnect,
            ],
            telnet_negotiate: false,
        },
    }
}

async fn run_server_port(port: u16, scenario: PortScenario) {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await
        .unwrap_or_else(|e| panic!("Failed to bind to {}: {}", addr, e));

    println!("Listening on port {}", port);

    if let Ok((mut stream, _)) = listener.accept().await {
        if scenario.telnet_negotiate {
            let negotiate = vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_SGA];
            let _ = stream.write_all(&negotiate).await;
        }

        for action in &scenario.actions {
            match action {
                ServerAction::SendLine(text) => {
                    let data = format!("{}\r\n", text);
                    if stream.write_all(data.as_bytes()).await.is_err() { break; }
                }
                ServerAction::SendRaw(bytes) => {
                    if stream.write_all(bytes).await.is_err() { break; }
                }
                ServerAction::SendLines(lines) => {
                    let mut data = String::new();
                    for line in lines {
                        data.push_str(line);
                        data.push_str("\r\n");
                    }
                    if stream.write_all(data.as_bytes()).await.is_err() { break; }
                }
                ServerAction::WaitForData(expected, timeout) => {
                    let mut buf = vec![0u8; 4096];
                    let mut collected = String::new();
                    let deadline = tokio::time::Instant::now() + *timeout;
                    loop {
                        let remaining = deadline - tokio::time::Instant::now();
                        if remaining.is_zero() { break; }
                        match tokio::time::timeout(remaining, stream.read(&mut buf)).await {
                            Ok(Ok(n)) if n > 0 => {
                                collected.push_str(&String::from_utf8_lossy(&buf[..n]));
                                if collected.contains(expected.as_str()) { break; }
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
                    if stream.write_all(&ga).await.is_err() { break; }
                }
                ServerAction::Disconnect => {
                    let _ = stream.shutdown().await;
                    break;
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut start_port: u16 = 19001;
    let mut scenario_names: Vec<&str> = vec!["more_flood", "auto_login_connect", "basic_output", "idle"];

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

    println!("Test server running on ports {}-{}", start_port, start_port + scenario_names.len() as u16 - 1);

    for handle in handles {
        let _ = handle.await;
    }
}
