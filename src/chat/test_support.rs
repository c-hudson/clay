//! Fake Discord/Slack servers for the chat integration tests: a minimal HTTP/1.1
//! responder (`FakeRest`) that records every request, and plain-`ws://` WebSocket
//! acceptors the tests script directly. Nothing here touches the network beyond
//! 127.0.0.1.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// A route handler: (method, path, body) -> (status, json body, extra headers).
pub type Handler = Arc<dyn Fn(&str, &str, &str) -> (u16, String, Vec<(String, String)>) + Send + Sync>;

pub struct FakeRest {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl FakeRest {
    pub async fn start(handler: Handler) -> FakeRest {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let reqs = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                let reqs = reqs.clone();
                tokio::spawn(async move {
                    // Serve requests on this connection until the client closes it.
                    let mut buf: Vec<u8> = Vec::new();
                    loop {
                        let head_end = loop {
                            if let Some(i) = find(&buf, b"\r\n\r\n") {
                                break i;
                            }
                            let mut chunk = [0u8; 4096];
                            match sock.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            }
                        };
                        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                        let mut lines = head.split("\r\n");
                        let request_line = lines.next().unwrap_or("");
                        let mut parts = request_line.split_whitespace();
                        let method = parts.next().unwrap_or("").to_string();
                        let path = parts.next().unwrap_or("").to_string();
                        let headers: Vec<(String, String)> = lines
                            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                            .collect();
                        let len: usize = headers
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, v)| v.parse().ok())
                            .unwrap_or(0);
                        let body_start = head_end + 4;
                        while buf.len() < body_start + len {
                            let mut chunk = [0u8; 4096];
                            match sock.read(&mut chunk).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            }
                        }
                        let body = String::from_utf8_lossy(&buf[body_start..body_start + len]).to_string();
                        buf.drain(..body_start + len);
                        reqs.lock().unwrap().push(Recorded { method: method.clone(), path: path.clone(), headers, body: body.clone() });
                        let (status, json, extra) = handler(&method, &path, &body);
                        let mut resp = format!(
                            "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
                            status,
                            json.len()
                        );
                        for (k, v) in extra {
                            resp.push_str(&format!("{}: {}\r\n", k, v));
                        }
                        resp.push_str("\r\n");
                        resp.push_str(&json);
                        if sock.write_all(resp.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        FakeRest { base: format!("http://{}", addr), requests }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A WebSocket listener; each `accept()` returns the next client connection.
pub struct FakeWs {
    pub url: String,
    listener: TcpListener,
}

pub type ServerWs = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

impl FakeWs {
    pub async fn start() -> FakeWs {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        FakeWs { url: format!("ws://{}", addr), listener }
    }

    pub async fn accept(&self) -> ServerWs {
        let (sock, _) = tokio::time::timeout(std::time::Duration::from_secs(10), self.listener.accept())
            .await
            .expect("timed out waiting for the client to connect")
            .unwrap();
        tokio_tungstenite::accept_async(sock).await.unwrap()
    }
}

/// Read the next text frame as JSON (skips pings/pongs), with a timeout.
pub async fn next_json(ws: &mut ServerWs) -> serde_json::Value {
    use futures::StreamExt;
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
            .await
            .expect("timed out waiting for a client frame")
            .expect("client closed")
            .expect("ws error");
        if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

pub async fn send_json(ws: &mut ServerWs, v: serde_json::Value) {
    use futures::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(v.to_string())).await.unwrap();
}

/// Receive the next `AppEvent::Chat` from the channel (with a timeout), skipping
/// any other event.
pub async fn next_chat(rx: &mut tokio::sync::mpsc::Receiver<crate::AppEvent>) -> crate::chat::ChatEvent {
    loop {
        let ev = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
            .await
            .expect("timed out waiting for a chat event")
            .expect("event channel closed");
        if let crate::AppEvent::Chat(_, _, e) = ev {
            return e;
        }
    }
}
