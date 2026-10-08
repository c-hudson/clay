//! The native GUI's connection to a Clay server.
//!
//! Phase 0 (the rendering spike) only needs a Clay on this machine: plain `ws://` to a
//! loopback address, logged in with the server's password through the ServerHello
//! challenge. TLS, CLAY-KNOCK, auth keys, reconnects and the rest of the client's duties
//! come in Phase 1.

use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use crate::websocket::{hash_password, hash_with_challenge, RemoteClientType, WsMessage};

/// What the connection reports to the GUI.
#[derive(Debug)]
pub enum NetEvent {
    Message(Box<WsMessage>),
    Closed(String),
}

/// Where to connect.
#[derive(Clone, Debug)]
pub struct Target {
    pub host: String,
    pub port: u16,
}

impl Target {
    /// `host[:port]`, defaulting to this machine and port 9000.
    pub fn parse(arg: Option<&str>) -> Result<Target, String> {
        let arg = arg.unwrap_or("127.0.0.1");
        let (host, port) = match arg.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') || h.starts_with('[') => {
                (h.trim_matches(|c| c == '[' || c == ']').to_string(), p.parse().map_err(|_| format!("invalid port '{}'", p))?)
            }
            _ => (arg.to_string(), 9000),
        };
        if !crate::util::is_loopback_host(&host) {
            return Err(format!(
                "{} is not this machine. The native GUI's prototype only connects locally so far.",
                host
            ));
        }
        Ok(Target { host, port })
    }
}

/// Sends messages to the server.
#[derive(Clone)]
pub struct Sender(mpsc::UnboundedSender<WsMessage>);

impl Sender {
    pub fn send(&self, msg: WsMessage) {
        let _ = self.0.unbounded_send(msg);
    }
}

/// Connect and log in on the tokio runtime; events arrive on the returned receiver.
pub fn connect(
    runtime: &tokio::runtime::Handle,
    target: Target,
    password: String,
    client_uid: String,
) -> (Sender, mpsc::UnboundedReceiver<NetEvent>) {
    let (out_tx, out_rx) = mpsc::unbounded::<WsMessage>();
    let (event_tx, event_rx) = mpsc::unbounded::<NetEvent>();
    runtime.spawn(async move {
        let reason = match run(target, password, client_uid, out_rx, event_tx.clone()).await {
            Ok(()) => "connection closed".to_string(),
            Err(e) => e,
        };
        let _ = event_tx.unbounded_send(NetEvent::Closed(reason));
    });
    (Sender(out_tx), event_rx)
}

async fn run(
    target: Target,
    password: String,
    client_uid: String,
    mut out_rx: mpsc::UnboundedReceiver<WsMessage>,
    events: mpsc::UnboundedSender<NetEvent>,
) -> Result<(), String> {
    let host = if target.host.contains(':') { format!("[{}]", target.host) } else { target.host.clone() };
    let url = format!("ws://{}:{}/clay/ws", host, target.port);
    let (ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("can't connect to {}: {}", url, e))?;
    let (mut write, mut read) = ws.split();

    let send = |msg: &WsMessage| Message::Text(serde_json::to_string(msg).unwrap_or_default());

    let mut authed = false;
    loop {
        tokio::select! {
            frame = read.next() => {
                let text = match frame {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(format!("connection error: {}", e)),
                };
                let msg: WsMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                match &msg {
                    WsMessage::ServerHello { challenge, .. } if !authed => {
                        if challenge.is_empty() {
                            return Err("the server sent no login challenge".to_string());
                        }
                        let reply = WsMessage::AuthRequest {
                            username: None,
                            password_hash: hash_with_challenge(&hash_password(&password), challenge),
                            current_world: None,
                            auth_key: None,
                            request_key: false,
                            challenge_response: true,
                            resume: Vec::new(),
                            resume_epochs: Vec::new(),
                            client_version: crate::VERSION.to_string(),
                            client_uid: client_uid.clone(),
                        };
                        write.send(send(&reply)).await.map_err(|e| e.to_string())?;
                    }
                    WsMessage::AuthResponse { success, error, .. } => {
                        if !success {
                            return Err(error.clone().unwrap_or_else(|| "login refused".to_string()));
                        }
                        authed = true;
                        let decl = WsMessage::ClientTypeDeclaration { client_type: RemoteClientType::RemoteGUI };
                        write.send(send(&decl)).await.map_err(|e| e.to_string())?;
                    }
                    WsMessage::Ping => {
                        write.send(send(&WsMessage::Pong)).await.map_err(|e| e.to_string())?;
                    }
                    _ => {}
                }
                if events.unbounded_send(NetEvent::Message(Box::new(msg))).is_err() {
                    return Ok(());
                }
            }
            out = out_rx.next() => {
                match out {
                    Some(msg) => write.send(send(&msg)).await.map_err(|e| e.to_string())?,
                    None => return Ok(()),
                }
            }
        }
    }
}
