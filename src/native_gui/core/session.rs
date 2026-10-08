//! A client core wired to a connection: carries out the core's effects (connect, send,
//! trust a certificate, keep a key) and hands back the ones for whoever shows the
//! result. The GUI, the offscreen snapshot and the headless verifier all drive the core
//! through this, so they behave identically.

use std::time::Instant;

use futures::channel::mpsc;

use super::client::{ClientCore, Credentials, Effect, Input};
use super::net::{Target, Transport};
use super::store;
use crate::websocket::WsMessage;

pub struct Session {
    pub core: ClientCore,
    pub target: Target,
    transport: Transport,
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
        let mut remembered = store::server(&target.key());
        if auth_key.is_some() {
            remembered.auth_key = auth_key;
        }
        let web_path = target.path.clone().or(remembered.web_path).unwrap_or_else(|| "clay".to_string());
        let (tx, rx) = mpsc::unbounded();
        let transport = super::net::spawn(runtime, target.clone(), remembered.auth_key.clone(), web_path, tx);
        let creds = Credentials { username: None, password, auth_key: remembered.auth_key };
        let core = ClientCore::new(creds, store::client_uid(), Instant::now());
        (Session { core, target, transport }, rx)
    }

    /// Carry out the effects that are ours; return the rest (`Changed`, `Event`).
    pub fn apply(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        let mut rest = Vec::new();
        for e in effects {
            match e {
                Effect::Connect => self.transport.connect(),
                Effect::Disconnect => self.transport.disconnect(),
                Effect::Send(msg) => self.transport.send(*msg),
                Effect::TrustCert { host, fingerprint } => crate::persistence::replace_pin(&host, &fingerprint),
                Effect::StoreAuthKey(key) => {
                    store::set_auth_key(&self.target.key(), &key);
                    if let Ok(mut k) = self.transport.knock_key.lock() {
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
        if let Some(path) = self.core.settings.as_ref().map(|s| s.web_path.clone()) {
            let changed = self.transport.web_path.lock().map(|mut p| {
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

    pub fn tick(&mut self) -> Vec<Effect> {
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
}
