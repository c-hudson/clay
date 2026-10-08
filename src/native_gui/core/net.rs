//! The native GUI's connection to a Clay server: TCP, then the CLAY-KNOCK preamble when
//! it has an auth key, then TLS pinned trust-on-first-use (`TofuVerifier`, shared
//! `~/.clay/known_hosts.dat` with `--console` and `--gui`), then the WebSocket.
//!
//! Plaintext `ws://` only ever goes to this machine (CLAUDE.md "Connection Security":
//! the server serves loopback plain and everything else TLS). Policy — when to connect,
//! what to send — is the client core's; this module only carries it out, one connection
//! at a time, and reports back as [`Input`]s.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use super::client::{CertMismatch, Input};
use crate::websocket::WsMessage;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Each step of the knock (the server allows 10 s for each too).
const KNOCK_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_PORT: u16 = 9000;

/// Where to connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    /// TLS. Always for another machine; for this one, only when asked for (`wss://`).
    pub secure: bool,
    /// The server's web path (`web_path`), when given as `host:port/path`.
    pub path: Option<String>,
}

impl Target {
    /// `[ws://|wss://]host[:port][/path]`; IPv6 as `[addr]:port`. Default: this machine,
    /// port 9000.
    pub fn parse(arg: Option<&str>) -> Result<Target, String> {
        let arg = arg.unwrap_or("127.0.0.1").trim();
        let (scheme, rest) = match arg.split_once("://") {
            Some((s, r)) if s.eq_ignore_ascii_case("ws") || s.eq_ignore_ascii_case("wss") => (Some(s.to_ascii_lowercase()), r),
            Some((s, _)) => return Err(format!("unknown scheme '{s}://' (use ws:// or wss://)")),
            None => (None, arg),
        };
        let (hostport, path) = match rest.find('/') {
            Some(i) => (&rest[..i], Some(rest[i..].trim_matches('/').to_string())),
            None => (rest, None),
        };
        let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or("unterminated [ in the address")?;
            let port = match after.strip_prefix(':') {
                Some(p) => p.parse().map_err(|_| format!("invalid port '{p}'"))?,
                None if after.is_empty() => DEFAULT_PORT,
                None => return Err(format!("unexpected '{after}' after the address")),
            };
            (h.to_string(), port)
        } else {
            match hostport.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().map_err(|_| format!("invalid port '{p}'"))?),
                Some(_) => (hostport.to_string(), DEFAULT_PORT), // a bare IPv6 address
                None => (hostport.to_string(), DEFAULT_PORT),
            }
        };
        if host.is_empty() {
            return Err("no host given".into());
        }
        let loopback = crate::util::is_loopback_host(&host);
        let secure = match scheme.as_deref() {
            Some("ws") if !loopback => {
                return Err(format!("{host} is not this machine: plaintext ws:// is refused (Clay's password must not cross the network in the clear)."));
            }
            Some("ws") => false,
            Some(_) => true,
            None => !loopback,
        };
        Ok(Target { host, port, secure, path: path.filter(|p| !p.is_empty()) })
    }

    pub fn is_loopback(&self) -> bool {
        crate::util::is_loopback_host(&self.host)
    }

    fn host_for_url(&self) -> String {
        if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() }
    }

    /// The server's name for pins and stored keys: `host:port`, as `--console` writes it.
    pub fn key(&self) -> String {
        format!("{}:{}", self.host_for_url(), self.port)
    }

    fn url(&self, web_path: &str) -> String {
        let path = if web_path.is_empty() { "/ws".to_string() } else { format!("/{web_path}/ws") };
        format!("{}://{}:{}{}", if self.secure { "wss" } else { "ws" }, self.host_for_url(), self.port, path)
    }
}

enum Command {
    Connect,
    Disconnect,
    Send(Box<WsMessage>),
}

/// The handle the session drives the connection through.
#[derive(Clone)]
pub struct Transport {
    tx: mpsc::UnboundedSender<Command>,
    /// The auth key to knock with (updated when the server hands a new one over).
    pub knock_key: Arc<Mutex<Option<String>>>,
    /// The server's web path (from the target, the store, or the server's settings).
    pub web_path: Arc<Mutex<String>>,
}

impl Transport {
    pub fn connect(&self) {
        let _ = self.tx.unbounded_send(Command::Connect);
    }

    pub fn disconnect(&self) {
        let _ = self.tx.unbounded_send(Command::Disconnect);
    }

    pub fn send(&self, msg: WsMessage) {
        let _ = self.tx.unbounded_send(Command::Send(Box::new(msg)));
    }
}

/// Start the connection task (idle until told to connect). Reports come back on `events`.
pub fn spawn(
    runtime: &tokio::runtime::Handle,
    target: Target,
    knock_key: Option<String>,
    web_path: String,
    events: mpsc::UnboundedSender<Input>,
) -> Transport {
    let (tx, mut rx) = mpsc::unbounded::<Command>();
    let transport = Transport { tx, knock_key: Arc::new(Mutex::new(knock_key)), web_path: Arc::new(Mutex::new(web_path)) };
    let (key, path) = (transport.knock_key.clone(), transport.web_path.clone());
    runtime.spawn(async move {
        // A server that rejects the knock (an old one, or a changed key) is not knocked at
        // again by this process; Android does the same.
        let knock_off = AtomicBool::new(false);
        while let Some(cmd) = rx.next().await {
            if !matches!(cmd, Command::Connect) {
                continue; // nothing open: sends and disconnects have nowhere to go
            }
            let key = key.lock().ok().and_then(|k| k.clone());
            let web_path = path.lock().map(|p| p.clone()).unwrap_or_default();
            match open(&target, key.as_deref(), &web_path, &knock_off).await {
                Ok(ws) => {
                    if events.unbounded_send(Input::Opened).is_err() {
                        return;
                    }
                    let reason = pump(ws, &mut rx, &events).await;
                    if events.unbounded_send(Input::Closed(reason)).is_err() {
                        return;
                    }
                }
                Err(Failure::Mismatch(m)) => {
                    let _ = events.unbounded_send(Input::CertMismatch(m));
                }
                Err(Failure::Other(e)) => {
                    let _ = events.unbounded_send(Input::Closed(e));
                }
            }
        }
    });
    transport
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum Failure {
    Mismatch(CertMismatch),
    Other(String),
}

async fn tcp(target: &Target) -> Result<TcpStream, Failure> {
    let addr = (target.host.as_str(), target.port);
    match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
        Ok(Ok(s)) => {
            let _ = s.set_nodelay(true);
            Ok(s)
        }
        Ok(Err(e)) => Err(Failure::Other(format!("can't reach {}: {e}", target.key()))),
        Err(_) => Err(Failure::Other(format!("can't reach {}: timed out", target.key()))),
    }
}

async fn open(target: &Target, key: Option<&str>, web_path: &str, knock_off: &AtomicBool) -> Result<Ws, Failure> {
    let mut stream = tcp(target).await?;
    if let Some(key) = key.filter(|k| !k.is_empty()) {
        if !target.is_loopback() && !knock_off.load(Ordering::Relaxed) {
            if let Err(e) = knock(&mut stream, key).await {
                crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: knock refused by {} ({e}); connecting without it", target.key()));
                knock_off.store(true, Ordering::Relaxed);
                stream = tcp(target).await?;
            }
        }
    }
    let url = target.url(target.path.as_deref().unwrap_or(web_path));
    let connector = if target.secure { Connector::Rustls(Arc::new(tls_config(&target.key()))) } else { Connector::Plain };
    let handshake = tokio_tungstenite::client_async_tls_with_config(url.as_str(), stream, None, Some(connector));
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(e)) => {
            if let Some(m) = crate::platform::danger::take_cert_mismatch() {
                return Err(Failure::Mismatch(CertMismatch { host: m.host, old_fingerprint: m.old_fingerprint, new_fingerprint: m.new_fingerprint }));
            }
            Err(Failure::Other(format!("{url}: {e}")))
        }
        Err(_) => Err(Failure::Other(format!("{url}: no answer"))),
    }
}

/// TLS that trusts the server's certificate on first sight and pins it, refusing a
/// changed one (D7). The signatures are still verified for real.
fn tls_config(pin_key: &str) -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(crate::platform::danger_rustls::TofuVerifier::new(pin_key.to_string())))
        .with_no_client_auth()
}

/// CLAY-KNOCK v1, the client side (the server's is `http::run_knock_handshake`; Android's
/// `NativeWebSocket.KnockSocket`): prove the auth key before TLS, so a server whose allow
/// list doesn't name this address lets the connection through to log in. On success the
/// stream sits exactly where TLS begins.
async fn knock(stream: &mut TcpStream, key: &str) -> Result<(), String> {
    use crate::http::{KNOCK_ACK, KNOCK_CHALLENGE_MAGIC, KNOCK_MAGIC, KNOCK_VERSION, knock_expected_response};
    let step = |what: &'static str| move |_| format!("{what}: timed out");
    let mut hello = [0u8; 6];
    hello[..4].copy_from_slice(&KNOCK_MAGIC);
    hello[4] = KNOCK_VERSION;
    tokio::time::timeout(KNOCK_TIMEOUT, stream.write_all(&hello)).await.map_err(step("hello"))?.map_err(|e| e.to_string())?;
    let mut challenge = [0u8; 34];
    tokio::time::timeout(KNOCK_TIMEOUT, stream.read_exact(&mut challenge)).await.map_err(step("challenge"))?.map_err(|e| e.to_string())?;
    if challenge[..2] != KNOCK_CHALLENGE_MAGIC {
        return Err("not a knock challenge".into());
    }
    let nonce: [u8; 32] = challenge[2..].try_into().expect("32 bytes");
    let response = knock_expected_response(key, &nonce);
    tokio::time::timeout(KNOCK_TIMEOUT, stream.write_all(&response)).await.map_err(step("response"))?.map_err(|e| e.to_string())?;
    let mut ack = [0u8; 2];
    tokio::time::timeout(KNOCK_TIMEOUT, stream.read_exact(&mut ack)).await.map_err(step("ack"))?.map_err(|e| e.to_string())?;
    if ack != KNOCK_ACK {
        return Err("no knock ack".into());
    }
    Ok(())
}

/// Carry messages both ways until the connection ends; returns why.
async fn pump(mut ws: Ws, commands: &mut mpsc::UnboundedReceiver<Command>, events: &mpsc::UnboundedSender<Input>) -> String {
    loop {
        tokio::select! {
            frame = ws.next() => {
                let text = match frame {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None => return "the server closed the connection".into(),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return format!("connection lost: {e}"),
                };
                match serde_json::from_str::<WsMessage>(&text) {
                    Ok(msg) => {
                        if events.unbounded_send(Input::Message(Box::new(msg))).is_err() {
                            let _ = ws.close(None).await;
                            return "closed".into();
                        }
                    }
                    Err(e) => {
                        // A message this build doesn't know: say which, rather than vanish
                        // (the server's WS-BAD-MESSAGE does the same in the other direction).
                        let kind = serde_json::from_str::<serde_json::Value>(&text)
                            .ok()
                            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
                            .unwrap_or_default();
                        crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: unreadable message type={kind} err={e}"));
                    }
                }
            }
            cmd = commands.next() => match cmd {
                Some(Command::Send(msg)) => {
                    let Ok(text) = serde_json::to_string(&*msg) else { continue };
                    if let Err(e) = ws.send(Message::Text(text)).await {
                        return format!("connection lost: {e}");
                    }
                }
                Some(Command::Disconnect) | None => {
                    let _ = ws.close(None).await;
                    return "closed by this client".into();
                }
                Some(Command::Connect) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_parse_and_plaintext_stays_on_this_machine() {
        let t = Target::parse(None).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.secure), ("127.0.0.1", 9000, false));
        let t = Target::parse(Some("mud.example.org")).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.secure), ("mud.example.org", 9000, true));
        let t = Target::parse(Some("10.0.0.5:9443/stealth")).unwrap();
        assert_eq!((t.port, t.secure, t.path.as_deref()), (9443, true, Some("stealth")));
        assert_eq!(t.url("stealth"), "wss://10.0.0.5:9443/stealth/ws");
        let t = Target::parse(Some("[::1]:9001")).unwrap();
        assert_eq!((t.host.as_str(), t.port, t.secure), ("::1", 9001, false));
        assert_eq!(t.key(), "[::1]:9001");
        assert!(Target::parse(Some("wss://localhost")).unwrap().secure);
        assert!(Target::parse(Some("ws://mud.example.org")).is_err());
        assert!(Target::parse(Some("http://x")).is_err());
        assert_eq!(Target::parse(Some("h:1")).unwrap().url(""), "wss://h:1/ws");
        assert_eq!(Target::parse(Some("h:1")).unwrap().url("clay"), "wss://h:1/clay/ws");
    }

    /// The knock against the server's own handshake code (`http::gate_connection`, via a
    /// minimal server that runs exactly the bytes it expects).
    #[tokio::test]
    async fn knocks_with_the_servers_own_digest() {
        use crate::http::{KNOCK_ACK, KNOCK_CHALLENGE_MAGIC, KNOCK_MAGIC, KNOCK_VERSION, knock_expected_response};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut hello = [0u8; 6];
            s.read_exact(&mut hello).await.unwrap();
            assert_eq!(&hello[..4], &KNOCK_MAGIC);
            assert_eq!(hello[4], KNOCK_VERSION);
            let nonce = [7u8; 32];
            let mut ch = KNOCK_CHALLENGE_MAGIC.to_vec();
            ch.extend_from_slice(&nonce);
            s.write_all(&ch).await.unwrap();
            let mut resp = [0u8; 32];
            s.read_exact(&mut resp).await.unwrap();
            let ok = resp == knock_expected_response("the-key", &nonce);
            if ok {
                s.write_all(&KNOCK_ACK).await.unwrap();
            }
            ok
        });
        let mut stream = TcpStream::connect(addr).await.unwrap();
        knock(&mut stream, "the-key").await.unwrap();
        assert!(server.await.unwrap());
    }

    #[tokio::test]
    async fn a_wrong_key_is_a_failed_knock() {
        use crate::http::{KNOCK_CHALLENGE_MAGIC, knock_expected_response};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut hello = [0u8; 6];
            s.read_exact(&mut hello).await.unwrap();
            let nonce = [1u8; 32];
            let mut ch = KNOCK_CHALLENGE_MAGIC.to_vec();
            ch.extend_from_slice(&nonce);
            s.write_all(&ch).await.unwrap();
            let mut resp = [0u8; 32];
            s.read_exact(&mut resp).await.unwrap();
            assert_ne!(resp, knock_expected_response("right-key", &nonce));
            // The server closes without an ack.
        });
        let mut stream = TcpStream::connect(addr).await.unwrap();
        assert!(knock(&mut stream, "wrong-key").await.is_err());
    }
}
