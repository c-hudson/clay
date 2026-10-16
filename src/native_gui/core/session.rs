//! A client core wired to a connection: carries out the core's effects (connect, send,
//! trust a certificate, keep a key) and hands back the ones for whoever shows the
//! result. The GUI, the offscreen snapshot and the headless verifier all drive the core
//! through this, so they behave identically.
//!
//! A `/window` is a second window on the same connection: a follower session, with a
//! core of its own (its own world on screen, its own more-mode and input) fed the
//! leader's messages. What the user does there waits in its outbox for the leader to
//! send; what the core says to keep a connection going (logins, keepalives, acks, gap
//! fills, what this client is looking at) is the leader's to say, and is dropped. The
//! leader says what its followers look at too (`set_other_views`).

use std::time::Instant;

use futures::channel::mpsc;

use super::client::{ClientCore, Credentials, Effect, Input};
use super::net::{Target, Transport};
use super::store;
use crate::websocket::WsMessage;

pub struct Session {
    pub core: ClientCore,
    pub target: Target,
    link: Link,
    /// Messages sent that a reply may answer (not the connection's own upkeep): a reply
    /// goes to the window that asked last.
    pub requests_sent: u64,
}

enum Link {
    /// This session's own connection.
    Socket(Transport),
    /// Another window's (the leader's), with what's waiting for it to send.
    Follower { outbox: Vec<WsMessage> },
}

/// What a client says only to keep its connection going, or to tell the server what it
/// is looking at: a follower's core says these too, and they're dropped (the leader's
/// count). Anything else a follower sends is the user's doing, and the leader sends it.
pub fn is_upkeep(msg: &WsMessage) -> bool {
    matches!(
        msg,
        WsMessage::AuthRequest { .. }
            | WsMessage::ClientTypeDeclaration { .. }
            | WsMessage::ClientVisibility { .. }
            | WsMessage::MarkWorldSeen { .. }
            | WsMessage::OtherViews { .. }
            | WsMessage::Ping
            | WsMessage::Pong
            | WsMessage::PongCheck { .. }
            | WsMessage::ReportDuplicate { .. }
            | WsMessage::ReportGap { .. }
            | WsMessage::ReportOutOfOrder { .. }
            | WsMessage::ReportSeqMismatch { .. }
            | WsMessage::ReportClientLifecycle { .. }
            | WsMessage::RequestScrollback { .. }
            | WsMessage::RequestState
            | WsMessage::UpdateViewState { .. }
    )
}

impl Session {
    /// A session for `target`, with what's remembered about it (its key, its web path) and
    /// the password if one was given. Events arrive on the returned receiver; feed them to
    /// [`Session::handle`].
    pub fn new(
        runtime: &tokio::runtime::Handle,
        target: Target,
        password: Option<String>,
        auth_key: Option<String>,
    ) -> (Session, mpsc::UnboundedReceiver<Input>) {
        Self::with_credentials(runtime, target, Credentials { username: None, password, auth_key })
    }

    /// The same, logging in with a name too (another window of a multiuser session).
    pub fn with_credentials(runtime: &tokio::runtime::Handle, target: Target, given: Credentials) -> (Session, mpsc::UnboundedReceiver<Input>) {
        let (password, auth_key) = (given.password, given.auth_key);
        let mut remembered = store::server(&target.key());
        if auth_key.is_some() {
            remembered.auth_key = auth_key;
        }
        let web_path = target.path.clone().or(remembered.web_path).unwrap_or_else(|| "clay".to_string());
        let (tx, rx) = mpsc::unbounded();
        let transport = super::net::spawn(runtime, target.clone(), remembered.auth_key.clone(), web_path, tx);
        let creds = Credentials { username: given.username, password, auth_key: remembered.auth_key };
        let core = ClientCore::new(creds, store::client_uid(), Instant::now());
        (Session { core, target, link: Link::Socket(transport), requests_sent: 0 }, rx)
    }

    /// Another window on `leader`'s connection (see the module's notes).
    pub fn follower(leader: &Session) -> Session {
        Session { core: leader.core.follower(), target: leader.target.clone(), link: Link::Follower { outbox: Vec::new() }, requests_sent: 0 }
    }

    pub fn is_follower(&self) -> bool {
        matches!(self.link, Link::Follower { .. })
    }

    /// A follower's messages for its leader to send.
    pub fn take_outbox(&mut self) -> Vec<WsMessage> {
        match &mut self.link {
            Link::Follower { outbox } => std::mem::take(outbox),
            Link::Socket(_) => Vec::new(),
        }
    }

    /// The address the next connection goes to (`wss://host:port/clay/ws`).
    pub fn url(&self) -> String {
        let web_path = match &self.link {
            Link::Socket(t) => t.web_path.lock().map(|p| p.clone()).unwrap_or_default(),
            Link::Follower { .. } => self.core.settings.as_ref().map(|s| s.web_path.clone()).unwrap_or_else(|| "clay".into()),
        };
        self.target.url(self.target.path.as_deref().unwrap_or(&web_path))
    }

    /// Carry out the effects that are ours; return the rest (`Changed`, `Event`).
    pub fn apply(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        let mut rest = Vec::new();
        for e in effects {
            let transport = match &mut self.link {
                Link::Socket(t) => t,
                // A follower only shows; the leader does the rest.
                Link::Follower { outbox } => {
                    match e {
                        Effect::Send(msg) if !is_upkeep(&msg) => outbox.push(*msg),
                        Effect::Changed | Effect::Event(_) => rest.push(e),
                        _ => {}
                    }
                    continue;
                }
            };
            match e {
                Effect::Connect => transport.connect(),
                Effect::Disconnect => transport.disconnect(),
                Effect::Send(msg) => {
                    if !is_upkeep(&msg) {
                        self.requests_sent += 1;
                    }
                    transport.send(*msg)
                }
                Effect::TrustCert { host, fingerprint } => crate::persistence::replace_pin(&host, &fingerprint),
                Effect::StoreAuthKey(key) => {
                    store::set_auth_key(&self.target.key(), &key);
                    if let Ok(mut k) = transport.knock_key.lock() {
                        *k = Some(key);
                    }
                }
                Effect::Changed => {
                    self.remember_web_path();
                    rest.push(Effect::Changed);
                }
                other => rest.push(other),
            }
        }
        rest
    }

    /// The server's web path, from its settings, for the next connection (unless the
    /// target named one).
    fn remember_web_path(&mut self) {
        if self.target.path.is_some() {
            return;
        }
        let Link::Socket(transport) = &self.link else { return };
        if let Some(path) = self.core.settings.as_ref().map(|s| s.web_path.clone()) {
            let changed = transport.web_path.lock().map(|mut p| {
                let changed = *p != path;
                *p = path.clone();
                changed
            });
            if changed.unwrap_or(false) {
                store::set_web_path(&self.target.key(), &path);
            }
        }
    }

    pub fn start(&mut self) -> Vec<Effect> {
        let e = self.core.start(Instant::now());
        self.apply(e)
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        let e = self.core.handle(input, Instant::now());
        self.apply(e)
    }

    /// The clock: keepalives, watchdogs, reconnects, background history. A follower's
    /// connection is its leader's, so its clock has nothing to do.
    pub fn tick(&mut self) -> Vec<Effect> {
        if self.is_follower() {
            return Vec::new();
        }
        let e = self.core.tick(Instant::now());
        self.apply(e)
    }

    pub fn login(&mut self, creds: Credentials) -> Vec<Effect> {
        let e = self.core.login(creds, Instant::now());
        self.apply(e)
    }

    pub fn trust_cert(&mut self) -> Vec<Effect> {
        let e = self.core.trust_cert();
        self.apply(e)
    }

    pub fn send(&mut self, msg: WsMessage) -> Vec<Effect> {
        let e = self.core.send_message(msg);
        self.apply(e)
    }

    /// The worlds the windows following this one show (`ClientCore::set_other_views`).
    pub fn set_other_views(&mut self, worlds: Vec<usize>) -> Vec<Effect> {
        let e = self.core.set_other_views(worlds);
        self.apply(e)
    }

    /// Start the connection over, everything fetched again (the menu's Reload and Resync).
    /// A follower has no connection to start over: nothing happens here, and the window
    /// asks its leader instead (`ClayView::restart_connection`). Running the core's
    /// restart on a follower would clear its worlds and wait for a reconnect that never
    /// comes - the splash screen with nothing behind it.
    pub fn restart(&mut self) -> Vec<Effect> {
        if self.is_follower() {
            return Vec::new();
        }
        let e = self.core.restart();
        self.apply(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_gui::core::client::Credentials;

    fn follower() -> Session {
        let core = ClientCore::new(Credentials::default(), "uid".into(), Instant::now());
        Session { core, target: Target::parse(None).expect("target"), link: Link::Follower { outbox: Vec::new() }, requests_sent: 0 }
    }

    #[test]
    fn a_follower_sends_the_users_doing_through_its_leader_and_nothing_else() {
        let mut s = follower();
        let send = |m: WsMessage| Effect::Send(Box::new(m));
        let rest = s.apply(vec![
            Effect::Connect,
            send(WsMessage::Ping),
            send(WsMessage::SendCommand { world_index: 0, command: "look".into() }),
            send(WsMessage::MarkWorldSeen { world_index: 1, previous_world_index: Some(0) }),
            send(WsMessage::ReleasePending { world_index: 0, count: 5 }),
            Effect::Disconnect,
            Effect::Changed,
        ]);
        assert!(matches!(rest.as_slice(), [Effect::Changed]), "only what's for the window to show");
        let out = s.take_outbox();
        assert!(matches!(out.as_slice(), [WsMessage::SendCommand { .. }, WsMessage::ReleasePending { .. }]), "{out:?}");
        assert!(s.take_outbox().is_empty());
        assert!(s.tick().is_empty(), "the leader's clock keeps the connection");
    }

    #[test]
    fn a_followers_restart_is_its_leaders_to_do() {
        let mut s = follower();
        let now = Instant::now();
        // As a leader's copy is: logged in, with the worlds.
        let _ = s.core.start(now);
        let _ = s.core.handle(Input::Opened, now);
        let _ = s.core.handle(Input::Message(Box::new(WsMessage::ServerHello { multiuser_mode: false, challenge: "abc123".into() })), now);
        let _ = s.core.handle(Input::Message(Box::new(WsMessage::AuthResponse { success: true, error: None, username: None, multiuser_mode: false })), now);
        let mut app = crate::App::new();
        app.worlds = vec![crate::World::new("Mini")];
        let _ = s.core.handle(Input::Message(Box::new(app.build_initial_state(0))), now);
        assert_eq!(s.core.worlds.len(), 1, "the state came in");
        assert!(s.restart().is_empty(), "nothing to do here");
        assert_eq!(s.core.worlds.len(), 1, "the worlds stay: the fresh state comes through the leader");
        assert!(s.take_outbox().is_empty());
    }
}
